use std::fmt;
use std::io::ErrorKind;
use std::sync::Arc;
use std::time::Duration;

use futures::SinkExt;

use tokio::net;
use tokio::select;
use tokio::sync::mpsc;
use tokio_stream::StreamExt;
use tokio_util::codec;

use oddity_rtsp_protocol::{
    AsServer, Codec, Error, RequestMaybeInterleaved, ResponseMaybeInterleaved,
};

use crate::net::handler::Handler;
use crate::runtime::task_manager::{Task, TaskContext};
use crate::runtime::Runtime;

pub enum ConnectionState {
    Disconnected(ConnectionId),
    Closed(ConnectionId),
}

pub type ConnectionStateTx = mpsc::UnboundedSender<ConnectionState>;
pub type ConnectionStateRx = mpsc::UnboundedReceiver<ConnectionState>;

/// Carries interleaved RTP/RTCP from a session to its connection. Bounded so a
/// slow or stalled client can't grow memory without limit: sessions `try_send`
/// and drop media when it is full.
pub type ResponseSenderTx = mpsc::Sender<ResponseMaybeInterleaved>;
pub type ResponseSenderRx = mpsc::Receiver<ResponseMaybeInterleaved>;

/// ~2-3 s of RTP at typical 4-8 Mbps bitrates.
const MEDIA_QUEUE_LEN: usize = 1024;
/// Max messages written per flush.
const MAX_BATCH: usize = 64;
/// A client that can't absorb a write for this long is considered dead.
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);

pub struct Connection {
    worker: Task,
}

impl Connection {
    pub async fn start(
        id: ConnectionId,
        inner: net::TcpStream,
        handler: Arc<Handler>,
        state_tx: ConnectionStateTx,
        runtime: &Runtime,
    ) -> Self {
        let (sender_tx, sender_rx) = mpsc::channel(MEDIA_QUEUE_LEN);
        // We batch writes ourselves; don't let Nagle add latency on top.
        let _ = inner.set_nodelay(true);

        tracing::trace!(%id, "starting connection");
        let worker = runtime
            .task()
            .spawn(move |task_context| {
                Self::run(
                    id,
                    inner,
                    handler,
                    state_tx,
                    sender_tx,
                    sender_rx,
                    task_context,
                )
            })
            .await;
        tracing::trace!(%id, "started connection");

        Connection { worker }
    }

    pub async fn close(&mut self) {
        tracing::trace!("closing connection");
        self.worker.stop().await;
        tracing::trace!("closed connection");
    }

    async fn run(
        id: ConnectionId,
        inner: net::TcpStream,
        handler: Arc<Handler>,
        state_tx: ConnectionStateTx,
        response_tx: ResponseSenderTx,
        mut response_rx: ResponseSenderRx,
        mut task_context: TaskContext,
    ) {
        let mut disconnected = false;

        let addr = inner
            .peer_addr()
            .map(|peer_addr| peer_addr.to_string())
            .unwrap_or("?".to_string());
        let (read, write) = inner.into_split();
        let mut inbound = codec::FramedRead::new(read, Codec::<AsServer>::new());
        let mut outbound = codec::FramedWrite::new(write, Codec::<AsServer>::new());

        loop {
            select! {
                // CANCEL SAFETY: `mpsc::Receiver::recv` is cancel safe.
                message = response_rx.recv() => {
                    let Some(message) = message else { break };
                    // Queue everything already waiting (typically all the RTP
                    // packets of a frame) and flush once, rather than one
                    // write syscall per packet.
                    let write = async {
                        outbound.feed(message).await?;
                        for _ in 1..MAX_BATCH {
                            match response_rx.try_recv() {
                                Ok(message) => outbound.feed(message).await?,
                                Err(_) => break,
                            }
                        }
                        outbound.flush().await
                    };
                    match tokio::time::timeout(WRITE_TIMEOUT, write).await {
                        Ok(Ok(())) => {},
                        Ok(Err(Error::Io(err))) if err.kind() == ErrorKind::ConnectionReset
                            || err.kind() == ErrorKind::BrokenPipe => {
                            disconnected = true;
                            tracing::info!(%id, %addr, "connection: client disconnected (reset)");
                            break;
                        },
                        Ok(Err(err)) => {
                            tracing::error!(%err, %id, %addr, "connection: failed to send message");
                            break;
                        },
                        Err(_) => {
                            disconnected = true;
                            tracing::warn!(%id, %addr, "connection: write stalled for {:?}; dropping client", WRITE_TIMEOUT);
                            break;
                        },
                    }
                },
                // CANCEL SAFETY: `StreamExt:next` is always cancel safe.
                request = inbound.next() => {
                    match request {
                        Some(Ok(request)) => {
                            match request {
                                RequestMaybeInterleaved::Message(request) => {
                                    let response = handler.handle(&request, &response_tx).await;
                                    let response = ResponseMaybeInterleaved::Message(response);
                                    match tokio::time::timeout(WRITE_TIMEOUT, outbound.send(response))
                                        .await
                                        .unwrap_or_else(|_| Err(Error::Io(ErrorKind::TimedOut.into())))
                                    {
                                        Ok(()) => {},
                                        Err(Error::Io(err)) if err.kind() == ErrorKind::ConnectionReset => {
                                            disconnected = true;
                                            tracing::info!(%id, %addr, "connection: client disconnected (reset)");
                                            break;
                                        },
                                        Err(err) => {
                                            tracing::error!(%err, %id, %addr, "connection: failed to send response");
                                            break;
                                        },
                                    }
                                },
                                RequestMaybeInterleaved::Interleaved { channel, .. } => {
                                    tracing::debug!(%id, %addr, %channel, "ignored request with interleaved data");
                                },
                            }
                        },
                        None => {
                            disconnected = true;
                            tracing::info!(%id, %addr, "connection: client disconnected");
                            break;
                        },
                        Some(Err(Error::Io(err))) if err.kind() == ErrorKind::ConnectionReset => {
                            disconnected = true;
                            tracing::info!(%id, %addr, "connection: client disconnected (reset)");
                            break;
                        },
                        Some(Err(err)) => {
                            tracing::error!(%err, %id, %addr, "connection: failed to read request");
                            break;
                        },
                    }
                },
                // CANCEL SAFETY: `TaskContext::wait_for_stop` is cancel safe.
                _ = task_context.wait_for_stop() => {
                    tracing::trace!(%id, %addr, "connection worker stopping");
                    break;
                },
            };
        }

        if disconnected {
            // Client disconnected.
            let _ = state_tx.send(ConnectionState::Disconnected(id));
        } else {
            // Reason for breaking out of loop was unexpected and not due to the
            // client disconnecting.
            let _ = state_tx.send(ConnectionState::Closed(id));
        }
        tracing::trace!(%id, %addr, "connection worker EOL");
    }
}

#[derive(Copy, Clone, Eq, PartialEq, Hash)]
pub struct ConnectionId(usize);

impl ConnectionId {
    pub fn new(id: usize) -> Self {
        Self(id)
    }
}

impl fmt::Display for ConnectionId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

pub struct ConnectionIdGenerator(usize);

impl ConnectionIdGenerator {
    pub fn new() -> Self {
        ConnectionIdGenerator(0)
    }

    pub fn generate(&mut self) -> ConnectionId {
        let id = self.0;
        self.0 += 1;
        ConnectionId::new(id)
    }
}

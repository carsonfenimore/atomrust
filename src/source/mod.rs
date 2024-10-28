pub mod source_manager;

use tokio::select;
use tokio::sync::broadcast;
use tokio::sync::mpsc;

use crate::media;

use video_rs as video;

use crate::runtime::task_manager::{Task, TaskContext};
use crate::runtime::Runtime;

pub enum SourceState {
    Stopped(SourcePath),
}

pub type SourceStateTx = mpsc::UnboundedSender<SourceState>;
pub type SourceStateRx = mpsc::UnboundedReceiver<SourceState>;

pub type SourceMediaInfoTx = broadcast::Sender<media::MediaInfo>;
pub type SourceMediaInfoRx = broadcast::Receiver<media::MediaInfo>;

pub type SourceResetTx = broadcast::Sender<media::MediaInfo>;
pub type SourceResetRx = broadcast::Receiver<media::MediaInfo>;

use crate::libcam::PacketTx;
use crate::libcam::PacketRx;
use crate::media::StreamInfo;


pub enum SourceControlMessage {
    StreamInfo,
}

pub type SourceControlTx = mpsc::UnboundedSender<SourceControlMessage>;
pub type SourceControlRx = mpsc::UnboundedReceiver<SourceControlMessage>;

pub struct Source {
    // pub name: String,
    // pub path: SourcePath,
    control_tx: SourceControlTx,
    media_info_tx: SourceMediaInfoTx,
    reset_tx: SourceResetTx,
    packet_tx: PacketTx,
    worker: Task,
}

impl Source {
    /// Any more than 16 media/stream info messages on the queue probably means
    /// something is really wrong and the server is overloaded.
    const MAX_QUEUED_INFO: usize = 16;

    pub async fn start(
        name: &str,
        path: SourcePath,
        packet_tx: PacketTx,
        stream_info: StreamInfo,
        state_tx: SourceStateTx,
        runtime: &Runtime,
    ) -> Result<Self, video::Error> {
        let path = normalize_path(path);

        let (control_tx, control_rx) = mpsc::unbounded_channel();
        let (media_info_tx, _) = broadcast::channel(Self::MAX_QUEUED_INFO);
        let (reset_tx, _) = broadcast::channel(Self::MAX_QUEUED_INFO);

        tracing::trace!(name, %path, "starting source");
        let worker = runtime
            .task()
            .spawn({
                let path = path.clone();
                let media_info_tx = media_info_tx.clone();
                let reset_tx = reset_tx.clone();
                move |task_context| {
                    Self::run(
                        path,
                        control_rx,
                        state_tx,
                        media_info_tx,
                        reset_tx,
                        stream_info,
                        task_context,
                    )
                }
            })
            .await;
        tracing::trace!(name, %path, "started source");

        Ok(Self {
            control_tx,
            media_info_tx,
            reset_tx,
            packet_tx,
            worker,
        })
    }

    pub async fn stop(&mut self) {
        tracing::trace!("sending stop signal to source");
        self.worker.stop().await;
        tracing::trace!("stopped source");
    }

    pub fn delegate(&mut self) -> SourceDelegate {
        SourceDelegate {
            control_tx: self.control_tx.clone(),
            media_info_rx: self.media_info_tx.subscribe(),
            reset_rx: self.reset_tx.subscribe(),
            packet_rx: self.packet_tx.subscribe(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn run(
        path: SourcePath,
        mut control_rx: SourceControlRx,
        state_tx: SourceStateTx,
        media_info_tx: SourceMediaInfoTx,
        _reset_tx: SourceResetTx, // todo: i think this is irrelevant for a live stream...
        stream_info: StreamInfo,
        mut task_context: TaskContext,
    ) {

        tracing::info!("Starting rpicam stream");
        loop {
            select! {
                // CANCEL SAFETY: `mpsc::UnboundedReceiver::recv` is cancel safe.
                message = control_rx.recv() => {
                    match message {
                        Some(SourceControlMessage::StreamInfo) => {
                            let _ = media_info_tx.send(media::MediaInfo::from_stream_info(stream_info.clone()));
                        },
                        None => {
                            tracing::error!(%path, "source control channel broke unexpectedly");
                            break ;
                        },
                    };
                },
                // CANCEL SAFETY: `TaskContext::wait_for_stop` is cancel safe.
                _ = task_context.wait_for_stop() => {
                    tracing::trace!(%path, "stopping source");
                    break;
                },
            }
        }
        let _ = state_tx.send(SourceState::Stopped(path));
    }
}

pub struct SourceDelegate {
    control_tx: SourceControlTx,
    media_info_rx: SourceMediaInfoRx,
    reset_rx: SourceResetRx,
    packet_rx: PacketRx,
}

impl SourceDelegate {
    pub async fn query_media_info(&mut self) -> Option<media::MediaInfo> {
        if let Ok(()) = self.control_tx.send(SourceControlMessage::StreamInfo) {
            self.media_info_rx.recv().await.ok()
        } else {
            None
        }
    }

    pub fn into_parts(self) -> (SourceResetRx, PacketRx) {
        (self.reset_rx, self.packet_rx)
    }
}

pub type SourcePath = String;
pub type SourcePathRef = str;

pub fn normalize_path(path: SourcePath) -> SourcePath {
    if path.starts_with('/') {
        path
    } else {
        format!("/{}", &path)
    }
}

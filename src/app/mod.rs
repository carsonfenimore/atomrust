pub mod config;
pub mod handler;

use std::error::Error;
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::runtime::task_manager::Task;
use crate::app::config::AppConfig;
use crate::app::handler::AppHandler;
use crate::net::server::Server;
use crate::runtime::Runtime;
use tokio::time::timeout;
use std::time::Duration;


use crate::session::session_manager::SessionManager;
use crate::source::source_manager::SourceManager;
use crate::libcam::LibCamContext;
use crate::libcam::PacketTx;
use crate::libcam::DetectionRx;
use crate::pipeline::summarize_detections;
use crate::pipeline::Detections;
use crate::media::StreamInfo;
use crate::hamqtt::HAMQTTClient;

macro_rules! handle_err {
    ($rt:ident, $expr:expr) => {
        match $expr {
            Ok(ret) => Ok(ret),
            Err(err) => {
                $rt.stop().await;
                Err(err)
            }
        }
    };
}

pub struct App {
    server: Server,
    context: Arc<RwLock<AppContext>>,
    runtime: Arc<Runtime>,
    libcam: LibCamContext,
    hamqtt: Arc<HAMQTTClient>,
}

impl App {
    pub async fn start(config: AppConfig) -> Result<App, Box<dyn Error>> {
        let runtime = Arc::new(Runtime::new());

        let mut libcam = LibCamContext::new(&config.camera, &config.pipeline );
        libcam.client.start(true);
        let stream_info = handle_err!(
            runtime,
            libcam.delegate_stream_info().recv().await
        )?;
        tracing::debug!("In app setting up sources; libcam index is {}\n", stream_info.index);

        let obj_detections = libcam.delegate_detection();
        tracing::debug!("In app setting up sources; libcam index is {}\n", stream_info.index);
            
        let mut context = initialize_context(runtime.clone()).await;
        handle_err!(
            runtime,
            register_sources_with_context(&config, 
                                          &mut context,
                                          stream_info,
                                          libcam.packet_tx.clone() ).await
        )?;

        let context = Arc::new(RwLock::new(context));
        let server = handle_err!(
            runtime,
            initialize_server(&config, context.clone(), runtime.clone(),).await
        )?;

        let ha_conf = &config.mqtt;
        let hamqtt = Arc::new(HAMQTTClient::new(ha_conf.host.as_str(), ha_conf.port, ha_conf.username.as_str(), ha_conf.password.as_str())?);

        handle_err!(
            runtime,
            create_mqtt_publisher(runtime.clone(), config.mqtt.obj_name.clone(), hamqtt.clone(), obj_detections).await
        )?;

        Ok(Self {
            server,
            context,
            runtime,
            libcam,
            hamqtt,
        })
    }


    pub async fn stop(&mut self) {
        self.server.stop().await;
        self.context.write().await.session_manager.stop().await;
        self.context.write().await.source_manager.stop().await;
        self.runtime.stop().await;
        self.libcam.stop();
    }
}

async fn run_mqtt_publish(objname: String,  mqtt: Arc<HAMQTTClient>, mut detrx: DetectionRx) {
    const OBJDET_TIMEOUT_MILLIS:u64 = 5000;
    loop {
        // After 5 seconds we just say no obj... in this way objdets clear...
        match timeout(Duration::from_millis(OBJDET_TIMEOUT_MILLIS), detrx.recv()).await {
            Ok(cmd) => {
                    let dets = cmd.unwrap();

                    let num_dets = dets.len();
                    tracing::debug!("Received {} detections", num_dets);
                    let _ = mqtt.publish(&objname, "objdet_total_objects", num_dets, "", "");

                    // Report classes
                    for (det_class, det_count) in summarize_detections(&dets) {
                        let _ = mqtt.publish(&objname, format!("objdet_{}", det_class.as_str()).as_str(), det_count, "", "");
                    }
                }
            _ => {
                tracing::debug!("Timeout waiting for objdet");
                let _ = mqtt.publish(&objname, "objdet_total_objects", 0, "", "");
                const DETS: Detections = Detections::new();
                for (det_class, _det_count) in summarize_detections(&DETS) {
                    let _ = mqtt.publish(&objname, format!("objdet_{}", det_class.as_str()).as_str(), 0, "", "");
                }
            },
        };
    }
}

async fn create_mqtt_publisher(runtime: Arc<Runtime>, objname: String, mqtt: Arc<HAMQTTClient>, detrx: DetectionRx) -> Result<Task, Box<dyn Error>> {
    let worker = runtime
        .task()
        .spawn({
            |task_context| {
                run_mqtt_publish( objname,
                    mqtt,
                    detrx )
            }
        })
    .await;

    Ok(worker)
}

async fn initialize_server(
    config: &AppConfig,
    context: Arc<RwLock<AppContext>>,
    runtime: Arc<Runtime>,
) -> Result<Server, Box<dyn Error>> {
    let handler = AppHandler::new(context.clone());
    Server::start(
        config.server.host.parse()?,
        config.server.port,
        handler,
        runtime.clone(),
    )
    .await
    .map_err(|err| err.into())
}

async fn initialize_context(runtime: Arc<Runtime>) -> AppContext {
    AppContext {
        source_manager: SourceManager::start(runtime.clone()).await,
        session_manager: SessionManager::start(runtime.clone()).await,
    }
}


async fn register_sources_with_context(
    config: &AppConfig,
    context: &mut AppContext,
    stream_info: StreamInfo,
    packet_tx: PacketTx,
) -> Result<(), Box<dyn Error>> {

    // session already waits for source_packet_rx...
    // if we simply pass the receive from libcam instead, we should be gtg
    tracing::info!(%config.camera, "registering source");
    context
        .source_manager
        .register_and_start(
            "rpicam",
            config.camera.rtsppath.clone(),
            stream_info,
            packet_tx
        )
        .await?;
    tracing::trace!("registered cam");
    Ok(())
}

pub struct AppContext {
    source_manager: SourceManager,
    session_manager: SessionManager,
}

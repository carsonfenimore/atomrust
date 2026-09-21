pub mod config;
pub mod handler;
mod sys_stats;

use std::error::Error;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use tokio::sync::RwLock;
use tokio::select;

use crate::runtime::task_manager::Task;
use crate::app::config::AppConfig;
use crate::app::handler::AppHandler;
use crate::net::server::Server;
use crate::runtime::Runtime;
use tokio::sync::broadcast::error::RecvError;
use tokio::time::timeout;
use tokio::time::sleep;
use std::time::Duration;
use std::time::Instant;


use crate::session::session_manager::SessionManager;
use crate::source::source_manager::SourceManager;
use crate::libcam::LibCamContext;
use crate::libcam::{mono_millis, FrameClock};
use crate::libcam::PacketTx;
use crate::libcam::RateRx;
use crate::libcam::DetectionRx;
use crate::pipeline::summarize_detections;
use crate::pipeline::Detections;
use crate::media::StreamInfo;
use crate::hamqtt::HAMQTTClient;
use config::MQTTConfig;
use sys_stats::SysStats;

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
    hamqtt: Option<Arc<HAMQTTClient>>,
}

impl App {
    pub async fn start(config: AppConfig) -> Result<App, Box<dyn Error>> {
        let runtime = Arc::new(Runtime::new());

        let mut libcam = LibCamContext::new(&config.camera, config.pipeline.as_ref())?;
        // Subscribe before starting the camera so the one-shot stream info
        // message can't be sent before anyone is listening.
        let mut stream_info_rx = libcam.delegate_stream_info();
        libcam.client.start(true);
        let stream_info = match timeout(STREAM_INFO_TIMEOUT, stream_info_rx.recv()).await {
            Ok(Ok(stream_info)) => stream_info,
            Ok(Err(err)) => {
                runtime.stop().await;
                return Err(err.into());
            }
            Err(_) => {
                runtime.stop().await;
                return Err(format!(
                    "no H.264 stream from the camera after {:?}; is a camera connected and detected (rpicam-hello --list-cameras)?",
                    STREAM_INFO_TIMEOUT
                ).into());
            }
        };
        spawn_frame_watchdog(libcam.last_frame.clone());

        let obj_detections = libcam.delegate_detection();
        let (lowres_rate_rx, h264_rate_rx) = libcam.delegate_rate();
            
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


        let mut hamqtt: Option<Arc<HAMQTTClient>> = None;
        if let Some(ha_conf) = config.mqtt.as_ref() {
            tracing::info!("Constructing MQTT client to {}:{}", ha_conf.host.as_str(), ha_conf.port);
            let thamqtt = Arc::new(HAMQTTClient::new(ha_conf.host.as_str(), ha_conf.port, ha_conf.username.as_str(), ha_conf.password.as_str())?);

            handle_err!(
                runtime,
                create_mqtt_publisher(runtime.clone(), ha_conf.obj_name.clone(), thamqtt.clone(), obj_detections).await
            )?;

            handle_err!(
                runtime,
                create_periodic_mqtt_publisher(runtime.clone(), ha_conf.obj_name.clone(), thamqtt.clone()).await
            )?;

            handle_err!(
                runtime,
                create_rate_publisher(runtime.clone(), ha_conf.obj_name.clone(), thamqtt.clone(), lowres_rate_rx, h264_rate_rx).await
            )?;

            hamqtt = Some(thamqtt);
        } else {
            tracing::info!("MQTT not specified - NOT Constructing MQTT client");
        }

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

/// How long to wait for the first H.264 frames at startup.
const STREAM_INFO_TIMEOUT: Duration = Duration::from_secs(20);
/// With no H.264 frames for this long the camera pipeline is considered wedged.
const FRAME_STALL_TIMEOUT: Duration = Duration::from_secs(15);

/// libcamlite restarts the camera on a libcamera timeout, but if frames stop
/// for any other reason nothing recovers. Exit so systemd restarts us cleanly.
fn spawn_frame_watchdog(last_frame: FrameClock) {
    tokio::spawn(async move {
        loop {
            sleep(Duration::from_secs(5)).await;
            let last = last_frame.load(Ordering::Relaxed);
            let now = mono_millis();
            if last != 0 && now.saturating_sub(last) > FRAME_STALL_TIMEOUT.as_millis() as u64 {
                tracing::error!(
                    "no camera frames for {}s; exiting so the service manager can restart us",
                    now.saturating_sub(last) / 1000
                );
                std::process::exit(2);
            }
        }
    });
}

async fn run_mqtt_publish(objname: String,  mqtt: Arc<HAMQTTClient>, mut detrx: DetectionRx) {
    const OBJDET_TIMEOUT_MILLIS:u64 = 5000;
    loop {
        // After 5 seconds we just say no obj... in this way objdets clear...
        match timeout(Duration::from_millis(OBJDET_TIMEOUT_MILLIS), detrx.recv()).await {
            Ok(Err(RecvError::Lagged(_))) => continue,
            Ok(Err(RecvError::Closed)) => {
                tracing::debug!("detection channel closed; stopping objdet publisher");
                return;
            }
            Ok(Ok(dets)) => {
                {
                    let num_dets = dets.len();
                    tracing::debug!("Received {} detections", num_dets);
                    let _ = mqtt.publish(&objname, "objdet_total_objects", num_dets, "", "").await;

                    // Report classes
                    for (det_class, det_count) in summarize_detections(&dets) {
                        let _ = mqtt.publish(&objname, format!("objdet_{}", det_class.as_str()).as_str(), det_count, "", "").await;
                    }
                }
            }
            _ => {
                tracing::debug!("Timeout waiting for objdet");
                let _ = mqtt.publish(&objname, "objdet_total_objects", 0, "", "").await;
                const DETS: Detections = Detections::new();
                for (det_class, _det_count) in summarize_detections(&DETS) {
                    let _ = mqtt.publish(&objname, format!("objdet_{}", det_class.as_str()).as_str(), 0, "", "").await;
                }
            },
        };
    }
}

async fn run_mqtt_rate_publish(objname: String,  mqtt: Arc<HAMQTTClient>, mut lowres_rate: RateRx, mut h264_rate: RateRx) {
    loop {
        select! {
            rxcount = lowres_rate.recv() => {
                match rxcount {
                    Ok(lowrescount) => {
                        tracing::debug!("Got framecount on lowres: {}", lowrescount);
                        let _ = mqtt.publish(&objname, "framecount_objdet", lowrescount, "", "").await;
                    }
                    Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => return,
                }
            },
            h264rxcount = h264_rate.recv() => {
                match h264rxcount {
                    Ok(h264count) => {
                        tracing::debug!("Got framecount on h264: {}", h264count);
                        let _ = mqtt.publish(&objname, "framecount_h264", h264count, "", "").await;
                    }
                    Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => return,
                }
            },
        };
    }
}

async fn run_periodic_mqtt_publish(objname: String,  mqtt: Arc<HAMQTTClient>) {
    const PERIODIC_PUBLISH_PERIOD:u64 = 5000;
    const CPU_TEMP_PATH: &str = "/sys/class/thermal/thermal_zone0/temp";

    let now = Instant::now();
    let mut sys = SysStats::new();
    loop {
        sleep(Duration::from_millis(PERIODIC_PUBLISH_PERIOD)).await;
        // TODO: put this into some kind of a pi_stats_publish class

        // cpu temp
        if let Some(cpu_temp) = read_trimmed(CPU_TEMP_PATH).await.and_then(|t| t.parse::<u32>().ok()) {
            let _ = mqtt.publish(&objname, "temperature_cpu", (cpu_temp as f32 / 1000.0) as u8, "temperature", "°C").await;
        }

        // cpu load (1 minute)
        if let Some(load1min) = read_trimmed("/proc/loadavg").await
            .and_then(|l| l.split_whitespace().next().and_then(|v| v.parse::<f32>().ok()))
        {
            let _ = mqtt.publish(&objname, "load_cpu", load1min, "", "").await;
        }

        // wireless
        if let Some(link) = wifi_link_info().await {
            for line_raw in link.lines() {
                let line_parts: Vec<&str> = line_raw.split_whitespace().collect();
                match line_parts.as_slice() {
                    ["rx", "bitrate:", value, unit, ..] => {
                        let _ = mqtt.publish(&objname, "wifi_rx_bitrate", *value, "", unit).await;
                    }
                    ["tx", "bitrate:", value, unit, ..] => {
                        let _ = mqtt.publish(&objname, "wifi_tx_bitrate", *value, "", unit).await;
                    }
                    ["signal:", value, unit, ..] => {
                        let _ = mqtt.publish(&objname, "wifi_signal", *value, "", unit).await;
                    }
                    _ => {}
                }
            }
        }

        // uptime
        let elapsed_time = now.elapsed();
        let _ = mqtt.publish(&objname, "uptime", elapsed_time.as_secs(), "", "s").await;

        sys.update();
        // mem free
        let _ = mqtt.publish(&objname, "mem_free", sys.mem_free, "", "%").await;
        // disk 
        let _ = mqtt.publish(&objname, "disk_available", sys.disk_avail, "", "%").await;
        // net
        for (int_name, tx, rx) in &sys.net_rate {
            let _ = mqtt.publish(&objname, format!("net_{}_tx", int_name).as_str(), tx, "", "B/s").await;
            let _ = mqtt.publish(&objname, format!("net_{}_rx", int_name).as_str(), rx, "", "B/s").await;
        }
    }
}

async fn read_trimmed(path: &str) -> Option<String> {
    tokio::fs::read_to_string(path).await.ok().map(|s| s.trim().to_string())
}

/// `iw dev wlan0 link`, or None on wired-only boards / when iw is missing.
async fn wifi_link_info() -> Option<String> {
    let output = tokio::process::Command::new("iw")
        .args(["dev", "wlan0", "link"])
        .output()
        .await
        .ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

async fn create_rate_publisher(runtime: Arc<Runtime>, objname: String, mqtt: Arc<HAMQTTClient>, lowres_rate: RateRx, h264_rate: RateRx) -> Result<Task, Box<dyn Error>> {
    let worker = runtime
        .task()
        .spawn({
            |_task_context| {
                run_mqtt_rate_publish( objname,
                    mqtt, lowres_rate, h264_rate )
            }
        })
    .await;

    Ok(worker)
}

async fn create_periodic_mqtt_publisher(runtime: Arc<Runtime>, objname: String, mqtt: Arc<HAMQTTClient>) -> Result<Task, Box<dyn Error>> {
    let worker = runtime
        .task()
        .spawn({
            |_task_context| {
                run_periodic_mqtt_publish( objname,
                    mqtt )
            }
        })
    .await;

    Ok(worker)
}

async fn create_mqtt_publisher(runtime: Arc<Runtime>, objname: String, mqtt: Arc<HAMQTTClient>, detrx: DetectionRx) -> Result<Task, Box<dyn Error>> {
    let worker = runtime
        .task()
        .spawn({
            |_task_context| {
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
    tracing::info!("Constructing RTSP server on {}:{}", config.server.host, config.server.port);
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

use std::fmt;
use std::path::Path;

use serde::Deserialize;

use config::{Config, ConfigError};

#[derive(Debug, Deserialize)]
pub struct AppConfig {
    pub server: Server,
    pub camera: Camera,
    pub mqtt: MQTTConfig,
    pub pipeline: PipelineConfig,
}

#[derive(Debug, Deserialize)]
pub struct PipelineConfig {
    pub model_filename: String,
    pub threshold: f32,
    pub label_filename: String,
    pub num_threads: u32,
}

#[derive(Debug, Deserialize)]
pub struct MQTTConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub obj_name: String,
}

#[derive(Debug, Deserialize)]
pub struct Server {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Deserialize)]
pub struct Camera {
    pub rtsppath: String,
    pub width: u32,
    pub height: u32,
    pub lowres_width: u32,
    pub lowres_height: u32,
    pub framerate: u8,
    pub bitrate: String,
    pub profile: String,
    pub intraperiod: u8,
}

impl fmt::Display for Camera {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "rpicam {}, {}x{}, {}fps, {} bitrate, {} profile, {} intra",
            self.rtsppath, self.width, self.height, self.framerate, self.bitrate, self.profile, self.intraperiod
        )
    }
}


impl Default for AppConfig {
    fn default() -> Self {
        Self {
            server: Server {
                host: "127.0.0.1".to_string(),
                port: 554,
            },
            camera: Camera {
                rtsppath: "/atomrust".to_string(),
                width: 1920,
                height: 1080,
                lowres_width: 300,
                lowres_height: 300,
                framerate: 30,
                bitrate: "2mbps".to_string(),
                profile: "main".to_string(),
                intraperiod: 5,
            },
            mqtt: MQTTConfig {
                host: "localhost".to_string(),
                port: 1883,
                username: "username".to_string(),
                password: "password".to_string(),
                obj_name: "atomcam".to_string(),
            },
            pipeline: PipelineConfig {
                model_filename: "ssd_mobilenet_v2_coco_quant_postprocess.tflite".to_string(),
                threshold: 0.6,
                label_filename: "coco_labels.txt".to_string(),
                num_threads: 2,
            },
        }
    }
}

impl AppConfig {
    pub fn from_file(path: &Path) -> Result<Self, ConfigError> {
        Config::builder()
            .add_source(config::File::from(path))
            .add_source(config::Environment::with_prefix("oddity"))
            .build()?
            .try_deserialize()
    }
}

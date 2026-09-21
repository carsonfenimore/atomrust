mod app;
mod media;
mod net;
mod runtime;
mod session;
mod source;
mod libcam;
mod hamqtt;
mod pipeline;

use std::env;
use std::error::Error;
use std::io::IsTerminal;
use std::path::Path;
use std::process;
use std::time::Duration;

use config::ConfigError;

use app::config::AppConfig;
use app::App;

use tokio::signal::ctrl_c;
use tokio::signal::unix::{signal, SignalKind};

use video_rs as video;

macro_rules! on_error_exit {
    ($expr:expr) => {
        match $expr {
            Ok(ret) => ret,
            Err(err) => {
                tracing::error!("{}", err);
                eprintln!("Error: {}", err);
                process::exit(1);
            }
        }
    };
}

const DEFAULT_CONFIG: &str = "/etc/atomrust/config.yml";

#[tokio::main]
async fn main() {
    on_error_exit!(initialize_tracing());
    on_error_exit!(initialize_media());

    let config = on_error_exit!(initialize_and_read_config());
    tracing::debug!(?config, "loaded config file");

    tracing::trace!("starting app");
    let mut app = on_error_exit!(App::start(config).await);
    tracing::trace!("started app");

    let mut sigterm = on_error_exit!(signal(SignalKind::terminate()));
    tracing::info!("running; waiting for SIGINT/SIGTERM");
    tokio::select! {
        res = ctrl_c() => on_error_exit!(res),
        _ = sigterm.recv() => {},
    }

    // libcamera can block forever on shutdown if the pipeline is wedged;
    // never let a clean stop hang past this.
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(8));
        eprintln!("shutdown timed out; exiting");
        process::exit(1);
    });

    tracing::info!("stopping app");
    app.stop().await;
    tracing::info!("stopped app");
}

fn initialize_tracing() -> Result<(), Box<dyn Error + Send + Sync>> {
    // LOG=debug etc. overrides; defaults to info. Under systemd, journald adds
    // its own timestamps and doesn't render ANSI colour codes.
    let filter = tracing_subscriber::EnvFilter::try_from_env("LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let interactive = std::io::stdout().is_terminal();
    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(interactive);
    if interactive {
        builder.try_init()
    } else {
        builder.without_time().try_init()
    }
}

fn initialize_media() -> Result<(), Box<dyn Error>> {
    video::init()
}

fn initialize_and_read_config() -> Result<AppConfig, ConfigError> {
    let config_file = env::args().nth(1).unwrap_or(DEFAULT_CONFIG.to_string());
    let config_file = Path::new(&config_file);
    tracing::trace!(config_file=%config_file.display(), "loading config");

    AppConfig::from_file(config_file)
}

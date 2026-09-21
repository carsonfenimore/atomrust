//! Async wrapper functions for [`video_rs::RtpMuxer`].

use tokio::task;

use video_rs as video;
use video_rs::rtp::RtpMuxerBuilder;

type Result<T> = std::result::Result<T, video::Error>;

pub async fn make_rtp_muxer_builder() -> Result<RtpMuxerBuilder> {
    task::spawn_blocking(RtpMuxerBuilder::new).await.unwrap()
}

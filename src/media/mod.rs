pub mod sdp;
pub mod video;

pub use video_rs::stream::StreamInfo;

pub use video_rs::Packet;

use std::fmt;
use std::path::PathBuf;

use video_rs::{Error, Location, Reader, Url};

type Result<T> = std::result::Result<T, Error>;

#[derive(Clone)]
pub enum MediaDescriptor {
    Stream(Url),
    File(PathBuf),
}

impl fmt::Display for MediaDescriptor {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            MediaDescriptor::File(path) => write!(f, "file: {}", path.display()),
            MediaDescriptor::Stream(url) => write!(f, "stream: {}", url)
        }
    }
}

impl From<MediaDescriptor> for Location {
    fn from(descriptor: MediaDescriptor) -> Self {
        match descriptor {
            MediaDescriptor::File(path) => Location::File(path),
            MediaDescriptor::Stream(url) => Location::Network(url),
        }
    }
}

#[derive(Clone)]
pub struct MediaInfo {
    pub streams: Vec<StreamInfo>,
}

impl MediaInfo {
    pub fn from_stream_info(stream_info: StreamInfo) -> Self {
        Self {
            streams: vec![stream_info],
        }
    }
}

#[derive(Clone)]
pub struct StreamState {
    pub rtp_seq: u16,
    pub rtp_timestamp: u32,
}

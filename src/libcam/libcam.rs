use ffmpeg_next as ffmpeg;
use ffmpeg::util::rational::Rational;
use ffmpeg::codec::Parameters;
use tokio::sync::broadcast;
use crate::libcam::timereporter::RateReporter;
use crate::app::config::Camera;
use ffmpeg_sys_next as ff;
use ffmpeg::codec::packet::Packet as AvPacket;

use rslibcamlitelib::{LibCamClient, StreamParams, StreamFormat, ExternalCallback};
use rslibcamlitelib::{begin_analysis, analyze};

use crate::media::StreamInfo;
pub type StreamTx = broadcast::Sender<StreamInfo>;
pub type StreamRx = broadcast::Receiver<StreamInfo>;

use crate::media::Packet;
pub type PacketTx = broadcast::Sender<Packet>;
pub type PacketRx = broadcast::Receiver<Packet>;

pub struct LibCamContext {
    pub client: LibCamClient,
    stream_tx: StreamTx,
    pub packet_tx: PacketTx,
}

pub struct LibCamCallback {
    lowres_params: StreamParams,
    h264_params: StreamParams,
    h264_reporter: RateReporter,
    low_reporter: RateReporter,
    stream_info_obj: *mut std::ffi::c_void,
    stream: *mut ff::AVStream,
    stream_tx: StreamTx,
    packet_tx: PacketTx,
}

impl LibCamContext {
    const MAX_QUEUED_PACKETS: usize = 1024;
    pub fn new(camera: &Camera) -> Self {
        let libcam = LibCamClient::new();
        let (stream_tx, _) = broadcast::channel(Self::MAX_QUEUED_PACKETS);
        let (packet_tx, _) = broadcast::channel(Self::MAX_QUEUED_PACKETS);
        let callback = LibCamCallback::new(&libcam, &camera, stream_tx.clone(), packet_tx.clone());
        libcam.setCallbacks(callback);

        Self { client: libcam, 
               stream_tx: stream_tx,
               packet_tx: packet_tx}
    }

    pub fn delegate_stream_info(&mut self) -> StreamRx {
        self.stream_tx.subscribe()
    }

    pub fn stop(&self){
        // TODO: implement stop
        //self.client.stop();
    }

}


impl LibCamCallback {

    pub fn new(libcam: &LibCamClient, config: &Camera, streamtx: StreamTx, packettx: PacketTx) -> Box<Self> {
        //let lowres = StreamParams{ width: 300, height: 300, format: StreamFormat::STREAM_FORMAT_RGB, framerate: 30};
        //let h264_params = StreamParams{ width: 1920, height: 1080, format:  StreamFormat::STREAM_FORMAT_H264, framerate: 30};

        let h264_params = StreamParams{ width: config.width, height: config.height, format:  StreamFormat::STREAM_FORMAT_H264, framerate: config.framerate};
        tracing::trace!("setup h264 {}x{}, framerate {}, profile {}, bitrate {}, intra {}", config.width, config.height, config.framerate, config.profile, config.bitrate, config.intraperiod);
        libcam.client.setupH264(&h264_params, config.intraperiod, &config.profile, &config.bitrate);

        let lowres = StreamParams{ width: config.lowres_width, height: config.lowres_height, format: StreamFormat::STREAM_FORMAT_RGB, framerate: config.framerate};
        tracing::trace!("trying lowres {}x{}", config.lowres_width, config.lowres_height);
        libcam.client.setupLowres(&lowres);

        let callback = Box::new(Self {
            lowres_params: lowres,
            h264_params: h264_params,
            h264_reporter: RateReporter::new(1.0, "h264"),
            low_reporter: RateReporter::new(1.0, "low"),
            stream_info_obj: unsafe { begin_analysis() },
            stream: std::ptr::null_mut(),
            stream_tx: streamtx,
            packet_tx: packettx,
        });

        callback
    }

}

impl ExternalCallback for LibCamCallback {
    unsafe fn callbackH264(&mut self, bytes: *mut u8, count: usize, _timestamp_us: i64, keyframe: bool ){
        tracing::trace!("Got h264 frame of {} bytes, keyframe {}, res {}x{}", count, keyframe, self.h264_params.width, self.h264_params.height);
        if self.stream.is_null(){
            self.stream = analyze(self.stream_info_obj, bytes, count);
            if ! self.stream.is_null() {
                tracing::trace!("Notifying app of streaminfo");
                let codepar = Parameters::wrap( (*self.stream).codecpar, None );
                let timebase = Rational::new( (*self.stream).time_base.num, (*self.stream).time_base.den );
                const STREAM_INDEX: usize = 0;
                let stream_info = StreamInfo::from_params(codepar, timebase, STREAM_INDEX).unwrap();
                let _ = self.stream_tx.send(stream_info);
            }
        } 

        if ! self.stream.is_null(){
            let timebase = Rational::new( (*self.stream).time_base.num, (*self.stream).time_base.den );
            let bytesvec = unsafe { std::slice::from_raw_parts(bytes, count) };
            let _ = self.packet_tx.send(Packet::new( AvPacket::copy(bytesvec), timebase));
        }
        self.h264_reporter.tick();
    }
    unsafe fn callbackLowres(&mut self, bytes: *mut u8, count: usize){
        tracing::trace!("Got h264 frame of {} bytes, res {}x{}", count, self.lowres_params.width, self.lowres_params.height);
        self.low_reporter.tick();
    }
}

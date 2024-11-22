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

use crate::media::video::times::Times;
use video_rs as video;

use crate::pipeline::TFLiteStage;
use crate::pipeline::Detections;
use crate::app::config::PipelineConfig;

pub type DetectionTx = broadcast::Sender<Detections>;
pub type DetectionRx = broadcast::Receiver<Detections>;

pub type RateTx = broadcast::Sender<f32>;
pub type RateRx = broadcast::Receiver<f32>;

pub struct LibCamContext {
    pub client: LibCamClient,
    stream_tx: StreamTx,
    pub packet_tx: PacketTx,
    detection_tx: DetectionTx,
    lowres_rate_tx: RateTx,
    h264_rate_tx: RateTx,
}

pub struct LibCamCallback<'a> {
    lowres_params: StreamParams,
    h264_params: StreamParams,
    h264_reporter: RateReporter,
    low_reporter: RateReporter,
    stream_info_obj: *mut std::ffi::c_void,
    stream: *mut ff::AVStream,
    stream_tx: StreamTx,
    packet_tx: PacketTx,
    times: Times,
    timebase: Rational,
    tflite: TFLiteStage<'a>,
    detection_tx: DetectionTx,
    lowres_rate_tx: RateTx,
    h264_rate_tx: RateTx,
}

impl LibCamContext {
    const MAX_QUEUED_PACKETS: usize = 30;
    const MAX_QUEUED_DETECTIONS : usize = 5;
    pub fn new(camera: &Camera, pipeline: &PipelineConfig) -> Self {
        let libcam = LibCamClient::new();
        let (stream_tx, _) = broadcast::channel(Self::MAX_QUEUED_PACKETS);
        let (packet_tx, _) = broadcast::channel(Self::MAX_QUEUED_PACKETS);
        let (detection_tx, _) = broadcast::channel(Self::MAX_QUEUED_DETECTIONS);
        let (lowres_rate_tx, _) = broadcast::channel(1);
        let (h264_rate_tx, _) = broadcast::channel(1);
        let callback = LibCamCallback::new(&libcam, &camera, &pipeline, 
                                            stream_tx.clone(), 
                                            packet_tx.clone(),
                                            detection_tx.clone(),
                                            lowres_rate_tx.clone(),
                                            h264_rate_tx.clone());
        libcam.setCallbacks(callback);

        Self { client: libcam, 
               stream_tx,
               packet_tx,
               detection_tx,
               lowres_rate_tx,
               h264_rate_tx,}
    }

    pub fn delegate_stream_info(&mut self) -> StreamRx {
        self.stream_tx.subscribe()
    }

    pub fn delegate_detection(&mut self) -> DetectionRx {
        self.detection_tx.subscribe()
    }

    pub fn delegate_rate(&mut self) -> (RateRx, RateRx) {
        let rxdet = self.lowres_rate_tx.subscribe();
        let h264rxdet = self.h264_rate_tx.subscribe();
        (rxdet, h264rxdet)
    }

    pub fn stop(&self){
        self.client.stop();
    }

}


impl<'a> LibCamCallback<'a> {

    pub fn new(libcam: &LibCamClient, config: &Camera, pipeline: &PipelineConfig, 
                streamtx: StreamTx, 
                packettx: PacketTx,
                detectiontx: DetectionTx,
                lowres_rate_tx: RateTx,
                h264_rate_tx: RateTx) -> Box<Self> {
        //let lowres = StreamParams{ width: 300, height: 300, format: StreamFormat::STREAM_FORMAT_RGB, framerate: 30};
        //let h264_params = StreamParams{ width: 1920, height: 1080, format:  StreamFormat::STREAM_FORMAT_H264, framerate: 30};

        let h264_params = StreamParams{ width: config.width, height: config.height, format:  StreamFormat::STREAM_FORMAT_H264, framerate: config.framerate};
        tracing::trace!("setup h264 {}x{}, framerate {}, profile {}, bitrate {}, intra {}", config.width, config.height, config.framerate, config.profile, config.bitrate, config.intraperiod);
        libcam.client.setupH264(&h264_params, config.intraperiod, &config.profile, &config.bitrate);

        let lowres = StreamParams{ width: config.lowres_width, height: config.lowres_height, format: StreamFormat::STREAM_FORMAT_RGB, framerate: config.framerate};
        tracing::trace!("trying lowres {}x{}", config.lowres_width, config.lowres_height);
        libcam.client.setupLowres(&lowres);

        let tflite = TFLiteStage::new(&pipeline, &config.lowres_width, &config.lowres_height).unwrap();
        let callback = Box::new(Self {
            lowres_params: lowres,
            h264_params: h264_params,
            h264_reporter: RateReporter::new(1.0, "h264"),
            low_reporter: RateReporter::new(1.0, "low"),
            stream_info_obj: unsafe { begin_analysis() },
            stream: std::ptr::null_mut(),
            stream_tx: streamtx,
            packet_tx: packettx,
            times: Times::new(),
            timebase: Rational::new( 1000, config.framerate as i32 * 1000 ),
            tflite,
            detection_tx: detectiontx,
            lowres_rate_tx,
            h264_rate_tx,
        });

        callback
    }

}

impl<'a> ExternalCallback for LibCamCallback<'a> {
    unsafe fn callbackH264(&mut self, bytes: *mut u8, count: usize, _timestamp_us: i64, keyframe: bool ){
        tracing::trace!("Got h264 frame of {} bytes, keyframe {}, res {}x{}", count, keyframe, self.h264_params.width, self.h264_params.height);
        if self.stream.is_null(){
            self.stream = analyze(self.stream_info_obj, bytes, count);
            if ! self.stream.is_null() {
                tracing::trace!("Notifying app of streaminfo");
                let codepar = Parameters::wrap( (*self.stream).codecpar, None );
                const STREAM_INDEX: usize = 0;
                let stream_info = StreamInfo::from_params(codepar, self.timebase, STREAM_INDEX).unwrap();
                let _ = self.stream_tx.send(stream_info);
            }
        } 

        if ! self.stream.is_null(){
            let bytesvec = unsafe { std::slice::from_raw_parts(bytes, count) };
            let mut pkt = Packet::new( AvPacket::copy(bytesvec), self.timebase);
            pkt.set_duration(video::Time::new(Some(1), self.timebase));
            self.times.update(&mut pkt);
            let _ = self.packet_tx.send(pkt);
        }
        if self.h264_reporter.isTimeToReport() {
            let _ = self.h264_rate_tx.send(self.h264_reporter.rate());
        }
        self.h264_reporter.tick();
    }
    unsafe fn callbackLowres(&mut self, bytes: *mut u8, count: usize){
        tracing::trace!("Got rgb frame; {} bytes, {}x{}", count, self.lowres_params.width, self.lowres_params.height);
        if self.low_reporter.isTimeToReport() {
            let _ = self.lowres_rate_tx.send(self.low_reporter.rate());
        }
        self.low_reporter.tick();

        let dets = self.tflite.detect( std::slice::from_raw_parts(bytes, count) ).unwrap();
        if dets.len() > 0 {
            let _ = self.detection_tx.send(dets);
        }
    }
}

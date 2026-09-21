use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use ffmpeg_next as ffmpeg;
use ffmpeg::util::rational::Rational;
use ffmpeg::codec::Parameters;
use tokio::sync::broadcast;
use crate::libcam::timereporter::RateReporter;
use crate::app::config::Camera;
use ffmpeg_sys_next as ff;
use ffmpeg::codec::packet::Packet as AvPacket;
use ffmpeg::codec::packet::Flags as AvPacketFlags;

use rslibcamlitelib::{LibCamClient, StreamParams, StreamFormat, ExternalCallback};
use rslibcamlitelib::{begin_analysis, analyze};

use crate::media::StreamInfo;
pub type StreamTx = broadcast::Sender<StreamInfo>;
pub type StreamRx = broadcast::Receiver<StreamInfo>;

use crate::media::Packet;
pub type PacketTx = broadcast::Sender<Packet>;
pub type PacketRx = broadcast::Receiver<Packet>;

use video_rs as video;

#[cfg(feature = "objdet")]
use crate::pipeline::TFLiteStage;
use crate::pipeline::Detections;
use crate::app::config::PipelineConfig;

pub type DetectionTx = broadcast::Sender<Detections>;
pub type DetectionRx = broadcast::Receiver<Detections>;

pub type RateTx = broadcast::Sender<f32>;
pub type RateRx = broadcast::Receiver<f32>;

/// `mono_millis()` of the last H.264 frame; 0 until the first frame. Used by
/// the app watchdog to detect a wedged camera pipeline.
pub type FrameClock = Arc<AtomicU64>;

/// Packets are timestamped in microseconds straight from the sensor; the RTP
/// muxer rescales to 90kHz.
const TIMEBASE_US: (i32, i32) = (1, 1_000_000);

pub struct LibCamContext {
    pub client: LibCamClient,
    stream_tx: StreamTx,
    pub packet_tx: PacketTx,
    detection_tx: DetectionTx,
    lowres_rate_tx: RateTx,
    h264_rate_tx: RateTx,
    pub last_frame: FrameClock,
}

struct H264State {
    reporter: RateReporter,
    stream_info_obj: *mut std::ffi::c_void,
    stream: *mut ff::AVStream,
    first_ts_us: Option<i64>,
    last_pts_us: i64,
    frame_duration_us: i64,
}

// SAFETY: the raw pointers are owned by libcamlite's StreamInfo, which lives for
// the life of the process and is only touched from behind the Mutex.
unsafe impl Send for H264State {}

struct LowresState {
    reporter: RateReporter,
    #[cfg(feature = "objdet")]
    tflite: Option<TFLiteStage<'static>>,
}

// SAFETY: the TFLite interpreter is only used from behind the Mutex, one frame
// at a time.
unsafe impl Send for LowresState {}

/// libcamlite calls `callbackH264` from the encoder output thread and
/// `callbackLowres` from a separate post-processing thread, possibly at the
/// same time, so each path keeps its own state behind its own lock.
pub struct LibCamCallback {
    h264_params: StreamParams,
    lowres_params: StreamParams,
    h264: Mutex<H264State>,
    lowres: Mutex<LowresState>,
    stream_tx: StreamTx,
    packet_tx: PacketTx,
    detection_tx: DetectionTx,
    lowres_rate_tx: RateTx,
    h264_rate_tx: RateTx,
    last_frame: FrameClock,
}

impl LibCamContext {
    const MAX_QUEUED_PACKETS: usize = 30;
    const MAX_QUEUED_DETECTIONS : usize = 5;
    pub fn new(camera: &Camera, pipeline: Option<&PipelineConfig>) -> Result<Self, Box<dyn std::error::Error>> {
        let libcam = LibCamClient::new();
        let (stream_tx, _) = broadcast::channel(Self::MAX_QUEUED_PACKETS);
        let (packet_tx, _) = broadcast::channel(Self::MAX_QUEUED_PACKETS);
        let (detection_tx, _) = broadcast::channel(Self::MAX_QUEUED_DETECTIONS);
        let (lowres_rate_tx, _) = broadcast::channel(1);
        let (h264_rate_tx, _) = broadcast::channel(1);
        let last_frame = Arc::new(AtomicU64::new(0));
        let callback = LibCamCallback::new(&libcam, &camera, pipeline,
                                            stream_tx.clone(),
                                            packet_tx.clone(),
                                            detection_tx.clone(),
                                            lowres_rate_tx.clone(),
                                            h264_rate_tx.clone(),
                                            last_frame.clone())?;
        libcam.setCallbacks(callback);

        Ok(Self { client: libcam,
               stream_tx,
               packet_tx,
               detection_tx,
               lowres_rate_tx,
               h264_rate_tx,
               last_frame,})
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

/// Monotonic milliseconds since process start (never 0). Wall-clock time is
/// unusable here: Pis have no RTC and NTP steps the clock after boot.
pub fn mono_millis() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64 + 1
}

impl LibCamCallback {

    #[allow(clippy::too_many_arguments)]
    pub fn new(libcam: &LibCamClient, config: &Camera, pipeline: Option<&PipelineConfig>,
                streamtx: StreamTx,
                packettx: PacketTx,
                detectiontx: DetectionTx,
                lowres_rate_tx: RateTx,
                h264_rate_tx: RateTx,
                last_frame: FrameClock) -> Result<Box<Self>, Box<dyn std::error::Error>> {
        let h264_params = StreamParams{ width: config.width, height: config.height, format:  StreamFormat::STREAM_FORMAT_H264, framerate: config.framerate};
        tracing::info!("setup h264 {}x{}, framerate {}, profile {}, bitrate {}, intra {}", config.width, config.height, config.framerate, config.profile, config.bitrate, config.intraperiod);
        libcam.client.setupH264(&h264_params, config.intraperiod, &config.profile, &config.bitrate);

        // The lowres RGB stream only feeds object detection; without a
        // pipeline, skip it so the ISP doesn't produce (and we don't
        // convert) frames nobody reads.
        let lowres = StreamParams{ width: config.lowres_width, height: config.lowres_height, format: StreamFormat::STREAM_FORMAT_RGB, framerate: config.framerate};
        if pipeline.is_some() {
            tracing::info!("setup lowres {}x{}", config.lowres_width, config.lowres_height);
            libcam.client.setupLowres(&lowres);
        }

        #[cfg(feature = "objdet")]
        let tflite = match pipeline {
            Some(pipeline) => Some(TFLiteStage::new(pipeline, &config.lowres_width, &config.lowres_height)?),
            None => {
                tracing::info!("Pipeline empty - not constructing TFLite pipeline");
                None
            }
        };
        #[cfg(not(feature = "objdet"))]
        if pipeline.is_some() {
            tracing::warn!("pipeline configured but atomrust was built without the `objdet` feature; ignoring");
        }

        let framerate = config.framerate.max(1) as i64;
        Ok(Box::new(Self {
            h264_params,
            lowres_params: lowres,
            h264: Mutex::new(H264State {
                reporter: RateReporter::new(1.0, "h264"),
                stream_info_obj: unsafe { begin_analysis() },
                stream: std::ptr::null_mut(),
                first_ts_us: None,
                last_pts_us: -1,
                frame_duration_us: 1_000_000 / framerate,
            }),
            lowres: Mutex::new(LowresState {
                reporter: RateReporter::new(1.0, "low"),
                #[cfg(feature = "objdet")]
                tflite,
            }),
            stream_tx: streamtx,
            packet_tx: packettx,
            detection_tx: detectiontx,
            lowres_rate_tx,
            h264_rate_tx,
            last_frame,
        }))
    }

}

impl ExternalCallback for LibCamCallback {
    unsafe fn callbackH264(&self, bytes: *mut u8, count: usize, timestamp_us: i64, keyframe: bool ){
        tracing::trace!("Got h264 frame of {} bytes, keyframe {}, res {}x{}", count, keyframe, self.h264_params.width, self.h264_params.height);
        self.last_frame.store(mono_millis(), Ordering::Relaxed);
        let Ok(mut st) = self.h264.lock() else { return };
        let timebase = Rational::new(TIMEBASE_US.0, TIMEBASE_US.1);

        if st.stream.is_null(){
            st.stream = analyze(st.stream_info_obj, bytes, count);
            if ! st.stream.is_null() {
                tracing::debug!("Notifying app of streaminfo");
                let codepar = Parameters::wrap( (*st.stream).codecpar, None );
                const STREAM_INDEX: usize = 0;
                match StreamInfo::from_params(codepar, timebase, STREAM_INDEX) {
                    Ok(stream_info) => { let _ = self.stream_tx.send(stream_info); }
                    Err(err) => tracing::error!(%err, "failed to build stream info from H.264 parameters"),
                }
            }
        }

        if ! st.stream.is_null(){
            // Use the sensor timestamp so RTP time tracks real time even when
            // frames are dropped or the sensor runs slower than configured.
            let first = *st.first_ts_us.get_or_insert(timestamp_us);
            let mut pts = timestamp_us - first;
            if pts <= st.last_pts_us {
                pts = st.last_pts_us + st.frame_duration_us;
            }
            st.last_pts_us = pts;

            let bytesvec = std::slice::from_raw_parts(bytes, count);
            let mut avpkt = AvPacket::copy(bytesvec);
            if keyframe {
                avpkt.set_flags(AvPacketFlags::KEY);
            }
            let mut pkt = Packet::new(avpkt, timebase);
            pkt.set_pts(video::Time::new(Some(pts), timebase));
            pkt.set_dts(video::Time::new(Some(pts), timebase));
            pkt.set_duration(video::Time::new(Some(st.frame_duration_us), timebase));
            let _ = self.packet_tx.send(pkt);
        }
        if st.reporter.isTimeToReport() {
            let _ = self.h264_rate_tx.send(st.reporter.rate());
        }
        st.reporter.tick();
    }

    unsafe fn callbackLowres(&self, bytes: *mut u8, count: usize){
        tracing::trace!("Got rgb frame; {} bytes, {}x{}", count, self.lowres_params.width, self.lowres_params.height);
        let Ok(mut st) = self.lowres.lock() else { return };
        if st.reporter.isTimeToReport() {
            let _ = self.lowres_rate_tx.send(st.reporter.rate());
        }
        st.reporter.tick();

        #[cfg(feature = "objdet")]
        if let Some(tflite) = st.tflite.as_mut() {
            match tflite.detect( std::slice::from_raw_parts(bytes, count) ) {
                Ok(dets) if !dets.is_empty() => { let _ = self.detection_tx.send(dets); }
                Ok(_) => {}
                Err(err) => tracing::warn!(%err, "object detection failed on frame"),
            }
        }
        #[cfg(not(feature = "objdet"))]
        let _ = (&self.detection_tx, bytes, count);
    }
}

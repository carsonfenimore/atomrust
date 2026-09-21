#[cfg(feature = "objdet")]
mod tflite;
#[cfg(feature = "objdet")]
pub use tflite::TFLiteStage;

mod summarize;
pub use summarize::DetectionSummary;
pub use summarize::summarize_detections;

#[derive(Clone)]
pub struct Detection {
    pub xmin: i32,
    pub ymin: i32,
    pub xmax: i32,
    pub ymax: i32,
    pub score: f32,
    pub class: String,
}

pub type Detections = Vec<Detection>;

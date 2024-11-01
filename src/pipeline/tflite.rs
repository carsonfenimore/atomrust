
use std::fs::{self, File};
use std::io::Read;

use tflite::ops::builtin::BuiltinOpResolver;
use tflite::{FlatBufferModel, InterpreterBuilder, Result};
use tflite::{Interpreter};
use crate::app::config::PipelineConfig;


pub struct TFLiteStage<'a> {
    interpreter: Interpreter<'a, BuiltinOpResolver>,
}

pub struct Detection {
    xmin: u32,
    ymin: u32,
    xmax: u32,
    ymax: u32,
    class_name: String,
}

pub type Detections = Vec<Detection>;



impl<'a> TFLiteStage<'a> {
    pub fn new(config: &PipelineConfig ) -> Result< Self > {
       let model = FlatBufferModel::build_from_file(&config.model_filename)?;
       let resolver = BuiltinOpResolver::default();
       let builder = InterpreterBuilder::new(model, resolver)?;
       let mut interpreter = builder.build()?;
       interpreter.allocate_tensors()?;
       let inputs = interpreter.inputs().to_vec();
       assert_eq!(inputs.len(), 1);
       let input_index = inputs[0];
       let outputs = interpreter.outputs().to_vec();
       assert_eq!(outputs.len(), 4);

       Ok( Self{
           interpreter
       })
    }

    pub fn detect(&mut self, rgb_bytes: &[u8]) -> Result< Detections >{
        let inputs = self.interpreter.inputs().to_vec();
        let input_index = inputs[0];
        let mut interp = self.interpreter.tensor_data_mut(input_index)?;
        let input_len_bytes = interp.len();
        // tODO: assert rgb_bytes == input_len_bytes
        interp[..input_len_bytes].copy_from_slice(&rgb_bytes[..input_len_bytes]);
        self.interpreter.invoke()?;

       let outputs = self.interpreter.outputs().to_vec();
       let locations: &[u8] = self.interpreter.tensor_data(outputs[0])?;
       let classes: &[u8] = self.interpreter.tensor_data(outputs[1])?;
       let scores: &[u8] = self.interpreter.tensor_data(outputs[2])?;
       let num_detections: &[u8] = self.interpreter.tensor_data(outputs[3])?;

       let detections = Detections::new();

       //  float y1 = heiScale * std::max(static_cast<float>(0.0), results[0][4 * i]);
       //  float x1 = widScale * std::max(static_cast<float>(0.0), results[0][4 * i + 1]);
       //  float y2 = heiScale * std::min(static_cast<float>(1.0), results[0][4 * i + 2]);
       //  float x2 = widScale * std::min(static_cast<float>(1.0), results[0][4 * i + 3]);

       Ok(detections)
    }
}

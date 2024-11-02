
use std::io::Read;

use tflite::ops::builtin::BuiltinOpResolver;
use tflite::{FlatBufferModel, InterpreterBuilder, Result};
use tflite::{Interpreter};
use crate::app::config::PipelineConfig;

use std::cmp::{max,min};
use std::collections::HashMap;
use std::fs::read_to_string;
use crate::Error;


pub struct TFLiteStage<'a> {
    interpreter: Interpreter<'a, BuiltinOpResolver>,
    labels: LabelParser,
    threshold: f32,
    width: u32,
    height: u32,
}

#[derive(Clone)]
pub struct Detection {
    xmin: i32,
    ymin: i32,
    xmax: i32,
    ymax: i32,
    score: f32,
    class: String,
}

pub type Detections = Vec<Detection>;

type LabelMap = HashMap<i32, String>;
struct LabelParser {
   labels: LabelMap,
}

impl LabelParser {
    pub fn new(filename: &String) -> Result<Self>{
        let lines:Vec<String> = read_to_string(filename) 
                            .unwrap()  
                            .lines()  
                            .map(String::from)  
                            .collect();
        let mut labels = LabelMap::new();
        for line in lines { 
           let parts: Vec<&str> = line.split_whitespace().collect();
           let class_num = parts[0].to_string().parse::<i32>().unwrap();
           let class = parts[1];
           labels.insert( class_num, class.to_string() );
        }
        Ok(Self { labels })
    }
    pub fn lookup(&self, class_num: i32) -> Option<&String> {
        self.labels.get(&class_num)
    }
    pub fn has(&self, class_num: i32) -> bool {
        self.labels.contains_key(&class_num)
    }
}


impl<'a> TFLiteStage<'a> {
    pub fn new(config: &PipelineConfig, width: &u32, height: &u32 ) -> Result< Self > {
       let model = FlatBufferModel::build_from_file(&config.model_filename)?;
       let resolver = BuiltinOpResolver::default();
       let builder = InterpreterBuilder::new(model, resolver)?;
       let mut interpreter = builder.build()?;
       interpreter.allocate_tensors()?;
       let inputs = interpreter.inputs().to_vec();
       assert_eq!(inputs.len(), 1);
       let input_index = inputs[0];
       let tinfos = interpreter.get_input_details()?;
       let expected_height = tinfos[0].dims[1] as u32;
       let expected_width = tinfos[0].dims[2] as u32;
       assert_eq!(&expected_width, width);
       assert_eq!(&expected_height, height);
       let outputs = interpreter.outputs().to_vec();
       assert_eq!(outputs.len(), 4);

       interpreter.set_num_threads(config.num_threads as i32); // so... you can have negative threads i see...

       let labels = LabelParser::new(&config.label_filename)?;
       Ok( Self{
           interpreter,
           labels,
           threshold: config.threshold,
           width:*width,
           height:*height,
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
       let locations:&[f32] = self.interpreter.tensor_data(outputs[0])?;
       let classes:&[f32] = self.interpreter.tensor_data(outputs[1])?;
       let scores:&[f32] = self.interpreter.tensor_data(outputs[2])?;
       let raw_num_detections:&[f32] = self.interpreter.tensor_data(outputs[3])?;
       let num_detections = raw_num_detections[0] as usize;

       let mut detections = Detections::new();

       let width:f32 = self.width as f32;
       let height:f32 = self.height as f32;
       // TODO: add nonmax suppression
       for index in 0..num_detections {
         let ymin = max(0, (height * locations[4 * index]) as i32);
         let xmin = max(0, (width * locations[4 * index + 1]) as i32);
         let ymax = min(height as i32 - 1, (height * locations[4 * index + 2]) as i32);
         let xmax = min(width as i32 - 1, (width * locations[4 * index + 3]) as i32);
         let class_num = classes[index] as i32;
         let class = if self.labels.has(class_num) { self.labels.lookup(class_num).unwrap().clone() } else {  "?".to_string() };
         let score = scores[index];
         if score > self.threshold {
             detections.push( Detection{ xmin, ymin, xmax, ymax, score, class } );
             tracing::trace!("det class {class_num} with score {score} at {xmin},{ymin} - {xmax},{ymax}");
         }
       }

       Ok(detections)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image;

    #[test]
    fn test_tflite(){
        let config = PipelineConfig{
                model_filename: "models/ssd_mobilenet_v2_coco_quant_postprocess.tflite".to_string(),
                threshold: 0.6,
                label_filename: "models/coco_labels.txt".to_string(),
                num_threads: 2,
                };
        let mut stage = TFLiteStage::new(&config, &300, &300).unwrap();
        let bytes = image::open("test_data/person.jpg").unwrap().into_bytes();
        let det = stage.detect(bytes.as_slice()).unwrap();
        assert_eq!(det.len(), 1);
        let result0 = &det[0];
        assert_eq!(result0.class, "person".to_string());
        assert!( result0.score > 0.98 );
        assert_eq!(result0.xmin, 126);
        assert_eq!(result0.ymin, 68);
        assert_eq!(result0.xmax, 245);
        assert_eq!(result0.ymax, 251);
    }
}


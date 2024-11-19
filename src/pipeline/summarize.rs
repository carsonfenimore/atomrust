use std::collections::HashMap;
use crate::pipeline::Detections;
use crate::pipeline::Detection;

// Returns <class, instance count>
pub type DetectionSummary = HashMap< String, u32 >;

pub fn summarize_detections(dets: &Detections) -> DetectionSummary {
    let mut dets_out = DetectionSummary::new();
    dets_out.insert("person".to_string(), 0);
    dets_out.insert("animal".to_string(), 0);
    dets_out.insert("vehicle".to_string(), 0);
    for det in dets {
        if det.class == "person" {
            *dets_out.get_mut("person").unwrap() += 1;
        } else if ["car","motorcycle","bus","truck"].contains(&det.class.as_str()) {
            *dets_out.get_mut("vehicle").unwrap() += 1;
        } else if ["dog","cat","horse"].contains(&det.class.as_str()) {
            *dets_out.get_mut("animal").unwrap() += 1;
        }
    }
    dets_out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_summary(){
        let mut dets : Detections = Detections::new();
        dets.push( Detection{ xmin:0, ymin:0, xmax:10, ymax:10, score:1.0, class:"person".to_string() } );
        dets.push( Detection{ xmin:10, ymin:20, xmax:110, ymax:210, score:0.81, class:"person".to_string() } );
        let summary = summarize_detections( &dets );
        assert_eq!(summary.len(), 3); // always (for now): people, animals, vehicles
        assert_eq!(summary.get("person").unwrap(), &2); // always (for now): people, animals, vehicles
        assert_eq!(summary.get("animal").unwrap(), &0); // always (for now): people, animals, vehicles
        assert_eq!(summary.get("vehicle").unwrap(), &0); // always (for now): people, animals, vehicles
    }
}

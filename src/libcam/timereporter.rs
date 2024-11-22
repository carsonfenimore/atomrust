use std::time::Instant;

#[allow(non_snake_case, deprecated)]
pub struct RateReporter {
    reportingSecs: f32,
    timeOfLastReport: Instant,
    count: u32, 
    tag: String,
}

#[allow(non_snake_case, deprecated)]
impl RateReporter {
    pub fn new(reportingSecs:f32, tag: &str) -> Self{
        RateReporter {
            reportingSecs: reportingSecs,
            timeOfLastReport: Instant::now(),
            count: 0,
            tag: tag.to_string()
        }
    }
    pub fn duration(&self) -> f32 {
        let dur = self.timeOfLastReport.elapsed();
        let secs:f32 = dur.as_secs() as f32 + dur.subsec_nanos() as f32 / 1_000_000_000.0;
        secs
    }
    pub fn isTimeToReport(&self) -> bool {
        self.duration() >= self.reportingSecs
    }
    pub fn rate(&self) -> f32 {
        let dur = self.duration();
        let rate: f32 = self.count as f32 / dur;
        rate
    }
    pub fn tick(&mut self){
        self.count += 1;
        if self.isTimeToReport() {
            tracing::debug!("{}: {} calls in past {} seconds; {} calls/sec", self.tag, self.count, self.duration(), self.rate());
            self.count = 0;
            self.timeOfLastReport = Instant::now();
        }
    }
}


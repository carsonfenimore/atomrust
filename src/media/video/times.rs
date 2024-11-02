use video_rs as video;

pub struct Times {
    next_dts: video::Time,
    next_pts: video::Time,
}

impl Times {
    pub fn new() -> Self {
        Times {
            next_dts: video::Time::zero(),
            next_pts: video::Time::zero(),
        }
    }

    pub fn update(&mut self, packet: &mut video::Packet) {
        if packet.duration().has_value() {
            packet.set_dts(self.next_dts);
            packet.set_pts(self.next_pts);
            self.next_dts = self.next_dts.aligned_with(packet.duration()).add();
            self.next_pts = self.next_pts.aligned_with(packet.duration()).add();
            tracing::trace!("Updating pts/dts {}/{} pkt dur {}", self.next_pts.as_secs(), self.next_dts.as_secs(), packet.duration());
        }
    }
}

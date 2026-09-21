use std::time::Instant;

use sysinfo::{Disks, MemoryRefreshKind, Networks, RefreshKind, System};

// intrface, tx, rx
type NetRate = Vec<(String, u32, u32)>;

pub struct SysStats {
    pub disk_avail: u8,
    pub net_rate: NetRate,
    pub mem_free: u8,
    sys: System,
    networks: Networks,
    last_net_refresh: Instant,
}

impl SysStats {
    pub fn new() -> Self {
        // Only memory is needed; new_all() would also enumerate every process.
        let sys = System::new_with_specifics(RefreshKind::new().with_memory(MemoryRefreshKind::new().with_ram()));
        let networks = Networks::new_with_refreshed_list();
        let net_rate = NetRate::new();
        SysStats { disk_avail: 0,
                   net_rate,
                   mem_free: 0,
                   sys, 
                   networks,
                   last_net_refresh: Instant::now(),}
    }

    pub fn update(&mut self) {
        self.sys.refresh_memory();
        let total = self.sys.total_memory().max(1) as f32;
        self.mem_free = (100.0 * (self.sys.available_memory() as f32 / total)) as u8;
        // sysinfo reports bytes since the previous refresh; convert to B/s.
        self.networks.refresh();
        let secs = self.last_net_refresh.elapsed().as_secs_f64().max(0.001);
        self.last_net_refresh = Instant::now();
        let mut net_out = NetRate::new();
        for (interface_name, data) in &self.networks {
            if interface_name == "lo" {
                continue;
            }
            net_out.push( (interface_name.clone(), (data.transmitted() as f64 / secs) as u32, (data.received() as f64 / secs) as u32) );
        }
        self.net_rate = net_out;

        let disks = Disks::new_with_refreshed_list();
        for disk in &disks {
            // With the overlay root enabled "/" is the tmpfs upper layer, so
            // this reports how much RAM-backed write space is left.
            if disk.mount_point() != std::path::Path::new("/") { continue; }

            let total = disk.total_space().max(1) as f32;
            self.disk_avail = (100.0 * (disk.available_space() as f32 / total)) as u8;
        }
    }
}


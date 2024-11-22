use std::{thread, time};

use sysinfo::{
    Components, Disks, Networks, System,
};

// intrface, tx, rx
type NetRate = Vec<(String, u32, u32)>;

pub struct SysStats {
    pub disk_avail: u8,
    pub net_rate: NetRate,
    pub mem_free: u8,
    sys: System,
    networks: Networks,
}

impl SysStats {
    pub fn new() -> Self {
        let sys = System::new_all();
        let networks = Networks::new_with_refreshed_list();
        let net_rate = NetRate::new();
        SysStats { disk_avail: 0,
                   net_rate,
                   mem_free: 0,
                   sys, 
                   networks,}
    }

    pub fn update(&mut self) {
        self.mem_free = (100.0 * ( self.sys.used_memory() as f32 / self.sys.total_memory() as f32 )) as u8;
        self.networks.refresh();
        let mut net_out = NetRate::new();
        for (interface_name, data) in &self.networks {
            if interface_name == "lo" {
                continue;
            }
            net_out.push( (interface_name.clone(), data.transmitted() as u32, data.received() as u32) );
        }
        self.net_rate = net_out;

        let disks = Disks::new_with_refreshed_list();
        for disk in &disks {
            let disk_name = disk.mount_point().to_str().unwrap();
            if disk_name  != "/" { continue; }

            self.disk_avail = (100.0 * (disk.available_space() as f32 / disk.total_space() as f32)) as u8;
        }
    }
}


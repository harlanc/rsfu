use std::time::{Duration, SystemTime};

pub const QUARTER_RESOLUTION: &'static str = "q";
pub const HALF_RESOLUTION: &'static str = "h";
pub const FULL_RESOLUTION: &'static str = "f";
#[derive(Default, Clone)]
pub struct SimulcastConfig {
    best_quality_first: bool,
    enable_temporal_layer: bool,
}

pub struct SimulcastTrackHelpers {
    switch_delay: SystemTime,
    temporal_supported: bool,
    temporal_enabled: bool,
    l_ts_calc: i64,

    pub p_ref_pic_id: u16,
    pub ref_pic_id: u16,
    pub l_pic_id: u16,
    pub p_ref_tlz_idx: u8,
    pub ref_tlz_idx: u8,
    pub l_tlz_idx: u8,
    pub ref_sn: u16,
}

impl SimulcastTrackHelpers {
    pub fn new() -> Self {
        Self {
            switch_delay: SystemTime::now(),
            temporal_supported: false,
            temporal_enabled: false,
            l_ts_calc: 0,

            p_ref_pic_id: 0,
            ref_pic_id: 0,
            l_pic_id: 0,
            p_ref_tlz_idx: 0,
            ref_tlz_idx: 0,
            l_tlz_idx: 0,
            ref_sn: 0,
        }
    }
}

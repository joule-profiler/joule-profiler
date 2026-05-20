#[derive(Debug, Clone)]
pub struct Snapshot {
    pub memory_current_min: u64,
    pub memory_current_max: u64,
    pub memory_anon_min: u64,
    pub memory_anon_max: u64,
    pub memory_file_min: u64,
    pub memory_file_max: u64,
    pub memory_swap_current_min: u64,
    pub memory_swap_current_max: u64,

    pub memory_peak: Option<u64>,
    pub memory_kernel_stack: Option<u64>,
    pub memory_slab: Option<u64>,
    pub cpu_usage_usec: Option<u64>,
    pub cpu_user_usec: Option<u64>,
    pub cpu_system_usec: Option<u64>,
    pub cpu_nr_periods: Option<u64>,
    pub cpu_nr_throttled: Option<u64>,
    pub cpu_throttled_usec: Option<u64>,
    pub io_rbytes: Option<u64>,
    pub io_wbytes: Option<u64>,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            memory_current_min: u64::MAX,
            memory_current_max: 0,
            memory_anon_min: u64::MAX,
            memory_anon_max: 0,
            memory_file_min: u64::MAX,
            memory_file_max: 0,
            memory_swap_current_min: u64::MAX,
            memory_swap_current_max: 0,
            memory_peak: None,
            memory_kernel_stack: None,
            memory_slab: None,
            cpu_usage_usec: None,
            cpu_user_usec: None,
            cpu_system_usec: None,
            cpu_nr_periods: None,
            cpu_nr_throttled: None,
            cpu_throttled_usec: None,
            io_rbytes: None,
            io_wbytes: None,
        }
    }
}

impl Snapshot {
    pub fn update(
        &mut self,
        memory_current: u64,
        memory_anon: u64,
        memory_file: u64,
        memory_swap_current: u64,
    ) {
        self.memory_current_min = self.memory_current_min.min(memory_current);
        self.memory_current_max = self.memory_current_max.max(memory_current);
        self.memory_anon_min = self.memory_anon_min.min(memory_anon);
        self.memory_anon_max = self.memory_anon_max.max(memory_anon);
        self.memory_file_min = self.memory_file_min.min(memory_file);
        self.memory_file_max = self.memory_file_max.max(memory_file);
        self.memory_swap_current_min = self.memory_swap_current_min.min(memory_swap_current);
        self.memory_swap_current_max = self.memory_swap_current_max.max(memory_swap_current);
    }

    pub fn reset_phase(&mut self) {
        self.memory_current_min = u64::MAX;
        self.memory_current_max = 0;
        self.memory_anon_min = u64::MAX;
        self.memory_anon_max = 0;
        self.memory_file_min = u64::MAX;
        self.memory_file_max = 0;
        self.memory_swap_current_min = u64::MAX;
        self.memory_swap_current_max = 0;
    }

    pub fn remove_sentinel_values(&mut self) {
        for v in [
            &mut self.memory_current_min,
            &mut self.memory_anon_min,
            &mut self.memory_file_min,
            &mut self.memory_swap_current_min,
        ] {
            if *v == u64::MAX {
                *v = 0;
            }
        }
    }
}

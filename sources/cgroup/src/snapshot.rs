#[derive(Debug, Clone, Default)]
pub struct Snapshot {

    // Memory (memory.current)
    pub memory_current: Option<u64>,

    pub memory_peak: Option<u64>,

    pub memory_anon: Option<u64>,

    pub memory_file: Option<u64>,

    pub memory_kernel_stack: Option<u64>,

    pub memory_slab: Option<u64>,

    pub memory_swap_current: Option<u64>,

    // CPU (cpu.stat)
    pub cpu_usage_usec: Option<u64>,

    pub cpu_user_usec: Option<u64>,

    pub cpu_system_usec: Option<u64>,

    pub cpu_nr_periods: Option<u64>,

    pub cpu_nr_throttled: Option<u64>,

    pub cpu_throttled_usec: Option<u64>,

    // I/O (io.stat)
    pub io_rbytes: Option<u64>,

    pub io_wbytes: Option<u64>,
}
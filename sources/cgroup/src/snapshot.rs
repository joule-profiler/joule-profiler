#[derive(Debug, Default, Clone, Copy)]
pub struct MemorySnapshot {
    pub current: u64,
    pub swap_current: Option<u64>,
    pub anon: Option<u64>,
    pub file: Option<u64>,
    pub peak: Option<u64>,
    pub shmem: Option<u64>,
    pub kernel: Option<u64>,
    pub kernel_stack: Option<u64>,
    pub slab: Option<u64>,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct CpuSnapshot {
    pub usage_usec: u64,
    pub user_usec: u64,
    pub system_usec: u64,

    pub nr_periods: Option<u64>,
    pub nr_throttled: Option<u64>,
    pub throttled_usec: Option<u64>,
    pub nr_bursts: Option<u64>,
    pub burst_usec: Option<u64>,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct IoSnapshot {
    pub rbytes: Option<u64>,
    pub wbytes: Option<u64>,
}

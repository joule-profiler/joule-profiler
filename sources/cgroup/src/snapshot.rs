/// Memory-related cgroup snapshot.
///
/// Represents a single read of `memory.stat` + memory pressure files.
///
/// All values are raw kernel counters in bytes.
#[derive(Debug, Default, Clone, Copy)]
pub struct MemorySnapshot {
    /// Total current memory usage.
    pub current: Option<u64>,

    /// Current swap usage.
    pub swap_current: Option<u64>,

    /// Anonymous memory usage (heap, stack, anon mmap).
    pub anon: Option<u64>,

    /// File-backed memory usage (page cache).
    pub file: Option<u64>,

    /// Peak memory usage observed by cgroup.
    pub peak: Option<u64>,

    /// Shared memory usage (tmpfs, /dev/shm).
    pub shmem: Option<u64>,

    /// Kernel memory usage.
    pub kernel: Option<u64>,

    /// Kernel stack memory usage.
    pub kernel_stack: Option<u64>,

    /// Slab allocator usage.
    pub slab: Option<u64>,
}

/// CPU-related cgroup snapshot.
///
/// Represents raw values from `cpu.stat`.
///
/// `usage_usec`, `user_usec`, and `system_usec` are always present
/// and represent cumulative CPU time in microseconds.
#[derive(Debug, Default, Clone, Copy)]
pub struct CpuSnapshot {
    /// Total CPU time consumed (user + kernel).
    pub usage_usec: u64,

    /// CPU time spent in user space.
    pub user_usec: u64,

    /// CPU time spent in kernel space.
    pub system_usec: u64,

    /// Number of scheduling periods.
    pub nr_periods: Option<u64>,

    /// Number of throttled periods due to CPU quota limits.
    pub nr_throttled: Option<u64>,

    /// Total time spent throttled.
    pub throttled_usec: Option<u64>,

    /// Number of burst events (burstable cgroups).
    pub nr_bursts: Option<u64>,

    /// Total burst time.
    pub burst_usec: Option<u64>,
}

/// I/O-related cgroup snapshot.
///
/// Represents raw values from `io.stat`.
#[derive(Debug, Default, Clone, Copy)]
pub struct IoSnapshot {
    /// Total bytes read by the cgroup.
    pub rbytes: Option<u64>,

    /// Total bytes written by the cgroup.
    pub wbytes: Option<u64>,
}

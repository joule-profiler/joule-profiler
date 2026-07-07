/// Snapshot of memory and I/O measurements for a process hierarchy.
///
/// All fields are summed across the process and all its descendants.
/// Memory values are in bytes.
#[derive(Default, Clone, Copy)]
pub struct ProcSnapshot {
    /// Virtual memory size, from `/proc/{pid}/stat`.
    pub vm_size: u64,

    /// Resident set size.
    pub rss: u64,

    /// Proportional set size (shared pages divided by the count of process using them).
    pub pss: u64,

    /// Shared memory (clean + dirty).
    pub shared: u64,

    /// Anonymous memory (not backed by a file).
    pub anon: u64,

    /// Cumulative bytes read since process start.
    pub read_bytes: u64,

    /// Cumulative bytes written since process start.
    pub write_bytes: u64,
}

impl std::ops::AddAssign for ProcSnapshot {
    fn add_assign(&mut self, other: Self) {
        self.vm_size += other.vm_size;
        self.rss += other.rss;
        self.pss += other.pss;
        self.shared += other.shared;
        self.anon += other.anon;
        self.read_bytes += other.read_bytes;
        self.write_bytes += other.write_bytes;
    }
}

/// Point-in-time system-wide memory statistics from `/proc/meminfo`.
///
/// All values are in bytes. Optional fields are absent when the kernel
/// does not expose them (e.g. `MemAvailable` on old kernels, `AnonPages`
/// on some minimal configurations).
#[derive(Default, Clone, Copy)]
pub struct GlobalSnapshot {
    /// Estimate of available memory.
    /// Preferred over `mem_free + cached` for computing used memory.
    pub mem_available: Option<u64>,

    /// Free memory, used if `mem_available` is not accessible.
    pub mem_free: u64,

    /// Cached memory.
    pub cached: u64,

    /// Anonymous pages (heap, stack, mmaps).
    pub anon: Option<u64>,

    /// Free swap space.
    pub swap_free: u64,
}

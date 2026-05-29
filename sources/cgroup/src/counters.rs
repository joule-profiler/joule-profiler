use crate::snapshot::{CpuSnapshot, IoSnapshot, MemorySnapshot};

/// A min/max accumulator for a single metric.
#[derive(Debug, Clone, Copy, Default)]
pub struct MinMax(Option<u64>, Option<u64>);

impl MinMax {
    /// Updates the min/max bounds with the provided value.
    pub fn update(&mut self, value: u64) {
        self.0 = Some(self.0.map_or(value, |m| m.min(value)));
        self.1 = Some(self.1.map_or(value, |m| m.max(value)));
    }

    pub fn reset(&mut self) {
        self.0 = None;
        self.1 = None;
    }

    pub fn min(&self) -> Option<u64> {
        self.0
    }

    pub fn max(&self) -> Option<u64> {
        self.1
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct BeginEnd(u64, u64);

impl BeginEnd {
    pub fn update(&mut self, value: u64) {
        self.1 = value;
    }

    pub fn diff(&self) -> u64 {
        self.1.saturating_sub(self.0)
    }

    pub fn new_phase(&mut self) {
        self.0 = self.1;
    }
}

trait NewPhase {
    fn new_phase(&mut self);
}

impl NewPhase for BeginEnd {
    fn new_phase(&mut self) {
        self.0 = self.1;
    }
}

impl NewPhase for Option<BeginEnd> {
    fn new_phase(&mut self) {
        if let Some(v) = self {
            v.new_phase();
        }
    }
}

trait UpdateOpt {
    fn update(&mut self, value: Option<u64>);
}

impl UpdateOpt for Option<BeginEnd> {
    fn update(&mut self, value: Option<u64>) {
        if let Some(v) = value {
            self.get_or_insert_default().update(v);
        }
    }
}

impl UpdateOpt for Option<MinMax> {
    fn update(&mut self, value: Option<u64>) {
        if let Some(v) = value {
            self.get_or_insert_default().update(v);
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct MemoryCounters {
    pub current: Option<MinMax>,
    pub swap_current: Option<MinMax>,
    pub anon: Option<MinMax>,
    pub file: Option<MinMax>,
    pub peak: Option<MinMax>,
    pub shmem: Option<MinMax>,
    pub kernel: Option<MinMax>,
    pub kernel_stack: Option<MinMax>,
    pub slab: Option<MinMax>,
}

impl MemoryCounters {
    pub fn update(&mut self, snapshot: &MemorySnapshot) {
        self.current.update(snapshot.current);
        self.swap_current.update(snapshot.swap_current);
        self.anon.update(snapshot.anon);
        self.file.update(snapshot.file);
        self.peak.update(snapshot.peak);
        self.shmem.update(snapshot.shmem);
        self.kernel.update(snapshot.kernel);
        self.kernel_stack.update(snapshot.kernel_stack);
        self.slab.update(snapshot.slab);
    }

    pub fn reset(&mut self) {
        self.current = None;
        self.swap_current = None;
        self.anon = None;
        self.file = None;
        self.peak = None;
        self.shmem = None;
        self.kernel = None;
        self.kernel_stack = None;
        self.slab = None;
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct CpuCounters {
    pub usage_usec: BeginEnd,
    pub user_usec: BeginEnd,
    pub system_usec: BeginEnd,

    pub nr_periods: Option<BeginEnd>,
    pub nr_throttled: Option<BeginEnd>,
    pub throttled_usec: Option<BeginEnd>,
    pub nr_bursts: Option<BeginEnd>,
    pub burst_usec: Option<BeginEnd>,
}

impl CpuCounters {
    pub fn update(&mut self, snapshot: &CpuSnapshot) {
        self.usage_usec.update(snapshot.usage_usec);
        self.user_usec.update(snapshot.user_usec);
        self.system_usec.update(snapshot.system_usec);
        self.nr_periods.update(snapshot.nr_periods);
        self.nr_throttled.update(snapshot.nr_throttled);
        self.throttled_usec.update(snapshot.throttled_usec);
        self.nr_bursts.update(snapshot.nr_bursts);
        self.burst_usec.update(snapshot.burst_usec);
    }

    pub fn new_phase(&mut self) {
        self.usage_usec.new_phase();
        self.user_usec.new_phase();
        self.system_usec.new_phase();
        self.nr_periods.new_phase();
        self.nr_throttled.new_phase();
        self.throttled_usec.new_phase();
        self.nr_bursts.new_phase();
        self.burst_usec.new_phase();
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct IoCounters {
    pub rbytes: Option<BeginEnd>,
    pub wbytes: Option<BeginEnd>,
}

impl IoCounters {
    pub fn update(&mut self, snapshot: &IoSnapshot) {
        if let Some(v) = snapshot.rbytes {
            self.rbytes.get_or_insert_default().update(v);
        }

        if let Some(v) = snapshot.wbytes {
            self.wbytes.get_or_insert_default().update(v);
        }
    }

    pub fn new_phase(&mut self) {
        if let Some(v) = &mut self.rbytes {
            v.new_phase();
        }

        if let Some(v) = &mut self.wbytes {
            v.new_phase();
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Counters {
    pub proc_memory: MemoryCounters,
    pub proc_cpu: CpuCounters,
    pub proc_io: IoCounters,

    pub global_memory: MemoryCounters,
    pub global_cpu: CpuCounters,
    pub global_io: IoCounters,
}

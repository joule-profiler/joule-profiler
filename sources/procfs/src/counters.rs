use crate::snapshot::{GlobalSnapshot, ProcSnapshot};

/// A min/max accumulator for a single metric.
#[derive(Debug, Clone, Copy, Default)]
pub struct MinMax(pub Option<u64>, Option<u64>);

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

/// Accumulated memory and I/O counters for a process hierarchy over a phase.
///
/// Memory fields track min/max over all snapshots in the phase.
/// I/O fields track cumulative byte counts: begin is set at phase start.
/// All metrics are in bytes.
#[derive(Debug, Default, Clone, Copy)]
pub struct ProcCounters {
    /// Virtual memory size.
    pub vm_size: MinMax,

    /// Resident set size.
    pub rss: MinMax,

    /// Proportional set size.
    pub pss: MinMax,

    /// Shared memory, clean + dirty.
    pub shared: MinMax,

    /// Anonymous memory.
    pub anon: MinMax,

    /// Cumulative bytes read at the beginning of a phase.
    pub begin_read_bytes: u64,

    /// Cumulative bytes written at the beginning of a phase.
    pub begin_write_bytes: u64,

    /// Highest cumulative bytes read observed during this phase.
    pub end_read_bytes: u64,

    /// Highest cumulative bytes written observed during this phase.
    pub end_write_bytes: u64,
}

impl ProcCounters {
    /// Merges a [`ProcSnapshot`] into the counters.
    ///
    /// I/O fields take the max of the current end and the snapshot value,
    /// since `/proc/{pid}/io` counters are monotonically increasing.
    pub fn update(&mut self, snapshot: &ProcSnapshot) {
        self.vm_size.update(snapshot.vm_size);
        self.rss.update(snapshot.rss);
        self.pss.update(snapshot.pss);
        self.shared.update(snapshot.shared);
        self.anon.update(snapshot.anon);
        self.end_read_bytes = self.end_read_bytes.max(snapshot.read_bytes);
        self.end_write_bytes = self.end_write_bytes.max(snapshot.write_bytes);
    }

    /// Resets memory min/max for the next phase and carries I/O end values forward as the new begin.
    pub fn reset(&mut self) {
        self.vm_size.reset();
        self.rss.reset();
        self.pss.reset();
        self.shared.reset();
        self.anon.reset();
        self.begin_read_bytes = self.end_read_bytes;
        self.begin_write_bytes = self.end_write_bytes;
    }
}

/// Computes used memory as `MemTotal - MemAvailable`.
///
/// If `MemAvailable` is present in `/proc/meminfo`, it is used directly (preferred).
/// Otherwise, falls back to `MemTotal - (MemFree + Cached)`, which is less accurate
/// but universally available.
///
/// Note: min/max are inverted relative to `available` since higher availability
/// means lower usage.
pub fn compute_mem_used(
    mem_total: u64,
    mem_available: Option<MinMax>,
    mem_free: MinMax,
    cached: MinMax,
) -> MinMax {
    if let Some(available) = mem_available {
        MinMax(
            Some(mem_total.saturating_sub(available.max().unwrap_or_default())),
            Some(mem_total.saturating_sub(available.min().unwrap_or_default())),
        )
    } else {
        MinMax(
            Some(mem_total.saturating_sub(
                mem_free.max().unwrap_or_default() + cached.max().unwrap_or_default(),
            )),
            Some(mem_total.saturating_sub(
                mem_free.min().unwrap_or_default() + cached.min().unwrap_or_default(),
            )),
        )
    }
}

/// Accumulated system-wide memory counters over a phase.
///
/// `mem_available` and `anon` are `Option` because they may not be present
/// in `/proc/meminfo` on all kernel configurations. They are initialized
/// lazily on the first snapshot that contains them.
#[derive(Debug, Default, Clone, Copy)]
pub struct GlobalCounters {
    /// Available memory (`MemAvailable`). None if not exposed by the kernel.
    pub mem_available: Option<MinMax>,

    /// Free memory (`MemFree`).
    pub mem_free: MinMax,

    /// Page cache (`Cached`).
    pub cached: MinMax,

    /// Anonymous pages (`AnonPages`). None if not exposed by the kernel.
    pub anon: Option<MinMax>,

    /// Free swap (`SwapFree`).
    pub swap_free: MinMax,
}

impl GlobalCounters {
    /// Merges a [`GlobalSnapshot`] into the counters.
    pub fn update(&mut self, snapshot: &GlobalSnapshot) {
        self.mem_free.update(snapshot.mem_free);
        self.cached.update(snapshot.cached);
        self.swap_free.update(snapshot.swap_free);
        if let Some(v) = snapshot.mem_available {
            self.mem_available.get_or_insert_default().update(v);
        }
        if let Some(v) = snapshot.anon {
            self.anon.get_or_insert_default().update(v);
        }
    }

    /// Resets all counters for the next phase.
    pub fn reset(&mut self) {
        self.mem_free.reset();
        self.cached.reset();
        self.swap_free.reset();
        if let Some(v) = &mut self.mem_available {
            v.reset();
        }
        if let Some(v) = &mut self.anon {
            v.reset();
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Counters {
    pub proc: ProcCounters,
    pub global: GlobalCounters,
}

impl Counters {
    pub fn update(&mut self, proc: &ProcSnapshot, global: &GlobalSnapshot) {
        self.proc.update(proc);
        self.global.update(global);
    }

    pub fn reset(&mut self) {
        self.proc.reset();
        self.global.reset();
    }
}

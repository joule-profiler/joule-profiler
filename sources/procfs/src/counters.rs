use crate::snapshot::{GlobalSnapshot, ProcSnapshot};

/// A min/max accumulator for a single metric.
///
/// Initialized with `min = u64::MAX` and `max = 0` as sentinels,
/// so the first [`update`](MinMax::update) call sets both bounds correctly.
/// Call [`remove_sentinel`](MinMax::remove_sentinel) before reading min.
#[derive(Debug, Clone, Copy)]
pub struct MinMax(pub u64, pub u64);

impl MinMax {
    /// Updates the min/max bounds with the provided value.
    pub fn update(&mut self, value: u64) {
        self.0 = self.0.min(value);
        self.1 = self.1.max(value);
    }

    /// Resets to sentinel state (`min = u64::MAX`, `max = 0`).
    pub fn reset(&mut self) {
        self.0 = u64::MAX;
        self.1 = 0;
    }

    /// Replaces the `u64::MAX` sentinel with `0` if no update was received.
    pub fn remove_sentinel(&mut self) {
        if self.0 == u64::MAX {
            self.0 = 0;
        }
    }

    pub fn min(&self) -> u64 {
        self.0
    }

    pub fn max(&self) -> u64 {
        self.1
    }
}

impl Default for MinMax {
    fn default() -> Self {
        Self(u64::MAX, 0)
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

    /// Replaces `u64::MAX` sentinels with `0` on all memory fields before reading.
    pub fn remove_sentinels(&mut self) {
        self.vm_size.remove_sentinel();
        self.rss.remove_sentinel();
        self.pss.remove_sentinel();
        self.shared.remove_sentinel();
        self.anon.remove_sentinel();
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
        if let Some(value) = snapshot.mem_available {
            self.mem_available.get_or_insert_default().update(value);
        }

        if let Some(value) = snapshot.anon {
            self.anon.get_or_insert_default().update(value);
        }

        self.mem_free.update(snapshot.mem_free);
        self.cached.update(snapshot.cached);
        self.swap_free.update(snapshot.swap_free);
    }

    /// Resets all counters for the next phase.
    pub fn reset(&mut self) {
        if let Some(mem_available) = &mut self.mem_available {
            mem_available.reset();
        }
        if let Some(anon) = &mut self.anon {
            anon.reset();
        }
        self.mem_free.reset();
        self.cached.reset();
        self.swap_free.reset();
    }

    /// Replaces `u64::MAX` sentinels with `0` on all fields before reading.
    pub fn remove_sentinels(&mut self) {
        if let Some(mem_available) = &mut self.mem_available {
            mem_available.remove_sentinel();
        }
        if let Some(anon) = &mut self.anon {
            anon.remove_sentinel();
        }
        self.mem_free.remove_sentinel();
        self.cached.remove_sentinel();
        self.swap_free.remove_sentinel();
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

    pub fn remove_sentinels(&mut self) {
        self.proc.remove_sentinels();
        self.global.remove_sentinels();
    }
}

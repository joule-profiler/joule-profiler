use crate::snapshot::{GlobalSnapshot, ProcSnapshot};

#[derive(Debug, Clone, Copy)]
pub struct MinMax(pub u64, pub u64);

impl MinMax {
    pub fn update(&mut self, value: u64) {
        self.0 = self.0.min(value);
        self.1 = self.1.max(value);
    }

    pub fn reset(&mut self) {
        self.0 = u64::MAX;
        self.1 = 0;
    }

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

#[derive(Debug, Default, Clone, Copy)]
pub struct ProcCounters {
    pub vm_size: MinMax,
    pub rss: MinMax,
    pub pss: MinMax,
    pub shared: MinMax,
    pub anon: MinMax,

    pub begin_read_bytes: u64,
    pub begin_write_bytes: u64,
    pub end_read_bytes: u64,
    pub end_write_bytes: u64,
}

impl ProcCounters {
    pub fn update(&mut self, snapshot: &ProcSnapshot) {
        self.vm_size.update(snapshot.vm_size);
        self.rss.update(snapshot.rss);
        self.pss.update(snapshot.pss);
        self.shared.update(snapshot.shared);
        self.anon.update(snapshot.anon);

        self.end_read_bytes = snapshot.read_bytes;
        self.end_write_bytes = snapshot.write_bytes;
    }

    pub fn reset(&mut self) {
        self.vm_size.reset();
        self.rss.reset();
        self.pss.reset();
        self.shared.reset();
        self.anon.reset();

        self.begin_read_bytes = self.end_read_bytes;
        self.begin_write_bytes = self.end_write_bytes;
    }

    pub fn remove_sentinels(&mut self) {
        self.vm_size.remove_sentinel();
        self.rss.remove_sentinel();
        self.pss.remove_sentinel();
        self.shared.remove_sentinel();
        self.anon.remove_sentinel();
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct GlobalCounters {
    pub mem_available: Option<MinMax>,
    pub mem_free: MinMax,
    pub cached: MinMax,
    pub anon: Option<MinMax>,
    pub swap_free: MinMax,
}

impl GlobalCounters {
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

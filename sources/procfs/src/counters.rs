#[derive(Debug, Default, Clone)]
pub struct MemoryCounters {
    pub min_vm_size: u64,
    pub min_rss: u64,
    pub min_pss: u64,
    pub min_shared: u64,
    pub min_anon: u64,

    pub max_vm_size: u64,
    pub max_rss: u64,
    pub max_pss: u64,
    pub max_shared: u64,
    pub max_anon: u64,
}

impl MemoryCounters {
    pub fn update(&mut self, vm_size: u64, rss: u64, pss: u64, shared: u64, anon: u64) {
        self.min_vm_size = self.min_vm_size.min(vm_size);
        self.max_vm_size = self.max_vm_size.max(vm_size);
        self.min_rss = self.min_rss.min(rss);
        self.max_rss = self.max_rss.max(rss);
        self.min_pss = self.min_pss.min(pss);
        self.max_pss = self.max_pss.max(pss);
        self.min_shared = self.min_shared.min(shared);
        self.max_shared = self.max_shared.max(shared);
        self.min_anon = self.min_anon.min(anon);
        self.max_anon = self.max_anon.max(anon);
    }

    pub fn reset_phase(&mut self) {
        self.max_vm_size = 0;
        self.max_rss = 0;
        self.max_pss = 0;
        self.max_shared = 0;
        self.max_anon = 0;

        self.min_vm_size = u64::MAX;
        self.min_rss = u64::MAX;
        self.min_pss = u64::MAX;
        self.min_shared = u64::MAX;
        self.min_anon = u64::MAX;
    }

    pub fn delta(current: u64, start: u64) -> i64 {
        current.cast_signed() - start.cast_signed()
    }

    pub fn remove_sentinel_values(&mut self) {
        let values = [
            &mut self.min_vm_size,
            &mut self.min_rss,
            &mut self.min_pss,
            &mut self.min_shared,
            &mut self.min_anon,
        ];
        for v in values {
            if *v == u64::MAX {
                *v = 0;
            }
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct IoCounters {
    pub begin_read_bytes: u64,
    pub begin_write_bytes: u64,
    pub end_read_bytes: u64,
    pub end_write_bytes: u64,
}

pub struct Counters {
    pub memory_counters: MemoryCounters,
    pub io_counters: IoCounters,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct MemoryCounters {
    pub proc: ProcMemoryCounters,
    pub global: GlobalMemoryCounters,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ProcMemoryCounters {
    pub min_vm_size: u64,
    pub max_vm_size: u64,

    pub min_rss: u64,
    pub max_rss: u64,

    pub min_pss: u64,
    pub max_pss: u64,

    pub min_shared: u64,
    pub max_shared: u64,

    pub min_anon: u64,
    pub max_anon: u64,
}

impl ProcMemoryCounters {
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
        self.min_vm_size = u64::MAX;
        self.max_vm_size = 0;

        self.min_rss = u64::MAX;
        self.max_rss = 0;

        self.min_pss = u64::MAX;
        self.max_pss = 0;

        self.min_shared = u64::MAX;
        self.max_shared = 0;

        self.min_anon = u64::MAX;
        self.max_anon = 0;
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
pub struct GlobalMemoryCounters {
    pub min_mem_available: Option<u64>,
    pub max_mem_available: Option<u64>,

    pub min_mem_free: u64,
    pub max_mem_free: u64,

    pub min_cached: u64,
    pub max_cached: u64,

    pub min_anon: Option<u64>,
    pub max_anon: Option<u64>,

    pub min_swap_free: u64,
    pub max_swap_free: u64,
}

impl GlobalMemoryCounters {
    pub fn update(
        &mut self,
        mem_available: Option<u64>,
        mem_free: u64,
        cached: u64,
        anon: Option<u64>,
        swap_free: u64,
    ) {
        if let Some(mem_available) = mem_available {
            self.min_mem_available =
                Some(if let Some(min_mem_available) = self.min_mem_available {
                    min_mem_available.min(mem_available)
                } else {
                    mem_available
                });

            self.max_mem_available =
                Some(if let Some(max_mem_available) = self.max_mem_available {
                    max_mem_available.max(mem_available)
                } else {
                    mem_available
                });
        }

        self.min_mem_free = self.min_mem_free.min(mem_free);
        self.max_mem_free = self.max_mem_free.max(mem_free);

        self.min_cached = self.min_cached.min(cached);
        self.max_cached = self.max_cached.max(cached);

        if let Some(anon) = anon {
            self.min_anon = Some(if let Some(min_anon) = self.min_anon {
                min_anon.min(anon)
            } else {
                anon
            });

            self.max_anon = Some(if let Some(max_anon) = self.max_anon {
                max_anon.max(anon)
            } else {
                anon
            });
        }

        self.min_anon = self.min_anon.min(anon);
        self.max_anon = self.max_anon.max(anon);

        self.min_swap_free = self.min_swap_free.min(swap_free);
        self.max_swap_free = self.max_swap_free.max(swap_free);
    }

    pub fn reset_phase(&mut self) {
        self.min_mem_available = self.min_mem_available.map(|_| u64::MAX);
        self.max_mem_available = self.max_mem_available.map(|_| 0);

        self.min_mem_free = u64::MAX;
        self.max_mem_free = 0;

        self.min_cached = u64::MAX;
        self.max_cached = 0;

        self.min_anon = self.min_anon.map(|_| u64::MAX);
        self.max_anon = self.max_anon.map(|_| 0);

        self.min_swap_free = u64::MAX;
        self.max_swap_free = 0;
    }

    pub fn remove_sentinel_values(&mut self) {
        self.min_mem_available = self
            .min_mem_available
            .map(|v| if v == u64::MAX { 0 } else { v });

        self.min_anon = self.min_anon.map(|v| if v == u64::MAX { 0 } else { v });

        let values = [
            &mut self.min_mem_free,
            &mut self.min_cached,
            &mut self.min_swap_free,
        ];
        for v in values {
            if *v == u64::MAX {
                *v = 0;
            }
        }
    }
}

impl MemoryCounters {
    pub fn update_proc(&mut self, vm_size: u64, rss: u64, pss: u64, shared: u64, anon: u64) {
        self.proc.update(vm_size, rss, pss, shared, anon);
    }

    pub fn update_global(
        &mut self,
        mem_available: Option<u64>,
        mem_free: u64,
        cached: u64,
        anon: Option<u64>,
        swap_free: u64,
    ) {
        self.global
            .update(mem_available, mem_free, cached, anon, swap_free);
    }

    pub fn reset_phase(&mut self) {
        self.proc.reset_phase();
        self.global.reset_phase();
    }

    pub fn remove_sentinel_values(&mut self) {
        self.proc.remove_sentinel_values();
        self.global.remove_sentinel_values();
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct IoCounters {
    pub begin_read_bytes: u64,
    pub begin_write_bytes: u64,
    pub end_read_bytes: u64,
    pub end_write_bytes: u64,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Counters {
    pub memory_counters: MemoryCounters,
    pub io_counters: IoCounters,
}

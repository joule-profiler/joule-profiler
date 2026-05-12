#[derive(Debug, Default, Clone)]
pub struct MemoryCounters {
    pub vm_size: u64,
    pub rss: u64,
    pub pss: u64,
    pub shared: u64,
    pub anon: u64,

    pub max_vm_size: u64,
    pub max_rss: u64,
    pub max_pss: u64,
    pub max_shared: u64,
    pub max_anon: u64,

    pub phase_start_vm_size: u64,
    pub phase_start_rss: u64,
    pub phase_start_pss: u64,
    pub phase_start_shared: u64,
    pub phase_start_anon: u64,
}

impl MemoryCounters {
    pub fn update(&mut self, vm_size: u64, rss: u64, pss: u64, shared: u64, anon: u64) {
        self.vm_size = vm_size;
        self.max_vm_size = self.max_vm_size.max(vm_size);
        self.rss = rss;
        self.max_rss = self.max_rss.max(rss);
        self.pss = pss;
        self.max_pss = self.max_pss.max(pss);
        self.shared = shared;
        self.max_shared = self.max_shared.max(shared);
        self.anon = anon;
        self.max_anon = self.max_anon.max(anon);
    }

    pub fn reset_phase(&mut self) {
        self.phase_start_vm_size = self.vm_size;
        self.phase_start_rss = self.rss;
        self.phase_start_pss = self.pss;
        self.phase_start_shared = self.shared;
        self.phase_start_anon = self.anon;

        self.max_vm_size = self.vm_size;
        self.max_rss = self.rss;
        self.max_pss = self.pss;
        self.max_shared = self.shared;
        self.max_anon = self.anon;
    }

    pub fn delta(current: u64, start: u64) -> i64 {
        current as i64 - start as i64
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
    pub io_counters: IoCounters
}
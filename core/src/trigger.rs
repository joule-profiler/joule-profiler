use std::sync::{
    Condvar, Mutex, MutexGuard, PoisonError,
    atomic::{AtomicBool, Ordering},
};

/// Wakes every sensor thread at each phase boundary using a condvar.
pub(crate) struct Trigger {
    boundaries: Mutex<u64>,
    condvar: Condvar,
    running: AtomicBool,
}

impl Trigger {
    fn lock(&self) -> MutexGuard<'_, u64> {
        self.boundaries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn waiter(&self) -> Waiter<'_> {
        Waiter {
            trigger: self,
            phase_index: 0,
        }
    }

    pub(crate) fn trigger(&self) {
        *self.lock() += 1;
        self.condvar.notify_all();
    }

    pub(crate) fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }

    pub(crate) fn stop(&self) {
        {
            let _boundaries = self.lock();
            self.running.store(false, Ordering::Release);
        }
        self.condvar.notify_all();
    }
}

impl Default for Trigger {
    fn default() -> Self {
        Self {
            boundaries: Mutex::new(0),
            condvar: Condvar::new(),
            running: AtomicBool::new(true),
        }
    }
}

pub(crate) struct Waiter<'a> {
    trigger: &'a Trigger,
    phase_index: u64,
}

impl Waiter<'_> {
    /// Blocks until the next boundary, and returns false once the run is over.
    pub(crate) fn wait(&mut self) -> bool {
        let trigger = self.trigger;
        let mut boundaries = trigger.lock();

        while *boundaries == self.phase_index && trigger.is_running() {
            boundaries = trigger
                .condvar
                .wait(boundaries)
                .unwrap_or_else(PoisonError::into_inner);
        }

        if *boundaries == self.phase_index {
            return false;
        }

        self.phase_index += 1;
        true
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, thread, time::Duration};

    use super::*;

    #[test]
    fn late_waiter_gets_every_boundary() {
        let trigger = Trigger::default();
        for _ in 0..3 {
            trigger.trigger();
        }
        trigger.stop();

        let mut waiter = trigger.waiter();
        let mut phase_index = 0;
        while waiter.wait() {
            phase_index += 1;
        }

        assert_eq!(phase_index, 3);
    }

    #[test]
    fn waiter_sees_end_of_run() {
        let trigger = Trigger::default();
        trigger.stop();

        assert!(!trigger.waiter().wait());
        assert!(!trigger.is_running());
    }

    #[test]
    fn stop_wakes_waiter() {
        let trigger = Arc::new(Trigger::default());
        let waited = Arc::clone(&trigger);
        let source = thread::spawn(move || waited.waiter().wait());

        thread::sleep(Duration::from_millis(50));
        trigger.stop();

        assert!(!source.join().unwrap());
    }

    #[test]
    fn boundary_wakes_waiter() {
        let trigger = Arc::new(Trigger::default());
        let waited = Arc::clone(&trigger);
        let source = thread::spawn(move || waited.waiter().wait());

        thread::sleep(Duration::from_millis(50));
        trigger.trigger();

        assert!(source.join().unwrap());
    }
}

use std::fmt::Display;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// A thread that calls `tick` every interval until it is dropped.
pub struct Poller {
    stop: Arc<(Mutex<bool>, Condvar)>,
    thread: Option<JoinHandle<()>>,
}

impl Poller {
    pub fn start<E: Display>(
        every: Duration,
        mut tick: impl FnMut() -> Result<(), E> + Send + 'static,
    ) -> Self {
        let stop = Arc::new((Mutex::new(false), Condvar::new()));
        let waited = Arc::clone(&stop);

        let thread = thread::spawn(move || {
            let (stopped, wake) = &*waited;
            let mut asked = lock(stopped);

            while !*asked {
                let (waiting, timed_out) = wake
                    .wait_timeout_while(asked, every, |stopped| !*stopped)
                    .unwrap_or_else(PoisonError::into_inner);
                asked = waiting;

                if !timed_out.timed_out() {
                    break;
                }

                drop(asked);
                if let Err(error) = tick() {
                    log::warn!("polling failed: {error}");
                }
                asked = lock(stopped);
            }
        });

        Self {
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for Poller {
    fn drop(&mut self) {
        let (stopped, wake) = &*self.stop;
        *lock(stopped) = true;
        wake.notify_all();

        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;

    #[test]
    fn a_poller_that_is_dropped_does_not_wait_for_its_next_look() {
        let poller = Poller::start(Duration::from_secs(30), || Ok::<(), &str>(()));

        let at = Instant::now();
        drop(poller);

        assert!(
            at.elapsed() < Duration::from_millis(500),
            "dropping it waited {:?} for a look that was never going to happen",
            at.elapsed()
        );
    }

    #[test]
    fn a_poller_looks_every_interval() {
        let looks = Arc::new(AtomicUsize::new(0));

        let counted = Arc::clone(&looks);
        let poller = Poller::start(Duration::from_millis(10), move || {
            counted.fetch_add(1, Ordering::Relaxed);
            Ok::<(), &str>(())
        });

        thread::sleep(Duration::from_millis(150));
        drop(poller);

        let looked = looks.load(Ordering::Relaxed);
        assert!((5..=25).contains(&looked), "looked {looked} times in 150ms");
    }

    #[test]
    fn a_poller_stopped_while_it_is_looking_does_not_sleep_again() {
        let poller = Poller::start(Duration::from_millis(5), || {
            thread::sleep(Duration::from_millis(200));
            Ok::<(), &str>(())
        });

        thread::sleep(Duration::from_millis(50));

        let at = Instant::now();
        drop(poller);

        assert!(
            at.elapsed() < Duration::from_millis(500),
            "dropping it waited {:?}",
            at.elapsed()
        );
    }
}

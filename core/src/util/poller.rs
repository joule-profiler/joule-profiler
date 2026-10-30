use std::{
    any::Any,
    error::Error,
    sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError},
    thread::{self, JoinHandle},
    time::Duration,
};

use thiserror::Error;

use crate::error::ErrorChain;

/// The polling thread panicked.
#[derive(Debug, Error)]
#[error("the polling thread panicked: {0}")]
pub struct PollerPanicked(String);

/// A thread that calls `tick` regularly until it is stopped or `tick` fails.
pub struct Poller<E: Error + From<PollerPanicked> + Send + 'static> {
    stop: Arc<(Mutex<bool>, Condvar)>,
    thread: Option<JoinHandle<Result<(), E>>>,
}

impl<E: Error + From<PollerPanicked> + Send + 'static> Poller<E> {
    /// Starts a thread that calls `tick` at a fixed interval `every`.
    pub fn start(
        every: Duration,
        mut tick: impl FnMut() -> Result<(), E> + Send + 'static,
    ) -> Self {
        Self::start_scheduled(every, move || tick().map(|()| every))
    }

    /// Starts a thread that waits `first`, then calls `tick` and waits the duration it returns,
    /// again and again.
    pub fn start_scheduled(
        first: Duration,
        mut tick: impl FnMut() -> Result<Duration, E> + Send + 'static,
    ) -> Self {
        let stop = Arc::new((Mutex::new(false), Condvar::new()));
        let waited = Arc::clone(&stop);

        let thread = thread::spawn(move || {
            let (stopped, wake) = &*waited;
            let mut asked = lock(stopped);
            let mut wait = first;

            while !*asked {
                let (waiting, timed_out) = wake
                    .wait_timeout_while(asked, wait, |stopped| !*stopped)
                    .unwrap_or_else(PoisonError::into_inner);
                asked = waiting;

                if !timed_out.timed_out() {
                    break;
                }

                drop(asked);
                wait = tick()?;
                asked = lock(stopped);
            }

            Ok(())
        });

        Self {
            stop,
            thread: Some(thread),
        }
    }

    /// Stops the thread and waits for it to end.
    ///
    /// # Errors
    ///
    /// Returns the error `tick` failed with, or [`PollerPanicked`] if it panicked.
    pub fn stop(mut self) -> Result<(), E> {
        self.join()
    }

    fn join(&mut self) -> Result<(), E> {
        let (stopped, wake) = &*self.stop;
        *lock(stopped) = true;
        wake.notify_all();

        match self.thread.take() {
            Some(thread) => thread
                .join()
                .unwrap_or_else(|payload| Err(PollerPanicked(panic_message(&*payload)).into())),
            None => Ok(()),
        }
    }
}

impl<E: Error + From<PollerPanicked> + Send + 'static> Drop for Poller<E> {
    fn drop(&mut self) {
        if let Err(error) = self.join() {
            log::warn!("polling failed: {}", ErrorChain(&error));
        }
    }
}

fn panic_message(payload: &(dyn Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|message| (*message).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "no message".to_owned())
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::atomic::{AtomicUsize, Ordering},
        time::Instant,
    };

    #[derive(Debug, Error)]
    enum TickError {
        #[error("the counter is gone")]
        Gone,

        #[error(transparent)]
        Panicked(#[from] PollerPanicked),
    }

    #[test]
    fn stop_does_not_wait() {
        let poller = Poller::start(Duration::from_secs(30), || Ok::<(), TickError>(()));

        let at = Instant::now();
        poller.stop().unwrap();

        assert!(
            at.elapsed() < Duration::from_millis(500),
            "stopping it waited {:?}",
            at.elapsed()
        );
    }

    #[test]
    fn polls_every_interval() {
        let looks = Arc::new(AtomicUsize::new(0));

        let counted = Arc::clone(&looks);
        let poller = Poller::start(Duration::from_millis(10), move || {
            counted.fetch_add(1, Ordering::Relaxed);
            Ok::<(), TickError>(())
        });

        thread::sleep(Duration::from_millis(150));
        poller.stop().unwrap();

        let looked = looks.load(Ordering::Relaxed);
        assert!((5..=25).contains(&looked), "looked {looked} times in 150ms");
    }

    #[test]
    fn stop_while_ticking() {
        let poller = Poller::start(Duration::from_millis(5), || {
            thread::sleep(Duration::from_millis(200));
            Ok::<(), TickError>(())
        });

        thread::sleep(Duration::from_millis(50));

        let at = Instant::now();
        poller.stop().unwrap();

        assert!(
            at.elapsed() < Duration::from_millis(500),
            "stopping it waited {:?}",
            at.elapsed()
        );
    }

    #[test]
    fn scheduled_wait() {
        let looks = Arc::new(AtomicUsize::new(0));

        let counted = Arc::clone(&looks);
        let poller = Poller::start_scheduled(Duration::from_millis(5), move || {
            counted.fetch_add(1, Ordering::Relaxed);
            Ok::<_, TickError>(Duration::from_secs(30))
        });

        thread::sleep(Duration::from_millis(150));
        poller.stop().unwrap();

        assert_eq!(looks.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn failing_tick_stops() {
        let looks = Arc::new(AtomicUsize::new(0));

        let counted = Arc::clone(&looks);
        let poller = Poller::start(Duration::from_millis(5), move || {
            counted.fetch_add(1, Ordering::Relaxed);
            Err(TickError::Gone)
        });

        thread::sleep(Duration::from_millis(100));

        assert_eq!(looks.load(Ordering::Relaxed), 1);
        assert!(matches!(poller.stop(), Err(TickError::Gone)));
    }

    #[test]
    fn panicking_tick_is_reported() {
        let poller = Poller::start_scheduled(
            Duration::from_millis(1),
            || -> Result<Duration, TickError> { panic!("no counter") },
        );

        thread::sleep(Duration::from_millis(50));

        assert_eq!(
            poller.stop().unwrap_err().to_string(),
            "the polling thread panicked: no counter"
        );
    }
}

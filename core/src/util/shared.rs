//! A value written by a polling thread and consumed by a sensor through a lock-free triple
//! buffer. Each `consume` resets the value for the producer.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use triple_buffer::{Input, Output, triple_buffer};

#[derive(Clone)]
struct Published<T> {
    phase: u64,
    value: T,
}

pub fn shared<T: Default + Clone + Send>() -> (Producer<T>, Consumer<T>) {
    let (input, output) = triple_buffer(&Published {
        phase: 0,
        value: T::default(),
    });
    let phase = Arc::new(AtomicU64::new(0));

    (
        Producer {
            input,
            phase: Arc::clone(&phase),
            open: 0,
            value: T::default(),
        },
        Consumer {
            output,
            phase,
            open: 0,
        },
    )
}

pub struct Producer<T: Send> {
    input: Input<Published<T>>,
    phase: Arc<AtomicU64>,
    open: u64,
    value: T,
}

impl<T: Default + Clone + Send> Producer<T> {
    /// Updates the value and publishes it.
    pub fn write(&mut self, f: impl FnOnce(&mut T)) {
        let phase = self.phase.load(Ordering::Acquire);
        if phase != self.open {
            self.open = phase;
            self.value = T::default();
        }

        f(&mut self.value);
        self.input.write(Published {
            phase,
            value: self.value.clone(),
        });
    }
}

pub struct Consumer<T: Send> {
    output: Output<Published<T>>,
    phase: Arc<AtomicU64>,
    open: u64,
}

impl<T: Default + Clone + Send> Consumer<T> {
    /// Takes what was written since the last call. A write racing with this call is dropped,
    /// never counted twice.
    pub fn consume(&mut self) -> T {
        let published = self.output.read();
        let value = if published.phase == self.open {
            published.value.clone()
        } else {
            T::default()
        };

        self.open += 1;
        self.phase.store(self.open, Ordering::Release);

        value
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;

    #[test]
    fn consuming_the_value_starts_it_over_for_the_producer() {
        let (mut producer, mut consumer) = shared::<Vec<i32>>();

        producer.write(|values| values.push(1));
        producer.write(|values| values.push(2));
        assert_eq!(consumer.consume(), [1, 2]);

        producer.write(|values| values.push(3));
        assert_eq!(consumer.consume(), [3]);
    }

    #[test]
    fn a_phase_the_producer_never_wrote_in_comes_back_empty() {
        let (mut producer, mut consumer) = shared::<Vec<i32>>();

        producer.write(|values| values.push(1));
        assert_eq!(consumer.consume(), [1]);
        assert_eq!(consumer.consume(), Vec::<i32>::new());
    }

    #[test]
    fn nothing_is_counted_twice_between_the_producer_and_the_consumer() {
        const WRITES: u64 = 100_000;
        let (mut producer, mut consumer) = shared::<u64>();

        let poller = thread::spawn(move || {
            for _ in 0..WRITES {
                producer.write(|count| *count += 1);
            }
        });

        let mut total = 0;
        while !poller.is_finished() {
            total += consumer.consume();
        }
        poller.join().unwrap();
        total += consumer.consume();

        assert!(total <= WRITES, "{total} counted out of {WRITES} written");
        assert!(total > 0, "nothing got through");
    }
}

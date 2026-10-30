use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use triple_buffer::{Input, Output, triple_buffer};

/// A value, with the phase it was written in.
#[derive(Clone)]
struct Published<T> {
    phase: u64,
    value: T,
}

/// Creates a value shared between a producer and a consumer, starting from its default.
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

/// The writing side of a [`shared`] value, owned by the polling thread.
pub struct Producer<T: Send> {
    input: Input<Published<T>>,

    /// The current phase, started by the consumer.
    phase: Arc<AtomicU64>,

    /// The phase `value` belongs to.
    open: u64,

    value: T,
}

impl<T: Default + Clone + Send> Producer<T> {
    /// Updates the value of the current phase and publishes it.
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

/// The reading side of a [`shared`] value, owned by the sensor.
pub struct Consumer<T: Send> {
    output: Output<Published<T>>,

    /// The current phase, shared with the producer.
    phase: Arc<AtomicU64>,

    /// The current phase.
    open: u64,
}

impl<T: Default + Clone + Send> Consumer<T> {
    /// Returns the value written since the previous call, and starts a new phase. A write
    /// racing with this call is dropped, never counted twice.
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
    fn consume_resets() {
        let (mut producer, mut consumer) = shared::<Vec<i32>>();

        producer.write(|values| values.push(1));
        producer.write(|values| values.push(2));
        assert_eq!(consumer.consume(), [1, 2]);

        producer.write(|values| values.push(3));
        assert_eq!(consumer.consume(), [3]);
    }

    #[test]
    fn unwritten_phase_is_empty() {
        let (mut producer, mut consumer) = shared::<Vec<i32>>();

        producer.write(|values| values.push(1));
        assert_eq!(consumer.consume(), [1]);
        assert_eq!(consumer.consume(), Vec::<i32>::new());
    }

    #[test]
    fn nothing_counted_twice() {
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

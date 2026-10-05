use std::sync::Arc;
use std::time::{Duration, Instant};

use bitflags::bitflags;
use joule_profiler_core::sensor::Sensor;
use joule_profiler_core::util::poller::Poller;
use joule_profiler_core::util::shared::{Consumer, Producer, shared};
use joule_profiler_core::util::stats::MinMaxMean;

use crate::error::{NvmlError, Result};
use crate::hardware::{NvmlHardware, NvmlWrapperHardware};

bitflags! {
    /// What a GPU is asked for.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct DeviceSupport: u8 {
        /// The GPU has an energy counter.
        const Energy = 1;

        /// No energy counter: the polled power is integrated instead.
        const Power = 1 << 1;

        const Vram = 1 << 2;
        const Utilization = 1 << 3;
    }
}

impl DeviceSupport {
    pub(crate) const ENERGY: Self = Self::Energy.union(Self::Power);

    /// What needs a polling thread.
    pub(crate) const POLLED: Self = Self::Power.union(Self::Vram).union(Self::Utilization);
}

#[derive(Debug, Clone, Copy)]
pub struct Device {
    /// The NVML index.
    pub(crate) index: u32,

    pub(crate) support: DeviceSupport,
}

pub(crate) type Devices = Arc<[Device]>;

/// Whether a GPU needs the polling thread. In minimal mode, a GPU with its own energy counter
/// does not.
pub(crate) fn polls(devices: &[Device]) -> bool {
    devices
        .iter()
        .any(|device| device.support.intersects(DeviceSupport::POLLED))
}

/// One GPU at a boundary.
#[derive(Default, Clone, Copy)]
pub struct DeviceSample {
    /// The GPU energy counter, in millijoules.
    pub(crate) energy_mj: Option<u64>,

    /// The integrated power since the last boundary, in nanojoules.
    pub(crate) drawn_nj: u64,

    pub(crate) vram: MinMaxMean,
    pub(crate) utilization: MinMaxMean,
}

pub struct Snapshot {
    pub(crate) devices: Vec<DeviceSample>,
}

/// What the polling thread read of one GPU since the last boundary.
#[derive(Default, Clone)]
struct PolledDevice {
    /// In nanojoules.
    drawn_nj: u64,

    vram: MinMaxMean,
    utilization: MinMaxMean,
}

pub struct NvmlSensor<H: NvmlHardware = NvmlWrapperHardware> {
    hardware: Arc<H>,
    devices: Devices,

    poll_interval: Duration,
    /// One per GPU.
    producers: Vec<Producer<PolledDevice>>,
    consumers: Vec<Consumer<PolledDevice>>,
    poller: Option<Poller>,
}

impl<H: NvmlHardware> NvmlSensor<H> {
    pub(crate) fn new(hardware: Arc<H>, devices: Devices, poll_interval: Duration) -> Self {
        let (producers, consumers) = devices.iter().map(|_| shared()).unzip();

        Self {
            hardware,
            devices,
            poll_interval,
            producers,
            consumers,
            poller: None,
        }
    }
}

impl<H: NvmlHardware> Sensor for NvmlSensor<H> {
    type Snapshot = Snapshot;

    type Error = NvmlError;

    fn name(&self) -> &'static str {
        "nvml"
    }

    /// Starts polling, if needed, before the target starts.
    fn init(&mut self) -> Result<()> {
        if !polls(&self.devices) {
            return Ok(());
        }

        let hardware = Arc::clone(&self.hardware);
        let devices = Arc::clone(&self.devices);
        let mut producers = std::mem::take(&mut self.producers);

        let mut last = vec![None; devices.len()];

        log::debug!(
            "nvml: looking at {} GPUs every {:?}",
            devices.len(),
            self.poll_interval
        );

        self.poller = Some(Poller::start(self.poll_interval, move || {
            sample(hardware.as_ref(), &devices, &mut producers, &mut last)
        }));

        Ok(())
    }

    fn measure(&mut self) -> Result<Snapshot> {
        let mut devices = Vec::with_capacity(self.devices.len());
        for (device, consumer) in self.devices.iter().zip(&mut self.consumers) {
            let polled = consumer.consume();

            let energy_mj = device
                .support
                .contains(DeviceSupport::Energy)
                .then(|| self.hardware.get_energy(*device))
                .transpose()?;

            devices.push(DeviceSample {
                energy_mj,
                drawn_nj: polled.drawn_nj,
                vram: polled.vram,
                utilization: polled.utilization,
            });
        }

        Ok(Snapshot { devices })
    }

    fn close(&mut self) -> Result<()> {
        // Dropping the poller joins its thread.
        self.poller = None;
        Ok(())
    }
}

/// Polls the power, VRAM and utilization of every GPU.
fn sample<H: NvmlHardware>(
    hardware: &H,
    devices: &[Device],
    producers: &mut [Producer<PolledDevice>],
    last: &mut [Option<(Instant, u32)>],
) -> Result<()> {
    let at = Instant::now();

    // Everything is read before anything is written, so a failed read drops the whole poll.
    let mut looks = Vec::with_capacity(devices.len());

    for device in devices {
        looks.push(Look {
            power: device
                .support
                .contains(DeviceSupport::Power)
                .then(|| hardware.get_power(*device))
                .transpose()?,
            vram: device
                .support
                .contains(DeviceSupport::Vram)
                .then(|| hardware.get_vram_usage(*device))
                .transpose()?,
            load: device
                .support
                .contains(DeviceSupport::Utilization)
                .then(|| hardware.get_utilization(*device))
                .transpose()?
                .map(u64::from),
        });
    }

    for ((producer, last), look) in producers.iter_mut().zip(last.iter_mut()).zip(looks) {
        let drawn_nj = look.power.map_or(0, |power| integrate(last, at, power));

        producer.write(|polled| {
            polled.drawn_nj += drawn_nj;
            if let Some(vram) = look.vram {
                polled.vram.add(vram);
            }
            if let Some(load) = look.load {
                polled.utilization.add(load);
            }
        });
    }

    Ok(())
}

/// The energy since the last power sample, in nanojoules (trapezoid rule).
fn integrate(last: &mut Option<(Instant, u32)>, at: Instant, milli_watts: u32) -> u64 {
    let drawn = last.map_or(0, |(before, previous)| {
        let micros = u64::try_from(at.duration_since(before).as_micros()).unwrap_or(u64::MAX);
        u64::from(previous.midpoint(milli_watts)) * micros
    });

    *last = Some((at, milli_watts));
    drawn
}

struct Look {
    power: Option<u32>,
    vram: Option<u64>,
    load: Option<u64>,
}

#[cfg(test)]
mod tests {
    use std::thread::sleep;

    use joule_profiler_core::sensor::Sensor;

    use super::*;
    use crate::hardware::MockNvmlHardware;

    fn device(support: DeviceSupport) -> Devices {
        vec![Device { index: 0, support }].into()
    }

    #[test]
    fn a_gpu_with_its_own_counter_is_not_polled() {
        let sensor = NvmlSensor::new(
            Arc::new(MockNvmlHardware::default()),
            device(DeviceSupport::Energy),
            Duration::from_millis(10),
        );

        assert!(!polls(&sensor.devices));
    }

    #[test]
    fn a_gpu_that_only_reports_its_power_is_polled_and_integrated() {
        let mut hardware = MockNvmlHardware::default();
        hardware.expect_get_power().returning(|_| Ok(50_000));

        let mut sensor = NvmlSensor::new(
            Arc::new(hardware),
            device(DeviceSupport::Power),
            Duration::from_millis(10),
        );

        assert!(polls(&sensor.devices));

        sensor.init().expect("opens");
        let previous = sensor.measure().expect("measures");
        sleep(Duration::from_millis(100));
        let current = sensor.measure().expect("measures");
        sensor.close().expect("closes");

        // The exact value depends on how many polls ran.
        let drawn = current.devices[0].drawn_nj;

        assert!(previous.devices[0].energy_mj.is_none());
        assert!(drawn > 0, "nothing was integrated between the boundaries");
    }
}

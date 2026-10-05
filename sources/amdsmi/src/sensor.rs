use std::sync::Arc;
use std::time::{Duration, Instant};

use bitflags::bitflags;
use joule_profiler_core::sensor::Sensor;
use joule_profiler_core::util::poller::Poller;
use joule_profiler_core::util::shared::{Consumer, Producer, shared};
use joule_profiler_core::util::stats::MinMaxMean;

use crate::error::{AmdSmiError, Result};
use crate::hardware::{AmdSmiHardware, AmdSmiWrapperHardware};

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

#[derive(Debug, Clone)]
pub struct Device {
    pub(crate) uuid: String,

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
    /// The GPU energy counter, in microjoules.
    pub(crate) energy_uj: Option<u64>,

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

pub struct AmdSmiSensor<H: AmdSmiHardware = AmdSmiWrapperHardware> {
    hardware: Arc<H>,
    devices: Devices,

    poll_interval: Duration,
    /// One per GPU.
    producers: Vec<Producer<PolledDevice>>,
    consumers: Vec<Consumer<PolledDevice>>,
    poller: Option<Poller>,
}

impl<H: AmdSmiHardware> AmdSmiSensor<H> {
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

impl<H: AmdSmiHardware> Sensor for AmdSmiSensor<H> {
    type Snapshot = Snapshot;

    type Error = AmdSmiError;

    fn name(&self) -> &'static str {
        "amdsmi"
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
            "amdsmi: looking at {} GPUs every {:?}",
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

            let energy_uj = device
                .support
                .contains(DeviceSupport::Energy)
                .then(|| self.hardware.get_energy(device))
                .transpose()?;

            devices.push(DeviceSample {
                energy_uj,
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
fn sample<H: AmdSmiHardware>(
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
                .then(|| hardware.get_power(device))
                .transpose()?,
            vram: device
                .support
                .contains(DeviceSupport::Vram)
                .then(|| hardware.get_vram_usage(device))
                .transpose()?,
            load: device
                .support
                .contains(DeviceSupport::Utilization)
                .then(|| hardware.get_utilization(device))
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
    use crate::hardware::MockAmdSmiHardware;

    fn device(support: DeviceSupport) -> Devices {
        vec![Device {
            uuid: "5bff74bf-0000-1000-80ab-6aa8d0e43a2c".to_owned(),
            support,
        }]
        .into()
    }

    #[test]
    fn a_gpu_with_its_own_counter_is_not_polled() {
        let sensor = AmdSmiSensor::new(
            Arc::new(MockAmdSmiHardware::default()),
            device(DeviceSupport::Energy),
            Duration::from_millis(10),
        );

        assert!(!polls(&sensor.devices));
    }

    #[test]
    fn a_gpu_that_only_reports_its_power_is_polled_and_integrated() {
        let mut hardware = MockAmdSmiHardware::default();
        hardware.expect_get_power().returning(|_| Ok(50_000));

        let mut sensor = AmdSmiSensor::new(
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

        assert!(previous.devices[0].energy_uj.is_none());
        assert!(drawn > 0, "nothing was integrated between the boundaries");
    }
}

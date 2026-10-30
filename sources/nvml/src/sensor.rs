use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use bitflags::bitflags;
use joule_profiler_core::{
    Sensor,
    util::{
        poller::Poller,
        shared::{Consumer, Producer, shared},
        stats::MinMaxMean,
    },
};

use crate::{
    error::{NvmlError, Result},
    hardware::{NvmlHardware, NvmlWrapperHardware},
};

bitflags! {
    /// What is read from a GPU.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) struct DeviceSupport: u8 {
        /// The energy counter.
        const Energy = 1;

        /// The power, polled and integrated, when the GPU has no energy counter.
        const Power = 1 << 1;

        /// The VRAM used.
        const Vram = 1 << 2;

        /// The utilization.
        const Utilization = 1 << 3;
    }
}

impl DeviceSupport {
    /// Either way of reading the energy.
    pub(crate) const ENERGY: Self = Self::Energy.union(Self::Power);

    /// What is read by the polling thread.
    pub(crate) const POLLED: Self = Self::Power.union(Self::Vram).union(Self::Utilization);
}

/// A GPU to measure.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Device {
    /// The NVML index.
    pub(crate) index: u32,

    pub(crate) support: DeviceSupport,
}

pub(crate) type Devices = Arc<[Device]>;

/// Returns whether a GPU needs the polling thread.
pub(crate) fn needs_polling(devices: &[Device]) -> bool {
    devices
        .iter()
        .any(|device| device.support.intersects(DeviceSupport::POLLED))
}

/// What a GPU reported at a boundary.
#[derive(Default, Clone, Copy)]
pub(crate) struct DeviceSample {
    /// The energy counter, in millijoules.
    pub(crate) energy_mj: Option<u64>,

    /// The power integrated since the previous boundary, in nanojoules.
    pub(crate) drawn_nj: u64,

    pub(crate) vram: MinMaxMean,
    pub(crate) utilization: MinMaxMean,
}

/// What the sensor reads at a boundary, one sample per GPU.
pub(crate) struct Snapshot {
    pub(crate) devices: Vec<DeviceSample>,
}

/// What the polling thread read of a GPU since the previous boundary.
#[derive(Default, Clone)]
struct PolledDevice {
    drawn_nj: u64,
    vram: MinMaxMean,
    utilization: MinMaxMean,
}

/// Reads the energy counters at each boundary, and polls the power, VRAM and utilization.
pub(crate) struct NvmlSensor<H: NvmlHardware = NvmlWrapperHardware> {
    hardware: Arc<H>,
    devices: Devices,
    poll_interval: Duration,

    producers: Vec<Producer<PolledDevice>>,
    consumers: Vec<Consumer<PolledDevice>>,
    poller: Option<Poller<NvmlError>>,
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

    /// Starts polling, if a GPU needs it.
    fn init(&mut self) -> Result<()> {
        if !needs_polling(&self.devices) {
            return Ok(());
        }

        let hardware = Arc::clone(&self.hardware);
        let devices = Arc::clone(&self.devices);
        let mut producers = std::mem::take(&mut self.producers);
        let mut last_power = vec![None; devices.len()];

        log::debug!(
            "nvml: polling {} GPUs every {:?}",
            devices.len(),
            self.poll_interval
        );

        self.poller = Some(Poller::start(self.poll_interval, move || {
            poll_devices(hardware.as_ref(), &devices, &mut producers, &mut last_power)
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
                .then(|| self.hardware.read_energy(*device))
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
        if let Some(poller) = self.poller.take() {
            poller.stop()?;
        }
        Ok(())
    }
}

/// What one poll read of a GPU.
struct Reading {
    power: Option<u32>,
    vram: Option<u64>,
    utilization: Option<u64>,
}

/// Polls the power, VRAM and utilization of every GPU. A failed read drops the whole poll.
fn poll_devices<H: NvmlHardware>(
    hardware: &H,
    devices: &[Device],
    producers: &mut [Producer<PolledDevice>],
    last_power: &mut [Option<(Instant, u32)>],
) -> Result<()> {
    let at = Instant::now();

    let readings = devices
        .iter()
        .map(|device| {
            let read = |support| device.support.contains(support);
            Ok(Reading {
                power: read(DeviceSupport::Power)
                    .then(|| hardware.read_power(*device))
                    .transpose()?,
                vram: read(DeviceSupport::Vram)
                    .then(|| hardware.read_vram_usage(*device))
                    .transpose()?,
                utilization: read(DeviceSupport::Utilization)
                    .then(|| hardware.read_utilization(*device))
                    .transpose()?
                    .map(u64::from),
            })
        })
        .collect::<Result<Vec<_>>>()?;

    for ((producer, last_power), reading) in producers.iter_mut().zip(last_power).zip(readings) {
        let drawn_nj = reading
            .power
            .map_or(0, |power| integrate_power(last_power, at, power));

        producer.write(|polled| {
            polled.drawn_nj += drawn_nj;
            if let Some(vram) = reading.vram {
                polled.vram.add(vram);
            }
            if let Some(utilization) = reading.utilization {
                polled.utilization.add(utilization);
            }
        });
    }

    Ok(())
}

/// Returns the energy drawn since the previous power reading, in nanojoules, with the
/// trapezoidal rule.
fn integrate_power(last_power: &mut Option<(Instant, u32)>, at: Instant, milliwatts: u32) -> u64 {
    let drawn = last_power.map_or(0, |(before, previous)| {
        let micros = u64::try_from(at.duration_since(before).as_micros()).unwrap_or(u64::MAX);
        u64::from(previous.midpoint(milliwatts)) * micros
    });

    *last_power = Some((at, milliwatts));
    drawn
}

#[cfg(test)]
mod tests {
    use std::thread::sleep;

    use super::*;
    use crate::hardware::MockNvmlHardware;

    fn device(support: DeviceSupport) -> Devices {
        vec![Device { index: 0, support }].into()
    }

    #[test]
    fn trapezoidal_integration() {
        let start = Instant::now();
        let mut last_power = None;

        assert_eq!(integrate_power(&mut last_power, start, 10_000), 0);
        assert_eq!(
            integrate_power(&mut last_power, start + Duration::from_millis(2), 30_000),
            40_000_000
        );
    }

    #[test]
    fn power_only_gpu_is_polled() {
        let mut hardware = MockNvmlHardware::default();
        hardware.expect_read_power().returning(|_| Ok(50_000));

        let mut sensor = NvmlSensor::new(
            Arc::new(hardware),
            device(DeviceSupport::Power),
            Duration::from_millis(10),
        );

        sensor.init().unwrap();
        let previous = sensor.measure().unwrap();
        sleep(Duration::from_millis(100));
        let current = sensor.measure().unwrap();
        sensor.close().unwrap();

        assert!(previous.devices[0].energy_mj.is_none());
        assert!(current.devices[0].drawn_nj > 0);
    }
}

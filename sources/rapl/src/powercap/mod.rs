//! Module `rapl` - Intel RAPL metric source.
//!
//! This module provides an implementation of a [`MetricReader`] for
//! collecting energy metrics from Intel RAPL (Running Average Power Limit) domains.
//!
//! The `Rapl` struct manages RAPL domains, reads energy counters,
//! and optionally supports periodic polling for continuous measurement.
//!
//! # Features
//!
//! - Discover available RAPL domains under a given path.
//! - Read instantaneous energy consumption snapshots.
//! - Compute energy usage between consecutive snapshots.
//! - Provide sensors information for integration with the profiler.
//! - Optional async scheduler for periodic measurement.
//!
//! # Usage
//!
//! ```no_run
//! use joule_profiler_source_rapl::powercap;
//! use joule_profiler_core::source::MetricReader;
//!
//! #[tokio::main]
//! async fn main() {
//!     // Initialize a RAPL reader (no polling, monitoring all sockets)
//!     let mut rapl = powercap::Rapl::try_default().unwrap();
//!
//!     // Measure and update internal counters
//!     rapl.measure().await.unwrap();
//!
//!     // Retrieve available sensors
//!     let sensors = rapl.get_sensors().unwrap();
//!
//!     // Retrieve collected counters
//!     let counters = rapl.retrieve().await.unwrap();
//! }
//! ```
//!
//! # Errors
//!
//! All RAPL operations return a [`RaplError`]. Possible errors include:
//! - [`RaplError::RaplNotAvailable`] - no RAPL domains found at the specified path.
//! - [`RaplError::InsufficientPermissions`] - requires elevated privileges to read powercap files.
//! - [`RaplError::UnsupportedOS`] - only Linux is supported.
//! - [`RaplError::RaplReadError`] or [`RaplError::InvalidRaplPath`] - problems reading counters or invalid paths.

use crate::MICRO_JOULE_UNIT;
use crate::error::RaplError;
use crate::powercap::compute::compute_measurement_from_snapshots;
use crate::powercap::config::RaplConfig;
use crate::powercap::domain::{RaplDomain, get_domains, read_energy};
use crate::snapshot::Snapshot;
use crate::util::check_os;
use futures::StreamExt;
use joule_profiler_core::sensor::{Sensor, Sensors};
use joule_profiler_core::source::MetricReader;
use joule_profiler_core::types::{Metric, Metrics};
use log::{debug, error, info, trace};
use std::collections::HashSet;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::{collections::HashMap, env, time::Duration};
use tokio::task::JoinHandle;
use tokio_timerfd::Interval;

mod compute;
pub mod config;
mod domain;
mod socket;

const DEFAULT_RAPL_PATH: &str = "/sys/devices/virtual/powercap/intel-rapl";
const POWERCAP_SOURCE_NAME: &str = "RAPL (Powercap)";

/// Custom result type for Rapl
type Result<T> = std::result::Result<T, RaplError>;

/// RAPL metric source providing energy metrics from Intel RAPL.
pub struct Rapl {
    rapl_path: String,

    /// Managed RAPL domains discovered under the base path. Each domain corresponds.
    domains: Vec<RaplDomain>,

    poll_interval: Option<Duration>,

    /// Current energy counters.
    current_counters: Arc<Mutex<Snapshot>>,

    /// Latest energy counters collected by this reader.
    last_snapshot: Arc<Mutex<Option<Snapshot>>>,

    /// The handle to the polling task for joining if polling is active.
    handle: Option<JoinHandle<Result<()>>>,
}

impl Rapl {
    /// Creates a new RAPL reader for the given path and sockets.
    ///
    /// `rapl_path` - base path to RAPL domains (e.g., `/sys/devices/virtual/powercap/intel-rapl`)
    /// `sockets` - optional set of CPU sockets to monitor
    /// `poll_interval` - optional interval for periodic measurement
    ///
    /// # Errors
    ///
    /// Returns a `RaplError` if:
    /// - RAPL interface is unavailable
    /// - Path is invalid
    /// - Permissions are insufficient
    pub fn new(
        rapl_path: Option<&str>,
        sockets_spec: Option<&HashSet<u32>>,
        poll_interval: Option<Duration>,
    ) -> Result<Self> {
        let rapl_path = rapl_base_path(rapl_path);

        trace!(
            "Attempting to initialize RAPL reader: rapl_path={rapl_path}, sockets={sockets_spec:?}"
        );

        check_os()?;

        let domains = get_domains(&rapl_path, sockets_spec)?;

        if domains.is_empty() {
            error!("No domain discovered");
            return Err(RaplError::NoDomains);
        }

        info!("Discovered {} RAPL domain(s)", domains.len());

        trace!(
            "Creating Rapl instance (domains={}, ticker={})",
            domains.len(),
            poll_interval.is_some()
        );

        Ok(Rapl {
            rapl_path,
            domains,
            poll_interval,
            current_counters: Arc::default(),
            last_snapshot: Arc::default(),
            handle: None,
        })
    }

    /// Initializes an Rapl source with default configuration.
    pub fn try_default() -> Result<Self> {
        Rapl::new(None, None, None)
    }

    /// Reads a snapshot of current energy counters for all domains.
    ///
    /// Returns a `Snapshot` containing the energy in microjoules.
    fn read_snapshot(&self) -> Result<Snapshot> {
        trace!(
            "Reading energy snapshot from {} RAPL domain(s)",
            self.domains.len()
        );

        let mut map = HashMap::with_capacity(self.domains.len());

        for domain in &self.domains {
            trace!(
                "Reading domain: type={:?} socket={}",
                domain.domain_type, domain.socket
            );

            let val_uj = read_energy(domain)?;
            trace!("Energy read: {val_uj} µJ");

            map.insert((domain.domain_type, domain.socket), val_uj);
        }

        Ok(Snapshot { metrics: map })
    }
}

impl MetricReader for Rapl {
    type Type = Snapshot;
    type Error = RaplError;
    type Config = RaplConfig;

    fn from_config(config: Self::Config) -> Result<Self> {
        Self::new(
            config.rapl_path.as_deref(),
            config.sockets_spec.as_ref(),
            config.poll_interval,
        )
    }

    async fn init(&mut self, _: i32) -> Result<()> {
        check_rapl_access(&self.rapl_path)?;

        let mut ticker = if let Some(interval) = self.poll_interval {
            Interval::new_interval(interval)?
        } else {
            return Ok(());
        };

        let domains = self.domains.clone();
        let last_snapshot = self.last_snapshot.clone();
        let current_counters = self.current_counters.clone();

        let handle = tokio::spawn(async move {
            loop {
                ticker.next().await;
                trace!("Rapl polled");

                let Ok(mut last_guard) = last_snapshot.try_lock() else {
                    trace!("RAPL measure skipped: previous measurement still running");
                    continue;
                };

                let new_snapshot = {
                    let mut map = HashMap::with_capacity(domains.len());
                    for domain in &domains {
                        let domain_index = (domain.domain_type, domain.socket);
                        let val_uj = read_energy(domain)?;
                        map.insert(domain_index, val_uj);
                    }
                    Snapshot { metrics: map }
                };

                if let Some(prev) = last_guard.as_ref() {
                    let metrics =
                        compute_measurement_from_snapshots(&domains, prev, &new_snapshot)?;

                    let mut counters = current_counters
                        .lock()
                        .map_err(|_| RaplError::MutexPoisoned)?;
                    *counters += metrics;
                }

                *last_guard = Some(new_snapshot);
            }
        });

        self.handle = Some(handle);
        Ok(())
    }

    async fn join(&mut self) -> Result<()> {
        if let Some(handle) = self.handle.take() {
            if handle.is_finished() {
                if let Ok(res) = handle.await {
                    return res;
                }
            } else {
                handle.abort();
            }
        }
        Ok(())
    }

    async fn measure(&mut self) -> Result<()> {
        let new_snapshot = self.read_snapshot()?;

        let mut last = self
            .last_snapshot
            .lock()
            .map_err(|_| RaplError::MutexPoisoned)?;

        if let Some(prev) = last.as_ref() {
            let metrics = compute_measurement_from_snapshots(&self.domains, prev, &new_snapshot)?;

            let mut counters = self
                .current_counters
                .lock()
                .map_err(|_| RaplError::MutexPoisoned)?;
            *counters += metrics;
        }

        *last = Some(new_snapshot);
        Ok(())
    }

    async fn retrieve(&mut self) -> Result<Snapshot> {
        let mut lock = self
            .current_counters
            .lock()
            .map_err(|_| RaplError::MutexPoisoned)?;
        Ok(std::mem::take(&mut *lock))
    }

    fn get_sensors(&self) -> Result<Sensors> {
        trace!("Building RAPL sensor list");

        let sensors = self
            .domains
            .iter()
            .map(|domain| {
                trace!("Registering sensor: {}", domain.get_name());
                Sensor::new(domain.get_name(), MICRO_JOULE_UNIT, Self::get_name())
            })
            .collect();

        Ok(sensors)
    }

    fn to_metrics(&self, snapshot: Self::Type) -> Result<Metrics> {
        Ok(snapshot
            .metrics
            .into_iter()
            .map(|((domain, socket), value)| {
                Metric::new(
                    domain.to_string_socket(socket),
                    value,
                    MICRO_JOULE_UNIT,
                    Self::get_name(),
                )
            })
            .collect())
    }

    fn get_name() -> &'static str {
        POWERCAP_SOURCE_NAME
    }

    fn get_id() -> &'static str {
        "rapl"
    }
}

/// Resolves the RAPL base path from configuration and environment.
fn rapl_base_path(config_override: Option<&str>) -> String {
    if let Some(path) = config_override {
        return path.to_string();
    }

    if let Ok(env_path) = env::var("JOULE_PROFILER_RAPL_PATH") {
        return env_path;
    }

    DEFAULT_RAPL_PATH.to_string()
}

/// Check if the program can read RAPL powercap files
fn check_rapl_access(base: &str) -> Result<()> {
    debug!("Checking RAPL access using base path: {base}");
    let path = Path::new(base);

    let entries = fs::read_dir(path).map_err(|e| match e.kind() {
        ErrorKind::PermissionDenied => RaplError::InsufficientPermissions,
        _ => e.into(),
    })?;

    for entry in entries.flatten() {
        let energy_path = entry.path().join("energy_uj");
        if energy_path.exists() {
            fs::read_to_string(&energy_path).map_err(|e| {
                if e.kind() == ErrorKind::PermissionDenied {
                    RaplError::InsufficientPermissions
                } else {
                    e.into()
                }
            })?;
            return Ok(());
        }
    }

    Err(RaplError::RaplNotAvailable(format!(
        "{base} is not an RAPL folder"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rapl_base_path_uses_override() {
        let path = rapl_base_path(Some("/custom/path"));
        assert_eq!(path, "/custom/path");
    }
}

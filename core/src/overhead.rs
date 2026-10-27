//! Estimation of Joule Profiler's own CPU and energy overhead during a run.
//!
//! Because hardware energy counters such as Intel/AMD RAPL measure socket-wide or
//! domain-wide energy (`PACKAGE`, `CORE`, `UNCORE`, `DRAM`, `PSYS`), the energy
//! consumed by the `joule-profiler` process itself (polling sources and detecting
//! phase tokens) is included in the raw measurements.
//!
//! To provide a control variable without introducing extra runtime polling threads,
//! [`OverheadSnapshot::capture`] records two lightweight boundary readings at the
//! start and end of the measurement window:
//! 1. Cumulative CPU time of the `joule-profiler` process across all its threads via
//!    `clock_gettime(CLOCK_PROCESS_CPUTIME_ID)`.
//! 2. Cumulative system-wide active CPU time from `/sys/fs/cgroup/cpu.stat`
//!    (`usage_usec`) with fallback to `/proc/stat`.
//!
//! At the end of the run, [`ProfilerOverhead::from_snapshots`] computes the
//! profiler's CPU share and attributes a proportional share of the measured RAPL
//! energy domains.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
#[cfg(unix)]
use std::mem::MaybeUninit;

use serde::Serialize;

use crate::aggregate::{Metric, MetricValue};
use crate::profiler::types::Phase;
use crate::unit::{MetricUnit, Unit, UnitPrefix};

/// Default paths for reading system-wide active CPU time on Linux.
const CGROUP_CPU_STAT_PATH: &str = "/sys/fs/cgroup/cpu.stat";
const PROC_STAT_PATH: &str = "/proc/stat";

/// Microjoule unit used for estimated energy overhead metrics.
const MICRO_JOULE_UNIT: MetricUnit = MetricUnit {
    prefix: UnitPrefix::Micro,
    unit: Unit::Joule,
};

/// Boundary snapshot of profiler and system active CPU time in microseconds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OverheadSnapshot {
    /// Cumulative CPU time (user + system) of the `joule-profiler` process in microseconds.
    pub profiler_cpu_us: u64,

    /// Cumulative active (non-idle) system-wide CPU time in microseconds.
    pub system_cpu_us: u64,
}

impl OverheadSnapshot {
    /// Captures a boundary snapshot of the current process and system CPU time.
    ///
    /// Uses `CLOCK_PROCESS_CPUTIME_ID` for sub-microsecond process CPU accounting
    /// across all profiler threads, and `/sys/fs/cgroup/cpu.stat` (or `/proc/stat`)
    /// for system-wide busy CPU time.
    pub fn capture() -> Self {
        Self {
            profiler_cpu_us: read_profiler_cpu_time_us(),
            system_cpu_us: read_system_cpu_time_us().unwrap_or(0),
        }
    }
}

/// Estimated resource and energy overhead of `JouleProfiler` over a profiling run.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ProfilerOverhead {
    /// CPU time (user + kernel) consumed by `JouleProfiler` during the run, in microseconds.
    pub cpu_time_us: u64,

    /// Total active system-wide CPU time during the run, in microseconds.
    pub system_cpu_time_us: u64,

    /// Share of active system CPU time consumed by `JouleProfiler` (`0.0` to `100.0`%).
    pub cpu_share_percent: f64,

    /// Headline non-overlapping RAPL energy estimated to be consumed by `JouleProfiler` (`µJ`).
    ///
    /// `None` when no RAPL energy source was active during the run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_energy_uj: Option<u64>,

    /// Per-domain estimated RAPL energy consumed by `JouleProfiler` (`µJ`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub domains: Vec<Metric>,
}

impl ProfilerOverhead {
    /// Computes the profiler overhead between `begin` and `end` snapshots across `phases`.
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    pub fn from_snapshots(
        begin: OverheadSnapshot,
        end: OverheadSnapshot,
        phases: &[Phase],
    ) -> Self {
        let cpu_time_us = end.profiler_cpu_us.saturating_sub(begin.profiler_cpu_us);
        let raw_system_us = end.system_cpu_us.saturating_sub(begin.system_cpu_us);
        let system_cpu_time_us = raw_system_us.max(cpu_time_us);

        let ratio = if system_cpu_time_us > 0 {
            (cpu_time_us as f64 / system_cpu_time_us as f64).clamp(0.0, 1.0)
        } else {
            0.0
        };

        let cpu_share_percent = (ratio * 10_000.0).round() / 100.0;

        // Aggregate total energy (in µJ) per RAPL domain across all phases.
        let mut domain_totals_uj: BTreeMap<String, (u64, String)> = BTreeMap::new();

        for phase in phases {
            for metric in &phase.metrics {
                if !is_rapl_energy_metric(metric) {
                    continue;
                }
                if let Some(energy_uj) = metric_to_microjoules(metric) {
                    let entry = domain_totals_uj
                        .entry(metric.name.clone())
                        .or_insert_with(|| (0, metric.source.clone()));
                    entry.0 = entry.0.saturating_add(energy_uj);
                }
            }
        }

        if domain_totals_uj.is_empty() {
            return Self {
                cpu_time_us,
                system_cpu_time_us,
                cpu_share_percent,
                estimated_energy_uj: None,
                domains: Vec::new(),
            };
        }

        let domains: Vec<Metric> = domain_totals_uj
            .iter()
            .map(|(name, (total_uj, source))| {
                let est_uj = (*total_uj as f64 * ratio).round() as u64;
                Metric::new(name.clone(), est_uj, MICRO_JOULE_UNIT, source.clone())
            })
            .collect();

        let non_overlapping_total_uj = sum_non_overlapping_rapl_domains(&domain_totals_uj);
        let estimated_energy_uj = Some((non_overlapping_total_uj as f64 * ratio).round() as u64);

        Self {
            cpu_time_us,
            system_cpu_time_us,
            cpu_share_percent,
            estimated_energy_uj,
            domains,
        }
    }
}

/// Checks whether a metric is a host CPU/DRAM energy measurement from RAPL.
fn is_rapl_energy_metric(metric: &Metric) -> bool {
    metric.unit.unit == Unit::Joule
        && metric
            .source
            .get(..4)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("rapl"))
}

/// Converts an energy [`Metric`] value into microjoules (`µJ`).
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn metric_to_microjoules(metric: &Metric) -> Option<u64> {
    if metric.unit.unit != Unit::Joule {
        return None;
    }

    let scale_to_uj: f64 = match metric.unit.prefix {
        UnitPrefix::Nano => 1e-3,
        UnitPrefix::Micro => 1.0,
        UnitPrefix::Milli => 1e3,
        UnitPrefix::None => 1e6,
        UnitPrefix::Kilo => 1e9,
        UnitPrefix::Mega => 1e12,
        UnitPrefix::Giga => 1e15,
    };

    let raw = match metric.value {
        MetricValue::UnsignedInteger(v) => v as f64,
        MetricValue::SignedInteger(v) if v >= 0 => v as f64,
        MetricValue::Float(v, _) if v >= 0.0 => v,
        MetricValue::SignedInteger(_) | MetricValue::Float(_, _) => return None,
    };

    Some((raw * scale_to_uj).round() as u64)
}

/// Sums RAPL domain energies without double-counting nested domains.
///
/// Intel/AMD RAPL domain hierarchy:
/// - `PSYS` encompasses the entire platform (all `PACKAGE` and `DRAM` domains).
/// - `PACKAGE-<s>` encompasses `CORE-<s>` and `UNCORE-<s>` on socket `s`, while
///   `DRAM-<s>` is a separate memory controller domain.
fn sum_non_overlapping_rapl_domains(domains: &BTreeMap<String, (u64, String)>) -> u64 {
    if let Some((psys_uj, _)) = domains
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("PSYS") || name.starts_with("PSYS-"))
        .map(|(_, val)| val)
    {
        return *psys_uj;
    }

    let mut sockets = BTreeSet::new();
    let mut unindexed_total = 0_u64;

    for (name, (val, _)) in domains {
        if let Some((_, socket_str)) = name.rsplit_once('-') {
            sockets.insert(socket_str.to_owned());
        } else {
            unindexed_total = unindexed_total.saturating_add(*val);
        }
    }

    if sockets.is_empty() {
        return unindexed_total;
    }

    let get_domain = |prefix: &str, socket: &str| -> Option<u64> {
        let target = format!("{prefix}-{socket}");
        domains
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(&target))
            .map(|(_, (v, _))| *v)
    };

    let mut total_uj = unindexed_total;
    for socket in sockets {
        let pkg_or_subdomains = get_domain("PACKAGE", &socket).unwrap_or_else(|| {
            get_domain("CORE", &socket).unwrap_or(0) + get_domain("UNCORE", &socket).unwrap_or(0)
        });
        let dram = get_domain("DRAM", &socket).unwrap_or(0);
        total_uj = total_uj
            .saturating_add(pkg_or_subdomains)
            .saturating_add(dram);
    }

    total_uj
}

/// Reads cumulative CPU time (`user + system`) of the current process in microseconds.
#[cfg(unix)]
#[allow(clippy::cast_sign_loss)]
fn read_profiler_cpu_time_us() -> u64 {
    let mut ts = MaybeUninit::<libc::timespec>::zeroed();
    // SAFETY: `ts` points to a valid `libc::timespec` struct.
    let rc = unsafe { libc::clock_gettime(libc::CLOCK_PROCESS_CPUTIME_ID, ts.as_mut_ptr()) };
    if rc == 0 {
        // SAFETY: `clock_gettime` succeeded and initialized `ts`.
        let ts = unsafe { ts.assume_init() };
        if ts.tv_sec >= 0 && ts.tv_nsec >= 0 {
            return (ts.tv_sec as u64)
                .saturating_mul(1_000_000)
                .saturating_add((ts.tv_nsec as u64) / 1_000);
        }
    }
    0
}

#[cfg(not(unix))]
fn read_profiler_cpu_time_us() -> u64 {
    0
}

/// Reads cumulative system-wide active CPU time in microseconds.
///
/// Prefers `/sys/fs/cgroup/cpu.stat` (`usage_usec`) for microsecond precision,
/// falling back to `/proc/stat` busy CPU ticks.
fn read_system_cpu_time_us() -> Option<u64> {
    if let Ok(content) = fs::read_to_string(CGROUP_CPU_STAT_PATH)
        && let Some(usage_usec) = parse_cgroup_usage_usec(&content)
    {
        return Some(usage_usec);
    }

    let content = fs::read_to_string(PROC_STAT_PATH).ok()?;
    let ticks_per_sec = read_clk_tck();
    parse_proc_stat_busy_us(&content, ticks_per_sec)
}

/// Parses `usage_usec <value>` from cgroup v2 `cpu.stat` content.
fn parse_cgroup_usage_usec(content: &str) -> Option<u64> {
    content.lines().find_map(|line| {
        let (key, value) = line.split_once(char::is_whitespace)?;
        if key == "usage_usec" {
            value.trim().parse::<u64>().ok()
        } else {
            None
        }
    })
}

/// Parses the aggregate `cpu` line from `/proc/stat` and returns busy time in microseconds.
///
/// `/proc/stat` fields after `cpu`:
/// `user` (0), `nice` (1), `system` (2), `idle` (3), `iowait` (4), `irq` (5), `softirq` (6), `steal` (7).
/// Busy ticks are `user + nice + system + irq + softirq + steal`.
fn parse_proc_stat_busy_us(content: &str, ticks_per_sec: u64) -> Option<u64> {
    if ticks_per_sec == 0 {
        return None;
    }

    let cpu_line = content.lines().find(|line| line.starts_with("cpu "))?;

    let fields: Vec<u64> = cpu_line
        .split_ascii_whitespace()
        .skip(1)
        .filter_map(|tok| tok.parse::<u64>().ok())
        .collect();

    if fields.len() < 4 {
        return None;
    }

    let user = fields[0];
    let nice = fields[1];
    let system = fields[2];
    let irq = fields.get(5).copied().unwrap_or(0);
    let softirq = fields.get(6).copied().unwrap_or(0);
    let steal = fields.get(7).copied().unwrap_or(0);

    let busy_ticks = user
        .saturating_add(nice)
        .saturating_add(system)
        .saturating_add(irq)
        .saturating_add(softirq)
        .saturating_add(steal);

    u64::try_from(u128::from(busy_ticks) * 1_000_000 / u128::from(ticks_per_sec)).ok()
}

/// Queries `_SC_CLK_TCK` via `sysconf`, defaulting to 100 Hz if unavailable.
#[cfg(unix)]
#[allow(clippy::cast_sign_loss)]
fn read_clk_tck() -> u64 {
    // SAFETY: `sysconf(_SC_CLK_TCK)` is thread-safe and has no preconditions.
    let val = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if val > 0 { val as u64 } else { 100 }
}

#[cfg(not(unix))]
fn read_clk_tck() -> u64 {
    100
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phase::PhaseToken;

    fn phase_with_metrics(index: usize, metrics: Vec<Metric>) -> Phase {
        Phase {
            index,
            start_token: PhaseToken::Start,
            end_token: PhaseToken::End,
            timestamp: 1_000,
            duration_ms: 100,
            start_token_line: None,
            end_token_line: None,
            metrics,
        }
    }

    fn rapl_metric(name: &str, value: u64) -> Metric {
        Metric::new(name, value, MICRO_JOULE_UNIT, "RAPL (perf_event)")
    }

    #[test]
    fn parse_cgroup_usage_usec_extracts_value() {
        let sample = "usage_usec 482910\nuser_usec 300000\nsystem_usec 182910\n";
        assert_eq!(parse_cgroup_usage_usec(sample), Some(482_910));
    }

    #[test]
    fn parse_cgroup_usage_usec_missing_returns_none() {
        assert_eq!(parse_cgroup_usage_usec("user_usec 100\n"), None);
    }

    #[test]
    fn parse_proc_stat_busy_us_excludes_idle_and_iowait() {
        let sample = "cpu  100 20 30 5000 400 5 10 15 0 0\ncpu0 50 10 15 2500 200 2 5 7 0 0\n";
        assert_eq!(parse_proc_stat_busy_us(sample, 100), Some(1_800_000));
    }

    #[test]
    fn parse_proc_stat_busy_us_invalid_returns_none() {
        assert_eq!(parse_proc_stat_busy_us("intr 1 2 3\n", 100), None);
        assert_eq!(parse_proc_stat_busy_us("cpu  10 20 30 40\n", 0), None);
    }

    #[test]
    fn from_snapshots_without_rapl_computes_cpu_share_only() {
        let begin = OverheadSnapshot {
            profiler_cpu_us: 1_000,
            system_cpu_us: 100_000,
        };
        let end = OverheadSnapshot {
            profiler_cpu_us: 3_500,
            system_cpu_us: 200_000,
        };
        let count_unit = MetricUnit {
            prefix: UnitPrefix::None,
            unit: Unit::Count,
        };
        let phases = vec![phase_with_metrics(
            0,
            vec![Metric::new(
                "CPU_CYCLES",
                50_000_u64,
                count_unit,
                "perf_event",
            )],
        )];

        let overhead = ProfilerOverhead::from_snapshots(begin, end, &phases);
        assert_eq!(overhead.cpu_time_us, 2_500);
        assert_eq!(overhead.system_cpu_time_us, 100_000);
        assert!((overhead.cpu_share_percent - 2.5).abs() < f64::EPSILON);
        assert_eq!(overhead.estimated_energy_uj, None);
        assert_eq!(overhead.domains.len(), 0);
    }

    #[test]
    fn from_snapshots_attributes_rapl_energy_and_avoids_core_double_counting() {
        let begin = OverheadSnapshot {
            profiler_cpu_us: 0,
            system_cpu_us: 0,
        };
        let end = OverheadSnapshot {
            profiler_cpu_us: 5_000,
            system_cpu_us: 100_000,
        };

        let phases = vec![
            phase_with_metrics(
                0,
                vec![
                    rapl_metric("PACKAGE-0", 4_000),
                    rapl_metric("CORE-0", 2_500),
                    rapl_metric("DRAM-0", 800),
                ],
            ),
            phase_with_metrics(
                1,
                vec![
                    rapl_metric("PACKAGE-0", 6_000),
                    rapl_metric("CORE-0", 3_500),
                    rapl_metric("DRAM-0", 1_200),
                ],
            ),
        ];

        let overhead = ProfilerOverhead::from_snapshots(begin, end, &phases);
        assert_eq!(overhead.cpu_time_us, 5_000);
        assert_eq!(overhead.system_cpu_time_us, 100_000);
        assert!((overhead.cpu_share_percent - 5.0).abs() < f64::EPSILON);
        assert_eq!(overhead.estimated_energy_uj, Some(600));
        assert_eq!(overhead.domains.len(), 3);
        assert_eq!(overhead.domains[0].name, "CORE-0");
        assert_eq!(overhead.domains[0].value, MetricValue::UnsignedInteger(300));
        assert_eq!(overhead.domains[1].name, "DRAM-0");
        assert_eq!(overhead.domains[1].value, MetricValue::UnsignedInteger(100));
        assert_eq!(overhead.domains[2].name, "PACKAGE-0");
        assert_eq!(overhead.domains[2].value, MetricValue::UnsignedInteger(500));
    }

    #[test]
    fn from_snapshots_falls_back_to_core_and_uncore_when_package_absent() {
        let begin = OverheadSnapshot {
            profiler_cpu_us: 0,
            system_cpu_us: 0,
        };
        let end = OverheadSnapshot {
            profiler_cpu_us: 10_000,
            system_cpu_us: 100_000,
        };
        let phases = vec![phase_with_metrics(
            0,
            vec![
                rapl_metric("CORE-0", 7_000),
                rapl_metric("UNCORE-0", 1_000),
                rapl_metric("DRAM-0", 2_000),
            ],
        )];

        let overhead = ProfilerOverhead::from_snapshots(begin, end, &phases);
        assert_eq!(overhead.estimated_energy_uj, Some(1_000));
    }

    #[test]
    fn from_snapshots_prefers_psys_when_available() {
        let begin = OverheadSnapshot {
            profiler_cpu_us: 0,
            system_cpu_us: 0,
        };
        let end = OverheadSnapshot {
            profiler_cpu_us: 10_000,
            system_cpu_us: 100_000,
        };
        let phases = vec![phase_with_metrics(
            0,
            vec![
                rapl_metric("PSYS", 20_000),
                rapl_metric("PACKAGE-0", 12_000),
                rapl_metric("DRAM-0", 3_000),
            ],
        )];

        let overhead = ProfilerOverhead::from_snapshots(begin, end, &phases);
        assert_eq!(overhead.estimated_energy_uj, Some(2_000));
    }

    #[test]
    fn from_snapshots_clamps_system_cpu_to_profiler_cpu() {
        let begin = OverheadSnapshot {
            profiler_cpu_us: 100,
            system_cpu_us: 5_000,
        };
        let end = OverheadSnapshot {
            profiler_cpu_us: 600,
            system_cpu_us: 5_000,
        };
        let phases = vec![phase_with_metrics(0, vec![rapl_metric("PACKAGE-0", 1_000)])];

        let overhead = ProfilerOverhead::from_snapshots(begin, end, &phases);
        assert_eq!(overhead.cpu_time_us, 500);
        assert_eq!(overhead.system_cpu_time_us, 500);
        assert!((overhead.cpu_share_percent - 100.0).abs() < f64::EPSILON);
        assert_eq!(overhead.estimated_energy_uj, Some(1_000));
    }

    #[test]
    fn capture_returns_monotonic_profiler_cpu_time() {
        let s1 = OverheadSnapshot::capture();
        let s2 = OverheadSnapshot::capture();
        assert!(s2.profiler_cpu_us >= s1.profiler_cpu_us);
    }
}

use crate::domain_type::RaplDomainType;
use thiserror::Error;

/// Errors that can occur when using the RAPL source.
#[derive(Debug, Error)]
pub enum RaplError {
    /// Intel RAPL interface is not available on this system.
    #[error("Intel RAPL not available: {0}")]
    RaplNotAvailable(String),

    /// No RAPL domains were discovered on this system.
    #[error("No RAPL domains found")]
    NoDomains,

    /// Failed to read a value from a RAPL domain.
    #[error("Failed to read RAPL domain: {0}")]
    RaplReadError(String),

    /// A RAPL sysfs path could not be parsed or resolved.
    #[error("Invalid RAPL domain path: {0}")]
    InvalidRaplPath(String),

    /// The socket specification string is malformed.
    #[error("Invalid socket specification: {0}")]
    InvalidSocketSpec(String),

    /// The requested socket index does not exist in the discovered RAPL domains.
    #[error("Socket {0} not found in available RAPL domains")]
    SocketNotFound(u32),

    /// A required system command was not found.
    #[error("Command not found: {0}")]
    CommandNotFound(String),

    /// RAPL is only supported on Linux.
    #[error("Unsupported operating system: {0}. Only Linux is supported")]
    UnsupportedOS(String),

    /// The process lacks the required permissions to access RAPL counters.
    #[error("Insufficient permissions to access RAPL counters. Try running with sudo")]
    InsufficientPermissions,

    /// A RAPL domain name could not be matched to a known type.
    #[error("Unknown domain {0}")]
    UnknownDomain(String),

    /// No CPU in the socket was able to open the perf counter for this domain.
    /// It can also appear if `perf_event_open` returns an error other than unsupported event for a specific CPU.
    #[error("Failed to open domain counter, {0}")]
    FailToOpenDomainCounter(String),

    /// The event format string for this domain could not be parsed.
    #[error("Invalid event format for domain {0}")]
    InvalidEventFormat(String),

    /// This RAPL domain is not supported by the hardware.
    #[error("Domain {0} not supported")]
    DomainNotSupported(RaplDomainType),

    /// The `perf_event_paranoid` level is too restrictive.
    #[error(transparent)]
    PerfParanoid(#[from] PerfParanoidError),

    /// The energy scale factor could not be retrieved from the perf event.
    #[error("Cannot retrieve perf RAPL scale")]
    RetrieveScaleError,

    /// Generic I/O error not covered by other variants.
    #[error(transparent)]
    IoError(std::io::Error),

    /// Failed to parse an energy value as an integer.
    #[error("Failed to parse energy value")]
    ParseEnergyError(
        #[from]
        #[source]
        std::num::ParseIntError,
    ),

    /// Failed to parse a domain scale value as a float.
    #[error("Failed to parse domain scale")]
    ParseDomainScale(
        #[from]
        #[source]
        std::num::ParseFloatError,
    ),

    /// Not enough snapshots have been taken to compute an energy delta.
    #[error("Not enough measures to compute RAPL counters differences")]
    NotEnoughSamples,

    #[error("RAPL source mutex poisoned.")]
    MutexPoisoned,
}

impl From<std::io::Error> for RaplError {
    fn from(err: std::io::Error) -> Self {
        match err.kind() {
            std::io::ErrorKind::NotFound => RaplError::RaplNotAvailable(err.to_string()),
            std::io::ErrorKind::PermissionDenied => RaplError::InsufficientPermissions,
            _ => RaplError::IoError(err),
        }
    }
}

/// Errors related to `/proc/sys/kernel/perf_event_paranoid`.
#[derive(Debug, Error)]
pub enum PerfParanoidError {
    /// The `perf_event_paranoid` file does not exist on this system.
    #[error("perf_event_paranoid file not found")]
    NotFound,

    /// The `perf_event_paranoid` file could not be read due to a lack of permissions.
    #[error("perf_event_paranoid not readable: {0}")]
    PermissionDenied(String),

    /// The paranoid level is too high to allow unprivileged perf access.
    #[error(
        "perf_event_paranoid level is {0}, try setting it to 0 or launch profiler with root rights"
    )]
    ParanoidLevelTooHigh(i8),

    /// Generic I/O error while reading the paranoid file.
    #[error(transparent)]
    IoError(std::io::Error),

    /// Failed to parse the paranoid level as an integer.
    #[error("Failed to parse paranoid level")]
    ParseParanoidLevelError(
        #[from]
        #[source]
        std::num::ParseIntError,
    ),
}

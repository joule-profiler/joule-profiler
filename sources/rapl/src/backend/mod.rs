use joule_profiler_core::info::Info;

use crate::domain::RaplDomain;
use crate::error::Result;

#[cfg(feature = "backend-perf")]
pub(crate) mod perf;

#[cfg(feature = "backend-powercap")]
pub(crate) mod powercap;

/// A backend to read the RAPL energy counters.
pub(crate) trait RaplBackend: Send + 'static {
    /// Returns the info of the backend.
    fn info(&self) -> Info;

    /// Returns the domains, in the order of the readings.
    fn domains(&self) -> &[RaplDomain];

    /// Opens the counters.
    fn open(&mut self) -> Result<()> {
        Ok(())
    }

    /// Reads the counter of every domain.
    fn read(&mut self) -> Result<Vec<u64>>;

    /// Close the opened counters.
    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

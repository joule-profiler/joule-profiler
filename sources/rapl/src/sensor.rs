use joule_profiler_core::sensor::Sensor;

use crate::backend::RaplBackend;
use crate::error::{RaplError, Result};

/// Façade for the backends implementing [`Sensor`].
pub(crate) struct RaplSensor<B: RaplBackend> {
    backend: B,
}

impl<B: RaplBackend> RaplSensor<B> {
    pub(crate) fn new(backend: B) -> Self {
        Self { backend }
    }
}

impl<B: RaplBackend> Sensor for RaplSensor<B> {
    type Snapshot = Vec<u64>;

    type Error = RaplError;

    fn name(&self) -> &'static str {
        "RAPL"
    }

    fn init(&mut self) -> Result<()> {
        self.backend.open()
    }

    fn measure(&mut self) -> Result<Self::Snapshot> {
        self.backend.read()
    }

    fn close(&mut self) -> Result<()> {
        self.backend.close()
    }
}

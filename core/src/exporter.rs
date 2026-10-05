use crate::error::{Error, Result};
use crate::phase::{PhaseInfo, SourceMetrics, Summary};
use crate::schema::Schema;

/// Writes out the phases of a run.
pub trait Exporter: Send + 'static {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Called before the target is resumed.
    fn begin(&mut self, _schema: &Schema) -> Result<(), Self::Error> {
        Ok(())
    }

    fn export(
        &mut self,
        phase: &PhaseInfo,
        sources: &[SourceMetrics<'_>],
    ) -> Result<(), Self::Error>;

    /// Called when no phase is waiting.
    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Called once the run is over.
    fn finish(&mut self, _summary: &Summary) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// Type-erased [`Exporter`].
pub(crate) trait AnyExporter: Send {
    fn begin(&mut self, schema: &Schema) -> Result<()>;
    fn export(&mut self, phase: &PhaseInfo, sources: &[SourceMetrics<'_>]) -> Result<()>;
    fn flush(&mut self) -> Result<()>;
    fn finish(&mut self, summary: &Summary) -> Result<()>;
}

impl<E: Exporter> AnyExporter for E {
    fn begin(&mut self, schema: &Schema) -> Result<()> {
        Exporter::begin(self, schema).map_err(Error::exporter)
    }

    fn export(&mut self, phase: &PhaseInfo, sources: &[SourceMetrics<'_>]) -> Result<()> {
        Exporter::export(self, phase, sources).map_err(Error::exporter)
    }

    fn flush(&mut self) -> Result<()> {
        Exporter::flush(self).map_err(Error::exporter)
    }

    fn finish(&mut self, summary: &Summary) -> Result<()> {
        Exporter::finish(self, summary).map_err(Error::exporter)
    }
}

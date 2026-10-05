use crate::injector::Target;

/// Takes the measurements of a source, on its own thread. Turning them into metrics is left to
/// its [`crate::processor::Processor`].
pub trait Sensor: Send + 'static {
    type Snapshot: Send + 'static;
    type Error: std::error::Error + Send + Sync + 'static;

    fn name(&self) -> &str;

    /// Called before the target is started.
    fn init(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Called once the target is started, before it is resumed.
    fn attach(&mut self, _target: Target) -> Result<(), Self::Error> {
        Ok(())
    }

    fn measure(&mut self) -> Result<Self::Snapshot, Self::Error>;

    /// Called once the run is over, even if `init` failed or was never called.
    fn close(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

use crate::{source::MetricSource, types::Metrics};

pub trait Processor<S: MetricSource>: Send {
    fn consume(
        &mut self,
        snapshot: S::Snapshot,
    ) -> impl Future<Output = Result<Option<Metrics>, S::Error>> + Send;

    fn init(&mut self) -> impl Future<Output = Result<(), S::Error>> + Send {
        async { Ok(()) }
    }

    fn clean(&mut self) -> impl Future<Output = Result<(), S::Error>> + Send {
        async { Ok(()) }
    }
}

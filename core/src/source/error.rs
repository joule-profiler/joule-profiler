use std::{fmt::Debug, time::Duration};
use thiserror::Error;

/// Errors that can occur when reading or aggregating metrics from a source.
///
/// This enum is used by all metric sources implementing [`MetricReader`](`super::MetricReader`)
/// to signal failures during measurements.
#[derive(Debug, Error)]
pub enum MetricSourceError {
    /// The source failed to retrieve its internal counters.
    #[error("Error retrieving source counters")]
    ErrorRetrievingCounters,

    /// The initialization of the source lasted more than the authorized time.
    #[error(
        "source initialization timed out after {0:?}. Use --init-timeout to increase the limit."
    )]
    InitTimeout(Duration),

    /// Error propagated from a custom metric source.
    #[error(transparent)]
    SourceError(#[from] Box<dyn std::error::Error + Send + Sync>),
}

/// Converts any compatible error into a [`MetricSourceError`].
///
/// Implemented for all types that are [`std::error::Error`] + [`Send`] + [`Sync`] + `'static`,
/// wrapping them in [`MetricSourceError::SourceError`].
pub trait IntoMetricSourceError {
    fn into_metric_source_error(self) -> MetricSourceError;
}

impl<T> IntoMetricSourceError for T
where
    T: std::error::Error + Send + Sync + 'static,
{
    fn into_metric_source_error(self) -> MetricSourceError {
        MetricSourceError::SourceError(Box::new(self))
    }
}

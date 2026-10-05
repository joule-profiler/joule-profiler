use perf_event::{Builder, events::Dynamic};

use crate::{
    domain::RaplDomain,
    error::{RaplError, Result},
    perf::PMU,
};

pub(crate) fn build_counter(domain: &'_ RaplDomain) -> Result<Builder<'_>> {
    let mut event = Dynamic::builder(PMU).map_err(RaplError::Io)?;
    event
        .event(domain.domain.to_perf_event())
        .map_err(RaplError::Io)?;
    let event = event.build().map_err(|e| RaplError::Event(e.to_string()))?;

    let mut builder = Builder::new(event);
    builder
        .any_pid()
        .one_cpu(domain.cpu as usize)
        .exclude_kernel(false)
        .exclude_hv(false);

    Ok(builder)
}

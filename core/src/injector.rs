use std::error::Error;

use thiserror::Error;

use crate::types::{InitContext, PhaseStartInfo};

#[derive(Debug, Error)]
#[error("injector error")]
pub struct InjectorError(#[from] Box<dyn std::error::Error + Send + Sync>);

impl InjectorError {
    pub fn boxed<E: Error + Send + Sync + 'static>(error: E) -> Self {
        Self(Box::new(error))
    }
}

pub trait Injector {
    type Error: Error + Send + Sync + 'static;

    fn init(&mut self) -> impl Future<Output = Result<InitContext, Self::Error>>;

    fn resume(&mut self) -> impl Future<Output = Result<(), Self::Error>> {
        async { Ok(()) }
    }

    fn next(&mut self) -> impl Future<Output = Result<Option<PhaseStartInfo>, Self::Error>>;

    fn abort(&mut self) -> impl Future<Output = Result<(), Self::Error>> {
        async { Ok(()) }
    }
}

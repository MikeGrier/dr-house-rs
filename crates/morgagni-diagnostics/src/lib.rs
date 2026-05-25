//! Investigators that consume a [`TraceBackend`](crate::backend::TraceBackend)
//! and produce structured diagnostic reports.
//!
//! # Status
//!
//! Placeholder scaffolding. The contract is defined so the rest of Dr
//! House can wire against it today; the first real investigator
//! (null-pointer dereference) will land against the mock backend in
//! `morgagni-diagnostics::backend::mock` before any real TTD wiring.

#![forbid(unsafe_code)]

pub mod backend;
pub mod investigator;
pub mod report;

pub use investigator::investigate;
pub use report::{InvestigationReport, RootCause};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("investigation not yet implemented")]
    NotImplemented,
    #[error("backend error: {0}")]
    Backend(String),
}

pub type Result<T> = std::result::Result<T, Error>;

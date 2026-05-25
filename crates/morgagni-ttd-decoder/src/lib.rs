//! morgagni-ttd-decoder
//!
//! Provides Rust bindings and decoding utilities for the Windows Time Travel
//! Debugger (TTD) replay API.  The native TTD DLLs are downloaded at build
//! time by `.github/scripts/download-ttd.ps1` and placed in
//! `extension/resources/ttd/{arch}/`.
//!
//! On Windows this crate exposes [`RunTrace`], a
//! [`morgagni_diagnostics::backend::TraceBackend`] implementation that opens
//! a `.run` file via the SDK and answers diagnostic queries on demand. On
//! non-Windows targets the crate is intentionally empty.

#[cfg(target_os = "windows")]
mod run_trace;

#[cfg(target_os = "windows")]
pub use run_trace::RunTrace;

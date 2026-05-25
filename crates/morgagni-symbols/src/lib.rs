//! Symbol resolution and symbol management for Morgagni.
//!
//! This crate is the boundary between raw addresses produced by
//! [`morgagni-ttd-decoder`] (module identity + RVA) and human-readable
//! `module!function+offset` strings. It also owns symbol-management
//! concerns: PDB search paths, symbol-server URLs, and on-disk cache
//! policy.
//!
//! # Status
//!
//! Placeholder. The public types here define the contract the rest of Dr
//! House will program against; the real PDB / `dbghelp` / symbol-server
//! implementation is deferred. All resolver methods currently return
//! `Ok(None)` (unresolved) so callers can wire the type into pipelines
//! today and get real names later without an API break.

#![forbid(unsafe_code)]

use std::path::PathBuf;

/// Identifies a loaded module precisely enough to fetch the matching PDB
/// from a symbol server.
///
/// The `timestamp` / `age` / `guid` triple is the canonical Microsoft
/// symbol-server key; producers (such as the TTD decoder) should fill all
/// three when they have them.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModuleId {
    pub name: String,
    pub base: u64,
    pub size: u64,
    pub timestamp: u32,
    pub age: u32,
    pub guid: [u8; 16],
}

/// A resolved symbolic location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSymbol {
    pub module: String,
    pub function: String,
    pub offset: u32,
    pub source: Option<SourceLocation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLocation {
    pub file: PathBuf,
    pub line: u32,
}

/// Configuration for a [`Resolver`]. Mirrors the conceptual surface of
/// `_NT_SYMBOL_PATH` without committing to any particular backend.
#[derive(Debug, Clone, Default)]
pub struct ResolverConfig {
    /// Local directories to search for PDBs.
    pub search_paths: Vec<PathBuf>,
    /// Symbol-server URLs (e.g. `https://msdl.microsoft.com/download/symbols`).
    pub symbol_servers: Vec<String>,
    /// On-disk cache directory for downloaded PDBs.
    pub cache_dir: Option<PathBuf>,
    /// If false, the resolver must not perform network I/O.
    pub allow_network: bool,
}

/// Errors that may be returned by a [`Resolver`].
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("symbol resolution is not yet implemented")]
    NotImplemented,
}

pub type Result<T> = std::result::Result<T, Error>;

/// Resolves addresses to symbolic names.
///
/// The placeholder implementation returns `Ok(None)` for every query.
/// Swap it for a real implementation (e.g. `dbghelp`-backed or pure-Rust
/// `pdb`-backed) without changing this API.
pub struct Resolver {
    _config: ResolverConfig,
}

impl Resolver {
    pub fn new(config: ResolverConfig) -> Self {
        Self { _config: config }
    }

    /// Resolve an absolute address, given the module list that was loaded
    /// at the time the address was captured.
    pub fn resolve_address(
        &self,
        _address: u64,
        _modules: &[ModuleId],
    ) -> Result<Option<ResolvedSymbol>> {
        Ok(None)
    }

    /// Resolve a module + RVA pair.
    pub fn resolve_rva(
        &self,
        _module: &ModuleId,
        _rva: u32,
    ) -> Result<Option<ResolvedSymbol>> {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_resolver_returns_none() {
        let r = Resolver::new(ResolverConfig::default());
        assert!(r.resolve_rva(&dummy_module(), 0x1000).unwrap().is_none());
    }

    fn dummy_module() -> ModuleId {
        ModuleId {
            name: "test.dll".into(),
            base: 0,
            size: 0,
            timestamp: 0,
            age: 0,
            guid: [0; 16],
        }
    }
}

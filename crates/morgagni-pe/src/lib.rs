//! PE/COFF parsing, export resolution, and x64 unwind-aware stack walking
//! for Morgagni.
//!
//! # Status
//!
//! Placeholder. The contract types and the [`ModuleImageReader`] trait
//! are defined so the decoder and diagnostics crates can compile against
//! this crate today; real PE parsing, export lookup, and unwind walking
//! are deferred.
//!
//! # Why this crate exists
//!
//! TTD records the in-memory image of every loaded module. That means
//! the PE header, export directory, and exception/unwind tables of a
//! module are *all available* via a memory read at the module base \u2014
//! no PDB, no symbol server, no on-disk DLL needed.
//!
//! This crate is the layer that turns those bytes into useful answers:
//! function boundaries (from `.pdata` / `RUNTIME_FUNCTION`), export
//! names, and unwind-driven stack walks. Symbol resolution proper
//! (PDB-backed `module!function+offset`) lives in `morgagni-symbols`;
//! this crate is about binary-format introspection only.

#![forbid(unsafe_code)]

use std::io;

/// Capability the caller (typically the TTD decoder) provides so this
/// crate can read PE bytes out of trace memory at a chosen position.
///
/// Implementations should treat `offset` as an offset into the module's
/// in-memory image starting at the module base \u2014 i.e. the same view
/// the running process sees, with sections at their virtual addresses.
pub trait ModuleImageReader {
    fn read(&self, module_base: u64, offset: u64, len: usize) -> io::Result<Vec<u8>>;
}

/// A function discovered via unwind information or exports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionRange {
    /// RVA of the function's first instruction, relative to module base.
    pub start_rva: u32,
    /// RVA one past the function's last instruction (exclusive).
    pub end_rva: u32,
    /// Export name if the function is exported; `None` otherwise.
    pub export_name: Option<String>,
}

/// One frame of a walked stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnwoundFrame {
    pub ip: u64,
    pub sp: u64,
    pub bp: u64,
    /// Module base the IP falls within, if any.
    pub module_base: Option<u64>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("PE introspection not yet implemented")]
    NotImplemented,
    #[error("I/O error reading module image: {0}")]
    Io(#[from] io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Enumerate functions in a module via its exception/unwind table.
///
/// Placeholder: returns `Err(NotImplemented)`.
pub fn enumerate_functions(
    _reader: &dyn ModuleImageReader,
    _module_base: u64,
) -> Result<Vec<FunctionRange>> {
    Err(Error::NotImplemented)
}

/// Resolve an exported symbol name to an RVA within a module.
///
/// Placeholder: returns `Ok(None)`.
pub fn resolve_export(
    _reader: &dyn ModuleImageReader,
    _module_base: u64,
    _name: &str,
) -> Result<Option<u32>> {
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ZeroReader;
    impl ModuleImageReader for ZeroReader {
        fn read(&self, _base: u64, _offset: u64, len: usize) -> io::Result<Vec<u8>> {
            Ok(vec![0u8; len])
        }
    }

    #[test]
    fn enumerate_functions_returns_not_implemented_for_now() {
        assert!(matches!(
            enumerate_functions(&ZeroReader, 0x1000_0000),
            Err(Error::NotImplemented)
        ));
    }

    #[test]
    fn resolve_export_returns_none_for_now() {
        assert!(
            resolve_export(&ZeroReader, 0x1000_0000, "main")
                .unwrap()
                .is_none()
        );
    }
}

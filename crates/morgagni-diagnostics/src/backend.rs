//! Pluggable trace backend.
//!
//! The diagnostics crate is written entirely against this trait so that
//! investigators can be developed and tested against a mock long before
//! the real TTD-backed implementation exists. See the design notes for
//! the rationale (separating "is our query API right?" from "did we get
//! the FFI right?").

use serde::{Deserialize, Serialize};
use std::io;

/// Opaque position in a trace. Comparable and serializable; otherwise
/// the diagnostics layer does not interpret its internals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Position {
    pub sequence: u64,
    pub steps: u64,
}

/// Opaque thread identifier within a trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct ThreadId(pub u32);

/// Architecture-neutral register identifier.
///
/// Initial coverage is x64. We use an enum (not a string) so callers
/// cannot misspell register names and so the mock can exhaustively
/// enumerate what it supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RegId {
    Rax,
    Rbx,
    Rcx,
    Rdx,
    Rsi,
    Rdi,
    Rbp,
    Rsp,
    R8,
    R9,
    R10,
    R11,
    R12,
    R13,
    R14,
    R15,
    Rip,
    Rflags,
}

/// A snapshot of integer-register state at one position.
#[derive(Debug, Clone, Default)]
pub struct Registers {
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub rsp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rip: u64,
    pub rflags: u64,
}

impl Registers {
    pub fn get(&self, reg: RegId) -> u64 {
        match reg {
            RegId::Rax => self.rax,
            RegId::Rbx => self.rbx,
            RegId::Rcx => self.rcx,
            RegId::Rdx => self.rdx,
            RegId::Rsi => self.rsi,
            RegId::Rdi => self.rdi,
            RegId::Rbp => self.rbp,
            RegId::Rsp => self.rsp,
            RegId::R8 => self.r8,
            RegId::R9 => self.r9,
            RegId::R10 => self.r10,
            RegId::R11 => self.r11,
            RegId::R12 => self.r12,
            RegId::R13 => self.r13,
            RegId::R14 => self.r14,
            RegId::R15 => self.r15,
            RegId::Rip => self.rip,
            RegId::Rflags => self.rflags,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ModuleInfo {
    pub name: String,
    pub base: u64,
    pub size: u64,
}

#[derive(Debug, Clone)]
pub struct TerminationEvent {
    pub thread: ThreadId,
    pub position: Position,
    pub kind: TerminationKind,
}

#[derive(Debug, Clone)]
pub enum TerminationKind {
    AccessViolation {
        access: MemoryAccessKind,
        address: u64,
    },
    /// Process terminated via `__fastfail` / `RtlFailFast2`. The faulting
    /// RIP is always inside ntdll's report routine; the true root cause is
    /// upstream and must be located by walking back through user-mode frames.
    /// `noncontinuable` mirrors `EXCEPTION_NONCONTINUABLE` (flags bit 0).
    FastFail {
        code: u32,
        noncontinuable: bool,
    },
    OtherException {
        code: u32,
        address: u64,
    },
    NormalExit {
        code: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemoryAccessKind {
    Read,
    Write,
    Execute,
}

#[derive(Debug, Clone)]
pub struct RawFrame {
    pub ip: u64,
    pub sp: u64,
    pub bp: u64,
}

/// Record of the last write to some storage before a given position.
#[derive(Debug, Clone)]
pub struct WriteRecord {
    pub position: Position,
    pub thread: ThreadId,
    /// IP of the instruction that performed the write.
    pub ip: u64,
    /// Value written (for register writes; the full new register value).
    pub value: u64,
}

/// Errors returned by a [`TraceBackend`]. Kept small on purpose; the
/// real engine will surface richer errors via the decoder crate.
#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    #[error("invalid position")]
    InvalidPosition,
    #[error("unknown thread")]
    UnknownThread,
    #[error("address out of range")]
    OutOfRange,
    #[error("operation not supported by this backend")]
    NotSupported,
    #[error("I/O: {0}")]
    Io(#[from] io::Error),
    #[error("backend internal: {0}")]
    Internal(String),
}

pub type BackendResult<T> = std::result::Result<T, BackendError>;

/// The minimal surface a diagnostics investigator needs.
///
/// The real TTD-backed implementation will live in
/// `morgagni-ttd-decoder` and implement this trait on top of the FFI.
pub trait TraceBackend {
    fn modules(&self) -> BackendResult<Vec<ModuleInfo>>;

    fn termination(&self) -> BackendResult<Option<TerminationEvent>>;

    fn registers(&self, thread: ThreadId, position: Position) -> BackendResult<Registers>;

    /// Read exactly `len` bytes from `address` at `position`.
    ///
    /// On success the returned `Vec<u8>` is guaranteed to have length
    /// `len`. Implementations MUST return `Err(BackendError::OutOfRange)`
    /// when only a prefix of the requested range is available (for
    /// example when the read crosses an unmapped page); they MUST NOT
    /// return a short buffer. Callers rely on the length invariant and
    /// would otherwise read past the end of valid data.
    fn read_memory(&self, position: Position, address: u64, len: usize) -> BackendResult<Vec<u8>>;

    fn stack(&self, thread: ThreadId, position: Position) -> BackendResult<Vec<RawFrame>>;

    /// Find the last write to a register on a thread strictly before
    /// `before`. Returns `Ok(None)` if no such write exists in the
    /// recorded portion of the trace.
    fn last_write_register(
        &self,
        thread: ThreadId,
        reg: RegId,
        before: Position,
    ) -> BackendResult<Option<WriteRecord>>;
}

pub mod json_loader;
pub mod mock;

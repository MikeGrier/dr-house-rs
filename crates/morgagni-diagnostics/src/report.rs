//! Structured investigation reports.
//!
//! These types are the *output* of an investigator. They are designed
//! to be:
//! - cheap to serialize (Copilot consumes JSON, not Rust types),
//! - small enough to fit in a single agent turn,
//! - composable so one investigator's report can be referenced by
//!   another.

use crate::backend::{MemoryAccessKind, Position, RegId, ThreadId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationReport {
    pub summary: String,
    pub root_cause: RootCause,
    pub fault: FaultDetails,
    pub stack: Vec<FrameSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RootCause {
    NullPointerDereference {
        register: RegId,
        last_written_at: Option<Position>,
        last_written_by_ip: Option<u64>,
    },
    Unknown { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FaultDetails {
    pub thread: ThreadId,
    pub position: Position,
    pub access: MemoryAccessKind,
    pub faulting_address: u64,
    pub faulting_ip: u64,
    pub instruction_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameSummary {
    pub ip: u64,
    pub sp: u64,
    pub module: Option<String>,
    pub rva: Option<u32>,
}

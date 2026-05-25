//! Structured investigation reports.
//!
//! These types are the *output* of an investigator. They are designed
//! to be:
//! - cheap to serialize (Copilot consumes JSON, not Rust types),
//! - small enough to fit in a single agent turn,
//! - composable so one investigator's report can be referenced by
//!   another.

use crate::backend::{MemoryAccessKind, Position, RegId, ThreadId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvestigationReport {
    pub summary: String,
    pub root_cause: RootCause,
    pub fault: FaultDetails,
    pub stack: Vec<FrameSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RootCause {
    NullPointerDereference {
        /// Pointer-carrying register identified as the source of the
        /// null/near-null value, when one could be attributed (e.g. the
        /// base or index register of the faulting memory operand).
        /// `None` for absolute-addressing forms like `[0]` where there
        /// is no register to name.
        register: Option<RegId>,
        last_written_at: Option<Position>,
        last_written_by_ip: Option<u64>,
    },
    Unknown {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FaultDetails {
    pub thread: ThreadId,
    pub position: Position,
    pub access: MemoryAccessKind,
    pub faulting_address: u64,
    pub faulting_ip: u64,
    pub instruction_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameSummary {
    pub ip: u64,
    pub sp: u64,
    pub module: Option<String>,
    pub rva: Option<u32>,
}

//! Investigators: consume a [`TraceBackend`] and emit an
//! [`InvestigationReport`].
//!
//! The first investigator targets the most common Rust-on-Windows crash
//! we capture in the fixtures: a null (or near-null) pointer dereference
//! that surfaces as an `EXCEPTION_ACCESS_VIOLATION` near address 0.
//!
//! Strategy:
//!   1. Look at [`TraceBackend::termination`]. Only `AccessViolation`
//!      with a low faulting address is treated as a null-deref candidate.
//!   2. Read the faulting instruction bytes via [`TraceBackend::read_memory`]
//!      at `RIP`, decode with `iced-x86`.
//!   3. Identify the memory operand's base register.
//!   4. Walk [`TraceBackend::last_write_register`] to find where the
//!      null/near-null value was written.

use iced_x86::{Decoder, DecoderOptions, Instruction, OpKind, Register};

use crate::backend::{
    BackendError, MemoryAccessKind, Position, RegId, TerminationEvent, TerminationKind,
    ThreadId, TraceBackend,
};
use crate::report::{FaultDetails, FrameSummary, InvestigationReport, RootCause};
use crate::{Error, Result};

/// Faulting address must be inside the first page (and a bit) to be
/// treated as a null-pointer dereference rather than some other AV.
/// Matches the Windows "NULL page" reservation and a generous slack for
/// `null + small_offset` patterns.
const NULL_PAGE_LIMIT: u64 = 0x1_0000;

/// Try to produce a structured report for whatever killed the process.
pub fn investigate<B: TraceBackend>(backend: &B) -> Result<InvestigationReport> {
    let term = backend
        .termination()
        .map_err(|e| Error::Backend(e.to_string()))?
        .ok_or_else(|| Error::Backend("trace has no termination event".into()))?;

    match term.kind {
        TerminationKind::AccessViolation { access, address } if address < NULL_PAGE_LIMIT => {
            investigate_null_deref(backend, &term, access, address)
        }
        TerminationKind::AccessViolation { access, address } => {
            Ok(unknown_av(backend, &term, access, address))
        }
        _ => Err(Error::NotImplemented),
    }
}

fn investigate_null_deref<B: TraceBackend>(
    backend: &B,
    term: &TerminationEvent,
    access: MemoryAccessKind,
    address: u64,
) -> Result<InvestigationReport> {
    let regs = backend
        .registers(term.thread, term.position)
        .map_err(|e| Error::Backend(e.to_string()))?;

    let bytes = read_instruction_bytes(backend, term.position, regs.rip)?;
    let insn = decode_one(&bytes, regs.rip);
    let text = format_instruction(&insn);

    let base_reg = memory_base_reg(&insn);

    let (last_written_at, last_written_by_ip) = match base_reg {
        Some(reg) => match backend.last_write_register(term.thread, reg, term.position) {
            Ok(Some(w)) => (Some(w.position), Some(w.ip)),
            Ok(None) => (None, None),
            Err(BackendError::NotSupported) => (None, None),
            Err(e) => return Err(Error::Backend(e.to_string())),
        },
        None => (None, None),
    };

    let summary = match base_reg {
        Some(reg) => format!(
            "null-pointer {access:?} via {reg:?} at {addr:#x} ({ip:#x}: {text})",
            access = access,
            reg = reg,
            addr = address,
            ip = regs.rip,
            text = text,
        ),
        None => format!(
            "null-pointer {access:?} at {addr:#x} ({ip:#x}: {text})",
            access = access,
            addr = address,
            ip = regs.rip,
            text = text,
        ),
    };

    Ok(InvestigationReport {
        summary,
        root_cause: RootCause::NullPointerDereference {
            register: base_reg.unwrap_or(RegId::Rip),
            last_written_at,
            last_written_by_ip,
        },
        fault: FaultDetails {
            thread: term.thread,
            position: term.position,
            access,
            faulting_address: address,
            faulting_ip: regs.rip,
            instruction_text: text,
        },
        stack: collect_stack(backend, term.thread, term.position),
    })
}

fn unknown_av<B: TraceBackend>(
    backend: &B,
    term: &TerminationEvent,
    access: MemoryAccessKind,
    address: u64,
) -> InvestigationReport {
    let (rip, text) = match backend.registers(term.thread, term.position) {
        Ok(regs) => {
            let bytes = read_instruction_bytes(backend, term.position, regs.rip)
                .unwrap_or_default();
            let text = if bytes.is_empty() {
                String::new()
            } else {
                format_instruction(&decode_one(&bytes, regs.rip))
            };
            (regs.rip, text)
        }
        Err(_) => (0, String::new()),
    };

    InvestigationReport {
        summary: format!("access violation ({access:?}) at {address:#x}"),
        root_cause: RootCause::Unknown {
            reason: format!("unhandled access violation at {address:#x}"),
        },
        fault: FaultDetails {
            thread: term.thread,
            position: term.position,
            access,
            faulting_address: address,
            faulting_ip: rip,
            instruction_text: text,
        },
        stack: collect_stack(backend, term.thread, term.position),
    }
}

fn collect_stack<B: TraceBackend>(
    backend: &B,
    thread: ThreadId,
    position: Position,
) -> Vec<FrameSummary> {
    let frames = match backend.stack(thread, position) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };
    let modules = backend.modules().unwrap_or_default();
    frames
        .into_iter()
        .map(|f| {
            let (module, rva) = modules
                .iter()
                .find(|m| f.ip >= m.base && f.ip < m.base.saturating_add(m.size))
                .map(|m| (Some(m.name.clone()), Some((f.ip - m.base) as u32)))
                .unwrap_or((None, None));
            FrameSummary { ip: f.ip, sp: f.sp, module, rva }
        })
        .collect()
}

fn read_instruction_bytes<B: TraceBackend>(
    backend: &B,
    position: Position,
    rip: u64,
) -> Result<Vec<u8>> {
    // x86-64 instructions are at most 15 bytes, but `read_memory` may
    // refuse a request that crosses a page or section boundary. Walk
    // down from the max until something sticks.
    let mut last_err: Option<BackendError> = None;
    for len in [15usize, 8, 4, 2, 1] {
        match backend.read_memory(position, rip, len) {
            Ok(b) => return Ok(b),
            Err(e) => last_err = Some(e),
        }
    }
    Err(Error::Backend(
        last_err
            .map(|e| e.to_string())
            .unwrap_or_else(|| "no instruction bytes available".into()),
    ))
}

fn decode_one(bytes: &[u8], rip: u64) -> Instruction {
    let mut decoder = Decoder::with_ip(64, bytes, rip, DecoderOptions::NONE);
    decoder.decode()
}

fn format_instruction(insn: &Instruction) -> String {
    use iced_x86::SpecializedFormatter;
    let mut f: SpecializedFormatter<iced_x86::DefaultSpecializedFormatterTraitOptions> =
        SpecializedFormatter::new();
    let mut out = String::new();
    f.format(insn, &mut out);
    out
}

/// Find the base register of the first memory operand in `insn`, if any.
fn memory_base_reg(insn: &Instruction) -> Option<RegId> {
    for i in 0..insn.op_count() {
        if insn.op_kind(i) == OpKind::Memory {
            return iced_to_regid(insn.memory_base());
        }
    }
    None
}

fn iced_to_regid(r: Register) -> Option<RegId> {
    Some(match r {
        Register::RAX => RegId::Rax,
        Register::RBX => RegId::Rbx,
        Register::RCX => RegId::Rcx,
        Register::RDX => RegId::Rdx,
        Register::RSI => RegId::Rsi,
        Register::RDI => RegId::Rdi,
        Register::RBP => RegId::Rbp,
        Register::RSP => RegId::Rsp,
        Register::R8 => RegId::R8,
        Register::R9 => RegId::R9,
        Register::R10 => RegId::R10,
        Register::R11 => RegId::R11,
        Register::R12 => RegId::R12,
        Register::R13 => RegId::R13,
        Register::R14 => RegId::R14,
        Register::R15 => RegId::R15,
        Register::RIP => RegId::Rip,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::mock::{MockTrace, MockWrite};
    use crate::backend::{
        ModuleInfo, Position, Registers, TerminationEvent, TerminationKind, WriteRecord,
    };

    /// Build a mock trace where:
    ///   mov rax, 0     ; at position {1,0}, ip 0x1400_0_1000
    ///   mov rcx, [rax] ; at position {1,1}, ip 0x1400_0_1003 — crashes
    fn null_deref_scenario() -> MockTrace {
        let thread = ThreadId(1);
        let mut t = MockTrace::new(thread);
        t.modules.push(ModuleInfo {
            name: "demo.exe".into(),
            base: 0x1_4000_0000,
            size: 0x10000,
        });

        // Instruction at the crash: `mov rcx, qword ptr [rax]` = 48 8B 08,
        // padded with NOPs so a 15-byte read succeeds.
        let crash_ip = 0x1_4000_1003u64;
        let mut bytes = vec![0x48, 0x8B, 0x08];
        bytes.extend(std::iter::repeat_n(0x90, 15));
        t.memory.insert(crash_ip, bytes);

        // Register snapshot at the crash: rax = 0 (null), rip points at the load.
        let regs_crash = Registers { rax: 0, rip: crash_ip, ..Default::default() };
        t.register_snapshots.insert(Position { sequence: 1, steps: 1 }, regs_crash);

        // Earlier write that zeroed rax.
        t.writes.push(MockWrite {
            position: Position { sequence: 1, steps: 0 },
            reg: RegId::Rax,
            value: 0,
            ip: 0x1_4000_1000,
        });

        t.termination = Some(TerminationEvent {
            thread,
            position: Position { sequence: 1, steps: 1 },
            kind: TerminationKind::AccessViolation {
                access: MemoryAccessKind::Read,
                address: 0,
            },
        });
        t
    }

    #[test]
    fn null_deref_identifies_base_register_and_writer() {
        let trace = null_deref_scenario();
        let report = investigate(&trace).expect("investigation should succeed");

        assert_eq!(report.fault.faulting_address, 0);
        assert_eq!(report.fault.access, MemoryAccessKind::Read);
        assert_eq!(report.fault.faulting_ip, 0x1_4000_1003);
        // Instruction text should mention a memory load through rax.
        assert!(
            report.fault.instruction_text.to_lowercase().contains("rax"),
            "expected disasm to mention rax, got {:?}",
            report.fault.instruction_text
        );

        match report.root_cause {
            RootCause::NullPointerDereference {
                register,
                last_written_at,
                last_written_by_ip,
            } => {
                assert_eq!(register, RegId::Rax);
                assert_eq!(last_written_at, Some(Position { sequence: 1, steps: 0 }));
                assert_eq!(last_written_by_ip, Some(0x1_4000_1000));
            }
            other => panic!("unexpected root cause: {:?}", other),
        }

        // Module attribution should map the faulting IP back to demo.exe.
        let _ = report.stack; // mock stack is empty by default; not asserted here.
        let _ = WriteRecord {
            // ensure trait re-export is used somewhere
            position: Position { sequence: 0, steps: 0 },
            thread: ThreadId(0),
            ip: 0,
            value: 0,
        };
    }

    #[test]
    fn high_address_av_is_unknown_not_null_deref() {
        let mut trace = null_deref_scenario();
        trace.termination = Some(TerminationEvent {
            thread: ThreadId(1),
            position: Position { sequence: 1, steps: 1 },
            kind: TerminationKind::AccessViolation {
                access: MemoryAccessKind::Read,
                address: 0xDEAD_BEEF_0000,
            },
        });
        let report = investigate(&trace).expect("should still produce a report");
        match report.root_cause {
            RootCause::Unknown { .. } => {}
            other => panic!("expected Unknown, got {:?}", other),
        }
    }
}

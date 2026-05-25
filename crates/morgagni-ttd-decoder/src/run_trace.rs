//! TTD `.run`-backed [`TraceBackend`].
//!
//! This is the production path: investigators on customer machines open a
//! `.run` file directly through the TTD Replay SDK (via
//! `morgagni-ttd-decoder-sys`) and answer queries on demand. There is no
//! JSON serialization in this path. The JSON loader in
//! [`super::json_loader`] exists only for (a) CI testing of investigator
//! logic against committed fixtures, and (b) producing portable bug-report
//! artifacts that users can share with the Morgagni team.
//!
//! # Threading
//!
//! TTD's `IReplayEngine` and its cursors are not internally thread-safe.
//! [`RunTrace`] makes itself `!Send + !Sync` via [`PhantomData`] so the
//! compiler will reject accidental cross-thread sharing. Wrap in a
//! `Mutex` at a higher layer if you really need to share one across
//! threads.

use std::marker::PhantomData;
use std::path::Path;

use morgagni_ttd_decoder_sys as sys;

use morgagni_diagnostics::backend::{
    BackendError, BackendResult, MemoryAccessKind, ModuleInfo, Position, RawFrame, RegId,
    Registers, TerminationEvent, TerminationKind, ThreadId, TraceBackend, WriteRecord,
};

/// A trace opened from a TTD `.run` file. Owns an engine handle and
/// destroys it on drop.
pub struct RunTrace {
    engine: sys::DhttdEngineHandle,
    modules: Vec<ModuleInfo>,
    threads: Vec<sys::DhttdThread>,
    termination: Option<TerminationEvent>,
    _not_send_sync: PhantomData<*mut u8>,
}

impl RunTrace {
    /// Open a `.run` trace. The companion `.idx`/`.out` files produced by
    /// the recorder must be alongside it.
    pub fn from_path(path: impl AsRef<Path>) -> BackendResult<Self> {
        let path = path.as_ref();
        let mut engine: sys::DhttdEngineHandle = 0;
        let rc = unsafe { sys::dhttd_engine_create(&mut engine) };
        if rc != 0 || engine == 0 {
            return Err(BackendError::Internal(format!(
                "CreateReplayEngine failed: 0x{rc:08x}"
            )));
        }
        // Guard against partial construction: if anything below fails,
        // destroy the engine before returning.
        let trace_utf16 = to_utf16_nul(path);
        let ok = unsafe { sys::dhttd_engine_initialize(engine, trace_utf16.as_ptr()) };
        if ok == 0 {
            unsafe { sys::dhttd_engine_destroy(engine) };
            return Err(BackendError::Internal(format!(
                "IReplayEngine::Initialize({}) failed",
                path.display()
            )));
        }

        let modules = read_modules(engine);
        let threads = read_threads(engine);
        let termination = read_first_exception(engine);

        Ok(Self {
            engine,
            modules,
            threads,
            termination,
            _not_send_sync: PhantomData,
        })
    }

    fn thread(&self, id: ThreadId) -> BackendResult<&sys::DhttdThread> {
        self.threads
            .iter()
            .find(|t| t.unique_id == id.0)
            .ok_or(BackendError::UnknownThread)
    }
}

impl Drop for RunTrace {
    fn drop(&mut self) {
        if self.engine != 0 {
            unsafe { sys::dhttd_engine_destroy(self.engine) };
        }
    }
}

impl TraceBackend for RunTrace {
    fn modules(&self) -> BackendResult<Vec<ModuleInfo>> {
        Ok(self.modules.clone())
    }

    fn termination(&self) -> BackendResult<Option<TerminationEvent>> {
        Ok(self.termination.clone())
    }

    fn registers(&self, thread: ThreadId, position: Position) -> BackendResult<Registers> {
        let t = self.thread(thread)?;
        if !position_in_range(position, t.active_time) {
            return Err(BackendError::InvalidPosition);
        }
        let cur = Cursor::new(self.engine)?;
        let p = to_sys_position(position);
        unsafe { sys::dhttd_cursor_set_position_on_thread(cur.handle, t.unique_id, p) };
        let mut regs = sys::DhttdAmd64Registers::default();
        let ok = unsafe { sys::dhttd_cursor_amd64_registers(cur.handle, &mut regs) };
        if ok == 0 {
            return Err(BackendError::Internal(
                "cursor amd64_registers returned 0".into(),
            ));
        }
        Ok(convert_registers(&regs))
    }

    fn read_memory(&self, position: Position, address: u64, len: usize) -> BackendResult<Vec<u8>> {
        if len == 0 {
            return Ok(Vec::new());
        }
        let cur = Cursor::new(self.engine)?;
        let p = to_sys_position(position);
        unsafe { sys::dhttd_cursor_set_position(cur.handle, p) };
        let mut buf = vec![0u8; len];
        let n = unsafe {
            sys::dhttd_cursor_read_memory(cur.handle, address, buf.as_mut_ptr(), buf.len())
        };
        if n < len {
            // Honor the TraceBackend contract: short reads are an
            // OutOfRange error, not a successful partial read.
            return Err(BackendError::OutOfRange);
        }
        Ok(buf)
    }

    fn stack(&self, _thread: ThreadId, _position: Position) -> BackendResult<Vec<RawFrame>> {
        // Frame-pointer / unwind synthesis is a future investigator concern;
        // not all callees use RBP, so this requires PDB-driven unwind data.
        Err(BackendError::NotSupported)
    }

    fn last_write_register(
        &self,
        thread: ThreadId,
        reg: RegId,
        before: Position,
    ) -> BackendResult<Option<WriteRecord>> {
        let t = self.thread(thread)?;
        let start = t.active_time.min;
        let end_excl = to_sys_position(before);
        // before must be strictly within the thread's active range.
        if !position_in_range(before, t.active_time) || tuple(end_excl) <= tuple(start) {
            return Ok(None);
        }
        let cur = Cursor::new(self.engine)?;
        unsafe { sys::dhttd_cursor_set_position_on_thread(cur.handle, t.unique_id, start) };

        let mut prev_regs = sys::DhttdAmd64Registers::default();
        let mut have_prev = false;
        let mut last_write: Option<WriteRecord> = None;
        // Hard safety cap so a pathological trace cannot wedge a query.
        // 50M steps comfortably exceeds any of our crash specimens.
        const MAX_STEPS: u64 = 50_000_000;
        let mut steps_taken: u64 = 0;

        loop {
            let mut cur_pos = sys::DhttdPosition::default();
            if unsafe { sys::dhttd_cursor_get_position(cur.handle, &mut cur_pos) } == 0 {
                break;
            }
            // Stop strictly before `end_excl`.
            if tuple(cur_pos) >= tuple(end_excl) {
                break;
            }

            let mut regs = sys::DhttdAmd64Registers::default();
            let regs_ok = unsafe { sys::dhttd_cursor_amd64_registers(cur.handle, &mut regs) } != 0;
            if regs_ok {
                if have_prev {
                    let prev_v = read_reg(&prev_regs, reg);
                    let new_v = read_reg(&regs, reg);
                    if prev_v != new_v {
                        last_write = Some(WriteRecord {
                            position: from_sys_position(cur_pos),
                            thread,
                            ip: regs.rip,
                            value: new_v,
                        });
                    }
                }
                prev_regs = regs;
                have_prev = true;
            }

            // Advance one step.
            let mut next = sys::DhttdPosition::default();
            let mut stop_reason: u32 = 0;
            let advanced = unsafe {
                sys::dhttd_cursor_replay_forward(
                    cur.handle,
                    end_excl,
                    1,
                    &mut next,
                    &mut stop_reason,
                )
            };
            if advanced == 0 {
                break;
            }
            if tuple(next) <= tuple(cur_pos) {
                // Cursor didn't advance past the position we just sampled;
                // protect against infinite loop. Using `cur_pos` (rather
                // than `prev_pos`, which only updates on successful register
                // reads) ensures we always make progress even when several
                // consecutive steps fail to read registers.
                break;
            }
            steps_taken += 1;
            if steps_taken > MAX_STEPS {
                return Err(BackendError::Internal(format!(
                    "last_write_register exceeded {MAX_STEPS} steps without reaching target"
                )));
            }
        }
        Ok(last_write)
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// RAII wrapper for an SDK cursor.
struct Cursor {
    handle: sys::DhttdCursorHandle,
}

impl Cursor {
    fn new(engine: sys::DhttdEngineHandle) -> BackendResult<Self> {
        let mut handle: sys::DhttdCursorHandle = 0;
        let rc = unsafe { sys::dhttd_cursor_create(engine, &mut handle) };
        if rc != 0 || handle == 0 {
            return Err(BackendError::Internal(format!(
                "NewCursor failed: 0x{rc:08x}"
            )));
        }
        Ok(Self { handle })
    }
}

impl Drop for Cursor {
    fn drop(&mut self) {
        if self.handle != 0 {
            unsafe { sys::dhttd_cursor_destroy(self.handle) };
        }
    }
}

fn read_modules(engine: sys::DhttdEngineHandle) -> Vec<ModuleInfo> {
    let count = unsafe { sys::dhttd_engine_module_instance_count(engine) };
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let mut m = sys::DhttdModule::default();
        let mut name_buf = vec![0u16; 512];
        let mut name_len: usize = 0;
        let ok = unsafe {
            sys::dhttd_engine_module_instance(
                engine,
                i,
                &mut m,
                name_buf.as_mut_ptr(),
                name_buf.len(),
                &mut name_len,
            )
        };
        if ok == 0 {
            continue;
        }
        // name_len is the *required* length, not bytes copied; clamp before slicing.
        let copied = name_len.min(name_buf.len().saturating_sub(1));
        let name = String::from_utf16_lossy(&name_buf[..copied]);
        out.push(ModuleInfo {
            name,
            base: m.address,
            size: m.size,
        });
    }
    out
}

fn read_threads(engine: sys::DhttdEngineHandle) -> Vec<sys::DhttdThread> {
    let count = unsafe { sys::dhttd_engine_thread_count(engine) };
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let mut t = sys::DhttdThread::default();
        if unsafe { sys::dhttd_engine_thread(engine, i, &mut t) } != 0 {
            out.push(t);
        }
    }
    out
}

fn read_first_exception(engine: sys::DhttdEngineHandle) -> Option<TerminationEvent> {
    let count = unsafe { sys::dhttd_engine_exception_count(engine) };
    if count == 0 {
        return None;
    }
    let mut ex = sys::DhttdException::default();
    let ok = unsafe { sys::dhttd_engine_exception(engine, 0, &mut ex) };
    if ok == 0 {
        return None;
    }
    Some(classify_exception(&ex))
}

fn classify_exception(ex: &sys::DhttdException) -> TerminationEvent {
    let params = &ex.parameters[..(ex.parameter_count as usize).min(ex.parameters.len())];
    let kind = if ex.code == 0xC000_0005 && params.len() >= 2 {
        let access = match params[0] {
            0 => MemoryAccessKind::Read,
            1 => MemoryAccessKind::Write,
            8 => MemoryAccessKind::Execute,
            _ => MemoryAccessKind::Read,
        };
        TerminationKind::AccessViolation {
            access,
            address: params[1],
        }
    } else if is_fast_fail_status(ex.code) {
        TerminationKind::FastFail {
            code: ex.code,
            noncontinuable: (ex.flags & 1) != 0,
        }
    } else {
        TerminationKind::OtherException {
            code: ex.code,
            address: ex.program_counter,
        }
    };
    TerminationEvent {
        thread: ThreadId(ex.thread_unique_id),
        position: from_sys_position(ex.position),
        kind,
    }
}

/// Mirrors the fail-fast set classified by `morgagni-diagnostics`' JSON
/// loader. Kept in sync manually; both lists are tiny.
fn is_fast_fail_status(code: u32) -> bool {
    matches!(code, 0xC000_0374 | 0xC000_0409 | 0xC000_041D | 0xC000_0602)
}

fn convert_registers(r: &sys::DhttdAmd64Registers) -> Registers {
    Registers {
        rax: r.rax,
        rbx: r.rbx,
        rcx: r.rcx,
        rdx: r.rdx,
        rsi: r.rsi,
        rdi: r.rdi,
        rbp: r.rbp,
        rsp: r.rsp,
        r8: r.r8,
        r9: r.r9,
        r10: r.r10,
        r11: r.r11,
        r12: r.r12,
        r13: r.r13,
        r14: r.r14,
        r15: r.r15,
        rip: r.rip,
        rflags: r.eflags as u64,
    }
}

fn read_reg(r: &sys::DhttdAmd64Registers, reg: RegId) -> u64 {
    match reg {
        RegId::Rax => r.rax,
        RegId::Rbx => r.rbx,
        RegId::Rcx => r.rcx,
        RegId::Rdx => r.rdx,
        RegId::Rsi => r.rsi,
        RegId::Rdi => r.rdi,
        RegId::Rbp => r.rbp,
        RegId::Rsp => r.rsp,
        RegId::R8 => r.r8,
        RegId::R9 => r.r9,
        RegId::R10 => r.r10,
        RegId::R11 => r.r11,
        RegId::R12 => r.r12,
        RegId::R13 => r.r13,
        RegId::R14 => r.r14,
        RegId::R15 => r.r15,
        RegId::Rip => r.rip,
        RegId::Rflags => r.eflags as u64,
    }
}

fn to_sys_position(p: Position) -> sys::DhttdPosition {
    sys::DhttdPosition {
        sequence: p.sequence,
        steps: p.steps,
    }
}

fn from_sys_position(p: sys::DhttdPosition) -> Position {
    Position {
        sequence: p.sequence,
        steps: p.steps,
    }
}

fn tuple(p: sys::DhttdPosition) -> (u64, u64) {
    (p.sequence, p.steps)
}

fn position_in_range(p: Position, r: sys::DhttdPositionRange) -> bool {
    let pt = (p.sequence, p.steps);
    pt >= tuple(r.min) && pt <= tuple(r.max)
}

fn to_utf16_nul(p: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    let mut v: Vec<u16> = p.as_os_str().encode_wide().collect();
    v.push(0);
    v
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// All RunTrace tests require a real `.run` file to be present; in CI
    /// without TTD captures we skip rather than fail. Set `MORGAGNI_RUN`
    /// to a path to exercise the full pipeline locally.
    fn open_fixture() -> Option<RunTrace> {
        // Prefer an env override (lets a dev point at any local capture).
        let path = std::env::var_os("MORGAGNI_RUN")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                // Anchor the default fixture to the repo, not CWD: cargo
                // runs tests with CWD = crate dir in some configurations
                // and = workspace root in others, so a relative path is
                // unreliable. CARGO_MANIFEST_DIR is this crate's directory.
                let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../fixtures/null-deref-x64.run");
                if p.exists() { Some(p) } else { None }
            })?;
        match RunTrace::from_path(&path) {
            Ok(t) => Some(t),
            Err(e) => {
                eprintln!(
                    "skipping RunTrace test: failed to open {}: {e}",
                    path.display()
                );
                None
            }
        }
    }

    #[test]
    fn opens_and_lists_modules() {
        let Some(t) = open_fixture() else { return };
        let ms = t.modules().unwrap();
        assert!(
            !ms.is_empty(),
            "trace should expose at least the main module"
        );
    }

    #[test]
    fn termination_present_for_crashing_trace() {
        let Some(t) = open_fixture() else { return };
        let term = t
            .termination()
            .unwrap()
            .expect("crash specimen must terminate");
        // We don't assert the exact kind here because the env override might
        // point at any trace; just sanity-check the event shape.
        assert!(term.position.sequence > 0 || term.position.steps > 0);
    }

    #[test]
    fn registers_at_termination_reasonable() {
        let Some(t) = open_fixture() else { return };
        let term = t.termination().unwrap().expect("term");
        let r = t.registers(term.thread, term.position).expect("regs");
        assert_ne!(r.rip, 0, "RIP at fault should not be zero");
    }

    #[test]
    fn read_memory_at_rip_returns_some_bytes() {
        let Some(t) = open_fixture() else { return };
        let term = t.termination().unwrap().expect("term");
        let r = t.registers(term.thread, term.position).expect("regs");
        let bytes = t.read_memory(term.position, r.rip, 16).expect("read");
        assert_eq!(bytes.len(), 16);
    }

    /// End-to-end: run the null-deref investigator against a real `.run`
    /// trace. Only meaningful when the default fixture
    /// `fixtures/null-deref-x64.run` is the one being opened (the env
    /// override may point at any trace), so we just check the report
    /// shape and print it for inspection with `cargo test -- --nocapture`.
    #[test]
    fn investigator_runs_against_run_trace() {
        let Some(t) = open_fixture() else { return };
        let report =
            morgagni_diagnostics::investigate(&t).expect("investigator should produce a report");
        eprintln!("---- investigation report ----");
        eprintln!("summary : {}", report.summary);
        eprintln!("root    : {:?}", report.root_cause);
        eprintln!(
            "fault   : ip={:#x} addr={:#x} access={:?} text={:?}",
            report.fault.faulting_ip,
            report.fault.faulting_address,
            report.fault.access,
            report.fault.instruction_text,
        );
        assert!(!report.summary.is_empty());
        assert_ne!(report.fault.faulting_ip, 0);
    }
}

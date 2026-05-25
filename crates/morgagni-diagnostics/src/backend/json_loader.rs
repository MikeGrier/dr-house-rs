//! [`TraceBackend`](super::TraceBackend) backed by a JSON dump produced by the
//! `ttd-dump` example in `morgagni-ttd-decoder-sys`.
//!
//! The JSON is a serialised "intermediate fact file" — a frozen snapshot of
//! everything the dumper extracted from a `.run` trace. It is not a substitute
//! for talking to the SDK directly; it is a fast, deterministic, version-able
//! input for investigator development and unit testing.
//!
//! # Schema overview
//!
//! - Top-level: `modules`, `threads`, `exceptions`, `keyframes`,
//!   `recorded_regions[]`, plus metadata.
//! - Each `recorded_region` covers one thread for a contiguous position range
//!   and contains a `steps[]` array in ascending position order. Each step is
//!   either a `keyframe` (full register snapshot) or a `delta` (only changed
//!   registers). To reconstruct full state at any step we binary-search to it,
//!   walk back to the nearest preceding keyframe, then replay deltas forward.
//! - The default index-only dump has empty `recorded_regions[]`; only `--whole`
//!   captures populate per-step data. Queries that require per-step data on an
//!   index-only dump return [`BackendError::OutOfRange`].

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use serde::de::{self, Deserializer, Visitor};
use serde::Deserialize;

use super::*;

// --- wire schema (private) --------------------------------------------------

#[derive(Deserialize)]
#[allow(dead_code)] // wire schema fields parsed for validation only
struct WirePositionRange {
    min: WirePosition,
    max: WirePosition,
}

/// `"sequence:steps"`, both as base-10.
#[derive(Clone, Copy)]
struct WirePosition(Position);

impl<'de> Deserialize<'de> for WirePosition {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = WirePosition;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(r#"position string of the form "sequence:steps""#)
            }
            fn visit_str<E: de::Error>(self, s: &str) -> Result<Self::Value, E> {
                let (a, b) = s
                    .split_once(':')
                    .ok_or_else(|| E::custom(format!("missing ':' in position {s:?}")))?;
                let sequence = a
                    .parse::<u64>()
                    .map_err(|e| E::custom(format!("bad sequence in {s:?}: {e}")))?;
                let steps = b
                    .parse::<u64>()
                    .map_err(|e| E::custom(format!("bad steps in {s:?}: {e}")))?;
                Ok(WirePosition(Position { sequence, steps }))
            }
        }
        d.deserialize_str(V)
    }
}

/// Hex u64. Accepts an optional `0x` prefix; lowercase digits.
#[derive(Clone, Copy, Default)]
struct Hex(u64);

impl<'de> Deserialize<'de> for Hex {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Hex;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("hex u64 string (with or without 0x prefix)")
            }
            fn visit_str<E: de::Error>(self, s: &str) -> Result<Self::Value, E> {
                let stripped = s
                    .strip_prefix("0x")
                    .or_else(|| s.strip_prefix("0X"))
                    .unwrap_or(s);
                u64::from_str_radix(stripped, 16)
                    .map(Hex)
                    .map_err(|e| E::custom(format!("bad hex {s:?}: {e}")))
            }
        }
        d.deserialize_str(V)
    }
}

#[derive(Deserialize)]
struct WireModule {
    name: String,
    address_hex: Hex,
    size: u64,
    // checksum, timestamp, load_sequence, unload_sequence ignored for now.
}

#[derive(Deserialize)]
struct WireThread {
    unique_id: u32,
    #[allow(dead_code)] // parsed from wire format but not yet surfaced
    os_thread_id: u32,
    // lifetime/active_time ignored for now.
}

#[derive(Deserialize)]
struct WireException {
    position: WirePosition,
    thread_unique_id: u32,
    code_hex: Hex,
    program_counter_hex: Hex,
    parameters_hex: Vec<Hex>,
    #[serde(default)]
    flags: u32,
    // context, record_address_hex, type ignored for backend mapping.
}

#[derive(Deserialize)]
struct WireRegs {
    rax: Hex,
    rcx: Hex,
    rdx: Hex,
    rbx: Hex,
    rsp: Hex,
    rbp: Hex,
    rsi: Hex,
    rdi: Hex,
    r8: Hex,
    r9: Hex,
    r10: Hex,
    r11: Hex,
    r12: Hex,
    r13: Hex,
    r14: Hex,
    r15: Hex,
    rip: Hex,
    eflags: Hex,
}

#[derive(Deserialize, Default)]
struct WireDelta {
    #[serde(default)]
    rax: Option<Hex>,
    #[serde(default)]
    rcx: Option<Hex>,
    #[serde(default)]
    rdx: Option<Hex>,
    #[serde(default)]
    rbx: Option<Hex>,
    #[serde(default)]
    rsp: Option<Hex>,
    #[serde(default)]
    rbp: Option<Hex>,
    #[serde(default)]
    rsi: Option<Hex>,
    #[serde(default)]
    rdi: Option<Hex>,
    #[serde(default)]
    r8: Option<Hex>,
    #[serde(default)]
    r9: Option<Hex>,
    #[serde(default)]
    r10: Option<Hex>,
    #[serde(default)]
    r11: Option<Hex>,
    #[serde(default)]
    r12: Option<Hex>,
    #[serde(default)]
    r13: Option<Hex>,
    #[serde(default)]
    r14: Option<Hex>,
    #[serde(default)]
    r15: Option<Hex>,
    #[serde(default)]
    rip: Option<Hex>,
    #[serde(default)]
    eflags: Option<Hex>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum WirePayload {
    Keyframe(WireRegs),
    Delta(WireDelta),
}

#[derive(Deserialize)]
struct WireStep {
    pos: WirePosition,
    #[serde(flatten)]
    payload: WirePayload,
}

#[derive(Deserialize)]
struct WireRegion {
    thread_unique_id: u32,
    #[allow(dead_code)]
    position_range: WirePositionRange,
    #[allow(dead_code)]
    truncated: bool,
    #[allow(dead_code)]
    keyframe_interval: u32,
    steps: Vec<WireStep>,
}

#[derive(Deserialize)]
struct WireDump {
    #[allow(dead_code)]
    trace_path: String,
    modules: Vec<WireModule>,
    threads: Vec<WireThread>,
    exceptions: Vec<WireException>,
    #[serde(default)]
    recorded_regions: Vec<WireRegion>,
}

// --- internal in-memory representation --------------------------------------

/// One region preprocessed for fast lookup.
struct Region {
    thread: ThreadId,
    /// Step positions in ascending order. `steps[i]` corresponds to
    /// `keyframe_indices[..]` and `payloads[i]`.
    positions: Vec<Position>,
    payloads: Vec<Payload>,
    /// Sorted indices into `positions` at which a keyframe (full snapshot) lives.
    keyframe_indices: Vec<usize>,
}

enum Payload {
    Keyframe(Registers),
    Delta(DeltaRegs),
}

#[derive(Default, Clone)]
struct DeltaRegs {
    rax: Option<u64>,
    rcx: Option<u64>,
    rdx: Option<u64>,
    rbx: Option<u64>,
    rsp: Option<u64>,
    rbp: Option<u64>,
    rsi: Option<u64>,
    rdi: Option<u64>,
    r8: Option<u64>,
    r9: Option<u64>,
    r10: Option<u64>,
    r11: Option<u64>,
    r12: Option<u64>,
    r13: Option<u64>,
    r14: Option<u64>,
    r15: Option<u64>,
    rip: Option<u64>,
    rflags: Option<u64>,
}

/// Loader that owns a parsed JSON dump and answers backend queries against it.
pub struct JsonTrace {
    modules: Vec<ModuleInfo>,
    threads: Vec<ThreadId>,
    termination: Option<TerminationEvent>,
    regions: Vec<Region>,
}

impl JsonTrace {
    /// Parse a JSON dump from `bytes`.
    pub fn from_bytes(bytes: &[u8]) -> BackendResult<Self> {
        let wire: WireDump = serde_json::from_slice(bytes)
            .map_err(|e| BackendError::Internal(format!("json parse: {e}")))?;
        Self::from_wire(wire)
    }

    /// Parse a JSON dump from `path`.
    pub fn from_path(path: impl AsRef<Path>) -> BackendResult<Self> {
        let bytes = std::fs::read(path.as_ref()).map_err(|e| {
            BackendError::Internal(format!("read {}: {e}", path.as_ref().display()))
        })?;
        Self::from_bytes(&bytes)
    }

    fn from_wire(w: WireDump) -> BackendResult<Self> {
        let modules: Vec<ModuleInfo> = w
            .modules
            .into_iter()
            .map(|m| ModuleInfo {
                name: m.name,
                base: m.address_hex.0,
                size: m.size,
            })
            .collect();

        let threads: Vec<ThreadId> = w.threads.iter().map(|t| ThreadId(t.unique_id)).collect();

        // Take the first exception (if any) as the termination event. Investigators
        // that care about secondary exceptions can read them via a future API; for
        // now the convention matches the mock backend's single-termination shape.
        let termination = w.exceptions.into_iter().next().map(map_termination);

        let mut regions = Vec::with_capacity(w.recorded_regions.len());
        for wr in w.recorded_regions {
            let mut positions = Vec::with_capacity(wr.steps.len());
            let mut payloads = Vec::with_capacity(wr.steps.len());
            let mut keyframe_indices = Vec::new();
            for (i, step) in wr.steps.into_iter().enumerate() {
                positions.push(step.pos.0);
                match step.payload {
                    WirePayload::Keyframe(k) => {
                        keyframe_indices.push(i);
                        payloads.push(Payload::Keyframe(regs_from_wire(k)));
                    }
                    WirePayload::Delta(d) => payloads.push(Payload::Delta(delta_from_wire(d))),
                }
            }
            // The dumper always emits a keyframe at index 0; reject malformed input
            // that would otherwise prevent any reconstruction.
            if !positions.is_empty() && keyframe_indices.first().copied() != Some(0) {
                return Err(BackendError::Internal(
                    "recorded region must begin with a keyframe step".into(),
                ));
            }
            // Positions must be strictly ascending (we rely on this for binary search).
            if positions.windows(2).any(|w| w[0] >= w[1]) {
                return Err(BackendError::Internal(
                    "recorded region steps must be strictly ascending in position".into(),
                ));
            }
            regions.push(Region {
                thread: ThreadId(wr.thread_unique_id),
                positions,
                payloads,
                keyframe_indices,
            });
        }

        Ok(Self {
            modules,
            threads,
            termination,
            regions,
        })
    }

    /// All recorded thread ids (whether or not they have whole-trace step data).
    pub fn threads_recorded(&self) -> &[ThreadId] {
        &self.threads
    }

    fn region_for(&self, thread: ThreadId, position: Position) -> BackendResult<&Region> {
        if !self.threads.contains(&thread) {
            return Err(BackendError::UnknownThread);
        }
        self.regions
            .iter()
            .find(|r| {
                r.thread == thread
                    && !r.positions.is_empty()
                    && position >= r.positions[0]
                    && position <= *r.positions.last().unwrap()
            })
            .ok_or(BackendError::OutOfRange)
    }

    /// Reconstruct full register state at `position` by replaying from the
    /// nearest preceding keyframe inside the region.
    fn reconstruct(region: &Region, position: Position) -> Registers {
        // Largest step index whose position is <= the target.
        let step_idx = match region.positions.binary_search(&position) {
            Ok(i) => i,
            Err(i) => i - 1, // i > 0 because caller already checked position >= positions[0].
        };
        // Largest keyframe index <= step_idx.
        let kf_idx = match region.keyframe_indices.binary_search(&step_idx) {
            Ok(i) => i,
            Err(i) => i - 1, // ditto: index 0 is always a keyframe.
        };
        let kf_step = region.keyframe_indices[kf_idx];
        let Payload::Keyframe(start) = &region.payloads[kf_step] else {
            unreachable!("keyframe_indices points at a Keyframe payload");
        };
        let mut state = start.clone();
        for p in &region.payloads[kf_step + 1..=step_idx] {
            if let Payload::Delta(d) = p {
                apply_delta(&mut state, d);
            } else if let Payload::Keyframe(k) = p {
                state = k.clone();
            }
        }
        state
    }
}

impl TraceBackend for JsonTrace {
    fn modules(&self) -> BackendResult<Vec<ModuleInfo>> {
        Ok(self.modules.clone())
    }

    fn termination(&self) -> BackendResult<Option<TerminationEvent>> {
        Ok(self.termination.clone())
    }

    fn registers(&self, thread: ThreadId, position: Position) -> BackendResult<Registers> {
        let region = self.region_for(thread, position)?;
        Ok(Self::reconstruct(region, position))
    }

    fn read_memory(
        &self,
        _position: Position,
        _address: u64,
        _len: usize,
    ) -> BackendResult<Vec<u8>> {
        // The dumper does not yet emit `memory_reads` outside of explicit
        // capture sites; until it does, this backend has no bytes to serve.
        Err(BackendError::NotSupported)
    }

    fn stack(&self, _thread: ThreadId, _position: Position) -> BackendResult<Vec<RawFrame>> {
        Err(BackendError::NotSupported)
    }

    fn last_write_register(
        &self,
        thread: ThreadId,
        reg: RegId,
        before: Position,
    ) -> BackendResult<Option<WriteRecord>> {
        if !self.threads.contains(&thread) {
            return Err(BackendError::UnknownThread);
        }
        // Linear scan: every region for this thread, every step before `before`,
        // remember the latest one whose payload changes `reg`. Investigators
        // that need many such queries on a hot path can preindex; not worth it
        // until we have a profile saying so.
        let mut best: Option<WriteRecord> = None;
        for region in self.regions.iter().filter(|r| r.thread == thread) {
            let mut state = Registers::default();
            let mut prev_value: u64 = 0;
            let mut have_prev = false;
            // RIP of the step that produced the *next* state is the address of
            // the instruction that performed the write. Carry it across the loop.
            let mut prev_rip: u64 = 0;
            for (i, payload) in region.payloads.iter().enumerate() {
                let pos = region.positions[i];
                if pos >= before {
                    break;
                }
                let before_value = if have_prev { Some(prev_value) } else { None };
                match payload {
                    Payload::Keyframe(k) => state = k.clone(),
                    Payload::Delta(d) => apply_delta(&mut state, d),
                }
                let new_value = state.get(reg);
                if let Some(bv) = before_value {
                    if bv != new_value {
                        best = Some(WriteRecord {
                            position: pos,
                            thread,
                            ip: prev_rip,
                            value: new_value,
                        });
                    }
                }
                prev_value = new_value;
                have_prev = true;
                prev_rip = state.rip;
            }
        }
        Ok(best)
    }
}

// --- helpers ----------------------------------------------------------------

fn regs_from_wire(w: WireRegs) -> Registers {
    Registers {
        rax: w.rax.0,
        rbx: w.rbx.0,
        rcx: w.rcx.0,
        rdx: w.rdx.0,
        rsi: w.rsi.0,
        rdi: w.rdi.0,
        rbp: w.rbp.0,
        rsp: w.rsp.0,
        r8: w.r8.0,
        r9: w.r9.0,
        r10: w.r10.0,
        r11: w.r11.0,
        r12: w.r12.0,
        r13: w.r13.0,
        r14: w.r14.0,
        r15: w.r15.0,
        rip: w.rip.0,
        rflags: w.eflags.0,
    }
}

fn delta_from_wire(w: WireDelta) -> DeltaRegs {
    DeltaRegs {
        rax: w.rax.map(|h| h.0),
        rcx: w.rcx.map(|h| h.0),
        rdx: w.rdx.map(|h| h.0),
        rbx: w.rbx.map(|h| h.0),
        rsp: w.rsp.map(|h| h.0),
        rbp: w.rbp.map(|h| h.0),
        rsi: w.rsi.map(|h| h.0),
        rdi: w.rdi.map(|h| h.0),
        r8: w.r8.map(|h| h.0),
        r9: w.r9.map(|h| h.0),
        r10: w.r10.map(|h| h.0),
        r11: w.r11.map(|h| h.0),
        r12: w.r12.map(|h| h.0),
        r13: w.r13.map(|h| h.0),
        r14: w.r14.map(|h| h.0),
        r15: w.r15.map(|h| h.0),
        rip: w.rip.map(|h| h.0),
        rflags: w.eflags.map(|h| h.0),
    }
}

fn apply_delta(state: &mut Registers, d: &DeltaRegs) {
    if let Some(v) = d.rax {
        state.rax = v;
    }
    if let Some(v) = d.rbx {
        state.rbx = v;
    }
    if let Some(v) = d.rcx {
        state.rcx = v;
    }
    if let Some(v) = d.rdx {
        state.rdx = v;
    }
    if let Some(v) = d.rsi {
        state.rsi = v;
    }
    if let Some(v) = d.rdi {
        state.rdi = v;
    }
    if let Some(v) = d.rbp {
        state.rbp = v;
    }
    if let Some(v) = d.rsp {
        state.rsp = v;
    }
    if let Some(v) = d.r8 {
        state.r8 = v;
    }
    if let Some(v) = d.r9 {
        state.r9 = v;
    }
    if let Some(v) = d.r10 {
        state.r10 = v;
    }
    if let Some(v) = d.r11 {
        state.r11 = v;
    }
    if let Some(v) = d.r12 {
        state.r12 = v;
    }
    if let Some(v) = d.r13 {
        state.r13 = v;
    }
    if let Some(v) = d.r14 {
        state.r14 = v;
    }
    if let Some(v) = d.r15 {
        state.r15 = v;
    }
    if let Some(v) = d.rip {
        state.rip = v;
    }
    if let Some(v) = d.rflags {
        state.rflags = v;
    }
}

fn map_termination(e: WireException) -> TerminationEvent {
    let code = e.code_hex.0 as u32;
    // Windows AVs (0xC0000005) put [op, address] in ExceptionInformation[].
    // op: 0=read, 1=write, 8=execute.
    let kind = if code == 0xC000_0005 && e.parameters_hex.len() >= 2 {
        let op = e.parameters_hex[0].0;
        let address = e.parameters_hex[1].0;
        let access = match op {
            0 => MemoryAccessKind::Read,
            1 => MemoryAccessKind::Write,
            8 => MemoryAccessKind::Execute,
            // Unknown op code: classify as a read; investigators that care can
            // re-inspect the raw exception later.
            _ => MemoryAccessKind::Read,
        };
        TerminationKind::AccessViolation { access, address }
    } else if is_fast_fail_status(code) {
        TerminationKind::FastFail {
            code,
            noncontinuable: (e.flags & 1) != 0,
        }
    } else {
        TerminationKind::OtherException {
            code,
            address: e.program_counter_hex.0,
        }
    };
    TerminationEvent {
        thread: ThreadId(e.thread_unique_id),
        position: e.position.0,
        kind,
    }
}

/// NTSTATUS codes raised via `__fastfail` / `RtlFailFast2`. These are
/// process-terminating by kernel policy regardless of any handler.
fn is_fast_fail_status(code: u32) -> bool {
    matches!(
        code,
        0xC000_0374 // STATUS_HEAP_CORRUPTION
        | 0xC000_0409 // STATUS_STACK_BUFFER_OVERRUN (also covers /GS, __fastfail)
        | 0xC000_041D // STATUS_FATAL_USER_CALLBACK_EXCEPTION
        | 0xC000_0602 // STATUS_FAIL_FAST_EXCEPTION
    )
}

// Suppress dead-code warnings on wire-only fields we keep for completeness.
#[allow(dead_code)]
const _: () = {
    let _ = std::mem::size_of::<BTreeMap<Position, Registers>>();
};

// ----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(s: u64, t: u64) -> Position {
        Position {
            sequence: s,
            steps: t,
        }
    }

    /// Minimal whole-trace dump: one region, one thread, three steps
    /// (keyframe / delta / delta), plus an AV exception.
    const SYNTHETIC: &str = r#"
    {
      "trace_path": "synthetic",
      "system_info": {},
      "lifetime": {"min": "0:0", "max": "100:0"},
      "modules": [
        {"name": "demo.exe", "address_hex": "0x7ff600000000", "size": 4096,
         "checksum": 0, "timestamp": 0, "load_sequence": 0, "unload_sequence": 0}
      ],
      "threads": [
        {"unique_id": 2, "os_thread_id": 1234,
         "lifetime":   {"min": "10:0", "max": "60:0"},
         "active_time":{"min": "10:0", "max": "60:0"}}
      ],
      "exceptions": [
        {"position": "55:7", "thread_unique_id": 2, "type": 0,
         "code_hex": "0xc0000005", "flags": 0,
         "record_address_hex": "0x0",
         "program_counter_hex": "0x7ff600001000",
         "parameters_hex": ["0x1", "0xdeadbeef"]}
      ],
      "keyframes": ["10:0", "60:0"],
      "recorded_regions": [
        {
          "anchor": {"kind": "thread_whole", "thread_unique_id": 2},
          "thread_unique_id": 2,
          "position_range": {"min": "10:0", "max": "55:7"},
          "step_count": 3,
          "truncated": false,
          "keyframe_interval": 64,
          "steps": [
            {"pos": "10:0", "keyframe": {
              "rax":"0","rcx":"0","rdx":"0","rbx":"0",
              "rsp":"1000","rbp":"0","rsi":"0","rdi":"0",
              "r8":"0","r9":"0","r10":"0","r11":"0",
              "r12":"0","r13":"0","r14":"0","r15":"0",
              "rip":"7ff600000100","eflags":"202"
            }},
            {"pos": "20:0", "delta": {"rax":"42","rip":"7ff600000110"}},
            {"pos": "55:7", "delta": {"rcx":"deadbeef","rip":"7ff600001000"}}
          ]
        }
      ]
    }
    "#;

    fn loaded() -> JsonTrace {
        JsonTrace::from_bytes(SYNTHETIC.as_bytes()).expect("synthetic parse")
    }

    #[test]
    fn modules_round_trip() {
        let t = loaded();
        let ms = t.modules().unwrap();
        assert_eq!(ms.len(), 1);
        assert_eq!(ms[0].name, "demo.exe");
        assert_eq!(ms[0].base, 0x7ff6_0000_0000);
        assert_eq!(ms[0].size, 4096);
    }

    #[test]
    fn termination_decodes_access_violation_write() {
        let t = loaded();
        let term = t.termination().unwrap().unwrap();
        assert_eq!(term.thread, ThreadId(2));
        assert_eq!(term.position, pos(55, 7));
        match term.kind {
            TerminationKind::AccessViolation { access, address } => {
                assert_eq!(access, MemoryAccessKind::Write);
                assert_eq!(address, 0xdead_beef);
            }
            other => panic!("expected AV, got {other:?}"),
        }
    }

    #[test]
    fn termination_decodes_fast_fail_heap_corruption() {
        // STATUS_HEAP_CORRUPTION (0xC0000374) with EXCEPTION_NONCONTINUABLE.
        // The PC lives in ntdll's report routine; params[0] is ntdll-internal.
        let json = SYNTHETIC.replace(
            r#""code_hex": "0xc0000005", "flags": 0"#,
            r#""code_hex": "0xc0000374", "flags": 1"#,
        );
        let t = JsonTrace::from_bytes(json.as_bytes()).expect("parse");
        let term = t.termination().unwrap().unwrap();
        match term.kind {
            TerminationKind::FastFail {
                code,
                noncontinuable,
            } => {
                assert_eq!(code, 0xC000_0374);
                assert!(noncontinuable);
            }
            other => panic!("expected FastFail, got {other:?}"),
        }
    }

    #[test]
    fn termination_decodes_other_exception() {
        // An NTSTATUS that is neither AV nor in the fail-fast set should
        // remain OtherException.
        let json = SYNTHETIC.replace(
            r#""code_hex": "0xc0000005", "flags": 0"#,
            r#""code_hex": "0xc0000094", "flags": 0"#, // STATUS_INTEGER_DIVIDE_BY_ZERO
        );
        let t = JsonTrace::from_bytes(json.as_bytes()).expect("parse");
        let term = t.termination().unwrap().unwrap();
        match term.kind {
            TerminationKind::OtherException { code, .. } => {
                assert_eq!(code, 0xC000_0094);
            }
            other => panic!("expected OtherException, got {other:?}"),
        }
    }

    #[test]
    fn registers_at_keyframe() {
        let r = loaded().registers(ThreadId(2), pos(10, 0)).unwrap();
        assert_eq!(r.rax, 0);
        assert_eq!(r.rip, 0x7ff6_0000_0100);
        assert_eq!(r.rflags, 0x202);
    }

    #[test]
    fn registers_replay_one_delta() {
        let r = loaded().registers(ThreadId(2), pos(20, 0)).unwrap();
        assert_eq!(r.rax, 0x42); // "42" in the JSON is hex
        assert_eq!(r.rip, 0x7ff6_0000_0110);
        // unchanged fields carry forward from the keyframe
        assert_eq!(r.rsp, 0x1000);
    }

    #[test]
    fn registers_replay_to_fault() {
        let r = loaded().registers(ThreadId(2), pos(55, 7)).unwrap();
        assert_eq!(r.rax, 0x42); // still set
        assert_eq!(r.rcx, 0xdead_beef);
        assert_eq!(r.rip, 0x7ff6_0000_1000);
    }

    #[test]
    fn registers_between_steps_uses_floor() {
        // Position 30:0 sits between step 20:0 (delta rax=42) and 55:7
        // (delta rcx=deadbeef). We expect state-at-floor = state-at-20:0.
        let r = loaded().registers(ThreadId(2), pos(30, 0)).unwrap();
        assert_eq!(r.rax, 0x42);
        assert_eq!(r.rcx, 0);
    }

    #[test]
    fn registers_unknown_thread() {
        assert!(matches!(
            loaded().registers(ThreadId(99), pos(20, 0)),
            Err(BackendError::UnknownThread)
        ));
    }

    #[test]
    fn registers_out_of_recorded_region() {
        // Position 100:0 is past the region max of 55:7.
        assert!(matches!(
            loaded().registers(ThreadId(2), pos(100, 0)),
            Err(BackendError::OutOfRange)
        ));
    }

    #[test]
    fn last_write_register_finds_delta() {
        let w = loaded()
            .last_write_register(ThreadId(2), RegId::Rcx, pos(56, 0))
            .unwrap()
            .expect("rcx is written at 55:7");
        assert_eq!(w.position, pos(55, 7));
        assert_eq!(w.value, 0xdead_beef);
    }

    #[test]
    fn last_write_register_strictly_before() {
        // before=55:7 must exclude the write *at* 55:7.
        let w = loaded()
            .last_write_register(ThreadId(2), RegId::Rcx, pos(55, 7))
            .unwrap();
        assert!(w.is_none(), "expected no rcx write before 55:7, got {w:?}");
    }

    #[test]
    fn last_write_register_none_for_unwritten_reg() {
        let w = loaded()
            .last_write_register(ThreadId(2), RegId::R15, pos(99, 0))
            .unwrap();
        assert!(w.is_none());
    }

    #[test]
    fn read_memory_not_supported_yet() {
        assert!(matches!(
            loaded().read_memory(pos(20, 0), 0x1000, 4),
            Err(BackendError::NotSupported)
        ));
    }

    #[test]
    fn rejects_region_without_initial_keyframe() {
        let bad = r#"
        {
          "trace_path":"x","system_info":{},
          "lifetime":{"min":"0:0","max":"1:0"},
          "modules":[],"threads":[{"unique_id":2,"os_thread_id":0,
            "lifetime":{"min":"0:0","max":"1:0"},"active_time":{"min":"0:0","max":"1:0"}}],
          "exceptions":[],"keyframes":[],
          "recorded_regions":[{
            "anchor":{"kind":"thread_whole","thread_unique_id":2},
            "thread_unique_id":2,
            "position_range":{"min":"0:0","max":"1:0"},
            "step_count":1,"truncated":false,"keyframe_interval":64,
            "steps":[{"pos":"0:0","delta":{}}]
          }]
        }
        "#;
        let err = JsonTrace::from_bytes(bad.as_bytes())
            .err()
            .expect("should reject");
        assert!(matches!(err, BackendError::Internal(_)), "got {err:?}");
    }

    #[test]
    fn rejects_non_ascending_positions() {
        let bad = r#"
        {
          "trace_path":"x","system_info":{},
          "lifetime":{"min":"0:0","max":"1:0"},
          "modules":[],"threads":[{"unique_id":2,"os_thread_id":0,
            "lifetime":{"min":"0:0","max":"1:0"},"active_time":{"min":"0:0","max":"1:0"}}],
          "exceptions":[],"keyframes":[],
          "recorded_regions":[{
            "anchor":{"kind":"thread_whole","thread_unique_id":2},
            "thread_unique_id":2,
            "position_range":{"min":"0:0","max":"1:0"},
            "step_count":2,"truncated":false,"keyframe_interval":64,
            "steps":[
              {"pos":"5:0","keyframe":{
                "rax":"0","rcx":"0","rdx":"0","rbx":"0","rsp":"0","rbp":"0",
                "rsi":"0","rdi":"0","r8":"0","r9":"0","r10":"0","r11":"0",
                "r12":"0","r13":"0","r14":"0","r15":"0","rip":"0","eflags":"0"
              }},
              {"pos":"5:0","delta":{}}
            ]
          }]
        }
        "#;
        let err = JsonTrace::from_bytes(bad.as_bytes())
            .err()
            .expect("should reject");
        assert!(matches!(err, BackendError::Internal(_)), "got {err:?}");
    }

    /// Smoke test against the committed real index-only fixture, if present.
    /// Skips silently if the fixture is missing (e.g. running outside the repo).
    #[test]
    fn loads_real_null_deref_index_fixture() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/null-deref-x64.json");
        if !path.exists() {
            return;
        }
        let trace = JsonTrace::from_path(&path).expect("real fixture parses");
        assert!(
            !trace.modules().unwrap().is_empty(),
            "real fixture has modules"
        );
        let term = trace
            .termination()
            .unwrap()
            .expect("real fixture has termination");
        assert_eq!(term.thread, ThreadId(2));
        // Index-only dump: registers() returns OutOfRange for any thread/pos.
        assert!(matches!(
            trace.registers(term.thread, term.position),
            Err(BackendError::OutOfRange)
        ));
    }
}

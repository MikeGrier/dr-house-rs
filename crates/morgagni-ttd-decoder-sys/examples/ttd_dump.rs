//! `ttd-dump` — dumps the contents of a TTD `.run` trace to JSON.
//!
//! Usage:
//!   cargo run -p morgagni-ttd-decoder-sys --example ttd-dump -- <path-to-trace.run> [out.json]
//!
//! Emits a single JSON document on stdout (or into `out.json` if a second
//! argument is provided) covering: system info, lifetime, modules, threads,
//! exceptions, and keyframes. This is intentionally a thin pass-through of
//! the SDK's view-array data so it can serve as ground truth for the safe
//! Rust wrapper and the diagnostics pipeline.

use std::ffi::OsString;
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;

use morgagni_ttd_decoder_sys as sys;
use serde::{Serialize, Serializer};

/// Trace position serialised as `"sequence:steps"` (e.g. `"55:193"`).
///
/// Compact and ordered: lexicographic string comparison happens to agree with
/// numeric `(sequence, steps)` order for any fixed-width prefix, but consumers
/// should parse the two halves and compare numerically.
#[derive(Clone, Copy)]
struct Position {
    sequence: u64,
    steps: u64,
}

impl Serialize for Position {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(&format_args!("{}:{}", self.sequence, self.steps))
    }
}

#[derive(Serialize)]
struct PositionRange {
    min: Position,
    max: Position,
}

#[derive(Serialize)]
struct SystemInfo {
    major_version: u16,
    minor_version: u16,
    process_id: u32,
    peb_address_hex: String,
    os_major: u32,
    os_minor: u32,
    os_build: u32,
    processors: u32,
    arch: &'static str,
}

#[derive(Serialize)]
struct Module {
    name: String,
    address_hex: String,
    size: u64,
    checksum: u32,
    timestamp: u32,
    load_sequence: u64,
    unload_sequence: u64,
}

#[derive(Serialize)]
struct Thread {
    unique_id: u32,
    os_thread_id: u32,
    lifetime: PositionRange,
    active_time: PositionRange,
}

#[derive(Serialize)]
struct Amd64Registers {
    rax: String,
    rcx: String,
    rdx: String,
    rbx: String,
    rsp: String,
    rbp: String,
    rsi: String,
    rdi: String,
    r8: String,
    r9: String,
    r10: String,
    r11: String,
    r12: String,
    r13: String,
    r14: String,
    r15: String,
    rip: String,
    eflags: String,
}

#[derive(Serialize)]
struct FaultContext {
    program_counter_hex: String,
    stack_pointer_hex: String,
    frame_pointer_hex: String,
    registers: Amd64Registers,
    /// Bytes from `record_address` (if non-null), hex-encoded. Empty if the
    /// faulting address is null or unreadable.
    faulting_memory_hex: String,
}

#[derive(Serialize)]
struct Exception {
    position: Position,
    thread_unique_id: u32,
    r#type: u32,
    code_hex: String,
    flags: u32,
    record_address_hex: String,
    program_counter_hex: String,
    parameters_hex: Vec<String>,
    context: Option<FaultContext>,
}

#[derive(Serialize)]
struct Dump {
    trace_path: String,
    system_info: SystemInfo,
    lifetime: PositionRange,
    modules: Vec<Module>,
    threads: Vec<Thread>,
    exceptions: Vec<Exception>,
    keyframes: Vec<Position>,
    /// Optional per-thread / per-event recorded execution regions. Empty in
    /// default "index-only" mode; populated by `--whole` and (later)
    /// `--around` modes. The mock backend serves arbitrary cursor queries by
    /// looking up positions inside these regions; positions outside any
    /// region return `OutOfRecordedRegion` to the caller.
    recorded_regions: Vec<RecordedRegion>,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum RegionAnchor {
    /// Region spans an entire thread's active lifetime.
    ThreadWhole { thread_unique_id: u32 },
    // Future variants: ExceptionWindow { exception_index, steps_before, steps_after },
    //                  ModuleLoad { module_index, ... }, etc.
}

#[derive(Serialize)]
struct RecordedRegion {
    anchor: RegionAnchor,
    thread_unique_id: u32,
    position_range: PositionRange,
    /// Number of steps captured. May be less than the position range implies
    /// if a safety cap was hit during capture.
    step_count: usize,
    /// True if the capture loop hit its safety cap before reaching the end of
    /// the region. The investigator can treat the trailing portion as missing.
    truncated: bool,
    /// Every `keyframe_interval` steps the dumper emits a full register set
    /// ("i-frame"); intervening steps emit only the registers that changed
    /// since the previous step. Consumers reconstructing a step's full state
    /// scan back to the nearest keyframe and replay deltas forward.
    keyframe_interval: u32,
    steps: Vec<RegionStep>,
}

#[derive(Serialize)]
struct RegionStep {
    #[serde(rename = "pos")]
    position: Position,
    #[serde(flatten)]
    payload: StepPayload,
    /// Memory snapshots taken at this step. Omitted from JSON when empty.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    memory_reads: Vec<MemoryRead>,
}

/// Externally-tagged enum: serialises as `{"keyframe": {...}}` or
/// `{"delta": {...}}`. Combined with `#[serde(flatten)]` on `RegionStep`,
/// each step becomes `{ "pos": "55:193", "delta": { "rip": "7ff..." } }`.
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum StepPayload {
    /// Full register snapshot. Anchors a run of subsequent deltas.
    Keyframe(Amd64Registers),
    /// Only the registers that changed since the previous step.
    Delta(RegisterDelta),
}

#[derive(Serialize, Default)]
struct RegisterDelta {
    #[serde(skip_serializing_if = "Option::is_none")]
    rax: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rcx: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rdx: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rbx: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rsp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rbp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rsi: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rdi: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    r8: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    r9: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    r10: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    r11: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    r12: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    r13: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    r14: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    r15: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rip: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    eflags: Option<String>,
}

fn compute_delta(
    prev: &sys::DhttdAmd64Registers,
    curr: &sys::DhttdAmd64Registers,
) -> RegisterDelta {
    let h = |v: u64| format!("{v:x}");
    let d = |p: u64, c: u64| (p != c).then(|| h(c));
    RegisterDelta {
        rax: d(prev.rax, curr.rax),
        rcx: d(prev.rcx, curr.rcx),
        rdx: d(prev.rdx, curr.rdx),
        rbx: d(prev.rbx, curr.rbx),
        rsp: d(prev.rsp, curr.rsp),
        rbp: d(prev.rbp, curr.rbp),
        rsi: d(prev.rsi, curr.rsi),
        rdi: d(prev.rdi, curr.rdi),
        r8: d(prev.r8, curr.r8),
        r9: d(prev.r9, curr.r9),
        r10: d(prev.r10, curr.r10),
        r11: d(prev.r11, curr.r11),
        r12: d(prev.r12, curr.r12),
        r13: d(prev.r13, curr.r13),
        r14: d(prev.r14, curr.r14),
        r15: d(prev.r15, curr.r15),
        rip: d(prev.rip, curr.rip),
        eflags: (prev.eflags != curr.eflags).then(|| format!("{:x}", curr.eflags)),
    }
}

#[derive(Serialize)]
struct MemoryRead {
    address_hex: String,
    bytes_hex: String,
}

fn arch_name(n: u32) -> &'static str {
    match n {
        1 => "x86",
        2 => "x64",
        3 => "arm",
        4 => "arm64",
        _ => "unknown",
    }
}

fn convert_position(p: sys::DhttdPosition) -> Position {
    Position {
        sequence: p.sequence,
        steps: p.steps,
    }
}
fn convert_range(r: sys::DhttdPositionRange) -> PositionRange {
    PositionRange {
        min: convert_position(r.min),
        max: convert_position(r.max),
    }
}

fn convert_registers(r: sys::DhttdAmd64Registers) -> Amd64Registers {
    let h = |v: u64| format!("{v:x}");
    Amd64Registers {
        rax: h(r.rax),
        rcx: h(r.rcx),
        rdx: h(r.rdx),
        rbx: h(r.rbx),
        rsp: h(r.rsp),
        rbp: h(r.rbp),
        rsi: h(r.rsi),
        rdi: h(r.rdi),
        r8: h(r.r8),
        r9: h(r.r9),
        r10: h(r.r10),
        r11: h(r.r11),
        r12: h(r.r12),
        r13: h(r.r13),
        r14: h(r.r14),
        r15: h(r.r15),
        rip: h(r.rip),
        eflags: format!("{:x}", r.eflags),
    }
}

fn to_utf16_with_nul(s: &OsString) -> Vec<u16> {
    let mut v: Vec<u16> = s.encode_wide().collect();
    v.push(0);
    v
}

fn main() -> std::io::Result<()> {
    // Minimal CLI: positional <trace.run> [out.json], plus --whole, --max-steps N,
    // --keyframe-interval N.
    let mut whole = false;
    let mut max_steps: usize = 5_000_000;
    let mut keyframe_interval: u32 = 64;
    let mut positional: Vec<OsString> = Vec::new();
    let mut it = std::env::args_os().skip(1);
    while let Some(a) = it.next() {
        match a.to_str() {
            Some("--whole") => whole = true,
            Some("--max-steps") => {
                let v = it.next().expect("--max-steps requires a value");
                max_steps = v
                    .to_string_lossy()
                    .parse()
                    .expect("--max-steps must be a positive integer");
            }
            Some("--keyframe-interval") => {
                let v = it.next().expect("--keyframe-interval requires a value");
                keyframe_interval = v
                    .to_string_lossy()
                    .parse()
                    .expect("--keyframe-interval must be a positive integer");
                assert!(keyframe_interval >= 1, "--keyframe-interval must be >= 1");
            }
            Some("-h") | Some("--help") => {
                eprintln!(
                    "usage: ttd-dump <trace.run> [out.json] [--whole] [--max-steps N] [--keyframe-interval N]"
                );
                return Ok(());
            }
            _ => positional.push(a),
        }
    }
    let mut pit = positional.into_iter();
    let trace = pit
        .next()
        .expect("usage: ttd-dump <trace.run> [out.json] [--whole]");
    let out_path: Option<PathBuf> = pit.next().map(PathBuf::from);

    let trace_path = PathBuf::from(&trace);
    let trace_utf16 = to_utf16_with_nul(&trace);

    // Engine lifecycle.
    let mut handle: sys::DhttdEngineHandle = 0;
    // SAFETY: out pointer is valid; sys crate makes no Rust assumptions yet.
    let rc = unsafe { sys::dhttd_engine_create(&mut handle) };
    if rc != 0 || handle == 0 {
        eprintln!("CreateReplayEngine failed: 0x{rc:08x}");
        std::process::exit(2);
    }
    // SAFETY: matched destroy in all exit paths.
    let ok = unsafe { sys::dhttd_engine_initialize(handle, trace_utf16.as_ptr()) };
    if ok == 0 {
        eprintln!("IReplayEngine::Initialize({}) failed", trace_path.display());
        unsafe { sys::dhttd_engine_destroy(handle) };
        std::process::exit(3);
    }

    // System info.
    let mut sysi = sys::DhttdSystemInfo::default();
    unsafe {
        sys::dhttd_engine_get_system_info(handle, &mut sysi);
    }
    let system_info = SystemInfo {
        major_version: sysi.major_version,
        minor_version: sysi.minor_version,
        process_id: sysi.process_id,
        peb_address_hex: format!("0x{:016x}", sysi.peb_address),
        os_major: sysi.os_major,
        os_minor: sysi.os_minor,
        os_build: sysi.os_build,
        processors: sysi.processors,
        arch: arch_name(sysi.arch),
    };

    // Lifetime.
    let mut lifetime = sys::DhttdPositionRange::default();
    unsafe {
        sys::dhttd_engine_get_lifetime(handle, &mut lifetime);
    }

    // Modules.
    let module_count = unsafe { sys::dhttd_engine_module_instance_count(handle) };
    let mut modules = Vec::with_capacity(module_count);
    for i in 0..module_count {
        let mut m = sys::DhttdModule::default();
        let mut name_buf = vec![0u16; 512];
        let mut name_len: usize = 0;
        let ok = unsafe {
            sys::dhttd_engine_module_instance(
                handle,
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
        let name = String::from_utf16_lossy(&name_buf[..name_len]);
        modules.push(Module {
            name,
            address_hex: format!("0x{:016x}", m.address),
            size: m.size,
            checksum: m.checksum,
            timestamp: m.timestamp,
            load_sequence: m.load_sequence,
            unload_sequence: m.unload_sequence,
        });
    }

    // Threads.
    let thread_count = unsafe { sys::dhttd_engine_thread_count(handle) };
    let mut threads = Vec::with_capacity(thread_count);
    for i in 0..thread_count {
        let mut t = sys::DhttdThread::default();
        if unsafe { sys::dhttd_engine_thread(handle, i, &mut t) } == 0 {
            continue;
        }
        threads.push(Thread {
            unique_id: t.unique_id,
            os_thread_id: t.os_thread_id,
            lifetime: convert_range(t.lifetime),
            active_time: convert_range(t.active_time),
        });
    }

    // Exceptions, each enriched with a cursor snapshot at the fault position.
    let exc_count = unsafe { sys::dhttd_engine_exception_count(handle) };
    let mut exceptions = Vec::with_capacity(exc_count);

    let mut cursor: sys::DhttdCursorHandle = 0;
    let cur_rc = unsafe { sys::dhttd_cursor_create(handle, &mut cursor) };
    let has_cursor = cur_rc == 0 && cursor != 0;
    if !has_cursor {
        eprintln!(
            "warning: NewCursor failed (0x{cur_rc:08x}); skipping per-exception register/memory snapshots"
        );
    }

    for i in 0..exc_count {
        let mut ev = sys::DhttdException::default();
        if unsafe { sys::dhttd_engine_exception(handle, i, &mut ev) } == 0 {
            continue;
        }
        let params: Vec<String> = ev.parameters[..ev.parameter_count.min(15) as usize]
            .iter()
            .map(|p| format!("0x{p:016x}"))
            .collect();

        let context = if has_cursor {
            unsafe {
                sys::dhttd_cursor_set_position(cursor, ev.position);
            }
            let pc = unsafe { sys::dhttd_cursor_program_counter(cursor) };
            let sp = unsafe { sys::dhttd_cursor_stack_pointer(cursor) };
            let fp = unsafe { sys::dhttd_cursor_frame_pointer(cursor) };
            let mut regs = sys::DhttdAmd64Registers::default();
            let regs_ok = unsafe { sys::dhttd_cursor_amd64_registers(cursor, &mut regs) } != 0;

            let mut faulting_hex = String::new();
            if ev.record_address != 0 {
                let mut buf = [0u8; 8];
                let n = unsafe {
                    sys::dhttd_cursor_read_memory(
                        cursor,
                        ev.record_address,
                        buf.as_mut_ptr(),
                        buf.len(),
                    )
                };
                for b in &buf[..n] {
                    use std::fmt::Write;
                    let _ = write!(&mut faulting_hex, "{b:02x}");
                }
            }

            if regs_ok {
                Some(FaultContext {
                    program_counter_hex: format!("0x{pc:016x}"),
                    stack_pointer_hex: format!("0x{sp:016x}"),
                    frame_pointer_hex: format!("0x{fp:016x}"),
                    registers: convert_registers(regs),
                    faulting_memory_hex: faulting_hex,
                })
            } else {
                None
            }
        } else {
            None
        };

        exceptions.push(Exception {
            position: convert_position(ev.position),
            thread_unique_id: ev.thread_unique_id,
            r#type: ev.r#type,
            code_hex: format!("0x{:08x}", ev.code),
            flags: ev.flags,
            record_address_hex: format!("0x{:016x}", ev.record_address),
            program_counter_hex: format!("0x{:016x}", ev.program_counter),
            parameters_hex: params,
            context,
        });
    }

    if has_cursor {
        unsafe {
            sys::dhttd_cursor_destroy(cursor);
        }
    }

    // Recorder keyframes (engine-internal, typically just trace bookends).
    // Pulled up here so the whole-trace loop can snap its own dense
    // keyframes onto these landmarks: a step whose position matches a
    // recorder keyframe is always emitted as Keyframe, regardless of where
    // the regular interval lands. For sparse engine keyframes this is
    // nearly free; for traces with many recorder keyframes it lets the
    // loader use them as cheap re-sync points.
    let kf_count = unsafe { sys::dhttd_engine_keyframe_count(handle) };
    let mut keyframes: Vec<Position> = Vec::with_capacity(kf_count);
    let mut kf_set: std::collections::HashSet<(u64, u64)> =
        std::collections::HashSet::with_capacity(kf_count);
    for i in 0..kf_count {
        let mut p = sys::DhttdPosition::default();
        if unsafe { sys::dhttd_engine_keyframe(handle, i, &mut p) } == 0 {
            continue;
        }
        kf_set.insert((p.sequence, p.steps));
        keyframes.push(convert_position(p));
    }

    // Whole-trace recording (optional): per thread, single-step from the
    // thread's active_time.min to active_time.max, recording registers at
    // each step. This produces one `RecordedRegion` per thread covering its
    // full lifetime. Subset modes (e.g. --around exception:N, future) use
    // the same schema but populate different bounds.
    let mut recorded_regions: Vec<RecordedRegion> = Vec::new();
    if whole {
        for t in &threads {
            let mut wcur: sys::DhttdCursorHandle = 0;
            let rc = unsafe { sys::dhttd_cursor_create(handle, &mut wcur) };
            if rc != 0 || wcur == 0 {
                eprintln!(
                    "warning: NewCursor failed for thread {} (0x{rc:08x}); skipping",
                    t.unique_id
                );
                continue;
            }

            let start = sys::DhttdPosition {
                sequence: t.active_time.min.sequence,
                steps: t.active_time.min.steps,
            };
            let end = sys::DhttdPosition {
                sequence: t.active_time.max.sequence,
                steps: t.active_time.max.steps,
            };
            unsafe {
                sys::dhttd_cursor_set_position_on_thread(wcur, t.unique_id, start);
            }

            let mut steps: Vec<RegionStep> = Vec::new();
            let mut truncated = false;
            let mut prev: Option<(u64, u64)> = None;
            let mut last_regs: Option<sys::DhttdAmd64Registers> = None;
            let mut since_keyframe: u32 = 0;
            loop {
                let mut cur = sys::DhttdPosition::default();
                if unsafe { sys::dhttd_cursor_get_position(wcur, &mut cur) } == 0 {
                    break;
                }

                let mut regs = sys::DhttdAmd64Registers::default();
                let regs_ok = unsafe { sys::dhttd_cursor_amd64_registers(wcur, &mut regs) } != 0;
                if regs_ok {
                    // Snap to a recorder keyframe if this position is one;
                    // emitting a full keyframe here lets the loader land on
                    // a recorder landmark without replaying any deltas.
                    if kf_set.contains(&(cur.sequence, cur.steps)) {
                        since_keyframe = 0;
                    }
                    let payload = match (since_keyframe, last_regs.as_ref()) {
                        (0, _) | (_, None) => {
                            since_keyframe = 0;
                            StepPayload::Keyframe(convert_registers(regs))
                        }
                        (_, Some(prev_regs)) => StepPayload::Delta(compute_delta(prev_regs, &regs)),
                    };
                    steps.push(RegionStep {
                        position: convert_position(cur),
                        payload,
                        memory_reads: Vec::new(),
                    });
                    last_regs = Some(regs);
                    since_keyframe = (since_keyframe + 1) % keyframe_interval;
                }
                if steps.len() >= max_steps {
                    eprintln!(
                        "warning: hit --max-steps cap ({max_steps}) for thread {}; region truncated",
                        t.unique_id
                    );
                    truncated = true;
                    break;
                }
                if (cur.sequence, cur.steps) >= (end.sequence, end.steps) {
                    break;
                }

                let mut next = sys::DhttdPosition::default();
                let mut stop_reason: u32 = 0;
                let ok = unsafe {
                    sys::dhttd_cursor_replay_forward(wcur, end, 1, &mut next, &mut stop_reason)
                };
                if ok == 0 {
                    break;
                }
                let now = (next.sequence, next.steps);
                if prev == Some(now) {
                    break;
                }
                prev = Some(now);
            }

            unsafe {
                sys::dhttd_cursor_destroy(wcur);
            }

            let step_count = steps.len();
            recorded_regions.push(RecordedRegion {
                anchor: RegionAnchor::ThreadWhole {
                    thread_unique_id: t.unique_id,
                },
                thread_unique_id: t.unique_id,
                position_range: PositionRange {
                    min: convert_position(start),
                    max: convert_position(end),
                },
                step_count,
                truncated,
                keyframe_interval,
                steps,
            });
        }
    }

    unsafe { sys::dhttd_engine_destroy(handle) };

    let dump = Dump {
        trace_path: trace_path.display().to_string(),
        system_info,
        lifetime: convert_range(lifetime),
        modules,
        threads,
        exceptions,
        keyframes,
        recorded_regions,
    };

    let json = serde_json::to_string_pretty(&dump).expect("serialize");
    match out_path {
        Some(p) => std::fs::write(&p, json)?,
        None => println!("{json}"),
    }
    Ok(())
}

# dr-house-ttd-decoder — Design Notes

Status: **In design** — no implementation yet. Capture decisions and rejected
alternatives here; promote to a `CHECKLIST.md` once the major choices are
settled.

> **Revision note:** an earlier draft of these notes assumed there was no
> public TTD replay API and that we'd need to drive WinDbg's `dbgeng.dll`.
> That was wrong. Microsoft ships the
> [TTD Replay Engine SDK](https://github.com/microsoft/WinDbg-Samples/tree/master/TTD/ReplayApi)
> with C++20 headers (NuGet package `Microsoft.TimeTravelDebugging.Apis`),
> usable directly on top of the `TTDReplay.dll` / `TTDReplayCPU.dll` binaries
> we already download. DbgEng is not required. This is the basis the design
> below is built on.

---

## 1. Purpose

This crate is the Rust-side wrapper around the TTD Replay Engine. The rest of
Dr House (the VS Code extension, the differential-diagnosis tools, and
ultimately Copilot) uses it to query information out of a TTD `.run` trace
file: threads, positions, stacks, registers, memory, modules,
exceptions/events, and (eventually) higher-level queries.

The crate is intentionally **a wrapper, not an engine**. All actual replay
work is delegated to Microsoft's native `TTDReplay.dll` / `TTDReplayCPU.dll`
via the documented `IReplayEngine` interface. Our job is:

1. Locate and load the native runtime.
2. Open a trace file via `IReplayEngine`.
3. Expose a small, ergonomic, JSON-friendly Rust API.
4. Convert native errors into a typed Rust `Error`.

---

## 2. Native interface — chosen approach

We use the **TTD Replay Engine SDK** (`Microsoft.TimeTravelDebugging.Apis`).

Key properties of the SDK that drive design:

- **C++20 interface headers** (`IReplayEngine.h` and friends) with abstract
  base classes / virtual methods. Not COM in the registration sense, but
  COM-shaped (interface pointers, factory entry points, GUID versioning).
- **Only runtime dependencies** are `TTDReplay.dll` + `TTDReplayCPU.dll`, both
  already downloaded into `extension/resources/ttd/{x64,arm64}/` by
  `download-ttd.ps1`.
- **GUID-versioned interfaces.** Requesting a stale GUID after an SDK bump
  returns null rather than crashing — gives us a clean compatibility story but
  means we own keeping the GUID in sync with the SDK we built against.
- **Experimental (major version 0).** Source compat is best-effort across
  revisions; binary compat is not promised. We will pin a specific SDK
  version and bump deliberately.
- **Clang-on-Windows compatible.** That matters because it gives us
  realistic options for building a shim with the same toolchain Rust uses.

---

## 3. Bridging C++ → Rust: the real design choice

The Replay Engine is a C++ interface API, not a plain C ABI. Rust cannot
consume C++ virtual classes directly. There are three realistic strategies:

### Option 1 — Hand-written C ABI shim in C++ + Rust bindings by hand
Write a small C++ translation unit that includes `IReplayEngine.h`, calls into
it, and exposes a flat `extern "C"` surface (opaque handles + free functions).
The Rust side declares `extern "C"` and uses `Box`/`*mut Opaque`.

- **Pros:** maximum control; ergonomic Rust on top; easy to keep the surface
  exactly as wide as Dr House needs; isolates SDK churn (we update the shim,
  the Rust API stays stable); compiles cleanly on Clang/MSVC; debuggable.
- **Cons:** we own a C++ file and a `build.rs`; every new operation needs a
  shim function.
- **Verdict:** **leading candidate.**

### Option 2 — `bindgen` over the C++ headers
Let bindgen parse the headers and emit FFI declarations.

- **Pros:** less hand-written code per-method.
- **Cons:** bindgen's C++ support is limited (templates, virtual inheritance,
  STL types in signatures are all rough); the presence of
  `IReplayEngineStl.h` strongly suggests STL types appear at the boundary,
  which is a non-starter for bindgen; produces an unstable, sprawling FFI
  surface that's hard to wrap safely. We'd likely end up writing a shim
  anyway.
- **Verdict:** **rejected** for the public-facing path. May still be useful
  internally for `IdnaBasicTypes.h` / `TTDCommonTypes.h` POD structs.

### Option 3 — Call the C++ vtables directly from Rust (COM-style)
Treat each interface as a `#[repr(C)]` struct of function pointers and call
through it from Rust.

- **Pros:** no C++ shim TU; smaller build pipeline.
- **Cons:** requires the interfaces to actually follow the Itanium-ish vtable
  layout MSVC uses for plain abstract classes — workable for single
  inheritance with no overloads, fragile otherwise. STL-typed parameters
  (`std::wstring_view`, ranges, etc.) cannot cross this boundary. We'd still
  need a C++ helper for any method that touches STL.
- **Verdict:** **possible later** for very simple interfaces, but not the
  baseline.

### Decision
Go with **Option 1**: a thin C++ shim exposing a C ABI, built by `build.rs`
via the `cc` crate, with hand-written `extern "C"` declarations and a safe
Rust wrapper on top.

### Decided
- **SDK acquisition: vendor.** Check a pinned snapshot of the SDK headers
  into the repo under `crates/dr-house-ttd-decoder-sys/vendor/ttd-sdk/` with
  a `VERSION` file recording the NuGet package version it came from and a
  short `UPDATING.md` describing the bump procedure. Rationale: the API is
  flagged "experimental" and GUID-versioned, so we want deliberate bumps
  rather than implicit drift; hermetic builds; no NuGet step in CI.
- **Shim location: in the `-sys` crate.** See section 5.

---

## 4. Runtime DLL loading

`TTDReplay.dll` / `TTDReplayCPU.dll` are not on the standard search path.

Options for finding them at run time:

1. **Delay-load + manual `LoadLibrary` from a known path.** Caller (or the
   crate's `init()`) provides the directory; we `LoadLibrary` from there
   before any replay call. Most flexible.
2. **`SetDllDirectory` / `AddDllDirectory` before first call.** Simple but
   global side effect.
3. **Co-locate next to the Rust consumer's binary.** Brittle for a VS Code
   extension whose layout we control.

**Tentative pick:** option 1 — `Engine::load` takes an explicit path to the
runtime directory; default to looking under `extension/resources/ttd/{arch}/`.
Caller can override via env var (`DR_HOUSE_TTD_RUNTIME_DIR`) for development.

---

## 5. Crate shape

**Decision: split up front.**

- `dr-house-ttd-decoder-sys` — owns the vendored SDK headers, the C++ shim,
  the `build.rs`, and the raw `extern "C"` declarations. Contains `unsafe`
  FFI only; no opinions.
- `dr-house-ttd-decoder` — safe Rust wrapper. Depends on `-sys`. This is
  what the rest of Dr House consumes.

Rationale: matches Rust convention; isolates `unsafe` and the build-script
complexity in one place; lets a future consumer (or test harness) take the
raw layer without dragging the safe wrapper along; makes the SDK-version
bump a self-contained change in one crate.

---

## 6. Public API sketch (subject to revision)

```rust
pub struct Engine { /* opaque, holds loaded runtime */ }

impl Engine {
    pub fn load(runtime_dir: &Path) -> Result<Self>;
    pub fn open_trace(&self, path: &Path) -> Result<Trace>;
}

pub struct Trace { /* opaque */ }

impl Trace {
    pub fn threads(&self) -> Result<Vec<ThreadInfo>>;
    pub fn modules(&self) -> Result<Vec<ModuleInfo>>;
    pub fn events(&self) -> Result<Vec<TraceEvent>>;
    pub fn cursor(&self, thread: ThreadId) -> Result<Cursor>;
}

pub struct Cursor<'t> { /* borrows Trace */ }

impl Cursor<'_> {
    pub fn position(&self) -> Position;          // (sequence, steps)
    pub fn seek(&mut self, position: Position) -> Result<()>;
    pub fn step(&mut self, dir: Direction) -> Result<StepOutcome>;
    pub fn registers(&self) -> Result<Registers>;
    pub fn read_memory(&self, addr: u64, len: usize) -> Result<Vec<u8>>;
    pub fn stack(&self) -> Result<Vec<StackFrame>>;
}
```

All public data types are `serde::Serialize` so the extension can pipe them
to tools / Copilot without further marshaling.

### Open questions
- **Cursor model.** The SDK is built around per-thread cursors; expose that
  shape directly (above) rather than a single global "current position" on
  `Trace`. I expect the SDK forces this anyway — confirm when we read the
  headers.
- **Symbol resolution.** Decided (see section 6): replay engine returns raw
  addresses + module identity + RVA; turning those into
  `module!function+offset` is the job of a sibling crate,
  `dr-house-symbols`. That crate also owns symbol *management* concerns
  (PDB path / symbol server / on-disk cache policy).
- **High-level queries.** See section 6.5 — the answer depends on what
  `IReplayEngine` actually exposes (Scenario A vs. B). Bias toward keeping
  them here until proven otherwise.

---

## 6.5. Consumer-driven API shape

Section 6 sketches what *the SDK* offers. This section asks the inverse:
what does *Copilot* need when investigating a crash, and how should that
shape the API on top of the primitives?

### The investigation, walked through

A crash drops the agent near the end of a trace. Its workflow looks like:

1. **Orient.** "What process, what killed it, when?" One cheap bundle:
   process info + module summary + recording duration + terminating event
   (exception code, faulting address, faulting thread, position) when one
   exists.
2. **Scene of the crime.** "Stack, registers, and disasm around the faulting
   instruction." Another bundle: `frame_context(thread, position)`.
3. **Differential — the canonical TTD backward query.** Given a hypothesis
   like "`mov rax,[rcx+0x10]` faulted because `rcx` is garbage", the
   follow-up is *who last wrote this storage*: last write to a memory range
   before T, last write to a register before T, who allocated/freed an
   object covering this address. These cannot be derived from
   one-instruction-at-a-time stepping at trace scale; TTD has indexes for
   them. **If the API forces step-by-step scanning here, the API has
   failed.**
4. **Broaden.** "Every call to `RtlAllocateHeap` whose return value is in
   `[A, B)`." "Every read of these four bytes before T." The WinDbg
   `TTD.Calls(...)` / `TTD.Memory(...)` queries. High-value, time-range
   filtered.
5. **Drill.** Take the most-recent or most-suspicious result, call
   `frame_context` at *that* position, and loop back to step 3 with a new
   target.

Cross-thread variants of step 4 are common ("did any *other* thread touch
this address between alloc and read?").

### Operations and latency budget

| Operation | Frequency | Latency budget |
|---|---|---|
| Open + metadata bundle | Once per session | Seconds (TTD indexes on open) |
| Terminating-event lookup | Once per session | Cheap |
| Seek to position | Constant | Sub-ms; must be indexed |
| Registers / small memory read | Constant | Microseconds |
| Stack walk | Constant | Milliseconds |
| **Last write to address before T** | Per follow-up turn | **Tens of ms** |
| **`TTD.Calls`-style enumeration** | Per follow-up turn | **Proportional to result count, not trace length** |
| **`TTD.Memory`-style enumeration** | Per follow-up turn | **Same** |
| Disasm around address | Per turn | Microseconds (Rust crate; not SDK) |

The bolded rows are the ones the design must actively protect.

### What we don't know yet

Whether `IReplayEngine` exposes the `Calls` / `Memory` queries as
first-class operations, or whether WinDbg's data model builds them on top
of lower-level primitives (memory-watchpoint stepping + indexed code
coverage). Two scenarios:

- **Scenario A:** SDK exposes the high-value queries directly. Decoder
  crate wraps them; any diagnostics crate is a thin convenience layer.
- **Scenario B:** the high-value queries must be built on top of
  watchpoints + indexed stepping. Then the decoder crate owns them,
  because doing it efficiently requires intimate knowledge of the
  engine's iterators, callbacks, and threading model.

**Working assumption: Scenario B.** Moving queries *out* of this crate
later is mechanical; moving them *in* later means redesigning whatever
sat on the wrong abstraction. Resolve before writing CHECKLIST.md.

### Two-layered API

**Layer 1 — primitives** (mostly mechanical wrap of `IReplayEngine`).
As sketched in section 6.

**Layer 2 — investigation queries** (shape constrained by what the SDK
actually offers):

```rust
impl Trace {
    pub fn termination(&self) -> Option<TerminationEvent>;

    pub fn frame_context(&self, thread: ThreadId, position: Position)
        -> Result<FrameContext>;             // stack + regs + disasm bundle

    pub fn last_write(&self, target: WriteTarget, before: Position)
        -> Result<Option<WriteRecord>>;
    // WriteTarget = Memory { addr, len } | Register(RegId)

    pub fn calls(&self, filter: CallFilter)
        -> Result<impl Iterator<Item = CallRecord> + '_>;

    pub fn memory_accesses(&self, filter: MemoryFilter)
        -> Result<impl Iterator<Item = MemoryAccess> + '_>;
}
```

### Three constraints to hold the line on

1. **All enumeration returns iterators, not `Vec`s.** Copilot wants
   "most-recent N matching X"; we must never materialize a full result
   set into memory or tokens.
2. **Positions are opaque, comparable, serializable.** The agent will
   hand back positions from previous results; it must not have to
   reconstruct anything.
3. **No symbol resolution at this layer.** Filters accept addresses or
   `(ModuleId, rva)` pairs; the symbols crate is responsible for the
   `name -> address` direction.

### Approach: ship, observe, iterate

We do not yet know which operations Copilot will reach for, in what order,
or where the friction points are. We will:

1. Implement Layer 1 (primitives) and the Layer 2 operations we can
   support cheaply.
2. Build a small corpus of sample programs with obvious and non-obvious
   defects, record TTD traces of them.
3. Drive Copilot through investigations of those traces using the API.
4. Use the feedback channel (section 6.6) to capture friction.
5. Iterate on the API shape based on actual usage data, not speculation.

---

## 6.6. Feedback loop for API iteration

The whole point of this crate is to let Copilot diagnose problems
efficiently. To iterate on it intelligently we need data on *how* Copilot
uses it. Two complementary mechanisms, both opt-in via
`Engine::with_feedback(...)`:

### Automatic call telemetry

Every public operation, when feedback is enabled, writes one structured
record to a session journal:

```rust
pub struct CallRecord {
    pub seq: u64,
    pub op: &'static str,            // e.g. "calls", "last_write"
    pub args_summary: String,        // small, redacted; not full args
    pub duration: Duration,
    pub result_kind: ResultKind,     // Ok { rows? } | Err { variant }
    pub trace_position: Option<Position>,
}
```

This is automatic and cheap; the agent does not have to do anything to
produce it. From it we learn: which ops dominate, what's slow, what's
failing, and the *sequences* of calls (which is the most interesting
signal — it tells us what bundles we should be offering).

### Explicit agent feedback

A dedicated method the agent calls when it hits friction:

```rust
impl Trace {
    pub fn feedback(&self, note: FeedbackNote);
}

pub struct FeedbackNote {
    pub kind: FeedbackKind,          // MissingOperation | SlowOperation | AwkwardShape | UnexpectedResult
    pub context: String,             // "I wanted to know X"
    pub workaround: Option<String>,  // "so I did Y, which took N calls"
    pub suggested_api: Option<String>, // free-form: "a method like ..."
}
```

The agent's *system prompt for trace investigation* will instruct it to
call `feedback` whenever it notices it had to do something inefficient or
use a sequence of low-level calls to derive what felt like it should be a
single high-level query.

### Journal output

Both streams write JSONL to a per-session file under a configurable
directory (default: workspace `.dr-house/sessions/<timestamp>.jsonl`).
Reviewing those files — ideally periodically, and especially after
running the sample-program corpus — is the input to the next API design
iteration. Treat the journal as a first-class artifact, not a debugging
aid.

### What this is *not*

- Not user telemetry. This is for *us* iterating on the API, and only
  runs when explicitly enabled.
- Not a benchmark suite. Latencies are recorded for relative comparison
  during an investigation, not for performance regression testing.
- Not a substitute for thinking about the design now. It's how we close
  the loop on what we got wrong.

---

## 7. Error handling

- Single `Error` enum via `thiserror`.
- Variants: `RuntimeLoad`, `TraceOpen`, `InvalidPosition`, `OutOfRange`,
  `NotSupported`, `Internal(String)` for SDK-reported failures we don't
  classify yet.
- The SDK uses `ErrorReporting.h`. Map whatever it reports to either a
  specific variant or `Internal`.

---

## 8. Cross-platform story

Crate is Windows-only in substance.

- Gate the implementation with `#[cfg(target_os = "windows")]`.
- On non-Windows, compile to a stub where every constructor returns
  `Err(Error::Unsupported)`. Keeps workspace `cargo check` green on
  Linux/macOS dev machines and CI.

---

## 9. Testing strategy

- **Unit tests:** position arithmetic, error mapping, anything that doesn't
  touch native code. Cross-platform.
- **Integration tests (Windows only):** open a fixture `.run`, list
  threads/modules, walk a few positions, read registers. Gated by
  `#[cfg(target_os = "windows")]` and skipped if the runtime DLLs aren't
  available.
- **Fixture trace:** check a small pre-recorded `.run` into the repo (or LFS
  if it's larger than a few MB). Probably from a trivial native program like
  `cmd /c echo hi`. **Open question:** acceptable to commit a binary fixture?
- **CI:** add a Windows job that runs the script to fetch DLLs, then runs the
  integration tests.

---

## 10. Things explicitly out of scope (initial version)

- **Recording.** Recorder is a separate concern; this crate only reads.
  (We will likely add a `dr-house-ttd-recorder` later that wraps `TTD.exe` or
  the live-recorder API.)
- **Kernel-mode traces.** TTD is user-mode only.
- **ARM64-trace replay on x64 hosts.** Not supported by TTD itself.
- **x86 host.** Not a supported target.
- **Symbol resolution.** Handled by a separate crate; this one returns
  module + RVA.

---

## 11. Open questions to resolve before writing CHECKLIST.md

1. ~~**SDK acquisition.**~~ **Resolved:** vendor.
2. ~~**Crate split.**~~ **Resolved:** `-sys` + safe wrapper from the start.
3. **`IReplayEngine` reality check.** Originally "does the SDK support the
   Engine/Trace/Cursor split?" — widened by section 6.5 to:
   - Cursor model: per-thread or global?
   - Does the SDK expose `Calls` / `Memory` queries directly (Scenario A)
     or only watchpoint+stepping primitives (Scenario B)?
   - What iteration shape do those queries actually have (callback?
     enumerator? indexed range?) — this dictates whether we can honor the
     "iterators, not Vecs" constraint.
   - Threading model: can multiple cursors be advanced concurrently, or
     does the engine serialize?
   Requires reading `IReplayEngine.h` end-to-end.
4. ~~**Symbolization boundary.**~~ **Resolved:** sibling crate
   `dr-house-symbols`. Placeholder created; real implementation deferred.
5. **Fixture trace.** Acceptable to commit a small binary `.run` for
   tests? If yes, how small, where, and how do we regenerate it?
6. **CI Windows runner.** Acceptable cost / runtime for the integration
   job?
7. **Sample-defect corpus.** What goes in it? At minimum: a clear
   null-deref crash, a use-after-free, a heap overflow caught by a later
   read, a race between two threads on a shared variable, and one
   logic-bug-with-no-exception (wrong value computed and then used much
   later). The corpus lives outside this crate — probably
   `samples/defects/` at the repo root.
8. **Feedback journal location and rotation.** Default path, max size,
   when to rotate. Probably under workspace `.dr-house/sessions/` with
   timestamped files; no rotation needed at this scale.

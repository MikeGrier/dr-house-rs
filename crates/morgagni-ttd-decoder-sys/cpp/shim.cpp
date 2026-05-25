// Copyright (c) Morgagni contributors. Licensed under the MIT License.
//
// Implementation of the stable C ABI declared in shim.h. This file is the
// only place that touches the SDK's C++ types; everything across the ABI
// boundary is plain C structs that the Rust crate mirrors verbatim.

#include "shim.h"

// The SDK headers want an assertion macro. The shim doesn't need anything
// fancy; route to CRT assert for now.
#include <assert.h>
#define DBG_ASSERT(cond) assert(cond)

#include <TTD/IReplayEngineStl.h>
#include <TTD/IReplayEngineRegisters.h>

#include <mutex>
#include <unordered_map>
#include <cstring>
#include <atomic>

using namespace TTD;
using namespace TTD::Replay;

namespace {

struct EngineSlot {
    UniqueReplayEngine engine;
};

struct CursorSlot {
    UniqueCursor cursor;
    DhttdEngineHandle engine_handle;
};

// Simple handle registry; engines are referenced by an opaque u64 so the Rust
// side never sees raw pointers (and we get controlled lifetimes).
std::mutex g_mutex;
std::unordered_map<uint64_t, EngineSlot> g_engines;
std::unordered_map<uint64_t, CursorSlot> g_cursors;
std::atomic<uint64_t> g_next_handle{1};

EngineSlot* slot_for(DhttdEngineHandle h) {
    std::lock_guard<std::mutex> lk(g_mutex);
    auto it = g_engines.find(h);
    return it == g_engines.end() ? nullptr : &it->second;
}

IReplayEngine* engine_for(DhttdEngineHandle h) {
    EngineSlot* s = slot_for(h);
    return s ? s->engine.get() : nullptr;
}

ICursor* cursor_for(DhttdCursorHandle h) {
    std::lock_guard<std::mutex> lk(g_mutex);
    auto it = g_cursors.find(h);
    return it == g_cursors.end() ? nullptr : it->second.cursor.get();
}

uint32_t arch_id_from_processor_arch(uint16_t pa) {
    // Mirrors PROCESSOR_ARCHITECTURE_* constants.
    switch (pa) {
        case 0:  return 1; // x86
        case 9:  return 2; // x64
        case 5:  return 3; // arm
        case 12: return 4; // arm64
        default: return 0;
    }
}

} // namespace

extern "C" uint32_t dhttd_engine_create(DhttdEngineHandle* out) {
    if (!out) return 0xFFFFFFFFu;
    auto [pEngine, rc] = MakeReplayEngine();
    if (rc != 0 || !pEngine) {
        *out = 0;
        return rc == 0 ? 0xFFFFFFFEu : rc;
    }
    uint64_t handle = g_next_handle.fetch_add(1);
    {
        std::lock_guard<std::mutex> lk(g_mutex);
        g_engines.emplace(handle, EngineSlot{ std::move(pEngine) });
    }
    *out = handle;
    return 0;
}

extern "C" int32_t dhttd_engine_initialize(DhttdEngineHandle h, const uint16_t* path_utf16) {
    IReplayEngine* e = engine_for(h);
    if (!e || !path_utf16) return 0;
    return e->Initialize(reinterpret_cast<wchar_t const*>(path_utf16)) ? 1 : 0;
}

extern "C" void dhttd_engine_destroy(DhttdEngineHandle h) {
    std::lock_guard<std::mutex> lk(g_mutex);
    g_engines.erase(h);
}

extern "C" int32_t dhttd_engine_get_system_info(DhttdEngineHandle h, DhttdSystemInfo* out) {
    IReplayEngine* e = engine_for(h);
    if (!e || !out) return 0;
    SystemInfo const& si = e->GetSystemInfo();
    std::memset(out, 0, sizeof(*out));
    out->major_version       = si.MajorVersion;
    out->minor_version       = si.MinorVersion;
    out->process_id          = si.ProcessId;
    out->peb_address         = static_cast<uint64_t>(e->GetPebAddress());
    out->os_major            = si.System.MajorVersion;
    out->os_minor            = si.System.MinorVersion;
    out->os_build            = si.System.BuildNumber;
    out->processors          = si.System.NumberOfProcessors;
    out->processor_level     = si.System.ProcessorLevel;
    out->processor_revision  = si.System.ProcessorRevision;
    out->platform_id         = si.System.PlatformId;
    out->product_type        = si.System.ProductType;
    out->suite_mask          = si.System.SuiteMask;
    out->arch                = arch_id_from_processor_arch(si.System.ProcessorArchitecture);
    return 1;
}

extern "C" int32_t dhttd_engine_get_lifetime(DhttdEngineHandle h, DhttdPositionRange* out) {
    IReplayEngine* e = engine_for(h);
    if (!e || !out) return 0;
    PositionRange const& lt = e->GetLifetime();
    out->min.sequence = static_cast<uint64_t>(lt.Min.Sequence);
    out->min.steps    = static_cast<uint64_t>(lt.Min.Steps);
    out->max.sequence = static_cast<uint64_t>(lt.Max.Sequence);
    out->max.steps    = static_cast<uint64_t>(lt.Max.Steps);
    return 1;
}

extern "C" size_t dhttd_engine_module_instance_count(DhttdEngineHandle h) {
    IReplayEngine* e = engine_for(h);
    return e ? e->GetModuleInstanceCount() : 0;
}

extern "C" int32_t dhttd_engine_module_instance(
    DhttdEngineHandle h,
    size_t index,
    DhttdModule* out_module,
    uint16_t* name_buffer_utf16,
    size_t name_capacity_chars,
    size_t* out_name_length_chars)
{
    IReplayEngine* e = engine_for(h);
    if (!e || !out_module) return 0;
    if (index >= e->GetModuleInstanceCount()) return 0;
    ModuleInstance const& mi = e->GetModuleInstanceList()[index];
    Module const& m = *mi.pModule;

    std::memset(out_module, 0, sizeof(*out_module));
    out_module->address         = static_cast<uint64_t>(m.Address);
    out_module->size            = m.Size;
    out_module->checksum        = m.Checksum;
    out_module->timestamp       = m.Timestamp;
    out_module->load_sequence   = static_cast<uint64_t>(mi.LoadTime);
    out_module->unload_sequence = static_cast<uint64_t>(mi.UnloadTime);

    size_t name_len = m.NameLength;
    if (name_buffer_utf16 && name_capacity_chars > 0) {
        size_t to_copy = name_len;
        if (to_copy >= name_capacity_chars) {
            to_copy = name_capacity_chars - 1;
        }
        std::memcpy(name_buffer_utf16, m.pName, to_copy * sizeof(wchar_t));
        name_buffer_utf16[to_copy] = 0;
        if (out_name_length_chars) *out_name_length_chars = to_copy;
    } else if (out_name_length_chars) {
        *out_name_length_chars = name_len;
    }
    return 1;
}

extern "C" size_t dhttd_engine_thread_count(DhttdEngineHandle h) {
    IReplayEngine* e = engine_for(h);
    return e ? e->GetThreadCount() : 0;
}

extern "C" int32_t dhttd_engine_thread(DhttdEngineHandle h, size_t index, DhttdThread* out) {
    IReplayEngine* e = engine_for(h);
    if (!e || !out) return 0;
    if (index >= e->GetThreadCount()) return 0;
    ThreadInfo const& t = e->GetThreadList()[index];
    out->unique_id        = static_cast<uint32_t>(t.UniqueId);
    out->os_thread_id     = static_cast<uint32_t>(t.Id);
    out->lifetime.min.sequence    = static_cast<uint64_t>(t.Lifetime.Min.Sequence);
    out->lifetime.min.steps       = static_cast<uint64_t>(t.Lifetime.Min.Steps);
    out->lifetime.max.sequence    = static_cast<uint64_t>(t.Lifetime.Max.Sequence);
    out->lifetime.max.steps       = static_cast<uint64_t>(t.Lifetime.Max.Steps);
    out->active_time.min.sequence = static_cast<uint64_t>(t.ActiveTime.Min.Sequence);
    out->active_time.min.steps    = static_cast<uint64_t>(t.ActiveTime.Min.Steps);
    out->active_time.max.sequence = static_cast<uint64_t>(t.ActiveTime.Max.Sequence);
    out->active_time.max.steps    = static_cast<uint64_t>(t.ActiveTime.Max.Steps);
    return 1;
}

extern "C" size_t dhttd_engine_exception_count(DhttdEngineHandle h) {
    IReplayEngine* e = engine_for(h);
    return e ? e->GetExceptionEventCount() : 0;
}

extern "C" int32_t dhttd_engine_exception(DhttdEngineHandle h, size_t index, DhttdException* out) {
    IReplayEngine* e = engine_for(h);
    if (!e || !out) return 0;
    if (index >= e->GetExceptionEventCount()) return 0;
    ExceptionEvent const& ev = e->GetExceptionEventList()[index];
    std::memset(out, 0, sizeof(*out));
    out->position.sequence = static_cast<uint64_t>(ev.Position.Sequence);
    out->position.steps    = static_cast<uint64_t>(ev.Position.Steps);
    out->thread_unique_id  = ev.pThreadInfo ? static_cast<uint32_t>(ev.pThreadInfo->UniqueId) : 0;
    out->type              = static_cast<uint32_t>(ev.Type);
    out->code              = ev.Code;
    out->flags             = ev.Flags;
    out->record_address    = static_cast<uint64_t>(ev.RecordAddress);
    out->program_counter   = static_cast<uint64_t>(ev.ProgramCounter);
    out->parameter_count   = ev.ParameterCount;
    size_t pc = ev.ParameterCount;
    if (pc > 15) pc = 15;
    for (size_t i = 0; i < pc; ++i) {
        out->parameters[i] = ev.Parameters[i];
    }
    return 1;
}

extern "C" size_t dhttd_engine_keyframe_count(DhttdEngineHandle h) {
    IReplayEngine* e = engine_for(h);
    return e ? e->GetKeyframeCount() : 0;
}

extern "C" int32_t dhttd_engine_keyframe(DhttdEngineHandle h, size_t index, DhttdPosition* out) {
    IReplayEngine* e = engine_for(h);
    if (!e || !out) return 0;
    if (index >= e->GetKeyframeCount()) return 0;
    Position const& p = e->GetKeyframeList()[index];
    out->sequence = static_cast<uint64_t>(p.Sequence);
    out->steps    = static_cast<uint64_t>(p.Steps);
    return 1;
}

// ---- Cursor APIs ----

extern "C" uint32_t dhttd_cursor_create(DhttdEngineHandle eh, DhttdCursorHandle* out) {
    if (!out) return 0xFFFFFFFFu;
    IReplayEngine* e = engine_for(eh);
    if (!e) { *out = 0; return 0xFFFFFFFDu; }
    ICursor* raw = e->NewCursor();
    if (!raw) { *out = 0; return 0xFFFFFFFCu; }
    uint64_t handle = g_next_handle.fetch_add(1);
    {
        std::lock_guard<std::mutex> lk(g_mutex);
        g_cursors.emplace(handle, CursorSlot{ UniqueCursor(raw), eh });
    }
    *out = handle;
    return 0;
}

extern "C" void dhttd_cursor_destroy(DhttdCursorHandle h) {
    std::lock_guard<std::mutex> lk(g_mutex);
    g_cursors.erase(h);
}

extern "C" void dhttd_cursor_set_position(DhttdCursorHandle h, DhttdPosition p) {
    ICursor* c = cursor_for(h);
    if (!c) return;
    Position pos{ static_cast<SequenceId>(p.sequence), static_cast<StepCount>(p.steps) };
    c->SetPosition(pos);
}

extern "C" void dhttd_cursor_set_position_on_thread(
    DhttdCursorHandle h,
    uint32_t unique_thread_id,
    DhttdPosition p)
{
    ICursor* c = cursor_for(h);
    if (!c) return;
    Position pos{ static_cast<SequenceId>(p.sequence), static_cast<StepCount>(p.steps) };
    c->SetPositionOnThread(static_cast<UniqueThreadId>(unique_thread_id), pos);
}

extern "C" uint64_t dhttd_cursor_program_counter(DhttdCursorHandle h) {
    ICursor* c = cursor_for(h);
    return c ? static_cast<uint64_t>(c->GetProgramCounter()) : 0;
}

extern "C" uint64_t dhttd_cursor_stack_pointer(DhttdCursorHandle h) {
    ICursor* c = cursor_for(h);
    return c ? static_cast<uint64_t>(c->GetStackPointer()) : 0;
}

extern "C" uint64_t dhttd_cursor_frame_pointer(DhttdCursorHandle h) {
    ICursor* c = cursor_for(h);
    return c ? static_cast<uint64_t>(c->GetFramePointer()) : 0;
}

extern "C" int32_t dhttd_cursor_amd64_registers(DhttdCursorHandle h, DhttdAmd64Registers* out) {
    ICursor* c = cursor_for(h);
    if (!c || !out) return 0;
    CROSS_PLATFORM_CONTEXT ctx = c->GetCrossPlatformContext();
    AMD64_CONTEXT const& a = ctx.Amd64Context;
    out->rax = a.Rax; out->rcx = a.Rcx; out->rdx = a.Rdx; out->rbx = a.Rbx;
    out->rsp = a.Rsp; out->rbp = a.Rbp; out->rsi = a.Rsi; out->rdi = a.Rdi;
    out->r8  = a.R8;  out->r9  = a.R9;  out->r10 = a.R10; out->r11 = a.R11;
    out->r12 = a.R12; out->r13 = a.R13; out->r14 = a.R14; out->r15 = a.R15;
    out->rip = a.Rip;
    out->eflags = a.EFlags;
    out->context_flags = a.ContextFlags;
    return 1;
}

extern "C" size_t dhttd_cursor_read_memory(
    DhttdCursorHandle h,
    uint64_t address,
    uint8_t* out_buf,
    size_t len)
{
    ICursor* c = cursor_for(h);
    if (!c || !out_buf || len == 0) return 0;
    BufferView view{ out_buf, len };
    MemoryBuffer mb = c->QueryMemoryBuffer(static_cast<GuestAddress>(address), view);
    return mb.Memory.Size;
}

extern "C" int32_t dhttd_cursor_get_position(DhttdCursorHandle h, DhttdPosition* out) {
    ICursor* c = cursor_for(h);
    if (!c || !out) return 0;
    Position const& p = c->GetPosition();
    out->sequence = static_cast<uint64_t>(p.Sequence);
    out->steps    = static_cast<uint64_t>(p.Steps);
    return 1;
}

extern "C" int32_t dhttd_cursor_replay_forward(
    DhttdCursorHandle h,
    DhttdPosition limit,
    uint64_t step_count,
    DhttdPosition* out_position,
    uint32_t* out_stop_reason)
{
    ICursor* c = cursor_for(h);
    if (!c) return 0;
    Position lim{ static_cast<SequenceId>(limit.sequence), static_cast<StepCount>(limit.steps) };
    auto result = c->ReplayForward(lim, static_cast<StepCount>(step_count));
    if (out_stop_reason) *out_stop_reason = static_cast<uint32_t>(result.StopReason);
    if (out_position) {
        Position const& p = c->GetPosition();
        out_position->sequence = static_cast<uint64_t>(p.Sequence);
        out_position->steps    = static_cast<uint64_t>(p.Steps);
    }
    return 1;
}

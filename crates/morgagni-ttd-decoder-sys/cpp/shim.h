// Copyright (c) Morgagni contributors. Licensed under the MIT License.
//
// Stable C ABI exposed by the TTD Replay Engine shim. All types here are
// hand-defined POD structs that the Rust side mirrors exactly. We never
// expose the SDK's C++ types across this boundary; the shim translates.

#pragma once

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

// Opaque handles. 0 means invalid.
typedef uint64_t DhttdEngineHandle;
typedef uint64_t DhttdCursorHandle;

// 128-bit position {sequence, steps}, mirroring TTD::Replay::Position layout.
typedef struct DhttdPosition {
    uint64_t sequence;
    uint64_t steps;
} DhttdPosition;

typedef struct DhttdPositionRange {
    DhttdPosition min;
    DhttdPosition max;
} DhttdPositionRange;

typedef struct DhttdSystemInfo {
    uint16_t major_version;
    uint16_t minor_version;
    uint32_t process_id;
    uint64_t peb_address;
    // OS / system info (subset).
    uint32_t os_major;
    uint32_t os_minor;
    uint32_t os_build;
    uint32_t processors;
    uint32_t processor_level;
    uint32_t processor_revision;
    uint32_t platform_id;
    uint32_t product_type;
    uint32_t suite_mask;
    // Architecture hint: 0=unknown, 1=x86, 2=x64, 3=arm, 4=arm64.
    uint32_t arch;
} DhttdSystemInfo;

typedef struct DhttdModule {
    uint64_t address;
    uint64_t size;
    uint32_t checksum;
    uint32_t timestamp;
    uint64_t load_sequence;
    uint64_t unload_sequence;
    // UTF-16 LE module name, NUL-terminated, written into a caller-provided
    // buffer. `name_capacity_chars` is the buffer size in wchar_t units; on
    // return `name_length_chars` is set to the number of chars written
    // (excluding NUL).
} DhttdModule;

typedef struct DhttdThread {
    uint32_t unique_id;
    uint32_t os_thread_id;
    DhttdPositionRange lifetime;
    DhttdPositionRange active_time;
} DhttdThread;

typedef struct DhttdException {
    DhttdPosition position;
    uint32_t thread_unique_id;
    uint32_t type;             // ExceptionType enum value
    uint32_t code;             // Win32 exception code (e.g. 0xC0000005)
    uint32_t flags;
    uint64_t record_address;
    uint64_t program_counter;
    uint32_t parameter_count;
    uint64_t parameters[15];
} DhttdException;

// ---- Engine lifecycle ----

// Returns 0 on success and stores a non-zero handle in *out. Non-zero return
// is the engine creation error code from the SDK.
uint32_t dhttd_engine_create(DhttdEngineHandle* out);

// Returns 1 on success, 0 on failure. `trace_path_utf16` is a NUL-terminated
// UTF-16 string.
int32_t dhttd_engine_initialize(DhttdEngineHandle engine, const uint16_t* trace_path_utf16);

void dhttd_engine_destroy(DhttdEngineHandle engine);

// ---- Top-level queries ----

int32_t dhttd_engine_get_system_info(DhttdEngineHandle engine, DhttdSystemInfo* out);
int32_t dhttd_engine_get_lifetime(DhttdEngineHandle engine, DhttdPositionRange* out);

size_t  dhttd_engine_module_instance_count(DhttdEngineHandle engine);
// Reads one ModuleInstance + its module metadata. Writes module name into
// caller buffer. Returns 1 on success, 0 on out-of-range index.
int32_t dhttd_engine_module_instance(
    DhttdEngineHandle engine,
    size_t index,
    DhttdModule* out_module,
    uint16_t* name_buffer_utf16,
    size_t name_capacity_chars,
    size_t* out_name_length_chars
);

size_t  dhttd_engine_thread_count(DhttdEngineHandle engine);
int32_t dhttd_engine_thread(DhttdEngineHandle engine, size_t index, DhttdThread* out);

size_t  dhttd_engine_exception_count(DhttdEngineHandle engine);
int32_t dhttd_engine_exception(DhttdEngineHandle engine, size_t index, DhttdException* out);

size_t  dhttd_engine_keyframe_count(DhttdEngineHandle engine);
int32_t dhttd_engine_keyframe(DhttdEngineHandle engine, size_t index, DhttdPosition* out);

// ---- Cursor lifecycle and queries ----
//
// Snapshot of AMD64 integer registers + RIP/RFLAGS at a cursor position.
// Subset of the SDK's CONTEXT used for null-deref-class investigation.
typedef struct DhttdAmd64Registers {
    uint64_t rax, rcx, rdx, rbx, rsp, rbp, rsi, rdi;
    uint64_t r8,  r9,  r10, r11, r12, r13, r14, r15;
    uint64_t rip;
    uint32_t eflags;
    uint32_t context_flags;
} DhttdAmd64Registers;

// Returns 0 on success and stores a non-zero handle in *out.
uint32_t dhttd_cursor_create(DhttdEngineHandle engine, DhttdCursorHandle* out);
void     dhttd_cursor_destroy(DhttdCursorHandle cursor);

// "Rounds up" to the closest valid position per the SDK contract.
void     dhttd_cursor_set_position(DhttdCursorHandle cursor, DhttdPosition position);

// Like dhttd_cursor_set_position, but seeds the cursor onto a specific thread
// identified by its UniqueThreadId. Required when iterating per-thread state
// in traces with more than one thread.
void     dhttd_cursor_set_position_on_thread(
    DhttdCursorHandle cursor,
    uint32_t unique_thread_id,
    DhttdPosition position
);

// Default-thread (cursor's current) queries. Return 0 on invalid cursor.
uint64_t dhttd_cursor_program_counter(DhttdCursorHandle cursor);
uint64_t dhttd_cursor_stack_pointer(DhttdCursorHandle cursor);
uint64_t dhttd_cursor_frame_pointer(DhttdCursorHandle cursor);

// Fills *out with the AMD64 GPR snapshot from the cursor's CROSS_PLATFORM_CONTEXT.
// Returns 1 on success, 0 on invalid cursor.
int32_t  dhttd_cursor_amd64_registers(DhttdCursorHandle cursor, DhttdAmd64Registers* out);

// Reads up to `len` bytes from guest memory at `address` into `out_buf`.
// Returns the number of bytes actually filled (may be < len, or 0 on miss).
size_t   dhttd_cursor_read_memory(
    DhttdCursorHandle cursor,
    uint64_t address,
    uint8_t* out_buf,
    size_t len
);

// Reads the cursor's current Position. Returns 1 on success, 0 on invalid cursor.
int32_t  dhttd_cursor_get_position(DhttdCursorHandle cursor, DhttdPosition* out);

// Step forward up to `step_count` steps, stopping no later than `limit`.
// Returns 1 on success (regardless of stop reason), 0 on invalid cursor.
// On success, writes the resulting position into *out_position (if non-null)
// and the SDK EventType StopReason as a u32 into *out_stop_reason (if non-null).
int32_t  dhttd_cursor_replay_forward(
    DhttdCursorHandle cursor,
    DhttdPosition limit,
    uint64_t step_count,
    DhttdPosition* out_position,
    uint32_t* out_stop_reason
);

#ifdef __cplusplus
} // extern "C"
#endif

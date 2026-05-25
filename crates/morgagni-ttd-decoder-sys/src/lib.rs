//! Raw FFI bindings to the TTD Replay Engine via the C ABI shim.
//!
//! This crate is intentionally minimal: it exposes the exact C surface
//! declared in `cpp/shim.h` and nothing else. All safe wrapping happens
//! one layer up in `morgagni-ttd-decoder`.
//!
//! # Runtime requirements
//!
//! The compiled binary depends on `TTDReplay.dll` and `TTDReplayCPU.dll`.
//! Both ship under `extension/resources/ttd/x64/` (downloaded by
//! `.github/scripts/download-ttd.ps1`). They are not bundled into the
//! executable; consumers must ensure they are reachable via the standard
//! DLL search path at run time.

#![allow(non_camel_case_types)]

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DhttdPosition {
    pub sequence: u64,
    pub steps: u64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DhttdPositionRange {
    pub min: DhttdPosition,
    pub max: DhttdPosition,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct DhttdSystemInfo {
    pub major_version: u16,
    pub minor_version: u16,
    pub process_id: u32,
    pub peb_address: u64,
    pub os_major: u32,
    pub os_minor: u32,
    pub os_build: u32,
    pub processors: u32,
    pub processor_level: u32,
    pub processor_revision: u32,
    pub platform_id: u32,
    pub product_type: u32,
    pub suite_mask: u32,
    /// 0=unknown, 1=x86, 2=x64, 3=arm, 4=arm64
    pub arch: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct DhttdModule {
    pub address: u64,
    pub size: u64,
    pub checksum: u32,
    pub timestamp: u32,
    pub load_sequence: u64,
    pub unload_sequence: u64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct DhttdThread {
    pub unique_id: u32,
    pub os_thread_id: u32,
    pub lifetime: DhttdPositionRange,
    pub active_time: DhttdPositionRange,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct DhttdException {
    pub position: DhttdPosition,
    pub thread_unique_id: u32,
    pub r#type: u32,
    pub code: u32,
    pub flags: u32,
    pub record_address: u64,
    pub program_counter: u64,
    pub parameter_count: u32,
    pub parameters: [u64; 15],
}

pub type DhttdEngineHandle = u64;
pub type DhttdCursorHandle = u64;

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct DhttdAmd64Registers {
    pub rax: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rbx: u64,
    pub rsp: u64,
    pub rbp: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rip: u64,
    pub eflags: u32,
    pub context_flags: u32,
}

#[cfg(target_os = "windows")]
unsafe extern "C" {
    pub fn dhttd_engine_create(out: *mut DhttdEngineHandle) -> u32;
    pub fn dhttd_engine_initialize(engine: DhttdEngineHandle, trace_path_utf16: *const u16) -> i32;
    pub fn dhttd_engine_destroy(engine: DhttdEngineHandle);
    pub fn dhttd_engine_get_system_info(
        engine: DhttdEngineHandle,
        out: *mut DhttdSystemInfo,
    ) -> i32;
    pub fn dhttd_engine_get_lifetime(
        engine: DhttdEngineHandle,
        out: *mut DhttdPositionRange,
    ) -> i32;
    pub fn dhttd_engine_module_instance_count(engine: DhttdEngineHandle) -> usize;
    pub fn dhttd_engine_module_instance(
        engine: DhttdEngineHandle,
        index: usize,
        out_module: *mut DhttdModule,
        name_buffer_utf16: *mut u16,
        name_capacity_chars: usize,
        out_name_length_chars: *mut usize,
    ) -> i32;
    pub fn dhttd_engine_thread_count(engine: DhttdEngineHandle) -> usize;
    pub fn dhttd_engine_thread(
        engine: DhttdEngineHandle,
        index: usize,
        out: *mut DhttdThread,
    ) -> i32;
    pub fn dhttd_engine_exception_count(engine: DhttdEngineHandle) -> usize;
    pub fn dhttd_engine_exception(
        engine: DhttdEngineHandle,
        index: usize,
        out: *mut DhttdException,
    ) -> i32;
    pub fn dhttd_engine_keyframe_count(engine: DhttdEngineHandle) -> usize;
    pub fn dhttd_engine_keyframe(
        engine: DhttdEngineHandle,
        index: usize,
        out: *mut DhttdPosition,
    ) -> i32;

    pub fn dhttd_cursor_create(engine: DhttdEngineHandle, out: *mut DhttdCursorHandle) -> u32;
    pub fn dhttd_cursor_destroy(cursor: DhttdCursorHandle);
    pub fn dhttd_cursor_set_position(cursor: DhttdCursorHandle, position: DhttdPosition);
    pub fn dhttd_cursor_set_position_on_thread(
        cursor: DhttdCursorHandle,
        unique_thread_id: u32,
        position: DhttdPosition,
    );
    pub fn dhttd_cursor_program_counter(cursor: DhttdCursorHandle) -> u64;
    pub fn dhttd_cursor_stack_pointer(cursor: DhttdCursorHandle) -> u64;
    pub fn dhttd_cursor_frame_pointer(cursor: DhttdCursorHandle) -> u64;
    pub fn dhttd_cursor_amd64_registers(
        cursor: DhttdCursorHandle,
        out: *mut DhttdAmd64Registers,
    ) -> i32;
    pub fn dhttd_cursor_read_memory(
        cursor: DhttdCursorHandle,
        address: u64,
        out_buf: *mut u8,
        len: usize,
    ) -> usize;
    pub fn dhttd_cursor_get_position(cursor: DhttdCursorHandle, out: *mut DhttdPosition) -> i32;
    pub fn dhttd_cursor_replay_forward(
        cursor: DhttdCursorHandle,
        limit: DhttdPosition,
        step_count: u64,
        out_position: *mut DhttdPosition,
        out_stop_reason: *mut u32,
    ) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_layout_is_two_u64s() {
        assert_eq!(core::mem::size_of::<DhttdPosition>(), 16);
        assert_eq!(core::mem::align_of::<DhttdPosition>(), 8);
    }

    #[test]
    fn exception_struct_default_zeroes_parameters() {
        let ex = DhttdException::default();
        assert!(ex.parameters.iter().all(|&p| p == 0));
        assert_eq!(ex.parameter_count, 0);
    }
}

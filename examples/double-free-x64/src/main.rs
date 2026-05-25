//! Trace bait: create a dedicated heap with HEAP_GENERATE_EXCEPTIONS, allocate
//! a block, free it, and free it again. The runtime detects the corruption and
//! raises STATUS_HEAP_CORRUPTION (or fast-fails) which TTD records.

use std::ffi::c_void;
use std::hint::black_box;
use std::ptr::null_mut;

#[link(name = "kernel32")]
extern "system" {
    fn HeapCreate(options: u32, initial: usize, max: usize) -> *mut c_void;
    fn HeapAlloc(heap: *mut c_void, flags: u32, bytes: usize) -> *mut u8;
    fn HeapFree(heap: *mut c_void, flags: u32, mem: *mut u8) -> i32;
}

const HEAP_GENERATE_EXCEPTIONS: u32 = 0x4;

#[inline(never)]
fn free_block(heap: *mut c_void, p: *mut u8) -> i32 {
    unsafe { HeapFree(heap, 0, p) }
}

fn main() {
    println!("double-free-x64: creating heap with HEAP_GENERATE_EXCEPTIONS");
    let heap = unsafe { HeapCreate(HEAP_GENERATE_EXCEPTIONS, 0, 0) };
    assert!(!heap.is_null(), "HeapCreate failed");

    let p = unsafe { HeapAlloc(heap, 0, 64) };
    assert!(!p.is_null(), "HeapAlloc failed");

    // Touch it so the trace shows the live write.
    unsafe { core::ptr::write(p as *mut u32, 0xC0DE_F00D) };

    println!("double-free-x64: first free (legitimate)");
    let ok = free_block(heap, black_box(p));
    assert!(ok != 0, "first HeapFree failed");

    println!("double-free-x64: second free (corruption)");
    let _ = free_block(black_box(heap), black_box(p));
    let _ = null_mut::<u8>();
    println!("you should never see this");
}

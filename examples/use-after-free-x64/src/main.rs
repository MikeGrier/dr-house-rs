//! Trace bait: allocate one page with `VirtualAlloc`, release it with
//! `VirtualFree(MEM_RELEASE)`, then write into the released address. The page
//! is unmapped, so the store deterministically faults.

use std::hint::black_box;
use std::ptr::null_mut;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn VirtualAlloc(addr: *mut u8, size: usize, alloc_type: u32, protect: u32) -> *mut u8;
    fn VirtualFree(addr: *mut u8, size: usize, free_type: u32) -> i32;
}

const MEM_COMMIT: u32 = 0x1000;
const MEM_RESERVE: u32 = 0x2000;
const MEM_RELEASE: u32 = 0x8000;
const PAGE_READWRITE: u32 = 0x04;

#[inline(never)]
fn poke(p: *mut u32, value: u32) {
    unsafe { core::ptr::write(p, value) };
}

fn main() {
    println!("use-after-free-x64: allocating one page");
    let page =
        unsafe { VirtualAlloc(null_mut(), 0x1000, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE) };
    assert!(!page.is_null(), "VirtualAlloc failed");

    // Touch it once so the trace shows a legitimate live write.
    unsafe { core::ptr::write(page as *mut u32, 0xC0DE_F00D) };

    println!("use-after-free-x64: releasing the page (UAF window opens)");
    let ok = unsafe { VirtualFree(page, 0, MEM_RELEASE) };
    assert!(ok != 0, "VirtualFree failed");

    println!("use-after-free-x64: writing into the freed page");
    poke(black_box(page as *mut u32), black_box(0x4242_4242));
    println!("you should never see this");
}

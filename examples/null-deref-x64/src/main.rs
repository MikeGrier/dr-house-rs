//! Trace bait: a Rust program whose only job is to crash with a write to
//! address 0 so the TTD recorder captures a deterministic null pointer
//! dereference. Used as the ground-truth scenario for Morgagni v0.

use std::hint::black_box;

#[inline(never)]
fn poke(p: *mut u32, value: u32) {
    // SAFETY: intentionally unsafe. `p` is null at the call site below.
    unsafe { core::ptr::write(p, value) };
}

fn main() {
    println!("null-deref-x64: about to write through a null pointer");
    let p: *mut u32 = core::ptr::null_mut();
    poke(black_box(p), black_box(0x4242_4242));
    // Unreachable.
    println!("you should never see this");
}

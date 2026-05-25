//! Trace bait: a "polluter" fills a stack buffer with a poison value, returns,
//! then a "victim" allocates the same shape uninitialized in the same frame and
//! reads it. The value is then dereferenced as a pointer, faulting on a
//! deterministic, recognisable address.

use std::hint::black_box;
use std::mem::MaybeUninit;

const POISON: u64 = 0xDEAD_BEEF_CAFE_F000;

#[inline(never)]
fn polluter() -> u64 {
    let mut buf = [0u64; 64];
    for i in 0..buf.len() {
        buf[i] = POISON | i as u64;
    }
    black_box(buf[42])
}

#[inline(never)]
fn victim() -> u64 {
    let buf: [MaybeUninit<u64>; 64] = unsafe { MaybeUninit::uninit().assume_init() };
    unsafe { buf[42].assume_init() }
}

fn main() {
    println!("uninit-read-x64: priming the stack with poison");
    let _ = polluter();

    println!("uninit-read-x64: reading uninitialized stack");
    let garbage = victim();
    println!("uninit-read-x64: got {garbage:#x}, dereferencing as pointer");

    // OR a kernel-space mask so the faulting address always lands in unmapped
    // memory regardless of what bytes the stack slot happened to hold. The low
    // bits still encode the uninit value, so the AV address is a fingerprint
    // of the uninit read.
    let p = (black_box(garbage) | 0xFFFF_8000_0000_0000) as *mut u32;
    unsafe { core::ptr::write(p, 0x4242_4242) };
    println!("you should never see this");
}

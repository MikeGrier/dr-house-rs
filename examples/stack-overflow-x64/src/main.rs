//! Trace bait: walk a raw pointer well past the end of a 16-byte stack array
//! and stamp 0xCC bytes over whatever is up the frame, including the saved
//! return address. The function's RET then transfers control to
//! 0xCCCCCCCC`CCCCCCCC and the CPU faults trying to fetch the instruction.

use std::hint::black_box;

#[inline(never)]
fn smash() {
    let mut buf = [0u8; 16];
    let p = buf.as_mut_ptr();
    for i in 0..256 {
        unsafe { core::ptr::write(p.add(i), 0xCC) };
    }
    black_box(&buf);
}

fn main() {
    println!("stack-overflow-x64: smashing 256 bytes onto a 16-byte buffer");
    smash();
    println!("you should never see this");
}

//! Poor-man's allocation profiler: captures a backtrace at the N-th allocation
//! inside a single encode call, so the *real* per-band/per-channel allocation
//! sites can be identified (the static count of `vec!`/`.to_vec()` literals does
//! not explain the measured hundreds of allocations per frame).
//!
//! ```text
//! cargo run --example alloc_trace            # 48k stereo, capture after warm-up
//! ALLOC_SKIP=500 cargo run --example alloc_trace
//! ```
//!
//! Build with the **dev** profile for debug symbols.

use oprs::coder::Encoder;
use oprs::{Application, Bitrate, Channels, SampleRate};
use std::alloc::{GlobalAlloc, Layout, System};
use std::backtrace::Backtrace;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

static COUNT: AtomicUsize = AtomicUsize::new(0);
static TARGET: AtomicUsize = AtomicUsize::new(usize::MAX);
static IN_HOOK: AtomicBool = AtomicBool::new(false);
static mut BT: Option<Backtrace> = None;

struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let n = COUNT.fetch_add(1, Ordering::Relaxed);
        if n == TARGET.load(Ordering::Relaxed) && !IN_HOOK.swap(true, Ordering::SeqCst) {
            let bt = Backtrace::force_capture();
            unsafe { BT = Some(bt) };
            IN_HOOK.store(false, Ordering::SeqCst);
        }
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GA: Counting = Counting;

fn main() {
    let skip: usize = std::env::var("ALLOC_SKIP")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(500);

    let channels = Channels::Stereo;
    let frame = 960; // 20 ms @ 48 kHz
    let pcm: Vec<f32> = (0..frame * channels as usize)
        .map(|i| ((i as f32) * 0.01).sin() * 0.3)
        .collect();
    let mut enc = Encoder::new(SampleRate::Hz48000, channels, Application::Audio).unwrap();
    enc.set_bitrate(Bitrate::BitsPerSecond(96000)).unwrap();
    enc.set_complexity(10).unwrap();
    let mut out = vec![0u8; 1500];

    for _ in 0..5 {
        enc.encode_float(&pcm, &mut out).unwrap();
    }

    let base = COUNT.load(Ordering::Relaxed);
    TARGET.store(base + skip, Ordering::SeqCst);
    let n = enc.encode_float(&pcm, &mut out).unwrap();
    let total = COUNT.load(Ordering::Relaxed) - base;
    TARGET.store(usize::MAX, Ordering::SeqCst);

    println!("one encode call: {n} bytes, {total} allocations");
    // `static mut` cannot be referenced directly in edition 2024; go through a raw pointer.
    let bt_cell: *const Option<Backtrace> = &raw const BT;
    match unsafe { (*bt_cell).as_ref() } {
        Some(bt) => println!("--- backtrace of allocation #{skip} of that call ---\n{bt}"),
        None => println!("(no backtrace captured; ALLOC_SKIP beyond this call's allocation count)"),
    }
}

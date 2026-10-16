//! Counts heap allocations per encode/decode call — quantifies how far the port
//! is from the C code's `VARDECL` (stack) allocations in the per-frame path.
//!
//! ```text
//! cargo run --release --example alloc_count
//! ```

use oprs::coder::{Decoder, Encoder};
use oprs::packet::Packet;
use oprs::{Application, Bitrate, Channels, MutSignals, SampleRate};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);

struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(new_size, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static A: Counting = Counting;

const FRAMES: usize = 200;

fn measure(name: &str, rate: SampleRate, channels: Channels, frame: usize, app: Application, bitrate: i32) {
    let frame_len = frame * channels as usize;
    let pcm: Vec<f32> = (0..frame_len).map(|i| ((i as f32) * 0.01).sin() * 0.3).collect();

    let mut enc = Encoder::new(rate, channels, app).expect("encoder");
    enc.set_bitrate(Bitrate::BitsPerSecond(bitrate)).unwrap();
    enc.set_complexity(10).unwrap();

    let mut out = vec![0u8; 1500];
    // warm up (state allocation happens here)
    for _ in 0..5 {
        enc.encode_float(&pcm, &mut out).unwrap();
    }

    let a0 = ALLOCS.load(Ordering::Relaxed);
    let b0 = BYTES.load(Ordering::Relaxed);
    let mut packets = Vec::with_capacity(FRAMES);
    for _ in 0..FRAMES {
        let n = enc.encode_float(&pcm, &mut out).unwrap();
        packets.push(out[..n].to_vec());
    }
    let enc_allocs = (ALLOCS.load(Ordering::Relaxed) - a0) - FRAMES; // subtract the packet copies
    let enc_bytes = BYTES.load(Ordering::Relaxed) - b0;

    let mut dec = Decoder::new(rate, channels).expect("decoder");
    let mut dec_out = vec![0i16; frame_len];
    for p in packets.iter().take(5) {
        let pkt = Packet::try_from(p.as_slice()).unwrap();
        dec.decode(Some(pkt), MutSignals::try_from(&mut dec_out[..]).unwrap(), false)
            .unwrap();
    }

    let a1 = ALLOCS.load(Ordering::Relaxed);
    let b1 = BYTES.load(Ordering::Relaxed);
    for p in &packets {
        let pkt = Packet::try_from(p.as_slice()).unwrap();
        dec.decode(Some(pkt), MutSignals::try_from(&mut dec_out[..]).unwrap(), false)
            .unwrap();
    }
    let dec_allocs = ALLOCS.load(Ordering::Relaxed) - a1;
    let dec_bytes = BYTES.load(Ordering::Relaxed) - b1;

    println!(
        "{name:<28} encode: {:6.2} allocs/frame ({:8.0} B/frame)   decode: {:6.2} allocs/frame ({:8.0} B/frame)",
        enc_allocs as f64 / FRAMES as f64,
        enc_bytes as f64 / FRAMES as f64,
        dec_allocs as f64 / FRAMES as f64,
        dec_bytes as f64 / FRAMES as f64,
    );
}

fn main() {
    measure("48k/stereo/audio/96k/20ms", SampleRate::Hz48000, Channels::Stereo, 960, Application::Audio, 96000);
    measure("48k/stereo/voip/32k/20ms", SampleRate::Hz48000, Channels::Stereo, 960, Application::Voip, 32000);
    measure("48k/stereo/ld/64k/2.5ms", SampleRate::Hz48000, Channels::Stereo, 120, Application::LowDelay, 64000);
    measure("16k/mono/voip/16k/20ms", SampleRate::Hz16000, Channels::Mono, 320, Application::Voip, 16000);
    measure("8k/mono/voip/12k/20ms", SampleRate::Hz8000, Channels::Mono, 160, Application::Voip, 12000);
}

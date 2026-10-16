//! Rust twin of `compat/tools/enctrace_cx.c`: acceptance configuration 0
//! through the public, `audiopus`-compatible API at complexity 6..10, so a
//! per-frame FNV diff against the C side shows whether the cfg-0 divergence
//! is gated on `run_analysis` (complexity >= 7).
//!
//!   cargo run --release --example enc_trace_cx > compat/out/enctrace_cx_rs.txt

use oprs::coder::Encoder;
use oprs::{Application, Bandwidth, Bitrate, Channels, SampleRate, Signal};

const FRAMES: usize = 50;
const MAX_PACKET: usize = 1500;
const FRAME_SAMPLES: usize = 960;

struct Fnv(u64);

impl Fnv {
    fn new() -> Fnv {
        Fnv(1469598103934665603)
    }
    fn upd(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(1099511628211);
        }
    }
}

struct Rng(u32);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1103515245).wrapping_add(12345);
        self.0
    }
    fn sample(&mut self) -> f32 {
        let v = ((self.next() >> 8) as i32) - 8388608;
        v as f32 / 8388608.0
    }
}

fn envelope(i: usize) -> f32 {
    0.25 + 0.75 * (i % 9973) as f32 / 9973.0
}

fn main() {
    for complexity in [6u8, 7, 8, 9, 10] {
        let mut enc = Encoder::new(SampleRate::Hz48000, Channels::Stereo, Application::Audio)
            .expect("create");
        enc.set_bitrate(Bitrate::BitsPerSecond(96000)).unwrap();
        enc.set_complexity(complexity).unwrap();
        enc.set_vbr(true).unwrap();
        enc.set_vbr_constraint(false).unwrap();
        enc.set_dtx(false).unwrap();
        enc.set_inband_fec(false).unwrap();
        enc.set_packet_loss_perc(0).unwrap();
        enc.set_bandwidth(Bandwidth::Auto).unwrap();
        enc.set_signal(Signal::Auto).unwrap();
        enc.set_lsb_depth(16).unwrap();

        let mut rng = Rng(0x1234567u32);
        let mut pcm = vec![0f32; FRAME_SAMPLES * 2];
        let mut total = Fnv::new();
        println!("cx {complexity}");
        for f in 0..FRAMES {
            for (i, s) in pcm.iter_mut().enumerate() {
                let idx = f * FRAME_SAMPLES * 2 + i;
                *s = rng.sample() * envelope(idx);
            }
            let mut out = vec![0u8; MAX_PACKET];
            let n = enc.encode_float(&pcm, &mut out).expect("encode");
            out.truncate(n);
            let mut h = Fnv::new();
            h.upd(&(n as i32).to_le_bytes());
            h.upd(&out);
            println!(
                "f {f:2} n={n:3} toc=0x{:02x} fnv=0x{:016x}",
                out[0], h.0
            );
            total.upd(&(n as i32).to_le_bytes());
            total.upd(&out);
        }
        println!("TOTAL enc=0x{:016x}", total.0);
    }
}

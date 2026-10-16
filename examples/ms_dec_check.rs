//! Independent check of the Rust multistream **decoder**: reads the packet trace
//! produced by `MS_TRACE=... compat/out/ms_ref2` (the C side of
//! `compat/tools/ms_oracle.c`, linked against `reference/lib-ref`) and decodes
//! every packet with the Rust port, printing the same FNV-1a of the decoded PCM
//! that the C tool prints as `dec_fnv`.
//!
//! This verifies the decoder before the Rust multistream *encoder* exists:
//!
//!   MS_TRACE=compat/out/ms_trace.bin compat/out/ms_ref2 > compat/out/ms_ref2.txt
//!   cargo build --release --example ms_dec_check
//!   ./target/release/examples/ms_dec_check compat/out/ms_trace.bin
//!   # cfg N dec_fnv=... must equal the dec_fnv values in compat/out/ms_ref2.txt
//!
//! Usage: `ms_dec_check [trace]` (default `compat/out/ms_trace.bin`).

use oprs::opus::multistream::OpusMSDecoder;

struct Rd {
    d: Vec<u8>,
    p: usize,
}

impl Rd {
    fn u32(&mut self) -> u32 {
        let b = &self.d[self.p..self.p + 4];
        self.p += 4;
        u32::from_le_bytes([b[0], b[1], b[2], b[3]])
    }
    fn bytes(&mut self, n: usize) -> &[u8] {
        let b = &self.d[self.p..self.p + n];
        self.p += n;
        b
    }
}

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

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "compat/out/ms_trace.bin".to_string());
    let mut r = Rd {
        d: std::fs::read(&path).expect("read the trace (set MS_TRACE and run compat/out/ms_ref2)"),
        p: 0,
    };
    assert_eq!(r.u32(), 0x4D53_544D, "bad trace magic");
    let fs = r.u32() as i32;
    let ncfg = r.u32();
    let mut total = Fnv::new();

    for c in 0..ncfg {
        let channels = r.u32() as i32;
        let streams = r.u32() as i32;
        let coupled = r.u32() as i32;
        let nframes = r.u32();
        let mut mapping = [0u8; 8];
        for m in mapping.iter_mut() {
            *m = r.u32() as u8;
        }
        let mapping = &mapping[..channels as usize];

        let mut err = 0;
        let mut dec = OpusMSDecoder::create(fs, channels, streams, coupled, mapping, &mut err)
            .unwrap_or_else(|| panic!("cfg {c}: create failed with {err}"));

        let mut h = Fnv::new();
        let mut samples = 0i64;
        for _ in 0..nframes {
            let len = r.u32() as usize;
            let data = r.bytes(len).to_vec();
            let mut pcm = vec![0i16; 960 * channels as usize];
            let n = dec.decode(Some(&data), len as i32, &mut pcm, 960, 0);
            assert!(n > 0, "cfg {c}: decode failed with {n}");
            h.upd(&n.to_le_bytes());
            for s in &pcm[..(n * channels) as usize] {
                h.upd(&s.to_le_bytes());
            }
            samples += n as i64;
        }
        println!("cfg {c} dec_fnv=0x{:016x} dec_samples={samples}", h.0);
        total.upd(&h.0.to_le_bytes());
    }
    println!("TOTAL dec=0x{:016x}", total.0);
}

//! Rust side of the kiss_fft / mdct bit-exactness probe.
//!
//! Mirrors `compat/c/probe_mdct.c` call for call and byte for byte.  Run
//! `bash compat/scripts/run-probe.sh mdct` to build both sides and diff their
//! output.

#![allow(non_snake_case)]

use oprs::celt::kiss_fft::*;
use oprs::celt::mdct::*;

struct ProbeCase {
    N: i32,
    overlap: i32,
    maxshift: i32,
}

const CASES: &[ProbeCase] = &[
    ProbeCase { N: 1920, overlap: 120, maxshift: 3 },
    ProbeCase { N: 960, overlap: 120, maxshift: 2 },
    ProbeCase { N: 480, overlap: 120, maxshift: 1 },
    ProbeCase { N: 240, overlap: 120, maxshift: 0 },
    ProbeCase { N: 120, overlap: 120, maxshift: 0 },
    ProbeCase { N: 3840, overlap: 480, maxshift: 3 },
    ProbeCase { N: 7680, overlap: 240, maxshift: 3 },
    ProbeCase { N: 3840, overlap: 240, maxshift: 2 },
];
const STRIDES: [i32; 3] = [1, 3, 8];
const NB: usize = 8;

/* state = state*1103515245 + 12345 */
struct Rng(u32);
impl Rng {
    fn reset(&mut self, s: u32) {
        self.0 = s;
    }
    fn rnd(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1103515245).wrapping_add(12345);
        self.0
    }
    fn sig(&mut self) -> f32 {
        (self.rnd() as i32) as f32 / 131072.0
    }
    fn unit(&mut self) -> f32 {
        (self.rnd() >> 8) as f32 / 16777216.0
    }
}

/* FNV-1a 64 over raw float bits. */
struct Fnv(u64);
impl Fnv {
    fn reset(&mut self) {
        self.0 = 0xcbf29ce484222325;
    }
    fn u32(&mut self, v: u32) {
        for i in 0..4 {
            self.0 ^= ((v >> (8 * i)) & 0xff) as u64;
            self.0 = self.0.wrapping_mul(1099511628211);
        }
    }
    fn f32(&mut self, v: f32) {
        self.u32(v.to_bits());
    }
}

fn print_array_info(tag: &str, x: &[f32]) {
    let len = x.len();
    let mut h = Fnv(0);
    h.reset();
    for &v in x {
        h.f32(v);
    }
    print!("{} len={} hash={:016x}", tag, len, h.0);
    for b in 0..NB {
        let lo = len * b / NB;
        let hi = len * (b + 1) / NB;
        h.reset();
        for &v in &x[lo..hi] {
            h.f32(v);
        }
        print!(" b{}={:016x}", b, h.0);
    }
    println!();
    print!("{} first", tag);
    for i in 0..len.min(4) {
        print!(" {:08x}", x[i].to_bits());
    }
    println!();
    print!("{} last", tag);
    for i in len.saturating_sub(4)..len {
        print!(" {:08x}", x[i].to_bits());
    }
    println!();
}

fn print_cpx_info(tag: &str, x: &[kiss_fft_cpx]) {
    let len = x.len();
    let mut h = Fnv(0);
    h.reset();
    for v in x {
        h.f32(v.r);
        h.f32(v.i);
    }
    print!("{} len={} hash={:016x}", tag, len, h.0);
    for b in 0..NB {
        let lo = len * b / NB;
        let hi = len * (b + 1) / NB;
        h.reset();
        for v in &x[lo..hi] {
            h.f32(v.r);
            h.f32(v.i);
        }
        print!(" b{}={:016x}", b, h.0);
    }
    println!();
    print!("{} first", tag);
    for i in 0..len.min(2) {
        print!(" {:08x},{:08x}", x[i].r.to_bits(), x[i].i.to_bits());
    }
    println!();
    print!("{} last", tag);
    for i in len.saturating_sub(2)..len {
        print!(" {:08x},{:08x}", x[i].r.to_bits(), x[i].i.to_bits());
    }
    println!();
}

fn main() {
    let mut rng = Rng(0);
    print!("probe_mdct: FIXED_POINT=0 CUSTOM_MODES=1 USE_SIMD=0\n");
    for (ci, c) in CASES.iter().enumerate() {
        let ci = ci as i32;
        let N = c.N;
        let ov = c.overlap;
        let ms = c.maxshift;
        let triglen = N - ((N >> 1) >> ms);
        let mut l = mdct_lookup::default();
        if clt_mdct_init(&mut l, N, ms, 0) == 0 {
            println!("case {} init FAILED", ci);
            continue;
        }
        println!(
            "case {} N={} overlap={} maxshift={} triglen={}",
            ci, N, ov, ms, triglen
        );

        let base_nfft = l.kfft[0].as_ref().unwrap().nfft;
        for i in 0..=ms {
            let st = l.kfft[i as usize].as_ref().unwrap();
            println!(
                "fft[{}] nfft={} shift={} scale={:08x}",
                i,
                st.nfft,
                st.shift,
                st.scale.to_bits()
            );
            print!("fft[{}] factors", i);
            for j in 0..MAXFACTORS {
                print!(" {}", st.factors[2 * j]);
                if st.factors[2 * j + 1] == 1 {
                    break;
                }
            }
            println!();
            let mut h = Fnv(0);
            h.reset();
            for j in 0..st.nfft {
                h.u32(st.bitrev[j as usize] as u16 as u32);
            }
            println!("fft[{}] bitrev_hash={:016x}", i, h.0);
            h.reset();
            for j in 0..base_nfft {
                h.f32(st.twiddles[j as usize].r);
                h.f32(st.twiddles[j as usize].i);
            }
            println!("fft[{}] twid_hash={:016x}", i, h.0);
            println!(
                "fft[{}] twid[0]={:08x},{:08x} twid[1]={:08x},{:08x} twid[n-1]={:08x},{:08x}",
                i,
                st.twiddles[0].r.to_bits(),
                st.twiddles[0].i.to_bits(),
                st.twiddles[1].r.to_bits(),
                st.twiddles[1].i.to_bits(),
                st.twiddles[(base_nfft - 1) as usize].r.to_bits(),
                st.twiddles[(base_nfft - 1) as usize].i.to_bits()
            );

            /* opus_fft_c() / opus_ifft_c() on deterministic complex input. */
            let n = st.nfft;
            let mut fin = vec![kiss_fft_cpx::default(); n as usize];
            let mut fout = vec![kiss_fft_cpx::default(); n as usize];
            for k in 0..2u32 {
                let seed = 0xF00D_0000u32 ^ ((ci as u32) << 9) ^ ((i as u32) << 6) ^ k;
                rng.reset(seed);
                for v in fin.iter_mut() {
                    v.r = rng.sig();
                    v.i = rng.sig();
                }
                for v in fout.iter_mut() {
                    *v = kiss_fft_cpx::default();
                }
                let tag = if k == 0 {
                    opus_fft_c(st, &fin, &mut fout);
                    format!("fft_c c{} i{}", ci, i)
                } else {
                    opus_ifft_c(st, &fin, &mut fout);
                    format!("ifft_c c{} i{}", ci, i)
                };
                print_cpx_info(&tag, &fout);
                if ci == 4 {
                    for (q, v) in fout.iter().enumerate() {
                        println!("full {} [{}] {:08x} {:08x}", tag, q, v.r.to_bits(), v.i.to_bits());
                    }
                }
            }
        }

        /* Generated MDCT twiddle table. */
        {
            let mut off = 0usize;
            let mut n2 = N >> 1;
            let mut h = Fnv(0);
            h.reset();
            for i in 0..triglen {
                h.f32(l.trig[i as usize]);
            }
            println!("trig hash={:016x}", h.0);
            for shift in 0..=ms {
                println!(
                    "trig[{}] len={} first={:08x},{:08x},{:08x},{:08x} last={:08x}",
                    shift,
                    n2,
                    l.trig[off].to_bits(),
                    l.trig[off + 1].to_bits(),
                    l.trig[off + 2].to_bits(),
                    l.trig[off + 3].to_bits(),
                    l.trig[off + n2 as usize - 1].to_bits()
                );
                off += n2 as usize;
                n2 >>= 1;
            }
        }

        let mut win = vec![0.0f32; ov as usize];
        rng.reset(0x7777_0000u32.wrapping_add(ci as u32));
        for w in win.iter_mut() {
            *w = rng.unit();
        }
        print_array_info("window", &win);

        for shift in 0..=ms {
            let ne = N >> shift;
            let n2 = ne >> 1;
            for dir in 0..2 {
                for &stride in STRIDES.iter() {
                    let seed = 0x5EED_0000u32
                        ^ ((ci as u32) << 9)
                        ^ ((shift as u32) << 6)
                        ^ ((stride as u32) << 2)
                        ^ (dir as u32);
                    if dir == 0 {
                        let inlen = (n2 + ov) as usize;
                        let outlen = (n2 * stride) as usize;
                        let mut input = vec![0.0f32; inlen];
                        let mut out = vec![0.0f32; outlen];
                        rng.reset(seed);
                        for v in input.iter_mut() {
                            *v = rng.sig();
                        }
                        clt_mdct_forward_c(&l, &input, &mut out, &win, ov, shift, stride, 0);
                        let tag = format!("fwd s{} st{}", shift, stride);
                        print_array_info(&tag, &out);
                        if ci == 4 {
                            for (i, v) in out.iter().enumerate() {
                                println!("full {} [{}] {:08x}", tag, i, v.to_bits());
                            }
                        }
                    } else {
                        let inlen = (n2 * stride) as usize;
                        let outlen = (N + ov) as usize;
                        let mut input = vec![0.0f32; inlen];
                        let mut out = vec![0.0f32; outlen];
                        rng.reset(seed);
                        for v in input.iter_mut() {
                            *v = rng.sig();
                        }
                        clt_mdct_backward_c(&l, &input, &mut out, &win, ov, shift, stride, 0);
                        let tag = format!("bwd s{} st{}", shift, stride);
                        print_array_info(&tag, &out[..(n2 + ov / 2) as usize]);
                        if ci == 4 {
                            for (i, v) in out[..(n2 + ov / 2) as usize].iter().enumerate() {
                                println!("full {} [{}] {:08x}", tag, i, v.to_bits());
                            }
                        }
                    }
                }
            }
        }

        if ci == 4 {
            let st = l.kfft[0].as_ref().unwrap();
            for i in 0..base_nfft {
                println!(
                    "full twid[{}] {:08x} {:08x}",
                    i,
                    st.twiddles[i as usize].r.to_bits(),
                    st.twiddles[i as usize].i.to_bits()
                );
            }
            for i in 0..triglen {
                println!("full trig[{}] {:08x}", i, l.trig[i as usize].to_bits());
            }
            for i in 0..st.nfft {
                println!("full bitrev[{}] {}", i, st.bitrev[i as usize]);
            }
        }

        clt_mdct_clear(&mut l, 0);
    }
}

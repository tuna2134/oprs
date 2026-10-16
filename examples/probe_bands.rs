//! Twin of `compat/c/probe_bands.c` — prints the identical output format so
//! that `bash compat/scripts/run-probe.sh bands` can diff the two.
//!
//! See `src/celt/bands.rs` for the port.  All randomness is the LCG
//! `st = st*1103515245 + 12345`; every float is printed as its raw IEEE-754
//! bits.

use oprs::celt::bands::trace;
use oprs::celt::bands::*;
use oprs::celt::entcode::celt_sudiv;
use oprs::celt::entdec::ec_dec_init;
use oprs::celt::entenc::{ec_enc_done, ec_enc_init};
use oprs::celt::modes::{opus_custom_mode_create, CELTMode};

// ---------------------------------------------------------------- helpers

struct Rng(u32);

impl Rng {
    fn new(s: u32) -> Rng {
        Rng(s)
    }
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1103515245).wrapping_add(12345);
        self.0
    }
    fn float(&mut self) -> f32 {
        ((self.next() >> 8) as i32 - 8388608) as f32 / 8388608.0
    }
}

/* Boundary values whose square stays finite (an inf/NaN spectrum is not a
   valid codec input and would make the x86 `(int)` cast in `stereo_itheta`
   differ from Rust's saturating `as i32`). */
const SPECIALS: [u32; 10] = [
    0x0000_0000, 0x8000_0000, 0x3f80_0000, 0xbf80_0000, 0x0000_0001, 0x8000_0001, 0x0080_0000,
    0x8080_0000, 0x4049_0fdb, 0xc049_0fdb,
];

fn fill_f32(x: &mut [f32], seed: u32, mode: i32) {
    let mut r = Rng::new(seed);
    for i in 0..x.len() {
        x[i] = if mode == 0 {
            r.float()
        } else if mode == 1 && (i % 5) == 3 {
            f32::from_bits(SPECIALS[(i / 5 + seed as usize) % 10])
        } else if mode == 2 {
            0.0
        } else {
            r.float() * (1 + (i % 7)) as f32
        };
    }
}

fn fb(v: f32) -> u32 {
    v.to_bits()
}

fn fnv_bytes(p: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in p {
        h ^= b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

fn fnv_f32(p: &[f32]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &f in p {
        for b in f.to_bits().to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100_0000_01b3);
        }
    }
    h
}

fn fnv_i16(p: &[i16]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &v in p {
        for b in v.to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100_0000_01b3);
        }
    }
    h
}

fn print_f32s(name: &str, p: &[f32]) {
    for (i, &v) in p.iter().enumerate() {
        println!("PB {} {} {:08x}", name, i, fb(v));
    }
}

fn imin(a: i32, b: i32) -> i32 {
    if a < b {
        a
    } else {
        b
    }
}
fn imax(a: i32, b: i32) -> i32 {
    if a > b {
        a
    } else {
        b
    }
}

// --------------------------------------------------- pure-helper sections

fn sec_hadamard() {
    for si in 0..4 {
        let stride = 2 << si;
        for had in 0..2 {
            let n0 = 32i32;
            let n = (n0 * stride) as usize;
            let mut x = vec![0.0f32; n];
            fill_f32(&mut x, 0x100 + (si * 10 + had) as u32, 0);
            println!("== ilv stride={} had={} N0={} ==", stride, had, n0);
            println!("H in {:016x}", fnv_f32(&x));
            interleave_hadamard(&mut x, n0, stride, had);
            println!("H ilv {:016x}", fnv_f32(&x));
            print_f32s("ilv", &x);
            deinterleave_hadamard(&mut x, n0, stride, had);
            println!("H deilv {:016x}", fnv_f32(&x));
            for (k, &v) in x.iter().enumerate() {
                println!("PB back {} {:08x}", k, fb(v));
            }
        }
    }
}

fn sec_haar() {
    const CASES: [[i32; 2]; 5] = [[8, 1], [8, 2], [4, 4], [16, 1], [2, 8]];
    for (c, cs) in CASES.iter().enumerate() {
        let (n0, stride) = (cs[0], cs[1]);
        let n = (n0 * stride) as usize;
        let mut x = vec![0.0f32; n];
        fill_f32(&mut x, 0x200 + c as u32, 0);
        println!("== haar N0={} stride={} ==", n0, stride);
        println!("H in {:016x}", fnv_f32(&x));
        haar1(&mut x, n0, stride);
        println!("H out {:016x}", fnv_f32(&x));
        print_f32s("haar", &x);
    }
}

fn sec_energies(m: &CELTMode) {
    const ENDS: [i32; 2] = [8, 21];
    for lm in 0..=3i32 {
        let mm = 1 << lm;
        let n = mm * m.shortMdctSize;
        for c in 1..=2i32 {
            for (ei, &end) in ENDS.iter().enumerate() {
                let nb = m.nbEBands;
                let mut x = vec![0.0f32; (c * n) as usize];
                fill_f32(
                    &mut x,
                    0x300 + (lm * 13 + ei as i32 * 5 + c) as u32,
                    if (lm & 1) != 0 { 1 } else { 3 },
                );
                if lm < 2 {
                    for k in 0..(n / 4) as usize {
                        x[k] = 0.0;
                    }
                }
                let mut band_e = vec![0.0f32; (2 * 21) as usize];
                /* `compute_band_energies` only writes `end` bands per channel;
                   pre-fill so the untouched entries are defined. */
                for v in band_e.iter_mut().take((c * nb) as usize) {
                    *v = -1.0;
                }
                compute_band_energies(m, &x, &mut band_e, end, c, lm, 4);
                println!("== energies lm={} C={} end={} ==", lm, c, end);
                println!("H in {:016x}", fnv_f32(&x));
                for i in 0..(c * nb) as usize {
                    println!("PB bandE {} {:08x}", i, fb(band_e[i]));
                }
                /* `normalise_bands` and `denormalise_bands` leave parts of each
                   buffer untouched; pre-fill so both sides see the same bytes.
                   `denormalise_bands` is per channel in the codec. */
                let mut xn = vec![0.0f32; (c * n) as usize];
                fill_f32(
                    &mut xn,
                    0x350 + (lm * 13 + ei as i32 * 5 + c) as u32,
                    0,
                );
                normalise_bands(m, &x, &mut xn, &band_e, end, c, mm);
                println!("H norm {:016x}", fnv_f32(&xn));
                if c * n <= 480 {
                    print_f32s("norm", &xn);
                }
                let mut log_e = vec![0.0f32; (c * nb) as usize];
                fill_f32(&mut log_e, 0x400 + (lm + c) as u32, 0);
                for v in log_e.iter_mut() {
                    *v = *v * 8.0 - 4.0;
                }
                let mut freq = vec![0.0f32; (c * n) as usize];
                fill_f32(
                    &mut freq,
                    0x410 + (lm * 13 + ei as i32 * 5 + c) as u32,
                    0,
                );
                for i in 0..c {
                    let o = (i * n) as usize;
                    let lo = (i * nb) as usize;
                    denormalise_bands(
                        m,
                        &xn[o..o + n as usize],
                        &mut freq[o..o + n as usize],
                        &log_e[lo..lo + nb as usize],
                        0,
                        end,
                        mm,
                        1,
                        0,
                    );
                }
                println!("H de {:016x}", fnv_f32(&freq));
                if c * n <= 480 {
                    print_f32s("de", &freq);
                }
                fill_f32(
                    &mut freq,
                    0x420 + (lm * 13 + ei as i32 * 5 + c) as u32,
                    0,
                );
                for i in 0..c {
                    let o = (i * n) as usize;
                    let lo = (i * nb) as usize;
                    denormalise_bands(
                        m,
                        &xn[o..o + n as usize],
                        &mut freq[o..o + n as usize],
                        &log_e[lo..lo + nb as usize],
                        0,
                        end,
                        mm,
                        2,
                        0,
                    );
                }
                println!("H de2 {:016x}", fnv_f32(&freq));
                let bytes: Vec<u8> = freq.iter().flat_map(|v| v.to_bits().to_le_bytes()).collect();
                println!("H de2b {:016x}", fnv_bytes(&bytes));
                fill_f32(
                    &mut freq,
                    0x430 + (lm * 13 + ei as i32 * 5 + c) as u32,
                    0,
                );
                for i in 0..c {
                    let o = (i * n) as usize;
                    let lo = (i * nb) as usize;
                    denormalise_bands(
                        m,
                        &xn[o..o + n as usize],
                        &mut freq[o..o + n as usize],
                        &log_e[lo..lo + nb as usize],
                        0,
                        end,
                        mm,
                        1,
                        1,
                    );
                }
                println!("H desil {:016x}", fnv_f32(&freq));
                let bytes: Vec<u8> = freq.iter().flat_map(|v| v.to_bits().to_le_bytes()).collect();
                println!("H desilz {:016x}", fnv_bytes(&bytes));
            }
        }
    }
}

fn sec_stereo_helpers(m: &CELTMode) {
    const NS: [i32; 4] = [2, 4, 16, 64];
    for (t, &n) in NS.iter().enumerate() {
        let mut x = vec![0.0f32; n as usize];
        let mut y = vec![0.0f32; n as usize];
        fill_f32(&mut x, 0x500 + t as u32, 0);
        fill_f32(&mut y, 0x600 + t as u32, 3);
        println!("== split/merge N={} ==", n);
        println!("H inx {:016x}", fnv_f32(&x));
        println!("H iny {:016x}", fnv_f32(&y));
        stereo_split(&mut x, &mut y, n);
        print_f32s("splitx", &x);
        print_f32s("splity", &y);
        let mut r = Rng::new(0x700 + t as u32);
        let mid = r.float() * 0.5 + 0.5;
        println!("PB mid 0 {:08x}", fb(mid));
        stereo_merge(&mut x, &mut y, mid, n, 4);
        print_f32s("mergex", &x);
        print_f32s("mergey", &y);
    }
    {
        let n = 64i32;
        let mut x = vec![0.0f32; n as usize];
        let mut y = vec![0.0f32; n as usize];
        let mut band_e = vec![0.0f32; (2 * 21) as usize];
        fill_f32(&mut x, 0x800, 0);
        fill_f32(&mut y, 0x810, 3);
        for v in band_e.iter_mut() {
            *v = 0.0;
        }
        band_e[5] = 0.0;
        band_e[5 + 21] = 0.0;
        band_e[6] = 1.5;
        band_e[6 + 21] = 0.25;
        band_e[7] = 1e-30;
        band_e[7 + 21] = 1e30;
        println!("== intensity_stereo ==");
        intensity_stereo(m, &mut x, &y, &band_e, 5, n);
        print_f32s("isx5", &x);
        intensity_stereo(m, &mut x, &y, &band_e, 6, n);
        print_f32s("isx6", &x);
        let mut r = Rng::new(0x820);
        for v in x.iter_mut() {
            *v = r.float();
        }
        intensity_stereo(m, &mut x, &y, &band_e, 7, n);
        print_f32s("isx7", &x);
    }
}

/// A deterministic `seed` produced by a real range decoder, so that
/// `anti_collapse` is driven by the entropy coder as well.
fn real_dec_seed() -> u32 {
    use oprs::celt::entdec::{ec_dec_bits, ec_dec_uint};
    use oprs::celt::entenc::{ec_enc_bits, ec_enc_uint};
    let mut e = ec_enc_init(32);
    ec_enc_bits(&mut e, 0xA5A5, 16);
    ec_enc_bits(&mut e, 0x1234, 16);
    ec_enc_uint(&mut e, 12345, 65536);
    ec_enc_done(&mut e);
    let mut d = ec_dec_init(&e.buf);
    let _ = ec_dec_bits(&mut d, 16);
    let _ = ec_dec_bits(&mut d, 16);
    let s = ec_dec_bits(&mut d, 16);
    (s << 16) | ec_dec_uint(&mut d, 65536)
}

fn sec_anti_collapse(m: &CELTMode) {
    for c in 1..=2i32 {
        for enc in 0..2i32 {
            let lm = 2i32;
            let mm = 4i32;
            let size = mm * m.shortMdctSize;
            let (start, end) = (0i32, 8i32);
            let nb = m.nbEBands;
            let mut x = vec![0.0f32; (c * size) as usize];
            /* the arrays are `2*nbEBands` in the codec (celt.h); the decoder
               reads `prev*logE[nbEBands+i]` when `C==1` */
            let mut log_e = vec![0.0f32; (2 * nb) as usize];
            let mut prev1 = vec![0.0f32; (2 * nb) as usize];
            let mut prev2 = vec![0.0f32; (2 * nb) as usize];
            let mut masks = vec![0u8; (2 * nb) as usize];
            let mut pulses = vec![0i32; 21];
            fill_f32(&mut x, 0x900 + (c * 2 + enc) as u32, 0);
            fill_f32(&mut log_e, 0x910 + (c * 2 + enc) as u32, 0);
            fill_f32(&mut prev1, 0x920 + (c * 2 + enc) as u32, 0);
            fill_f32(&mut prev2, 0x930 + (c * 2 + enc) as u32, 0);
            for i in 0..(c * nb) as usize {
                log_e[i] = log_e[i] * 8.0 - 2.0;
                prev1[i] = prev1[i] * 8.0 - 4.0;
                prev2[i] = prev2[i] * 8.0 - 4.0;
            }
            for i in 0..(2 * nb) as usize {
                masks[i] = if i % 3 == 0 { 0 } else { (i & 0xFF) as u8 };
            }
            for (i, p) in pulses.iter_mut().enumerate() {
                *p = ((i as i32) * 37) % 200;
            }
            println!("== anti_collapse C={} enc={} ==", c, enc);
            println!("H in {:016x}", fnv_f32(&x));
            anti_collapse(
                m,
                &mut x,
                &masks,
                lm,
                c,
                size,
                start,
                end,
                &log_e,
                &prev1,
                &prev2,
                &pulses,
                real_dec_seed(),
                enc,
                4,
            );
            println!("H out {:016x}", fnv_f32(&x));
            for cc in 0..c {
                let off = (cc * size) as usize;
                println!(
                    "H outc{} {:016x}",
                    cc,
                    fnv_f32(&x[off..off + size as usize])
                );
            }
            print_f32s("ac", &x);
        }
    }
}

fn sec_hysteresis() {
    const THR: [f32; 5] = [0.3, 0.6, 0.9, 1.2, 1.5];
    const HYS: [f32; 5] = [0.05, 0.05, 0.05, 0.05, 0.05];
    const VALS: [f32; 9] = [0.0, 0.29, 0.31, 0.58, 0.62, 0.94, 0.96, 1.49, 2.0];
    println!("== hysteresis ==");
    for (i, &v) in VALS.iter().enumerate() {
        for p in 0..5 {
            println!(
                "HYS {} {} {}",
                i,
                p,
                hysteresis_decision(v, &THR, &HYS, 5, p)
            );
        }
    }
}

fn sec_spreading(m: &CELTMode) {
    for lm in 0..=2i32 {
        let mm = 1 << lm;
        let n = mm * m.shortMdctSize;
        for c in 1..=2i32 {
            let mut x = vec![0.0f32; (c * n) as usize];
            let mut w = vec![0i32; 21];
            fill_f32(&mut x, 0xA00 + (lm * 3 + c) as u32, 3);
            for i in 0..m.nbEBands as usize {
                w[i] = (i as i32 % 4) + 1;
            }
            let mut average = 0i32;
            let mut hf_average = 0i32;
            let mut tapset = 0i32;
            println!("== spread lm={} C={} ==", lm, c);
            for t in 0..4 {
                let d = spreading_decision(
                    m,
                    &x,
                    &mut average,
                    t,
                    &mut hf_average,
                    &mut tapset,
                    if t != 0 { 1 } else { 0 },
                    m.nbEBands,
                    c,
                    mm,
                    &w,
                );
                println!("SPR {} {} {} {} {}", t, d, average, hf_average, tapset);
            }
        }
    }
}

// ---------------------------------------------------- range-coded section

#[derive(Clone, Copy)]
struct Rcfg {
    start: i32,
    end: i32,
    lm: i32,
    c: i32,
    short_blocks: i32,
    spread: i32,
    dual: i32,
    intensity: i32,
    tfpat: i32,
    total_bits: i32,
    balance: i32,
    coded_bands: i32,
    complexity: i32,
    disable_inv: i32,
}

const RCFG: [Rcfg; 15] = [
    Rcfg { start: 0, end: 21, lm: 0, c: 1, short_blocks: 0, spread: 2, dual: 0, intensity: 21, tfpat: 0, total_bits: 4000, balance: 0, coded_bands: 21, complexity: 0, disable_inv: 0 },
    Rcfg { start: 0, end: 21, lm: 0, c: 2, short_blocks: 0, spread: 2, dual: 0, intensity: 10, tfpat: 0, total_bits: 8000, balance: 0, coded_bands: 21, complexity: 0, disable_inv: 0 },
    Rcfg { start: 0, end: 21, lm: 1, c: 1, short_blocks: 0, spread: 1, dual: 0, intensity: 21, tfpat: 1, total_bits: 6000, balance: 0, coded_bands: 21, complexity: 0, disable_inv: 0 },
    Rcfg { start: 0, end: 21, lm: 1, c: 2, short_blocks: 0, spread: 3, dual: 0, intensity: 12, tfpat: 1, total_bits: 12000, balance: 0, coded_bands: 21, complexity: 0, disable_inv: 0 },
    Rcfg { start: 0, end: 17, lm: 2, c: 1, short_blocks: 1, spread: 0, dual: 0, intensity: 17, tfpat: 2, total_bits: 9000, balance: 0, coded_bands: 17, complexity: 0, disable_inv: 0 },
    Rcfg { start: 0, end: 17, lm: 2, c: 2, short_blocks: 1, spread: 2, dual: 0, intensity: 8, tfpat: 4, total_bits: 16000, balance: 0, coded_bands: 17, complexity: 0, disable_inv: 0 },
    Rcfg { start: 0, end: 21, lm: 3, c: 1, short_blocks: 1, spread: 3, dual: 0, intensity: 21, tfpat: 3, total_bits: 14000, balance: 0, coded_bands: 21, complexity: 0, disable_inv: 0 },
    Rcfg { start: 0, end: 21, lm: 3, c: 2, short_blocks: 1, spread: 1, dual: 1, intensity: 14, tfpat: 3, total_bits: 22000, balance: 0, coded_bands: 21, complexity: 0, disable_inv: 0 },
    Rcfg { start: 0, end: 21, lm: 2, c: 1, short_blocks: 0, spread: 2, dual: 0, intensity: 21, tfpat: 4, total_bits: 3000, balance: 0, coded_bands: 12, complexity: 0, disable_inv: 0 },
    Rcfg { start: 0, end: 12, lm: 1, c: 2, short_blocks: 0, spread: 0, dual: 0, intensity: 6, tfpat: 4, total_bits: 11000, balance: 50, coded_bands: 12, complexity: 0, disable_inv: 1 },
    Rcfg { start: 4, end: 21, lm: 3, c: 2, short_blocks: 1, spread: 2, dual: 0, intensity: 16, tfpat: 5, total_bits: 20000, balance: 0, coded_bands: 21, complexity: 10, disable_inv: 0 },
    Rcfg { start: 0, end: 21, lm: 0, c: 2, short_blocks: 0, spread: 3, dual: 1, intensity: 21, tfpat: 0, total_bits: 6000, balance: 0, coded_bands: 21, complexity: 10, disable_inv: 0 },
    Rcfg { start: 0, end: 8, lm: 0, c: 2, short_blocks: 1, spread: 0, dual: 1, intensity: 4, tfpat: 2, total_bits: 3000, balance: 0, coded_bands: 8, complexity: 0, disable_inv: 0 },
    Rcfg { start: 0, end: 21, lm: 3, c: 2, short_blocks: 1, spread: 2, dual: 0, intensity: 16, tfpat: 3, total_bits: 22000, balance: 500, coded_bands: 18, complexity: 0, disable_inv: 0 },
    // hybrid-style start != 0: exercises `special_hybrid_folding`'s copy
    Rcfg { start: 17, end: 21, lm: 3, c: 2, short_blocks: 1, spread: 0, dual: 1, intensity: 21, tfpat: 0, total_bits: 15000, balance: 0, coded_bands: 21, complexity: 0, disable_inv: 0 },
];

const RC_SIZE: usize = 1275;

fn tf_of(cf: &Rcfg, i: i32) -> i32 {
    let k = i - cf.start;
    if cf.short_blocks == 0 {
        return 0;
    }
    match cf.tfpat {
        0 => 0,
        1 => {
            if (k % 2) != 0 {
                1
            } else {
                0
            }
        }
        2 => k % (cf.lm + 1),
        3 => {
            if (k % 2) == 0 {
                cf.lm
            } else {
                0
            }
        }
        4 => -(k % 2),
        _ => {
            let mut v = (k % 5) - 2;
            if v > cf.lm {
                v = cf.lm;
            }
            v
        }
    }
}

fn print_trace(evs: &[trace::Event], cf: &Rcfg, pulses: &[i32]) {
    let nev = evs.len();
    println!("TR n={}", nev);
    for (k, e) in evs.iter().enumerate() {
        println!("TR {} {} {} {} {}", k, e.0, e.1, e.2, e.3);
    }
    let mut bi = cf.start;
    let mut balance = cf.balance;
    for e in evs.iter() {
        if e.0 != 1706 {
            continue;
        }
        let tell = e.1 as i32;
        if bi != cf.start {
            balance -= tell;
        }
        let remaining = cf.total_bits - tell - 1;
        let b;
        if bi <= cf.coded_bands - 1 {
            let cb = celt_sudiv(balance, imin(3, cf.coded_bands - bi));
            b = imax(0, imin(16383, imin(remaining + 1, pulses[bi as usize] + cb)));
        } else {
            b = 0;
        }
        println!("BAND {} {} {} {} {}", bi, tell, remaining, balance, b);
        balance += pulses[bi as usize] + tell;
        bi += 1;
    }
}

fn run_rc(m: &CELTMode, idx: usize) {
    let cf = RCFG[idx];
    let mm = 1 << cf.lm;
    let n = mm * m.shortMdctSize;
    let c = cf.c;
    let nb = m.nbEBands;
    let mut pulses = vec![0i32; 21];
    let mut tf_res = vec![0i32; 21];
    let mut masks = vec![0u8; (2 * 21) as usize];
    let mut band_e = vec![0.0f32; (2 * 21) as usize];
    let mut r = Rng::new(0xB00 + idx as u32);
    for p in pulses.iter_mut() {
        *p = (r.next() % 256) as i32;
    }
    for i in cf.start..cf.end {
        tf_res[i as usize] = tf_of(&cf, i);
    }
    for v in band_e.iter_mut() {
        *v = 0.001 + (r.next() % 1000) as f32 / 100.0;
    }
    let mut xbuf = vec![0.0f32; (2 * 960) as usize];
    fill_f32(
        &mut xbuf[..(c * n) as usize],
        0xB10 + idx as u32,
        if idx % 2 != 0 { 1 } else { 0 },
    );
    if idx % 3 == 0 {
        for k in 0..n as usize {
            xbuf[k] = 0.0;
        }
    }

    println!(
        "== rc idx={} start={} end={} LM={} C={} B={} spread={} dual={} intensity={} total={} balance={} codedBands={} complexity={} disable_inv={} ==",
        idx, cf.start, cf.end, cf.lm, cf.c, cf.short_blocks, cf.spread, cf.dual,
        cf.intensity, cf.total_bits, cf.balance, cf.coded_bands, cf.complexity, cf.disable_inv
    );
    for (i, p) in pulses.iter().enumerate().take(nb as usize) {
        println!("IN pulses {} {}", i, p);
    }
    for i in 0..cf.end {
        println!("IN tf {} {}", i, tf_res[i as usize]);
    }
    println!("H inx {:016x}", fnv_f32(&xbuf[..(c * n) as usize]));

    let mut enc = ec_enc_init(RC_SIZE as u32);
    let mut seed: u32 = 0x5eed_1234;
    trace::set(true);
    {
        let (xa, ya): (&mut [f32], Option<&mut [f32]>) = if c == 2 {
            let (a, b) = xbuf.split_at_mut(n as usize);
            (a, Some(b))
        } else {
            (&mut xbuf[..n as usize], None)
        };
        quant_all_bands(
            1,
            m,
            cf.start,
            cf.end,
            xa,
            ya,
            &mut masks,
            &band_e,
            &pulses,
            cf.short_blocks,
            cf.spread,
            cf.dual,
            cf.intensity,
            &tf_res,
            cf.total_bits,
            cf.balance,
            &mut enc,
            cf.lm,
            cf.coded_bands,
            &mut seed,
            cf.complexity,
            4,
            cf.disable_inv,
        );
    }
    let evs = trace::take();
    println!("H encbuf {:016x}", fnv_bytes(&enc.buf));
    println!(
        "ENC offs={} nbits={} rng={:08x} val={:08x} rem={} ext={} endoffs={}",
        enc.offs, enc.nbits_total, enc.rng, enc.val, enc.rem, enc.ext, enc.end_offs
    );
    println!("ENC seed={:08x}", seed);
    print_trace(&evs, &cf, &pulses);
    ec_enc_done(&mut enc);
    println!("H encdone {:016x}", fnv_bytes(&enc.buf));
    println!(
        "ENC2 offs={} nbits={} rng={:08x} val={:08x} rem={} ext={} endoffs={}",
        enc.offs, enc.nbits_total, enc.rng, enc.val, enc.rem, enc.ext, enc.end_offs
    );

    // ---- decode the same bitstream ----
    for v in masks.iter_mut() {
        *v = 0;
    }
    for v in xbuf.iter_mut() {
        *v = 0.0;
    }
    let mut dec = ec_dec_init(&enc.buf);
    let mut seed2: u32 = 0x5eed_1234;
    trace::set(true);
    {
        let (xa, ya): (&mut [f32], Option<&mut [f32]>) = if c == 2 {
            let (a, b) = xbuf.split_at_mut(n as usize);
            (a, Some(b))
        } else {
            (&mut xbuf[..n as usize], None)
        };
        quant_all_bands(
            0,
            m,
            cf.start,
            cf.end,
            xa,
            ya,
            &mut masks,
            &band_e,
            &pulses,
            cf.short_blocks,
            cf.spread,
            cf.dual,
            cf.intensity,
            &tf_res,
            cf.total_bits,
            cf.balance,
            &mut dec,
            cf.lm,
            cf.coded_bands,
            &mut seed2,
            0,
            4,
            cf.disable_inv,
        );
    }
    let evs2 = trace::take();
    /* `ext` is deliberately not printed: C's `ec_dec_init()` leaves it
       uninitialised until the first `ec_decode*()` (docs/progress.md records
       this as an accepted divergence — the Rust port zeroes it). */
    println!(
        "DEC offs={} nbits={} rng={:08x} val={:08x} rem={} endoffs={}",
        dec.offs, dec.nbits_total, dec.rng, dec.val, dec.rem, dec.end_offs
    );
    println!("DEC seed={:08x}", seed2);
    for i in 0..(c * nb) as usize {
        println!("OUT mask {} {}", i, masks[i]);
    }
    for i in 0..(c * n) as usize {
        println!("PB out {} {:08x}", i, fb(xbuf[i]));
    }
    print_trace(&evs2, &cf, &pulses);
}

fn main() {
    let mut err = 0;
    let m = match opus_custom_mode_create(48000, 960, &mut err) {
        Some(m) => m,
        None => {
            println!("mode_create failed err={}", err);
            return;
        }
    };
    println!("== mode ==");
    println!(
        "MODE err={} Fs={} nbEBands={} effEBands={} shortMdctSize={} overlap={} maxLM={} nbShortMdcts={} nbAllocVectors={}",
        err, m.Fs, m.nbEBands, m.effEBands, m.shortMdctSize, m.overlap, m.maxLM,
        m.nbShortMdcts, m.nbAllocVectors
    );
    for i in 0..=m.nbEBands as usize {
        println!("EB {} {}", i, m.eBands[i]);
    }
    for i in 0..m.nbEBands as usize {
        println!("LN {} {}", i, m.logN[i]);
    }
    for i in 0..4 {
        println!("PB preemph {} {:08x}", i, fb(m.preemph[i]));
    }
    let nidx = ((m.maxLM + 2) * m.nbEBands) as usize;
    println!("H cacheidx {:016x}", fnv_i16(&m.cache.index[..nidx]));
    println!(
        "H cachebits {:016x}",
        fnv_bytes(&m.cache.bits[..m.cache.size as usize])
    );

    sec_hadamard();
    sec_haar();
    sec_energies(&m);
    sec_stereo_helpers(&m);
    sec_anti_collapse(&m);
    sec_hysteresis();
    sec_spreading(&m);
    for i in 0..RCFG.len() {
        run_rc(&m, i);
    }
}

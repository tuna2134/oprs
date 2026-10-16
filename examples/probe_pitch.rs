//! Twin of `compat/c/probe_pitch.c` — prints the identical output format so
//! that `bash compat/scripts/run-probe.sh pitch` can diff the two.
//!
//! See `src/celt/pitch.rs` and `src/celt/celt_lpc.rs` for the ported code and
//! `docs/rtcd-kernels.md` for the arch-4 SIMD kernels.
//!
//! Every output value is compared bit-for-bit except the *sign* of a NaN,
//! which is cleared before hashing (`canon()` below): gcc and clang/LLVM
//! propagate the sign of a NaN differently through `a + (-b)*c` sequences.
//! NaN payloads, infinities, signed zeros and every finite value are compared
//! exactly, and audio-like input never produces a NaN.

use oprs::celt::celt_lpc::{_celt_autocorr, _celt_lpc, celt_fir, celt_iir};
use oprs::celt::kernels;
use oprs::celt::pitch::{celt_fir5, celt_pitch_xcorr_c, find_best_pitch, pitch_downsample, pitch_search, remove_doubling};

/* ------------------------------------------------------------------ */
/* Deterministic inputs                                                */
/* ------------------------------------------------------------------ */

const SPECIALS: [u32; 14] = [
    0x0000_0000, 0x8000_0000, 0x3f80_0000, 0xbf80_0000, 0x0000_0001, 0x8000_0001, 0x007f_ffff,
    0x0080_0000, 0x7f7f_ffff, 0xff7f_ffff, 0x3f00_0000, 0xbf00_0000, 0x7f80_0000, 0xff80_0000,
];

/// mode 0: LCG values in [-scale, scale]; mode 1: boundary patterns from
/// `SPECIALS`; mode 2: all +0.0.  Mirrors the C `fill()` exactly.
fn fill(b: &mut [f32], scale: f32, seed: u32, mode: i32) {
    let mut st = seed.wrapping_mul(2654435761).wrapping_add(1);
    for i in 0..b.len() {
        if mode == 2 {
            b[i] = 0.0;
        } else if mode == 1 || ((i as i32 + seed as i32) % 7) == 3 {
            b[i] = f32::from_bits(SPECIALS[(((i / 7) as i32 + seed as i32) % 14) as usize]);
        } else {
            st = st.wrapping_mul(1103515245).wrapping_add(12345);
            let v = ((st >> 8) as i32) - 8388608;
            b[i] = (v as f32) / 8388608.0 * scale;
        }
    }
}

/* ------------------------------------------------------------------ */
/* FNV-1a 64 over raw IEEE-754 bits                                    */
/* ------------------------------------------------------------------ */

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

fn fnv_u32(h: &mut u64, w: u32) {
    for i in 0..4 {
        *h ^= ((w >> (8 * i)) & 0xff) as u64;
        *h = h.wrapping_mul(0x100_0000_01b3);
    }
}

/// NaN canonicalisation: gcc (the reference build) and clang/LLVM propagate
/// the *sign* of a NaN differently through `a + (-b)*c` sequences (gcc keeps
/// the `xorps` negation, LLVM fuses it into a subtraction).  The sign of a NaN
/// is unspecified in IEEE-754 and never occurs for audio input, so the probe
/// compares NaNs by a canonical pattern and everything else bit-exactly.
fn canon(f: f32) -> u32 {
    if f.is_nan() { f.to_bits() & 0x7fff_ffff } else { f.to_bits() }
}

fn fnv_f32_cont(mut h: u64, p: &[f32]) -> u64 {
    for &f in p {
        fnv_u32(&mut h, canon(f));
    }
    h
}

fn fnv_f32_hash(p: &[f32]) -> u64 {
    fnv_f32_cont(FNV_OFFSET, p)
}

fn fnv_i32_hash(p: &[i32]) -> u64 {
    let mut h = FNV_OFFSET;
    for &v in p {
        fnv_u32(&mut h, v as u32);
    }
    h
}

fn mix2(a: u64, b: u64) -> u64 {
    a ^ b.wrapping_mul(0x9e37_79b9_7f4a_7c15)
}

/* ------------------------------------------------------------------ */
/* Reporting                                                           */
/* ------------------------------------------------------------------ */

fn report(fname: &str, par: &str, input: u64, out: u64) {
    println!("{:<11} {:<38} in={:016x} out={:016x}", fname, par, input, out);
}

fn report_i1(fname: &str, par: &str, input: u64, out: u64, i0: i32) {
    println!(
        "{:<11} {:<38} in={:016x} out={:016x} i0={}",
        fname, par, input, out, i0
    );
}

fn report_i2(fname: &str, par: &str, input: u64, out: u64, i0: i32, i1: i32) {
    println!(
        "{:<11} {:<38} in={:016x} out={:016x} i0={} i1={}",
        fname, par, input, out, i0, i1
    );
}

/* ------------------------------------------------------------------ */
/* Sections                                                            */
/* ------------------------------------------------------------------ */

/// A: `celt_pitch_xcorr` (dispatched, arch 4 -> AVX2) and `celt_pitch_xcorr_c`.
fn sec_xcorr() {
    const LENS: [usize; 24] = [
        1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 15, 16, 17, 23, 24, 25, 31, 32, 33, 63, 64,
    ];
    const MPS: [i32; 12] = [1, 2, 3, 4, 5, 7, 8, 9, 16, 17, 32, 33];
    let mut x = vec![0f32; 128];
    let mut y = vec![0f32; 320];
    let mut xc = vec![0f32; 64];
    let mut xc2 = vec![0f32; 64];
    for &len in LENS.iter() {
        for &mp in MPS.iter() {
            fill(&mut x, 1.0, 1000 + len as u32, 0);
            fill(&mut y, 0.75, 2000 + mp as u32, 0);
            let input = mix2(fnv_f32_hash(&x), fnv_f32_hash(&y));
            xc.fill(0.0);
            xc2.fill(0.0);
            kernels::celt_pitch_xcorr(&x, &y, &mut xc, len as i32, mp);
            let par = format!("len={} mp={} arch=4", len, mp);
            report("pxcorr", &par, input, fnv_f32_hash(&xc[..mp as usize]));
            celt_pitch_xcorr_c(&x, &y, &mut xc2, len as i32, mp, 4);
            report("pxcorr_c", &par, input, fnv_f32_hash(&xc2[..mp as usize]));
        }
    }
}

/// B: `find_best_pitch`.
fn sec_find_best() {
    const FB_LEN: [usize; 8] = [1, 2, 3, 4, 5, 8, 16, 31];
    const FB_MP: [i32; 8] = [1, 2, 3, 4, 5, 8, 16, 33];
    let mut xcorr = vec![0f32; 64];
    let mut y = vec![0f32; 160];
    for t in 0..8usize {
        let len = FB_LEN[t];
        let mp = FB_MP[t];
        let mut bp = [0i32; 2];
        fill(&mut xcorr, 3.0, 3000 + t as u32, 0);
        fill(&mut y, 1.0, 3100 + t as u32, 0);
        if t % 2 == 0 {
            for i in 0..mp as usize {
                if i % 3 == 0 {
                    xcorr[i] = -0.5;
                }
            }
        }
        xcorr[(t * 5) % mp as usize] = 1000.0 + t as f32;
        let input = mix2(fnv_f32_hash(&xcorr), fnv_f32_hash(&y));
        find_best_pitch(&xcorr, &y, len as i32, mp, &mut bp);
        let par = format!("len={} mp={}", len, mp);
        report_i2("findbest", &par, input, fnv_i32_hash(&bp), bp[0], bp[1]);
    }
}

/// C: `_celt_autocorr`.
fn sec_autocorr() {
    const NA: [usize; 9] = [8, 12, 16, 17, 24, 32, 33, 64, 128];
    const LA: [i32; 6] = [1, 3, 4, 8, 16, 24];
    const OA: [i32; 3] = [0, 2, 8];
    let mut x = vec![0f32; 260];
    let mut win = vec![0f32; 32];
    let mut ac = vec![0f32; 40];
    for mode in 0..2i32 {
        for &n in NA.iter() {
            for &lag in LA.iter() {
                if lag >= n as i32 {
                    continue;
                }
                for &ov in OA.iter() {
                    if ov > n as i32 / 2 {
                        continue;
                    }
                    fill(&mut x, 1.0, 4000 + (n as u32 * 7 + lag as u32), mode);
                    fill(&mut win, 1.0, 5000 + ov as u32, mode);
                    let input = mix2(fnv_f32_hash(&x), fnv_f32_hash(&win));
                    ac.fill(0.0);
                    let window = if ov != 0 { Some(&win[..]) } else { None };
                    let shift = _celt_autocorr(&x, &mut ac, window, ov, lag, n as i32, 4);
                    let par = format!("n={} lag={} ov={} mode={}", n, lag, ov, mode);
                    report_i1(
                        "autocorr",
                        &par,
                        input,
                        fnv_f32_hash(&ac[..(lag + 1) as usize]),
                        shift,
                    );
                }
            }
        }
    }
}

/// D: `_celt_lpc`.
fn sec_lpc() {
    const PS: [usize; 8] = [1, 2, 3, 4, 8, 12, 16, 24];
    let mut ac = vec![0f32; 40];
    let mut lpc = vec![0f32; 24];
    for t in 0..4i32 {
        for &p in PS.iter() {
            fill(&mut ac, 1.0, 6000 + (t * 31 + p as i32) as u32, t % 2);
            if t == 1 {
                ac[0] = 0.0;
            }
            if t == 2 {
                ac[0] = 1e-10;
            }
            if t == 3 {
                ac[0] = 1.0000001e-10;
            }
            for v in lpc.iter_mut() {
                *v = 12345.0;
            }
            _celt_lpc(&mut lpc, &ac, p as i32);
            let par = format!("p={} t={}", p, t);
            report(
                "lpc",
                &par,
                fnv_f32_hash(&ac[..p + 1]),
                fnv_f32_hash(&lpc[..p]),
            );
        }
    }
}

/// E: `celt_fir` (`celt_fir_c` is the live implementation).
fn sec_fir() {
    const ORDS: [usize; 7] = [1, 2, 3, 4, 8, 16, 24];
    const NS: [usize; 13] = [1, 2, 3, 4, 5, 7, 8, 9, 16, 17, 33, 64, 100];
    let mut xb = vec![0f32; 160];
    let mut num = vec![0f32; 24];
    let mut y = vec![0f32; 128];
    for mode in 0..2i32 {
        for &ord in ORDS.iter() {
            for &n in NS.iter() {
                fill(&mut xb, 1.0, 7000 + (n * 3 + ord) as u32, mode);
                fill(&mut num, 1.0, 8000 + ord as u32, mode);
                let input = mix2(fnv_f32_hash(&xb), fnv_f32_hash(&num));
                y.fill(0.0);
                celt_fir(&xb, &num, &mut y, n as i32, ord as i32, 4);
                let par = format!("n={} ord={} mode={}", n, ord, mode);
                report("fir", &par, input, fnv_f32_hash(&y[..n]));
            }
        }
    }
}

/// F: `celt_iir`.
fn sec_iir() {
    const ORDS: [usize; 4] = [4, 8, 12, 24];
    const NS: [usize; 13] = [1, 2, 3, 4, 5, 7, 8, 9, 16, 17, 33, 64, 100];
    let mut x = vec![0f32; 128];
    let mut den = vec![0f32; 24];
    let mut y = vec![0f32; 128];
    let mut mem = vec![0f32; 24];
    for mode in 0..2i32 {
        for &ord in ORDS.iter() {
            for &n in NS.iter() {
                // celt_iir's tail reads _y[N-i-1] for i < ord, so the C (and
                // this port) require N >= ord.
                if n < ord {
                    continue;
                }
                fill(&mut x, 1.0, 9000 + (n + ord) as u32, mode);
                fill(&mut den, 0.5, 9100 + ord as u32, mode);
                fill(&mut mem, 0.25, 9200 + ord as u32, mode);
                let input = mix2(
                    fnv_f32_hash(&x),
                    mix2(fnv_f32_hash(&den), fnv_f32_hash(&mem)),
                );
                y.fill(0.0);
                celt_iir(&x, &den, &mut y, n as i32, ord as i32, &mut mem, 4);
                let out = fnv_f32_cont(fnv_f32_hash(&y[..n]), &mem[..ord]);
                let par = format!("n={} ord={} mode={}", n, ord, mode);
                report("iir", &par, input, out);
            }
        }
    }
}

/// G: `pitch_downsample`.
fn sec_downsample() {
    const LENS: [usize; 10] = [8, 9, 16, 17, 32, 64, 120, 127, 128, 200];
    let mut ch0 = vec![0f32; 1024];
    let mut ch1 = vec![0f32; 1024];
    let mut xlp = vec![0f32; 256];
    for mode in 0..2i32 {
        for ci in 1..=2i32 {
            for fi in 0..2i32 {
                let factor = if fi == 0 { 2 } else { 4 };
                for &len in LENS.iter() {
                    fill(&mut ch0, 1.0, 10000 + (len + factor as usize) as u32, mode);
                    fill(&mut ch1, 0.5, 11000 + (len + factor as usize) as u32, mode);
                    let input = mix2(fnv_f32_hash(&ch0), fnv_f32_hash(&ch1));
                    xlp.fill(0.0);
                    let chans: [&[f32]; 2] = [&ch0, &ch1];
                    pitch_downsample(&chans, &mut xlp, len as i32, ci, factor, 4);
                    let par = format!("len={} C={} factor={} mode={}", len, ci, factor, mode);
                    report("downsamp", &par, input, fnv_f32_hash(&xlp[..len]));
                }
            }
        }
    }
}

/// H: `pitch_search`.
fn sec_pitch_search() {
    const LENS: [usize; 11] = [16, 17, 32, 33, 48, 64, 100, 120, 128, 200, 256];
    const MPS: [i32; 8] = [4, 5, 8, 16, 17, 32, 40, 100];
    let mut xlp = vec![0f32; 300];
    let mut y = vec![0f32; 600];
    for mode in 0..2i32 {
        for &len in LENS.iter() {
            for &mp in MPS.iter() {
                fill(&mut xlp, 1.0, 12000 + len as u32, mode);
                fill(&mut y, 1.0, 13000 + mp as u32, mode);
                let input = mix2(fnv_f32_hash(&xlp), fnv_f32_hash(&y));
                let mut pitch: i32 = -999;
                pitch_search(&xlp, &y, len as i32, mp, &mut pitch, 4);
                let par = format!("len={} mp={} mode={}", len, mp, mode);
                report_i1("psearch", &par, input, fnv_i32_hash(&[pitch]), pitch);
            }
        }
    }
}

/// I: `remove_doubling`.
fn sec_remove_doubling() {
    const MP: [i32; 6] = [1024, 1024, 512, 512, 256, 1024];
    const MIP: [i32; 6] = [15, 15, 15, 30, 30, 24];
    const NN: [i32; 6] = [960, 120, 480, 240, 240, 960];
    const T0S: [i32; 6] = [300, 900, 100, 64, 150, 500];
    const PP: [i32; 6] = [320, 880, 120, 60, 140, 480];
    const PG: [f32; 6] = [0.5, 0.25, 0.75, 0.1, 0.9, 0.0];
    let mut buf = vec![0f32; 4096];
    for mode in 0..3i32 {
        for i in 0..6usize {
            fill(&mut buf, 1.0, 14000 + i as u32, mode);
            let input = fnv_f32_hash(&buf);
            let mut t0 = T0S[i];
            let g = remove_doubling(&buf, MP[i], MIP[i], NN[i], &mut t0, PP[i], PG[i], 4);
            let par = format!(
                "mp={} mip={} N={} T0={} pp={} mode={}",
                MP[i], MIP[i], NN[i], T0S[i], PP[i], mode
            );
            report_i1("remdbl", &par, input, fnv_f32_hash(&[g]), t0);
        }
    }
}

/// J: `celt_fir5`.
fn sec_fir5() {
    const NS: [usize; 9] = [1, 2, 3, 4, 5, 8, 16, 31, 64];
    let mut buf = vec![0f32; 128];
    let mut num = vec![0f32; 5];
    for mode in 0..2i32 {
        for &n in NS.iter() {
            fill(&mut buf, 1.0, 15000 + n as u32, mode);
            fill(&mut num, 1.0, 16000 + n as u32, mode);
            let input = mix2(fnv_f32_hash(&buf), fnv_f32_hash(&num));
            celt_fir5(&mut buf, &num, n as i32);
            let par = format!("n={} mode={}", n, mode);
            report("fir5", &par, input, fnv_f32_hash(&buf[..n]));
        }
    }
}

fn main() {
    println!("probe_pitch arch=4");
    sec_xcorr();
    sec_find_best();
    sec_autocorr();
    sec_lpc();
    sec_fir();
    sec_iir();
    sec_downsample();
    sec_pitch_search();
    sec_remove_doubling();
    sec_fir5();
}

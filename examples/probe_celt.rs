//! Rust twin of `compat/c/probe_celt.c`.
//!
//! Run with `bash compat/scripts/run-probe.sh celt`; the outputs must be
//! byte-identical.

use oprs::celt::celt::*;
use oprs::celt::modes::{opus_custom_mode_create, opus_custom_mode_destroy};

const HIST: usize = 1100;
const XBUF: usize = HIST + 2048;

// ---- deterministic input ---------------------------------------------------

fn fill_sig(p: &mut [f32], s: u32) {
    let mut st = s;
    for i in 0..p.len() {
        st = st.wrapping_mul(1103515245).wrapping_add(12345);
        let v = (((st >> 9) & 0xFFFF) as i32) - 32768;
        p[i] = v as f32 / 4096.0;
        if i % 13 == 0 {
            p[i] = 0.0;
        }
    }
}

fn fill_window(w: &mut [f32], s: u32) {
    let mut st = s;
    for x in w.iter_mut() {
        st = st.wrapping_mul(1103515245).wrapping_add(12345);
        *x = (st & 0xFFFF) as f32 / 65536.0;
    }
}

fn fnv_f32s(p: &[f32]) -> u64 {
    let mut h: u64 = 1469598103934665603;
    for &v in p {
        let u = v.to_bits();
        for b in 0..4 {
            h ^= ((u >> (8 * b)) & 0xff) as u64;
            h = h.wrapping_mul(1099511628211);
        }
    }
    h
}

/// Prints all N words for short buffers, else the first/last four.
fn print_tail(p: &[f32], n: usize) {
    if n <= 13 {
        for i in 0..n {
            print!(" {:08x}", p[i].to_bits());
        }
    } else {
        for i in 0..4 {
            print!(" {:08x}", p[i].to_bits());
        }
        print!(" ..");
        for i in n - 4..n {
            print!(" {:08x}", p[i].to_bits());
        }
    }
}

// ---- 1. resampling_factor --------------------------------------------------

fn section_resampling() {
    const RATES: [i32; 20] = [
        8000, 12000, 16000, 24000, 48000, 96000, 44100, 22050, 16001, 7999, 8001, 12001, 24001,
        48001, 0, 1, -1, 12345, 32000, 192000,
    ];
    println!("[resampling_factor]");
    for &rate in RATES.iter() {
        println!("rsf {} {}", rate, resampling_factor(rate));
    }
}

// ---- 2. opus_strerror ------------------------------------------------------

fn section_strerror() {
    println!("[opus_strerror]");
    for e in -9..=2 {
        println!("err {} {}", e, opus_strerror(e));
    }
}

// ---- 3. comb_filter_const_c (plain C) --------------------------------------

const CFC_T: [i32; 11] = [0, 1, 2, 3, 15, 16, 17, 20, 33, 100, 1024];
const CFC_N: [i32; 34] = [
    1, 2, 3, 4, 5, 6, 7, 8, 9, 12, 13, 15, 16, 17, 19, 20, 23, 24, 31, 32, 33, 47, 48, 63, 64, 65,
    127, 128, 129, 255, 256, 257, 1000, 1024,
];
const CFC_G: [[f32; 3]; 5] = [
    [0.0, 0.0, 0.0],
    [1.0, -1.0, 0.5],
    [-0.25, 0.125, -0.0625],
    [0.3066406250, 0.2170410156, 0.1296386719],
    [1e-8, -1e-8, 3.0],
];

fn section_comb_const() {
    let mut xbuf = vec![0f32; XBUF];
    let mut ybuf = vec![0f32; 2048];
    let mut k: u32 = 0;
    println!("[comb_filter_const_c]");
    for &t in CFC_T.iter() {
        for &n in CFC_N.iter() {
            for g in CFC_G.iter() {
                let (g10, g11, g12) = (g[0], g[1], g[2]);
                fill_sig(&mut xbuf[..HIST + n as usize + 8], 7000 + k);
                fill_sig(&mut ybuf[..n as usize + 8], 8000 + k);
                let input = fnv_f32s(&xbuf[HIST - t as usize - 2..HIST + n as usize]);
                comb_filter_const_c(
                    &mut ybuf[..n as usize],
                    &xbuf[HIST - t as usize - 2..],
                    t,
                    n,
                    g10,
                    g11,
                    g12,
                );
                let output = fnv_f32s(&ybuf[..n as usize]);
                print!(
                    "cfc T={} N={} g10={:08x} g11={:08x} g12={:08x} in={:016x} out={:016x}",
                    t,
                    n,
                    g10.to_bits(),
                    g11.to_bits(),
                    g12.to_bits(),
                    input,
                    output
                );
                print_tail(&ybuf, n as usize);
                println!();
                k += 1;
            }
        }
    }
}

// ---- 4. comb_filter (arch 4: SSE comb_filter_const) ------------------------

const CF_PAIR: [[i32; 2]; 12] = [
    [0, 0],
    [0, 15],
    [15, 0],
    [15, 15],
    [15, 16],
    [16, 15],
    [17, 18],
    [20, 20],
    [33, 64],
    [100, 15],
    [240, 240],
    [1024, 1024],
];
const CF_N: [i32; 24] = [
    1, 2, 3, 4, 5, 6, 7, 8, 9, 15, 16, 17, 31, 32, 33, 63, 64, 65, 120, 127, 128, 129, 255, 256,
];
const CF_G: [[f32; 2]; 7] = [
    [0.0, 0.0],
    [1.0, 0.0],
    [0.0, 1.0],
    [1.0, 1.0],
    [-1.0, -1.0],
    [0.5, -0.5],
    [-1.0, 1.0],
];
const CF_OV: [i32; 12] = [0, 1, 2, 3, 4, 7, 8, 15, 16, 17, 24, 120];

fn section_comb_filter() {
    let mut xbuf = vec![0f32; XBUF];
    let mut ybuf = vec![0f32; 2048];
    let mut window = vec![0f32; 128];
    let mut k: u32 = 0;
    println!("[comb_filter]");
    for (pi, pair) in CF_PAIR.iter().enumerate() {
        let (t0, t1) = (pair[0], pair[1]);
        let mut h = if t0 > t1 { t0 } else { t1 };
        if h < COMBFILTER_MINPERIOD {
            h = COMBFILTER_MINPERIOD;
        }
        h += 2;
        for (ni, &n) in CF_N.iter().enumerate() {
            for (gi, g) in CF_G.iter().enumerate() {
                let (g0, g1) = (g[0], g[1]);
                let tap0 = ((pi + ni + gi) % 3) as i32;
                let tap1 = ((2 * pi + ni + gi) % 3) as i32;
                let mut ov = CF_OV[(5 * pi + 3 * ni + gi) % 12];
                if ov > n {
                    ov = n;
                }
                fill_window(&mut window[..], 4242 + k);
                fill_sig(&mut xbuf[..HIST + n as usize + 8], 10000 + k);
                fill_sig(&mut ybuf[..n as usize + 8], 20000 + k);
                let input = fnv_f32s(&xbuf[HIST - h as usize..HIST + n as usize]);
                comb_filter(
                    &mut ybuf[..n as usize],
                    &xbuf[HIST - h as usize..HIST + n as usize],
                    t0,
                    t1,
                    n,
                    g0,
                    g1,
                    tap0,
                    tap1,
                    &window,
                    ov,
                    4,
                );
                let output = fnv_f32s(&ybuf[..n as usize]);
                print!(
                    "cf T0={} T1={} N={} g0={:08x} g1={:08x} tap0={} tap1={} ov={} in={:016x} out={:016x}",
                    t0,
                    t1,
                    n,
                    g0.to_bits(),
                    g1.to_bits(),
                    tap0,
                    tap1,
                    ov,
                    input,
                    output
                );
                print_tail(&ybuf, n as usize);
                println!();
                // In-place (x == y), the decoder's aliasing case.
                if k % 2 == 0 {
                    fill_sig(&mut xbuf[..HIST + n as usize + 8], 10000 + k);
                    let ainput = fnv_f32s(&xbuf[HIST - h as usize..HIST + n as usize]);
                    comb_filter_in_place(
                        &mut xbuf,
                        HIST as i32,
                        t0,
                        t1,
                        n,
                        g0,
                        g1,
                        tap0,
                        tap1,
                        &window,
                        ov,
                        4,
                    );
                    let aoutput = fnv_f32s(&xbuf[HIST..HIST + n as usize]);
                    print!(
                        "cfA T0={} T1={} N={} g0={:08x} g1={:08x} tap0={} tap1={} ov={} in={:016x} out={:016x}",
                        t0,
                        t1,
                        n,
                        g0.to_bits(),
                        g1.to_bits(),
                        tap0,
                        tap1,
                        ov,
                        ainput,
                        aoutput
                    );
                    print_tail(&xbuf[HIST..], n as usize);
                    println!();
                }
                k += 1;
            }
        }
    }
}

// ---- 5. init_caps ----------------------------------------------------------

const MODE_FS: [i32; 14] = [
    48000, 48000, 48000, 8000, 12000, 16000, 24000, 16000, 48000, 24000, 8000, 12000, 44100, 8000,
];
const MODE_FRAME: [i32; 14] = [960, 240, 120, 40, 60, 80, 120, 320, 800, 240, 160, 240, 441, 20];

fn section_init_caps() {
    println!("[init_caps]");
    for mi in 0..MODE_FS.len() {
        let fs = MODE_FS[mi];
        let frame = MODE_FRAME[mi];
        let mut err = 0i32;
        let m = opus_custom_mode_create(fs, frame, &mut err);
        let m = match m {
            None => {
                println!("mode Fs={} frame={} err={} NULL", fs, frame, err);
                continue;
            }
            Some(m) => m,
        };
        println!(
            "mode Fs={} frame={} err={} nbEBands={} maxLM={} shortMdctSize={}",
            fs, frame, err, m.nbEBands, m.maxLM, m.shortMdctSize
        );
        for lm in 0..=m.maxLM {
            for c in 1..=2 {
                let mut cap = [0i32; 64];
                init_caps(&m, &mut cap, lm, c);
                print!("caps LM={} C={}", lm, c);
                for i in 0..m.nbEBands as usize {
                    print!(" {}", cap[i]);
                }
                println!();
            }
        }
        opus_custom_mode_destroy(Some(m));
    }
}

fn main() {
    section_resampling();
    section_strerror();
    section_comb_const();
    section_comb_filter();
    section_init_caps();
}

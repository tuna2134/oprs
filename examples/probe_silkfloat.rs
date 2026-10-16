//! Rust side of the SILK float analysis (FLP) bit-exactness probe.
//!
//! NOTE ON COVERAGE: the cases that dispatch `silk_NSQ_del_dec` to the arch-4
//! `silk_NSQ_del_dec_avx2` kernel (encoder complexity >= 6, i.e.
//! nStatesDelayedDecision 3/4) are gated behind CHECK_AVX2_STATE below.  The
//! gate exists because that kernel once diverged from the C reference in its
//! final state write-back only (identical pulses, different
//! sLPC_Q14/sAR2_Q14/sLF_AR_shp_Q14/sDiff_shp_Q14); it is bit-exact now, so the
//! gate is ON, and `case_pred_chain` additionally prints the NSQ state directly
//! for the non-AVX2 kernel.  Keep the gate: with it off, no probe in the tree
//! covers the AVX2 state path.
//!
//! Mirrors `compat/c/probe_silkfloat.c` line for line.  Run
//! `bash compat/scripts/run-probe.sh silkfloat` to build both sides and diff
//! their output.

#![allow(non_snake_case)]
#![allow(unused_assignments)]
#![allow(clippy::needless_range_loop)]

use oprs::celt::entenc::{ec_enc_done, ec_enc_init};
use oprs::silk::control::silk_EncControlStruct;
use oprs::silk::control_codec::silk_control_encoder;
use oprs::silk::define::*;
use oprs::silk::float::apply_sine_window_flp::silk_apply_sine_window_FLP;
use oprs::silk::float::autocorrelation_flp::silk_autocorrelation_FLP;
use oprs::silk::float::burg_modified_flp::silk_burg_modified_FLP;
use oprs::silk::float::bwexpander_flp::silk_bwexpander_FLP;
use oprs::silk::float::corrmatrix_flp::{silk_corrMatrix_FLP, silk_corrVector_FLP};
use oprs::silk::float::encode_frame_flp::{silk_encode_do_VAD_FLP, silk_encode_frame_FLP};
use oprs::silk::float::energy_flp::silk_energy_FLP;
use oprs::silk::float::find_lpc_flp::silk_find_LPC_FLP;
use oprs::silk::float::find_ltp_flp::silk_find_LTP_FLP;
use oprs::silk::float::find_pitch_lags_flp::silk_find_pitch_lags_FLP;
use oprs::silk::float::find_pred_coefs_flp::silk_find_pred_coefs_FLP;
use oprs::silk::float::inner_product_flp::{
    silk_inner_product_FLP, silk_inner_product_FLP_avx2, silk_inner_product_FLP_c,
};
use oprs::silk::float::k2a_flp::silk_k2a_FLP;
use oprs::silk::float::lpc_analysis_filter_flp::silk_LPC_analysis_filter_FLP;
use oprs::silk::float::lpc_inv_pred_gain_flp::silk_LPC_inverse_pred_gain_FLP;
use oprs::silk::float::ltp_analysis_filter_flp::silk_LTP_analysis_filter_FLP;
use oprs::silk::float::ltp_scale_ctrl_flp::silk_LTP_scale_ctrl_FLP;
use oprs::silk::float::main_flp::{silk_encoder_control_FLP, silk_encoder_state_FLP};
use oprs::silk::float::noise_shape_analysis_flp::silk_noise_shape_analysis_FLP;
use oprs::silk::float::pitch_analysis_core_flp::silk_pitch_analysis_core_FLP;
use oprs::silk::float::process_gains_flp::silk_process_gains_FLP;
use oprs::silk::float::regularize_correlations_flp::silk_regularize_correlations_FLP;
use oprs::silk::float::residual_energy_flp::{
    silk_residual_energy_covar_FLP, silk_residual_energy_FLP,
};
use oprs::silk::float::scale_copy_vector_flp::silk_scale_copy_vector_FLP;
use oprs::silk::float::scale_vector_flp::silk_scale_vector_FLP;
use oprs::silk::float::schur_flp::silk_schur_FLP;
use oprs::silk::float::sigproc_flp::{silk_float2int, silk_short2float_array};
use oprs::silk::float::sort_flp::silk_insertion_sort_decreasing_FLP;
use oprs::silk::float::warped_autocorrelation_flp::silk_warped_autocorrelation_FLP;
use oprs::silk::float::wrappers_flp::{
    silk_A2NLSF_FLP, silk_NLSF2A_FLP, silk_NSQ_wrapper_FLP, silk_process_NLSFs_FLP,
    silk_quant_LTP_gains_FLP,
};
use oprs::silk::init_encoder::silk_init_encoder;
use oprs::silk::lp_variable_cutoff::silk_LP_variable_cutoff;

/* ------------------------------------------------------------------ */
/* Deterministic input set                                             */
/* ------------------------------------------------------------------ */

static mut RNG_STATE: u32 = 0;

fn rng_seed(s: u32) {
    unsafe {
        RNG_STATE = s;
    }
}
fn rnd() -> i32 {
    unsafe {
        RNG_STATE = RNG_STATE.wrapping_mul(1103515245).wrapping_add(12345);
        RNG_STATE as i32
    }
}
fn rnd_below(m: i32) -> i32 {
    ((rnd() as u32) % (m as u32)) as i32
}

/* Audio-like value roughly in [-1000, 1000). */
fn rnd_f() -> f32 {
    let v = rnd() % 65536 - 32768;
    ((v as f64 / 32768.0) as f32) * 1000.0
}

/* Small value in [-1, 1). */
fn rnd_small() -> f32 {
    let v = rnd() % 65536 - 32768;
    (v as f64 / 32768.0) as f32
}

/* Speech-like, band-limited signal (three sines), bounded well inside the
 * int16 domain.  Consumes 6 LCG draws for the partials plus one per sample. */
fn gen_speech(a: &mut [f32], n: usize, fs_kHz: i32) {
    const PI_D: f64 = 3.14159265358979323846;
    let f1 = 100.0 + rnd_below(100) as f64;
    let f2 = 200.0 + rnd_below(400) as f64;
    let mut f3 = 800.0 + rnd_below(1200) as f64;
    let p1 = rnd_below(628) as f64 / 100.0;
    let p2 = rnd_below(628) as f64 / 100.0;
    let p3 = rnd_below(628) as f64 / 100.0;
    let nyq = fs_kHz as f64 * 1000.0 * 0.5;
    if f3 > nyq * 0.8 {
        f3 = nyq * 0.4;
    }
    for i in 0..n {
        let t = i as f64 / (fs_kHz as f64 * 1000.0);
        let v = 0.6 * (2.0 * PI_D * f1 * t + p1).sin()
            + 0.3 * (2.0 * PI_D * f2 * t + p2).sin()
            + 0.1 * (2.0 * PI_D * f3 * t + p3).sin();
        a[i] = (v * 20000.0) as f32;
    }
}

const PAT_BITS: [u32; 16] = [
    0x0000_0000, 0x8000_0000, 0x3F80_0000, 0xBF80_0000, 0x0000_0001, 0x007F_FFFF, 0x0080_0000,
    0x7F7F_FFFF, 0x7F80_0000, 0xFF80_0000, 0x7FC0_0000, 0x3400_0000, 0x4B00_0000, 0xC000_0000,
    0x3F00_0000, 0x7FFF_FFFF,
];
const PAT_FINITE_BITS: [u32; 16] = [
    0x0000_0000, 0x8000_0000, 0x3F80_0000, 0xBF80_0000, 0x0000_0001, 0x007F_FFFF, 0x0080_0000,
    0x7F7F_FFFF, 0x0000_0002, 0x80FF_FFFF, 0x3400_0000, 0xB400_0000, 0x4B00_0000, 0xC000_0000,
    0x3F00_0000, 0x7EFF_FFFF,
];

fn bits_to_f(b: u32) -> f32 {
    f32::from_bits(b)
}
fn pat(i: i32) -> f32 {
    bits_to_f(PAT_BITS[(i & 15) as usize])
}
fn pat_finite(i: i32) -> f32 {
    bits_to_f(PAT_FINITE_BITS[(i & 15) as usize])
}

/* ------------------------------------------------------------------ */
/* Print helpers                                                       */
/* ------------------------------------------------------------------ */

/* The payload of a NaN is not specified by IEEE-754 and differs between the
 * two compilers' instruction selection (clang vs rustc), so every NaN is
 * printed as the canonical quiet NaN.  A NaN appearing where the other side
 * has a number (or vice versa) still shows up as a difference. */
fn f32_print_bits(v: f32) -> u32 {
    if v.is_nan() {
        0x7FC0_0000
    } else {
        v.to_bits()
    }
}

fn ph(n: &str) {
    println!("== {} ==", n);
}
fn pi(n: &str, v: i64) {
    println!("{}={}", n, v);
}
fn pf(n: &str, v: f32) {
    println!("{}={:08x}", n, f32_print_bits(v));
}
fn pd(n: &str, v: f64) {
    if v.is_nan() {
        println!("{}=7ff8000000000000", n);
    } else {
        println!("{}={:016x}", n, v.to_bits());
    }
}
fn pfa(n: &str, a: &[f32], len: usize) {
    let mut i = 0;
    while i < len {
        print!("{}[{}]", n, i);
        let mut j = i;
        while j < i + 4 && j < len {
            print!(" {:08x}", f32_print_bits(a[j]));
            j += 1;
        }
        println!();
        i += 4;
    }
}
fn pia(n: &str, a: &[i32], len: usize) {
    let mut i = 0;
    while i < len {
        print!("{}[{}]", n, i);
        let mut j = i;
        while j < i + 8 && j < len {
            print!(" {}", a[j]);
            j += 1;
        }
        println!();
        i += 8;
    }
}
fn pi16a(n: &str, a: &[i16], len: usize) {
    let mut i = 0;
    while i < len {
        print!("{}[{}]", n, i);
        let mut j = i;
        while j < i + 8 && j < len {
            print!(" {}", a[j]);
            j += 1;
        }
        println!();
        i += 8;
    }
}
fn pi8a(n: &str, a: &[i8], len: usize) {    let mut i = 0;
    while i < len {
        print!("{}[{}]", n, i);
        let mut j = i;
        while j < i + 8 && j < len {
            print!(" {}", a[j]);
            j += 1;
        }
        println!();
        i += 8;
    }
}

/* ------------------------------------------------------------------ */
/* 1. inner_product_FLP: C, AVX2 and the arch dispatch                 */
/* ------------------------------------------------------------------ */

const IP_SIZES: [i32; 30] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 15, 16, 17, 23, 24, 25, 31, 32, 33, 63, 64, 65, 100,
    160, 240, 320, 384,
];

fn case_inner_product() {
    let mut a = [0.0f32; 384];
    let mut b = [0.0f32; 384];

    ph("inner_product_random");
    for c in 0..IP_SIZES.len() {
        let n = IP_SIZES[c];
        rng_seed(0x1000 + c as u32);
        for i in 0..n {
            a[i as usize] = rnd_f();
        }
        for i in 0..n {
            b[i as usize] = rnd_f();
        }
        println!("n={}", n);
        pd("  c", silk_inner_product_FLP_c(&a, &b, n));
        pd("  avx2", silk_inner_product_FLP_avx2(&a, &b, n));
        pd("  dispatch4", silk_inner_product_FLP(&a, &b, n, 4));
        pd("  dispatch0", silk_inner_product_FLP(&a, &b, n, 0));
    }

    ph("inner_product_patterns");
    for c in 0..IP_SIZES.len() {
        let n = IP_SIZES[c];
        for i in 0..n {
            a[i as usize] = pat(i);
        }
        for i in 0..n {
            b[i as usize] = pat(i + 5);
        }
        println!("n={}", n);
        pd("  c", silk_inner_product_FLP_c(&a, &b, n));
        pd("  avx2", silk_inner_product_FLP_avx2(&a, &b, n));
        pd("  dispatch4", silk_inner_product_FLP(&a, &b, n, 4));
    }

    ph("inner_product_avx2_finite_patterns");
    for c in 0..IP_SIZES.len() {
        let n = IP_SIZES[c];
        for i in 0..n {
            a[i as usize] = pat_finite(i * 3 + 1);
        }
        for i in 0..n {
            b[i as usize] = pat_finite(i * 7 + 2);
        }
        println!("n={}", n);
        pd("  avx2", silk_inner_product_FLP_avx2(&a, &b, n));
        pd("  c", silk_inner_product_FLP_c(&a, &b, n));
    }
}

/* ------------------------------------------------------------------ */
/* 2. energy_FLP                                                       */
/* ------------------------------------------------------------------ */

fn case_energy() {
    let mut a = [0.0f32; 384];

    ph("energy_random");
    for c in 0..IP_SIZES.len() {
        let n = IP_SIZES[c];
        rng_seed(0x2000 + c as u32);
        for i in 0..n {
            a[i as usize] = rnd_f();
        }
        println!("n={}", n);
        pd("  e", silk_energy_FLP(&a, n));
    }
    ph("energy_patterns");
    for c in 0..IP_SIZES.len() {
        let n = IP_SIZES[c];
        for i in 0..n {
            a[i as usize] = pat(i + 3);
        }
        println!("n={}", n);
        pd("  e", silk_energy_FLP(&a, n));
    }
}

/* ------------------------------------------------------------------ */
/* 3. scale_vector_FLP / scale_copy_vector_FLP                         */
/* ------------------------------------------------------------------ */

fn case_scale_vectors() {
    const LENS: [i32; 14] = [0, 1, 2, 3, 4, 5, 7, 8, 9, 16, 17, 33, 64, 100];
    const GAINS: [u32; 6] = [
        0x0000_0000, 0x8000_0000, 0x3F80_0000, 0xBF80_0000, 0x0000_0001, 0x4000_0000,
    ];
    let mut buf_scale = [0.0f32; 128];
    let mut src_scale = [0.0f32; 128];
    let mut dst_scale = [0.0f32; 128];

    ph("scale_vector");
    for c in 0..LENS.len() {
        let n = LENS[c];
        for j in 0..GAINS.len() {
            rng_seed(0x3000 + (c * 8 + j) as u32);
            for i in 0..n {
                buf_scale[i as usize] = rnd_f();
            }
            silk_scale_vector_FLP(&mut buf_scale, bits_to_f(GAINS[j]), n);
            println!("n={}", n);
            pf("  gain", bits_to_f(GAINS[j]));
            pfa("  v", &buf_scale, n as usize);
        }
    }

    ph("scale_copy_vector");
    for c in 0..LENS.len() {
        let n = LENS[c];
        for j in 0..GAINS.len() {
            rng_seed(0x3800 + (c * 8 + j) as u32);
            for i in 0..n {
                src_scale[i as usize] = rnd_f();
            }
            for i in 0..128 {
                dst_scale[i] = 0.0;
            }
            silk_scale_copy_vector_FLP(&mut dst_scale, &src_scale, bits_to_f(GAINS[j]), n);
            println!("n={}", n);
            pf("  gain", bits_to_f(GAINS[j]));
            pfa("  in", &src_scale, n as usize);
            pfa("  out", &dst_scale, n as usize);
        }
    }
}

/* ------------------------------------------------------------------ */
/* 4. bwexpander_FLP                                                   */
/* ------------------------------------------------------------------ */

fn case_bwexpander() {
    const DS: [i32; 6] = [1, 2, 3, 10, 16, 24];
    const CHIRPS: [u32; 5] = [0x0000_0000, 0x3F00_0000, 0x3F7D_70A4, 0x3F80_0000, 0xBF80_0000];
    let mut ar_bwe = [0.0f32; 24];

    ph("bwexpander");
    for c in 0..DS.len() {
        let d = DS[c];
        for j in 0..CHIRPS.len() {
            rng_seed(0x4000 + (c * 8 + j) as u32);
            for i in 0..d {
                ar_bwe[i as usize] = rnd_small();
            }
            silk_bwexpander_FLP(&mut ar_bwe, d, bits_to_f(CHIRPS[j]));
            println!("d={}", d);
            pf("  chirp", bits_to_f(CHIRPS[j]));
            pfa("  ar", &ar_bwe, d as usize);
        }
    }
}

/* ------------------------------------------------------------------ */
/* 5. apply_sine_window_FLP                                            */
/* ------------------------------------------------------------------ */

fn case_sine_window() {
    const LENS: [i32; 7] = [4, 8, 32, 64, 128, 240, 320];
    let mut win_in = [0.0f32; 320];
    let mut win_out = [0.0f32; 320];

    ph("apply_sine_window");
    for c in 0..LENS.len() {
        let n = LENS[c];
        for w in 1..=2 {
            rng_seed(0x5000 + (c * 4 + w as usize) as u32);
            for i in 0..n {
                win_in[i as usize] = rnd_f();
            }
            for i in 0..n {
                win_out[i as usize] = 0.0;
            }
            silk_apply_sine_window_FLP(&mut win_out, &win_in, w, n);
            println!("n={} w={}", n, w);
            pfa("  in", &win_in, n as usize);
            pfa("  out", &win_out, n as usize);
        }
    }
}

/* ------------------------------------------------------------------ */
/* 6. silk_insertion_sort_decreasing_FLP                               */
/* ------------------------------------------------------------------ */

fn case_sort() {
    const LK: [[i32; 2]; 8] = [
        [4, 4],
        [8, 3],
        [8, 8],
        [16, 1],
        [16, 16],
        [24, 12],
        [1, 1],
        [2, 1],
    ];
    let mut srt_a = [0.0f32; 24];
    let mut srt_idx = [0i32; 24];

    ph("insertion_sort");
    for c in 0..LK.len() {
        let (l, k) = (LK[c][0], LK[c][1]);
        rng_seed(0x6000 + c as u32);
        for i in 0..l {
            srt_a[i as usize] = rnd_below(10) as f32;
        }
        for i in 0..l {
            srt_idx[i as usize] = -1;
        }
        silk_insertion_sort_decreasing_FLP(&mut srt_a, &mut srt_idx, l, k);
        println!("L={} K={}", l, k);
        pfa("  a", &srt_a, l as usize);
        pia("  idx", &srt_idx, l as usize);
    }
    /* distinct values, worst case order */
    rng_seed(0x6100);
    for i in 0..16 {
        srt_a[i] = (16 - i) as f32;
    }
    for i in 0..16 {
        srt_idx[i] = -1;
    }
    silk_insertion_sort_decreasing_FLP(&mut srt_a, &mut srt_idx, 16, 16);
    println!("L=16 K=16 desc");
    pfa("  a", &srt_a, 16);
    pia("  idx", &srt_idx, 16);
}

/* ------------------------------------------------------------------ */
/* 7. silk_k2a_FLP and silk_schur_FLP                                  */
/* ------------------------------------------------------------------ */

fn case_k2a_schur() {
    const ORDERS: [i32; 8] = [1, 2, 5, 8, 10, 12, 16, 24];
    let mut rc_k2a = [0.0f32; 24];
    let mut a_k2a = [0.0f32; 24];
    let mut ac_schur = [0.0f32; 25];
    let mut refl_schur = [0.0f32; 24];

    ph("k2a");
    for c in 0..ORDERS.len() {
        let n = ORDERS[c];
        rng_seed(0x7000 + c as u32);
        for i in 0..n {
            rc_k2a[i as usize] = rnd_small() * 0.9;
        }
        for i in 0..n {
            a_k2a[i as usize] = 0.0;
        }
        silk_k2a_FLP(&mut a_k2a, &rc_k2a, n);
        println!("order={}", n);
        pfa("  rc", &rc_k2a, n as usize);
        pfa("  A", &a_k2a, n as usize);
    }
    /* mixed magnitudes incl. boundary reflection coefficients */
    for c in 0..3 {
        let n = 16;
        rng_seed(0x7100 + c as u32);
        for i in 0..n {
            rc_k2a[i as usize] = if i == c { 0.0 } else { rnd_small() * 0.99 };
        }
        for i in 0..n {
            a_k2a[i as usize] = 0.0;
        }
        silk_k2a_FLP(&mut a_k2a, &rc_k2a, n);
        println!("order={} variant={}", n, c);
        pfa("  A", &a_k2a, n as usize);
    }

    ph("schur");
    for c in 0..ORDERS.len() {
        let n = ORDERS[c];
        rng_seed(0x7200 + c as u32);
        /* positive definite-ish autocorrelation: energy + decaying random taps */
        for i in 0..=n {
            ac_schur[i as usize] = 0.0;
        }
        for _i in 0..64 {
            let v = rnd_f();
            for t in 0..=n {
                ac_schur[t as usize] += v * v / (t + 1) as f32;
            }
        }
        ac_schur[0] += 1.0;
        for i in 0..n {
            refl_schur[i as usize] = 0.0;
        }
        let res = silk_schur_FLP(&mut refl_schur, &ac_schur, n);
        println!("order={}", n);
        pfa("  ac", &ac_schur, (n + 1) as usize);
        pfa("  rc", &refl_schur, n as usize);
        pf("  res", res);
    }
    /* degenerate: zero and negative definite input */
    for c in 0..2 {
        let n = 10;
        for i in 0..=n {
            ac_schur[i as usize] = if c == 0 { 0.0 } else { -1.0 };
        }
        for i in 0..n {
            refl_schur[i as usize] = 0.0;
        }
        let res = silk_schur_FLP(&mut refl_schur, &ac_schur, n);
        println!("order={} degenerate={}", n, c);
        pfa("  rc", &refl_schur, n as usize);
        pf("  res", res);
    }
}

/* ------------------------------------------------------------------ */
/* 8. silk_autocorrelation_FLP                                         */
/* ------------------------------------------------------------------ */

fn case_autocorrelation() {
    const SIZES: [i32; 12] = [1, 2, 3, 4, 8, 16, 39, 40, 64, 160, 240, 320];
    const COUNTS: [i32; 4] = [1, 5, 11, 17];
    let mut ac_in = [0.0f32; 320];
    let mut ac_out = [0.0f32; 32];

    ph("autocorrelation");
    for c in 0..SIZES.len() {
        let n = SIZES[c];
        rng_seed(0x8000 + c as u32);
        for i in 0..n {
            ac_in[i as usize] = rnd_f();
        }
        for j in 0..COUNTS.len() {
            let m = COUNTS[j];
            for i in 0..32 {
                ac_out[i] = 0.0;
            }
            silk_autocorrelation_FLP(&mut ac_out, &ac_in, n, m, 4);
            println!("n={} m={}", n, m);
            pfa("  r4", &ac_out, if m < n { m } else { n } as usize);
            for i in 0..32 {
                ac_out[i] = 0.0;
            }
            silk_autocorrelation_FLP(&mut ac_out, &ac_in, n, m, 0);
            pfa("  r0", &ac_out, if m < n { m } else { n } as usize);
        }
    }
    /* pattern input */
    for i in 0..40 {
        ac_in[i] = pat(i as i32);
    }
    for i in 0..20 {
        ac_out[i] = 0.0;
    }
    silk_autocorrelation_FLP(&mut ac_out, &ac_in, 40, 17, 4);
    println!("n=40 m=17 patterns");
    pfa("  r4", &ac_out, 17);
}

/* ------------------------------------------------------------------ */
/* 9. silk_warped_autocorrelation_FLP                                  */
/* ------------------------------------------------------------------ */

fn case_warped_autocorrelation() {
    const LENS: [i32; 6] = [1, 2, 15, 16, 160, 240];
    const ORDERS: [i32; 4] = [2, 10, 16, 24];
    const WARPS: [u32; 4] = [0x0000_0000, 0x3DCC_CCCD, 0x3F00_0000, 0xBE99_999A];
    let mut wac_in = [0.0f32; 240];
    let mut wac_out = [0.0f32; 25];

    ph("warped_autocorrelation");
    for c in 0..LENS.len() {
        let n = LENS[c];
        rng_seed(0x9000 + c as u32);
        for i in 0..n {
            wac_in[i as usize] = rnd_f();
        }
        for j in 0..ORDERS.len() {
            let order = ORDERS[j];
            for k in 0..WARPS.len() {
                for i in 0..=order {
                    wac_out[i as usize] = 0.0;
                }
                silk_warped_autocorrelation_FLP(
                    &mut wac_out,
                    &wac_in,
                    bits_to_f(WARPS[k]),
                    n,
                    order,
                );
                println!("n={} order={}", n, order);
                pf("  warp", bits_to_f(WARPS[k]));
                pfa("  corr", &wac_out, (order + 1) as usize);
            }
        }
    }
}

/* ------------------------------------------------------------------ */
/* 10. silk_LPC_inverse_pred_gain_FLP                                  */
/* ------------------------------------------------------------------ */

fn case_lpc_inv_pred_gain() {
    const ORDERS: [i32; 9] = [1, 2, 5, 6, 8, 10, 12, 16, 24];
    let mut a_ipg = [0.0f32; 24];
    let mut rc_ipg = [0.0f32; 24];

    ph("lpc_inverse_pred_gain");
    for c in 0..ORDERS.len() {
        let n = ORDERS[c];
        for v in 0..3 {
            rng_seed(0xA000 + (c * 4 + v) as u32);
            for i in 0..n {
                a_ipg[i as usize] = 0.0;
            }
            if v == 0 {
                /* stable: build from reflection coefficients < 1 */
                for i in 0..n {
                    rc_ipg[i as usize] = rnd_small() * 0.8;
                }
                silk_k2a_FLP(&mut a_ipg, &rc_ipg, n);
            } else if v == 1 {
                for i in 0..n {
                    a_ipg[i as usize] = rnd_small() * 0.5;
                }
            } else {
                for i in 0..n {
                    a_ipg[i as usize] = rnd_f() * 0.01;
                }
                a_ipg[(n - 1) as usize] = 2.0;
            }
            println!("order={} variant={}", n, v);
            pfa("  A", &a_ipg, n as usize);
            pf("  invGain", silk_LPC_inverse_pred_gain_FLP(&a_ipg, n));
        }
    }
}

/* ------------------------------------------------------------------ */
/* 11. silk_LPC_analysis_filter_FLP                                    */
/* ------------------------------------------------------------------ */

fn case_lpc_analysis_filter() {
    const ORDERS: [i32; 5] = [6, 8, 10, 12, 16];
    const LENS: [i32; 6] = [16, 17, 32, 80, 240, 320];
    let mut laf_s = [0.0f32; 320];
    let mut laf_a = [0.0f32; 16];
    let mut laf_r = [0.0f32; 320];

    ph("lpc_analysis_filter");
    for c in 0..ORDERS.len() {
        let order = ORDERS[c];
        for d in 0..LENS.len() {
            let n = LENS[d];
            if n < order {
                continue;
            }
            rng_seed(0xB000 + (c * 8 + d) as u32);
            for i in 0..n {
                laf_s[i as usize] = rnd_f();
            }
            for i in 0..order {
                laf_a[i as usize] = rnd_small() * 2.0;
            }
            for i in 0..n {
                laf_r[i as usize] = 0.0;
            }
            silk_LPC_analysis_filter_FLP(&mut laf_r, &laf_a, &laf_s, n, order);
            println!("order={} n={}", order, n);
            pfa("  A", &laf_a, order as usize);
            pfa("  s", &laf_s, n as usize);
            pfa("  r", &laf_r, n as usize);
        }
    }
    /* boundary input */
    for i in 0..64 {
        laf_s[i] = pat(i as i32);
    }
    for i in 0..16 {
        laf_a[i] = if i % 3 == 0 { 1.0 } else { -0.5 };
    }
    for i in 0..64 {
        laf_r[i] = 0.0;
    }
    silk_LPC_analysis_filter_FLP(&mut laf_r, &laf_a, &laf_s, 64, 16);
    println!("order=16 n=64 patterns");
    pfa("  r", &laf_r, 64);
}

/* ------------------------------------------------------------------ */
/* 12. silk_LTP_analysis_filter_FLP                                    */
/* ------------------------------------------------------------------ */

fn case_ltp_analysis_filter() {
    let mut ltp_x = [0.0f32; 704];
    let mut ltp_b = [0.0f32; 25];
    let mut ltp_pitch = [0i32; 4];
    let mut ltp_gains = [0.0f32; 4];
    let mut ltp_res = [0.0f32; 1536];

    ph("ltp_analysis_filter");
    for c in 0..3i32 {
        let nb = if c == 2 { 2 } else { 4 };
        let subfr = if c == 0 { 40 } else { 80 };
        let pre = 16;
        rng_seed(0xC000 + c as u32);
        for i in 0..2 * 320 + 64 {
            ltp_x[i] = rnd_f();
        }
        for i in 0..25 {
            ltp_b[i] = rnd_small();
        }
        for i in 0..4 {
            ltp_pitch[i] = 20 + i as i32 * 7;
        }
        for i in 0..4 {
            ltp_gains[i] = 1.0 / (i + 1) as f32;
        }
        for i in 0..4 * (320 + 64) {
            ltp_res[i] = 0.0;
        }
        silk_LTP_analysis_filter_FLP(
            &mut ltp_res,
            &ltp_x,
            64,
            &ltp_b,
            &ltp_pitch,
            &ltp_gains,
            subfr,
            nb,
            pre,
        );
        println!("nb={} subfr={} pre={}", nb, subfr, pre);
        pfa("  B", &ltp_b, (5 * nb) as usize);
        pfa("  res", &ltp_res, (nb * (subfr + pre)) as usize);
    }
}

/* ------------------------------------------------------------------ */
/* 13. silk_residual_energy_FLP / _covar_FLP                           */
/* ------------------------------------------------------------------ */

fn case_residual_energy() {
    const SUBFRS: [i32; 3] = [16, 40, 80];
    let mut re_x = [0.0f32; 640];
    let mut re_a = [[0.0f32; 16]; 2];
    let mut re_gains = [0.0f32; 4];
    let mut re_nrgs = [0.0f32; 4];

    ph("residual_energy");
    for c in 0..SUBFRS.len() {
        for d in 0..2 {
            let subfr = SUBFRS[c];
            let nb = if d == 0 { 2 } else { 4 };
            let order = if c == 0 { 10 } else { 16 };
            if subfr * nb > 384 {
                continue;
            }
            rng_seed(0xD000 + (c * 4 + d) as u32);
            for i in 0..2 * 320 {
                re_x[i] = rnd_f();
            }
            for k in 0..2 {
                for i in 0..16 {
                    re_a[k][i] = rnd_small() * 0.5;
                }
            }
            for i in 0..4 {
                re_gains[i] = 0.5 + i as f32;
            }
            for i in 0..4 {
                re_nrgs[i] = -1.0;
            }
            silk_residual_energy_FLP(&mut re_nrgs, &re_x, &re_a, &re_gains, subfr, nb, order);
            println!("subfr={} nb={} order={}", subfr, nb, order);
            pfa("  nrgs", &re_nrgs, nb as usize);
        }
    }

    ph("residual_energy_covar");
    {
        const DS: [i32; 5] = [1, 2, 5, 10, 16];
        let mut rc_cov = [0.0f32; 16];
        let mut rwxx_cov = [0.0f32; 256];
        let mut rwxx_cov2 = [0.0f32; 16];
        for c in 0..DS.len() {
            let d = DS[c];
            rng_seed(0xD800 + c as u32);
            for i in 0..d {
                rc_cov[i as usize] = rnd_small() * 0.1;
            }
            for i in 0..d {
                for j2 in i..d {
                    let v = rnd_small() * 0.1;
                    /* `matrix_c_ptr(wXX, i, j, D)` is column-major */
                    rwxx_cov[(i + d * j2) as usize] = v;
                    rwxx_cov[(j2 + d * i) as usize] = v;
                }
                rwxx_cov[(i + d * i) as usize] += 1.0;
            }
            for i in 0..d {
                rwxx_cov2[i as usize] = rnd_small() * 0.1;
            }
            let e = silk_residual_energy_covar_FLP(&rc_cov, &mut rwxx_cov, &rwxx_cov2, 1.0, d);
            println!("D={}", d);
            pf("  e", e);
            pfa("  wXX", &rwxx_cov, (d * d) as usize);
        }
    }
}

/* ------------------------------------------------------------------ */
/* 14. silk_regularize_correlations_FLP                                */
/* ------------------------------------------------------------------ */

fn case_regularize() {
    const DS: [i32; 5] = [1, 2, 5, 10, 16];
    let mut reg_xx = [0.0f32; 256];
    let mut reg_xx1 = [0.0f32; 1];

    ph("regularize_correlations");
    for c in 0..DS.len() {
        let d = DS[c];
        rng_seed(0xE000 + c as u32);
        for i in 0..d * d {
            reg_xx[i as usize] = rnd_small();
        }
        reg_xx1[0] = rnd_small();
        silk_regularize_correlations_FLP(&mut reg_xx, &mut reg_xx1, 0.25, d);
        println!("D={}", d);
        pfa("  XX", &reg_xx, (d * d) as usize);
        pf("  xx", reg_xx1[0]);
    }
}

/* ------------------------------------------------------------------ */
/* 15. silk_corrMatrix_FLP / silk_corrVector_FLP                       */
/* ------------------------------------------------------------------ */

fn case_corrmatrix() {
    const LS: [i32; 3] = [8, 16, 40];
    const ORDERS: [i32; 4] = [2, 5, 10, 16];
    let mut cm_x = [0.0f32; 64];
    let mut cm_t = [0.0f32; 40];
    let mut cm_xx = [0.0f32; 256];
    let mut cm_xt = [0.0f32; 16];

    ph("corrmatrix");
    for c in 0..LS.len() {
        let l = LS[c];
        for d in 0..ORDERS.len() {
            let order = ORDERS[d];
            rng_seed(0xF000 + (c * 8 + d) as u32);
            for i in 0..l + order - 1 + 8 {
                cm_x[i as usize] = rnd_f();
            }
            for i in 0..16 * 16 {
                cm_xx[i] = 0.0;
            }
            for i in 0..16 {
                cm_xt[i] = 0.0;
            }
            for i in 0..l {
                cm_t[i as usize] = rnd_f();
            }
            silk_corrMatrix_FLP(&cm_x, l, order, &mut cm_xx, 4);
            silk_corrVector_FLP(&cm_x, &cm_t, l, order, &mut cm_xt, 4);
            println!("L={} order={}", l, order);
            pfa("  x", &cm_x, (l + order - 1) as usize);
            pfa("  t", &cm_t, l as usize);
            pfa("  XX", &cm_xx, (order * order) as usize);
            pfa("  Xt", &cm_xt, order as usize);
        }
    }
}

/* ------------------------------------------------------------------ */
/* 16. silk_burg_modified_FLP                                          */
/* ------------------------------------------------------------------ */

fn case_burg() {
    /* (D, subfr_length, nb_subfr) with subfr_length * nb_subfr <= 384 */
    const CFG: [[i32; 3]; 12] = [
        [6, 40, 4],
        [8, 40, 4],
        [10, 40, 4],
        [12, 40, 4],
        [16, 40, 4],
        [10, 80, 4],
        [16, 80, 4],
        [16, 80, 2],
        [16, 40, 2],
        [20, 40, 4],
        [24, 40, 4],
        [16, 16, 4],
    ];
    const MIN_INV: [u32; 3] = [0x3DCC_CCCD, 0x38D1_B717, 0x3F80_0000];
    let mut burg_x = [0.0f32; 384];
    let mut burg_a = [0.0f32; 24];

    ph("burg_modified");
    for c in 0..CFG.len() {
        let d = CFG[c][0];
        let subfr = CFG[c][1];
        let nb = CFG[c][2];
        for j in 0..MIN_INV.len() {
            rng_seed(0x10000 + (c * 8 + j) as u32);
            for i in 0..384 {
                burg_x[i] = rnd_f();
            }
            for i in 0..24 {
                burg_a[i] = 0.0;
            }
            /* the reference C entry point is silk_burg_modified_FLP (not
             * dispatched in the float build) */
            let e = silk_burg_modified_FLP(&mut burg_a, &burg_x, bits_to_f(MIN_INV[j]), subfr, nb, d, 4);
            println!("D={} subfr={} nb={}", d, subfr, nb);
            pf("  minInvGain", bits_to_f(MIN_INV[j]));
            pfa("  A", &burg_a, d as usize);
            pf("  nrg", e);
        }
    }
    /* boundary input */
    for i in 0..384 {
        burg_x[i] = pat(i as i32);
    }
    for i in 0..24 {
        burg_a[i] = 0.0;
    }
    {
        let e = silk_burg_modified_FLP(&mut burg_a, &burg_x, 0.1, 40, 4, 16, 4);
        println!("D=16 patterns");
        pfa("  A", &burg_a, 16);
        pf("  nrg", e);
    }
}

/* ------------------------------------------------------------------ */
/* 17. silk_find_LTP_FLP                                               */
/* ------------------------------------------------------------------ */

fn case_find_ltp() {
    let mut fl_r = [0.0f32; 640];
    let mut fl_lag = [0i32; 4];
    let mut fl_xx = [0.0f32; 100];
    let mut fl_xx_x = [0.0f32; 20];

    ph("find_ltp");
    for c in 0..3i32 {
        let nb = if c == 2 { 2 } else { 4 };
        let subfr = if c == 0 { 40 } else { 80 };
        rng_seed(0x11000 + c as u32);
        for i in 0..2 * 320 {
            fl_r[i] = rnd_f();
        }
        for i in 0..4 {
            fl_lag[i] = 20 + i as i32 * 11;
        }
        for i in 0..4 * 25 {
            fl_xx[i] = 0.0;
        }
        for i in 0..4 * 5 {
            fl_xx_x[i] = 0.0;
        }
        silk_find_LTP_FLP(&mut fl_xx, &mut fl_xx_x, &fl_r, 160, &fl_lag, subfr, nb, 4);
        println!("nb={} subfr={}", nb, subfr);
        pfa("  XX", &fl_xx, (nb * 25) as usize);
        pfa("  xX", &fl_xx_x, (nb * 5) as usize);
    }
}

/* ------------------------------------------------------------------ */
/* 18/19. encoder-state cases                                          */
/* ------------------------------------------------------------------ */

fn init_enc(e: &mut silk_encoder_state_FLP, fs_kHz: i32, nb_subfr: i32) {
    *e = silk_encoder_state_FLP::default();
    e.sCmn.fs_kHz = fs_kHz;
    e.sCmn.nb_subfr = nb_subfr;
    e.sCmn.subfr_length = 5 * fs_kHz;
    e.sCmn.frame_length = nb_subfr * 5 * fs_kHz;
    e.sCmn.ltp_mem_length = 20 * fs_kHz;
    e.sCmn.la_pitch = 2 * fs_kHz;
    e.sCmn.la_shape = 5 * fs_kHz;
    e.sCmn.shapeWinLength = 5 * fs_kHz + 2 * e.sCmn.la_shape;
    e.sCmn.pitch_LPC_win_length = (if nb_subfr == 4 { 24 } else { 14 }) * fs_kHz;
    e.sCmn.arch = 4;
}

fn fill_enc_random(e: &mut silk_encoder_state_FLP, seed: u32) {
    rng_seed(seed);
    let fs = e.sCmn.fs_kHz;
    gen_speech(&mut e.x_buf, 2 * 320 + 80, fs);
    e.LTPCorr = rnd_small();
    e.sCmn.speech_activity_Q8 = rnd_below(256);
    e.sCmn.input_tilt_Q15 = rnd_below(32768) - 16384;
    e.sCmn.SNR_dB_Q7 = rnd_below(100 * 128);
    for i in 0..4 {
        e.sCmn.input_quality_bands_Q15[i] = rnd_below(32768);
    }
    e.sCmn.prevSignalType = rnd_below(3) as i8;
    e.sCmn.prevLag = rnd_below(20) * e.sCmn.fs_kHz;
    e.sCmn.first_frame_after_reset = rnd_below(2);
    e.sCmn.useInterpolatedNLSFs = rnd_below(2);
    e.sCmn.indices.quantOffsetType = rnd_below(2) as i8;
    e.sCmn.indices.signalType = rnd_below(3) as i8;
    e.sCmn.warping_Q16 = rnd_below(3) * 1000;
    e.sCmn.useCBR = rnd_below(2);
    e.sCmn.nStatesDelayedDecision = 1 + rnd_below(4);
    e.sCmn.shapingLPCOrder = 24;
    e.sCmn.predictLPCOrder = 16;
    e.sCmn.pitchEstimationLPCOrder = 16;
    e.sCmn.pitchEstimationComplexity = 2;
    /* `control_codec.c`: silk_SMULWB(16384, speech_activity_Q8 + 8192), i.e.
     * (16384*(sa+8192))>>16 = (sa+8192)/4, in Q16 (0.03 .. 0.09). */
    e.sCmn.pitchEstimationThreshold_Q16 = (e.sCmn.speech_activity_Q8 + 8192) / 4;
}

fn case_ltp_scale_ctrl() {
    ph("ltp_scale_ctrl");
    for c in 0..6i32 {
        let mut e = silk_encoder_state_FLP::default();
        let mut ctrl = silk_encoder_control_FLP::default();
        init_enc(&mut e, 16, 4);
        rng_seed(0x12000 + c as u32);
        e.sCmn.PacketLoss_perc = rnd_below(101);
        e.sCmn.nFramesPerPacket = 1 + rnd_below(3);
        e.sCmn.LBRR_flag = rnd_below(2) as i8;
        e.sCmn.SNR_dB_Q7 = rnd_below(100 * 128);
        ctrl.LTPredCodGain = rnd_small() * 40.0;
        silk_LTP_scale_ctrl_FLP(&mut e, &mut ctrl, c % 3);
        println!("c={} cond={}", c, c % 3);
        pf("  LTPredCodGain", ctrl.LTPredCodGain);
        pi("  loss", e.sCmn.PacketLoss_perc as i64);
        pi("  nFramesPerPacket", e.sCmn.nFramesPerPacket as i64);
        pi("  LBRR_flag", e.sCmn.LBRR_flag as i64);
        pi("  SNR_dB_Q7", e.sCmn.SNR_dB_Q7 as i64);
        pi("  LTP_scaleIndex", e.sCmn.indices.LTP_scaleIndex as i64);
        pf("  LTP_scale", ctrl.LTP_scale);
    }
}

fn case_process_gains() {
    ph("process_gains");
    for c in 0..4i32 {
        let mut e = silk_encoder_state_FLP::default();
        let mut ctrl = silk_encoder_control_FLP::default();
        init_enc(&mut e, 16, 4);
        rng_seed(0x13000 + c as u32);
        e.sCmn.SNR_dB_Q7 = rnd_below(100 * 128);
        e.sCmn.indices.signalType = if c % 2 == 0 {
            TYPE_VOICED as i8
        } else {
            TYPE_UNVOICED as i8
        };
        e.sCmn.nStatesDelayedDecision = 1 + rnd_below(4);
        e.sCmn.speech_activity_Q8 = rnd_below(256);
        e.sCmn.input_tilt_Q15 = rnd_below(32768) - 16384;
        e.sShape.LastGainIndex = rnd_below(64) as i8;
        for i in 0..4 {
            ctrl.Gains[i] = rnd_small() * 100.0;
            ctrl.ResNrg[i] = rnd_below(1000) as f32;
        }
        ctrl.LTPredCodGain = rnd_small() * 40.0;
        ctrl.input_quality = rnd_small();
        ctrl.coding_quality = rnd_small();
        silk_process_gains_FLP(&mut e, &mut ctrl, c % 3);
        println!("c={}", c);
        pi("  signalType", e.sCmn.indices.signalType as i64);
        pi("  quantOffsetType", e.sCmn.indices.quantOffsetType as i64);
        pfa("  Gains", &ctrl.Gains, 4);
        pia("  GainsUnq_Q16", &ctrl.GainsUnq_Q16, 4);
        pi8a("  GainsIndices", &e.sCmn.indices.GainsIndices, 4);
        pf("  Lambda", ctrl.Lambda);
        pi("  lastGainIndexPrev", ctrl.lastGainIndexPrev as i64);
        pi("  LastGainIndex", e.sShape.LastGainIndex as i64);
    }
}

/* ------------------------------------------------------------------ */
/* 20. silk_find_pitch_lags_FLP                                        */
/* ------------------------------------------------------------------ */

fn case_find_pitch_lags() {
    const FS: [i32; 2] = [8, 16];
    const NBS: [i32; 2] = [4, 2];

    ph("find_pitch_lags");
    for c in 0..2 {
        for d in 0..2 {
            let mut e = silk_encoder_state_FLP::default();
            let mut ctrl = silk_encoder_control_FLP::default();
            let mut res = [0.0f32; 2 * 320 + 80];
            init_enc(&mut e, FS[c], NBS[d]);
            fill_enc_random(&mut e, 0x14000 + (c * 4 + d) as u32);
            /* deterministic signal type so both branches get exercised */
            e.sCmn.indices.signalType = ((c + d) % 3) as i8;
            e.sCmn.first_frame_after_reset = 0;
            let xb = e.x_buf;
            let arch = e.sCmn.arch;
            silk_find_pitch_lags_FLP(&mut e, &mut ctrl, &mut res, &xb, arch);
            println!("fs={} nb={}", FS[c], NBS[d]);
            pfa(
                "  res",
                &res,
                (e.sCmn.la_pitch + e.sCmn.frame_length + e.sCmn.ltp_mem_length) as usize,
            );
            pf("  predGain", ctrl.predGain);
            pia("  pitchL", &ctrl.pitchL, 4);
            pi("  signalType", e.sCmn.indices.signalType as i64);
            pi("  lagIndex", e.sCmn.indices.lagIndex as i64);
            pi("  contourIndex", e.sCmn.indices.contourIndex as i64);
            pf("  LTPCorr", e.LTPCorr);
        }
    }
}

/* ------------------------------------------------------------------ */
/* 21. silk_pitch_analysis_core_FLP                                    */
/* ------------------------------------------------------------------ */

fn case_pitch_core() {
    const FS: [i32; 3] = [8, 12, 16];
    const NBS: [i32; 2] = [4, 2];
    const CMPL: [i32; 3] = [0, 1, 2];
    let mut pc_frame = [0.0f32; 640];

    ph("pitch_analysis_core");
    for c in 0..3 {
        for d in 0..2 {
            for e2 in 0..3 {
                let mut pitch_out = [-1i32; 4];
                let mut lagIndex: i16 = -1;
                let mut contourIndex: i8 = -1;
                let nb = NBS[d];
                let frame_length = (20 + nb * 5) * FS[c];
                rng_seed(0x15000 + (c * 16 + d * 4 + e2) as u32);
                if e2 == 2 {
                    /* low-correlation input exercises the early-return path */
                    for i in 0..640 {
                        pc_frame[i] = rnd_f() * 0.5;
                    }
                } else {
                    gen_speech(&mut pc_frame, 640, FS[c]);
                }
                let mut LTPCorr = rnd_small() * 0.5;
                let prevLag = if c == 2 { 12 * FS[c] } else { 0 };
                let ret = silk_pitch_analysis_core_FLP(
                    &pc_frame,
                    &mut pitch_out,
                    &mut lagIndex,
                    &mut contourIndex,
                    &mut LTPCorr,
                    prevLag,
                    0.5,
                    0.3,
                    FS[c],
                    CMPL[e2],
                    nb,
                    4,
                );
                println!(
                    "fs={} nb={} complexity={} frame_length={}",
                    FS[c], nb, CMPL[e2], frame_length
                );
                pi("  ret", ret as i64);
                pia("  pitchL", &pitch_out, 4);
                pi("  lagIndex", lagIndex as i64);
                pi("  contourIndex", contourIndex as i64);
                pf("  LTPCorr", LTPCorr);
            }
        }
    }
}

/* ------------------------------------------------------------------ */
/* 22. silk_noise_shape_analysis_FLP                                   */
/* ------------------------------------------------------------------ */

fn case_noise_shape() {
    const FS: [i32; 3] = [8, 12, 16];
    let mut ns_pitch_res = [0.0f32; 320];

    ph("noise_shape_analysis");
    for c in 0..3 {
        let mut e = silk_encoder_state_FLP::default();
        let mut ctrl = silk_encoder_control_FLP::default();
        init_enc(&mut e, FS[c], 4);
        fill_enc_random(&mut e, 0x16000 + c as u32);
        e.sCmn.indices.signalType = (if c == 0 {
            TYPE_VOICED
        } else if c == 1 {
            TYPE_UNVOICED
        } else {
            TYPE_NO_VOICE_ACTIVITY
        }) as i8;
        e.sCmn.warping_Q16 = if c == 0 { 0 } else { 1500 };
        e.sCmn.useCBR = if c == 2 { 1 } else { 0 };
        ctrl.predGain = rnd_small() * 100.0;
        ctrl.input_quality = 0.5;
        ctrl.coding_quality = 0.5;
        for i in 0..4 {
            ctrl.pitchL[i] = 20 + i as i32 * 7;
        }
        for i in 0..4 {
            e.sCmn.input_quality_bands_Q15[i] = rnd_below(32768);
        }
        for i in 0..240 {
            ns_pitch_res[i] = 0.0;
        }
        rng_seed(0x16100 + c as u32);
        gen_speech(&mut ns_pitch_res, 320, FS[c]);
        for i in 0..320 {
            ns_pitch_res[i] *= 0.3;
        }
        let la_shape = e.sCmn.la_shape;
        let xb = e.x_buf;
        let off = (e.sCmn.ltp_mem_length - la_shape) as usize;
        silk_noise_shape_analysis_FLP(&mut e, &mut ctrl, &ns_pitch_res, &xb[off..]);
        println!("fs={}", FS[c]);
        pfa("  AR", &ctrl.AR, 4 * 24);
        pfa("  Gains", &ctrl.Gains, 4);
        pfa("  LF_MA_shp", &ctrl.LF_MA_shp, 4);
        pfa("  LF_AR_shp", &ctrl.LF_AR_shp, 4);
        pfa("  Tilt", &ctrl.Tilt, 4);
        pfa("  HarmShapeGain", &ctrl.HarmShapeGain, 4);
        pf("  input_quality", ctrl.input_quality);
        pf("  coding_quality", ctrl.coding_quality);
        pi("  quantOffsetType", e.sCmn.indices.quantOffsetType as i64);
        pf("  HarmShapeGain_smth", e.sShape.HarmShapeGain_smth);
        pf("  Tilt_smth", e.sShape.Tilt_smth);
    }
}

/* true = also run the configurations whose NSQ goes through
 * `silk_NSQ_del_dec_avx2` (complexity >= 6, nStatesDelayedDecision 3/4).
 * Currently enabled and bit-exact; see the module note. */
const CHECK_AVX2_STATE: bool = true;

/* ------------------------------------------------------------------ */
/* 23. wrappers_FLP                                                    */
/* ------------------------------------------------------------------ */

/* Drive the real `silk_init_encoder` + `silk_control_encoder` path so the
 * encoder state is exactly the one the public encoder would produce. */
fn setup_encoder(
    e: &mut silk_encoder_state_FLP,
    API_fs: i32,
    payload_ms: i32,
    bitrate: i32,
    complexity: i32,
    loss: i32,
    fec: i32,
    cbr: i32,
) {
    let mut ctl = silk_EncControlStruct::default();
    silk_init_encoder(e, 4);
    ctl.nChannelsAPI = 1;
    ctl.nChannelsInternal = 1;
    ctl.API_sampleRate = API_fs;
    ctl.maxInternalSampleRate = 16000;
    ctl.minInternalSampleRate = 8000;
    ctl.desiredInternalSampleRate = 16000;
    ctl.payloadSize_ms = payload_ms;
    ctl.bitRate = bitrate;
    ctl.packetLossPercentage = loss;
    ctl.complexity = complexity;
    ctl.useInBandFEC = fec;
    ctl.useDRED = 0;
    ctl.LBRR_coded = 0;
    ctl.useDTX = 0;
    ctl.useCBR = cbr;
    ctl.maxBits = bitrate * payload_ms / 1000;
    ctl.toMono = 0;
    ctl.opusCanSwitch = 0;
    ctl.reducedDependency = 0;
    ctl.internalSampleRate = 0;
    ctl.allowBandwidthSwitch = 0;
    ctl.inWBmodeWithoutVariableLP = 0;
    silk_control_encoder(e, &mut ctl, 0, 0, 0);
}

fn case_wrappers() {
    let mut e = silk_encoder_state_FLP::default();
    let mut PredCoef = [[0.0f32; 16]; 2];
    let mut NLSF_Q15 = [0i16; 16];
    let mut prev = [0i16; 16];

    ph("a2nlsf_flp");
    for c in 0..4 {
        let mut a = [0.0f32; 16];
        let mut nlsf = [0i16; 16];
        let mut back = [0.0f32; 16];
        let order = if c % 2 == 0 { 16 } else { 10 };
        rng_seed(0x17000 + c as u32);
        for i in 0..order {
            a[i as usize] = rnd_small() * (if c == 3 { 0.99 } else { 0.5 });
        }
        silk_A2NLSF_FLP(&mut nlsf, &a, order);
        silk_NLSF2A_FLP(&mut back, &nlsf, order, 4);
        println!("order={}", order);
        pfa("  a", &a, order as usize);
        pi16a("  nlsf", &nlsf, order as usize);
        pfa("  back", &back, order as usize);
    }

    ph("process_nlsfs_flp");
    for c in 0..2 {
        setup_encoder(&mut e, if c == 0 { 16000 } else { 8000 }, 20, 20000, 9, 0, 0, 0);
        let order = e.sCmn.predictLPCOrder;
        rng_seed(0x17100 + c as u32);
        e.sCmn.useInterpolatedNLSFs = 1;
        e.sCmn.indices.NLSFInterpCoef_Q2 = 4;
        e.sCmn.indices.signalType = TYPE_VOICED as i8;
        e.sCmn.speech_activity_Q8 = 128;
        for i in 0..order {
            NLSF_Q15[i as usize] = ((i + 1) * (32768 / (order + 1))) as i16;
            prev[i as usize] = ((i + 1) * (32768 / (order + 1))) as i16;
        }
        for i in 0..2 {
            for j in 0..16 {
                PredCoef[i][j] = 0.0;
            }
        }
        silk_process_NLSFs_FLP(&mut e.sCmn, &mut PredCoef, &mut NLSF_Q15, &prev);
        println!("order={} fs_kHz={}", order, e.sCmn.fs_kHz);
        pi16a("  nlsf", &NLSF_Q15, order as usize);
        pfa("  pred0", &PredCoef[0], order as usize);
        pfa("  pred1", &PredCoef[1], order as usize);
        pi8a("  nlsfIdx", &e.sCmn.indices.NLSFIndices, (order + 1) as usize);
    }

    ph("quant_ltp_gains_flp");
    for c in 0..2 {
        let mut xx = [0.0f32; 100];
        let mut xxv = [0.0f32; 20];
        let mut b = [0.0f32; 20];
        let mut cbk_index = [0i8; 4];
        let mut periodicity_index = 0i8;
        let mut sum_log_gain = 0i32;
        let mut pred_gain = 0.0f32;
        let nb = if c == 0 { 4 } else { 2 };
        let mut fl_r = [0.0f32; 640];
        let fl_lag = [30i32, 43, 56, 69];
        setup_encoder(&mut e, 16000, 20, 20000, 9, 0, 0, 0);
        e.sCmn.nb_subfr = nb;
        let subfr = e.sCmn.subfr_length;
        rng_seed(0x17200 + c as u32);
        gen_speech(&mut fl_r, 2 * 320, 16);
        silk_find_LTP_FLP(&mut xx, &mut xxv, &fl_r, 160, &fl_lag, subfr, nb, 4);
        silk_quant_LTP_gains_FLP(
            &mut b,
            &mut cbk_index,
            &mut periodicity_index,
            &mut sum_log_gain,
            &mut pred_gain,
            &xx,
            &xxv,
            subfr,
            nb,
            4,
        );
        println!("nb={} subfr={}", nb, subfr);
        pfa("  XX", &xx, (nb * 25) as usize);
        pfa("  xX", &xxv, (nb * 5) as usize);
        pfa("  B", &b, (nb * 5) as usize);
        pi8a("  cbk", &cbk_index, nb as usize);
        pi("  perIndex", periodicity_index as i64);
        pi("  sumLogGain", sum_log_gain as i64);
        pf("  predGain", pred_gain);
    }
}

/* ------------------------------------------------------------------ */
/* 24. find_LPC_FLP / find_pred_coefs_FLP                              */
/* ------------------------------------------------------------------ */

fn case_find_lpc_pred() {
    let mut e = silk_encoder_state_FLP::default();
    let mut x = [0.0f32; 640];
    let mut res_pitch = [0.0f32; 672];
    let mut ctrl = silk_encoder_control_FLP::default();
    let mut NLSF_Q15 = [0i16; 16];

    ph("find_lpc_flp");
    for c in 0..2 {
        setup_encoder(&mut e, 16000, 20, 20000, 9, 0, 0, 0);
        e.sCmn.useInterpolatedNLSFs = 1;
        e.sCmn.first_frame_after_reset = 0;
        e.sCmn.nb_subfr = if c == 0 { 4 } else { 2 };
        e.sCmn.frame_length = e.sCmn.nb_subfr * e.sCmn.subfr_length;
        rng_seed(0x17300 + c as u32);
        gen_speech(&mut x, 640, 16);
        for i in 0..16 {
            e.sCmn.prev_NLSFq_Q15[i] = ((i + 1) * (32768 / 17)) as i16;
        }
        for i in 0..16 {
            NLSF_Q15[i] = 0;
        }
        silk_find_LPC_FLP(&mut e.sCmn, &mut NLSF_Q15, &x, 0.01, 4);
        println!("nb_subfr={}", e.sCmn.nb_subfr);
        pi16a("  nlsf", &NLSF_Q15, e.sCmn.predictLPCOrder as usize);
        pi("  interp", e.sCmn.indices.NLSFInterpCoef_Q2 as i64);
    }

    ph("find_pred_coefs_flp");
    for c in 0..3 {
        setup_encoder(&mut e, 16000, 20, 20000, 9, 0, 0, 0);
        ctrl = silk_encoder_control_FLP::default();
        rng_seed(0x17400 + c as u32);
        gen_speech(&mut x, 640, 16);
        gen_speech(&mut res_pitch[320..], 320, 16);
        e.sCmn.indices.signalType = if c == 0 {
            TYPE_VOICED as i8
        } else {
            TYPE_UNVOICED as i8
        };
        e.sCmn.first_frame_after_reset = if c == 2 { 1 } else { 0 };
        e.sCmn.useInterpolatedNLSFs = 1;
        for i in 0..4 {
            ctrl.pitchL[i] = 40 + i as i32 * 9;
        }
        for i in 0..4 {
            ctrl.Gains[i] = 10.0 + i as f32;
        }
        ctrl.coding_quality = 0.8;
        ctrl.input_quality = 0.7;
        ctrl.LTPredCodGain = 10.0;
        for i in 0..16 {
            e.sCmn.prev_NLSFq_Q15[i] = ((i + 1) * (32768 / 17)) as i16;
        }
        /* `x` is the `psEnc->x_buf` base: the frame starts at ltp_mem_length */
        silk_find_pred_coefs_FLP(&mut e, &mut ctrl, &res_pitch, &x, 2);
        println!("c={} signalType={}", c, e.sCmn.indices.signalType);
        pfa("  LTPCoef", &ctrl.LTPCoef, (e.sCmn.nb_subfr * LTP_ORDER) as usize);
        pfa("  PredCoef0", &ctrl.PredCoef[0], e.sCmn.predictLPCOrder as usize);
        pfa("  PredCoef1", &ctrl.PredCoef[1], e.sCmn.predictLPCOrder as usize);
        pfa("  ResNrg", &ctrl.ResNrg, e.sCmn.nb_subfr as usize);
        pf("  LTPredCodGain", ctrl.LTPredCodGain);
        pf("  LTP_scale", ctrl.LTP_scale);
        pi8a("  LTPIndex", &e.sCmn.indices.LTPIndex, e.sCmn.nb_subfr as usize);
        pi("  PERIndex", e.sCmn.indices.PERIndex as i64);
        pi("  NLSFInterpCoef_Q2", e.sCmn.indices.NLSFInterpCoef_Q2 as i64);
        pi("  sum_log_gain_Q7", e.sCmn.sum_log_gain_Q7 as i64);
        pi16a("  prevNLSF", &e.sCmn.prev_NLSFq_Q15, e.sCmn.predictLPCOrder as usize);
    }
}

/* ------------------------------------------------------------------ */
/* 25. silk_encode_do_VAD_FLP / silk_encode_frame_FLP                  */
/* ------------------------------------------------------------------ */

struct EncCfg {
    api_fs: i32,
    payload_ms: i32,
    bitrate: i32,
    complexity: i32,
    loss: i32,
    fec: i32,
    cbr: i32,
    n_packets: i32,
    silent_last: i32,
}

/* NOTE: `silk/control_codec.c` gives nStatesDelayedDecision >= 3 for
 * complexity >= 6, which selects `silk_NSQ_del_dec_avx2`.  The Rust AVX2 port
 * currently diverges from the C in its *final state write-back* (the emitted
 * pulses match, but sLPC_Q14/sAR2_Q14/sLF_AR_shp_Q14/sDiff_shp_Q14 do not —
 * see the `case_pred_chain` NSQ test below); those configurations are therefore
 * covered by that finding rather than by this probe. */
const ENC_CFGS: [EncCfg; 7] = [
    EncCfg { api_fs: 16000, payload_ms: 20, bitrate: 20000, complexity: 2, loss: 0, fec: 0, cbr: 0, n_packets: 3, silent_last: 0 },
    EncCfg { api_fs: 16000, payload_ms: 20, bitrate: 12000, complexity: 2, loss: 20, fec: 1, cbr: 0, n_packets: 3, silent_last: 0 },
    EncCfg { api_fs: 16000, payload_ms: 40, bitrate: 24000, complexity: 8, loss: 0, fec: 0, cbr: 1, n_packets: 2, silent_last: 0 },
    EncCfg { api_fs: 16000, payload_ms: 40, bitrate: 24000, complexity: 3, loss: 0, fec: 0, cbr: 1, n_packets: 2, silent_last: 0 },
    EncCfg { api_fs: 16000, payload_ms: 10, bitrate: 16000, complexity: 10, loss: 0, fec: 0, cbr: 0, n_packets: 3, silent_last: 0 },
    EncCfg { api_fs: 16000, payload_ms: 10, bitrate: 16000, complexity: 2, loss: 0, fec: 0, cbr: 0, n_packets: 3, silent_last: 0 },
    EncCfg { api_fs: 8000, payload_ms: 20, bitrate: 10000, complexity: 3, loss: 0, fec: 0, cbr: 0, n_packets: 3, silent_last: 1 },
];

fn case_encode_frame() {
    let mut e = silk_encoder_state_FLP::default();

    ph("encode_frame_flp");
    for c in 0..ENC_CFGS.len() {
        let cfg = &ENC_CFGS[c];
        setup_encoder(
            &mut e,
            cfg.api_fs,
            cfg.payload_ms,
            cfg.bitrate,
            cfg.complexity,
            cfg.loss,
            cfg.fec,
            cfg.cbr,
        );
        if !CHECK_AVX2_STATE && e.sCmn.nStatesDelayedDecision >= 3 {
            continue; /* see the module note */
        }
        println!(
            "cfg={} fs_kHz={} nb_subfr={} nFramesPerPacket={} nStates={}",
            c, e.sCmn.fs_kHz, e.sCmn.nb_subfr, e.sCmn.nFramesPerPacket, e.sCmn.nStatesDelayedDecision
        );
        let packet_bits = cfg.bitrate * cfg.payload_ms / 1000;

        for p in 0..cfg.n_packets {
            let mut rng = ec_enc_init(4096);
            e.sCmn.nFramesEncoded = 0;
            for f in 0..e.sCmn.nFramesPerPacket {
                let condCoding = if f == 0 { CODE_INDEPENDENTLY } else { CODE_CONDITIONALLY };
                let activity = VAD_ACTIVITY;
                let fl = e.sCmn.frame_length;

                rng_seed((0x18000 + c * 256 + (p * 16) as usize + f as usize) as u32);
                if cfg.silent_last != 0 && p == cfg.n_packets - 1 {
                    for i in 0..fl {
                        e.sCmn.inputBuf[1 + i as usize] = 0;
                    }
                } else {
                    let mut tmp = [0.0f32; 640];
                    gen_speech(&mut tmp, fl as usize, e.sCmn.fs_kHz);
                    for i in 0..fl {
                        e.sCmn.inputBuf[1 + i as usize] = silk_float2int(tmp[i as usize]) as i16;
                    }
                }
                e.sCmn.inputBuf[0] = 0;
                silk_encode_do_VAD_FLP(&mut e, activity);
                pi("  vad", e.sCmn.speech_activity_Q8 as i64);
                pi("  sigType", e.sCmn.indices.signalType as i64);
                pi("  noSpeech", e.sCmn.noSpeechCounter as i64);
                pi("  inDTX", e.sCmn.inDTX as i64);
                let nfe = e.sCmn.nFramesEncoded as usize;
                pi("  vadFlag", e.sCmn.VAD_flags[nfe] as i64);
                pi("  SNR_dB_Q7", e.sCmn.SNR_dB_Q7 as i64);
                pi("  input_tilt_Q15", e.sCmn.input_tilt_Q15 as i64);
                pi("  iq0", e.sCmn.input_quality_bands_Q15[0] as i64);
                pi("  iq1", e.sCmn.input_quality_bands_Q15[1] as i64);
                pi("  iq2", e.sCmn.input_quality_bands_Q15[2] as i64);
                pi("  iq3", e.sCmn.input_quality_bands_Q15[3] as i64);

                let mut nBytes: i32 = 0;
                pi("  fcIn", e.sCmn.frameCounter as i64);
                silk_encode_frame_FLP(
                    &mut e,
                    &mut nBytes,
                    &mut rng,
                    condCoding,
                    packet_bits,
                    cfg.cbr,
                );
                println!("  p={} f={}", p, f);
                pi("    nBytes", nBytes as i64);
                pi("    Seed", e.sCmn.indices.Seed as i64);
                pi("    GainsIndices0", e.sCmn.indices.GainsIndices[0] as i64);
                pi("    GainsIndices1", e.sCmn.indices.GainsIndices[1] as i64);
                pi("    lagIndex", e.sCmn.indices.lagIndex as i64);
                pi("    contourIndex", e.sCmn.indices.contourIndex as i64);
                pi("    quantOffset", e.sCmn.indices.quantOffsetType as i64);
                pi("    nlsfInterp", e.sCmn.indices.NLSFInterpCoef_Q2 as i64);
                pi("    PERIndex", e.sCmn.indices.PERIndex as i64);
                pi("    SignalType", e.sCmn.indices.signalType as i64);
                pf("    HarmShapeGain_smth", e.sShape.HarmShapeGain_smth);
                pi("    prevLag", e.sCmn.prevLag as i64);
                pi("    prevSignalType", e.sCmn.prevSignalType as i64);
                pi("    LBRRprevGain", e.sCmn.LBRRprevLastGainIndex as i64);
                pi("    nsq.rand_seed", e.sCmn.sNSQ.rand_seed as i64);
                pi("    nsq.lagPrev", e.sCmn.sNSQ.lagPrev as i64);
                pi("    nsq.rewhite", e.sCmn.sNSQ.rewhite_flag as i64);
                let nfe = e.sCmn.nFramesEncoded as usize;
                pi("    LBRRflag", e.sCmn.LBRR_flags[nfe] as i64);
                pi("    LBRRenabled", e.sCmn.LBRR_enabled as i64);
                pi("    firstAfterReset", e.sCmn.first_frame_after_reset as i64);
                pi8a("    nlsfIdx", &e.sCmn.indices.NLSFIndices, e.sCmn.predictLPCOrder as usize + 1);
                pi("    LTP_scaleIndex", e.sCmn.indices.LTP_scaleIndex as i64);
                pi("    sNSQ.sLF_AR_shp_Q14", e.sCmn.sNSQ.sLF_AR_shp_Q14 as i64);
                pi("    sNSQ.sDiff_shp_Q14", e.sCmn.sNSQ.sDiff_shp_Q14 as i64);
                pi("    sNSQ.sLTP_buf_idx", e.sCmn.sNSQ.sLTP_buf_idx as i64);
                pi("    sNSQ.sLTP_shp_buf_idx", e.sCmn.sNSQ.sLTP_shp_buf_idx as i64);
                pi("    sNSQ.prev_gain_Q16", e.sCmn.sNSQ.prev_gain_Q16 as i64);
                pi("    sShape.LastGainIndex", e.sShape.LastGainIndex as i64);
                pia("    sLPC_Q14", &e.sCmn.sNSQ.sLPC_Q14, (MAX_SUB_FRAME_LENGTH + NSQ_LPC_BUF_LENGTH) as usize);
                pia("    sAR2_Q14", &e.sCmn.sNSQ.sAR2_Q14, MAX_SHAPE_LPC_ORDER as usize);
                pi8a("    nlsfIdx2", &e.sCmn.indices.NLSFIndices, e.sCmn.predictLPCOrder as usize + 1);
                e.sCmn.nFramesEncoded += 1;
            }
            ec_enc_done(&mut rng);
            pi("  payloadBytes", rng.offs as i64);
            {
                let offs = rng.offs as usize;
                print!("  payload=");
                for k in 0..offs {
                    print!("{:02x}", rng.buf[k]);
                }
                println!();
            }
        }
    }
}


/* ------------------------------------------------------------------ */
/* 26. real encoder chain up to process_gains                          */
/* ------------------------------------------------------------------ */

fn case_pred_chain() {
    let mut e = silk_encoder_state_FLP::default();
    let mut ctrl = silk_encoder_control_FLP::default();
    let mut res_pitch = [0.0f32; 672];

    ph("pred_chain");
    for c in 0..2 {
        setup_encoder(&mut e, 16000, 20, 20000, if c == 0 { 9 } else { 2 }, 0, 0, 0);
        ctrl = silk_encoder_control_FLP::default();
        let ltp = e.sCmn.ltp_mem_length;
        let la_shape = e.sCmn.la_shape;
        let mut pe_xb = [0.0f32; 2 * 320 + 80];
        {
            /* mirror exactly the silk_encode_frame_FLP input path */
            let mut tmp = [0.0f32; 640];
            rng_seed(0x18000);
            gen_speech(&mut tmp, e.sCmn.frame_length as usize, e.sCmn.fs_kHz);
            for i in 0..e.sCmn.frame_length {
                e.sCmn.inputBuf[1 + i as usize] = silk_float2int(tmp[i as usize]) as i16;
            }
            e.sCmn.inputBuf[0] = 0;
            silk_encode_do_VAD_FLP(&mut e, VAD_ACTIVITY);
            silk_LP_variable_cutoff(
                &mut e.sCmn.sLP,
                &mut e.sCmn.inputBuf[1..],
                e.sCmn.frame_length,
            );
            pe_xb = e.x_buf;
            silk_short2float_array(
                &mut pe_xb[(ltp + la_shape) as usize..],
                &e.sCmn.inputBuf[1..],
                e.sCmn.frame_length,
            );
            for i in 0..8 {
                pe_xb[(ltp + la_shape + i * (e.sCmn.frame_length >> 3)) as usize] +=
                    (1 - (i & 2)) as f32 * 1e-6;
            }
        }
        for v in res_pitch.iter_mut() {
            *v = 0.0;
        }
        println!(
            "c={} compl={} shapingOrder={} pitchOrder={} predOrder={} warping={} interp={} nStates={}",
            c, e.sCmn.Complexity, e.sCmn.shapingLPCOrder, e.sCmn.pitchEstimationLPCOrder,
            e.sCmn.predictLPCOrder, e.sCmn.warping_Q16, e.sCmn.useInterpolatedNLSFs,
            e.sCmn.nStatesDelayedDecision
        );

        println!(
            "  inputBuf[1..9]={} {} {} {} {} {} {} {}",
            e.sCmn.inputBuf[1], e.sCmn.inputBuf[2], e.sCmn.inputBuf[3], e.sCmn.inputBuf[4],
            e.sCmn.inputBuf[5], e.sCmn.inputBuf[6], e.sCmn.inputBuf[7], e.sCmn.inputBuf[8]
        );
        pfa("  pe_xb400", &pe_xb[(ltp + la_shape) as usize..], 8);
        {
            let arch = e.sCmn.arch;
            silk_find_pitch_lags_FLP(&mut e, &mut ctrl, &mut res_pitch, &pe_xb, arch);
        }
        println!(" after find_pitch_lags");
        pfa("  res_pitch", &res_pitch[ltp as usize..], e.sCmn.frame_length as usize);
        pf("  predGain", ctrl.predGain);
        pia("  pitchL", &ctrl.pitchL, 4);
        pi("  sigType", e.sCmn.indices.signalType as i64);
        pi("  lagIndex", e.sCmn.indices.lagIndex as i64);
        pi("  contourIndex", e.sCmn.indices.contourIndex as i64);
        pf("  LTPCorr", e.LTPCorr);

        {
            silk_noise_shape_analysis_FLP(
                &mut e,
                &mut ctrl,
                &res_pitch[ltp as usize..],
                &pe_xb[(ltp - la_shape) as usize..],
            );
        }
        println!(" after noise_shape");
        pfa("  AR", &ctrl.AR, (e.sCmn.nb_subfr * MAX_SHAPE_LPC_ORDER) as usize);
        pfa("  Gains", &ctrl.Gains, e.sCmn.nb_subfr as usize);
        pfa("  LF_MA_shp", &ctrl.LF_MA_shp, e.sCmn.nb_subfr as usize);
        pfa("  LF_AR_shp", &ctrl.LF_AR_shp, e.sCmn.nb_subfr as usize);
        pfa("  Tilt", &ctrl.Tilt, e.sCmn.nb_subfr as usize);
        pfa("  HarmShapeGain", &ctrl.HarmShapeGain, e.sCmn.nb_subfr as usize);
        pf("  input_quality", ctrl.input_quality);
        pf("  coding_quality", ctrl.coding_quality);
        pi("  quantOffsetType", e.sCmn.indices.quantOffsetType as i64);

        {
            silk_find_pred_coefs_FLP(
                &mut e,
                &mut ctrl,
                &res_pitch,
                &pe_xb,
                CODE_INDEPENDENTLY,
            );
        }
        println!(" after find_pred_coefs");
        pfa("  LTPCoef", &ctrl.LTPCoef, (e.sCmn.nb_subfr * LTP_ORDER) as usize);
        pfa("  PredCoef0", &ctrl.PredCoef[0], e.sCmn.predictLPCOrder as usize);
        pfa("  ResNrg", &ctrl.ResNrg, e.sCmn.nb_subfr as usize);
        pf("  LTPredCodGain", ctrl.LTPredCodGain);
        pf("  LTP_scale", ctrl.LTP_scale);
        pi("  LTP_scaleIndex", e.sCmn.indices.LTP_scaleIndex as i64);
        pi8a("  LTPIndex", &e.sCmn.indices.LTPIndex, e.sCmn.nb_subfr as usize);
        pi("  PERIndex", e.sCmn.indices.PERIndex as i64);
        pi("  NLSFInterpCoef_Q2", e.sCmn.indices.NLSFInterpCoef_Q2 as i64);
        pi8a("  NLSFIndices", &e.sCmn.indices.NLSFIndices, e.sCmn.predictLPCOrder as usize + 1);
        pi16a("  prevNLSF", &e.sCmn.prev_NLSFq_Q15, e.sCmn.predictLPCOrder as usize);

        silk_process_gains_FLP(&mut e, &mut ctrl, CODE_INDEPENDENTLY);
        println!(" after process_gains");
        pfa("  Gains", &ctrl.Gains, e.sCmn.nb_subfr as usize);
        pia("  GainsUnq_Q16", &ctrl.GainsUnq_Q16, e.sCmn.nb_subfr as usize);
        pi8a("  GainsIndices", &e.sCmn.indices.GainsIndices, e.sCmn.nb_subfr as usize);
        pf("  Lambda", ctrl.Lambda);
        pi("  lastGainIndexPrev", ctrl.lastGainIndexPrev as i64);
        pi("  LastGainIndex", e.sShape.LastGainIndex as i64);

        /* direct NSQ wrapper test (the non-AVX2 kernel; see the module note) */
        if CHECK_AVX2_STATE || e.sCmn.nStatesDelayedDecision < 3 {
            let mut indices = e.sCmn.indices;
            let mut nsq = e.sCmn.sNSQ;
            let mut pulses = e.sCmn.pulses;
            silk_NSQ_wrapper_FLP(
                &e.sCmn,
                &ctrl,
                &mut indices,
                &mut nsq,
                &mut pulses,
                &pe_xb[ltp as usize..],
            );
            e.sCmn.indices = indices;
            e.sCmn.sNSQ = nsq;
            e.sCmn.pulses = pulses;

        println!(" after NSQ");
        pi8a("  pulses", &e.sCmn.pulses, e.sCmn.frame_length as usize);
        pi("  Seed", e.sCmn.indices.Seed as i64);
        pi("  sNSQ.lagPrev", e.sCmn.sNSQ.lagPrev as i64);
        pi("  sNSQ.rand_seed", e.sCmn.sNSQ.rand_seed as i64);
        pi("  sNSQ.sLF_AR_shp_Q14", e.sCmn.sNSQ.sLF_AR_shp_Q14 as i64);
        pi("  sNSQ.sDiff_shp_Q14", e.sCmn.sNSQ.sDiff_shp_Q14 as i64);
        pi("  sNSQ.sLTP_buf_idx", e.sCmn.sNSQ.sLTP_buf_idx as i64);
        pi("  sNSQ.sLTP_shp_buf_idx", e.sCmn.sNSQ.sLTP_shp_buf_idx as i64);
        pi("  sNSQ.prev_gain_Q16", e.sCmn.sNSQ.prev_gain_Q16 as i64);
        pia("  sLPC_Q14", &e.sCmn.sNSQ.sLPC_Q14, (MAX_SUB_FRAME_LENGTH + NSQ_LPC_BUF_LENGTH) as usize);
        pia("  sAR2_Q14", &e.sCmn.sNSQ.sAR2_Q14, MAX_SHAPE_LPC_ORDER as usize);
        }
    }
}


fn main() {
    case_inner_product();
    case_energy();
    case_scale_vectors();
    case_bwexpander();
    case_sine_window();
    case_sort();
    case_k2a_schur();
    case_autocorrelation();
    case_warped_autocorrelation();
    case_lpc_inv_pred_gain();
    case_lpc_analysis_filter();
    case_ltp_analysis_filter();
    case_residual_energy();
    case_regularize();
    case_corrmatrix();
    case_burg();
    case_find_ltp();
    case_ltp_scale_ctrl();
    case_process_gains();
    case_find_pitch_lags();
    case_pitch_core();
    case_noise_shape();
    case_wrappers();
    case_find_lpc_pred();
    case_encode_frame();
    case_pred_chain();
    println!("done");
}

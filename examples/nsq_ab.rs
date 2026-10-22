//! Per-call A/B of the two `silk_NSQ_del_dec` bodies in this crate.
//!
//! `src/silk/nsq_del_dec.rs` carries three implementations of the
//! delayed-decision noise shaper:
//!
//! * `silk_NSQ_del_dec_c` — the bit-exact C body: the **default** build's only
//!   reachable body, and the fallback of the `fast` build,
//! * `silk_NSQ_del_dec_avx2` — the arch-4 body: AVX2 intrinsics when the host
//!   has AVX2 (run-time `have_avx2()`, in every build), the lane-vector
//!   emulation otherwise (the emulation is the executable specification of the
//!   intrinsics; it is ~3.9x slower than the C body and is only a
//!   non-AVX2-host fallback),
//! * `silk_NSQ_del_dec` — the dispatcher that chooses between them.
//!
//! This harness times the C body against the AVX2 body **on identical input**,
//! alternating the two call by call so host drift cancels, and reports the
//! per-call nanoseconds (min of N repetitions, plus the mean).  Every call starts from a byte-identical `silk_nsq_state` /
//! `SideInfoIndices` snapshot, so the timed region is exactly one NSQ call.
//!
//! It also reports
//!
//! * whether the two bodies produced identical output (`pulses`, quantised
//!   speech `xq`, the LPC/shaping states and the noise seed) on this input set,
//! * what the **dispatcher** costs for `arch = 4` — i.e. what the encoder
//!   actually pays end to end — and which body it selects in this build.
//!
//! ```text
//! cargo run --release --example nsq_ab                    # default build: C body vs arch-4 body
//! cargo run --release --features fast --example nsq_ab    # fast build: same NSQ bodies
//! ```
//!
//! No dependencies.  The inputs are generated deterministically (same generator
//! as `probe_silkenc`) and no encoder is run, so this is independent of the
//! codec's front end.

#![allow(non_snake_case)]

use oprs::silk::control::silk_EncControlStruct;
use oprs::silk::control_codec::silk_control_encoder;
use oprs::silk::define::*;
use oprs::silk::float::main_flp::silk_encoder_state_FLP;
use oprs::silk::init_encoder::silk_init_encoder;
use oprs::silk::nsq_del_dec::*;
use oprs::silk::sigproc_fix::SILK_FIX_CONST;
use oprs::silk::structs::{SideInfoIndices, silk_encoder_state, silk_nsq_state};
use oprs::silk::typedef::*;
use oprs::types::*;
use std::time::Instant;

/* ------------------------------------------------------------------ */
/* Deterministic input set (same LCG as the probes)                    */
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

fn rnd_range(lo: i32, hi: i32) -> i32 {
    lo + ((rnd() as u32) % ((hi - lo + 1) as u32)) as i32
}

/* ------------------------------------------------------------------ */
/* The inputs of one NSQ call                                          */
/* ------------------------------------------------------------------ */

/// Signature shared by `silk_NSQ_del_dec_c` and `silk_NSQ_del_dec_avx2`.
type Body = fn(
    &silk_encoder_state,
    &mut silk_nsq_state,
    &mut SideInfoIndices,
    &[opus_int16],
    &mut [opus_int8],
    &[[opus_int16; MAX_LPC_ORDER as usize]; 2],
    &[opus_int16],
    &[opus_int16],
    &[opus_int],
    &[opus_int],
    &[opus_int32],
    &[opus_int32],
    &[opus_int],
    opus_int,
    opus_int,
);

/// One case's inputs plus the per-call mutable state that has to be restored
/// before every timed call.
struct Call {
    nsq: silk_nsq_state,
    indices: SideInfoIndices,
    x16: Vec<opus_int16>,
    pred_coef_q12: [[opus_int16; MAX_LPC_ORDER as usize]; 2],
    ltp_coef_q14: Vec<opus_int16>,
    ar_q13: Vec<opus_int16>,
    harm_shape: [opus_int; MAX_NB_SUBFR as usize],
    tilt: [opus_int; MAX_NB_SUBFR as usize],
    lf_shp: [opus_int32; MAX_NB_SUBFR as usize],
    gains: [opus_int32; MAX_NB_SUBFR as usize],
    pitch_l: [opus_int; MAX_NB_SUBFR as usize],
    lambda_q10: opus_int,
    ltp_scale_q14: opus_int,
}

/// What one NSQ call produced (everything the caller sees).
struct Out {
    pulses: Vec<opus_int8>,
    nsq: silk_nsq_state,
    indices: SideInfoIndices,
}

impl Out {
    /// Do two NSQ calls agree on every output the encoder can observe?
    fn same_as(&self, o: &Out) -> bool {
        self.pulses == o.pulses
            && self.nsq.xq == o.nsq.xq
            && self.nsq.sLTP_shp_Q14 == o.nsq.sLTP_shp_Q14
            && self.nsq.sLPC_Q14 == o.nsq.sLPC_Q14
            && self.nsq.sAR2_Q14 == o.nsq.sAR2_Q14
            && self.nsq.sLF_AR_shp_Q14 == o.nsq.sLF_AR_shp_Q14
            && self.nsq.sDiff_shp_Q14 == o.nsq.sDiff_shp_Q14
            && self.indices.Seed == o.indices.Seed
    }
}

impl Call {
    /// One call of body `f` from the pristine snapshot.
    fn run(&self, enc: &silk_encoder_state_FLP, f: Body) -> Out {
        let mut nsq = self.nsq;
        let mut idx = self.indices;
        let mut pulses = vec![0 as opus_int8; self.x16.len()];
        f(
            &enc.sCmn,
            &mut nsq,
            &mut idx,
            &self.x16,
            &mut pulses,
            &self.pred_coef_q12,
            &self.ltp_coef_q14,
            &self.ar_q13,
            &self.harm_shape,
            &self.tilt,
            &self.lf_shp,
            &self.gains,
            &self.pitch_l,
            self.lambda_q10,
            self.ltp_scale_q14,
        );
        Out { pulses, nsq, indices: idx }
    }

    /// One call of the dispatched entry (`arch = 4`, as the encoder calls it).
    fn run_dispatch(&self, enc: &silk_encoder_state_FLP, arch: opus_int) -> Out {
        let mut nsq = self.nsq;
        let mut idx = self.indices;
        let mut pulses = vec![0 as opus_int8; self.x16.len()];
        silk_NSQ_del_dec(
            &enc.sCmn,
            &mut nsq,
            &mut idx,
            &self.x16,
            &mut pulses,
            &self.pred_coef_q12,
            &self.ltp_coef_q14,
            &self.ar_q13,
            &self.harm_shape,
            &self.tilt,
            &self.lf_shp,
            &self.gains,
            &self.pitch_l,
            self.lambda_q10,
            self.ltp_scale_q14,
            arch,
        );
        Out { pulses, nsq, indices: idx }
    }
}

/* ------------------------------------------------------------------ */

fn setup_encoder(e: &mut silk_encoder_state_FLP, fs_khz: i32) {
    let mut ctl = silk_EncControlStruct::default();
    ctl.nChannelsAPI = 1;
    ctl.nChannelsInternal = 1;
    ctl.API_sampleRate = 48000;
    ctl.maxInternalSampleRate = 16000;
    ctl.minInternalSampleRate = 8000;
    ctl.desiredInternalSampleRate = fs_khz * 1000;
    ctl.payloadSize_ms = 20;
    ctl.bitRate = 32000;
    ctl.complexity = 9;
    *e = silk_encoder_state_FLP::default();
    silk_init_encoder(e, 4);
    silk_control_encoder(e, &mut ctl, 0, 0, 0);
}

struct Case {
    fs_khz: i32,
    n_states: i32,
    voiced: bool,
    warping: bool,
    quant_offset: i32,
}

/// Covers what the encoder actually reaches: `nStatesDelayedDecision` 3 and 4,
/// voiced/unvoiced, warping on/off, and all three SILK internal rates.
const CASES: [Case; 8] = [
    Case { fs_khz: 16, n_states: 4, voiced: true, warping: true, quant_offset: 0 },
    Case { fs_khz: 16, n_states: 4, voiced: false, warping: false, quant_offset: 1 },
    Case { fs_khz: 16, n_states: 3, voiced: true, warping: false, quant_offset: 1 },
    Case { fs_khz: 16, n_states: 3, voiced: false, warping: true, quant_offset: 0 },
    Case { fs_khz: 12, n_states: 4, voiced: true, warping: true, quant_offset: 0 },
    Case { fs_khz: 12, n_states: 3, voiced: false, warping: false, quant_offset: 1 },
    Case { fs_khz: 8, n_states: 4, voiced: true, warping: false, quant_offset: 1 },
    Case { fs_khz: 8, n_states: 3, voiced: false, warping: true, quant_offset: 0 },
];

fn make_case(c: &Case, cas: usize) -> (silk_encoder_state_FLP, Call) {
    let mut enc = silk_encoder_state_FLP::default();
    setup_encoder(&mut enc, c.fs_khz);
    enc.sCmn.nStatesDelayedDecision = c.n_states;
    enc.sCmn.shapingLPCOrder = if c.n_states == 4 { 24 } else { 20 };
    enc.sCmn.useInterpolatedNLSFs = 1;
    enc.sCmn.warping_Q16 = if c.warping {
        enc.sCmn.fs_kHz * SILK_FIX_CONST(0.015, 16)
    } else {
        0
    };

    let frame = enc.sCmn.frame_length as usize;
    let mut call = Call {
        nsq: enc.sCmn.sNSQ,
        indices: SideInfoIndices::default(),
        x16: vec![0; frame],
        pred_coef_q12: [[0; MAX_LPC_ORDER as usize]; 2],
        ltp_coef_q14: vec![0; (LTP_ORDER * MAX_NB_SUBFR) as usize],
        ar_q13: vec![0; (MAX_NB_SUBFR * MAX_SHAPE_LPC_ORDER) as usize],
        harm_shape: [0; MAX_NB_SUBFR as usize],
        tilt: [0; MAX_NB_SUBFR as usize],
        lf_shp: [0; MAX_NB_SUBFR as usize],
        gains: [0; MAX_NB_SUBFR as usize],
        pitch_l: [0; MAX_NB_SUBFR as usize],
        lambda_q10: if (cas & 1) != 0 { 3000 } else { 1024 },
        ltp_scale_q14: 15565,
    };
    call.nsq.lagPrev = 100;
    call.nsq.prev_gain_Q16 = 65536;
    call.indices.signalType = if c.voiced {
        TYPE_VOICED as i8
    } else {
        TYPE_NO_VOICE_ACTIVITY as i8
    };
    call.indices.quantOffsetType = c.quant_offset as i8;
    call.indices.Seed = (cas & 3) as i8;
    /* interp == 4 -> LSF_interpolation_flag == 0 -> only subframe 0 re-whitens */
    call.indices.NLSFInterpCoef_Q2 = if (cas & 1) != 0 { 2 } else { 4 };

    rng_seed(42000 + cas as u32);
    for i in 0..frame {
        call.x16[i] = rnd_range(-8000, 8000) as i16;
    }
    for i in 0..(2 * MAX_LPC_ORDER) as usize {
        call.pred_coef_q12[i / MAX_LPC_ORDER as usize][i % MAX_LPC_ORDER as usize] =
            rnd_range(-1500, 1500) as i16;
    }
    for i in 0..call.ltp_coef_q14.len() {
        call.ltp_coef_q14[i] = rnd_range(-3000, 3000) as i16;
    }
    for i in 0..call.ar_q13.len() {
        call.ar_q13[i] = rnd_range(-1500, 1500) as i16;
    }
    for i in 0..MAX_NB_SUBFR as usize {
        call.harm_shape[i] = rnd_range(0, 3000);
        call.tilt[i] = rnd_range(-2000, 2000);
        call.lf_shp[i] = rnd_range(0, 30000);
        call.gains[i] = rnd_range(65536, 400000);
        /* constant per case: every index the kernel reads has been written */
        call.pitch_l[i] = 33 + 7 * cas as i32;
    }
    (enc, call)
}

/// `true` when the AVX2 entry executes the intrinsics: that is the x86-64
/// run-time `have_avx2()` check inside `silk_noise_shape_quantizer_del_dec_avx2`,
/// in *every* build (the arch-4 dispatch is not a `fast`-only kernel).  Only a
/// host without AVX2 falls back to the lane-vector emulation.
fn intrinsics_live() -> bool {
    cfg!(target_arch = "x86_64") && std::arch::is_x86_feature_detected!("avx2")
}

fn label(c: &Case) -> String {
    format!(
        "{}k/{}st/{}{}",
        c.fs_khz,
        c.n_states,
        if c.voiced { "voiced" } else { "unvoiced" },
        if c.warping { "/warp" } else { "" }
    )
}

fn main() {
    let fast = cfg!(feature = "fast");
    println!(
        "nsq_ab — build: {}",
        if fast {
            "fast (--features fast)"
        } else {
            "default (bit-exact C body only)"
        }
    );
    println!(
        "body B (silk_NSQ_del_dec_avx2): {}",
        if intrinsics_live() {
            "AVX2 intrinsics"
        } else {
            "lane-vector emulation (no intrinsics in this build)"
        }
    );
    println!(
        "dispatcher silk_NSQ_del_dec(arch=4) selects: {}",
        if intrinsics_live() {
            "the AVX2 body"
        } else {
            "the C body"
        }
    );
    println!();

    /* Alternating per-call timing, c, a, dispatch, c, a, dispatch, ... so the
       host's drift hits all three equally.  Each timed region is exactly one
       NSQ call. */
    const WARMUP: usize = 5;
    const ITERS: usize = 300;

    let c_body: Body = silk_NSQ_del_dec_c;
    let b_body: Body = silk_NSQ_del_dec_avx2;

    println!(
        "{:<28} {:>9} {:>9} {:>7} {:>9} {:>11}   B vs C",
        "case", "C min", "B min", "C/B", "disp", "disp/B"
    );
    let mut body_ratios = Vec::new();
    let mut disp_ratios = Vec::new();
    for (cas, c) in CASES.iter().enumerate() {
        let (enc, call) = make_case(c, cas);

        for _ in 0..WARMUP {
            let _ = call.run(&enc, c_body);
            let _ = call.run(&enc, b_body);
            let _ = call.run_dispatch(&enc, 4);
        }

        let out_c = call.run(&enc, c_body);
        let out_b = call.run(&enc, b_body);
        let out_d = call.run_dispatch(&enc, 4);
        let same = out_c.same_as(&out_b);
        /* The dispatcher must reproduce whichever body it claims to select. */
        let disp_matches_b = out_d.same_as(&out_b);
        let disp_matches_c = out_d.same_as(&out_c);

        /* `min` is the robust estimator on a shared host: the fastest
           repetition is the one with the least interference, while a mean is
           pulled around by the scheduler.  The mean is printed too. */
        let mut t_c = 0u128;
        let mut t_b = 0u128;
        let mut t_d = 0u128;
        let (mut min_c, mut min_b, mut min_d) = (u128::MAX, u128::MAX, u128::MAX);
        for _ in 0..ITERS {
            let t = Instant::now();
            let _ = call.run(&enc, c_body);
            let d = t.elapsed().as_nanos();
            t_c += d;
            min_c = min_c.min(d);

            let t = Instant::now();
            let _ = call.run(&enc, b_body);
            let d = t.elapsed().as_nanos();
            t_b += d;
            min_b = min_b.min(d);

            let t = Instant::now();
            let _ = call.run_dispatch(&enc, 4);
            let d = t.elapsed().as_nanos();
            t_d += d;
            min_d = min_d.min(d);
        }
        let [c_ns, b_ns, d_ns] = [t_c, t_b, t_d].map(|t| t as f64 / ITERS as f64);
        let [c_min, b_min, d_min] = [min_c, min_b, min_d].map(|t| t as f64);
        body_ratios.push(c_min / b_min);
        disp_ratios.push(d_min / b_min);

        println!(
            "{:<28} {:>9.1} {:>9.1} {:>7.2} {:>9.1} {:>11.2}   {}  (mean {:>8.1}/{:>8.1}/{:>8.1})",
            label(c),
            c_min,
            b_min,
            c_min / b_min,
            d_min,
            d_min / b_min,
            if same {
                "bit-identical"
            } else {
                "pulses/state DIFFER"
            },
            c_ns,
            b_ns,
            d_ns,
        );
        assert!(
            disp_matches_b || disp_matches_c,
            "{label}: dispatcher output matches neither body",
            label = label(c)
        );
    }

    let geomean = |v: &[f64]| (v.iter().map(|r| r.ln()).sum::<f64>() / v.len() as f64).exp();
    let join = |v: &[f64]| {
        v.iter()
            .map(|r| format!("{r:.2}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    println!();
    println!(
        "C/B per case:               {}",
        join(&body_ratios)
    );
    println!(
        "C/B geomean over {} cases:  {:.2}x   (>1: the AVX2 body is faster)",
        body_ratios.len(),
        geomean(&body_ratios)
    );
    println!(
        "dispatch/body-B per case:   {}",
        join(&disp_ratios)
    );
    println!(
        "dispatch overhead geomean:  {:.2}x   (1.00 = the dispatcher is free; >1 includes the\n\
         \x20                           arch-4 branch plus the run-time `have_avx2()` load)",
        geomean(&disp_ratios)
    );
    println!();
    println!(
        "note: this build's `silk_NSQ_del_dec` routes arch 4 to the {} body, so the third\n\
         \x20     timing column is what the encoder actually pays for NSQ.",
        if intrinsics_live() { "AVX2" } else { "C" }
    );
}

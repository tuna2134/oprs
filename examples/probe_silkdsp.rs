//! Rust side of the SILK decoder-DSP bit-exactness probe.
//!
//! Mirrors `compat/c/probe_silkdsp.c` line for line.  Run
//! `bash compat/scripts/run-probe.sh silkdsp` to build both sides and diff
//! their output.

#![allow(non_snake_case)]

use oprs::silk::ana_filt_bank_1::*;
use oprs::silk::biquad_alt::*;
use oprs::silk::bwexpander::*;
use oprs::silk::bwexpander_32::*;
use oprs::silk::code_signs::*;
use oprs::silk::decode_pitch::*;
use oprs::silk::define::*;
use oprs::silk::float::main_flp::*;
use oprs::silk::gain_quant::*;
use oprs::silk::hp_variable_cutoff::*;
use oprs::silk::inner_prod_aligned::*;
use oprs::silk::interpolate::*;
use oprs::silk::lin2log::*;
use oprs::silk::log2lin::*;
use oprs::silk::lp_variable_cutoff::*;
use oprs::silk::lpc_fit::*;
use oprs::silk::lpc_inv_pred_gain::*;
use oprs::silk::nlsf2a::*;
use oprs::silk::nlsf_decode::*;
use oprs::silk::nlsf_stabilize::*;
use oprs::silk::nlsf_unpack::*;
use oprs::silk::resampler::*;
use oprs::silk::resampler_down2::*;
use oprs::silk::resampler_down2_3::*;
use oprs::silk::resampler_private_ar2::*;
use oprs::silk::resampler_private_down_fir::*;
use oprs::silk::resampler_private_iir_fir::*;
use oprs::silk::resampler_private_up2_hq::*;
use oprs::silk::resampler_rom::*;
use oprs::silk::shell_coder::*;
use oprs::silk::sigm_q15::*;
use oprs::silk::sort::*;
use oprs::silk::stereo_decode_pred::*;
use oprs::silk::stereo_ms_to_lr::*;
use oprs::silk::structs::*;
use oprs::silk::sum_sqr_shift::*;
use oprs::silk::tables_nlsf_cb_nb_mb::silk_NLSF_CB_NB_MB;
use oprs::silk::tables_nlsf_cb_wb::silk_NLSF_CB_WB;

use oprs::celt::entcode::ec_ctx;
use oprs::celt::entdec::ec_dec_init;
use oprs::celt::entenc::{ec_enc_done, ec_enc_init};

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
fn rnd_i16() -> i16 {
    rnd() as i16
}

/* ------------------------------------------------------------------ */
/* Print helpers                                                       */
/* ------------------------------------------------------------------ */

fn vals32(a: &[i32]) {
    for (i, &v) in a.iter().enumerate() {
        if i + 1 < a.len() {
            print!("{}, ", v);
        } else {
            print!("{}", v);
        }
    }
}
fn vals16(a: &[i16]) {
    for (i, &v) in a.iter().enumerate() {
        if i + 1 < a.len() {
            print!("{}, ", v);
        } else {
            print!("{}", v);
        }
    }
}
fn pa32(n: &str, a: &[i32]) {
    print!("{}=[", n);
    vals32(a);
    println!("]");
}
fn pa16(n: &str, a: &[i16]) {
    print!("{}=[", n);
    vals16(a);
    println!("]");
}
fn pau8(n: &str, a: &[u8]) {
    print!("{}=[", n);
    for (i, &v) in a.iter().enumerate() {
        if i + 1 < a.len() {
            print!("{}, ", v);
        } else {
            print!("{}", v);
        }
    }
    println!("]");
}

struct Pack(usize);
impl Pack {
    fn new() -> Self {
        Pack(0)
    }
    fn pk(&mut self, n: &str, i: usize, v: i64) {
        print!("{}[{}]={} ", n, i, v);
        self.0 += 1;
        if self.0 % 8 == 0 {
            println!();
        }
    }
    fn done(&mut self) {
        if self.0 % 8 != 0 {
            println!();
        }
        self.0 = 0;
    }
}

const SENT: i16 = 0x7A5A;
fn lead_non_sent(o: &[i16]) -> usize {
    let mut i = 0;
    while i < o.len() && o[i] != SENT {
        i += 1;
    }
    i
}
fn fill_sent(o: &mut [i16]) {
    for v in o.iter_mut() {
        *v = SENT;
    }
}

fn p_ec(tag: &str, e: &ec_ctx) {
    /* `ext` is not initialised by ec_dec_init(), so it is omitted here. */
    println!(
        "{} storage={} end_offs={} end_window={} nend_bits={} nbits_total={} offs={} rng={} val={} rem={} error={}",
        tag, e.storage, e.end_offs, e.end_window, e.nend_bits, e.nbits_total, e.offs, e.rng, e.val,
        e.rem, e.error
    );
}

/* ------------------------------------------------------------------ */
/* 1. bwexpander / bwexpander_32                                       */
/* ------------------------------------------------------------------ */

fn sec_bwexpander() {
    let chirps: [i32; 6] = [-6554, 0, 6554, 32768, 65536, 131072];
    let ds: [i32; 6] = [1, 2, 3, 10, 16, 24];
    let mut ar = [0i16; 24];
    let mut ar32 = [0i32; 24];

    println!("## bwexpander");
    for di in 0..6 {
        let d = ds[di];
        for ci in 0..6 {
            rng_seed(0x10000u32 + (di * 16 + ci) as u32);
            for i in 0..d {
                ar[i as usize] = rnd_i16();
            }
            silk_bwexpander(&mut ar[..d as usize], d, chirps[ci]);
            print!("bwexpander d={} chirp={} ", d, chirps[ci]);
            pa16("ar", &ar[..d as usize]);
        }
    }

    println!("## bwexpander_32");
    for di in 0..6 {
        let d = ds[di];
        for ci in 0..6 {
            rng_seed(0x11000u32 + (di * 16 + ci) as u32);
            for i in 0..d {
                ar32[i as usize] = rnd();
            }
            silk_bwexpander_32(&mut ar32[..d as usize], d, chirps[ci]);
            print!("bwexpander_32 d={} chirp={} ", d, chirps[ci]);
            pa32("ar", &ar32[..d as usize]);
        }
    }
}

/* ------------------------------------------------------------------ */
/* 2. sum_sqr_shift / inner_prod_aligned_scale                         */
/* ------------------------------------------------------------------ */

fn sec_sum_sqr() {
    let lens: [i32; 15] = [1, 2, 3, 4, 5, 7, 8, 15, 16, 17, 32, 33, 64, 100, 320];
    let mut x = [0i16; 320];

    println!("## sum_sqr_shift");
    for li in 0..15 {
        let len = lens[li];
        rng_seed(0x20000u32 + len as u32);
        for i in 0..len {
            x[i as usize] = rnd_i16();
        }
        let (energy, shift) = silk_sum_sqr_shift(&x, len);
        println!(
            "sum_sqr_shift len={} energy={} shift={}",
            len, energy, shift
        );
    }
    for v in x.iter_mut() {
        *v = 0;
    }
    let (energy, shift) = silk_sum_sqr_shift(&x, 16);
    println!("sum_sqr_shift zero len=16 energy={} shift={}", energy, shift);
    for v in x.iter_mut() {
        *v = 32767;
    }
    let (energy, shift) = silk_sum_sqr_shift(&x, 16);
    println!("sum_sqr_shift max len=16 energy={} shift={}", energy, shift);
    for v in x.iter_mut() {
        *v = -32768i16;
    }
    let (energy, shift) = silk_sum_sqr_shift(&x, 16);
    println!("sum_sqr_shift min len=16 energy={} shift={}", energy, shift);
    for i in 0..16 {
        x[i] = if i & 1 != 0 { -32768 } else { 32767 };
    }
    let (energy, shift) = silk_sum_sqr_shift(&x, 16);
    println!("sum_sqr_shift alt len=16 energy={} shift={}", energy, shift);
}

fn sec_inner_prod() {
    let lens: [i32; 6] = [1, 2, 8, 16, 64, 320];
    let scales: [i32; 5] = [0, 1, 4, 8, 15];
    let mut v1 = [0i16; 320];
    let mut v2 = [0i16; 320];

    println!("## inner_prod_aligned_scale");
    for li in 0..6 {
        let len = lens[li];
        for si in 0..5 {
            rng_seed(0x21000u32 + (len * 8 + si) as u32);
            for i in 0..len {
                v1[i as usize] = rnd_i16();
                v2[i as usize] = rnd_i16();
            }
            let sum = silk_inner_prod_aligned_scale(&v1, &v2, scales[si as usize], len);
            println!(
                "inner_prod len={} scale={} sum={}",
                len, scales[si as usize], sum
            );
        }
    }
}

/* ------------------------------------------------------------------ */
/* 3. sort                                                             */
/* ------------------------------------------------------------------ */

fn sec_sort() {
    let lk: [[i32; 2]; 6] = [
        [1, 1],
        [5, 3],
        [5, 5],
        [8, 3],
        [16, 8],
        [32, 10],
    ];
    let lens: [i32; 5] = [1, 2, 5, 16, 24];
    let mut a = [0i32; 32];
    let mut idx = [0i32; 32];
    let mut a16 = [0i16; 24];

    println!("## sort");
    for ci in 0..6 {
        let (l, k) = (lk[ci][0], lk[ci][1]);
        rng_seed(0x30000u32 + (l * 8 + k) as u32);
        for i in 0..l {
            a[i as usize] = rnd_below(2001) - 1000;
        }
        for i in 0..k {
            idx[i as usize] = 0;
        }
        silk_insertion_sort_increasing(&mut a, &mut idx, l, k);
        print!("sort_inc L={} K={} ", l, k);
        pa32("a", &a[..l as usize]);
        print!("sort_inc L={} K={} ", l, k);
        pa32("idx", &idx[..k as usize]);
    }
    for ci in 0..5 {
        let l = lens[ci];
        rng_seed(0x31000u32 + l as u32);
        for i in 0..l {
            a16[i as usize] = (rnd_below(101) - 50) as i16;
        }
        silk_insertion_sort_increasing_all_values_int16(&mut a16, l);
        print!("sort_all16 L={} ", l);
        pa16("a", &a16[..l as usize]);
    }
}

/* ------------------------------------------------------------------ */
/* 4. interpolate                                                      */
/* ------------------------------------------------------------------ */

fn sec_interpolate() {
    let ds: [i32; 5] = [1, 2, 10, 16, 24];
    let mut xi = [0i16; 24];
    let mut x0 = [0i16; 24];
    let mut x1 = [0i16; 24];

    println!("## interpolate");
    for di in 0..5 {
        let d = ds[di];
        for fi in 0..=4 {
            rng_seed(0x40000u32 + (di * 8 + fi) as u32);
            for i in 0..d {
                x0[i as usize] = rnd_i16();
                x1[i as usize] = rnd_i16();
                xi[i as usize] = 0;
            }
            silk_interpolate(&mut xi, &x0, &x1, fi as i32, d);
            print!("interpolate d={} ifact={} ", d, fi);
            pa16("xi", &xi[..d as usize]);
        }
    }
}

/* ------------------------------------------------------------------ */
/* 5. lin2log / log2lin / sigm_Q15                                     */
/* ------------------------------------------------------------------ */

fn sec_log() {
    let mut v = [0i32; 1024];
    let mut pk = Pack::new();

    println!("## lin2log");
    rng_seed(0x50001u32);
    v[0] = 0;
    v[1] = 1;
    v[2] = 2;
    v[3] = 3;
    v[4] = 4;
    v[5] = 0x7FFFFFFF;
    v[6] = 0x40000000;
    v[7] = -1;
    v[8] = i32::MIN;
    v[9] = -1000000;
    v[10] = 1000000;
    v[11] = 60;
    v[12] = 100;
    v[13] = 3932160;
    v[14] = 65536;
    v[15] = 32767;
    for i in 16..1024 {
        v[i] = rnd();
    }
    for i in 0..1024 {
        pk.pk("lin2log", i, silk_lin2log(v[i]) as i64);
    }
    pk.done();

    println!("## log2lin");
    for i in 0..1024 {
        pk.pk("log2lin", i, silk_log2lin((i - 8) as i32) as i64);
    }
    pk.done();
    for i in 1024..3984 {
        pk.pk("log2lin", i, silk_log2lin((i - 8) as i32) as i64);
    }
    pk.done();
    {
        let bnd: [i32; 9] = [
            -1000000,
            -1,
            0,
            3966,
            3967,
            3968,
            1000000,
            0x7FFFFFFF,
            i32::MIN,
        ];
        for (b, &x) in bnd.iter().enumerate() {
            pk.pk("log2lin_bnd", b, silk_log2lin(x) as i64);
        }
        pk.done();
    }

    println!("## sigm_Q15");
    for i in 0..448 {
        pk.pk("sigm_Q15", i, silk_sigm_Q15(i as i32 - 224) as i64);
    }
    pk.done();
    {
        let bnd: [i32; 8] = [-1000000, -193, -192, -191, 191, 192, 193, 1000000];
        for (b, &x) in bnd.iter().enumerate() {
            pk.pk("sigm_Q15_bnd", b, silk_sigm_Q15(x) as i64);
        }
        pk.done();
    }
}

/* ------------------------------------------------------------------ */
/* 6. gain quantizer                                                   */
/* ------------------------------------------------------------------ */

fn sec_gain() {
    let mut ind = [0i8; 4];
    let mut gain_Q16 = [0i32; 4];
    let mut prev: i8;

    println!("## gains_quant");
    for c in 0..12 {
        let nb_subfr = 1 + (c % 4);
        let conditional = c % 2;
        rng_seed(0x60000u32 + c as u32);
        prev = rnd_below(64) as i8;
        for k in 0..nb_subfr {
            if c < 6 {
                gain_Q16[k as usize] = rnd() & 0x7FFFFFFF;
            } else {
                gain_Q16[k as usize] = 1 + rnd_below(1000);
            }
        }
        if c == 4 {
            gain_Q16[0] = 1;
        }
        if c == 5 {
            gain_Q16[0] = 0x7FFFFFFF;
        }
        silk_gains_quant(&mut ind, &mut gain_Q16, &mut prev, conditional, nb_subfr);
        print!(
            "gains_quant c={} nb={} cond={} prev={} ",
            c, nb_subfr, conditional, prev
        );
        pa32("gain_Q16", &gain_Q16[..nb_subfr as usize]);
        print!("gains_quant c={} nb={} cond={} ", c, nb_subfr, conditional);
        print!("ind=[");
        for k in 0..nb_subfr {
            if k + 1 < nb_subfr {
                print!("{}, ", ind[k as usize]);
            } else {
                print!("{}", ind[k as usize]);
            }
        }
        println!("]");
    }

    println!("## gains_dequant");
    for c in 0..12 {
        let nb_subfr = 1 + (c % 4);
        let conditional = c % 2;
        rng_seed(0x61000u32 + c as u32);
        prev = rnd_below(64) as i8;
        for k in 0..nb_subfr {
            if c < 6 {
                ind[k as usize] = rnd_below(64) as i8;
            } else {
                ind[k as usize] = rnd_below(41) as i8;
            }
        }
        silk_gains_dequant(&mut gain_Q16, &ind, &mut prev, conditional, nb_subfr);
        print!(
            "gains_dequant c={} nb={} cond={} prev={} ",
            c, nb_subfr, conditional, prev
        );
        pa32("gain_Q16", &gain_Q16[..nb_subfr as usize]);
    }

    println!("## gains_ID");
    for c in 0..6 {
        let nb_subfr = 1 + (c % 4);
        rng_seed(0x62000u32 + c as u32);
        for k in 0..nb_subfr {
            ind[k as usize] = rnd() as i8;
        }
        let id = silk_gains_ID(&ind, nb_subfr);
        println!("gains_ID c={} nb={} id={}", c, nb_subfr, id);
    }
}

/* ------------------------------------------------------------------ */
/* 7. NLSF                                                             */
/* ------------------------------------------------------------------ */

fn sec_nlsf() {
    let cbs: [&silk_NLSF_CB_struct; 2] = [&silk_NLSF_CB_NB_MB, &silk_NLSF_CB_WB];
    let mut ec_ix = [0i16; 16];
    let mut pred_Q8 = [0u8; 16];
    let mut NLSF_Q15 = [0i16; MAX_LPC_ORDER as usize];
    let mut st_Q15 = [0i16; MAX_LPC_ORDER as usize];
    let mut NLSFIndices = [0i8; (MAX_LPC_ORDER + 1) as usize];
    let mut a_Q12 = [0i16; MAX_LPC_ORDER as usize];

    println!("## NLSF_unpack");
    for cb in 0..2 {
        let order = cbs[cb].order as i32;
        let nV = cbs[cb].nVectors as i32;
        let sel = [0i32, 1, 7];
        for si in 0..3 {
            let idx = sel[si] % nV;
            for i in 0..16 {
                ec_ix[i] = 0;
                pred_Q8[i] = 0;
            }
            silk_NLSF_unpack(&mut ec_ix, &mut pred_Q8, cbs[cb], idx);
            print!("NLSF_unpack cb={} idx={} order={} ", cb, idx, order);
            pa16("ec_ix", &ec_ix[..order as usize]);
            print!("NLSF_unpack cb={} idx={} order={} ", cb, idx, order);
            pau8("pred_Q8", &pred_Q8[..order as usize]);
        }
        for i in 0..16 {
            ec_ix[i] = 0;
            pred_Q8[i] = 0;
        }
        silk_NLSF_unpack(&mut ec_ix, &mut pred_Q8, cbs[cb], nV - 1);
        print!("NLSF_unpack cb={} idx={} order={} ", cb, nV - 1, order);
        pa16("ec_ix", &ec_ix[..order as usize]);
        print!("NLSF_unpack cb={} idx={} order={} ", cb, nV - 1, order);
        pau8("pred_Q8", &pred_Q8[..order as usize]);
    }

    println!("## NLSF_decode");
    for cb in 0..2 {
        let order = cbs[cb].order as i32;
        for t in 0..8 {
            rng_seed(0x70000u32 + (cb * 16 + t) as u32);
            NLSFIndices[0] = rnd_below(cbs[cb].nVectors as i32) as i8;
            for i in 1..=order {
                NLSFIndices[i as usize] = (rnd_below(21) - 10) as i8;
            }
            for i in 0..16 {
                NLSF_Q15[i] = 0;
            }
            silk_NLSF_decode(&mut NLSF_Q15, &NLSFIndices, cbs[cb]);
            print!(
                "NLSF_decode cb={} t={} idx0={} ",
                cb, t, NLSFIndices[0]
            );
            pa16("NLSF_Q15", &NLSF_Q15[..order as usize]);
        }
    }

    println!("## NLSF_stabilize");
    for cb in 0..2 {
        let order = cbs[cb].order as i32;
        for t in 0..6 {
            rng_seed(0x71000u32 + (cb * 16 + t) as u32);
            if t == 0 {
                for i in 0..order {
                    NLSF_Q15[i as usize] = 0;
                }
            } else if t == 1 {
                for i in 0..order {
                    NLSF_Q15[i as usize] = 32767;
                }
            } else if t == 2 {
                for i in 0..order {
                    NLSF_Q15[i as usize] = 1000;
                }
            } else {
                for i in 0..order {
                    NLSF_Q15[i as usize] = rnd_below(32768) as i16;
                }
            }
            for i in 0..16 {
                st_Q15[i] = NLSF_Q15[i];
            }
            silk_NLSF_stabilize(&mut st_Q15, cbs[cb].deltaMin_Q15, order);
            print!("NLSF_stabilize cb={} t={} ", cb, t);
            pa16("in", &NLSF_Q15[..order as usize]);
            print!("NLSF_stabilize cb={} t={} ", cb, t);
            pa16("out", &st_Q15[..order as usize]);
        }
    }

    println!("## NLSF2A");
    for cb in 0..2 {
        let order = cbs[cb].order as i32;
        for t in 0..4 {
            rng_seed(0x72000u32 + (cb * 16 + t) as u32);
            for i in 0..order {
                NLSF_Q15[i as usize] = rnd_below(32768) as i16;
            }
            silk_NLSF_stabilize(&mut NLSF_Q15, cbs[cb].deltaMin_Q15, order);
            for k in 0..16 {
                a_Q12[k] = 0;
            }
            silk_NLSF2A(&mut a_Q12, &NLSF_Q15, order, 0);
            print!("NLSF2A cb={} t={} d={} ", cb, t, order);
            pa16("a_Q12", &a_Q12[..order as usize]);
            println!(
                "NLSF2A cb={} t={} d={} gain={}",
                cb,
                t,
                order,
                silk_LPC_inverse_pred_gain_c(&a_Q12, order)
            );
        }
    }
}

/* ------------------------------------------------------------------ */
/* 8. LPC                                                              */
/* ------------------------------------------------------------------ */

fn sec_lpc() {
    let qpairs: [[i32; 2]; 5] = [[12, 13], [12, 16], [8, 16], [0, 2], [4, 16]];
    let ds: [i32; 4] = [1, 4, 10, 16];
    let mut a_QOUT = [0i16; 24];
    let mut a_QIN = [0i32; 24];

    println!("## LPC_fit");
    for qi in 0..5 {
        for di in 0..4 {
            let d = ds[di];
            let (QOUT, QIN) = (qpairs[qi][0], qpairs[qi][1]);
            for c in 0..2 {
                rng_seed(0x80000u32 + (qi * 64 + di * 8 + c) as u32);
                for i in 0..d {
                    if c == 0 {
                        a_QIN[i as usize] = rnd();
                    } else {
                        a_QIN[i as usize] = rnd() >> (10 + di);
                    }
                }
                if c == 1 && d > 0 {
                    a_QIN[0] = 0x001FFFFF;
                }
                for i in 0..24 {
                    a_QOUT[i] = 0;
                }
                silk_LPC_fit(&mut a_QOUT, &mut a_QIN, QOUT, QIN, d);
                print!("LPC_fit QOUT={} QIN={} d={} c={} ", QOUT, QIN, d, c);
                pa16("a_QOUT", &a_QOUT[..d as usize]);
                print!("LPC_fit QOUT={} QIN={} d={} c={} ", QOUT, QIN, d, c);
                pa32("a_QIN", &a_QIN[..d as usize]);
            }
        }
    }

    println!("## LPC_inverse_pred_gain");
    for di in 0..2 {
        let d = if di == 0 { 10 } else { 16 };
        for c in 0..6 {
            rng_seed(0x81000u32 + (di * 16 + c) as u32);
            if c == 0 || c == 3 {
                for i in 0..d {
                    a_QOUT[i as usize] = rnd_i16();
                }
            } else if c == 1 {
                for i in 0..d {
                    a_QOUT[i as usize] = 0;
                }
            } else if c == 2 {
                for i in 0..d {
                    a_QOUT[i as usize] = 32767;
                }
            } else if c == 4 {
                for i in 0..d {
                    a_QOUT[i as usize] = (rnd_below(4096) - 2048) as i16;
                }
            } else {
                for i in 0..d {
                    a_QOUT[i as usize] = (rnd_below(8192) - 4096) as i16;
                }
            }
            let gain = silk_LPC_inverse_pred_gain_c(&a_QOUT, d);
            print!("LPC_inv d={} c={} ", d, c);
            pa16("A_Q12", &a_QOUT[..d as usize]);
            println!("LPC_inv d={} c={} gain={}", d, c, gain);
        }
    }
}

/* ------------------------------------------------------------------ */
/* 9. shell coder / code signs                                         */
/* ------------------------------------------------------------------ */

fn sec_entropy() {
    let mut buf = [0u8; 256];
    let mut pulses0 = [0i32; 16];
    let mut pulses1 = [0i16; 16];

    println!("## shell_coder");
    for t in 0..8 {
        rng_seed(0x90000u32 + t as u32);
        let pulses4 = 1 + rnd_below(16);
        for j in 0..16 {
            pulses0[j] = 0;
        }
        for _ in 0..pulses4 {
            let j = rnd_below(16) as usize;
            pulses0[j] += 1;
        }
        for b in buf.iter_mut() {
            *b = 0;
        }
        let mut enc = ec_enc_init(256);
        silk_shell_encoder(&mut enc, &pulses0);
        ec_enc_done(&mut enc);
        print!("shell_enc t={} pulses4={} ", t, pulses4);
        print!("pulses0=[");
        for j in 0..16 {
            if j + 1 < 16 {
                print!("{}, ", pulses0[j]);
            } else {
                print!("{}", pulses0[j]);
            }
        }
        println!("]");
        p_ec("shell_enc", &enc);
        print!("shell_enc buf=");
        for j in 0..32 {
            print!("{:02x}", enc.buf[j]);
        }
        println!();
        for j in 0..16 {
            pulses1[j] = 0;
        }
        let mut dec = ec_dec_init(&enc.buf);
        silk_shell_decoder(&mut pulses1, &mut dec, pulses4);
        print!("shell_dec t={} pulses4={} ", t, pulses4);
        pa16("pulses0", &pulses1);
        p_ec("shell_dec", &dec);
    }

    println!("## code_signs");
    for t in 0..12 {
        let signalType = t % 3;
        let quantOffsetType = (t / 3) % 2;
        let nblocks = 1 + (t % 4);
        let length = 16 * nblocks;
        let mut pulses = [0i8; 64];
        let mut pulses_dec = [0i16; 64];
        let mut sum_pulses = [0i32; 64];
        rng_seed(0x91000u32 + t as u32);
        for i in 0..length as usize {
            let v = rnd_below(17) - 8;
            pulses[i] = v as i8;
            pulses_dec[i] = if v < 0 { -v as i16 } else { v as i16 };
        }
        for i in 0..nblocks {
            let mut s = 0;
            for j in 0..16 {
                let v = pulses[16 * i + j] as i32;
                s += if v < 0 { -v } else { v };
            }
            sum_pulses[i] = s;
        }
        for b in buf.iter_mut() {
            *b = 0;
        }
        let mut enc = ec_enc_init(256);
        silk_encode_signs(
            &mut enc,
            &pulses,
            length as i32,
            signalType as i32,
            quantOffsetType as i32,
            &sum_pulses,
        );
        ec_enc_done(&mut enc);
        print!(
            "signs t={} type={} off={} length={} ",
            t, signalType, quantOffsetType, length
        );
        pa32("sum_pulses", &sum_pulses[..nblocks as usize]);
        p_ec("signs_enc", &enc);
        print!("signs_enc buf=");
        for j in 0..32 {
            print!("{:02x}", enc.buf[j]);
        }
        println!();
        let mut dec = ec_dec_init(&enc.buf);
        silk_decode_signs(
            &mut dec,
            &mut pulses_dec,
            length as i32,
            signalType as i32,
            quantOffsetType as i32,
            &sum_pulses,
        );
        print!(
            "signs t={} type={} off={} length={} ",
            t, signalType, quantOffsetType, length
        );
        print!("pulses=[");
        for j in 0..length as usize {
            if j + 1 < length as usize {
                print!("{}, ", pulses[j]);
            } else {
                print!("{}", pulses[j]);
            }
        }
        println!("]");
        print!(
            "signs t={} type={} off={} length={} ",
            t, signalType, quantOffsetType, length
        );
        pa16("decoded", &pulses_dec[..length as usize]);
        p_ec("signs_dec", &dec);
    }
}

/* ------------------------------------------------------------------ */
/* 10. biquad / filter bank                                            */
/* ------------------------------------------------------------------ */

fn sec_filter() {
    let mut in_ = [0i16; 640];
    let mut out = [0i16; 640];
    let mut B_Q28 = [0i32; 3];
    let mut A_Q28 = [0i32; 2];
    let mut S = [0i32; 4];
    let mut S2 = [0i32; 2];
    let mut outL = [0i16; 320];
    let mut outH = [0i16; 320];
    let lens1: [i32; 4] = [1, 8, 64, 320];
    let lens2: [i32; 4] = [1, 8, 64, 320];

    println!("## biquad_alt_stride1");
    for li in 0..4 {
        let len = lens1[li];
        rng_seed(0xa0000u32 + len as u32);
        for i in 0..len {
            in_[i as usize] = rnd_i16();
        }
        for i in 0..3 {
            B_Q28[i] = rnd() >> 6;
        }
        for i in 0..2 {
            A_Q28[i] = rnd() >> 6;
        }
        for i in 0..4 {
            S[i] = rnd() >> 6;
        }
        silk_biquad_alt_stride1(&in_, &B_Q28, &A_Q28, &mut S, &mut out, len);
        print!("biquad1 len={} ", len);
        pa32("B", &B_Q28);
        print!("biquad1 len={} ", len);
        pa32("A", &A_Q28);
        print!("biquad1 len={} ", len);
        pa16("out", &out[..len as usize]);
        print!("biquad1 len={} ", len);
        pa32("S", &S[..2]);
    }

    println!("## biquad_alt_stride2");
    for li in 0..4 {
        let len = lens2[li];
        rng_seed(0xa1000u32 + len as u32);
        for i in 0..2 * len {
            in_[i as usize] = rnd_i16();
        }
        for i in 0..3 {
            B_Q28[i] = rnd() >> 6;
        }
        for i in 0..2 {
            A_Q28[i] = rnd() >> 6;
        }
        for i in 0..4 {
            S[i] = rnd() >> 6;
        }
        silk_biquad_alt_stride2_c(&in_, &B_Q28, &A_Q28, &mut S, &mut out, len);
        print!("biquad2 len={} ", len);
        pa32("B", &B_Q28);
        print!("biquad2 len={} ", len);
        pa32("A", &A_Q28);
        print!("biquad2 len={} ", len);
        pa16("out", &out[..(2 * len) as usize]);
        print!("biquad2 len={} ", len);
        pa32("S", &S);
    }

    println!("## ana_filt_bank_1");
    {
        let ns: [i32; 5] = [2, 4, 8, 64, 320];
        for li in 0..5 {
            let n = ns[li];
            rng_seed(0xa2000u32 + n as u32);
            for i in 0..n {
                in_[i as usize] = rnd_i16();
            }
            for i in 0..2 {
                S2[i] = rnd() >> 6;
            }
            silk_ana_filt_bank_1(&in_, &mut S2, &mut outL, &mut outH, n);
            print!("ana_filt N={} ", n);
            pa16("outL", &outL[..(n / 2) as usize]);
            print!("ana_filt N={} ", n);
            pa16("outH", &outH[..(n / 2) as usize]);
            print!("ana_filt N={} ", n);
            pa32("S", &S2[..2]);
        }
    }
}

/* ------------------------------------------------------------------ */
/* 11. cutoff / stereo                                                 */
/* ------------------------------------------------------------------ */

fn sec_cutoff() {
    let modes: [i32; 4] = [-1, 0, 1, 2];
    let tfs: [i32; 5] = [0, 1, 100, 255, 256];
    let mut frame = [0i16; 320];

    println!("## LP_variable_cutoff");
    for mi in 0..4 {
        for ti in 0..5 {
            rng_seed(0xb0000u32 + (mi * 8 + ti) as u32);
            let mut psLP = silk_LP_state::default();
            psLP.mode = modes[mi];
            psLP.transition_frame_no = tfs[ti];
            psLP.saved_fs_kHz = 8;
            psLP.In_LP_State[0] = rnd() >> 4;
            psLP.In_LP_State[1] = rnd() >> 4;
            for i in 0..320 {
                frame[i] = rnd_i16();
            }
            silk_LP_variable_cutoff(&mut psLP, &mut frame, 320);
            print!("LP_cut mode={} tf={} ", modes[mi], tfs[ti]);
            pa16("frame", &frame);
            println!(
                "LP_cut mode={} tf={} state=[{}, {}] tfno={} mode2={} fs={}",
                modes[mi],
                tfs[ti],
                psLP.In_LP_State[0],
                psLP.In_LP_State[1],
                psLP.transition_frame_no,
                psLP.mode,
                psLP.saved_fs_kHz
            );
        }
    }

    println!("## HP_variable_cutoff");
    for ti in 0..6 {
        rng_seed(0xb1000u32 + ti as u32);
        let mut state_Fxx = [silk_encoder_state_Fxx::default(); 2];
        state_Fxx[0].sCmn.prevSignalType = if ti == 3 {
            0
        } else if ti == 4 {
            1
        } else {
            2
        };
        state_Fxx[0].sCmn.fs_kHz = if ti == 1 {
            12
        } else if ti == 2 {
            24
        } else if ti == 5 {
            16
        } else {
            8
        };
        state_Fxx[0].sCmn.prevLag = 20 + ti * 60;
        state_Fxx[0].sCmn.input_quality_bands_Q15[0] = rnd_below(32768);
        state_Fxx[0].sCmn.speech_activity_Q8 = rnd_below(256);
        state_Fxx[0].sCmn.variable_HP_smth1_Q15 = rnd() & 0x0FFFFFFF;
        silk_HP_variable_cutoff(&mut state_Fxx);
        println!(
            "HP_cut t={} prevSignalType={} fs_kHz={} prevLag={} quality={} activity={} smth1={}",
            ti,
            state_Fxx[0].sCmn.prevSignalType,
            state_Fxx[0].sCmn.fs_kHz,
            state_Fxx[0].sCmn.prevLag,
            state_Fxx[0].sCmn.input_quality_bands_Q15[0],
            state_Fxx[0].sCmn.speech_activity_Q8,
            state_Fxx[0].sCmn.variable_HP_smth1_Q15
        );
    }
}

fn sec_stereo() {
    let fss: [i32; 3] = [8, 12, 16];
    let mut x1 = [0i16; (MAX_FRAME_LENGTH + 2) as usize];
    let mut x2 = [0i16; (MAX_FRAME_LENGTH + 2) as usize];
    let mut pred_Q13 = [0i32; 2];

    println!("## stereo_MS_to_LR");
    for fi in 0..3 {
        let fs_kHz = fss[fi];
        let frame_length = fs_kHz * 20;
        for t in 0..2 {
            rng_seed(0xc0000u32 + (fi * 8 + t) as u32);
            let mut state = stereo_dec_state::default();
            for i in 0..2 {
                state.pred_prev_Q13[i] = rnd_i16();
                state.sMid[i] = rnd_i16();
                state.sSide[i] = rnd_i16();
            }
            for i in 0..frame_length + 2 {
                x1[i as usize] = rnd_i16();
                x2[i as usize] = rnd_i16();
            }
            pred_Q13[0] = rnd() >> 18;
            pred_Q13[1] = rnd() >> 18;
            silk_stereo_MS_to_LR(&mut state, &mut x1, &mut x2, &pred_Q13, fs_kHz, frame_length);
            print!(
                "MS_to_LR fs={} t={} pred=[{}, {}] ",
                fs_kHz, t, pred_Q13[0], pred_Q13[1]
            );
            pa16("x1", &x1[..(frame_length + 2) as usize]);
            print!("MS_to_LR fs={} t={} ", fs_kHz, t);
            pa16("x2", &x2[..(frame_length + 2) as usize]);
            println!(
                "MS_to_LR fs={} t={} prev=[{}, {}] sMid=[{}, {}] sSide=[{}, {}]",
                fs_kHz,
                t,
                state.pred_prev_Q13[0],
                state.pred_prev_Q13[1],
                state.sMid[0],
                state.sMid[1],
                state.sSide[0],
                state.sSide[1]
            );
        }
    }

    println!("## stereo_decode_pred");
    {
        let mut sbuf = [0u8; 64];
        rng_seed(0xc1000u32);
        for i in 0..64 {
            sbuf[i] = rnd() as u8;
        }
        let mut dec = ec_dec_init(&sbuf);
        let mut p = [0i32; 2];
        for t in 0..4 {
            silk_stereo_decode_pred(&mut dec, &mut p);
            println!("stereo_pred t={} p=[{}, {}]", t, p[0], p[1]);
        }
        let mut mid = 0i32;
        silk_stereo_decode_mid_only(&mut dec, &mut mid);
        println!("stereo_mid mid={}", mid);
        p_ec("stereo_dec", &dec);
    }
}

/* ------------------------------------------------------------------ */
/* 12. decode_pitch                                                    */
/* ------------------------------------------------------------------ */

fn sec_decode_pitch() {
    let fss: [i32; 3] = [8, 12, 16];
    let nbs: [i32; 2] = [4, 2];
    let mut pitch_lags = [0i32; 4];

    println!("## decode_pitch");
    for fi in 0..3 {
        let fs = fss[fi];
        for ni in 0..2 {
            let nb = nbs[ni];
            let (min_lag, max_lag) = (2 * fs, 18 * fs);
            let cbk = if fs == 8 {
                if nb == 4 {
                    11
                } else {
                    3
                }
            } else if nb == 4 {
                34
            } else {
                12
            };
            let lagsel = [0i32, 1, 5];
            for li in 0..3 {
                let mut lagIndex = lagsel[li];
                if li == 2 {
                    lagIndex = (max_lag - min_lag) / 2;
                }
                for ci in 0..3 {
                    let contourIndex = if ci == 0 {
                        0
                    } else if ci == 1 {
                        cbk / 2
                    } else {
                        cbk - 1
                    };
                    for k in 0..4 {
                        pitch_lags[k] = 0;
                    }
                    silk_decode_pitch(
                        lagIndex as i16,
                        contourIndex as i8,
                        &mut pitch_lags,
                        fs,
                        nb,
                    );
                    print!(
                        "decode_pitch Fs={} nb={} lagIndex={} contour={} ",
                        fs, nb, lagIndex, contourIndex
                    );
                    pa32("pitch_lags", &pitch_lags[..nb as usize]);
                }
            }
            for k in 0..4 {
                pitch_lags[k] = 0;
            }
            silk_decode_pitch(
                (max_lag - min_lag + 3) as i16,
                0,
                &mut pitch_lags,
                fs,
                nb,
            );
            print!(
                "decode_pitch Fs={} nb={} lagIndex={} contour=0 ",
                fs,
                nb,
                max_lag - min_lag + 3
            );
            pa32("pitch_lags", &pitch_lags[..nb as usize]);
        }
    }
}

/* ------------------------------------------------------------------ */
/* 13. resampler private functions                                     */
/* ------------------------------------------------------------------ */

fn sec_res_private() {
    let lens: [i32; 4] = [1, 8, 64, 300];
    let mut S = [0i32; 6];
    let mut out_Q8 = [0i32; 640];
    let mut out = [0i16; 2048];
    let mut in_ = [0i16; 1024];

    println!("## resampler_private_AR2");
    for li in 0..4 {
        let len = lens[li];
        rng_seed(0xd0000u32 + len as u32);
        for i in 0..2 {
            S[i] = rnd() >> 4;
        }
        for i in 0..len {
            in_[i as usize] = rnd_i16();
        }
        for i in 0..640 {
            out_Q8[i] = 0;
        }
        silk_resampler_private_AR2(&mut S, &mut out_Q8, &in_, &silk_Resampler_2_3_COEFS_LQ, len);
        print!("AR2 len={} ", len);
        pa32("out_Q8", &out_Q8[..len as usize]);
        print!("AR2 len={} ", len);
        pa32("S", &S[..2]);
    }

    println!("## resampler_private_up2_HQ");
    for li in 0..4 {
        let len = lens[li];
        rng_seed(0xd1000u32 + len as u32);
        for i in 0..6 {
            S[i] = rnd() >> 4;
        }
        for i in 0..len {
            in_[i as usize] = rnd_i16();
        }
        fill_sent(&mut out);
        silk_resampler_private_up2_HQ(&mut S, &mut out, &in_, len);
        print!("up2_HQ len={} ", len);
        pa16("out", &out[..(2 * len) as usize]);
        print!("up2_HQ len={} ", len);
        pa32("S", &S);
    }

    println!("## resampler_down2");
    {
        let lens2: [i32; 6] = [2, 4, 16, 64, 300, 480];
        for li in 0..6 {
            let len = lens2[li];
            rng_seed(0xd2000u32 + len as u32);
            for i in 0..2 {
                S[i] = rnd() >> 4;
            }
            for i in 0..len {
                in_[i as usize] = rnd_i16();
            }
            fill_sent(&mut out);
            silk_resampler_down2(&mut S, &mut out, &in_, len);
            print!("down2 len={} ", len);
            pa16("out", &out[..(len / 2) as usize]);
            print!("down2 len={} ", len);
            pa32("S", &S[..2]);
        }
    }

    println!("## resampler_down2_3");
    {
        let lens3: [i32; 5] = [3, 16, 64, 300, 520];
        for li in 0..5 {
            let len = lens3[li];
            rng_seed(0xd3000u32 + len as u32);
            for i in 0..6 {
                S[i] = rnd() >> 4;
            }
            for i in 0..len {
                in_[i as usize] = rnd_i16();
            }
            fill_sent(&mut out);
            silk_resampler_down2_3(&mut S, &mut out, &in_, len);
            let cnt = lead_non_sent(&out);
            print!("down2_3 len={} count={} out=[", len, cnt);
            vals16(&out[..cnt]);
            println!("]");
            print!("down2_3 len={} ", len);
            pa32("S", &S);
        }
    }
}

fn sec_res_fir() {
    let iir_pairs: [[i32; 2]; 6] = [
        [8000, 48000],
        [12000, 48000],
        [16000, 24000],
        [16000, 48000],
        [12000, 16000],
        [8000, 12000],
    ];
    let dfir_pairs: [[i32; 2]; 7] = [
        [16000, 12000],
        [12000, 8000],
        [24000, 12000],
        [48000, 16000],
        [48000, 12000],
        [48000, 8000],
        [24000, 16000],
    ];
    let inlens: [i32; 4] = [8, 64, 300, 520];
    let mut in_ = [0i16; 1024];
    let mut out = [0i16; 8192];

    println!("## resampler_private_IIR_FIR");
    for pi in 0..6 {
        for li in 0..4 {
            let inLen = inlens[li];
            rng_seed(0xd4000u32 + (pi * 16 + li) as u32);
            let mut s = silk_resampler_state_struct::default();
            silk_resampler_init(&mut s, iir_pairs[pi][0], iir_pairs[pi][1], 0);
            for i in 0..6 {
                s.sIIR[i] = rnd() >> 4;
            }
            let mut fir = [0i16; SILK_RESAMPLER_MAX_FIR_ORDER as usize];
            for i in 0..36 {
                fir[i] = rnd_i16();
            }
            s.sFIR = silk_resampler_sFIR { i16: fir };
            for i in 0..inLen {
                in_[i as usize] = rnd_i16();
            }
            fill_sent(&mut out);
            silk_resampler_private_IIR_FIR(&mut s, &mut out, &in_, inLen);
            let cnt = lead_non_sent(&out);
            println!(
                "IIR_FIR pair={}/{} inLen={} count={}",
                iir_pairs[pi][0], iir_pairs[pi][1], inLen, cnt
            );
            print!(
                "IIR_FIR pair={}/{} inLen={} out=[",
                iir_pairs[pi][0], iir_pairs[pi][1], inLen
            );
            vals16(&out[..cnt]);
            println!("]");
            print!(
                "IIR_FIR pair={}/{} inLen={} ",
                iir_pairs[pi][0], iir_pairs[pi][1], inLen
            );
            pa32("sIIR", &s.sIIR);
            print!(
                "IIR_FIR pair={}/{} inLen={} ",
                iir_pairs[pi][0], iir_pairs[pi][1], inLen
            );
            let fir16 = unsafe { s.sFIR.i16 };
            pa16("sFIR", &fir16[..8]);
        }
    }

    println!("## resampler_private_down_FIR");
    for pi in 0..7 {
        for li in 0..4 {
            let inLen = inlens[li];
            rng_seed(0xd5000u32 + (pi * 16 + li) as u32);
            let mut s = silk_resampler_state_struct::default();
            silk_resampler_init(&mut s, dfir_pairs[pi][0], dfir_pairs[pi][1], 1);
            for i in 0..6 {
                s.sIIR[i] = rnd() >> 4;
            }
            let mut fir = [0i32; SILK_RESAMPLER_MAX_FIR_ORDER as usize];
            for i in 0..36 {
                fir[i] = rnd();
            }
            s.sFIR = silk_resampler_sFIR { i32: fir };
            for i in 0..inLen {
                in_[i as usize] = rnd_i16();
            }
            fill_sent(&mut out);
            silk_resampler_private_down_FIR(&mut s, &mut out, &in_, inLen);
            let cnt = lead_non_sent(&out);
            println!(
                "down_FIR pair={}/{} order={} fracs={} inLen={} count={}",
                dfir_pairs[pi][0], dfir_pairs[pi][1], s.FIR_Order, s.FIR_Fracs, inLen, cnt
            );
            print!(
                "down_FIR pair={}/{} inLen={} out=[",
                dfir_pairs[pi][0], dfir_pairs[pi][1], inLen
            );
            vals16(&out[..cnt]);
            println!("]");
            print!(
                "down_FIR pair={}/{} inLen={} ",
                dfir_pairs[pi][0], dfir_pairs[pi][1], inLen
            );
            pa32("sIIR", &s.sIIR);
            print!(
                "down_FIR pair={}/{} inLen={} ",
                dfir_pairs[pi][0], dfir_pairs[pi][1], inLen
            );
            let fir32 = unsafe { s.sFIR.i32 };
            pa32("sFIR32", &fir32);
        }
    }
}

/* ------------------------------------------------------------------ */
/* 14. resampler full path                                             */
/* ------------------------------------------------------------------ */

fn p_res_state(tag: &str, s: &silk_resampler_state_struct) {
    println!(
        "{} fn={} batch={} invRatio={} firOrder={} firFracs={} fsIn={} fsOut={} delay={}",
        tag,
        s.resampler_function,
        s.batchSize,
        s.invRatio_Q16,
        s.FIR_Order,
        s.FIR_Fracs,
        s.Fs_in_kHz,
        s.Fs_out_kHz,
        s.inputDelay
    );
    pa32("sIIR", &s.sIIR);
    let fir16 = unsafe { s.sFIR.i16 };
    pa16("sFIR16", &fir16);
    pa16("delayBuf", &s.delayBuf);
}

fn sec_resampler() {
    let enc_in: [i32; 5] = [8000, 12000, 16000, 24000, 48000];
    let enc_out: [i32; 3] = [8000, 12000, 16000];
    let dec_in: [i32; 3] = [8000, 12000, 16000];
    let dec_out: [i32; 5] = [8000, 12000, 16000, 24000, 48000];
    let mut in_ = [0i16; 1024];
    let mut out = [0i16; 8192];

    println!("## resampler_init");
    for i in 0..5 {
        for j in 0..3 {
            let mut s = silk_resampler_state_struct::default();
            let r = silk_resampler_init(&mut s, enc_in[i], enc_out[j], 1);
            println!(
                "res_init enc {}->{} ret={}",
                enc_in[i], enc_out[j], r
            );
            if r == 0 {
                p_res_state("res_init enc", &s);
            }
        }
    }
    for i in 0..3 {
        for j in 0..5 {
            let mut s = silk_resampler_state_struct::default();
            let r = silk_resampler_init(&mut s, dec_in[i], dec_out[j], 0);
            println!(
                "res_init dec {}->{} ret={}",
                dec_in[i], dec_out[j], r
            );
            if r == 0 {
                p_res_state("res_init dec", &s);
            }
        }
    }

    println!("## resampler");
    for i in 0..5 {
        for j in 0..3 {
            let inLen = 240;
            rng_seed(0xe0000u32 + (i * 8 + j) as u32);
            let mut s = silk_resampler_state_struct::default();
            silk_resampler_init(&mut s, enc_in[i], enc_out[j], 1);
            for k in 0..6 {
                s.sIIR[k] = rnd() >> 4;
            }
            let mut fir = [0i32; SILK_RESAMPLER_MAX_FIR_ORDER as usize];
            for k in 0..36 {
                fir[k] = rnd();
            }
            s.sFIR = silk_resampler_sFIR { i32: fir };
            for k in 0..96 {
                s.delayBuf[k] = rnd_i16();
            }
            for k in 0..inLen {
                in_[k as usize] = rnd_i16();
            }
            fill_sent(&mut out);
            silk_resampler(&mut s, &mut out, &in_, inLen);
            let cnt = lead_non_sent(&out);
            println!(
                "resampler enc {}->{} inLen={} count={}",
                enc_in[i], enc_out[j], inLen, cnt
            );
            print!("resampler enc {}->{} out=[", enc_in[i], enc_out[j]);
            vals16(&out[..cnt]);
            println!("]");
            p_res_state("resampler enc", &s);
        }
    }
    for i in 0..3 {
        for j in 0..5 {
            let inLen = 240;
            rng_seed(0xe1000u32 + (i * 8 + j) as u32);
            let mut s = silk_resampler_state_struct::default();
            silk_resampler_init(&mut s, dec_in[i], dec_out[j], 0);
            for k in 0..6 {
                s.sIIR[k] = rnd() >> 4;
            }
            let mut fir = [0i32; SILK_RESAMPLER_MAX_FIR_ORDER as usize];
            for k in 0..36 {
                fir[k] = rnd();
            }
            s.sFIR = silk_resampler_sFIR { i32: fir };
            for k in 0..96 {
                s.delayBuf[k] = rnd_i16();
            }
            for k in 0..inLen {
                in_[k as usize] = rnd_i16();
            }
            fill_sent(&mut out);
            silk_resampler(&mut s, &mut out, &in_, inLen);
            let cnt = lead_non_sent(&out);
            println!(
                "resampler dec {}->{} inLen={} count={}",
                dec_in[i], dec_out[j], inLen, cnt
            );
            print!("resampler dec {}->{} out=[", dec_in[i], dec_out[j]);
            vals16(&out[..cnt]);
            println!("]");
            p_res_state("resampler dec", &s);
        }
    }
}

fn main() {
    println!("probe_silkdsp: begin");
    sec_bwexpander();
    sec_sum_sqr();
    sec_inner_prod();
    sec_sort();
    sec_interpolate();
    sec_log();
    sec_gain();
    sec_nlsf();
    sec_lpc();
    sec_entropy();
    sec_filter();
    sec_cutoff();
    sec_stereo();
    sec_decode_pitch();
    sec_res_private();
    sec_res_fir();
    sec_resampler();
    println!("probe_silkdsp: end");
}

//! Rust side of the SILK *encoder* integer-core bit-exactness probe.
//!
//! Mirrors `compat/c/probe_silkenc.c` line for line.  Run
//! `bash compat/scripts/run-probe.sh silkenc` to build both sides and diff
//! their output.

#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]
#![allow(clippy::needless_range_loop)]

use oprs::celt::entcode::ec_ctx;
use oprs::celt::entenc::{ec_enc_done, ec_enc_init};
use oprs::silk::a2nlsf::*;
use oprs::silk::control::*;
use oprs::silk::control_audio_bandwidth::*;
use oprs::silk::control_codec::*;
use oprs::silk::control_snr::*;
use oprs::silk::define::*;
use oprs::silk::encode_indices::*;
use oprs::silk::encode_pulses::*;
use oprs::silk::float::main_flp::*;
use oprs::silk::init_encoder::*;
use oprs::silk::lpc_analysis_filter::*;
use oprs::silk::nlsf_del_dec_quant::*;
use oprs::silk::nlsf_encode::*;
use oprs::silk::nlsf_unpack::*;
use oprs::silk::nlsf_vq::*;
use oprs::silk::nlsf_vq_weights_laroia::*;
use oprs::silk::nsq::*;
use oprs::silk::nsq_del_dec::*;
use oprs::silk::process_nlsfs::*;
use oprs::silk::quant_ltp_gains::*;
use oprs::silk::sigproc_fix::SILK_FIX_CONST;
use oprs::silk::stereo_encode_pred::*;
use oprs::silk::stereo_find_predictor::*;
use oprs::silk::stereo_lr_to_ms::*;
use oprs::silk::stereo_quant_pred::*;
use oprs::silk::structs::*;
use oprs::silk::tables_nlsf_cb_nb_mb::silk_NLSF_CB_NB_MB;
use oprs::silk::tables_nlsf_cb_wb::silk_NLSF_CB_WB;
use oprs::silk::tables_other::*;
use oprs::silk::tables_pitch_lag::*;
use oprs::silk::typedef::*;
use oprs::silk::vad::*;
use oprs::silk::vq_wmat_ec::*;

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
fn rnd_range(lo: i32, hi: i32) -> i32 {
    lo + ((rnd() as u32) % ((hi - lo + 1) as u32)) as i32
}

/* ------------------------------------------------------------------ */
/* Print helpers                                                       */
/* ------------------------------------------------------------------ */

fn p1s(n: &str, v: i32) {
    println!("{}={}", n, v);
}

fn pa32(n: &str, a: &[i32]) {
    print!("{}[{}]:", n, a.len());
    for v in a {
        print!(" {}", v);
    }
    println!();
}
fn pa16(n: &str, a: &[i16]) {
    print!("{}[{}]:", n, a.len());
    for v in a {
        print!(" {}", v);
    }
    println!();
}
fn pa8(n: &str, a: &[i8]) {
    print!("{}[{}]:", n, a.len());
    for v in a {
        print!(" {}", v);
    }
    println!();
}
fn pau8(n: &str, a: &[u8]) {
    print!("{}[{}]:", n, a.len());
    for v in a {
        print!(" {}", v);
    }
    println!();
}

fn fnv_buf(b: &[u8]) -> u64 {
    let mut h: u64 = 1469598103934665603;
    for &x in b {
        h ^= x as u64;
        h = h.wrapping_mul(1099511628211);
    }
    h
}
#[allow(dead_code)]
fn fnv_i32(a: &[i32]) -> u64 {
    let mut h: u64 = 1469598103934665603;
    for &x in a {
        let v = x as u32;
        for k in 0..4 {
            h ^= ((v >> (8 * k)) & 0xFF) as u64;
            h = h.wrapping_mul(1099511628211);
        }
    }
    h
}
#[allow(dead_code)]
fn fnv_i16(a: &[i16]) -> u64 {
    let mut h: u64 = 1469598103934665603;
    for &x in a {
        let v = (x as i32 as u32) & 0xFFFF;
        h ^= (v & 0xFF) as u64;
        h = h.wrapping_mul(1099511628211);
        h ^= ((v >> 8) & 0xFF) as u64;
        h = h.wrapping_mul(1099511628211);
    }
    h
}
#[allow(dead_code)]
fn fnv_i8(a: &[i8]) -> u64 {
    let mut h: u64 = 1469598103934665603;
    for &x in a {
        h ^= (x as u8) as u64;
        h = h.wrapping_mul(1099511628211);
    }
    h
}

/* Range-coder byte stream of a completed ec_enc. */
fn ec_dump(tag: &str, enc: &mut ec_ctx) {
    let tell = enc.ec_tell();
    ec_enc_done(enc);
    let nb = ((tell + 7) >> 3) as usize;
    println!("{}.ec_tell={}", tag, tell);
    println!("{}.ec_storage={}", tag, enc.storage);
    println!("{}.ec_nb={}", tag, nb);
    print!("{}.ec_bytes:", tag);
    for i in 0..nb {
        print!(" {:02x}", enc.buf[i]);
    }
    println!();
    println!("{}.ec_hash=0x{:016x}", tag, fnv_buf(&enc.buf[..nb]));
}

/* ------------------------------------------------------------------ */
/* State printers                                                      */
/* ------------------------------------------------------------------ */

fn print_vad(tag: &str, v: &silk_VAD_state) {
    pa32(&format!("{}.AnaState", tag), &v.AnaState);
    pa32(&format!("{}.AnaState1", tag), &v.AnaState1);
    pa32(&format!("{}.AnaState2", tag), &v.AnaState2);
    pa32(&format!("{}.XnrgSubfr", tag), &v.XnrgSubfr);
    pa32(&format!("{}.NrgRatioSmth_Q8", tag), &v.NrgRatioSmth_Q8);
    println!("{}.HPstate={}", tag, v.HPstate);
    pa32(&format!("{}.NL", tag), &v.NL);
    pa32(&format!("{}.inv_NL", tag), &v.inv_NL);
    pa32(&format!("{}.NoiseLevelBias", tag), &v.NoiseLevelBias);
}

fn print_nsq(tag: &str, s: &silk_nsq_state, lml: i32, fl: i32) {
    p1s(&format!("{}.lagPrev", tag), s.lagPrev);
    p1s(&format!("{}.sLTP_buf_idx", tag), s.sLTP_buf_idx);
    p1s(&format!("{}.sLTP_shp_buf_idx", tag), s.sLTP_shp_buf_idx);
    p1s(&format!("{}.rand_seed", tag), s.rand_seed);
    p1s(&format!("{}.prev_gain_Q16", tag), s.prev_gain_Q16);
    p1s(&format!("{}.rewhite_flag", tag), s.rewhite_flag);
    p1s(&format!("{}.sLF_AR_shp_Q14", tag), s.sLF_AR_shp_Q14);
    p1s(&format!("{}.sDiff_shp_Q14", tag), s.sDiff_shp_Q14);
    pa32(
        &format!("{}.sLPC_Q14", tag),
        &s.sLPC_Q14[..NSQ_LPC_BUF_LENGTH as usize],
    );
    pa32(&format!("{}.sAR2_Q14", tag), &s.sAR2_Q14);
    pa16(
        &format!("{}.xq_frame", tag),
        &s.xq[lml as usize..(lml + fl) as usize],
    );
    pa16(&format!("{}.xq_all", tag), &s.xq);
    pa32(
        &format!("{}.sLTP_shp_Q14_frame", tag),
        &s.sLTP_shp_Q14[lml as usize..(lml + fl) as usize],
    );
    pa32(&format!("{}.sLTP_shp_Q14_all", tag), &s.sLTP_shp_Q14);
}

/* ------------------------------------------------------------------ */
/* Encoder setup shared by the control / NSQ sections                  */
/* ------------------------------------------------------------------ */

fn setup_encoder(e: &mut silk_encoder_state_FLP, complexity: i32, payload_ms: i32, n_channels: i32) {
    let mut ctl = silk_EncControlStruct::default();
    ctl.nChannelsAPI = n_channels;
    ctl.nChannelsInternal = n_channels;
    ctl.API_sampleRate = 48000;
    ctl.maxInternalSampleRate = 16000;
    ctl.minInternalSampleRate = 8000;
    ctl.desiredInternalSampleRate = 16000;
    ctl.payloadSize_ms = payload_ms;
    ctl.bitRate = 32000;
    ctl.complexity = complexity;
    *e = silk_encoder_state_FLP::default();
    silk_init_encoder(e, 4);
    silk_control_encoder(e, &mut ctl, 0, 0, 0);
}

fn main() {
    /* ============================================================= */
    /* 1. silk_LPC_analysis_filter                                   */
    /* ============================================================= */
    for cas in 0..4 {
        let d = if (cas & 1) != 0 { 10 } else { 16 };
        let len = if cas < 2 { 60 } else { 120 } as usize;
        let mut input = vec![0i16; len];
        let mut out = vec![0i16; len];
        let mut b = vec![0i16; d as usize];
        rng_seed(1000 + cas as u32);
        for i in 0..len {
            input[i] = rnd() as i16;
        }
        for i in 0..d as usize {
            b[i] = if cas == 3 {
                if i == 0 {
                    4096
                } else {
                    0
                }
            } else {
                rnd() as i16
            };
        }
        silk_LPC_analysis_filter(&mut out, &input, &b, len as i32, d, 4);
        println!("lpcaf case={} d={} len={}", cas, d, len);
        pa16(&format!("lpcaf.in.{}", cas), &input);
        pa16(&format!("lpcaf.B.{}", cas), &b);
        pa16(&format!("lpcaf.out.{}", cas), &out);
    }

    /* ============================================================= */
    /* 2. silk_VAD_Init / silk_VAD_GetSA_Q8 / silk_VAD_GetNoiseLevels */
    /* ============================================================= */
    {
        let mut enc = silk_encoder_state::default();
        let mut input = vec![0i16; MAX_FRAME_LENGTH as usize];
        let mut px = [0i32; VAD_N_BANDS as usize];

        let ret = silk_VAD_Init(&mut enc.sVAD);
        println!("vad_init ret={}", ret);
        print_vad("vad_init", &enc.sVAD);
        p1s("vad_init.counter", enc.sVAD.counter);

        for cas in 0..3 {
            let mut enc = silk_encoder_state::default();
            enc.fs_kHz = 16;
            enc.frame_length = if cas == 2 { 160 } else { 320 };
            silk_VAD_Init(&mut enc.sVAD);
            rng_seed(2000 + cas as u32);
            for frame in 0..3 {
                let mut enc_c = enc;
                for i in 0..enc.frame_length as usize {
                    input[i] = (rnd() >> (cas * 5)) as i16;
                }
                let sa4 = silk_VAD_GetSA_Q8(&mut enc, &input, 4);
                let sac = silk_VAD_GetSA_Q8_c(&mut enc_c, &input);
                println!(
                    "vad case={} frame={} sa_arch4={} sa_c={}",
                    cas, frame, sa4, sac
                );
                p1s(
                    &format!("vad.speech_activity_Q8.{}.{}", cas, frame),
                    enc.speech_activity_Q8,
                );
                p1s(
                    &format!("vad.input_tilt_Q15.{}.{}", cas, frame),
                    enc.input_tilt_Q15,
                );
                pa32(
                    &format!("vad.input_quality_bands_Q15.{}.{}", cas, frame),
                    &enc.input_quality_bands_Q15,
                );
                print_vad(&format!("vad.state.{}.{}", cas, frame), &enc.sVAD);
                p1s(&format!("vad.counter.{}.{}", cas, frame), enc.sVAD.counter);
            }
        }

        /* Direct noise-level estimation over boundary energies. */
        for cas in 0..4 {
            let mut st = silk_VAD_state::default();
            silk_VAD_Init(&mut st);
            rng_seed(2500 + cas as u32);
            if cas == 0 {
                for k in 0..VAD_N_BANDS as usize {
                    px[k] = 0;
                }
            } else if cas == 1 {
                for k in 0..VAD_N_BANDS as usize {
                    px[k] = 0x7FFFFFFF;
                }
            } else if cas == 2 {
                for k in 0..VAD_N_BANDS as usize {
                    px[k] = rnd_below(1 << 20);
                }
            } else {
                for k in 0..VAD_N_BANDS as usize {
                    px[k] = 1 << 28;
                }
            }
            println!("vad_noise case={}", cas);
            pa32("vad_noise.pX", &px);
            silk_VAD_GetNoiseLevels(&px, &mut st);
            print_vad("vad_noise.state", &st);
            p1s("vad_noise.counter", st.counter);
        }
    }

    /* ============================================================= */
    /* 3. silk_VQ_WMat_EC_c / silk_VQ_WMat_EC (arch 4)               */
    /* ============================================================= */
    for cas in 0..4 {
        let l: i32 = if cas == 0 {
            8
        } else if cas == 1 {
            16
        } else {
            32
        };
        let mut xx_q17 = [0i32; 25];
        let mut xx_x_q17 = [0i32; 5];
        let mut cb_q7 = [0i8; 32 * 5];
        let mut cb_gain_q7 = [0u8; 32];
        let mut cl_q5 = [0u8; 32];
        rng_seed(3000 + cas as u32);
        for i in 0..25 {
            xx_q17[i] = rnd_range(-(1 << 20), 1 << 20);
        }
        for i in 0..5 {
            xx_x_q17[i] = rnd_range(-(1 << 18), 1 << 18);
        }
        for i in 0..(l * 5) as usize {
            cb_q7[i] = rnd_range(-64, 64) as i8;
        }
        for i in 0..l as usize {
            cb_gain_q7[i] = rnd_range(0, 255) as u8;
            cl_q5[i] = rnd_range(0, 63) as u8;
        }
        let max_gain_q7 = if cas == 0 { 0 } else { rnd_range(0, 5000) };
        println!("vqwmat case={} L={} max_gain={}", cas, l, max_gain_q7);
        pa32("vqwmat.XX", &xx_q17);
        pa32("vqwmat.xX", &xx_x_q17);
        pa8("vqwmat.cb", &cb_q7[..(l * 5) as usize]);
        pau8("vqwmat.cb_gain", &cb_gain_q7[..l as usize]);
        pau8("vqwmat.cl", &cl_q5[..l as usize]);
        let mut ind_a = 0i8;
        let mut res_a = 0i32;
        let mut rate_a = 0i32;
        let mut gain_a = 0i32;
        let mut ind_b = 0i8;
        let mut res_b = 0i32;
        let mut rate_b = 0i32;
        let mut gain_b = 0i32;
        let mut ind_c = 0i8;
        let mut res_c = 0i32;
        let mut rate_c = 0i32;
        let mut gain_c = 0i32;
        silk_VQ_WMat_EC_c(
            &mut ind_a,
            &mut res_a,
            &mut rate_a,
            &mut gain_a,
            &xx_q17,
            &xx_x_q17,
            &cb_q7,
            &cb_gain_q7,
            &cl_q5,
            40,
            max_gain_q7,
            l,
        );
        silk_VQ_WMat_EC(
            &mut ind_b,
            &mut res_b,
            &mut rate_b,
            &mut gain_b,
            &xx_q17,
            &xx_x_q17,
            &cb_q7,
            &cb_gain_q7,
            &cl_q5,
            40,
            max_gain_q7,
            l,
            4,
        );
        silk_VQ_WMat_EC(
            &mut ind_c,
            &mut res_c,
            &mut rate_c,
            &mut gain_c,
            &xx_q17,
            &xx_x_q17,
            &cb_q7,
            &cb_gain_q7,
            &cl_q5,
            40,
            max_gain_q7,
            l,
            0,
        );
        println!(
            "vqwmat.a ind={} res={} rate={} gain={}",
            ind_a, res_a, rate_a, gain_a
        );
        println!(
            "vqwmat.b ind={} res={} rate={} gain={}",
            ind_b, res_b, rate_b, gain_b
        );
        println!(
            "vqwmat.c ind={} res={} rate={} gain={}",
            ind_c, res_c, rate_c, gain_c
        );
    }

    /* ============================================================= */
    /* 4. silk_quant_LTP_gains (arch 4 and arch 0)                   */
    /* ============================================================= */
    for cas in 0..3 {
        let nb_subfr = if cas == 2 { 2 } else { 4 };
        let mut xx_q17 = [0i32; (MAX_NB_SUBFR * 25) as usize];
        let mut xx_x_q17 = [0i32; (MAX_NB_SUBFR * 5) as usize];
        let mut b_a = [0i16; (MAX_NB_SUBFR * LTP_ORDER) as usize];
        let mut b_b = [0i16; (MAX_NB_SUBFR * LTP_ORDER) as usize];
        let mut idx_a = [0i8; MAX_NB_SUBFR as usize];
        let mut idx_b = [0i8; MAX_NB_SUBFR as usize];
        let mut per_a = 0i8;
        let mut per_b = 0i8;
        let mut pred_a = 0i32;
        let mut pred_b = 0i32;
        rng_seed(4000 + cas as u32);
        for i in 0..nb_subfr * 25 {
            xx_q17[i as usize] = rnd_range(-(1 << 20), 1 << 20);
        }
        for i in 0..nb_subfr * 5 {
            xx_x_q17[i as usize] = rnd_range(-(1 << 18), 1 << 18);
        }
        let mut sum_a = cas;
        let mut sum_b = cas;
        println!("ltpg case={} nb_subfr={}", cas, nb_subfr);
        pa32("ltpg.XX", &xx_q17[..(nb_subfr * 25) as usize]);
        pa32("ltpg.xX", &xx_x_q17[..(nb_subfr * 5) as usize]);
        silk_quant_LTP_gains(
            &mut b_a,
            &mut idx_a,
            &mut per_a,
            &mut sum_a,
            &mut pred_a,
            &xx_q17,
            &xx_x_q17,
            80,
            nb_subfr,
            4,
        );
        silk_quant_LTP_gains(
            &mut b_b,
            &mut idx_b,
            &mut per_b,
            &mut sum_b,
            &mut pred_b,
            &xx_q17,
            &xx_x_q17,
            80,
            nb_subfr,
            0,
        );
        pa16(
            "ltpg.B_arch4",
            &b_a[..(nb_subfr * LTP_ORDER) as usize],
        );
        pa8("ltpg.idx_arch4", &idx_a[..nb_subfr as usize]);
        println!(
            "ltpg.a per={} sum={} pred={}",
            per_a, sum_a, pred_a
        );
        pa16(
            "ltpg.B_arch0",
            &b_b[..(nb_subfr * LTP_ORDER) as usize],
        );
        pa8("ltpg.idx_arch0", &idx_b[..nb_subfr as usize]);
        println!(
            "ltpg.b per={} sum={} pred={}",
            per_b, sum_b, pred_b
        );
    }

    /* ============================================================= */
    /* 5. silk_A2NLSF                                                */
    /* ============================================================= */
    for cas in 0..4 {
        let d = if (cas & 1) != 0 { 10 } else { 16 };
        let mut a_q16 = [0i32; MAX_LPC_ORDER as usize];
        let mut nlsf = [0i16; MAX_LPC_ORDER as usize];
        rng_seed(5000 + cas as u32);
        for i in 0..d as usize {
            a_q16[i] = if cas >= 2 {
                0
            } else {
                rnd_range(-25000, 25000)
            };
        }
        println!("a2nlsf case={} d={}", cas, d);
        pa32("a2nlsf.a_in", &a_q16[..d as usize]);
        silk_A2NLSF(&mut nlsf, &mut a_q16, d);
        pa16("a2nlsf.NLSF", &nlsf[..d as usize]);
        pa32("a2nlsf.a_out", &a_q16[..d as usize]);
    }

    /* ============================================================= */
    /* 6. silk_NLSF_VQ_weights_laroia                                */
    /* ============================================================= */
    for cas in 0..4 {
        let d = if (cas & 1) != 0 { 10 } else { 16 };
        let mut nlsf = [0i16; MAX_LPC_ORDER as usize];
        let mut w = [0i16; MAX_LPC_ORDER as usize];
        rng_seed(6000 + cas as u32);
        for i in 0..d as usize {
            nlsf[i] = if cas == 2 {
                (i as i32 * (32768 / d)) as i16
            } else if cas == 3 {
                rnd_range(0, 32767) as i16
            } else {
                rnd_range(1, 32767) as i16
            };
        }
        println!("laroia case={} D={}", cas, d);
        pa16("laroia.nlsf", &nlsf[..d as usize]);
        silk_NLSF_VQ_weights_laroia(&mut w, &nlsf, d);
        pa16("laroia.w", &w[..d as usize]);
    }

    /* ============================================================= */
    /* 7. silk_NLSF_VQ                                               */
    /* ============================================================= */
    for cas in 0..2 {
        let cb = if cas == 0 {
            &silk_NLSF_CB_NB_MB
        } else {
            &silk_NLSF_CB_WB
        };
        let ord = cb.order as i32;
        let k = cb.nVectors as i32;
        let mut in_q15 = [0i16; MAX_LPC_ORDER as usize];
        let mut err = [0i32; 64];
        rng_seed(7000 + cas as u32);
        for i in 0..ord as usize {
            in_q15[i] = rnd_range(0, 32767) as i16;
        }
        println!("nlsf_vq case={} order={} K={}", cas, ord, k);
        pa16("nlsf_vq.in", &in_q15[..ord as usize]);
        silk_NLSF_VQ(
            &mut err,
            &in_q15,
            cb.CB1_NLSF_Q8,
            cb.CB1_Wght_Q9,
            k,
            ord,
        );
        pa32("nlsf_vq.err", &err[..k as usize]);
    }

    /* ============================================================= */
    /* 8. silk_NLSF_del_dec_quant                                    */
    /* ============================================================= */
    for cas in 0..4 {
        let cb = if (cas & 1) != 0 {
            &silk_NLSF_CB_WB
        } else {
            &silk_NLSF_CB_NB_MB
        };
        let ord = cb.order as i32;
        let mut x_q10 = [0i16; MAX_LPC_ORDER as usize];
        let mut w_q5 = [0i16; MAX_LPC_ORDER as usize];
        let mut ec_ix = [0i16; MAX_LPC_ORDER as usize];
        let mut pred_q8 = [0u8; MAX_LPC_ORDER as usize];
        let mut indices = [0i8; MAX_LPC_ORDER as usize];
        rng_seed(8000 + cas as u32);
        for i in 0..ord as usize {
            x_q10[i] = rnd_range(-20000, 20000) as i16;
            w_q5[i] = rnd_range(1, 4000) as i16;
        }
        let mu = if cas < 2 { 0 } else { rnd_range(0, 32767) };
        silk_NLSF_unpack(&mut ec_ix, &mut pred_q8, cb, cas & 3);
        println!("nlsf_ddq case={} order={} mu={}", cas, ord, mu);
        pa16("nlsf_ddq.x", &x_q10[..ord as usize]);
        pa16("nlsf_ddq.w", &w_q5[..ord as usize]);
        pau8("nlsf_ddq.pred", &pred_q8[..ord as usize]);
        pa16("nlsf_ddq.ec_ix", &ec_ix[..ord as usize]);
        let rd = silk_NLSF_del_dec_quant(
            &mut indices,
            &x_q10,
            &w_q5,
            &pred_q8,
            &ec_ix,
            cb.ec_Rates_Q5,
            cb.quantStepSize_Q16 as i32,
            cb.invQuantStepSize_Q6,
            mu,
            ord as i16,
        );
        pa8("nlsf_ddq.indices", &indices[..ord as usize]);
        p1s("nlsf_ddq.rd", rd);
    }

    /* ============================================================= */
    /* 9. silk_NLSF_encode                                           */
    /* ============================================================= */
    for cas in 0..4 {
        let cb = if (cas & 1) != 0 {
            &silk_NLSF_CB_WB
        } else {
            &silk_NLSF_CB_NB_MB
        };
        let ord = cb.order as i32;
        let mut nlsf = [0i16; MAX_LPC_ORDER as usize];
        let mut w = [0i16; MAX_LPC_ORDER as usize];
        let mut indices = [0i8; (MAX_LPC_ORDER + 1) as usize];
        let n_survivors = if cas < 2 { 4 } else { 16 };
        let signal_type = if cas == 3 { 2 } else { cas & 1 };
        let mu = 1000;
        rng_seed(9000 + cas as u32);
        /* Sorted NLSF input */
        {
            let mut acc = rnd_range(100, 800);
            for i in 0..ord as usize {
                nlsf[i] = acc as i16;
                acc += rnd_range(200, 1500);
                if acc > 32000 {
                    acc = 32000;
                }
            }
        }
        silk_NLSF_VQ_weights_laroia(&mut w, &nlsf, ord);
        println!(
            "nlsf_enc case={} order={} nSurv={} signalType={} mu={}",
            cas, ord, n_survivors, signal_type, mu
        );
        pa16("nlsf_enc.in", &nlsf[..ord as usize]);
        pa16("nlsf_enc.w", &w[..ord as usize]);
        let rd = silk_NLSF_encode(&mut indices, &mut nlsf, cb, &w, mu, n_survivors, signal_type);
        pa8("nlsf_enc.indices", &indices[..(ord + 1) as usize]);
        pa16("nlsf_enc.nlsf_out", &nlsf[..ord as usize]);
        p1s("nlsf_enc.rd", rd);
    }

    /* ============================================================= */
    /* 10. silk_process_NLSFs                                        */
    /* ============================================================= */
    for cas in 0..2 {
        let mut enc = silk_encoder_state::default();
        let mut pred_coef_q12 = [[0i16; MAX_LPC_ORDER as usize]; 2];
        let mut nlsf = [0i16; MAX_LPC_ORDER as usize];
        let mut prev = [0i16; MAX_LPC_ORDER as usize];
        rng_seed(10000 + cas as u32);
        enc.nb_subfr = if cas == 0 { 4 } else { 2 };
        enc.predictLPCOrder = 16;
        enc.arch = 4;
        enc.speech_activity_Q8 = if cas == 0 { 128 } else { 200 };
        enc.useInterpolatedNLSFs = 1;
        enc.NLSF_MSVQ_Survivors = 8;
        enc.indices.NLSFInterpCoef_Q2 = if cas == 0 { 2 } else { 4 };
        enc.indices.signalType = TYPE_VOICED as i8;
        enc.psNLSF_CB = &silk_NLSF_CB_WB;
        {
            let mut acc = 500;
            for i in 0..16usize {
                prev[i] = acc as i16;
                acc += 1400;
                nlsf[i] = (acc + rnd_range(-300, 300)) as i16;
            }
        }
        println!(
            "proc_nlsf case={} nb_subfr={} interpCoef={}",
            cas, enc.nb_subfr, enc.indices.NLSFInterpCoef_Q2
        );
        pa16("proc_nlsf.in", &nlsf);
        pa16("proc_nlsf.prev", &prev);
        silk_process_NLSFs(&mut enc, &mut pred_coef_q12, &mut nlsf, &prev);
        pa16("proc_nlsf.pred0", &pred_coef_q12[0]);
        pa16("proc_nlsf.pred1", &pred_coef_q12[1]);
        pa16("proc_nlsf.nlsf_out", &nlsf);
        pa8("proc_nlsf.indices", &enc.indices.NLSFIndices);
    }

    /* ============================================================= */
    /* 11. silk_encode_pulses                                        */
    /* ============================================================= */
    for cas in 0..6 {
        let fl = if cas == 4 {
            120
        } else if cas == 5 {
            160
        } else {
            320
        };
        let signal_type = cas % 3;
        let quant_offset_type = (cas / 3) & 1;
        let mut pulses = vec![0i8; (MAX_FRAME_LENGTH + SHELL_CODEC_FRAME_LENGTH) as usize];
        rng_seed(11000 + cas as u32);
        for i in 0..fl as usize {
            pulses[i] = if cas == 0 {
                rnd_range(-6, 6) as i8
            } else if cas == 1 {
                rnd_range(-16, 16) as i8
            } else if cas == 2 {
                if (cas & 1) != 0 {
                    127
                } else {
                    -128
                }
            } else {
                rnd_range(-20, 20) as i8
            };
        }
        println!(
            "enc_pulses case={} fl={} signalType={} qot={}",
            cas, fl, signal_type, quant_offset_type
        );
        pa8("enc_pulses.in", &pulses[..fl as usize]);
        let mut e = ec_enc_init(4096);
        silk_encode_pulses(
            &mut e,
            signal_type,
            quant_offset_type,
            &mut pulses,
            fl,
        );
        ec_dump(&format!("enc_pulses.{}", cas), &mut e);
    }

    /* ============================================================= */
    /* 12. silk_encode_indices                                       */
    /* ============================================================= */
    for cas in 0..4 {
        let mut enc = silk_encoder_state::default();
        let cond_coding = if cas == 0 {
            CODE_INDEPENDENTLY
        } else {
            CODE_CONDITIONALLY
        };
        let encode_lbrr = if cas == 3 { 1 } else { 0 };
        rng_seed(12000 + cas as u32);
        enc.nb_subfr = if (cas & 1) != 0 { 4 } else { 2 };
        enc.fs_kHz = 16;
        enc.predictLPCOrder = 16;
        enc.arch = 4;
        enc.psNLSF_CB = &silk_NLSF_CB_WB;
        enc.pitch_lag_low_bits_iCDF = &silk_uniform8_iCDF;
        enc.pitch_contour_iCDF = &silk_pitch_contour_iCDF;
        enc.ec_prevSignalType = if cas == 1 {
            TYPE_VOICED
        } else {
            TYPE_NO_VOICE_ACTIVITY
        };
        enc.ec_prevLagIndex = 40;
        enc.indices.signalType = TYPE_VOICED as i8;
        enc.indices.quantOffsetType = (cas & 1) as i8;
        enc.indices.NLSFInterpCoef_Q2 = 4;
        enc.indices.Seed = 3;
        enc.indices.GainsIndices[0] = rnd_range(0, 40) as i8;
        for i in 1..MAX_NB_SUBFR as usize {
            enc.indices.GainsIndices[i] = rnd_range(0, 40) as i8;
        }
        enc.indices.NLSFIndices[0] = rnd_range(0, 10) as i8;
        for i in 1..=16usize {
            enc.indices.NLSFIndices[i] = rnd_range(-10, 10) as i8;
        }
        enc.indices.lagIndex = rnd_range(20, 140) as i16;
        enc.indices.contourIndex =
            rnd_range(0, if enc.nb_subfr == MAX_NB_SUBFR { 30 } else { 10 }) as i8;
        enc.indices.PERIndex = rnd_range(0, 2) as i8;
        for i in 0..MAX_NB_SUBFR as usize {
            enc.indices.LTPIndex[i] = rnd_range(0, (8 << enc.indices.PERIndex) - 1) as i8;
        }
        enc.indices.LTP_scaleIndex = if cond_coding == CODE_INDEPENDENTLY {
            rnd_range(0, 2) as i8
        } else {
            0
        };
        enc.indices_LBRR[0] = enc.indices;
        enc.nFramesPerPacket = 1;
        enc.LBRR_flags[0] = 1;
        println!(
            "enc_indices case={} condCoding={} LBRR={}",
            cas, cond_coding, encode_lbrr
        );
        pa8("enc_indices.gains", &enc.indices.GainsIndices);
        pa8("enc_indices.nlsf", &enc.indices.NLSFIndices);
        p1s("enc_indices.lagIndex", enc.indices.lagIndex as i32);
        p1s("enc_indices.contourIndex", enc.indices.contourIndex as i32);
        p1s("enc_indices.PERIndex", enc.indices.PERIndex as i32);
        pa8("enc_indices.ltp", &enc.indices.LTPIndex);
        p1s("enc_indices.ltpScale", enc.indices.LTP_scaleIndex as i32);
        let mut e = ec_enc_init(4096);
        silk_encode_indices(&mut enc, &mut e, 0, encode_lbrr, cond_coding);
        ec_dump(&format!("enc_indices.{}", cas), &mut e);
        p1s("enc_indices.ec_prevSignalType", enc.ec_prevSignalType);
        p1s("enc_indices.ec_prevLagIndex", enc.ec_prevLagIndex as i32);
    }

    /* ============================================================= */
    /* 13. stereo_encode_pred / stereo_encode_mid_only               */
    /* ============================================================= */
    for cas in 0..2 {
        let mut ix = [[0i8; 3]; 2];
        rng_seed(13000 + cas as u32);
        for i in 0..2 {
            ix[i][0] = rnd_below(3) as i8;
            ix[i][1] = rnd_below(STEREO_QUANT_SUB_STEPS) as i8;
            ix[i][2] = rnd_below(5) as i8;
        }
        println!("stereo_pred case={}", cas);
        pa8("stereo_pred.ix0", &ix[0]);
        pa8("stereo_pred.ix1", &ix[1]);
        let mut e = ec_enc_init(1024);
        silk_stereo_encode_pred(&mut e, &ix);
        silk_stereo_encode_mid_only(&mut e, (cas & 1) as i8);
        ec_dump(&format!("stereo_pred.{}", cas), &mut e);
    }

    /* ============================================================= */
    /* 14. silk_stereo_quant_pred                                    */
    /* ============================================================= */
    for cas in 0..4 {
        let mut pred = [0i32; 2];
        let mut ix = [[0i8; 3]; 2];
        rng_seed(14000 + cas as u32);
        if cas == 0 {
            pred[0] = 0;
            pred[1] = 0;
        } else if cas == 1 {
            pred[0] = 1 << 14;
            pred[1] = -(1 << 14);
        } else {
            pred[0] = rnd_range(-(1 << 14), 1 << 14);
            pred[1] = rnd_range(-(1 << 14), 1 << 14);
        }
        println!("stereo_quant case={}", cas);
        pa32("stereo_quant.pred_in", &pred);
        silk_stereo_quant_pred(&mut pred, &mut ix);
        pa32("stereo_quant.pred_out", &pred);
        pa8("stereo_quant.ix0", &ix[0]);
        pa8("stereo_quant.ix1", &ix[1]);
    }

    /* ============================================================= */
    /* 15. silk_stereo_find_predictor                                */
    /* ============================================================= */
    for cas in 0..3 {
        let mut x = [0i16; 160];
        let mut y = [0i16; 160];
        let mut amps = [0i32; 2];
        let mut ratio = 0i32;
        rng_seed(15000 + cas as u32);
        for i in 0..160 {
            x[i] = rnd_range(-3000, 3000) as i16;
            y[i] = if cas == 0 {
                (x[i] / 2) as i16
            } else if cas == 1 {
                (-x[i]) as i16
            } else {
                rnd_range(-3000, 3000) as i16
            };
        }
        amps[0] = if cas == 2 { 0 } else { rnd_below(1 << 20) };
        amps[1] = if cas == 2 { 0 } else { rnd_below(1 << 20) };
        println!("stereo_findp case={}", cas);
        pa16("stereo_findp.x", &x[..40]);
        pa16("stereo_findp.y", &y[..40]);
        pa32("stereo_findp.amps_in", &amps);
        let pred = silk_stereo_find_predictor(&mut ratio, &x, &y, &mut amps, 160, 1000);
        p1s("stereo_findp.pred", pred);
        p1s("stereo_findp.ratio", ratio);
        pa32("stereo_findp.amps_out", &amps);
    }

    /* ============================================================= */
    /* 16. silk_stereo_LR_to_MS                                      */
    /* ============================================================= */
    for cas in 0..2 {
        let mut st = stereo_enc_state::default();
        let fl = if cas == 0 { 320 } else { 160 };
        let mut x1 = vec![0i16; (MAX_FRAME_LENGTH + 2) as usize];
        let mut x2 = vec![0i16; (MAX_FRAME_LENGTH + 2) as usize];
        rng_seed(16000 + cas as u32);
        for frame in 0..3 {
            let mut ix = [[0i8; 3]; 2];
            let mut mid_only_flag = 0i8;
            let mut rates = [0i32; 2];
            for i in 0..fl as usize {
                x1[i + 2] = rnd_range(-20000, 20000) as i16;
                x2[i + 2] = (x1[i + 2] / (1 + (frame & 1))) as i16;
            }
            x1[0] = 0;
            x1[1] = 0;
            x2[0] = 0;
            x2[1] = 0;
            let to_mono = if frame == 2 { 1 } else { 0 };
            println!("stereo_lrms case={} frame={} fl={}", cas, frame, fl);
            let prev_speech = if st.smth_width_Q14 != 0 { 100 } else { 0 };
            /* The C passes `&buf[2]`; the Rust port takes the whole buffer. */
            silk_stereo_LR_to_MS(
                &mut st,
                &mut x1,
                &mut x2,
                &mut ix,
                &mut mid_only_flag,
                &mut rates,
                32000,
                prev_speech,
                to_mono,
                16,
                fl,
            );
            pa16(
                &format!("stereo_lrms.x1.{}.{}", cas, frame),
                &x1[..(fl + 2) as usize],
            );
            pa16(
                &format!("stereo_lrms.x2.{}.{}", cas, frame),
                &x2[..(fl + 2) as usize],
            );
            pa8(&format!("stereo_lrms.ix.{}.{}", cas, frame), &ix[0]);
            p1s(
                &format!("stereo_lrms.mid_only.{}.{}", cas, frame),
                mid_only_flag as i32,
            );
            pa32(&format!("stereo_lrms.rates.{}.{}", cas, frame), &rates);
            pa16(
                &format!("stereo_lrms.state.{}.{}", cas, frame),
                &st.pred_prev_Q13,
            );
            pa16(
                &format!("stereo_lrms.sMid.{}.{}", cas, frame),
                &st.sMid,
            );
            pa16(
                &format!("stereo_lrms.sSide.{}.{}", cas, frame),
                &st.sSide,
            );
            pa32(
                &format!("stereo_lrms.mid_side_amp_Q0.{}.{}", cas, frame),
                &st.mid_side_amp_Q0,
            );
            p1s(
                &format!("stereo_lrms.smth_width.{}.{}", cas, frame),
                st.smth_width_Q14 as i32,
            );
            p1s(
                &format!("stereo_lrms.width_prev.{}.{}", cas, frame),
                st.width_prev_Q14 as i32,
            );
            p1s(
                &format!("stereo_lrms.silent_side_len.{}.{}", cas, frame),
                st.silent_side_len as i32,
            );
        }
    }

    /* ============================================================= */
    /* 17. silk_init_encoder / control_encoder / audio_bandwidth / SNR */
    /* ============================================================= */
    {
        for cas in 0..6 {
            let mut enc = silk_encoder_state_FLP::default();
            let mut ctl = silk_EncControlStruct::default();
            let ret = silk_init_encoder(&mut enc, 4);
            println!(
                "init_enc case={} ret={} arch={} hp1={} hp2={} ffar={} counter={}",
                cas,
                ret,
                enc.sCmn.arch,
                enc.sCmn.variable_HP_smth1_Q15,
                enc.sCmn.variable_HP_smth2_Q15,
                enc.sCmn.first_frame_after_reset,
                enc.sCmn.sVAD.counter
            );
            let complexity = match cas {
                0 => 0,
                1 => 1,
                2 => 3,
                3 => 5,
                4 => 7,
                _ => 9,
            };
            let payload_ms = if (cas & 1) != 0 { 10 } else { 20 };
            ctl.nChannelsAPI = 1;
            ctl.nChannelsInternal = 1;
            ctl.API_sampleRate = 48000;
            ctl.maxInternalSampleRate = 16000;
            ctl.minInternalSampleRate = 8000;
            ctl.desiredInternalSampleRate = 16000;
            ctl.payloadSize_ms = payload_ms;
            ctl.bitRate = 32000;
            ctl.complexity = complexity;
            ctl.packetLossPercentage = 10;
            let ret = silk_control_encoder(&mut enc, &mut ctl, 0, 0, 0);
            println!(
                "ctl_enc case={} complexity={} payload_ms={} ret={}",
                cas, complexity, payload_ms, ret
            );
            p1s("ctl.fs_kHz", enc.sCmn.fs_kHz);
            p1s("ctl.nb_subfr", enc.sCmn.nb_subfr);
            p1s("ctl.frame_length", enc.sCmn.frame_length);
            p1s("ctl.subfr_length", enc.sCmn.subfr_length);
            p1s("ctl.ltp_mem_length", enc.sCmn.ltp_mem_length);
            p1s("ctl.la_pitch", enc.sCmn.la_pitch);
            p1s("ctl.la_shape", enc.sCmn.la_shape);
            p1s("ctl.shapeWinLength", enc.sCmn.shapeWinLength);
            p1s("ctl.max_pitch_lag", enc.sCmn.max_pitch_lag);
            p1s("ctl.pitch_LPC_win_length", enc.sCmn.pitch_LPC_win_length);
            p1s("ctl.nFramesPerPacket", enc.sCmn.nFramesPerPacket);
            p1s(
                "ctl.nStatesDelayedDecision",
                enc.sCmn.nStatesDelayedDecision,
            );
            p1s("ctl.shapingLPCOrder", enc.sCmn.shapingLPCOrder);
            p1s("ctl.predictLPCOrder", enc.sCmn.predictLPCOrder);
            p1s(
                "ctl.pitchEstimationComplexity",
                enc.sCmn.pitchEstimationComplexity,
            );
            p1s(
                "ctl.pitchEstimationLPCOrder",
                enc.sCmn.pitchEstimationLPCOrder,
            );
            p1s(
                "ctl.pitchEstimationThreshold_Q16",
                enc.sCmn.pitchEstimationThreshold_Q16,
            );
            p1s("ctl.warping_Q16", enc.sCmn.warping_Q16);
            p1s("ctl.useInterpolatedNLSFs", enc.sCmn.useInterpolatedNLSFs);
            p1s("ctl.NLSF_MSVQ_Survivors", enc.sCmn.NLSF_MSVQ_Survivors);
            p1s("ctl.prevLag", enc.sCmn.prevLag);
            p1s("ctl.prevSignalType", enc.sCmn.prevSignalType as i32);
            p1s(
                "ctl.controlled_since_last_payload",
                enc.sCmn.controlled_since_last_payload,
            );
            p1s("ctl.LBRR_GainIncreases", enc.sCmn.LBRR_GainIncreases);
            p1s("ctl.SNR_dB_Q7", enc.sCmn.SNR_dB_Q7);
            p1s("ctl.psNLSF_CB_nVectors", enc.sCmn.psNLSF_CB.nVectors as i32);
            p1s("ctl.psNLSF_CB_order", enc.sCmn.psNLSF_CB.order as i32);
            for k in 0..3 {
                let new_rate = if k == 0 {
                    64000
                } else if k == 1 {
                    12000
                } else {
                    6000
                };
                let fs = silk_control_audio_bandwidth(&mut enc.sCmn, &mut ctl);
                let ret = silk_control_SNR(&mut enc.sCmn, new_rate);
                println!("bw case={} k={} fs={} ret={} snr={} switchReady={} maxBits={} mode={} transition={}",
                         cas, k, fs, ret, enc.sCmn.SNR_dB_Q7, ctl.switchReady, ctl.maxBits,
                         enc.sCmn.sLP.mode, enc.sCmn.sLP.transition_frame_no);
                p1s("bw.In_LP_State0", enc.sCmn.sLP.In_LP_State[0]);
                p1s("bw.In_LP_State1", enc.sCmn.sLP.In_LP_State[1]);
                p1s("bw.TargetRate_bps", enc.sCmn.TargetRate_bps);
            }
            /* Second call with a changed API rate: exercises the early-return
               resampler-refresh path (float2short + resampler + short2float). */
            ctl.API_sampleRate = if (cas & 1) != 0 { 12000 } else { 24000 };
            let ret = silk_control_encoder(&mut enc, &mut ctl, 0, 0, 0);
            println!(
                "ctl_enc2 case={} ret={} api={} prev_api={} fs={}",
                cas, ret, ctl.API_sampleRate, enc.sCmn.prev_API_fs_Hz, enc.sCmn.fs_kHz
            );
            p1s(
                "ctl2.resampler_batchSize",
                enc.sCmn.resampler_state.batchSize,
            );
            p1s(
                "ctl2.resampler_FIR_Order",
                enc.sCmn.resampler_state.FIR_Order,
            );
            p1s(
                "ctl2.resampler_invRatio_Q16",
                enc.sCmn.resampler_state.invRatio_Q16,
            );
            /* Third call, same rate: no refresh. */
            let ret = silk_control_encoder(&mut enc, &mut ctl, 0, 0, 0);
            println!(
                "ctl_enc3 case={} ret={} api={} prev_api={}",
                cas, ret, ctl.API_sampleRate, enc.sCmn.prev_API_fs_Hz
            );
            /* Force a full re-configuration with the original API rate. */
            enc.sCmn.controlled_since_last_payload = 0;
            ctl.API_sampleRate = 48000;
            let ret = silk_control_encoder(&mut enc, &mut ctl, 0, 0, 0);
            println!(
                "ctl_enc4 case={} ret={} api={} prev_api={} fs={} nb_subfr={} frame_length={}",
                cas,
                ret,
                ctl.API_sampleRate,
                enc.sCmn.prev_API_fs_Hz,
                enc.sCmn.fs_kHz,
                enc.sCmn.nb_subfr,
                enc.sCmn.frame_length
            );
            p1s(
                "ctl4.resampler_batchSize",
                enc.sCmn.resampler_state.batchSize,
            );
            p1s(
                "ctl4.resampler_FIR_Order",
                enc.sCmn.resampler_state.FIR_Order,
            );
            p1s(
                "ctl4.resampler_invRatio_Q16",
                enc.sCmn.resampler_state.invRatio_Q16,
            );
            p1s(
                "ctl4.resampler_inputDelay",
                enc.sCmn.resampler_state.inputDelay,
            );
            let total_rate = enc.sCmn.TargetRate_bps;
            println!("ctl.total_rate={}", total_rate);
        }
        /* Unsupported packet size returns the error code but still configures. */
        {
            let mut enc = silk_encoder_state_FLP::default();
            let mut ctl = silk_EncControlStruct::default();
            silk_init_encoder(&mut enc, 4);
            ctl.nChannelsAPI = 1;
            ctl.nChannelsInternal = 1;
            ctl.API_sampleRate = 48000;
            ctl.maxInternalSampleRate = 16000;
            ctl.minInternalSampleRate = 8000;
            ctl.desiredInternalSampleRate = 16000;
            ctl.payloadSize_ms = 25;
            ctl.bitRate = 32000;
            ctl.complexity = 4;
            let ret = silk_control_encoder(&mut enc, &mut ctl, 0, 0, 0);
            println!(
                "ctl_bad_packet ret={} nFramesPerPacket={} nb_subfr={} frame_length={} packet_ms={}",
                ret,
                enc.sCmn.nFramesPerPacket,
                enc.sCmn.nb_subfr,
                enc.sCmn.frame_length,
                enc.sCmn.PacketSize_ms
            );
        }
    }

    /* ============================================================= */
    /* 18. silk_NSQ (arch 4) and silk_NSQ_del_dec (arch 4)           */
    /* ============================================================= */
    for cas in 0..14 {
        let mut enc = silk_encoder_state_FLP::default();
        let mut x16 = [0i16; MAX_FRAME_LENGTH as usize];
        let mut pulses = [0i8; MAX_FRAME_LENGTH as usize];
        let mut pred_coef_q12 = [[0i16; MAX_LPC_ORDER as usize]; 2];
        let mut ltp_coef_q14 = [0i16; (LTP_ORDER * MAX_NB_SUBFR) as usize];
        let mut ar_q13 = [0i16; (MAX_NB_SUBFR * MAX_SHAPE_LPC_ORDER) as usize];
        let mut harm_shape = [0i32; MAX_NB_SUBFR as usize];
        let mut tilt = [0i32; MAX_NB_SUBFR as usize];
        let mut lf_shp = [0i32; MAX_NB_SUBFR as usize];
        let mut gains = [0i32; MAX_NB_SUBFR as usize];
        let mut pitch_l = [0i32; MAX_NB_SUBFR as usize];
        let mut indices = SideInfoIndices::default();

        /* cas 0..3: silk_NSQ (nStates 1, warping 0); 4..11: del_dec.
           The table also sweeps Lambda_Q10 > 2048, which is the aggressive-RDO
           branch of the AVX2 quantiser (the default encoder reaches it). */
        const NSTATES_TAB: [i32; 14] = [1, 1, 1, 1, 1, 2, 3, 4, 4, 3, 4, 3, 4, 4];
        const LAMBDA_TAB: [i32; 14] = [
            0, 1024, 3000, 1024, 3000, 3000, 1024, 1024, 3000, 2600, 2600, 1024, 3000, 1024,
        ];
        const PAYLOAD_TAB: [i32; 14] = [20, 20, 20, 20, 20, 20, 20, 20, 20, 20, 10, 10, 20, 10];
        /* NLSFInterpCoef_Q2 == 4 -> LSF_interpolation_flag == 0 -> only
           subframe 0 is re-whitened and all subframes use PredCoef row 1. */
        const INTERP_TAB: [i32; 14] = [4, 2, 4, 2, 2, 4, 2, 4, 4, 2, 4, 2, 4, 2];
        let interp = INTERP_TAB[cas as usize];
        let use_deldec = cas >= 4;
        let n_states = NSTATES_TAB[cas as usize];
        let lambda_q10 = LAMBDA_TAB[cas as usize];
        setup_encoder(
            &mut enc,
            if (cas & 1) != 0 { 9 } else { 3 },
            PAYLOAD_TAB[cas as usize],
            1,
        );
        enc.sCmn.nStatesDelayedDecision = n_states;
        enc.sCmn.shapingLPCOrder = if n_states == 4 {
            24
        } else if n_states == 3 {
            20
        } else {
            12
        };
        enc.sCmn.useInterpolatedNLSFs = 1;
        let warping = if cas >= 6 {
            enc.sCmn.fs_kHz * SILK_FIX_CONST(0.015, 16)
        } else {
            0
        };
        enc.sCmn.warping_Q16 = warping;
        let nb_subfr = enc.sCmn.nb_subfr;
        let mut nsq = enc.sCmn.sNSQ;
        nsq.lagPrev = 100;
        nsq.prev_gain_Q16 = 65536;
        indices.signalType = if (cas & 1) != 0 {
            TYPE_VOICED as i8
        } else {
            TYPE_NO_VOICE_ACTIVITY as i8
        };
        indices.quantOffsetType = ((cas >> 1) & 1) as i8;
        indices.Seed = (cas & 3) as i8;
        indices.NLSFInterpCoef_Q2 = interp as i8;
        let ltp_scale_q14 = 15565;
        rng_seed(18000 + cas as u32);
        for i in 0..enc.sCmn.frame_length as usize {
            x16[i] = rnd_range(-8000, 8000) as i16;
        }
        for i in 0..(2 * MAX_LPC_ORDER) as usize {
            pred_coef_q12[i / MAX_LPC_ORDER as usize][i % MAX_LPC_ORDER as usize] =
                rnd_range(-1500, 1500) as i16;
        }
        for i in 0..(LTP_ORDER * MAX_NB_SUBFR) as usize {
            ltp_coef_q14[i] = rnd_range(-3000, 3000) as i16;
        }
        for i in 0..(MAX_NB_SUBFR * MAX_SHAPE_LPC_ORDER) as usize {
            ar_q13[i] = rnd_range(-1500, 1500) as i16;
        }
        for i in 0..MAX_NB_SUBFR as usize {
            harm_shape[i] = rnd_range(0, 3000);
            tilt[i] = rnd_range(-2000, 2000);
            lf_shp[i] = rnd_range(0, 30000);
            gains[i] = if cas >= 12 {
                rnd_range(82570, 1 << 30)
            } else {
                rnd_range(65536, 400000)
            };
            pitch_l[i] = 33 + 7 * cas; /* constant per case: everything read has been written */
        }
        println!(
            "nsq case={} deldec={} nStates={} warping={} signalType={} lambda={} interp={}",
            cas,
            use_deldec as i32,
            n_states,
            warping,
            indices.signalType as i32,
            lambda_q10,
            interp
        );
        println!(
            "nsq.frame_length={} nb_subfr={} subfr_length={} shapingLPC={} predictLPC={}",
            enc.sCmn.frame_length,
            nb_subfr,
            enc.sCmn.subfr_length,
            enc.sCmn.shapingLPCOrder,
            enc.sCmn.predictLPCOrder
        );
        pa16("nsq.x16", &x16[..enc.sCmn.frame_length as usize]);
        pa16("nsq.pred0", &pred_coef_q12[0]);
        pa16("nsq.pred1", &pred_coef_q12[1]);
        pa16("nsq.ltp", &ltp_coef_q14);
        pa16("nsq.ar", &ar_q13);
        pa32("nsq.harm", &harm_shape);
        pa32("nsq.tilt", &tilt);
        pa32("nsq.lf", &lf_shp);
        pa32("nsq.gains", &gains);
        pa32("nsq.pitchL", &pitch_l);

        if !use_deldec {
            silk_NSQ(
                &enc.sCmn,
                &mut nsq,
                &mut indices,
                &x16,
                &mut pulses,
                &pred_coef_q12,
                &ltp_coef_q14,
                &ar_q13,
                &harm_shape,
                &tilt,
                &lf_shp,
                &gains,
                &pitch_l,
                lambda_q10,
                ltp_scale_q14,
                4,
            );
        } else {
            silk_NSQ_del_dec(
                &enc.sCmn,
                &mut nsq,
                &mut indices,
                &x16,
                &mut pulses,
                &pred_coef_q12,
                &ltp_coef_q14,
                &ar_q13,
                &harm_shape,
                &tilt,
                &lf_shp,
                &gains,
                &pitch_l,
                lambda_q10,
                ltp_scale_q14,
                4,
            );
        }
        pa8("nsq.pulses", &pulses[..enc.sCmn.frame_length as usize]);
        pa8("nsq.seed_out", core::slice::from_ref(&indices.Seed));
        /* For the 10 ms frames, also run the scalar C body on a fresh copy so
           the AVX2 result can be cross-checked against it. */
        if enc.sCmn.frame_length == 160 {
            let mut nsq_c = enc.sCmn.sNSQ;
            let mut idx_c = indices;
            let mut pulses_c = [0i8; MAX_FRAME_LENGTH as usize];
            nsq_c.lagPrev = 100;
            nsq_c.prev_gain_Q16 = 65536;
            idx_c.Seed = (cas & 3) as i8;
            silk_NSQ_del_dec_c(
                &enc.sCmn,
                &mut nsq_c,
                &mut idx_c,
                &x16,
                &mut pulses_c,
                &pred_coef_q12,
                &ltp_coef_q14,
                &ar_q13,
                &harm_shape,
                &tilt,
                &lf_shp,
                &gains,
                &pitch_l,
                lambda_q10,
                ltp_scale_q14,
            );
            pa8("nsq.pulses_cbody", &pulses_c[..enc.sCmn.frame_length as usize]);
            print_nsq(
                "nsq.cbody",
                &nsq_c,
                enc.sCmn.ltp_mem_length,
                enc.sCmn.frame_length,
            );
            p1s("nsq.cbody_match", 0);
        }
        print_nsq(
            "nsq",
            &nsq,
            enc.sCmn.ltp_mem_length,
            enc.sCmn.frame_length,
        );
        let mut e = ec_enc_init(4096);
        silk_encode_pulses(
            &mut e,
            indices.signalType as i32,
            indices.quantOffsetType as i32,
            &mut pulses,
            enc.sCmn.frame_length,
        );
        ec_dump(&format!("nsq.ec.{}", cas), &mut e);
        println!("nsq.case_end={}", cas);
    }

    /* ============================================================= */
    /* 19. silk_NSQ_del_dec fallback vs AVX2 on identical input       */
    /* ============================================================= */
    for cas in 0..4 {
        let mut enc = silk_encoder_state_FLP::default();
        let mut x16 = [0i16; MAX_FRAME_LENGTH as usize];
        let mut pulses_a = [0i8; MAX_FRAME_LENGTH as usize];
        let mut pulses_b = [0i8; MAX_FRAME_LENGTH as usize];
        let mut pred_coef_q12 = [[0i16; MAX_LPC_ORDER as usize]; 2];
        let mut ltp_coef_q14 = [0i16; (LTP_ORDER * MAX_NB_SUBFR) as usize];
        let mut ar_q13 = [0i16; (MAX_NB_SUBFR * MAX_SHAPE_LPC_ORDER) as usize];
        let mut harm_shape = [0i32; MAX_NB_SUBFR as usize];
        let mut tilt = [0i32; MAX_NB_SUBFR as usize];
        let mut lf_shp = [0i32; MAX_NB_SUBFR as usize];
        let mut gains = [0i32; MAX_NB_SUBFR as usize];
        let mut pitch_l = [0i32; MAX_NB_SUBFR as usize];
        let n_states = cas + 1;
        setup_encoder(&mut enc, 9, 20, 1);
        enc.sCmn.nStatesDelayedDecision = n_states;
        enc.sCmn.shapingLPCOrder = if n_states >= 4 {
            24
        } else if n_states == 3 {
            20
        } else {
            12
        };
        enc.sCmn.warping_Q16 = if cas >= 2 {
            enc.sCmn.fs_kHz * SILK_FIX_CONST(0.015, 16)
        } else {
            0
        };
        let mut nsq_a = enc.sCmn.sNSQ;
        nsq_a.lagPrev = 100;
        nsq_a.prev_gain_Q16 = 65536;
        let mut nsq_b = nsq_a;
        let mut idx_a = SideInfoIndices::default();
        idx_a.signalType = TYPE_VOICED as i8;
        idx_a.Seed = 1;
        idx_a.NLSFInterpCoef_Q2 = 2;
        let mut idx_b = idx_a;
        rng_seed(19000 + cas as u32);
        for i in 0..enc.sCmn.frame_length as usize {
            x16[i] = rnd_range(-8000, 8000) as i16;
        }
        for i in 0..(2 * MAX_LPC_ORDER) as usize {
            pred_coef_q12[i / MAX_LPC_ORDER as usize][i % MAX_LPC_ORDER as usize] =
                rnd_range(-1500, 1500) as i16;
        }
        for i in 0..(LTP_ORDER * MAX_NB_SUBFR) as usize {
            ltp_coef_q14[i] = rnd_range(-3000, 3000) as i16;
        }
        for i in 0..(MAX_NB_SUBFR * MAX_SHAPE_LPC_ORDER) as usize {
            ar_q13[i] = rnd_range(-1500, 1500) as i16;
        }
        for i in 0..MAX_NB_SUBFR as usize {
            harm_shape[i] = rnd_range(0, 3000);
            tilt[i] = rnd_range(-2000, 2000);
            lf_shp[i] = rnd_range(0, 30000);
            gains[i] = if cas >= 12 {
                rnd_range(82570, 1 << 30)
            } else {
                rnd_range(65536, 400000)
            };
            pitch_l[i] = 35 + 25 * cas; /* constant per case: everything read has been written */
        }
        println!(
            "deldec_cmp case={} nStates={} warping={}",
            cas, n_states, enc.sCmn.warping_Q16
        );
        {
            let mut dd = DECISION_DELAY.min(enc.sCmn.subfr_length);
            for i in 0..enc.sCmn.nb_subfr as usize {
                dd = dd.min(pitch_l[i] - LTP_ORDER / 2 - 1);
            }
            println!("deldec_cmp.decisionDelay={}", dd);
        }
        pa32(
            "deldec_cmp.pitchL",
            &pitch_l[..enc.sCmn.nb_subfr as usize],
        );
        pa16("deldec_cmp.x16", &x16[..enc.sCmn.frame_length as usize]);
        println!(
            "deldec_cmp.nStates_field={}",
            enc.sCmn.nStatesDelayedDecision
        );
        silk_NSQ_del_dec_c(
            &enc.sCmn,
            &mut nsq_a,
            &mut idx_a,
            &x16,
            &mut pulses_a,
            &pred_coef_q12,
            &ltp_coef_q14,
            &ar_q13,
            &harm_shape,
            &tilt,
            &lf_shp,
            &gains,
            &pitch_l,
            1024,
            15565,
        );
        silk_NSQ_del_dec(
            &enc.sCmn,
            &mut nsq_b,
            &mut idx_b,
            &x16,
            &mut pulses_b,
            &pred_coef_q12,
            &ltp_coef_q14,
            &ar_q13,
            &harm_shape,
            &tilt,
            &lf_shp,
            &gains,
            &pitch_l,
            1024,
            15565,
            4,
        );
        pa8(
            "deldec_cmp.pulses_c",
            &pulses_a[..enc.sCmn.frame_length as usize],
        );
        pa8(
            "deldec_cmp.pulses_dispatched",
            &pulses_b[..enc.sCmn.frame_length as usize],
        );
        print_nsq(
            "deldec_cmp.nsq_c",
            &nsq_a,
            enc.sCmn.ltp_mem_length,
            enc.sCmn.frame_length,
        );
        print_nsq(
            "deldec_cmp.nsq_dispatched",
            &nsq_b,
            enc.sCmn.ltp_mem_length,
            enc.sCmn.frame_length,
        );
        p1s("deldec_cmp.seed_c", idx_a.Seed as i32);
        p1s("deldec_cmp.seed_dispatched", idx_b.Seed as i32);
    }

    println!("probe_silkenc: END");
}

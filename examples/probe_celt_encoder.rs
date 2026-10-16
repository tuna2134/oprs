//! Rust twin of `compat/c/probe_celt_encoder.c`.
//!
//! Run with `bash compat/scripts/run-probe.sh celt_encoder`; the outputs must
//! be byte-identical.

use oprs::celt::celt::{
    AnalysisInfo, CELT_SET_CHANNELS_REQUEST, CELT_SET_END_BAND_REQUEST,
    CELT_SET_INPUT_CLIPPING_REQUEST, CELT_SET_PREDICTION_REQUEST, CELT_SET_SIGNALLING_REQUEST,
    CELT_SET_START_BAND_REQUEST, OPUS_SET_ENERGY_MASK_REQUEST, OPUS_SET_LFE_REQUEST, LEAK_BANDS,
};
use oprs::celt::celt_encoder::{
    celt_encode_with_ec, celt_encoder_get_size, opus_custom_encode, opus_custom_encode_float,
    opus_custom_encoder_create, opus_custom_encoder_ctl, opus_custom_encoder_ctl_analysis,
    opus_custom_encoder_ctl_energy_mask, opus_custom_encoder_get_size, OpusCustomEncoder,
    SIZEOF_CELT_ENCODER,
};
use oprs::celt::entcode::ec_tell_frac;
use oprs::celt::entenc::ec_enc_init;
use oprs::celt::modes::opus_custom_mode_create;
use oprs::ffi::{
    OPUS_SET_BITRATE_REQUEST, OPUS_SET_COMPLEXITY_REQUEST, OPUS_SET_LSB_DEPTH_REQUEST,
    OPUS_SET_PACKET_LOSS_PERC_REQUEST, OPUS_SET_PHASE_INVERSION_DISABLED_REQUEST,
    OPUS_SET_VBR_CONSTRAINT_REQUEST, OPUS_SET_VBR_REQUEST,
};

const MAXPKT: usize = 6000;

/* ------------------------------------------------------------------ */
/* deterministic input                                                */
/* ------------------------------------------------------------------ */

fn lcg_next(s: &mut u32) -> u32 {
    *s = s.wrapping_mul(1103515245).wrapping_add(12345);
    *s
}

fn fnv_u8(p: &[u8]) -> u64 {
    let mut h: u64 = 1469598103934665603;
    for &b in p {
        h ^= b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    h
}

fn fnv_i16(p: &[i16]) -> u64 {
    let mut h: u64 = 1469598103934665603;
    for &v in p {
        let v = v as u16;
        h ^= (v & 0xff) as u64;
        h = h.wrapping_mul(1099511628211);
        h ^= ((v >> 8) & 0xff) as u64;
        h = h.wrapping_mul(1099511628211);
    }
    h
}

fn fnv_f32(p: &[f32]) -> u64 {
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

fn gen_pcm_i16(p: &mut [i16], frame_idx: i32, loud: i32, s: &mut u32) {
    let n = p.len() as i32;
    let g: i32 = if frame_idx % 3 == 0 {
        256
    } else if frame_idx % 3 == 1 {
        96
    } else {
        32
    };
    if loud >= 2 {
        /* Pure tone: period 100 / 30 / 400 samples. */
        let period = if loud == 2 {
            100.0f64
        } else if loud == 3 {
            30.0f64
        } else {
            400.0f64
        };
        for i in 0..p.len() {
            p[i] = (((2.0f64 * std::f64::consts::PI * i as f64 / period).sin()) * 30000.0) as i16;
        }
        return;
    }
    for i in 0..p.len() {
        let raw = ((lcg_next(s) >> 16) as u16 as i16) as i32;
        let mut v = (raw * g) >> 8;
        if (frame_idx & 1) != 0 && (i as i32) > n / 2 {
            v = (v >> 2) + if loud != 0 { 4000 } else { -2000 };
        }
        if v > 32767 {
            v = 32767;
        }
        if v < -32768 {
            v = -32768;
        }
        p[i] = v as i16;
    }
}

/* ------------------------------------------------------------------ */
/* cases                                                              */
/* ------------------------------------------------------------------ */

#[derive(Clone, Copy)]
struct Case {
    LM: i32,
    CC: i32,
    C: i32,
    start: i32,
    end: i32,
    nbBytes: i32,
    vbr: i32,
    cvbr: i32,
    bitrate: i32,
    complexity: i32,
    lsb: i32,
    pred: i32,
    lfe: i32,
    signalling: i32,
    loss: i32,
    inv: i32,
    clip: i32,
    entry: i32,
    analysis: i32,
    mask: i32,
    frames: i32,
    loud: i32,
    seed: u32,
}

static CASES: [Case; 61] = [
    Case { LM: 0, CC: 1, C: 1, start: 0, end: 21, nbBytes: 40, vbr: 0, cvbr: 0, bitrate: -1, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 0, analysis: 0, mask: 0, frames: 3, loud: 0, seed: 101 },
    Case { LM: 0, CC: 1, C: 1, start: 0, end: 21, nbBytes: 200, vbr: 0, cvbr: 0, bitrate: -1, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 0, analysis: 0, mask: 0, frames: 3, loud: 0, seed: 102 },
    Case { LM: 0, CC: 2, C: 2, start: 0, end: 21, nbBytes: 200, vbr: 0, cvbr: 0, bitrate: -1, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 0, analysis: 0, mask: 0, frames: 3, loud: 0, seed: 103 },
    Case { LM: 1, CC: 1, C: 1, start: 0, end: 21, nbBytes: 100, vbr: 0, cvbr: 0, bitrate: -1, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 0, analysis: 0, mask: 0, frames: 3, loud: 0, seed: 104 },
    Case { LM: 1, CC: 2, C: 2, start: 0, end: 21, nbBytes: 250, vbr: 0, cvbr: 0, bitrate: -1, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 0, analysis: 0, mask: 0, frames: 3, loud: 0, seed: 105 },
    Case { LM: 1, CC: 2, C: 1, start: 0, end: 21, nbBytes: 250, vbr: 0, cvbr: 0, bitrate: -1, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 0, analysis: 0, mask: 0, frames: 3, loud: 0, seed: 106 },
    Case { LM: 2, CC: 1, C: 1, start: 0, end: 21, nbBytes: 150, vbr: 0, cvbr: 0, bitrate: -1, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 0, analysis: 0, mask: 0, frames: 3, loud: 0, seed: 107 },
    Case { LM: 2, CC: 2, C: 2, start: 0, end: 21, nbBytes: 400, vbr: 0, cvbr: 0, bitrate: -1, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 0, analysis: 0, mask: 0, frames: 3, loud: 0, seed: 108 },
    Case { LM: 3, CC: 1, C: 1, start: 0, end: 21, nbBytes: 400, vbr: 0, cvbr: 0, bitrate: -1, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 0, analysis: 0, mask: 0, frames: 3, loud: 0, seed: 109 },
    Case { LM: 3, CC: 2, C: 2, start: 0, end: 21, nbBytes: 800, vbr: 0, cvbr: 0, bitrate: -1, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 0, analysis: 0, mask: 0, frames: 3, loud: 0, seed: 110 },
    Case { LM: 3, CC: 1, C: 1, start: 0, end: 21, nbBytes: 4000, vbr: 0, cvbr: 0, bitrate: 32000, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 0, analysis: 0, mask: 0, frames: 3, loud: 0, seed: 111 },
    Case { LM: 3, CC: 1, C: 1, start: 0, end: 21, nbBytes: 4000, vbr: 0, cvbr: 0, bitrate: 64000, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 0, analysis: 0, mask: 0, frames: 3, loud: 0, seed: 112 },
    Case { LM: 2, CC: 2, C: 2, start: 0, end: 21, nbBytes: 4000, vbr: 0, cvbr: 0, bitrate: 48000, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 0, analysis: 0, mask: 0, frames: 3, loud: 0, seed: 113 },
    Case { LM: 3, CC: 1, C: 1, start: 0, end: 21, nbBytes: 4000, vbr: 0, cvbr: 0, bitrate: 1000000, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 0, analysis: 0, mask: 0, frames: 3, loud: 0, seed: 114 },
    Case { LM: 3, CC: 1, C: 1, start: 0, end: 21, nbBytes: 4000, vbr: 1, cvbr: 0, bitrate: 64000, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 0, analysis: 0, mask: 0, frames: 3, loud: 0, seed: 115 },
    Case { LM: 3, CC: 1, C: 1, start: 0, end: 21, nbBytes: 4000, vbr: 1, cvbr: 1, bitrate: 64000, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 0, analysis: 0, mask: 0, frames: 3, loud: 0, seed: 116 },
    Case { LM: 2, CC: 2, C: 2, start: 0, end: 21, nbBytes: 4000, vbr: 1, cvbr: 1, bitrate: 32000, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 0, analysis: 0, mask: 0, frames: 3, loud: 0, seed: 117 },
    Case { LM: 1, CC: 1, C: 1, start: 0, end: 21, nbBytes: 4000, vbr: 1, cvbr: 1, bitrate: 16000, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 0, analysis: 0, mask: 0, frames: 3, loud: 0, seed: 118 },
    Case { LM: 3, CC: 2, C: 2, start: 0, end: 21, nbBytes: 4000, vbr: 1, cvbr: 1, bitrate: 128000, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 0, analysis: 0, mask: 0, frames: 3, loud: 0, seed: 119 },
    Case { LM: 3, CC: 1, C: 1, start: 0, end: 21, nbBytes: 400, vbr: 0, cvbr: 0, bitrate: -1, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 1, analysis: 0, mask: 0, frames: 3, loud: 0, seed: 120 },
    Case { LM: 3, CC: 1, C: 1, start: 0, end: 21, nbBytes: 400, vbr: 0, cvbr: 0, bitrate: -1, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 1, analysis: 1, mask: 0, frames: 3, loud: 0, seed: 121 },
    Case { LM: 3, CC: 2, C: 2, start: 0, end: 21, nbBytes: 400, vbr: 0, cvbr: 0, bitrate: -1, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 2, analysis: 0, mask: 0, frames: 3, loud: 0, seed: 122 },
    Case { LM: 3, CC: 2, C: 2, start: 0, end: 21, nbBytes: 4000, vbr: 1, cvbr: 1, bitrate: 48000, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 2, analysis: 0, mask: 0, frames: 4, loud: 0, seed: 123 },
    Case { LM: 2, CC: 1, C: 1, start: 0, end: 21, nbBytes: 50, vbr: 0, cvbr: 0, bitrate: -1, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 2, analysis: 0, mask: 0, frames: 3, loud: 0, seed: 124 },
    Case { LM: 3, CC: 1, C: 1, start: 0, end: 21, nbBytes: 4000, vbr: 0, cvbr: 0, bitrate: 96000, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 2, analysis: 0, mask: 0, frames: 2, loud: 0, seed: 125 },
    Case { LM: 3, CC: 1, C: 1, start: 0, end: 21, nbBytes: 3, vbr: 0, cvbr: 0, bitrate: -1, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 0, analysis: 0, mask: 0, frames: 3, loud: 0, seed: 126 },
    Case { LM: 3, CC: 1, C: 1, start: 0, end: 21, nbBytes: 4, vbr: 0, cvbr: 0, bitrate: -1, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 0, analysis: 0, mask: 0, frames: 3, loud: 0, seed: 127 },
    Case { LM: 3, CC: 2, C: 2, start: 0, end: 21, nbBytes: 5, vbr: 0, cvbr: 0, bitrate: -1, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 0, analysis: 0, mask: 0, frames: 3, loud: 0, seed: 128 },
    Case { LM: 3, CC: 1, C: 1, start: 5, end: 21, nbBytes: 2000, vbr: 0, cvbr: 0, bitrate: -1, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 0, analysis: 0, mask: 0, frames: 3, loud: 0, seed: 129 },
    Case { LM: 3, CC: 2, C: 2, start: 3, end: 18, nbBytes: 2000, vbr: 0, cvbr: 0, bitrate: -1, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 0, analysis: 0, mask: 0, frames: 3, loud: 0, seed: 130 },
    Case { LM: 2, CC: 1, C: 1, start: 0, end: 10, nbBytes: 400, vbr: 0, cvbr: 0, bitrate: -1, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 0, analysis: 0, mask: 0, frames: 3, loud: 0, seed: 131 },
    Case { LM: 2, CC: 2, C: 2, start: 0, end: 4, nbBytes: 300, vbr: 0, cvbr: 0, bitrate: -1, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 0, analysis: 0, mask: 0, frames: 3, loud: 0, seed: 132 },
    Case { LM: 3, CC: 1, C: 1, start: 0, end: 21, nbBytes: 800, vbr: 0, cvbr: 0, bitrate: -1, complexity: 0, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 0, analysis: 0, mask: 0, frames: 3, loud: 0, seed: 133 },
    Case { LM: 3, CC: 1, C: 1, start: 0, end: 21, nbBytes: 800, vbr: 0, cvbr: 0, bitrate: -1, complexity: 1, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 0, analysis: 0, mask: 0, frames: 3, loud: 0, seed: 134 },
    Case { LM: 3, CC: 1, C: 1, start: 0, end: 21, nbBytes: 800, vbr: 0, cvbr: 0, bitrate: -1, complexity: 2, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 0, analysis: 0, mask: 0, frames: 3, loud: 0, seed: 135 },
    Case { LM: 3, CC: 1, C: 1, start: 0, end: 21, nbBytes: 800, vbr: 0, cvbr: 0, bitrate: -1, complexity: 8, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 0, analysis: 0, mask: 0, frames: 3, loud: 0, seed: 136 },
    Case { LM: 3, CC: 1, C: 1, start: 0, end: 21, nbBytes: 800, vbr: 0, cvbr: 0, bitrate: -1, complexity: 10, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 0, analysis: 0, mask: 0, frames: 3, loud: 0, seed: 137 },
    Case { LM: 3, CC: 1, C: 1, start: 0, end: 21, nbBytes: 800, vbr: 0, cvbr: 0, bitrate: -1, complexity: 5, lsb: 8, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 0, analysis: 0, mask: 0, frames: 3, loud: 0, seed: 138 },
    Case { LM: 3, CC: 1, C: 1, start: 0, end: 21, nbBytes: 800, vbr: 0, cvbr: 0, bitrate: -1, complexity: 5, lsb: 16, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 0, analysis: 0, mask: 0, frames: 3, loud: 0, seed: 139 },
    Case { LM: 3, CC: 1, C: 1, start: 0, end: 21, nbBytes: 800, vbr: 0, cvbr: 0, bitrate: -1, complexity: 5, lsb: 24, pred: 0, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 0, analysis: 0, mask: 0, frames: 3, loud: 0, seed: 140 },
    Case { LM: 3, CC: 1, C: 1, start: 0, end: 21, nbBytes: 800, vbr: 0, cvbr: 0, bitrate: -1, complexity: 5, lsb: 24, pred: 1, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 0, analysis: 0, mask: 0, frames: 3, loud: 0, seed: 141 },
    Case { LM: 3, CC: 1, C: 1, start: 0, end: 21, nbBytes: 800, vbr: 0, cvbr: 0, bitrate: -1, complexity: 5, lsb: 24, pred: 2, lfe: 1, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 0, analysis: 0, mask: 0, frames: 3, loud: 0, seed: 142 },
    Case { LM: 3, CC: 1, C: 1, start: 0, end: 21, nbBytes: 800, vbr: 0, cvbr: 0, bitrate: -1, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 5, inv: 0, clip: 0, entry: 0, analysis: 0, mask: 0, frames: 3, loud: 0, seed: 143 },
    Case { LM: 3, CC: 1, C: 1, start: 0, end: 21, nbBytes: 800, vbr: 0, cvbr: 0, bitrate: -1, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 15, inv: 0, clip: 0, entry: 0, analysis: 0, mask: 0, frames: 3, loud: 0, seed: 144 },
    Case { LM: 3, CC: 2, C: 2, start: 0, end: 21, nbBytes: 800, vbr: 0, cvbr: 0, bitrate: -1, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 1, clip: 0, entry: 0, analysis: 0, mask: 0, frames: 3, loud: 0, seed: 145 },
    Case { LM: 3, CC: 2, C: 2, start: 0, end: 21, nbBytes: 1200, vbr: 0, cvbr: 0, bitrate: -1, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 1, entry: 0, analysis: 0, mask: 0, frames: 3, loud: 0, seed: 146 },
    Case { LM: 3, CC: 2, C: 2, start: 0, end: 21, nbBytes: 1200, vbr: 0, cvbr: 0, bitrate: -1, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 1, entry: 1, analysis: 0, mask: 0, frames: 3, loud: 1, seed: 147 },
    Case { LM: 3, CC: 1, C: 1, start: 0, end: 21, nbBytes: 1200, vbr: 0, cvbr: 0, bitrate: -1, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 0, analysis: 1, mask: 0, frames: 3, loud: 0, seed: 148 },
    Case { LM: 3, CC: 2, C: 2, start: 0, end: 21, nbBytes: 2000, vbr: 1, cvbr: 1, bitrate: 64000, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 0, analysis: 1, mask: 0, frames: 3, loud: 0, seed: 149 },
    Case { LM: 3, CC: 1, C: 1, start: 0, end: 21, nbBytes: 1200, vbr: 0, cvbr: 0, bitrate: -1, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 0, analysis: 0, mask: 1, frames: 3, loud: 0, seed: 150 },
    Case { LM: 3, CC: 1, C: 1, start: 0, end: 21, nbBytes: 1200, vbr: 0, cvbr: 0, bitrate: -1, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 0, analysis: 0, mask: 1, frames: 3, loud: 1, seed: 151 },
    Case { LM: 3, CC: 1, C: 1, start: 0, end: 21, nbBytes: 1200, vbr: 0, cvbr: 0, bitrate: -1, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 0, analysis: 0, mask: 0, frames: 3, loud: 2, seed: 152 },
    Case { LM: 3, CC: 1, C: 1, start: 0, end: 21, nbBytes: 2000, vbr: 0, cvbr: 0, bitrate: -1, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 2, analysis: 0, mask: 0, frames: 2, loud: 2, seed: 153 },
    Case { LM: 2, CC: 2, C: 2, start: 0, end: 21, nbBytes: 2000, vbr: 0, cvbr: 0, bitrate: -1, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 1, analysis: 0, mask: 0, frames: 3, loud: 3, seed: 154 },
    Case { LM: 3, CC: 1, C: 1, start: 0, end: 21, nbBytes: 1500, vbr: 0, cvbr: 0, bitrate: -1, complexity: 8, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 0, analysis: 0, mask: 0, frames: 3, loud: 4, seed: 155 },
    Case { LM: 3, CC: 2, C: 2, start: 3, end: 18, nbBytes: 1500, vbr: 0, cvbr: 0, bitrate: -1, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 0, analysis: 1, mask: 0, frames: 2, loud: 2, seed: 156 },
    Case { LM: 3, CC: 2, C: 2, start: 0, end: 21, nbBytes: 4000, vbr: 0, cvbr: 0, bitrate: -1, complexity: 10, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 0, analysis: 0, mask: 0, frames: 3, loud: 0, seed: 157 },
    Case { LM: 3, CC: 2, C: 2, start: 0, end: 21, nbBytes: 4000, vbr: 0, cvbr: 0, bitrate: -1, complexity: 8, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 2, analysis: 0, mask: 0, frames: 2, loud: 0, seed: 158 },
    Case { LM: 3, CC: 1, C: 1, start: 17, end: 21, nbBytes: 10, vbr: 0, cvbr: 0, bitrate: -1, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 1, loss: 0, inv: 0, clip: 0, entry: 0, analysis: 0, mask: 0, frames: 3, loud: 0, seed: 159 },
    Case { LM: 3, CC: 1, C: 1, start: 0, end: 21, nbBytes: 400, vbr: 0, cvbr: 0, bitrate: -1, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 0, loss: 0, inv: 0, clip: 0, entry: 0, analysis: 0, mask: 0, frames: 3, loud: 0, seed: 160 },
    Case { LM: 3, CC: 2, C: 2, start: 0, end: 21, nbBytes: 800, vbr: 0, cvbr: 0, bitrate: -1, complexity: 5, lsb: 24, pred: 2, lfe: 0, signalling: 0, loss: 0, inv: 0, clip: 0, entry: 2, analysis: 0, mask: 0, frames: 3, loud: 0, seed: 161 },
];

/* ------------------------------------------------------------------ */
/* state / range-coder printing                                       */
/* ------------------------------------------------------------------ */

fn print_bands(name: &str, v: &[f32]) {
    print!("{}", name);
    for &x in v {
        print!(" {:08x}", x.to_bits());
    }
    println!();
}

fn print_state(st: &OpusCustomEncoder, CC: i32, nbEBands: i32) {
    let _ = CC;
    let _ = nbEBands;
    println!(
        "st rng={} pfperiod={} pfgain={:08x} pftapset={} consec={} delayedIntra={:08x} stereoSaving={:08x} intensity={} lastCoded={} spread={} tapset={} tonalavg={} hfavg={} vbrres={} vbrdrift={} vbroff={} vbrcnt={} overlapmax={:08x} specavg={:08x}",
        st.rng,
        st.prefilter_period,
        st.prefilter_gain.to_bits(),
        st.prefilter_tapset,
        st.consec_transient,
        st.delayedIntra.to_bits(),
        st.stereo_saving.to_bits(),
        st.intensity,
        st.lastCodedBands,
        st.spread_decision,
        st.tapset_decision,
        st.tonal_average,
        st.hf_average,
        st.vbr_reservoir,
        st.vbr_drift,
        st.vbr_offset,
        st.vbr_count,
        st.overlap_max.to_bits(),
        st.spec_avg.to_bits(),
    );
    print_bands("oldBandE", &st.oldBandE);
    print_bands("oldLogE", &st.oldLogE);
    print_bands("oldLogE2", &st.oldLogE2);
}

fn print_ec(enc: &oprs::celt::entcode::ec_ctx) {
    println!(
        "ec tell={} tellfrac={} offs={} endoffs={} error={} nbits={} rng={} val={} rem={} ext={}",
        enc.ec_tell(),
        ec_tell_frac(enc),
        enc.offs,
        enc.end_offs,
        enc.error,
        enc.nbits_total,
        enc.rng,
        enc.val,
        enc.rem,
        enc.ext,
    );
}

fn print_hex(p: &[u8]) {
    print!("hex");
    for &b in p {
        print!(" {:02x}", b);
    }
    println!();
}

fn ctl(st: &mut OpusCustomEncoder, req: i32, v: i32) {
    let mut x = v;
    opus_custom_encoder_ctl(st, req, &mut x);
}

fn main() {
    println!("sizeof_encoder {}", SIZEOF_CELT_ENCODER);
    println!(
        "get_size {} {}",
        celt_encoder_get_size(1),
        celt_encoder_get_size(2)
    );
    {
        let mut err = 0i32;
        let m = opus_custom_mode_create(48000, 960, &mut err).unwrap();
        println!(
            "csize {} {}",
            opus_custom_encoder_get_size(&m, 1),
            opus_custom_encoder_get_size(&m, 2)
        );
    }

    for ci in 0..CASES.len() {
        let cs = CASES[ci];
        let mut err = 0i32;
        let mode = opus_custom_mode_create(48000, 960, &mut err).unwrap();
        let CC = cs.CC;
        let C = cs.C;
        let N = 120 << cs.LM;
        let nbEBands = mode.nbEBands;
        let mut st = opus_custom_encoder_create(mode, CC, &mut err).unwrap();

        let mut pcm_i16 = vec![0i16; (CC * N + 8) as usize];
        let mut pcm_f = vec![0f32; (CC * N + 8) as usize];
        let mut pkt = vec![0u8; MAXPKT + 16];
        let mut mask = vec![0f32; (2 * nbEBands) as usize];

        let mut lcg_s: u32 = cs.seed;

        ctl(&mut st, OPUS_SET_COMPLEXITY_REQUEST, cs.complexity);
        ctl(&mut st, OPUS_SET_LSB_DEPTH_REQUEST, cs.lsb);
        ctl(&mut st, CELT_SET_PREDICTION_REQUEST, cs.pred);
        ctl(&mut st, CELT_SET_START_BAND_REQUEST, cs.start);
        ctl(&mut st, CELT_SET_END_BAND_REQUEST, cs.end);
        ctl(&mut st, OPUS_SET_VBR_REQUEST, cs.vbr);
        ctl(&mut st, OPUS_SET_VBR_CONSTRAINT_REQUEST, cs.cvbr);
        ctl(&mut st, OPUS_SET_BITRATE_REQUEST, cs.bitrate);
        ctl(&mut st, CELT_SET_CHANNELS_REQUEST, C);
        ctl(&mut st, OPUS_SET_PACKET_LOSS_PERC_REQUEST, cs.loss);
        ctl(&mut st, OPUS_SET_PHASE_INVERSION_DISABLED_REQUEST, cs.inv);
        ctl(&mut st, CELT_SET_SIGNALLING_REQUEST, cs.signalling);
        ctl(&mut st, CELT_SET_INPUT_CLIPPING_REQUEST, cs.clip);
        ctl(&mut st, OPUS_SET_LFE_REQUEST, cs.lfe);
        if cs.analysis != 0 {
            let mut info = AnalysisInfo::default();
            info.valid = 1;
            info.tonality = 0.6f32;
            info.tonality_slope = 0.1f32;
            info.noisiness = 0.2f32;
            info.activity = 0.25f32;
            info.music_prob = 0.7f32;
            info.bandwidth = 20;
            info.activity_probability = 0.5f32;
            info.max_pitch_ratio = 0.8f32;
            for k in 0..LEAK_BANDS as usize {
                info.leak_boost[k] = (k * 3) as u8;
            }
            opus_custom_encoder_ctl_analysis(&mut st, Some(&info));
        }
        if cs.mask != 0 {
            for k in 0..(2 * nbEBands) as usize {
                mask[k] = -0.6f32 - 0.13f32 * k as f32;
            }
            opus_custom_encoder_ctl_energy_mask(&mut st, Some(&mask));
        }

        println!(
            "case {} LM={} CC={} C={} start={} end={} nbBytes={} vbr={} cvbr={} bitrate={} cplx={} lsb={} pred={} lfe={} sig={} loss={} inv={} clip={} entry={} ana={} mask={} frames={} seed={}",
            ci, cs.LM, CC, C, cs.start, cs.end, cs.nbBytes, cs.vbr, cs.cvbr, cs.bitrate,
            cs.complexity, cs.lsb, cs.pred, cs.lfe, cs.signalling, cs.loss, cs.inv, cs.clip,
            cs.entry, cs.analysis, cs.mask, cs.frames, cs.seed
        );

        for frame in 0..cs.frames {
            for b in pkt.iter_mut() {
                *b = 0;
            }
            gen_pcm_i16(&mut pcm_i16[..(CC * N) as usize], frame, cs.loud, &mut lcg_s);
            for i in 0..(CC * N) as usize {
                pcm_f[i] = pcm_i16[i] as f32 / 32768.0f32;
            }
            if cs.loud != 0 {
                for i in 0..(CC * N) as usize {
                    pcm_f[i] *= 3.0f32;
                }
            }

            println!(
                "frame {} pin={:016x} pfi={:016x}",
                frame,
                fnv_i16(&pcm_i16[..(CC * N) as usize]),
                fnv_f32(&pcm_f[..(CC * N) as usize])
            );

            if cs.entry == 0 {
                let ret = opus_custom_encode(
                    &mut st,
                    &pcm_i16[..(CC * N) as usize],
                    N,
                    &mut pkt,
                    cs.nbBytes,
                );
                let n = if ret > 0 { ret as usize } else { 0 };
                println!("pkt len={} fnv={:016x}", ret, fnv_u8(&pkt[..n]));
                if frame == 0 {
                    print_hex(&pkt[..n]);
                }
            } else if cs.entry == 1 {
                let ret = opus_custom_encode_float(
                    &mut st,
                    &pcm_f[..(CC * N) as usize],
                    N,
                    &mut pkt,
                    cs.nbBytes,
                );
                let n = if ret > 0 { ret as usize } else { 0 };
                println!("pkt len={} fnv={:016x}", ret, fnv_u8(&pkt[..n]));
                if frame == 0 {
                    print_hex(&pkt[..n]);
                }
            } else {
                let mut enc = ec_enc_init(cs.nbBytes as u32);
                let ret = celt_encode_with_ec(
                    &mut st,
                    &pcm_f[..(CC * N) as usize],
                    N,
                    &mut pkt,
                    cs.nbBytes,
                    Some(&mut enc),
                );
                let n = enc.storage as usize;
                println!("pkt len={} fnv={:016x}", ret, fnv_u8(&enc.buf[..n]));
                if frame == 0 {
                    print_hex(&enc.buf[..n]);
                }
                print_ec(&enc);
            }
            print_state(&st, CC, nbEBands);
        }

        drop(st);
    }
}

//! Rust side of the SILK encoder API glue bit-exactness probe.
//!
//! Mirrors `compat/c/probe_silk_encode.c` line for line.  Run
//! `bash compat/scripts/run-probe.sh silk_encode` to build both sides and diff
//! their output.

#![allow(non_snake_case)]
#![allow(clippy::needless_range_loop)]

use oprs::celt::arch::INT16TORES;
use oprs::celt::entcode::ec_ctx;
use oprs::celt::entenc::{ec_enc_done, ec_enc_init};
use oprs::silk::control::silk_EncControlStruct;
use oprs::silk::define::*;
use oprs::silk::enc_api::{silk_Encode, silk_Get_Encoder_Size, silk_InitEncoder};
use oprs::silk::float::main_flp::silk_encoder;
use oprs::silk::structs::{silk_encoder_state, SideInfoIndices};

const PROBE_MAX_BYTES: u32 = 1500;
const PROBE_ARCH: i32 = 4;

/* ------------------------------------------------------------------ */
/* Deterministic input set                                             */
/* ------------------------------------------------------------------ */

static mut G_RNG: u32 = 0;

fn rng_seed(s: u32) {
    unsafe { G_RNG = s }
}
fn rnd() -> i32 {
    unsafe {
        G_RNG = G_RNG.wrapping_mul(1103515245).wrapping_add(12345);
        G_RNG as i32
    }
}

/* audio kinds: 0 = voiced-ish square + noise, 1 = digital silence,
   2 = very quiet noise */
fn gen_audio(pcm: &mut [f32], n: usize, ch: i32, kind: i32, api_hz: i32) {
    let mut per = api_hz / 400;
    if per < 2 {
        per = 2;
    }
    for i in 0..n {
        let r = rnd();
        let nz = ((r as u32) >> 17) as i32;
        let mut v: i32;
        if kind == 1 {
            v = 0;
        } else if kind == 2 {
            v = (nz >> 4) - 1024;
        } else {
            let ph = (i as i32 / ch) % per;
            let tone = if ph < per / 2 { 7000 } else { -7000 };
            v = tone + ((nz >> 2) - 4096);
        }
        if v > 32767 {
            v = 32767;
        }
        if v < -32768 {
            v = -32768;
        }
        pcm[i] = INT16TORES(v as i16);
    }
}

/* ------------------------------------------------------------------ */
/* Hashing                                                             */
/* ------------------------------------------------------------------ */

fn fnv_u8(mut h: u64, v: u32, nbytes: i32) -> u64 {
    for k in 0..nbytes {
        h ^= ((v >> (8 * k)) & 0xFF) as u64;
        h = h.wrapping_mul(1099511628211);
    }
    h
}
fn fnv_bytes(b: &[u8]) -> u64 {
    let mut h: u64 = 1469598103934665603;
    for &x in b {
        h ^= x as u64;
        h = h.wrapping_mul(1099511628211);
    }
    h
}
fn fnv_i8(a: &[i8]) -> u64 {
    let mut h: u64 = 1469598103934665603;
    for &x in a {
        h = fnv_u8(h, x as u8 as u32, 1);
    }
    h
}
fn fnv_i16(a: &[i16]) -> u64 {
    let mut h: u64 = 1469598103934665603;
    for &x in a {
        h = fnv_u8(h, x as u16 as u32, 2);
    }
    h
}
fn fnv_i32(a: &[i32]) -> u64 {
    let mut h: u64 = 1469598103934665603;
    for &x in a {
        h = fnv_u8(h, x as u32, 4);
    }
    h
}
fn fnv_f32(a: &[f32]) -> u64 {
    let mut h: u64 = 1469598103934665603;
    for &x in a {
        h = fnv_u8(h, x.to_bits(), 4);
    }
    h
}
fn fnv_indices(ix: &SideInfoIndices) -> u64 {
    let mut h: u64 = 1469598103934665603;
    for i in 0..MAX_NB_SUBFR as usize {
        h = fnv_u8(h, ix.GainsIndices[i] as u8 as u32, 1);
    }
    for i in 0..MAX_NB_SUBFR as usize {
        h = fnv_u8(h, ix.LTPIndex[i] as u8 as u32, 1);
    }
    for i in 0..(MAX_LPC_ORDER + 1) as usize {
        h = fnv_u8(h, ix.NLSFIndices[i] as u8 as u32, 1);
    }
    h = fnv_u8(h, ix.lagIndex as u16 as u32, 2);
    h = fnv_u8(h, ix.contourIndex as u8 as u32, 1);
    h = fnv_u8(h, ix.signalType as u8 as u32, 1);
    h = fnv_u8(h, ix.quantOffsetType as u8 as u32, 1);
    h = fnv_u8(h, ix.NLSFInterpCoef_Q2 as u8 as u32, 1);
    h = fnv_u8(h, ix.PERIndex as u8 as u32, 1);
    h = fnv_u8(h, ix.LTP_scaleIndex as u8 as u32, 1);
    h = fnv_u8(h, ix.Seed as u8 as u32, 1);
    h
}

/* ------------------------------------------------------------------ */
/* State printing                                                      */
/* ------------------------------------------------------------------ */

fn print_chan(c0: &silk_encoder_state) {
    println!(
        "  nsq xq_fn={:016x} ltpshp_fn={:016x} slpc_fn={:016x} ar2_fn={:016x} \
         sLF_AR={} sDiff={} lagPrev={} sLTP_buf_idx={} sLTP_shp_idx={} rand_seed={} prev_gain_Q16={} rewhite={}",
        fnv_i16(&c0.sNSQ.xq),
        fnv_i32(&c0.sNSQ.sLTP_shp_Q14),
        fnv_i32(&c0.sNSQ.sLPC_Q14),
        fnv_i32(&c0.sNSQ.sAR2_Q14),
        c0.sNSQ.sLF_AR_shp_Q14,
        c0.sNSQ.sDiff_shp_Q14,
        c0.sNSQ.lagPrev,
        c0.sNSQ.sLTP_buf_idx,
        c0.sNSQ.sLTP_shp_buf_idx,
        c0.sNSQ.rand_seed,
        c0.sNSQ.prev_gain_Q16,
        c0.sNSQ.rewhite_flag
    );
    println!(
        "  idx Gains=[{} {} {} {}] LTP=[{} {} {} {}] lag={} contour={} sig={} qoff={} interp={} per={} ltpscale={} seed={} nlsf_fn={:016x}",
        c0.indices.GainsIndices[0],
        c0.indices.GainsIndices[1],
        c0.indices.GainsIndices[2],
        c0.indices.GainsIndices[3],
        c0.indices.LTPIndex[0],
        c0.indices.LTPIndex[1],
        c0.indices.LTPIndex[2],
        c0.indices.LTPIndex[3],
        c0.indices.lagIndex,
        c0.indices.contourIndex,
        c0.indices.signalType,
        c0.indices.quantOffsetType,
        c0.indices.NLSFInterpCoef_Q2,
        c0.indices.PERIndex,
        c0.indices.LTP_scaleIndex,
        c0.indices.Seed,
        fnv_i8(&c0.indices.NLSFIndices)
    );
    let fl = if c0.frame_length > 0 {
        c0.frame_length as usize
    } else {
        MAX_FRAME_LENGTH as usize
    };
    println!(
        "  lbrr idx_fn={:016x} pulses0_fn={:016x} pulses1_fn={:016x} pulses2_fn={:016x}",
        fnv_indices(&c0.indices_LBRR[0]),
        fnv_i8(&c0.pulses_LBRR[0][..fl]),
        fnv_i8(&c0.pulses_LBRR[1][..fl]),
        fnv_i8(&c0.pulses_LBRR[2][..fl])
    );
}

fn print_state(psEnc: &silk_encoder, ctl: &silk_EncControlStruct) {
    let c0 = &psEnc.state_Fxx[0].sCmn;
    let sh0 = &psEnc.state_Fxx[0].sShape;

    println!(
        "  state fs_kHz={} nChannelsAPI={} nChannelsInternal={} nPrevChannelsInternal={} \
         nFramesEncoded={} nFramesPerPacket={} nb_subfr={} frame_length={} subfr_length={} \
         frameCounter={} prev_NLSFq_fn={:016x} inputBufIx={} arch={}",
        c0.fs_kHz,
        psEnc.nChannelsAPI,
        psEnc.nChannelsInternal,
        psEnc.nPrevChannelsInternal,
        c0.nFramesEncoded,
        c0.nFramesPerPacket,
        c0.nb_subfr,
        c0.frame_length,
        c0.subfr_length,
        c0.frameCounter,
        fnv_i16(&c0.prev_NLSFq_Q15),
        c0.inputBufIx,
        c0.arch
    );
    println!(
        "  flags VAD=[{} {} {}] LBRR=[{} {} {}] LBRR_flag={} LBRR_enabled={} LBRR_GainIncreases={} \
         speech_activity_Q8={} inDTX={} noSpeechCounter={} useDTX={} useCBR={} useInBandFEC={} first_frame_after_reset={}",
        c0.VAD_flags[0],
        c0.VAD_flags[1],
        c0.VAD_flags[2],
        c0.LBRR_flags[0],
        c0.LBRR_flags[1],
        c0.LBRR_flags[2],
        c0.LBRR_flag,
        c0.LBRR_enabled,
        c0.LBRR_GainIncreases,
        c0.speech_activity_Q8,
        c0.inDTX,
        c0.noSpeechCounter,
        c0.useDTX,
        c0.useCBR,
        c0.useInBandFEC,
        c0.first_frame_after_reset
    );
    println!(
        "  misc prevLag={} prevSignalType={} pitch_LPC_win={} max_pitch_lag={} TargetRate_bps={} \
         SNR_dB_Q7={} sum_log_gain_Q7={} input_tilt={} variable_HP1={} variable_HP2={} \
         nBitsUsedLBRR={} nBitsExceeded={} allowBandwidthSwitch={} timeSinceSwitchAllowed_ms={} \
         prev_decode_only_middle={}",
        c0.prevLag,
        c0.prevSignalType,
        c0.pitch_LPC_win_length,
        c0.max_pitch_lag,
        c0.TargetRate_bps,
        c0.SNR_dB_Q7,
        c0.sum_log_gain_Q7,
        c0.input_tilt_Q15,
        c0.variable_HP_smth1_Q15,
        c0.variable_HP_smth2_Q15,
        psEnc.nBitsUsedLBRR,
        psEnc.nBitsExceeded,
        psEnc.allowBandwidthSwitch,
        psEnc.timeSinceSwitchAllowed_ms,
        psEnc.prev_decode_only_middle
    );
    println!(
        "  shape lastgain={} tilt_bits={:08x} hsg_bits={:08x} LTPCorr_bits={:08x} \
         sLP_mode={} sLP_trans={} sLP_savedfs={} sLP_ILP=[{} {}] xbuf_fn={:016x}",
        sh0.LastGainIndex,
        sh0.Tilt_smth.to_bits(),
        sh0.HarmShapeGain_smth.to_bits(),
        psEnc.state_Fxx[0].LTPCorr.to_bits(),
        c0.sLP.mode,
        c0.sLP.transition_frame_no,
        c0.sLP.saved_fs_kHz,
        c0.sLP.In_LP_State[0],
        c0.sLP.In_LP_State[1],
        fnv_f32(&psEnc.state_Fxx[0].x_buf)
    );
    println!(
        "  ctl internalSampleRate={} signalType={} offset={} stereoWidth_Q14={} inWB={} \
         switchReady={} allowBW={} payloadSize_ms={} complexity={} maxInternal={} minInternal={} desiredInternal={}",
        ctl.internalSampleRate,
        ctl.signalType,
        ctl.offset,
        ctl.stereoWidth_Q14,
        ctl.inWBmodeWithoutVariableLP,
        ctl.switchReady,
        ctl.allowBandwidthSwitch,
        ctl.payloadSize_ms,
        ctl.complexity,
        ctl.maxInternalSampleRate,
        ctl.minInternalSampleRate,
        ctl.desiredInternalSampleRate
    );
    println!(
        "  stereo smth_width={} width_prev={} pred_prev=[{} {}] mid_side_amp=[{} {} {} {}] \
         silent_side_len={}",
        psEnc.sStereo.smth_width_Q14,
        psEnc.sStereo.width_prev_Q14,
        psEnc.sStereo.pred_prev_Q13[0],
        psEnc.sStereo.pred_prev_Q13[1],
        psEnc.sStereo.mid_side_amp_Q0[0],
        psEnc.sStereo.mid_side_amp_Q0[1],
        psEnc.sStereo.mid_side_amp_Q0[2],
        psEnc.sStereo.mid_side_amp_Q0[3],
        psEnc.sStereo.silent_side_len
    );
    if psEnc.nChannelsInternal == 2 {
        let c1 = &psEnc.state_Fxx[1].sCmn;
        println!(
            "  ch1 fs_kHz={} VAD=[{} {} {}] LBRR=[{} {} {}] LBRR_flag={} prev_gain_Q16={} rand_seed={} \
             prevLag={} prevSignalType={} lastgain={} nFramesEncoded={} inputBufIx={}",
            c1.fs_kHz,
            c1.VAD_flags[0],
            c1.VAD_flags[1],
            c1.VAD_flags[2],
            c1.LBRR_flags[0],
            c1.LBRR_flags[1],
            c1.LBRR_flags[2],
            c1.LBRR_flag,
            c1.sNSQ.prev_gain_Q16,
            c1.sNSQ.rand_seed,
            c1.prevLag,
            c1.prevSignalType,
            psEnc.state_Fxx[1].sShape.LastGainIndex,
            c1.nFramesEncoded,
            c1.inputBufIx
        );
    }
    print_chan(c0);
}

/* ------------------------------------------------------------------ */
/* Settings matrix                                                     */
/* ------------------------------------------------------------------ */

#[derive(Clone, Copy)]
struct EncCase {
    fs_kHz: i32,
    api_hz: i32,
    payload_ms: i32,
    ch_api: i32,
    ch_int: i32,
    bitrate: i32,
    complexity: i32,
    dtx: i32,
    fec: i32,
    lbrr: i32,
    cbr: i32,
    tomono: i32,
    red: i32,
    loss: i32,
    activity: i32,
    audio: i32,
    nframes: i32,
    prefill: i32,
}

/*                      fs  api    pl ci cx   br    cx dtx fec lbrr cbr tmon red loss act aud nf pre */
static G_CASES: &[EncCase] = &[
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 24000, complexity: 10, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 3, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 24000, complexity: 0, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 3, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 24000, complexity: 1, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 3, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 24000, complexity: 2, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 3, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 24000, complexity: 3, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 3, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 24000, complexity: 4, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 3, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 24000, complexity: 5, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 3, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 24000, complexity: 6, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 3, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 24000, complexity: 7, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 3, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 24000, complexity: 8, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 3, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 24000, complexity: 9, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 3, prefill: 0 },
    EncCase { fs_kHz: 8, api_hz: 8000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 12000, complexity: 2, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 4, prefill: 0 },
    EncCase { fs_kHz: 12, api_hz: 12000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 16000, complexity: 2, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 4, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 24000, complexity: 2, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 4, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 48000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 24000, complexity: 2, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 3, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 8000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 24000, complexity: 2, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 3, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 24000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 24000, complexity: 2, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 3, prefill: 0 },
    EncCase { fs_kHz: 12, api_hz: 48000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 16000, complexity: 2, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 3, prefill: 0 },
    EncCase { fs_kHz: 8, api_hz: 48000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 12000, complexity: 2, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 3, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 2, ch_int: 2, bitrate: 32000, complexity: 10, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 3, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 2, ch_int: 1, bitrate: 24000, complexity: 10, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 3, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 48000, payload_ms: 20, ch_api: 2, ch_int: 2, bitrate: 48000, complexity: 10, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 3, prefill: 0 },
    EncCase { fs_kHz: 8, api_hz: 8000, payload_ms: 20, ch_api: 2, ch_int: 2, bitrate: 16000, complexity: 2, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 4, prefill: 0 },
    EncCase { fs_kHz: 12, api_hz: 12000, payload_ms: 20, ch_api: 2, ch_int: 2, bitrate: 24000, complexity: 2, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 4, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 2, ch_int: 2, bitrate: 8000, complexity: 10, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 4, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 2, ch_int: 2, bitrate: 6000, complexity: 10, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 4, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 10, ch_api: 1, ch_int: 1, bitrate: 24000, complexity: 10, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 4, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 40, ch_api: 1, ch_int: 1, bitrate: 24000, complexity: 10, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 3, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 60, ch_api: 1, ch_int: 1, bitrate: 24000, complexity: 10, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 3, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 60, ch_api: 2, ch_int: 2, bitrate: 48000, complexity: 10, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 2, prefill: 0 },
    EncCase { fs_kHz: 12, api_hz: 12000, payload_ms: 40, ch_api: 1, ch_int: 1, bitrate: 16000, complexity: 2, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 3, prefill: 0 },
    EncCase { fs_kHz: 8, api_hz: 8000, payload_ms: 60, ch_api: 1, ch_int: 1, bitrate: 12000, complexity: 2, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 3, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 48000, payload_ms: 60, ch_api: 2, ch_int: 2, bitrate: 64000, complexity: 10, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 2, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 10, ch_api: 2, ch_int: 1, bitrate: 20000, complexity: 10, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 4, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 6000, complexity: 10, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 4, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 8000, complexity: 10, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 4, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 12000, complexity: 10, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 4, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 16000, complexity: 10, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 4, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 32000, complexity: 10, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 4, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 64000, complexity: 10, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 4, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 96000, complexity: 10, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 4, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 5000, complexity: 10, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 4, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 80000, complexity: 10, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 4, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 24000, complexity: 10, dtx: 0, fec: 0, lbrr: 0, cbr: 1, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 4, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 2, ch_int: 2, bitrate: 48000, complexity: 10, dtx: 0, fec: 0, lbrr: 0, cbr: 1, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 3, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 24000, complexity: 10, dtx: 0, fec: 1, lbrr: 1, cbr: 1, tomono: 0, red: 0, loss: 10, activity: 1, audio: 0, nframes: 8, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 24000, complexity: 10, dtx: 1, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 20, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 24000, complexity: 10, dtx: 1, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 1, nframes: 20, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 2, ch_int: 2, bitrate: 32000, complexity: 10, dtx: 1, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 1, nframes: 20, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 24000, complexity: 10, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 1, nframes: 6, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 24000, complexity: 10, dtx: 0, fec: 1, lbrr: 1, cbr: 0, tomono: 0, red: 0, loss: 5, activity: 1, audio: 0, nframes: 8, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 24000, complexity: 10, dtx: 0, fec: 1, lbrr: 1, cbr: 0, tomono: 0, red: 0, loss: 10, activity: 1, audio: 0, nframes: 8, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 24000, complexity: 10, dtx: 0, fec: 1, lbrr: 1, cbr: 0, tomono: 0, red: 0, loss: 30, activity: 1, audio: 0, nframes: 8, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 24000, complexity: 10, dtx: 0, fec: 0, lbrr: 1, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 8, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 16000, complexity: 10, dtx: 0, fec: 1, lbrr: 1, cbr: 0, tomono: 0, red: 0, loss: 20, activity: 1, audio: 0, nframes: 8, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 2, ch_int: 2, bitrate: 32000, complexity: 10, dtx: 0, fec: 1, lbrr: 1, cbr: 0, tomono: 0, red: 0, loss: 10, activity: 1, audio: 0, nframes: 8, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 40, ch_api: 1, ch_int: 1, bitrate: 24000, complexity: 10, dtx: 0, fec: 1, lbrr: 1, cbr: 0, tomono: 0, red: 0, loss: 10, activity: 1, audio: 0, nframes: 6, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 60, ch_api: 1, ch_int: 1, bitrate: 32000, complexity: 10, dtx: 0, fec: 1, lbrr: 1, cbr: 0, tomono: 0, red: 0, loss: 10, activity: 1, audio: 0, nframes: 6, prefill: 0 },
    EncCase { fs_kHz: 12, api_hz: 12000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 16000, complexity: 10, dtx: 0, fec: 1, lbrr: 1, cbr: 0, tomono: 0, red: 0, loss: 30, activity: 1, audio: 0, nframes: 8, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 24000, complexity: 10, dtx: 1, fec: 1, lbrr: 1, cbr: 0, tomono: 0, red: 0, loss: 20, activity: 1, audio: 0, nframes: 20, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 24000, complexity: 10, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 1, red: 0, loss: 0, activity: 1, audio: 0, nframes: 4, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 2, ch_int: 2, bitrate: 48000, complexity: 10, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 1, red: 0, loss: 0, activity: 1, audio: 0, nframes: 4, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 2, ch_int: 1, bitrate: 24000, complexity: 10, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 1, red: 0, loss: 0, activity: 1, audio: 0, nframes: 4, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 24000, complexity: 10, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 1, loss: 0, activity: 1, audio: 0, nframes: 4, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 2, ch_int: 2, bitrate: 48000, complexity: 10, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 1, loss: 0, activity: 1, audio: 0, nframes: 4, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 24000, complexity: 10, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 0, audio: 0, nframes: 4, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 24000, complexity: 10, dtx: 1, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 0, audio: 0, nframes: 20, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 2, ch_int: 2, bitrate: 32000, complexity: 10, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 0, audio: 0, nframes: 4, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 24000, complexity: 10, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 2, nframes: 6, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 24000, complexity: 10, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 1, nframes: 6, prefill: 0 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 24000, complexity: 10, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 3, prefill: 1 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 24000, complexity: 10, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 3, prefill: 2 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 60, ch_api: 1, ch_int: 1, bitrate: 24000, complexity: 10, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 2, prefill: 2 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 2, ch_int: 2, bitrate: 48000, complexity: 10, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 3, prefill: 1 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 20, ch_api: 2, ch_int: 2, bitrate: 48000, complexity: 10, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 3, prefill: 2 },
    EncCase { fs_kHz: 8, api_hz: 8000, payload_ms: 20, ch_api: 1, ch_int: 1, bitrate: 12000, complexity: 2, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 3, prefill: 2 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 10, ch_api: 1, ch_int: 1, bitrate: 24000, complexity: 10, dtx: 0, fec: 0, lbrr: 0, cbr: 0, tomono: 0, red: 0, loss: 0, activity: 1, audio: 0, nframes: 4, prefill: 1 },
    EncCase { fs_kHz: 16, api_hz: 16000, payload_ms: 40, ch_api: 2, ch_int: 2, bitrate: 32000, complexity: 10, dtx: 0, fec: 1, lbrr: 1, cbr: 0, tomono: 0, red: 0, loss: 10, activity: 1, audio: 0, nframes: 4, prefill: 2 },
];

/* ------------------------------------------------------------------ */
/* One case                                                            */
/* ------------------------------------------------------------------ */

fn report(
    psEnc: &silk_encoder,
    ctl: &silk_EncControlStruct,
    enc: &ec_ctx,
    ret: i32,
    nBytesOut: i32,
    tell: i32,
    len: i32,
    tag: &str,
) {
    println!(
        "  {} ret={} nBytes={} tell={} len={} offs={} end_offs={} nend_bits={} nbits_total={} \
         rng={} val={} rem={} ext={} error={}",
        tag,
        ret,
        nBytesOut,
        tell,
        len,
        enc.offs,
        enc.end_offs,
        enc.nend_bits,
        enc.nbits_total,
        enc.rng,
        enc.val,
        enc.rem,
        enc.ext,
        enc.error
    );
    print!("  {} payload:", tag);
    if len <= 0 {
        print!(" -");
    } else {
        for i in 0..len as usize {
            print!("{:02x}", enc.buf[i]);
        }
    }
    let n = if len > 0 { len as usize } else { 0 };
    println!(" fnv={:016x}", fnv_bytes(&enc.buf[..n]));
    print_state(psEnc, ctl);
}

fn run_case(ci: usize, c: &EncCase) {
    let frame_size = c.payload_ms * c.api_hz / 1000;
    let nch = c.ch_api;
    let total = (c.nframes * frame_size * nch) as usize;
    let pre_n = ((c.api_hz / 100) * nch) as usize;

    println!(
        "=== case {} fs={} api={} payload_ms={} ch={}/{} br={} cx={} dtx={} fec={} lbrr={} cbr={} \
         tomono={} red={} loss={} act={} audio={} nframes={} prefill={} ===",
        ci,
        c.fs_kHz,
        c.api_hz,
        c.payload_ms,
        c.ch_api,
        c.ch_int,
        c.bitrate,
        c.complexity,
        c.dtx,
        c.fec,
        c.lbrr,
        c.cbr,
        c.tomono,
        c.red,
        c.loss,
        c.activity,
        c.audio,
        c.nframes,
        c.prefill
    );

    rng_seed(0x1000u32 + ci as u32);

    let mut pre = vec![0.0f32; 0];
    if c.prefill != 0 {
        pre = vec![0.0f32; pre_n];
        gen_audio(&mut pre, pre_n, nch, c.audio, c.api_hz);
    }
    let mut pcm = vec![0.0f32; total];
    gen_audio(&mut pcm, total, nch, c.audio, c.api_hz);

    let mut encSizeBytes = 0i32;
    let mut encSizeMono = 0i32;
    let mut ret = silk_Get_Encoder_Size(&mut encSizeBytes, c.ch_api);
    silk_Get_Encoder_Size(&mut encSizeMono, 1);
    println!(
        "  Get_Encoder_Size ret={} nonzero={} mono_lt_stereo={}",
        ret,
        (encSizeBytes > 0) as i32,
        (encSizeMono < encSizeBytes) as i32
    );

    let mut psEnc = Box::new(silk_encoder::default());
    let mut ctl = silk_EncControlStruct::default();
    ret = silk_InitEncoder(&mut psEnc, c.ch_api, PROBE_ARCH, &mut ctl);
    println!(
        "  InitEncoder ret={} nChannelsAPI={} nChannelsInternal={} q_nChannelsAPI={} \
         q_internalSampleRate={} mono_lt_stereo={}",
        ret,
        psEnc.nChannelsAPI,
        psEnc.nChannelsInternal,
        ctl.nChannelsAPI,
        ctl.internalSampleRate,
        (encSizeMono < encSizeBytes) as i32
    );

    ctl.nChannelsAPI = c.ch_api;
    ctl.nChannelsInternal = c.ch_int;
    ctl.API_sampleRate = c.api_hz;
    ctl.maxInternalSampleRate = 16000;
    ctl.minInternalSampleRate = 8000;
    ctl.desiredInternalSampleRate = 16000;
    ctl.payloadSize_ms = c.payload_ms;
    ctl.bitRate = c.bitrate;
    ctl.packetLossPercentage = c.loss;
    ctl.complexity = c.complexity;
    ctl.useInBandFEC = c.fec;
    ctl.useDRED = 0;
    ctl.LBRR_coded = c.lbrr;
    ctl.useDTX = c.dtx;
    ctl.useCBR = c.cbr;
    ctl.maxBits = c.bitrate * c.payload_ms / 1000;
    ctl.toMono = c.tomono;
    ctl.opusCanSwitch = 0;
    ctl.reducedDependency = c.red;

    if c.prefill != 0 {
        let mut enc = ec_enc_init(PROBE_MAX_BYTES);
        let mut nBytes: i32 = PROBE_MAX_BYTES as i32;
        ret = silk_Encode(
            &mut psEnc,
            &mut ctl,
            &pre,
            c.api_hz / 100,
            &mut enc,
            &mut nBytes,
            c.prefill,
            c.activity,
        );
        let tell = enc.ec_tell();
        let len = (tell + 7) >> 3;
        ec_enc_done(&mut enc);
        println!(
            "  pref{} payloadSize_ms={} complexity={}",
            c.prefill, ctl.payloadSize_ms, ctl.complexity
        );
        report(&psEnc, &ctl, &enc, ret, nBytes, tell, len, "pre");
    }

    for f in 0..c.nframes {
        let mut enc = ec_enc_init(PROBE_MAX_BYTES);
        let mut nBytes: i32 = PROBE_MAX_BYTES as i32;
        let off = (f * frame_size * nch) as usize;
        let n = (frame_size * nch) as usize;
        ret = silk_Encode(
            &mut psEnc,
            &mut ctl,
            &pcm[off..off + n],
            frame_size,
            &mut enc,
            &mut nBytes,
            0,
            c.activity,
        );
        let tell = enc.ec_tell();
        let len = (tell + 7) >> 3;
        ec_enc_done(&mut enc);
        let tag = format!("f{}", f);
        report(&psEnc, &ctl, &enc, ret, nBytes, tell, len, &tag);
    }
}

fn main() {
    println!(
        "probe_silk_encode: silk_Encode, {} cases",
        G_CASES.len()
    );
    for (i, c) in G_CASES.iter().enumerate() {
        run_case(i, c);
    }
    println!("probe_silk_encode: done");
}

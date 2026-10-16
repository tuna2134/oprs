//! Bit-exactness probe for `src/opus/encoder.rs` (libopus 1.6.1).
//!
//! The Rust twin of `compat/c/probe_opus_encoder.c`; both print the identical
//! format and `bash compat/scripts/run-probe.sh opus_encoder` must report
//! `IDENTICAL`.  See the C probe's header for what the sections cover.

use oprs::celt::celt::{AnalysisInfo, COMBFILTER_MAXPERIOD, LEAK_BANDS};
use oprs::celt::celt_encoder::OpusCustomEncoder;
use oprs::celt::modes::{opus_custom_mode_create, opus_custom_mode_destroy};
use oprs::ffi::*;
use oprs::opus::encoder::analysis::{
    run_analysis, tonality_analysis_init, AnalysisPcm, TonalityAnalysisState, ANALYSIS_BUF_SIZE,
    MAX_NEURONS, NB_FRAMES, NB_TBANDS,
};
use oprs::opus::encoder::{OpusEncoder, OPUS_ENCODER_STRUCT_SIZE};
use oprs::silk::float::main_flp::silk_encoder;
use oprs::types::*;

/* Requests that `src/opus_private.h` / `celt.h` define but `ffi.rs` does not. */
const OPUS_SET_VOICE_RATIO_REQUEST: i32 = 11018;
const OPUS_GET_VOICE_RATIO_REQUEST: i32 = 11019;
const OPUS_SET_FORCE_MODE_REQUEST: i32 = 11002;
const OPUS_SET_LFE_REQUEST: i32 = 10024;
const MODE_CELT_ONLY: i32 = 1002;

/* ------------------------------------------------------------------ */
/* helpers                                                            */
/* ------------------------------------------------------------------ */

fn f2u(f: f32) -> u32 {
    f.to_bits()
}

fn fnv_bytes(b: &[u8]) -> u64 {
    let mut h = 1469598103934665603u64;
    for &x in b {
        h ^= x as u64;
        h = h.wrapping_mul(1099511628211);
    }
    h
}

fn fnv_upd(h: u64, b: &[u8]) -> u64 {
    let mut h = h;
    for &x in b {
        h ^= x as u64;
        h = h.wrapping_mul(1099511628211);
    }
    h
}

fn fnv_f32(p: &[f32]) -> u64 {
    let mut bytes = Vec::with_capacity(p.len() * 4);
    for &x in p {
        bytes.extend_from_slice(&x.to_bits().to_le_bytes());
    }
    fnv_bytes(&bytes)
}

fn fnv_i32(p: &[i32]) -> u64 {
    let mut bytes = Vec::with_capacity(p.len() * 4);
    for &x in p {
        bytes.extend_from_slice(&x.to_le_bytes());
    }
    fnv_bytes(&bytes)
}

fn fnv_i16(p: &[i16]) -> u64 {
    let mut bytes = Vec::with_capacity(p.len() * 2);
    for &x in p {
        bytes.extend_from_slice(&x.to_le_bytes());
    }
    fnv_bytes(&bytes)
}

fn fnv_u8(p: &[u8]) -> u64 {
    fnv_bytes(p)
}

/* ------------------------------------------------------------------ */
/* deterministic input (identical to the oracle / enctrace)           */
/* ------------------------------------------------------------------ */

struct Rng {
    s: u32,
}

impl Rng {
    fn next(&mut self) -> u32 {
        self.s = self.s.wrapping_mul(1103515245).wrapping_add(12345);
        self.s
    }
    fn sample(&mut self) -> f32 {
        let v = ((self.next() >> 8) as i32) - 8388608;
        (v as f32) / 8388608.0
    }
}

fn envelope(i: usize) -> f32 {
    0.25 + 0.75 * ((i % 9973) as f32) / 9973.0
}

static mut LCG_S: u32 = 0;

fn lcg_next() -> u32 {
    unsafe {
        LCG_S = LCG_S.wrapping_mul(1103515245).wrapping_add(12345);
        LCG_S
    }
}

fn gen_pcm_i16(p: &mut [i16], frame_idx: i32, channels: i32) {
    let g = match frame_idx % 4 {
        0 => 12000,
        1 => 4000,
        2 => 300,
        _ => 0,
    };
    let period = if channels == 2 { 53 } else { 37 };
    for i in 0..p.len() {
        let r = lcg_next();
        let mut v = (r % 2001) as i32 - 1000;
        let ph = (i as i32) % period;
        let tone = if ph < period / 2 {
            ph * 20
        } else {
            (period - ph) * 20
        };
        v += tone;
        p[i] = (v * g / 1000) as i16;
    }
}

fn gen_pcm_f32(p: &mut [f32], frame_idx: i32, channels: i32) {
    let g = match frame_idx % 4 {
        0 => 0.9f32,
        1 => 0.3f32,
        2 => 0.02f32,
        _ => 0.0f32,
    };
    let period = if channels == 2 { 53 } else { 37 };
    for i in 0..p.len() {
        let r = lcg_next();
        let v = ((r % 20001) as i32 - 10000) as f32 / 10000.0;
        let ph = (i as i32) % period;
        let tone = (if ph < period / 2 {
            ph * 20
        } else {
            (period - ph) * 20
        }) as f32
            / 10000.0;
        p[i] = (v + tone) * g;
    }
}

/* ------------------------------------------------------------------ */
/* state printers                                                     */
/* ------------------------------------------------------------------ */

fn print_ainfo(tag: &str, k: i32, ai: &AnalysisInfo) {
    println!(
        "{} {} valid={} ton={:08x} tslope={:08x} noisy={:08x} act={:08x} mprob={:08x} mmin={:08x} mmax={:08x} bw={} actp={:08x} mpr={:08x} lb={:016x}",
        tag, k, ai.valid, f2u(ai.tonality), f2u(ai.tonality_slope), f2u(ai.noisiness),
        f2u(ai.activity), f2u(ai.music_prob), f2u(ai.music_prob_min), f2u(ai.music_prob_max),
        ai.bandwidth, f2u(ai.activity_probability), f2u(ai.max_pitch_ratio),
        fnv_u8(&ai.leak_boost[..LEAK_BANDS as usize])
    );
}

fn print_astate(tag: &str, k: i32, s: &TonalityAnalysisState) {
    let mut e = Vec::with_capacity((NB_FRAMES * NB_TBANDS) as usize);
    for row in s.E.iter() {
        e.extend_from_slice(row);
    }
    let mut loge = Vec::with_capacity((NB_FRAMES * NB_TBANDS) as usize);
    for row in s.logE.iter() {
        loge.extend_from_slice(row);
    }
    println!(
        "{} {} count={} ec={} wp={} rp={} rs={} ao={} mf={} pbw={} et={:08x} lec={:08x} init={} ang={:016x} inmem={:016x} E={:016x} logE={:016x} rnn={:016x} mem={:016x} lowE={:016x}",
        tag, k, s.count, s.E_count, s.write_pos, s.read_pos, s.read_subframe,
        s.analysis_offset, s.mem_fill, s.prev_bandwidth, f2u(s.Etracker), f2u(s.lowECount),
        s.initialized, fnv_f32(&s.angle), fnv_f32(&s.inmem), fnv_f32(&e), fnv_f32(&loge),
        fnv_f32(&s.rnn_state), fnv_f32(&s.mem), fnv_f32(&s.lowE)
    );
}

fn print_state(st: &OpusEncoder, k: i32) {
    println!(
        "ST {} rf={:08x} mode={} pmode={} bw={} abw={} sch={} pch={} pfs={} first={} sbw={} hbg={:08x} hp={:08x},{:08x},{:08x},{:08x} hsw={} vhp={} dbw={} nba={} pse={:08x} nff={} wid={:08x},{:08x},{:08x},{:08x},{:08x} em={} br={} ubr={}",
        k, st.rangeFinal, st.mode, st.prev_mode, st.bandwidth, st.auto_bandwidth,
        st.stream_channels, st.prev_channels, st.prev_framesize, st.first, st.silk_bw_switch,
        f2u(st.prev_HB_gain), f2u(st.hp_mem[0]), f2u(st.hp_mem[1]), f2u(st.hp_mem[2]),
        f2u(st.hp_mem[3]), st.hybrid_stereo_width_Q14 as i32, st.variable_HP_smth2_Q15,
        st.detected_bandwidth, st.nb_no_activity_ms_Q1, f2u(st.peak_signal_energy),
        st.nonfinal_frame, f2u(st.width_mem.XX), f2u(st.width_mem.XY), f2u(st.width_mem.YY),
        f2u(st.width_mem.smoothed_width), f2u(st.width_mem.max_follower),
        st.energy_masking.is_some() as i32, st.bitrate_bps, st.user_bitrate_bps
    );
    let sm = &st.silk_mode;
    println!(
        "SM {} nca={} nci={} api={} mxsr={} mnsr={} dsr={} ps={} br={} pl={} cx={} fec={} dred={} lbrr={} dtx={} cbr={} mb={} tm={} ocs={} rd={} isr={} abw={} iwb={} swq={} sr={} st={} off={}",
        k, sm.nChannelsAPI, sm.nChannelsInternal, sm.API_sampleRate, sm.maxInternalSampleRate,
        sm.minInternalSampleRate, sm.desiredInternalSampleRate, sm.payloadSize_ms, sm.bitRate,
        sm.packetLossPercentage, sm.complexity, sm.useInBandFEC, sm.useDRED, sm.LBRR_coded,
        sm.useDTX, sm.useCBR, sm.maxBits, sm.toMono, sm.opusCanSwitch, sm.reducedDependency,
        sm.internalSampleRate, sm.allowBandwidthSwitch, sm.inWBmodeWithoutVariableLP,
        sm.stereoWidth_Q14, sm.switchReady, sm.signalType, sm.offset
    );
    if st.application != OPUS_APPLICATION_RESTRICTED_CELT {
        let se: &silk_encoder = &st.silk_enc;
        let (nsp1, hp1, fsk1) = if st.channels == 2 {
            (
                se.state_Fxx[1].sCmn.noSpeechCounter,
                se.state_Fxx[1].sCmn.variable_HP_smth1_Q15,
                se.state_Fxx[1].sCmn.fs_kHz,
            )
        } else {
            (0, 0, 0)
        };
        println!(
            "SE {} nca={} nci={} npc={} tss={} abw={} nbu={} nbe={} dom={} nsp0={} nsp1={} hp0={} hp1={} fsk0={} fsk1={}",
            k, se.nChannelsAPI, se.nChannelsInternal, se.nPrevChannelsInternal,
            se.timeSinceSwitchAllowed_ms, se.allowBandwidthSwitch, se.nBitsUsedLBRR,
            se.nBitsExceeded, se.prev_decode_only_middle,
            se.state_Fxx[0].sCmn.noSpeechCounter, nsp1,
            se.state_Fxx[0].sCmn.variable_HP_smth1_Q15, hp1,
            se.state_Fxx[0].sCmn.fs_kHz, fsk1
        );
    } else {
        println!("SE {} skip=1", k);
    }
    if st.application != OPUS_APPLICATION_RESTRICTED_SILK {
        let ce: &OpusCustomEncoder = &st.celt_enc;
        let nbEBands = ce.mode.nbEBands;
        let n = (ce.channels * nbEBands) as usize;
        println!(
            "CE {} rng={:08x} sd={} di={:08x} ta={} lcb={} hfa={} td={} pp={} pg={:08x} pt={} ct={} vr={} vd={} vo={} vc={} om={:08x} ss={:08x} int={} sp={:08x} obe={:016x} ole={:016x} ole2={:016x} ee={:016x} anv={} im={:016x} pm={:016x} pe={:016x}",
            k, ce.rng, ce.spread_decision, f2u(ce.delayedIntra), ce.tonal_average,
            ce.lastCodedBands, ce.hf_average, ce.tapset_decision, ce.prefilter_period,
            f2u(ce.prefilter_gain), ce.prefilter_tapset, ce.consec_transient,
            ce.vbr_reservoir, ce.vbr_drift, ce.vbr_offset, ce.vbr_count,
            f2u(ce.overlap_max), f2u(ce.stereo_saving), ce.intensity, f2u(ce.spec_avg),
            fnv_f32(&ce.oldBandE[..n]), fnv_f32(&ce.oldLogE[..n]), fnv_f32(&ce.oldLogE2[..n]),
            fnv_f32(&ce.energyError[..n]), ce.analysis.valid,
            fnv_f32(&ce.in_mem), fnv_f32(&ce.prefilter_mem),
            fnv_f32(&ce.preemph_memE) ^ fnv_f32(&ce.preemph_memD)
        );
        if ce.analysis.valid != 0 {
            print_ainfo("CA", k, &ce.analysis);
        } else {
            println!("CA {} valid=0", k);
        }
    } else {
        println!("CE {} skip=1", k);
    }
    print_astate("AN", k, &st.analysis);
    println!("DB {} db={:016x}", k, fnv_f32(&st.delay_buffer[..(st.encoder_buffer * st.channels) as usize]));
}

/* ------------------------------------------------------------------ */
/* section `analysis`: standalone run_analysis drive (oracle cfg 0)   */
/* ------------------------------------------------------------------ */

const CFG0_FRAMES: i32 = 50;
const CFG0_FRAME: i32 = 960;

fn section_analysis() {
    let mut err = 0i32;
    let mode = opus_custom_mode_create(48000, 960, &mut err).expect("mode");
    let mut st = TonalityAnalysisState::default();
    let mut info = AnalysisInfo::default();
    tonality_analysis_init(&mut st, 48000);
    st.application = OPUS_APPLICATION_AUDIO;
    let mut rng = Rng { s: 0x1234567 };
    let mut pcm = vec![0.0f32; (CFG0_FRAME * 2) as usize];
    for f in 0..CFG0_FRAMES {
        for i in 0..pcm.len() {
            let idx = f as usize * pcm.len() + i;
            pcm[i] = rng.sample() * envelope(idx);
        }
        run_analysis(
            &mut st,
            &mode,
            Some(AnalysisPcm::Float(&pcm)),
            CFG0_FRAME,
            CFG0_FRAME,
            0,
            -2,
            2,
            48000,
            16,
            &mut info,
        );
        println!("A {} pcm={:016x}", f, fnv_f32(&pcm));
        print_ainfo("AI", f, &info);
        print_astate("AS", f, &st);
    }
    opus_custom_mode_destroy(Some(mode));
}

/* ------------------------------------------------------------------ */
/* section `cfg0`: the acceptance configuration-0 encode              */
/* ------------------------------------------------------------------ */

fn section_cfg0() {
    let mut err = 0i32;
    let mut enc = OpusEncoder::create(48000, 2, OPUS_APPLICATION_AUDIO, &mut err).expect("enc");
    println!("CFG0 create={}", err);
    {
        let mut v = 96000;
        enc.ctl(OPUS_SET_BITRATE_REQUEST, &mut v);
        let mut v = 10;
        enc.ctl(OPUS_SET_COMPLEXITY_REQUEST, &mut v);
        let mut v = 1;
        enc.ctl(OPUS_SET_VBR_REQUEST, &mut v);
        let mut v = 0;
        enc.ctl(OPUS_SET_VBR_CONSTRAINT_REQUEST, &mut v);
        let mut v = 0;
        enc.ctl(OPUS_SET_DTX_REQUEST, &mut v);
        let mut v = 0;
        enc.ctl(OPUS_SET_INBAND_FEC_REQUEST, &mut v);
        let mut v = 0;
        enc.ctl(OPUS_SET_PACKET_LOSS_PERC_REQUEST, &mut v);
        let mut v = OPUS_AUTO;
        enc.ctl(OPUS_SET_BANDWIDTH_REQUEST, &mut v);
        let mut v = OPUS_AUTO;
        enc.ctl(OPUS_SET_SIGNAL_REQUEST, &mut v);
        let mut v = 16;
        enc.ctl(OPUS_SET_LSB_DEPTH_REQUEST, &mut v);
    }
    let mut rng = Rng { s: 0x1234567 };
    let mut pcm = vec![0.0f32; (CFG0_FRAME * 2) as usize];
    let mut packet = vec![0u8; 1500];
    let mut total = 1469598103934665603u64;
    for f in 0..CFG0_FRAMES {
        for i in 0..pcm.len() {
            let idx = f as usize * pcm.len() + i;
            pcm[i] = rng.sample() * envelope(idx);
        }
        let n = enc.encode_float(&pcm, CFG0_FRAME, &mut packet, 1500);
        print!("F {} n={} fnv={:016x} hex=", f, n, fnv_bytes(&packet[..n.max(0) as usize]));
        for i in 0..n {
            print!("{:02x}", packet[i as usize]);
        }
        println!();
        total = fnv_upd(total, &n.to_le_bytes());
        total = fnv_upd(total, &packet[..n.max(0) as usize]);
        print_state(&enc, f);
    }
    println!("CFG0 total={:016x}", total);
}

/* ------------------------------------------------------------------ */
/* section `sweep`: a wide configuration sweep                        */
/* ------------------------------------------------------------------ */

struct Cfg {
    Fs: i32,
    ch: i32,
    app: i32,
    fsz: i32,
    bitrate: i32,
    vbr: i32,
    cvbr: i32,
    cplx: i32,
    dtx: i32,
    fec: i32,
    loss: i32,
    ubw: i32,
    mbw: i32,
    force: i32,
    sig: i32,
    lsb: i32,
    pred: i32,
    dur: i32,
    maxbytes: i32,
    nframes: i32,
    float_in: i32,
}

const ARG: i32 = 5000;

static CFGS: &[Cfg] = &[
    Cfg { Fs: 48000, ch: 2, app: OPUS_APPLICATION_AUDIO, fsz: 960, bitrate: 96000, vbr: 1, cvbr: 0, cplx: 10, dtx: 0, fec: 0, loss: 0, ubw: -1000, mbw: 1105, force: -1000, sig: -1000, lsb: 16, pred: 0, dur: ARG, maxbytes: 1500, nframes: 5, float_in: 1 },
    Cfg { Fs: 48000, ch: 2, app: OPUS_APPLICATION_AUDIO, fsz: 960, bitrate: 96000, vbr: 1, cvbr: 1, cplx: 10, dtx: 0, fec: 0, loss: 0, ubw: -1000, mbw: 1105, force: -1000, sig: -1000, lsb: 16, pred: 0, dur: ARG, maxbytes: 1500, nframes: 5, float_in: 1 },
    Cfg { Fs: 48000, ch: 2, app: OPUS_APPLICATION_AUDIO, fsz: 960, bitrate: 96000, vbr: 0, cvbr: 0, cplx: 10, dtx: 0, fec: 0, loss: 0, ubw: -1000, mbw: 1105, force: -1000, sig: -1000, lsb: 16, pred: 0, dur: ARG, maxbytes: 1500, nframes: 5, float_in: 1 },
    Cfg { Fs: 48000, ch: 2, app: OPUS_APPLICATION_VOIP, fsz: 960, bitrate: 32000, vbr: 1, cvbr: 1, cplx: 9, dtx: 0, fec: 0, loss: 0, ubw: -1000, mbw: 1105, force: -1000, sig: -1000, lsb: 16, pred: 0, dur: ARG, maxbytes: 1500, nframes: 5, float_in: 0 },
    Cfg { Fs: 48000, ch: 2, app: OPUS_APPLICATION_VOIP, fsz: 960, bitrate: 32000, vbr: 1, cvbr: 1, cplx: 9, dtx: 1, fec: 0, loss: 0, ubw: -1000, mbw: 1105, force: -1000, sig: -1000, lsb: 16, pred: 0, dur: ARG, maxbytes: 1500, nframes: 8, float_in: 0 },
    Cfg { Fs: 48000, ch: 2, app: OPUS_APPLICATION_VOIP, fsz: 960, bitrate: 24000, vbr: 1, cvbr: 1, cplx: 9, dtx: 1, fec: 1, loss: 20, ubw: -1000, mbw: 1105, force: -1000, sig: -1000, lsb: 16, pred: 0, dur: ARG, maxbytes: 1500, nframes: 8, float_in: 0 },
    Cfg { Fs: 48000, ch: 1, app: OPUS_APPLICATION_VOIP, fsz: 960, bitrate: 16000, vbr: 1, cvbr: 1, cplx: 9, dtx: 0, fec: 1, loss: 10, ubw: -1000, mbw: 1105, force: 1, sig: -1000, lsb: 16, pred: 0, dur: ARG, maxbytes: 1500, nframes: 5, float_in: 0 },
    Cfg { Fs: 48000, ch: 2, app: OPUS_APPLICATION_AUDIO, fsz: 120, bitrate: 128000, vbr: 1, cvbr: 1, cplx: 10, dtx: 0, fec: 0, loss: 0, ubw: -1000, mbw: 1105, force: -1000, sig: -1000, lsb: 24, pred: 0, dur: ARG, maxbytes: 1500, nframes: 8, float_in: 1 },
    Cfg { Fs: 48000, ch: 2, app: OPUS_APPLICATION_RESTRICTED_LOWDELAY, fsz: 120, bitrate: 128000, vbr: 1, cvbr: 1, cplx: 10, dtx: 0, fec: 0, loss: 0, ubw: -1000, mbw: 1105, force: -1000, sig: -1000, lsb: 24, pred: 0, dur: ARG, maxbytes: 1500, nframes: 8, float_in: 1 },
    Cfg { Fs: 48000, ch: 2, app: OPUS_APPLICATION_RESTRICTED_LOWDELAY, fsz: 240, bitrate: 96000, vbr: 1, cvbr: 1, cplx: 10, dtx: 0, fec: 0, loss: 0, ubw: -1000, mbw: 1105, force: -1000, sig: -1000, lsb: 24, pred: 0, dur: ARG, maxbytes: 1500, nframes: 5, float_in: 0 },
    Cfg { Fs: 48000, ch: 2, app: OPUS_APPLICATION_AUDIO, fsz: 480, bitrate: 64000, vbr: 1, cvbr: 1, cplx: 9, dtx: 0, fec: 0, loss: 0, ubw: -1000, mbw: 1105, force: -1000, sig: -1000, lsb: 16, pred: 0, dur: ARG, maxbytes: 1500, nframes: 5, float_in: 1 },
    Cfg { Fs: 48000, ch: 2, app: OPUS_APPLICATION_AUDIO, fsz: 1920, bitrate: 64000, vbr: 1, cvbr: 1, cplx: 9, dtx: 0, fec: 0, loss: 0, ubw: -1000, mbw: 1105, force: -1000, sig: -1000, lsb: 16, pred: 0, dur: ARG, maxbytes: 1500, nframes: 3, float_in: 1 },
    Cfg { Fs: 48000, ch: 2, app: OPUS_APPLICATION_AUDIO, fsz: 2880, bitrate: 64000, vbr: 1, cvbr: 1, cplx: 9, dtx: 0, fec: 0, loss: 0, ubw: -1000, mbw: 1105, force: -1000, sig: -1000, lsb: 16, pred: 0, dur: ARG, maxbytes: 1500, nframes: 2, float_in: 0 },
    Cfg { Fs: 48000, ch: 1, app: OPUS_APPLICATION_AUDIO, fsz: 3840, bitrate: 32000, vbr: 1, cvbr: 1, cplx: 9, dtx: 0, fec: 0, loss: 0, ubw: -1000, mbw: 1105, force: -1000, sig: -1000, lsb: 16, pred: 0, dur: 5007, maxbytes: 1500, nframes: 2, float_in: 0 },
    Cfg { Fs: 48000, ch: 1, app: OPUS_APPLICATION_AUDIO, fsz: 4800, bitrate: 32000, vbr: 1, cvbr: 1, cplx: 9, dtx: 0, fec: 0, loss: 0, ubw: -1000, mbw: 1105, force: -1000, sig: -1000, lsb: 16, pred: 0, dur: 5008, maxbytes: 1500, nframes: 2, float_in: 0 },
    Cfg { Fs: 48000, ch: 2, app: OPUS_APPLICATION_AUDIO, fsz: 5760, bitrate: 64000, vbr: 1, cvbr: 1, cplx: 9, dtx: 0, fec: 0, loss: 0, ubw: -1000, mbw: 1105, force: -1000, sig: -1000, lsb: 16, pred: 0, dur: 5009, maxbytes: 1500, nframes: 1, float_in: 0 },
    Cfg { Fs: 24000, ch: 2, app: OPUS_APPLICATION_AUDIO, fsz: 480, bitrate: 48000, vbr: 1, cvbr: 1, cplx: 9, dtx: 0, fec: 0, loss: 0, ubw: -1000, mbw: 1105, force: -1000, sig: -1000, lsb: 16, pred: 0, dur: ARG, maxbytes: 1500, nframes: 5, float_in: 1 },
    Cfg { Fs: 24000, ch: 1, app: OPUS_APPLICATION_VOIP, fsz: 480, bitrate: 24000, vbr: 1, cvbr: 1, cplx: 9, dtx: 0, fec: 0, loss: 0, ubw: -1000, mbw: 1105, force: -1000, sig: -1000, lsb: 16, pred: 0, dur: ARG, maxbytes: 1500, nframes: 5, float_in: 0 },
    Cfg { Fs: 16000, ch: 2, app: OPUS_APPLICATION_AUDIO, fsz: 320, bitrate: 32000, vbr: 1, cvbr: 1, cplx: 9, dtx: 0, fec: 0, loss: 0, ubw: -1000, mbw: 1105, force: -1000, sig: -1000, lsb: 16, pred: 0, dur: ARG, maxbytes: 1500, nframes: 5, float_in: 0 },
    Cfg { Fs: 16000, ch: 1, app: OPUS_APPLICATION_VOIP, fsz: 320, bitrate: 16000, vbr: 1, cvbr: 1, cplx: 9, dtx: 0, fec: 0, loss: 0, ubw: -1000, mbw: 1105, force: -1000, sig: -1000, lsb: 16, pred: 0, dur: ARG, maxbytes: 1500, nframes: 5, float_in: 0 },
    Cfg { Fs: 12000, ch: 1, app: OPUS_APPLICATION_VOIP, fsz: 240, bitrate: 12000, vbr: 1, cvbr: 1, cplx: 9, dtx: 0, fec: 0, loss: 0, ubw: -1000, mbw: 1105, force: -1000, sig: -1000, lsb: 16, pred: 0, dur: ARG, maxbytes: 1500, nframes: 5, float_in: 0 },
    Cfg { Fs: 12000, ch: 2, app: OPUS_APPLICATION_AUDIO, fsz: 240, bitrate: 24000, vbr: 1, cvbr: 1, cplx: 9, dtx: 0, fec: 0, loss: 0, ubw: -1000, mbw: 1105, force: -1000, sig: -1000, lsb: 16, pred: 0, dur: ARG, maxbytes: 1500, nframes: 5, float_in: 0 },
    Cfg { Fs: 8000, ch: 1, app: OPUS_APPLICATION_VOIP, fsz: 160, bitrate: 8000, vbr: 1, cvbr: 1, cplx: 9, dtx: 0, fec: 0, loss: 0, ubw: -1000, mbw: 1105, force: -1000, sig: -1000, lsb: 16, pred: 0, dur: ARG, maxbytes: 1500, nframes: 5, float_in: 0 },
    Cfg { Fs: 8000, ch: 2, app: OPUS_APPLICATION_AUDIO, fsz: 160, bitrate: 16000, vbr: 1, cvbr: 1, cplx: 9, dtx: 0, fec: 0, loss: 0, ubw: -1000, mbw: 1105, force: -1000, sig: -1000, lsb: 16, pred: 0, dur: ARG, maxbytes: 1500, nframes: 5, float_in: 0 },
    Cfg { Fs: 48000, ch: 2, app: OPUS_APPLICATION_AUDIO, fsz: 960, bitrate: -1000, vbr: 1, cvbr: 1, cplx: 9, dtx: 0, fec: 0, loss: 0, ubw: -1000, mbw: 1105, force: -1000, sig: -1000, lsb: 16, pred: 0, dur: ARG, maxbytes: 1500, nframes: 5, float_in: 0 },
    Cfg { Fs: 48000, ch: 2, app: OPUS_APPLICATION_AUDIO, fsz: 960, bitrate: -1, vbr: 1, cvbr: 1, cplx: 9, dtx: 0, fec: 0, loss: 0, ubw: -1000, mbw: 1105, force: -1000, sig: -1000, lsb: 16, pred: 0, dur: ARG, maxbytes: 1500, nframes: 5, float_in: 0 },
    Cfg { Fs: 48000, ch: 2, app: OPUS_APPLICATION_AUDIO, fsz: 960, bitrate: 6000, vbr: 0, cvbr: 0, cplx: 9, dtx: 0, fec: 0, loss: 0, ubw: -1000, mbw: 1105, force: -1000, sig: -1000, lsb: 16, pred: 0, dur: ARG, maxbytes: 1500, nframes: 5, float_in: 0 },
    Cfg { Fs: 48000, ch: 2, app: OPUS_APPLICATION_AUDIO, fsz: 960, bitrate: 64000, vbr: 1, cvbr: 1, cplx: 9, dtx: 0, fec: 0, loss: 0, ubw: 1101, mbw: 1101, force: -1000, sig: -1000, lsb: 16, pred: 0, dur: ARG, maxbytes: 1500, nframes: 5, float_in: 0 },
    Cfg { Fs: 48000, ch: 2, app: OPUS_APPLICATION_AUDIO, fsz: 960, bitrate: 64000, vbr: 1, cvbr: 1, cplx: 9, dtx: 0, fec: 0, loss: 0, ubw: 1104, mbw: 1104, force: -1000, sig: -1000, lsb: 16, pred: 0, dur: ARG, maxbytes: 1500, nframes: 5, float_in: 0 },
    Cfg { Fs: 48000, ch: 2, app: OPUS_APPLICATION_AUDIO, fsz: 960, bitrate: 64000, vbr: 1, cvbr: 1, cplx: 9, dtx: 0, fec: 0, loss: 0, ubw: -1000, mbw: 1103, force: -1000, sig: -1000, lsb: 16, pred: 0, dur: ARG, maxbytes: 1500, nframes: 5, float_in: 0 },
    Cfg { Fs: 48000, ch: 2, app: OPUS_APPLICATION_AUDIO, fsz: 960, bitrate: 64000, vbr: 1, cvbr: 1, cplx: 9, dtx: 0, fec: 0, loss: 0, ubw: -1000, mbw: 1105, force: 1, sig: 3001, lsb: 16, pred: 0, dur: ARG, maxbytes: 1500, nframes: 5, float_in: 0 },
    Cfg { Fs: 48000, ch: 2, app: OPUS_APPLICATION_AUDIO, fsz: 960, bitrate: 64000, vbr: 1, cvbr: 1, cplx: 9, dtx: 0, fec: 0, loss: 0, ubw: -1000, mbw: 1105, force: 2, sig: 3002, lsb: 16, pred: 0, dur: ARG, maxbytes: 1500, nframes: 5, float_in: 0 },
    Cfg { Fs: 48000, ch: 2, app: OPUS_APPLICATION_AUDIO, fsz: 960, bitrate: 64000, vbr: 1, cvbr: 1, cplx: 9, dtx: 0, fec: 0, loss: 0, ubw: -1000, mbw: 1105, force: -1000, sig: -1000, lsb: 8, pred: 0, dur: ARG, maxbytes: 1500, nframes: 5, float_in: 0 },
    Cfg { Fs: 48000, ch: 2, app: OPUS_APPLICATION_AUDIO, fsz: 960, bitrate: 64000, vbr: 1, cvbr: 1, cplx: 9, dtx: 0, fec: 0, loss: 0, ubw: -1000, mbw: 1105, force: -1000, sig: -1000, lsb: 24, pred: 1, dur: ARG, maxbytes: 1500, nframes: 5, float_in: 0 },
    Cfg { Fs: 48000, ch: 2, app: OPUS_APPLICATION_VOIP, fsz: 960, bitrate: 16000, vbr: 1, cvbr: 1, cplx: 9, dtx: 0, fec: 1, loss: 30, ubw: -1000, mbw: 1105, force: -1000, sig: -1000, lsb: 16, pred: 0, dur: ARG, maxbytes: 1500, nframes: 5, float_in: 0 },
    Cfg { Fs: 48000, ch: 2, app: OPUS_APPLICATION_AUDIO, fsz: 960, bitrate: 32000, vbr: 1, cvbr: 1, cplx: 9, dtx: 0, fec: 0, loss: 0, ubw: -1000, mbw: 1105, force: -1000, sig: -1000, lsb: 16, pred: 0, dur: ARG, maxbytes: 200, nframes: 5, float_in: 0 },
    Cfg { Fs: 48000, ch: 2, app: OPUS_APPLICATION_AUDIO, fsz: 960, bitrate: 32000, vbr: 1, cvbr: 1, cplx: 9, dtx: 0, fec: 0, loss: 0, ubw: -1000, mbw: 1105, force: -1000, sig: -1000, lsb: 16, pred: 0, dur: ARG, maxbytes: 80, nframes: 5, float_in: 0 },
    Cfg { Fs: 48000, ch: 1, app: OPUS_APPLICATION_AUDIO, fsz: 960, bitrate: 32000, vbr: 1, cvbr: 1, cplx: 9, dtx: 0, fec: 0, loss: 0, ubw: -1000, mbw: 1105, force: -1000, sig: -1000, lsb: 16, pred: 0, dur: ARG, maxbytes: 40, nframes: 5, float_in: 0 },
    Cfg { Fs: 48000, ch: 2, app: OPUS_APPLICATION_RESTRICTED_SILK, fsz: 960, bitrate: 32000, vbr: 1, cvbr: 1, cplx: 9, dtx: 0, fec: 0, loss: 0, ubw: -1000, mbw: 1105, force: -1000, sig: -1000, lsb: 16, pred: 0, dur: ARG, maxbytes: 1500, nframes: 5, float_in: 0 },
    Cfg { Fs: 48000, ch: 2, app: OPUS_APPLICATION_RESTRICTED_CELT, fsz: 960, bitrate: 32000, vbr: 1, cvbr: 1, cplx: 9, dtx: 0, fec: 0, loss: 0, ubw: -1000, mbw: 1105, force: -1000, sig: -1000, lsb: 16, pred: 0, dur: ARG, maxbytes: 1500, nframes: 5, float_in: 0 },
];

fn sweep_complexity() {
    for cx in 0..=10 {
        let mut err = 0i32;
        let mut enc =
            OpusEncoder::create(48000, 2, OPUS_APPLICATION_AUDIO, &mut err).expect("enc");
        {
            let mut v = 64000;
            enc.ctl(OPUS_SET_BITRATE_REQUEST, &mut v);
            let mut v = cx;
            enc.ctl(OPUS_SET_COMPLEXITY_REQUEST, &mut v);
            let mut v = 1;
            enc.ctl(OPUS_SET_VBR_REQUEST, &mut v);
            let mut v = 1;
            enc.ctl(OPUS_SET_VBR_CONSTRAINT_REQUEST, &mut v);
            let mut v = 0;
            enc.ctl(OPUS_SET_DTX_REQUEST, &mut v);
            let mut v = 0;
            enc.ctl(OPUS_SET_INBAND_FEC_REQUEST, &mut v);
            let mut v = 0;
            enc.ctl(OPUS_SET_PACKET_LOSS_PERC_REQUEST, &mut v);
            let mut v = OPUS_AUTO;
            enc.ctl(OPUS_SET_BANDWIDTH_REQUEST, &mut v);
            let mut v = OPUS_AUTO;
            enc.ctl(OPUS_SET_SIGNAL_REQUEST, &mut v);
            let mut v = 16;
            enc.ctl(OPUS_SET_LSB_DEPTH_REQUEST, &mut v);
        }
        unsafe { LCG_S = 0xabcdef01 };
        println!("CX {}", cx);
        let mut pcm = vec![0i16; 960 * 2];
        let mut packet = vec![0u8; 1500];
        for f in 0..5 {
            gen_pcm_i16(&mut pcm, f, 2);
            let n = enc.encode(&pcm, 960, &mut packet, 1500);
            println!("F {} n={} fnv={:016x}", f, n, fnv_bytes(&packet[..n.max(0) as usize]));
            print_state(&enc, f);
        }
    }
}

fn sweep_ctl() {
    let mut err = 0i32;
    let mut enc = OpusEncoder::create(48000, 2, OPUS_APPLICATION_AUDIO, &mut err).expect("enc");
    let sets: &[(i32, i32)] = &[
        (OPUS_SET_BITRATE_REQUEST, 64000),
        (OPUS_SET_COMPLEXITY_REQUEST, 4),
        (OPUS_SET_VBR_REQUEST, 0),
        (OPUS_SET_VBR_CONSTRAINT_REQUEST, 0),
        (OPUS_SET_DTX_REQUEST, 1),
        (OPUS_SET_INBAND_FEC_REQUEST, 2),
        (OPUS_SET_PACKET_LOSS_PERC_REQUEST, 17),
        (OPUS_SET_BANDWIDTH_REQUEST, OPUS_BANDWIDTH_WIDEBAND),
        (OPUS_SET_MAX_BANDWIDTH_REQUEST, OPUS_BANDWIDTH_SUPERWIDEBAND),
        (OPUS_SET_FORCE_CHANNELS_REQUEST, 1),
        (OPUS_SET_SIGNAL_REQUEST, OPUS_SIGNAL_MUSIC),
        (OPUS_SET_LSB_DEPTH_REQUEST, 14),
        (OPUS_SET_PREDICTION_DISABLED_REQUEST, 1),
        (OPUS_SET_EXPERT_FRAME_DURATION_REQUEST, OPUS_FRAMESIZE_40_MS),
        (OPUS_SET_PHASE_INVERSION_DISABLED_REQUEST, 1),
        (OPUS_SET_VOICE_RATIO_REQUEST, 37),
        (OPUS_SET_FORCE_MODE_REQUEST, MODE_CELT_ONLY),
        (OPUS_SET_LFE_REQUEST, 1),
    ];
    let gets: &[i32] = &[
        OPUS_GET_BITRATE_REQUEST,
        OPUS_GET_COMPLEXITY_REQUEST,
        OPUS_GET_VBR_REQUEST,
        OPUS_GET_VBR_CONSTRAINT_REQUEST,
        OPUS_GET_DTX_REQUEST,
        OPUS_GET_INBAND_FEC_REQUEST,
        OPUS_GET_PACKET_LOSS_PERC_REQUEST,
        OPUS_GET_BANDWIDTH_REQUEST,
        OPUS_GET_MAX_BANDWIDTH_REQUEST,
        OPUS_GET_FORCE_CHANNELS_REQUEST,
        OPUS_GET_SIGNAL_REQUEST,
        OPUS_GET_LSB_DEPTH_REQUEST,
        OPUS_GET_PREDICTION_DISABLED_REQUEST,
        OPUS_GET_EXPERT_FRAME_DURATION_REQUEST,
        OPUS_GET_PHASE_INVERSION_DISABLED_REQUEST,
        OPUS_GET_VOICE_RATIO_REQUEST,
        OPUS_GET_SAMPLE_RATE_REQUEST,
        OPUS_GET_LOOKAHEAD_REQUEST,
        OPUS_GET_APPLICATION_REQUEST,
        OPUS_GET_IN_DTX_REQUEST,
    ];
    println!("CTL create={}", err);
    for (i, &(req, val)) in sets.iter().enumerate() {
        let mut v = val;
        let r = enc.ctl(req, &mut v);
        println!("SET {} -> {}", i, r);
    }
    for (i, &req) in gets.iter().enumerate() {
        let mut v = 0x5a5a5a5a;
        let r = enc.ctl(req, &mut v);
        println!("GET {} -> {} v={}", i, r, v);
    }
    let mut v = 0;
    println!("BAD bitrate={}", enc.ctl(OPUS_SET_BITRATE_REQUEST, &mut v));
    let mut v = 11;
    println!("BAD complexity={}", enc.ctl(OPUS_SET_COMPLEXITY_REQUEST, &mut v));
    let mut v = 7;
    println!("BAD lsb={}", enc.ctl(OPUS_SET_LSB_DEPTH_REQUEST, &mut v));
    let mut v = OPUS_APPLICATION_VOIP;
    println!("BAD app={}", enc.ctl(OPUS_SET_APPLICATION_REQUEST, &mut v));
    let mut v = 0;
    println!("BAD unknown={}", enc.ctl(999999, &mut v));
}

fn sweep_configs() {
    for (c, g) in CFGS.iter().enumerate() {
        let mut err = 0i32;
        let mut enc = match OpusEncoder::create(g.Fs, g.ch, g.app, &mut err) {
            Some(e) => e,
            None => {
                println!(
                    "C {} Fs={} ch={} app={} fsz={} br={} vbr={} cvbr={} cx={} dtx={} fec={} loss={} ubw={} mbw={} force={} sig={} lsb={} pred={} dur={} mb={} nf={} flt={} create={}",
                    c, g.Fs, g.ch, g.app, g.fsz, g.bitrate, g.vbr, g.cvbr, g.cplx, g.dtx, g.fec,
                    g.loss, g.ubw, g.mbw, g.force, g.sig, g.lsb, g.pred, g.dur, g.maxbytes,
                    g.nframes, g.float_in, err
                );
                continue;
            }
        };
        println!(
            "C {} Fs={} ch={} app={} fsz={} br={} vbr={} cvbr={} cx={} dtx={} fec={} loss={} ubw={} mbw={} force={} sig={} lsb={} pred={} dur={} mb={} nf={} flt={} create={}",
            c, g.Fs, g.ch, g.app, g.fsz, g.bitrate, g.vbr, g.cvbr, g.cplx, g.dtx, g.fec, g.loss,
            g.ubw, g.mbw, g.force, g.sig, g.lsb, g.pred, g.dur, g.maxbytes, g.nframes, g.float_in,
            err
        );
        {
            let mut v = g.bitrate;
            enc.ctl(OPUS_SET_BITRATE_REQUEST, &mut v);
            let mut v = g.cplx;
            enc.ctl(OPUS_SET_COMPLEXITY_REQUEST, &mut v);
            let mut v = g.vbr;
            enc.ctl(OPUS_SET_VBR_REQUEST, &mut v);
            let mut v = g.cvbr;
            enc.ctl(OPUS_SET_VBR_CONSTRAINT_REQUEST, &mut v);
            let mut v = g.dtx;
            enc.ctl(OPUS_SET_DTX_REQUEST, &mut v);
            let mut v = g.fec;
            enc.ctl(OPUS_SET_INBAND_FEC_REQUEST, &mut v);
            let mut v = g.loss;
            enc.ctl(OPUS_SET_PACKET_LOSS_PERC_REQUEST, &mut v);
            let mut v = g.ubw;
            enc.ctl(OPUS_SET_BANDWIDTH_REQUEST, &mut v);
            let mut v = g.mbw;
            enc.ctl(OPUS_SET_MAX_BANDWIDTH_REQUEST, &mut v);
            let mut v = g.force;
            enc.ctl(OPUS_SET_FORCE_CHANNELS_REQUEST, &mut v);
            let mut v = g.sig;
            enc.ctl(OPUS_SET_SIGNAL_REQUEST, &mut v);
            let mut v = g.lsb;
            enc.ctl(OPUS_SET_LSB_DEPTH_REQUEST, &mut v);
            let mut v = g.pred;
            enc.ctl(OPUS_SET_PREDICTION_DISABLED_REQUEST, &mut v);
            let mut v = g.dur;
            enc.ctl(OPUS_SET_EXPERT_FRAME_DURATION_REQUEST, &mut v);
        }
        unsafe { LCG_S = 0x12345678u32.wrapping_add(c as u32 * 7919) };
        let total = (g.fsz * g.ch) as usize;
        let mut pcm_i = vec![0i16; 5760 * 2];
        let mut pcm_f = vec![0f32; 5760 * 2];
        let mut packet = vec![0u8; 1500];
        for f in 0..g.nframes {
            let n;
            if g.float_in != 0 {
                gen_pcm_f32(&mut pcm_f[..total], f, g.ch);
                println!("IN {} {:016x}", f, fnv_f32(&pcm_f[..total]));
                n = enc.encode_float(&pcm_f[..total], g.fsz, &mut packet, g.maxbytes);
            } else {
                gen_pcm_i16(&mut pcm_i[..total], f, g.ch);
                println!("IN {} {:016x}", f, fnv_i16(&pcm_i[..total]));
                n = enc.encode(&pcm_i[..total], g.fsz, &mut packet, g.maxbytes);
            }
            print!("F {} n={} fnv={:016x} hex=", f, n, fnv_bytes(&packet[..n.max(0) as usize]));
            for i in 0..n {
                print!("{:02x}", packet[i as usize]);
            }
            println!();
            print_state(&enc, f);
        }
    }
}

fn main() {
    println!(
        "SIZES get_size1={} get_size2={} struct={}",
        OpusEncoder::get_size(1),
        OpusEncoder::get_size(2),
        OPUS_ENCODER_STRUCT_SIZE
    );
    section_analysis();
    section_cfg0();
    sweep_ctl();
    sweep_complexity();
    sweep_configs();
}

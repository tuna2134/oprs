//! Rust side of the SILK decoder-core bit-exactness probe.
//!
//! Reads the op trace written by `compat/c/probe_silk_decode.c` and replays it
//! through the Rust `silk_Decode`, printing the identical format.
//!
//! Run `bash compat/scripts/run-probe.sh silk_decode` from the repository root.

#![allow(non_snake_case)]
#![allow(clippy::needless_range_loop)]

use std::fs::File;
use std::io::Read;

use oprs::celt::entdec::ec_dec_init;
use oprs::silk::control::silk_DecControlStruct;
use oprs::silk::dec_api::{
    silk_Decode, silk_Get_Decoder_Size, silk_InitDecoder, silk_ResetDecoder,
};
use oprs::silk::define::*;
use oprs::silk::fixed::structs_fix::silk_decoder;
use oprs::silk::plc::LPCNetPLCState;
use oprs::silk::structs::{silk_resampler_state_struct, SILK_RESAMPLER_MAX_FIR_ORDER};
use oprs::silk::typedef::*;

const PROBE_ARCH: opus_int = 4;
const MAX_PCM: usize = 8192;

struct Op {
    lostFlag: i32,
    newPacketFlag: i32,
    newCase: i32,
    reset: i32,
    fs_internal: i32,
    payload_ms: i32,
    ch_int: i32,
    ch_api: i32,
    fs_api: i32,
    enable_deep_plc: i32,
    osce_method: i32,
    payload: Vec<u8>,
}

fn read_i32(f: &mut File) -> i32 {
    let mut b = [0u8; 4];
    f.read_exact(&mut b).expect("truncated trace");
    i32::from_le_bytes(b)
}

fn read_ops(path: &str) -> Vec<Op> {
    let mut f = File::open(path).unwrap_or_else(|e| panic!("cannot open {}: {}", path, e));
    let magic = read_i32(&mut f) as u32;
    let version = read_i32(&mut f) as u32;
    assert_eq!(magic, 0x53444543, "bad trace magic");
    assert_eq!(version, 2, "bad trace version");
    let n = read_i32(&mut f);
    let mut ops = Vec::with_capacity(n as usize);
    for _ in 0..n {
        let lostFlag = read_i32(&mut f);
        let newPacketFlag = read_i32(&mut f);
        let newCase = read_i32(&mut f);
        let reset = read_i32(&mut f);
        let fs_internal = read_i32(&mut f);
        let payload_ms = read_i32(&mut f);
        let ch_int = read_i32(&mut f);
        let ch_api = read_i32(&mut f);
        let fs_api = read_i32(&mut f);
        let enable_deep_plc = read_i32(&mut f);
        let osce_method = read_i32(&mut f);
        let nbytes = read_i32(&mut f);
        let mut payload = vec![0u8; nbytes.max(0) as usize];
        if nbytes > 0 {
            f.read_exact(&mut payload).expect("truncated payload");
        }
        ops.push(Op {
            lostFlag,
            newPacketFlag,
            newCase,
            reset,
            fs_internal,
            payload_ms,
            ch_int,
            ch_api,
            fs_api,
            enable_deep_plc,
            osce_method,
            payload,
        });
    }
    ops
}

/* ------------------------------------------------------------------ */
/* Hash helpers (FNV-1a over the raw little-endian bytes)              */
/* ------------------------------------------------------------------ */

fn fnv_init() -> u32 {
    2166136261
}

fn fnv_bytes(h: u32, bytes: &[u8]) -> u32 {
    let mut h = h;
    for &b in bytes {
        h ^= b as u32;
        h = h.wrapping_mul(16777619);
    }
    h
}

fn fnv_i32(h: u32, v: &[i32]) -> u32 {
    let mut bytes = Vec::with_capacity(v.len() * 4);
    for &x in v {
        bytes.extend_from_slice(&x.to_le_bytes());
    }
    fnv_bytes(h, &bytes)
}

fn fnv_i16(h: u32, v: &[i16]) -> u32 {
    let mut bytes = Vec::with_capacity(v.len() * 2);
    for &x in v {
        bytes.extend_from_slice(&x.to_le_bytes());
    }
    fnv_bytes(h, &bytes)
}

fn fnv_f32(h: u32, v: &[f32]) -> u32 {
    let mut bytes = Vec::with_capacity(v.len() * 4);
    for &x in v {
        bytes.extend_from_slice(&x.to_bits().to_le_bytes());
    }
    fnv_bytes(h, &bytes)
}

/* ------------------------------------------------------------------ */
/* Printing                                                            */
/* ------------------------------------------------------------------ */

fn print_resampler(c: i32, r: &silk_resampler_state_struct) {
    /* The C hashes the `i32` view of the `sFIR` union; both sides see the same
       144 bytes. */
    let fir = unsafe { r.sFIR.i32 };
    let fir_hash = fnv_i32(fnv_init(), &fir[..SILK_RESAMPLER_MAX_FIR_ORDER as usize]);
    let del_hash = fnv_i16(fnv_init(), &r.delayBuf);
    println!(
        "st {} resampler sIIR {} {} {} {} {} {} sFIR {:08x} delayBuf {:08x} fn {} batch {} invRatio {} firOrder {} firFracs {} fsin {} fsout {} inputDelay {}",
        c, r.sIIR[0], r.sIIR[1], r.sIIR[2], r.sIIR[3], r.sIIR[4], r.sIIR[5], fir_hash, del_hash,
        r.resampler_function, r.batchSize, r.invRatio_Q16, r.FIR_Order, r.FIR_Fracs, r.Fs_in_kHz,
        r.Fs_out_kHz, r.inputDelay
    );
}

fn print_state(d: &silk_decoder, ctl: &silk_DecControlStruct) {
    println!("ctl prevPitchLag {}", ctl.prevPitchLag);
    println!(
        "top nChAPI {} nChInt {} prev_decode_only_middle {} pred {} {} sMid {} {} sSide {} {}",
        d.nChannelsAPI,
        d.nChannelsInternal,
        d.prev_decode_only_middle,
        d.sStereo.pred_prev_Q13[0],
        d.sStereo.pred_prev_Q13[1],
        d.sStereo.sMid[0],
        d.sStereo.sMid[1],
        d.sStereo.sSide[0],
        d.sStereo.sSide[1]
    );
    for c in 0..2i32 {
        let s = &d.channel_state[c as usize];
        let ix = &s.indices;
        println!(
            "st {} scalars lagPrev {} prevSignalType {} prev_gain_Q16 {} LastGainIndex {} fs_kHz {} fs_API_hz {} nb_subfr {} frame_length {} subfr_length {} ltp_mem_length {} LPC_order {} first_frame_after_reset {} lossCnt {} nFramesDecoded {} nFramesPerPacket {} ec_prevSignalType {} ec_prevLagIndex {} arch {}",
            c, s.lagPrev, s.prevSignalType, s.prev_gain_Q16, s.LastGainIndex, s.fs_kHz, s.fs_API_hz,
            s.nb_subfr, s.frame_length, s.subfr_length, s.ltp_mem_length, s.LPC_order,
            s.first_frame_after_reset, s.lossCnt, s.nFramesDecoded, s.nFramesPerPacket,
            s.ec_prevSignalType, s.ec_prevLagIndex, s.arch
        );
        println!(
            "st {} vad {} {} {} lbrr_flag {} lbrr {} {} {}",
            c,
            s.VAD_flags[0],
            s.VAD_flags[1],
            s.VAD_flags[2],
            s.LBRR_flag,
            s.LBRR_flags[0],
            s.LBRR_flags[1],
            s.LBRR_flags[2]
        );
        println!(
            "st {} hashes exc {:08x} outBuf {:08x} sLPC {:08x} prevNLSF {:08x}",
            c,
            fnv_i32(fnv_init(), &s.exc_Q14),
            fnv_i16(fnv_init(), &s.outBuf),
            fnv_i32(fnv_init(), &s.sLPC_Q14_buf),
            fnv_i16(fnv_init(), &s.prevNLSF_Q15)
        );
        print!(
            "st {} indices sig {} qoff {} interp {} seed {} lag {} contour {} per {} ltpscale {} gains {} {} {} {} ltp {} {} {} {} nlsf",
            c, ix.signalType, ix.quantOffsetType, ix.NLSFInterpCoef_Q2, ix.Seed, ix.lagIndex,
            ix.contourIndex, ix.PERIndex, ix.LTP_scaleIndex, ix.GainsIndices[0], ix.GainsIndices[1],
            ix.GainsIndices[2], ix.GainsIndices[3], ix.LTPIndex[0], ix.LTPIndex[1], ix.LTPIndex[2],
            ix.LTPIndex[3]
        );
        for i in 0..(MAX_LPC_ORDER + 1) as usize {
            print!(" {}", ix.NLSFIndices[i]);
        }
        println!();
        println!(
            "st {} plc pitchL_Q8 {} ltp {} {} {} {} {} prevLPC {:08x} last_frame_lost {} rand_seed {} randScale {} conc_energy {} conc_shift {} prevLTP_scale {} prevGain {} {} fs_kHz {} nb_subfr {} subfr_length {} deep_plc {}",
            c, s.sPLC.pitchL_Q8, s.sPLC.LTPCoef_Q14[0], s.sPLC.LTPCoef_Q14[1],
            s.sPLC.LTPCoef_Q14[2], s.sPLC.LTPCoef_Q14[3], s.sPLC.LTPCoef_Q14[4],
            fnv_i16(fnv_init(), &s.sPLC.prevLPC_Q12), s.sPLC.last_frame_lost, s.sPLC.rand_seed,
            s.sPLC.randScale_Q14, s.sPLC.conc_energy, s.sPLC.conc_energy_shift,
            s.sPLC.prevLTP_scale_Q14, s.sPLC.prevGain_Q16[0], s.sPLC.prevGain_Q16[1], s.sPLC.fs_kHz,
            s.sPLC.nb_subfr, s.sPLC.subfr_length, s.sPLC.enable_deep_plc
        );
        println!(
            "st {} cng smthGain {} rand_seed {} fs_kHz {} exc {:08x} nlsf {:08x} synth {:08x}",
            c,
            s.sCNG.CNG_smth_Gain_Q16,
            s.sCNG.rand_seed,
            s.sCNG.fs_kHz,
            fnv_i32(fnv_init(), &s.sCNG.CNG_exc_buf_Q14),
            fnv_i16(fnv_init(), &s.sCNG.CNG_smth_NLSF_Q15),
            fnv_i32(fnv_init(), &s.sCNG.CNG_synth_state)
        );
        println!(
            "st {} osce method {} reset {}",
            c, s.osce.method, s.osce.features.reset
        );
        print_resampler(c, &s.resampler_state);
    }
}

/* ------------------------------------------------------------------ */
/* Main                                                                */
/* ------------------------------------------------------------------ */

fn main() {
    let path = std::env::var("SILK_DECODE_PACKET_FILE")
        .unwrap_or_else(|_| "compat/out/probe_silk_decode_packets.bin".to_string());
    let ops = read_ops(&path);

    let mut dec = Box::new(silk_decoder::default());
    let mut dec_size: opus_int = 0;
    silk_Get_Decoder_Size(&mut dec_size);
    let _ = dec_size;
    let mut lpcnet = LPCNetPLCState {
        loaded: 1,
        fec_fill_pos: 0,
    };

    let mut pcm = vec![0.0f32; MAX_PCM];

    for (op_idx, o) in ops.iter().enumerate() {
        if o.newCase != 0 {
            silk_InitDecoder(&mut dec);
        }
        if o.reset != 0 {
            silk_ResetDecoder(&mut dec);
        }

        let mut ctl = silk_DecControlStruct::default();
        ctl.nChannelsAPI = o.ch_api;
        ctl.nChannelsInternal = o.ch_int;
        ctl.API_sampleRate = o.fs_api;
        ctl.internalSampleRate = o.fs_internal;
        ctl.payloadSize_ms = o.payload_ms;
        ctl.enable_deep_plc = o.enable_deep_plc;
        ctl.osce_method = o.osce_method;

        let dummy = [0u8; 4];
        let mut range = if o.lostFlag == 1 {
            ec_dec_init(&dummy)
        } else {
            ec_dec_init(&o.payload)
        };

        for v in pcm.iter_mut() {
            *v = 0.0;
        }
        let mut n_out: i32 = 0;
        let ret = silk_Decode(
            &mut dec,
            &mut ctl,
            o.lostFlag,
            o.newPacketFlag,
            &mut range,
            &mut pcm,
            &mut n_out,
            Some(&mut lpcnet),
            PROBE_ARCH,
        );

        println!(
            "== op {} case {} reset {} lost {} newpkt {} fs_int {} ms {} ch_int {} ch_api {} fs_api {} nbytes {} ret {}",
            op_idx, o.newCase, o.reset, o.lostFlag, o.newPacketFlag, o.fs_internal, o.payload_ms,
            o.ch_int, o.ch_api, o.fs_api, o.payload.len(), ret
        );
        let nsamp = (o.ch_api * n_out) as usize;
        let h = fnv_f32(fnv_init(), &pcm[..nsamp]);
        print!("out n {} fnv {:08x}", n_out, h);
        let mut i = 0;
        while i < 4 && i < nsamp {
            print!(" {:08x}", pcm[i].to_bits());
            i += 1;
        }
        i = if nsamp >= 4 { nsamp - 4 } else { 0 };
        while i < nsamp {
            print!(" {:08x}", pcm[i].to_bits());
            i += 1;
        }
        println!();
        print_state(&dec, &ctl);
    }
}

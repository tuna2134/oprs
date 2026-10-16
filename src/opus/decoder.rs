/* Copyright (c) 2010 Xiph.Org Foundation, Skype Limited
   Copyright (c) 2024 Arm Limited
   Written by Jean-Marc Valin and Koen Vos */
/*
   Redistribution and use in source and binary forms, with or without
   modification, are permitted provided that the following conditions
   are met:

   - Redistributions of source code must retain the above copyright
   notice, this list of conditions and the following disclaimer.

   - Redistributions in binary form must reproduce the above copyright
   notice, this list of conditions and the following disclaimer in the
   documentation and/or other materials provided with the distribution.

   THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
   ``AS IS'' AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
   LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
   A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER
   OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL,
   EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO,
   PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR
   PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF
   LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING
   NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
   SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
*/
//! Transliteration of `src/opus_decoder.c` from libopus 1.6.1 into safe Rust.
//!
//! # Configuration
//!
//! Floating point (`FIXED_POINT`, `FLOAT_APPROX`, `ENABLE_QEXT`, `ENABLE_RES24`
//! undefined); `ENABLE_DEEP_PLC`, `ENABLE_DRED`, `ENABLE_OSCE`,
//! `ENABLE_OSCE_BWE`, `CUSTOM_MODES` and `ENABLE_HARDENING` defined.  The
//! `#ifdef ENABLE_QEXT` blocks are dropped (as everywhere else in this port);
//! the `ENABLE_OSCE_BWE` *fields* of `silk_DecControlStruct` do not exist in
//! this port's [`silk_DecControlStruct`] (see `docs/porting-conventions.md`
//! §9, which pins `ENABLE_OSCE_BWE` as undefined), so the two extended-mode
//! values are tracked in `OpusDecoder::osce_extended_mode` /
//! `prev_osce_extended_mode` and set exactly as the C does.  They are never
//! consumed: `silk::dec_api::silk_Decode` drops the `ENABLE_OSCE_BWE` blocks,
//! which is output-neutral because the blind-BWE path additionally requires
//! `OPUS_SET_OSCE_BWE`-style state that the reference reaches only at decoder
//! complexity >= 4.
//!
//! # DNN / DRED
//!
//! `dnn/` is out of scope for this port (`docs/progress.md`, "DNN/DRED
//! scoping").  The `lpcnet_plc_*` and `opus_dred_*` entry points referenced
//! here are clearly-marked stubs; they reproduce the C exactly for packets
//! with **no DRED extension** and for decoder complexity < 5 (the public API
//! cannot select a higher one), which is every packet this port handles:
//!
//! * `lpcnet_plc_update()` / `lpcnet_plc_fec_clear()` only touch DNN state;
//!   `lpcnet_plc_conceal()` is unreachable below complexity 5 (and the DRED
//!   FEC path is unreachable without a DRED extension).
//! * The C reference loads the compiled-in `lpcnet`/OSCE weights, so its
//!   `loaded` flags are 1; this port leaves them 0.  Every consumer below
//!   complexity 5 takes the same branch either way, and the *signal* is
//!   unchanged (the two `_plc_update` variants are no-ops in Rust).
//!
//! # State layout
//!
//! C stores the SILK decoder and the CELT decoder inside one allocation
//! (`(char*)st + st->silk_dec_offset` / `celt_dec_offset`); the port owns them
//! as ordinary fields.  [`OPUS_DECODER_STRUCT_SIZE`] is the C
//! `sizeof(OpusDecoder)` — the value `opus_decoder_get_size()` is built from —
//! and is validated by `examples/probe_opus_decoder.rs` against the reference.
//!
//! `LPCNetPLCState` exists twice in this port (`celt::celt_decoder` and
//! `silk::plc`, with different reduced field sets); C has a single object, so
//! the port keeps the two stubs side by side and initialises/resets both.

use crate::celt::arch::*;
use crate::celt::celt::{
    CELT_SET_CHANNELS_REQUEST, CELT_SET_END_BAND_REQUEST, CELT_SET_SIGNALLING_REQUEST,
    CELT_SET_START_BAND_REQUEST,
};
use crate::celt::celt_decoder::{
    celt_decoder_ctl, celt_decoder_get_size, celt_decoder_init, celt_decode_with_ec,
    celt_decode_with_ec_dred, CELTDecoder, LPCNetPLCState as CeltLPCNetPLCState,
};
use crate::celt::entcode::ec_ctx;
use crate::celt::entdec::{ec_dec_bit_logp, ec_dec_init, ec_dec_uint};
use crate::celt::mathops::{celt_exp2, celt_float2int16};
use crate::ffi;
use crate::opus::packet::{
    opus_packet_get_bandwidth, opus_packet_get_mode, opus_packet_get_nb_channels,
    opus_packet_get_nb_samples, opus_packet_get_samples_per_frame, opus_packet_parse_impl,
    opus_pcm_soft_clip, OpusExtensionIterator, MODE_CELT_ONLY, MODE_HYBRID, MODE_SILK_ONLY,
    OPUS_BANDWIDTH_FULLBAND, OPUS_BANDWIDTH_MEDIUMBAND, OPUS_BANDWIDTH_NARROWBAND,
    OPUS_BANDWIDTH_SUPERWIDEBAND, OPUS_BANDWIDTH_WIDEBAND,
};
use crate::silk::control::silk_DecControlStruct;
use crate::silk::dec_api::{silk_Decode, silk_Get_Decoder_Size, silk_InitDecoder, silk_ResetDecoder};
use crate::silk::decode_frame::{OSCE_METHOD_LACE, OSCE_METHOD_NOLACE, OSCE_METHOD_NONE};
use crate::silk::fixed::structs_fix::silk_decoder;
use crate::silk::init_decoder::opus_select_arch;
use crate::silk::plc::LPCNetPLCState as SilkLPCNetPLCState;
use crate::silk::typedef::opus_int;
use crate::types::*;

/* ------------------------------------------------------------------------- *
 * Constants
 * ------------------------------------------------------------------------- */

/// `sizeof(struct OpusDecoder)` in the reference configuration, i.e. the float
/// build with `ENABLE_DEEP_PLC`/`ENABLE_OSCE` and the `ENABLE_OSCE_BWE` fields
/// of `silk_DecControlStruct` present.  Measured from the reference: a copy of
/// the C struct compiled with the reference headers reports 68112, and
/// `opus_decoder_get_size()` is `align(68112) + align(sizeof(silk_decoder)) +
/// celt_decoder_get_size(channels)`.
///
/// It is a constant for the same reason as
/// [`crate::celt::celt_decoder::CELT_DECODER_STRUCT_SIZE`]: the Rust state
/// struct has a different layout (owned sub-decoders, `Vec`s, opaque DNN
/// blocks), so `size_of::<OpusDecoder>()` is meaningless for the C API.
pub const OPUS_DECODER_STRUCT_SIZE: i32 = 68112;

/// `align(sizeof(silk_decoder))` in the reference configuration.
///
/// `sizeof(silk_decoder)` is 112624 in the reference: `silk_decoder_state`
/// carries both `silk_OSCE_struct osce` and `silk_OSCE_BWE_struct osce_bwe`
/// (`ENABLE_OSCE_BWE` is defined there), and `silk_decoder` adds two of them.
///
/// The port's [`crate::silk::fixed::structs_fix::silk_decoder`] is 75936
/// bytes for two independently documented reasons, so
/// `silk_Get_Decoder_Size()` cannot be used for this constant:
///
/// * `docs/porting-conventions.md` §9 drops the `ENABLE_OSCE_BWE` fields
///   (2 * 18368 bytes), and
/// * three pointer members of `silk_decoder_state` (`pitch_lag_low_bits_iCDF`,
///   `pitch_contour_iCDF`, `psNLSF_CB`) become `&'static [T]` slices, 8 bytes
///   wider each than the C pointers (`docs/progress.md`, "Known deliberate
///   divergences"), i.e. +24 bytes twice.
///
/// `opus_decoder_get_size()` must report the C value, so the measured
/// reference constant is used exactly as `celt_decoder_get_size()` uses
/// `CELT_DECODER_STRUCT_SIZE`.
pub const SILK_DECODER_SIZE: i32 = 112624;

/// `OSCE_MODE_*` (`dnn/osce.h`).  Tracked for fidelity only; see the module
/// comment.
pub const OSCE_MODE_SILK_ONLY: opus_int = 1000;
/// `OSCE_MODE_HYBRID`
pub const OSCE_MODE_HYBRID: opus_int = 1001;
/// `OSCE_MODE_CELT_ONLY`
pub const OSCE_MODE_CELT_ONLY: opus_int = 1002;
/// `OSCE_MODE_SILK_BBWE`
pub const OSCE_MODE_SILK_BBWE: opus_int = 1003;

/// `DRED_NUM_FEATURES` (`dnn/dred_rdovae_constants.h`).
pub const DRED_NUM_FEATURES: i32 = 20;
/// `DRED_NUM_REDUNDANCY_FRAMES` (`dnn/dred_config.h`) = `2*DRED_MAX_LATENTS`.
pub const DRED_NUM_REDUNDANCY_FRAMES: i32 = 2 * 26;

/// `align()` (`src/opus_private.h`): rounds up to the alignment of a
/// `union { void*; opus_int32; opus_val32; }`, which is 8 on x86-64.
#[inline(always)]
fn align(i: i32) -> i32 {
    ((i + 7) / 8) * 8
}

/// `OpusDRED` (`dnn/dred_decoder.h`), reduced to the members
/// [`opus_decode_native`] reads.  The DRED decoder itself is a DNN component
/// and is not ported; no ported entry point can construct this type, so the
/// `dred != NULL` branch of `opus_decode_native` is unreachable from the
/// public API.
pub struct OpusDRED {
    /// `float fec_features[2*DRED_NUM_REDUNDANCY_FRAMES*DRED_NUM_FEATURES]`
    pub fec_features: Vec<f32>,
    /// `float state[DRED_STATE_DIM]`
    pub state: Vec<f32>,
    /// `float latents[(DRED_NUM_REDUNDANCY_FRAMES/2)*(DRED_LATENT_DIM+1)]`
    pub latents: Vec<f32>,
    /// `int nb_latents`
    pub nb_latents: i32,
    /// `int process_stage`
    pub process_stage: i32,
    /// `int dred_offset`
    pub dred_offset: i32,
}

impl Default for OpusDRED {
    fn default() -> Self {
        OpusDRED {
            fec_features: vec![0.0; (2 * DRED_NUM_REDUNDANCY_FRAMES * DRED_NUM_FEATURES) as usize],
            state: Vec::new(),
            latents: Vec::new(),
            nb_latents: 0,
            process_stage: -1,
            dred_offset: 0,
        }
    }
}

/* ------------------------------------------------------------------------- *
 * DNN stubs (`dnn/lpcnet_plc.c`) — see the module comment.
 * ------------------------------------------------------------------------- */

/// `lpcnet_plc_init()` (`dnn/lpcnet_plc.c:65`).  **DNN stub**: the network is
/// not ported.  The C loads the compiled-in weights and calls
/// `lpcnet_plc_reset()`; here the whole reduced state is zeroed, which leaves
/// `loaded == 0` so that no neural path can be selected.
fn lpcnet_plc_init(st: &mut CeltLPCNetPLCState) {
    *st = CeltLPCNetPLCState::default();
}

/// `lpcnet_plc_reset()` (`dnn/lpcnet_plc.c:45`).  **DNN stub**: clears
/// everything from `LPCNET_PLC_RESET_START` (`fec`) onwards.  Clearing the
/// whole state is a superset that keeps `loaded == 0`.
fn lpcnet_plc_reset(st: &mut CeltLPCNetPLCState) {
    *st = CeltLPCNetPLCState::default();
}

/// `lpcnet_plc_reset()` applied to the SILK view of the state.
fn lpcnet_plc_reset_silk(st: &mut SilkLPCNetPLCState) {
    *st = SilkLPCNetPLCState::default();
}

/// `lpcnet_plc_fec_clear()` (`dnn/lpcnet_plc.c:109`).
fn lpcnet_plc_fec_clear(st: &mut CeltLPCNetPLCState) {
    st.fec_read_pos = 0;
    st.fec_fill_pos = 0;
    st.fec_skip = 0;
}

/// `lpcnet_plc_fec_add()` (`dnn/lpcnet_plc.c:99`).  **DNN stub**; only reached
/// through `opus_dred_parse()`/`opus_decoder_dred_decode*()`, which this port
/// does not expose.
fn lpcnet_plc_fec_add(st: &mut CeltLPCNetPLCState, features: Option<&[f32]>) {
    let _ = (st, features);
}

/* ------------------------------------------------------------------------- *
 * struct OpusDecoder (src/opus_decoder.c:65)
 * ------------------------------------------------------------------------- */

/// `struct OpusDecoder` from `src/opus_decoder.c`.
///
/// The two `*_dec_offset` members of the C struct have no counterpart: the
/// SILK and CELT decoders are owned fields here.  Everything from
/// `stream_channels` to `rangeFinal` is the C "reset region"
/// (`OPUS_DECODER_RESET_START`), cleared by `OPUS_RESET_STATE`.
#[derive(Default)]
pub struct OpusDecoder {
    /// `int channels`
    pub channels: i32,
    /// `opus_int32 Fs` — sampling rate at the API level.
    pub Fs: i32,
    /// `silk_DecControlStruct DecControl`
    pub DecControl: silk_DecControlStruct,
    /// `int decode_gain`
    pub decode_gain: i32,
    /// `int complexity`
    pub complexity: i32,
    /// `int ignore_extensions`
    pub ignore_extensions: i32,
    /// `int arch`
    pub arch: i32,
    /// `#ifdef ENABLE_OSCE_BWE`: `DecControl.enable_osce_bwe` (a field this
    /// port's `silk_DecControlStruct` does not carry).
    pub enable_osce_bwe: i32,
    /// `#ifdef ENABLE_OSCE_BWE`: `DecControl.osce_extended_mode`.
    pub osce_extended_mode: i32,
    /// `#ifdef ENABLE_OSCE_BWE`: `DecControl.prev_osce_extended_mode`.
    pub prev_osce_extended_mode: i32,
    /// `#ifdef ENABLE_DEEP_PLC`: `LPCNetPLCState lpcnet` (CELT view).
    pub lpcnet: CeltLPCNetPLCState,
    /* Everything beyond this point gets cleared on a reset. */
    /// `int stream_channels`
    pub stream_channels: i32,
    /// `int bandwidth`
    pub bandwidth: i32,
    /// `int mode`
    pub mode: i32,
    /// `int prev_mode`
    pub prev_mode: i32,
    /// `int frame_size`
    pub frame_size: i32,
    /// `int prev_redundancy`
    pub prev_redundancy: i32,
    /// `int last_packet_duration`
    pub last_packet_duration: i32,
    /// `opus_val16 softclip_mem[2]` (float build only).
    pub softclip_mem: [opus_val16; 2],
    /// `opus_uint32 rangeFinal`
    pub rangeFinal: opus_uint32,
    /* The decoder states C places in the same allocation. */
    /// The `silk_decoder` at `silk_dec_offset` (`#ifdef ENABLE_OSCE` included).
    pub silk_dec: silk_decoder,
    /// The SILK view of `LPCNetPLCState` (see the module comment).
    pub lpcnet_silk: SilkLPCNetPLCState,
    /// The `CELTDecoder` at `celt_dec_offset`.
    pub celt_dec: CELTDecoder,
}

impl OpusDecoder {
    /// `opus_decoder_get_size()` (`src/opus_decoder.c:121`).
    pub fn get_size(channels: i32) -> i32 {
        if channels < 1 || channels > 2 {
            return 0;
        }
        /* The C reads `silk_Get_Decoder_Size()` into `silkDecSizeBytes` and
           aligns it.  The port keeps the call (for the error return) but uses
           the reference constant: the Rust `silk_decoder` is deliberately a
           different size (see `SILK_DECODER_SIZE`). */
        let mut silk_dec_size_bytes: opus_int = 0;
        let ret = silk_Get_Decoder_Size(&mut silk_dec_size_bytes);
        if ret != 0 {
            return 0;
        }
        let silk_dec_size_bytes = align(SILK_DECODER_SIZE);
        let celtDecSizeBytes = celt_decoder_get_size(channels);
        align(OPUS_DECODER_STRUCT_SIZE) + silk_dec_size_bytes + celtDecSizeBytes
    }

    /// `opus_decoder_init()` (`src/opus_decoder.c:135`).
    pub fn init(&mut self, fs: i32, channels: i32) -> i32 {
        if !(fs == 48000 || fs == 24000 || fs == 16000 || fs == 12000 || fs == 8000)
            || (channels != 1 && channels != 2)
        {
            return ffi::OPUS_BAD_ARG;
        }

        /* `OPUS_CLEAR((char*)st, opus_decoder_get_size(channels))`: the whole
           allocation (both sub-decoders included) is zeroed. */
        *self = OpusDecoder::default();

        /* Initialize SILK decoder */
        let mut silkDecSizeBytes: opus_int = 0;
        let ret = silk_Get_Decoder_Size(&mut silkDecSizeBytes);
        if ret != 0 {
            return ffi::OPUS_INTERNAL_ERROR;
        }

        self.stream_channels = channels;
        self.channels = channels;
        self.complexity = 0;

        self.Fs = fs;
        self.DecControl.API_sampleRate = self.Fs;
        self.DecControl.nChannelsAPI = self.channels;

        /* Reset decoder */
        let ret = silk_InitDecoder(&mut self.silk_dec);
        if ret != 0 {
            return ffi::OPUS_INTERNAL_ERROR;
        }

        /* Initialize CELT decoder */
        let ret = celt_decoder_init(&mut self.celt_dec, fs, channels);
        if ret != ffi::OPUS_OK {
            return ffi::OPUS_INTERNAL_ERROR;
        }

        let mut signalling = 0i32;
        celt_decoder_ctl(&mut self.celt_dec, CELT_SET_SIGNALLING_REQUEST, &mut signalling);

        self.prev_mode = 0;
        self.frame_size = fs / 400;

        lpcnet_plc_init(&mut self.lpcnet);
        lpcnet_plc_reset_silk(&mut self.lpcnet_silk);

        self.arch = opus_select_arch();
        ffi::OPUS_OK
    }

    /// `opus_decoder_create()` (`src/opus_decoder.c:186`).
    ///
    /// The C distinguishes "bad argument" from "allocation failed"; Rust's
    /// `Box` cannot fail, so `OPUS_ALLOC_FAIL` is unreachable here.
    pub fn create(fs: i32, channels: i32, error: &mut i32) -> Option<Box<OpusDecoder>> {
        if !(fs == 48000 || fs == 24000 || fs == 16000 || fs == 12000 || fs == 8000)
            || (channels != 1 && channels != 2)
        {
            *error = ffi::OPUS_BAD_ARG;
            return None;
        }
        let mut st = Box::new(OpusDecoder::default());
        let ret = st.init(fs, channels);
        *error = ret;
        if ret != ffi::OPUS_OK {
            return None;
        }
        Some(st)
    }

    /// `opus_decoder_destroy()` (`src/opus_decoder.c:1244`): the owned `Box`
    /// makes this a no-op, kept for contract parity.
    pub fn destroy(self) {}

    /// `opus_decoder_ctl()` (`src/opus_decoder.c:1031`).
    ///
    /// `value` is an in/out integer: GET requests write their result into it,
    /// SET requests read the value from it (this is how the variadic C
    /// function is exposed; see `docs/internal-api.md`).
    pub fn ctl(&mut self, request: i32, value: &mut i32) -> i32 {
        match request {
            ffi::OPUS_GET_BANDWIDTH_REQUEST => {
                *value = self.bandwidth;
            }
            ffi::OPUS_SET_COMPLEXITY_REQUEST => {
                if *value < 0 || *value > 10 {
                    return ffi::OPUS_BAD_ARG;
                }
                self.complexity = *value;
                celt_decoder_ctl(&mut self.celt_dec, ffi::OPUS_SET_COMPLEXITY_REQUEST, value);
            }
            ffi::OPUS_GET_COMPLEXITY_REQUEST => {
                *value = self.complexity;
            }
            ffi::OPUS_SET_OSCE_BWE_REQUEST => {
                if *value < 0 || *value > 1 {
                    return ffi::OPUS_BAD_ARG;
                }
                self.enable_osce_bwe = *value;
            }
            ffi::OPUS_GET_OSCE_BWE_REQUEST => {
                *value = self.enable_osce_bwe;
            }
            ffi::OPUS_GET_FINAL_RANGE_REQUEST => {
                *value = self.rangeFinal as i32;
            }
            ffi::OPUS_RESET_STATE => {
                /* `OPUS_CLEAR((char*)&st->OPUS_DECODER_RESET_START, ...)` */
                self.stream_channels = 0;
                self.bandwidth = 0;
                self.mode = 0;
                self.prev_mode = 0;
                self.frame_size = 0;
                self.prev_redundancy = 0;
                self.last_packet_duration = 0;
                self.softclip_mem = [0.0; 2];
                self.rangeFinal = 0;

                celt_decoder_ctl(&mut self.celt_dec, ffi::OPUS_RESET_STATE, value);
                silk_ResetDecoder(&mut self.silk_dec);
                self.stream_channels = self.channels;
                self.frame_size = self.Fs / 400;
                lpcnet_plc_reset(&mut self.lpcnet);
                lpcnet_plc_reset_silk(&mut self.lpcnet_silk);
            }
            ffi::OPUS_GET_SAMPLE_RATE_REQUEST => {
                *value = self.Fs;
            }
            ffi::OPUS_GET_PITCH_REQUEST => {
                if self.prev_mode == MODE_CELT_ONLY {
                    return celt_decoder_ctl(&mut self.celt_dec, ffi::OPUS_GET_PITCH_REQUEST, value);
                } else {
                    *value = self.DecControl.prevPitchLag;
                }
            }
            ffi::OPUS_GET_GAIN_REQUEST => {
                *value = self.decode_gain;
            }
            ffi::OPUS_SET_GAIN_REQUEST => {
                if *value < -32768 || *value > 32767 {
                    return ffi::OPUS_BAD_ARG;
                }
                self.decode_gain = *value;
            }
            ffi::OPUS_GET_LAST_PACKET_DURATION_REQUEST => {
                *value = self.last_packet_duration;
            }
            ffi::OPUS_SET_PHASE_INVERSION_DISABLED_REQUEST => {
                if *value < 0 || *value > 1 {
                    return ffi::OPUS_BAD_ARG;
                }
                return celt_decoder_ctl(
                    &mut self.celt_dec,
                    ffi::OPUS_SET_PHASE_INVERSION_DISABLED_REQUEST,
                    value,
                );
            }
            ffi::OPUS_GET_PHASE_INVERSION_DISABLED_REQUEST => {
                return celt_decoder_ctl(
                    &mut self.celt_dec,
                    ffi::OPUS_GET_PHASE_INVERSION_DISABLED_REQUEST,
                    value,
                );
            }
            ffi::OPUS_SET_IGNORE_EXTENSIONS_REQUEST => {
                if *value < 0 || *value > 1 {
                    return ffi::OPUS_BAD_ARG;
                }
                self.ignore_extensions = *value;
            }
            ffi::OPUS_GET_IGNORE_EXTENSIONS_REQUEST => {
                *value = self.ignore_extensions;
            }
            _ => {
                /*fprintf(stderr, "unknown opus_decoder_ctl() request: %d", request);*/
                return ffi::OPUS_UNIMPLEMENTED;
            }
        }
        ffi::OPUS_OK
    }

    /// `opus_decode()` (`src/opus_decoder.c:896`, float body).
    pub fn decode(
        &mut self,
        data: Option<&[u8]>,
        len: i32,
        pcm: &mut [i16],
        frame_size: i32,
        decode_fec: i32,
    ) -> i32 {
        if frame_size <= 0 {
            return ffi::OPUS_BAD_ARG;
        }
        let mut frame_size = frame_size;
        if data.is_some() && len > 0 && decode_fec == 0 {
            let nb_samples = opus_decoder_get_nb_samples(self, data.unwrap(), len);
            if nb_samples > 0 {
                frame_size = IMIN(frame_size, nb_samples);
            } else {
                return ffi::OPUS_INVALID_PACKET;
            }
        }
        celt_assert!(self.channels == 1 || self.channels == 2);
        let mut out = vec![0 as opus_res; (frame_size * self.channels) as usize];

        let ret = opus_decode_native(
            self, data, len, &mut out, frame_size, decode_fec, 0, None, 1, None, 0,
        );
        if ret > 0 {
            let n = ((ret * self.channels) as usize).min(pcm.len()).min(out.len());
            celt_float2int16(&out[..n], &mut pcm[..n], n as i32, self.arch);
        }
        ret
    }

    /// `opus_decode24()` (`src/opus_decoder.c:945`, float body: `FIXED_POINT`
    /// and `ENABLE_RES24` are both undefined).
    ///
    /// This method is an addition to the set documented in
    /// `docs/internal-api.md`; `src/coder/decoder.rs` does not use it.
    pub fn decode24(
        &mut self,
        data: Option<&[u8]>,
        len: i32,
        pcm: &mut [i32],
        frame_size: i32,
        decode_fec: i32,
    ) -> i32 {
        if frame_size <= 0 {
            return ffi::OPUS_BAD_ARG;
        }
        let mut frame_size = frame_size;
        if data.is_some() && len > 0 && decode_fec == 0 {
            let nb_samples = opus_decoder_get_nb_samples(self, data.unwrap(), len);
            if nb_samples > 0 {
                frame_size = IMIN(frame_size, nb_samples);
            } else {
                return ffi::OPUS_INVALID_PACKET;
            }
        }
        celt_assert!(self.channels == 1 || self.channels == 2);
        let mut out = vec![0 as opus_res; (frame_size * self.channels) as usize];

        let ret = opus_decode_native(
            self, data, len, &mut out, frame_size, decode_fec, 0, None, 0, None, 0,
        );
        if ret > 0 {
            let n = ((ret * self.channels) as usize).min(pcm.len()).min(out.len());
            for i in 0..n {
                pcm[i] = RES2INT24(out[i]);
            }
        }
        ret
    }

    /// `opus_decode_float()` (`src/opus_decoder.c:985`, float body: no
    /// conversion, `soft_clip == 0`).
    pub fn decode_float(
        &mut self,
        data: Option<&[u8]>,
        len: i32,
        pcm: &mut [f32],
        frame_size: i32,
        decode_fec: i32,
    ) -> i32 {
        if frame_size <= 0 {
            return ffi::OPUS_BAD_ARG;
        }
        opus_decode_native(self, data, len, pcm, frame_size, decode_fec, 0, None, 0, None, 0)
    }

    /// `opus_decoder_get_nb_samples()` (`src/opus_decoder.c:1333`) as a
    /// method; the free function of the same name is the literal port.
    pub fn get_nb_samples(&self, packet: &[u8], len: i32) -> i32 {
        opus_decoder_get_nb_samples(self, packet, len)
    }
}

/// `opus_decoder_get_nb_samples()` (`src/opus_decoder.c:1333`).
pub fn opus_decoder_get_nb_samples(dec: &OpusDecoder, packet: &[u8], len: opus_int32) -> i32 {
    opus_packet_get_nb_samples(packet, len, dec.Fs)
}

/* ------------------------------------------------------------------------- *
 * smooth_fade() (src/opus_decoder.c:237, float body)
 * ------------------------------------------------------------------------- */

/// `smooth_fade()` (`src/opus_decoder.c:237`), `!ENABLE_RES24` body.
///
/// The C passes `out == in1` (the first redundancy block) or `out == in2`
/// (every other call site); the element-wise loop reads both inputs at index
/// `i*channels+c` before writing the same index, so the aliasing cannot be
/// observed.  The port therefore takes the destination plus the *other* input
/// and a flag saying whether the destination is the C `in1` or `in2`.
fn smooth_fade(
    dst: &mut [opus_res],
    other: &[opus_res],
    dst_is_in1: bool,
    overlap: i32,
    channels: i32,
    window: &[celt_coef],
    Fs: opus_int32,
) {
    let inc = 48000 / Fs;
    let mut c = 0;
    while c < channels {
        let mut i = 0;
        while i < overlap {
            let k = (i * channels + c) as usize;
            let w = COEF2VAL16(window[(i * inc) as usize]);
            let w = MULT16_16_Q15(w, w);
            let (in1, in2) = if dst_is_in1 {
                (dst[k], other[k])
            } else {
                (other[k], dst[k])
            };
            dst[k] = SHR32(MAC16_16(MULT16_16(w, in2), Q15ONE - w, in1), 15);
            i += 1;
        }
        c += 1;
    }
}

/* ------------------------------------------------------------------------- *
 * opus_decode_frame() (src/opus_decoder.c:271)
 * ------------------------------------------------------------------------- */

/// `opus_decode_frame()` (`src/opus_decoder.c:271`).
///
/// `data == None` is the C `data == NULL` (PLC); `len` is the C `len`
/// argument.  `pcm` is the destination pointer, i.e. it may be longer than
/// `frame_size*channels` (the C only guarantees that much is usable).
fn opus_decode_frame(
    st: &mut OpusDecoder,
    data: Option<&[u8]>,
    len_in: i32,
    mut pcm: &mut [opus_res],
    frame_size_in: i32,
    decode_fec: i32,
) -> i32 {
    let mut data = data;
    let mut len = len_in;
    let mut frame_size = frame_size_in;
    let celt_ret: i32;
    let mut dec: ec_ctx;
    let mut audiosize: i32;
    let mode: i32;
    let bandwidth: i32;
    let mut transition = 0i32;
    let mut redundancy = 0i32;
    let mut redundancy_bytes = 0i32;
    let mut celt_to_silk = 0i32;
    let mut redundant_rng: opus_uint32 = 0;

    let F20 = st.Fs / 50;
    let F10 = F20 >> 1;
    let F5 = F10 >> 1;
    let F2_5 = F5 >> 1;
    if frame_size < F2_5 {
        return ffi::OPUS_BUFFER_TOO_SMALL;
    }
    /* Limit frame_size to avoid excessive stack allocations. */
    frame_size = IMIN(frame_size, st.Fs / 25 * 3);
    /* Payloads of 1 (2 including ToC) or 0 trigger the PLC/DTX */
    if len <= 1 {
        data = None;
        /* In that case, don't conceal more than what the ToC says */
        frame_size = IMIN(frame_size, st.frame_size);
    }
    if data.is_some() {
        audiosize = st.frame_size;
        mode = st.mode;
        bandwidth = st.bandwidth;
        let d = data.unwrap();
        let n = (len.max(0) as usize).min(d.len());
        dec = ec_dec_init(&d[..n]);
    } else {
        audiosize = frame_size;
        /* Run PLC using last used mode (CELT if we ended with CELT redundancy) */
        mode = if st.prev_redundancy != 0 { MODE_CELT_ONLY } else { st.prev_mode };
        bandwidth = 0;

        if mode == 0 {
            /* If we haven't got any packet yet, all we can do is return zeros */
            for i in 0..(audiosize * st.channels) as usize {
                pcm[i] = 0.0;
            }
            return audiosize;
        }

        /* Avoids trying to run the PLC on sizes other than 2.5 (CELT), 5 (CELT),
           10, or 20 (e.g. 12.5 or 30 ms). */
        if audiosize > F20 {
            let mut remaining = audiosize;
            let mut off = 0i32;
            loop {
                let ret =
                    opus_decode_frame(st, None, 0, &mut pcm[off as usize..], IMIN(remaining, F20), 0);
                if ret < 0 {
                    return ret;
                }
                off += ret * st.channels;
                remaining -= ret;
                if remaining <= 0 {
                    break;
                }
            }
            return frame_size;
        } else if audiosize < F20 {
            if audiosize > F10 {
                audiosize = F10;
            } else if mode != MODE_SILK_ONLY && audiosize > F5 && audiosize < F10 {
                audiosize = F5;
            }
        }
        /* The C leaves `dec` uninitialised on this path; it is only ever read
           by the `mode == MODE_SILK_ONLY` branch below and overwritten by
           `if (len <= 1) st->rangeFinal = 0;` (data == NULL implies len == 0),
           and the CELT branch returns before looking at it. */
        dec = ec_dec_init(&[]);
    }

    /* Port-only invariant (the C relies on it silently): every branch above
       keeps `audiosize` at or above the 2.5 ms floor checked at entry, which
       is what makes the fixed `F2_5`-sized slices of `pcm` below sound. */
    celt_assert!(audiosize >= F2_5);

    /* In fixed-point, we can tell CELT to do the accumulation on top of the
       SILK PCM buffer. This saves some stack space. */
    let celt_accum = (mode != MODE_CELT_ONLY) as i32;

    if data.is_some()
        && st.prev_mode > 0
        && ((mode == MODE_CELT_ONLY
            && st.prev_mode != MODE_CELT_ONLY
            && st.prev_redundancy == 0)
            || (mode != MODE_CELT_ONLY && st.prev_mode == MODE_CELT_ONLY))
    {
        transition = 1;
    }
    /* `pcm_transition_{celt,silk}` and `redundant_audio` are the C stack
       arrays `ALLOC(..., ALLOC_NONE=1, ...)` of `F5*channels` elements; both
       are allocated unconditionally here (a VLA has no observable side
       effect). */
    let mut pcm_transition_celt = vec![0 as opus_res; (F5 * st.channels).max(1) as usize];
    let mut pcm_transition_silk = vec![0 as opus_res; (F5 * st.channels).max(1) as usize];
    if transition != 0 && mode == MODE_CELT_ONLY {
        opus_decode_frame(st, None, 0, &mut pcm_transition_celt, IMIN(F5, audiosize), 0);
    }
    if audiosize > frame_size {
        /*fprintf(stderr, "PCM buffer too small: %d vs %d (mode = %d)\n", audiosize, frame_size, mode);*/
        return ffi::OPUS_BAD_ARG;
    } else {
        frame_size = audiosize;
    }

    /* SILK processing */
    if mode != MODE_CELT_ONLY {
        let lost_flag: i32;
        let mut decoded_samples: i32;
        let pcm_too_small = frame_size < F10;
        let mut pcm_silk = vec![0 as opus_res; (F10 * st.channels).max(1) as usize];

        if st.prev_mode == MODE_CELT_ONLY {
            silk_ResetDecoder(&mut st.silk_dec);
        }

        /* The SILK PLC cannot produce frames of less than 10 ms */
        st.DecControl.payloadSize_ms = IMAX(10, 1000 * audiosize / st.Fs);

        if data.is_some() {
            st.DecControl.nChannelsInternal = st.stream_channels;
            if mode == MODE_SILK_ONLY {
                if bandwidth == OPUS_BANDWIDTH_NARROWBAND {
                    st.DecControl.internalSampleRate = 8000;
                } else if bandwidth == OPUS_BANDWIDTH_MEDIUMBAND {
                    st.DecControl.internalSampleRate = 12000;
                } else if bandwidth == OPUS_BANDWIDTH_WIDEBAND {
                    st.DecControl.internalSampleRate = 16000;
                } else {
                    st.DecControl.internalSampleRate = 16000;
                    celt_assert!(false);
                }
            } else {
                /* Hybrid mode */
                st.DecControl.internalSampleRate = 16000;
            }
        }
        st.DecControl.enable_deep_plc = (st.complexity >= 5) as i32;
        st.DecControl.osce_method = OSCE_METHOD_NONE;
        if st.complexity >= 6 {
            st.DecControl.osce_method = OSCE_METHOD_LACE;
        }
        if st.complexity >= 7 {
            st.DecControl.osce_method = OSCE_METHOD_NOLACE;
        }
        /* `#ifdef ENABLE_OSCE_BWE`: the extended mode lives in an
           `OpusDecoder` field in this port (see the module comment). */
        if st.complexity >= 4
            && st.enable_osce_bwe != 0
            && st.Fs == 48000
            && st.DecControl.internalSampleRate == 16000
            && (mode == MODE_SILK_ONLY || data.is_none())
        {
            /* request WB -> FB signal extension */
            st.osce_extended_mode = OSCE_MODE_SILK_BBWE;
        } else {
            /* at this point, mode can only be MODE_SILK_ONLY or MODE_HYBRID */
            st.osce_extended_mode = if mode == MODE_SILK_ONLY {
                OSCE_MODE_SILK_ONLY
            } else {
                OSCE_MODE_HYBRID
            };
        }
        if st.prev_mode == MODE_CELT_ONLY {
            /* Update extended mode for CELT->SILK transition */
            st.prev_osce_extended_mode = OSCE_MODE_CELT_ONLY;
        }

        lost_flag = if data.is_none() {
            1
        } else {
            2 * (decode_fec != 0) as i32
        };
        decoded_samples = 0;
        let mut pcm_ptr_off: i32 = 0;
        loop {
            /* Call SILK decoder */
            let first_frame = (decoded_samples == 0) as i32;
            let mut silk_frame_size: opus_int32 = 0;
            let arch = st.arch;
            let out_slice: &mut [opus_res] = if pcm_too_small {
                &mut pcm_silk[pcm_ptr_off as usize..]
            } else {
                &mut pcm[pcm_ptr_off as usize..]
            };
            let silk_ret = silk_Decode(
                &mut st.silk_dec,
                &mut st.DecControl,
                lost_flag,
                first_frame,
                &mut dec,
                out_slice,
                &mut silk_frame_size,
                Some(&mut st.lpcnet_silk),
                arch,
            );
            if silk_ret != 0 {
                if lost_flag != 0 {
                    /* PLC failure should not be fatal */
                    silk_frame_size = frame_size;
                    let base = pcm_ptr_off as usize;
                    let n = (frame_size * st.channels) as usize;
                    let buf: &mut [opus_res] = if pcm_too_small {
                        &mut pcm_silk
                    } else {
                        &mut pcm
                    };
                    buf[base..base + n].fill(0.0);
                } else {
                    return ffi::OPUS_INTERNAL_ERROR;
                }
            }
            /* `#ifdef ENABLE_OSCE_BWE`, at the end of `silk_Decode()`
               (`silk/dec_API.c:437`): `decControl->prev_osce_extended_mode =
               decControl->osce_extended_mode`.  The port's
               `silk_DecControlStruct` has no such field, so the update lives
               here (see the module comment). */
            st.prev_osce_extended_mode = st.osce_extended_mode;
            pcm_ptr_off += silk_frame_size * st.channels;
            decoded_samples += silk_frame_size;
            if decoded_samples >= frame_size {
                break;
            }
        }
        if pcm_too_small {
            let n = (frame_size * st.channels) as usize;
            pcm[..n].copy_from_slice(&pcm_silk[..n]);
        }
    }

    let mut start_band = 0i32;
    if decode_fec == 0
        && mode != MODE_CELT_ONLY
        && data.is_some()
        && dec.ec_tell() + 17 + 20 * ((mode == MODE_HYBRID) as i32) <= 8 * len
    {
        /* Check if we have a redundant 0-8 kHz band */
        if mode == MODE_HYBRID {
            redundancy = ec_dec_bit_logp(&mut dec, 12);
        } else {
            redundancy = 1;
        }
        if redundancy != 0 {
            celt_to_silk = ec_dec_bit_logp(&mut dec, 1);
            /* redundancy_bytes will be at least two, in the non-hybrid
               case due to the ec_tell() check above */
            redundancy_bytes = if mode == MODE_HYBRID {
                (ec_dec_uint(&mut dec, 256) as i32) + 2
            } else {
                len - ((dec.ec_tell() + 7) >> 3)
            };
            len -= redundancy_bytes;
            /* This is a sanity check. It should never happen for a valid
               packet, so the exact behaviour is not normative. */
            if len * 8 < dec.ec_tell() {
                len = 0;
                redundancy_bytes = 0;
                redundancy = 0;
            }
            /* Shrink decoder because of raw bits */
            dec.storage = dec.storage.wrapping_sub(redundancy_bytes as opus_uint32);
        }
    }
    if mode != MODE_CELT_ONLY {
        start_band = 17;
    }

    if redundancy != 0 {
        transition = 0;
    }

    if transition != 0 && mode != MODE_CELT_ONLY {
        opus_decode_frame(st, None, 0, &mut pcm_transition_silk, IMIN(F5, audiosize), 0);
    }

    if bandwidth != 0 {
        let mut endband = 21;
        match bandwidth {
            OPUS_BANDWIDTH_NARROWBAND => endband = 13,
            OPUS_BANDWIDTH_MEDIUMBAND | OPUS_BANDWIDTH_WIDEBAND => endband = 17,
            OPUS_BANDWIDTH_SUPERWIDEBAND => endband = 19,
            OPUS_BANDWIDTH_FULLBAND => endband = 21,
            _ => {
                celt_assert!(false);
            }
        }
        celt_decoder_ctl(&mut st.celt_dec, CELT_SET_END_BAND_REQUEST, &mut endband);
    }
    let mut stream_channels = st.stream_channels;
    celt_decoder_ctl(&mut st.celt_dec, CELT_SET_CHANNELS_REQUEST, &mut stream_channels);

    /* Only allocation memory for redundancy if/when needed */
    let mut redundant_audio = vec![0 as opus_res; (F5 * st.channels).max(1) as usize];

    /* 5 ms redundant frame for CELT->SILK*/
    if redundancy != 0 && celt_to_silk != 0 {
        /* If the previous frame did not use CELT (the first redundancy frame in
           a transition from SILK may have been lost) then the CELT decoder is
           stale at this point and the redundancy audio is not useful, however
           the final range is still needed (for testing), so the redundancy is
           always decoded but the decoded audio may not be used */
        let mut start = 0i32;
        celt_decoder_ctl(&mut st.celt_dec, CELT_SET_START_BAND_REQUEST, &mut start);
        let red: Option<&[u8]> = data.map(|d| &d[len as usize..]);
        celt_decode_with_ec(
            &mut st.celt_dec,
            red,
            redundancy_bytes,
            &mut redundant_audio,
            F5,
            None,
            0,
        );
        let mut v = 0i32;
        celt_decoder_ctl(&mut st.celt_dec, ffi::OPUS_GET_FINAL_RANGE_REQUEST, &mut v);
        redundant_rng = v as opus_uint32;
    }

    /* MUST be after PLC */
    let mut start_band_v = start_band;
    celt_decoder_ctl(&mut st.celt_dec, CELT_SET_START_BAND_REQUEST, &mut start_band_v);

    if mode != MODE_SILK_ONLY {
        let celt_frame_size = IMIN(F20, frame_size);
        /* Make sure to discard any previous CELT state */
        if mode != st.prev_mode && st.prev_mode > 0 && st.prev_redundancy == 0 {
            let mut v = 0i32;
            celt_decoder_ctl(&mut st.celt_dec, ffi::OPUS_RESET_STATE, &mut v);
        }
        /* Decode CELT */
        let celt_data: Option<&[u8]> = if decode_fec != 0 { None } else { data };
        celt_ret = celt_decode_with_ec_dred(
            &mut st.celt_dec,
            celt_data,
            len,
            pcm,
            celt_frame_size,
            Some(&mut dec),
            celt_accum,
            Some(&mut st.lpcnet),
        );
        let mut v = 0i32;
        celt_decoder_ctl(&mut st.celt_dec, ffi::OPUS_GET_FINAL_RANGE_REQUEST, &mut v);
        st.rangeFinal = v as opus_uint32;
    } else {
        let silence: [u8; 2] = [0xFF, 0xFF];
        if celt_accum == 0 {
            for i in 0..(frame_size * st.channels) as usize {
                pcm[i] = 0.0;
            }
        }
        /* For hybrid -> SILK transitions, we let the CELT MDCT
           do a fade-out by decoding a silence frame */
        if st.prev_mode == MODE_HYBRID
            && !(redundancy != 0 && celt_to_silk != 0 && st.prev_redundancy != 0)
        {
            let mut start = 0i32;
            celt_decoder_ctl(&mut st.celt_dec, CELT_SET_START_BAND_REQUEST, &mut start);
            celt_decode_with_ec(&mut st.celt_dec, Some(&silence), 2, pcm, F2_5, None, celt_accum);
        }
        st.rangeFinal = dec.rng;
        celt_ret = 0;
    }

    let window: Vec<celt_coef> = st.celt_dec.mode.window.clone();

    /* 5 ms redundant frame for SILK->CELT */
    if redundancy != 0 && celt_to_silk == 0 {
        let mut v = 0i32;
        celt_decoder_ctl(&mut st.celt_dec, ffi::OPUS_RESET_STATE, &mut v);
        let mut start = 0i32;
        celt_decoder_ctl(&mut st.celt_dec, CELT_SET_START_BAND_REQUEST, &mut start);

        let red: Option<&[u8]> = data.map(|d| &d[len as usize..]);
        celt_decode_with_ec(
            &mut st.celt_dec,
            red,
            redundancy_bytes,
            &mut redundant_audio,
            F5,
            None,
            0,
        );
        let mut v = 0i32;
        celt_decoder_ctl(&mut st.celt_dec, ffi::OPUS_GET_FINAL_RANGE_REQUEST, &mut v);
        redundant_rng = v as opus_uint32;
        let off = (st.channels * (frame_size - F2_5)) as usize;
        let red_off = (st.channels * F2_5) as usize;
        /* out == in1 (pcm), in2 == redundant_audio+F2_5 */
        smooth_fade(
            &mut pcm[off..],
            &redundant_audio[red_off..],
            true,
            F2_5,
            st.channels,
            &window,
            st.Fs,
        );
    }
    /* 5ms redundant frame for CELT->SILK; ignore if the previous frame did not
       use CELT (the first redundancy frame in a transition from SILK may have
       been lost) */
    if redundancy != 0
        && celt_to_silk != 0
        && (st.prev_mode != MODE_SILK_ONLY || st.prev_redundancy != 0)
    {
        let mut c = 0;
        while c < st.channels {
            let mut i = 0;
            while i < F2_5 {
                pcm[(st.channels * i + c) as usize] =
                    redundant_audio[(st.channels * i + c) as usize];
                i += 1;
            }
            c += 1;
        }
        let off = (st.channels * F2_5) as usize;
        /* out == in2 (pcm), in1 == redundant_audio+F2_5 */
        smooth_fade(
            &mut pcm[off..],
            &redundant_audio[off..],
            false,
            F2_5,
            st.channels,
            &window,
            st.Fs,
        );
    }
    if transition != 0 {
        let pcm_transition: &[opus_res] = if mode == MODE_CELT_ONLY {
            &pcm_transition_celt
        } else {
            &pcm_transition_silk
        };
        if audiosize >= F5 {
            let n = (st.channels * F2_5) as usize;
            pcm[..n].copy_from_slice(&pcm_transition[..n]);
            /* out == in2 (pcm), in1 == pcm_transition+F2_5 */
            smooth_fade(
                &mut pcm[n..],
                &pcm_transition[n..],
                false,
                F2_5,
                st.channels,
                &window,
                st.Fs,
            );
        } else {
            /* Not enough time to do a clean transition, but we do it anyway
               This will not preserve amplitude perfectly and may introduce
               a bit of temporal aliasing, but it shouldn't be too bad and
               that's pretty much the best we can do. In any case, generating this
               transition it pretty silly in the first place */
            smooth_fade(
                &mut pcm[..],
                pcm_transition,
                false,
                F2_5,
                st.channels,
                &window,
                st.Fs,
            );
        }
    }

    if st.decode_gain != 0 {
        let gain = celt_exp2(MULT16_16_P15(
            QCONST16(6.48814081e-4f32, 25),
            st.decode_gain as opus_val16,
        ));
        for i in 0..(frame_size * st.channels) as usize {
            let x = MULT16_32_P16(pcm[i], gain);
            pcm[i] = SATURATE(x, 32767.0);
        }
    }

    if len <= 1 {
        st.rangeFinal = 0;
    } else {
        st.rangeFinal ^= redundant_rng;
    }

    st.prev_mode = mode;
    st.prev_redundancy = (redundancy != 0 && celt_to_silk == 0) as i32;

    if celt_ret < 0 {
        celt_ret
    } else {
        audiosize
    }
}

/* ------------------------------------------------------------------------- *
 * opus_decode_native() (src/opus_decoder.c:716)
 * ------------------------------------------------------------------------- */

/// `opus_decode_native()` (`src/opus_decoder.c:716`).
///
/// `self_delimited`, `packet_offset`, `soft_clip` and `dred`/`dred_offset`
/// mirror the C arguments; the ported entry points only ever pass
/// `self_delimited == 0`, `packet_offset == None`, `dred == None`.
#[allow(clippy::too_many_arguments)]
fn opus_decode_native(
    st: &mut OpusDecoder,
    data: Option<&[u8]>,
    len: i32,
    pcm: &mut [opus_res],
    frame_size: i32,
    decode_fec: i32,
    self_delimited: i32,
    packet_offset: Option<&mut i32>,
    soft_clip: i32,
    dred: Option<&OpusDRED>,
    dred_offset: i32,
) -> i32 {
    let mut nb_samples: i32;
    let count: i32;
    let packet_frame_size: i32;
    let packet_bandwidth: i32;
    let packet_mode: i32;
    let packet_stream_channels: i32;
    /* 48 x 2.5 ms = 120 ms */
    let size: [opus_int16; 48];

    if decode_fec < 0 || decode_fec > 1 {
        return ffi::OPUS_BAD_ARG;
    }
    /* For FEC/PLC, frame_size has to be a multiple of 2.5 ms */
    if (decode_fec != 0 || len == 0 || data.is_none()) && frame_size % (st.Fs / 400) != 0 {
        return ffi::OPUS_BAD_ARG;
    }
    /* `#ifdef ENABLE_DRED`.  Only reachable through the (unported)
       `opus_decoder_dred_decode*()` entry points. */
    if let Some(d) = dred {
        if d.process_stage == 2 {
            lpcnet_plc_fec_clear(&mut st.lpcnet);
            let F10 = st.Fs / 100;
            /* if blend==0, the last PLC call was "update" and we need to feed
               two extra 10-ms frames. */
            let init_frames = if st.lpcnet.blend == 0 { 2 } else { 0 };
            let features_per_frame = IMAX(1, frame_size / F10);
            let needed_feature_frames = init_frames + features_per_frame;
            for i in 0..needed_feature_frames {
                /* We floor instead of rounding because 5-ms overlap
                   compensates for the missing 0.5 rounding offset. */
                let tmp = (dred_offset as f32) + (((d.dred_offset * F10) / 4) as f32);
                let feature_offset = init_frames - i - 2 + (tmp / F10 as f32).floor() as i32;
                if feature_offset <= 4 * d.nb_latents - 1 && feature_offset >= 0 {
                    let base = (feature_offset * DRED_NUM_FEATURES) as usize;
                    let end = (base + DRED_NUM_FEATURES as usize).min(d.fec_features.len());
                    lpcnet_plc_fec_add(&mut st.lpcnet, Some(&d.fec_features[base..end]));
                } else if feature_offset >= 0 {
                    lpcnet_plc_fec_add(&mut st.lpcnet, None);
                }
            }
        }
    }
    if len == 0 || data.is_none() {
        let mut pcm_count = 0i32;
        loop {
            let mut ret = opus_decode_frame(
                st,
                None,
                0,
                &mut pcm[pcm_count as usize * st.channels as usize..],
                frame_size - pcm_count,
                0,
            );
            if ret < 0 {
                return ret;
            }
            pcm_count += ret;
            if pcm_count >= frame_size {
                break;
            }
        }
        celt_assert!(pcm_count == frame_size);
        st.last_packet_duration = pcm_count;
        return pcm_count;
    } else if len < 0 {
        return ffi::OPUS_BAD_ARG;
    }

    let data = data.unwrap();
    packet_mode = opus_packet_get_mode(data);
    packet_bandwidth = opus_packet_get_bandwidth(data);
    packet_frame_size = opus_packet_get_samples_per_frame(data, st.Fs);
    packet_stream_channels = opus_packet_get_nb_channels(data);

    let parsed = match opus_packet_parse_impl(data, len, self_delimited != 0) {
        Ok(p) => p,
        Err(e) => return e,
    };
    if let Some(po) = packet_offset {
        *po = parsed.packet_offset;
    }
    let ignore_extensions = st.ignore_extensions != 0;
    let frames = parsed.frames;
    size = parsed.size;
    count = parsed.count;
    let padding: &[u8] = if ignore_extensions { &[] } else { parsed.padding };
    if count < 0 {
        return count;
    }
    /* `opus_extension_iterator_init(&iter, padding, padding_len, count)`.  The
       iterator is only consumed by the `#ifdef ENABLE_QEXT` block of the loop
       below, which this configuration does not compile, so nothing reads it. */
    let _iter = OpusExtensionIterator::new(padding, padding.len() as i32, count);

    if decode_fec != 0 {
        let duration_copy: i32;
        let mut ret: i32;
        /* If no FEC can be present, run the PLC (recursive call) */
        if frame_size < packet_frame_size
            || packet_mode == MODE_CELT_ONLY
            || st.mode == MODE_CELT_ONLY
        {
            return opus_decode_native(st, None, 0, pcm, frame_size, 0, 0, None, soft_clip, None, 0);
        }
        /* Otherwise, run the PLC on everything except the size for which we might have FEC */
        duration_copy = st.last_packet_duration;
        if frame_size - packet_frame_size != 0 {
            ret = opus_decode_native(
                st,
                None,
                0,
                pcm,
                frame_size - packet_frame_size,
                0,
                0,
                None,
                soft_clip,
                None,
                0,
            );
            if ret < 0 {
                st.last_packet_duration = duration_copy;
                return ret;
            }
            celt_assert!(ret == frame_size - packet_frame_size);
        }
        /* Complete with FEC */
        st.mode = packet_mode;
        st.bandwidth = packet_bandwidth;
        st.frame_size = packet_frame_size;
        st.stream_channels = packet_stream_channels;
        let off = (st.channels * (frame_size - packet_frame_size)) as usize;
        ret = opus_decode_frame(
            st,
            Some(frames[0]),
            size[0] as i32,
            &mut pcm[off..],
            packet_frame_size,
            1,
        );
        if ret < 0 {
            return ret;
        } else {
            st.last_packet_duration = frame_size;
            return frame_size;
        }
    }

    if count * packet_frame_size > frame_size {
        return ffi::OPUS_BUFFER_TOO_SMALL;
    }

    /* Update the state as the last step to avoid updating it on an invalid packet */
    st.mode = packet_mode;
    st.bandwidth = packet_bandwidth;
    st.frame_size = packet_frame_size;
    st.stream_channels = packet_stream_channels;

    nb_samples = 0;
    let mut i = 0;
    while i < count {
        /* `#ifdef ENABLE_QEXT`: the per-frame extension lookup is not
           compiled in this configuration. */
        let mut ret = opus_decode_frame(
            st,
            Some(frames[i as usize]),
            size[i as usize] as i32,
            &mut pcm[nb_samples as usize * st.channels as usize..],
            frame_size - nb_samples,
            0,
        );
        if ret < 0 {
            return ret;
        }
        celt_assert!(ret == packet_frame_size);
        nb_samples += ret;
        i += 1;
    }
    st.last_packet_duration = nb_samples;
    if soft_clip != 0 {
        opus_pcm_soft_clip_impl(pcm, nb_samples, st.channels, &mut st.softclip_mem, st.arch);
    } else {
        st.softclip_mem[0] = 0.0;
        st.softclip_mem[1] = 0.0;
    }
    nb_samples
}

/* ------------------------------------------------------------------------- *
 * opus_pcm_soft_clip_impl() (src/opus.c:39)
 * ------------------------------------------------------------------------- */

/// `opus_pcm_soft_clip_impl()` (`src/opus.c:39`).
///
/// The body lives in [`crate::opus::packet::opus_pcm_soft_clip`], which is the
/// same function with `arch == 0`.  On x86 `opus_limit2_checkwithin1()` is not
/// runtime-dispatched (`OPUS_X86_PRESUME_SSE2`, and no x86 variant of it
/// exists), so the `arch` argument cannot change the result.
fn opus_pcm_soft_clip_impl(
    pcm: &mut [f32],
    frame_size: i32,
    channels: i32,
    declip_mem: &mut [f32],
    arch: i32,
) {
    let _ = arch;
    opus_pcm_soft_clip(pcm, frame_size, channels, declip_mem);
}

/* ------------------------------------------------------------------------- *
 * opus_dred_* stubs
 * ------------------------------------------------------------------------- */

/// `opus_decoder_dred_decode*()` / `opus_dred_*` are the DRED (DNN) entry
/// points; they are **not ported** (`docs/progress.md`, "DNN/DRED scoping"):
/// they need the `RDOVAEDec` network and a `OpusDREDDecoder` that can only be
/// produced by `opus_dred_decoder_create()`, which is likewise not ported.
/// Keeping them out means [`opus_decode_native`]'s `dred != None` branch is
/// unreachable from any ported entry point.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn align_matches_x86_64() {
        assert_eq!(align(0), 0);
        assert_eq!(align(1), 8);
        assert_eq!(align(7), 8);
        assert_eq!(align(8), 8);
        assert_eq!(align(9), 16);
        assert_eq!(align(68112), 68112);
        assert_eq!(align(112624), 112624);
    }

    /// The sizes must be the ones the reference reports:
    /// `align(sizeof(OpusDecoder)) + align(sizeof(silk_decoder)) +
    /// celt_decoder_get_size(channels)`, i.e. 191580 / 200348.
    #[test]
    fn get_size_matches_reference() {
        assert_eq!(OpusDecoder::get_size(0), 0);
        assert_eq!(OpusDecoder::get_size(3), 0);
        assert_eq!(OpusDecoder::get_size(1), 191580);
        assert_eq!(OpusDecoder::get_size(2), 200348);
    }

    #[test]
    fn create_and_init_validation() {
        let mut error = 0;
        assert!(OpusDecoder::create(44100, 2, &mut error).is_none());
        assert_eq!(error, ffi::OPUS_BAD_ARG);
        assert!(OpusDecoder::create(48000, 3, &mut error).is_none());
        assert_eq!(error, ffi::OPUS_BAD_ARG);

        let mut st = OpusDecoder::default();
        assert_eq!(st.init(96000, 1), ffi::OPUS_BAD_ARG);
        assert_eq!(st.init(48000, 0), ffi::OPUS_BAD_ARG);

        let mut error = 0;
        let st = OpusDecoder::create(48000, 2, &mut error).expect("48 kHz stereo");
        assert_eq!(error, ffi::OPUS_OK);
        assert_eq!(st.Fs, 48000);
        assert_eq!(st.channels, 2);
        assert_eq!(st.stream_channels, 2);
        assert_eq!(st.frame_size, 48000 / 400);
        assert_eq!(st.arch, 4);
        assert_eq!(st.complexity, 0);
        assert_eq!(st.DecControl.API_sampleRate, 48000);
        assert_eq!(st.DecControl.nChannelsAPI, 2);
    }

    #[test]
    fn ctl_getters_and_setters() {
        let mut error = 0;
        let mut st = OpusDecoder::create(16000, 1, &mut error).expect("16 kHz mono");

        let mut v = 0i32;
        assert_eq!(st.ctl(ffi::OPUS_GET_SAMPLE_RATE_REQUEST, &mut v), ffi::OPUS_OK);
        assert_eq!(v, 16000);
        assert_eq!(st.ctl(ffi::OPUS_GET_BANDWIDTH_REQUEST, &mut v), ffi::OPUS_OK);
        assert_eq!(v, 0);
        assert_eq!(st.ctl(ffi::OPUS_GET_COMPLEXITY_REQUEST, &mut v), ffi::OPUS_OK);
        assert_eq!(v, 0);
        assert_eq!(st.ctl(ffi::OPUS_GET_GAIN_REQUEST, &mut v), ffi::OPUS_OK);
        assert_eq!(v, 0);
        assert_eq!(
            st.ctl(ffi::OPUS_GET_LAST_PACKET_DURATION_REQUEST, &mut v),
            ffi::OPUS_OK
        );
        assert_eq!(v, 0);
        assert_eq!(
            st.ctl(ffi::OPUS_GET_IGNORE_EXTENSIONS_REQUEST, &mut v),
            ffi::OPUS_OK
        );
        assert_eq!(v, 0);
        assert_eq!(
            st.ctl(ffi::OPUS_GET_PHASE_INVERSION_DISABLED_REQUEST, &mut v),
            ffi::OPUS_OK
        );
        assert_eq!(v, 1); /* mono decoder: disable_inv initialised to 1 */
        assert_eq!(st.ctl(ffi::OPUS_GET_PITCH_REQUEST, &mut v), ffi::OPUS_OK);
        assert_eq!(v, 0);

        /* SET bounds */
        let mut c = 11i32;
        assert_eq!(st.ctl(ffi::OPUS_SET_COMPLEXITY_REQUEST, &mut c), ffi::OPUS_BAD_ARG);
        let mut c = 4i32;
        assert_eq!(st.ctl(ffi::OPUS_SET_COMPLEXITY_REQUEST, &mut c), ffi::OPUS_OK);
        assert_eq!(st.complexity, 4);
        let mut g = 32768i32;
        assert_eq!(st.ctl(ffi::OPUS_SET_GAIN_REQUEST, &mut g), ffi::OPUS_BAD_ARG);
        let mut g = -1234i32;
        assert_eq!(st.ctl(ffi::OPUS_SET_GAIN_REQUEST, &mut g), ffi::OPUS_OK);
        assert_eq!(st.decode_gain, -1234);
        let mut b = 2i32;
        assert_eq!(
            st.ctl(ffi::OPUS_SET_IGNORE_EXTENSIONS_REQUEST, &mut b),
            ffi::OPUS_BAD_ARG
        );
        let mut b = 1i32;
        assert_eq!(
            st.ctl(ffi::OPUS_SET_IGNORE_EXTENSIONS_REQUEST, &mut b),
            ffi::OPUS_OK
        );
        assert_eq!(st.ignore_extensions, 1);
        let mut o = 2i32;
        assert_eq!(st.ctl(ffi::OPUS_SET_OSCE_BWE_REQUEST, &mut o), ffi::OPUS_BAD_ARG);
        let mut o = 1i32;
        assert_eq!(st.ctl(ffi::OPUS_SET_OSCE_BWE_REQUEST, &mut o), ffi::OPUS_OK);
        assert_eq!(st.enable_osce_bwe, 1);
        let mut p = 1i32;
        assert_eq!(
            st.ctl(ffi::OPUS_SET_PHASE_INVERSION_DISABLED_REQUEST, &mut p),
            ffi::OPUS_OK
        );
        let mut p = 0i32;
        assert_eq!(
            st.ctl(ffi::OPUS_GET_PHASE_INVERSION_DISABLED_REQUEST, &mut p),
            ffi::OPUS_OK
        );
        assert_eq!(p, 1);

        /* Unknown request */
        let mut x = 0i32;
        assert_eq!(st.ctl(0x7FFF_0000, &mut x), ffi::OPUS_UNIMPLEMENTED);

        /* OPUS_RESET_STATE clears the reset region but keeps the settings. */
        st.mode = 1002;
        st.prev_mode = 1002;
        st.bandwidth = 1105;
        st.last_packet_duration = 960;
        st.rangeFinal = 0xDEAD_BEEF;
        st.softclip_mem = [1.5, -1.5];
        let mut r = 0i32;
        assert_eq!(st.ctl(ffi::OPUS_RESET_STATE, &mut r), ffi::OPUS_OK);
        assert_eq!(st.mode, 0);
        assert_eq!(st.prev_mode, 0);
        assert_eq!(st.bandwidth, 0);
        assert_eq!(st.last_packet_duration, 0);
        assert_eq!(st.rangeFinal, 0);
        assert_eq!(st.softclip_mem, [0.0, 0.0]);
        assert_eq!(st.stream_channels, st.channels);
        assert_eq!(st.frame_size, st.Fs / 400);
        assert_eq!(st.decode_gain, -1234);
        assert_eq!(st.complexity, 4);
    }

    #[test]
    fn decode_rejects_bad_frame_size() {
        let mut error = 0;
        let mut st = OpusDecoder::create(48000, 1, &mut error).expect("48 kHz mono");
        let mut pcm = vec![0i16; 5760];
        assert_eq!(st.decode(None, 0, &mut pcm, 0, 0), ffi::OPUS_BAD_ARG);
        assert_eq!(st.decode(None, 0, &mut pcm, -3, 0), ffi::OPUS_BAD_ARG);
        let mut fpcm = vec![0f32; 5760];
        assert_eq!(st.decode_float(None, 0, &mut fpcm, 0, 0), ffi::OPUS_BAD_ARG);
        assert_eq!(st.decode24(None, 0, &mut vec![0i32; 5760], 0, 0), ffi::OPUS_BAD_ARG);
        /* FEC with a frame size that is not a multiple of 2.5 ms */
        assert_eq!(st.decode(None, 0, &mut pcm, 481, 1), ffi::OPUS_BAD_ARG);
    }

    #[test]
    fn get_nb_samples_matches_packet_helper() {
        let mut error = 0;
        let st = OpusDecoder::create(48000, 1, &mut error).expect("48 kHz mono");
        /* TOC 0x00: SILK narrowband 10 ms -> 480 samples. */
        let packet = [0x00u8, 1, 2, 3];
        assert_eq!(st.get_nb_samples(&packet, 4), 480);
        assert_eq!(
            st.get_nb_samples(&packet, 4),
            opus_packet_get_nb_samples(&packet, 4, 48000)
        );
        /* An empty packet is invalid. */
        assert_eq!(st.get_nb_samples(&[], 0), ffi::OPUS_BAD_ARG);
    }

    /// End-to-end check through the `audiopus`-compatible layer that
    /// `src/coder/decoder.rs` is written against: the same CTL paths, the
    /// `RefCell` borrow pattern and the `Packet`/`MutSignals` newtypes.
    #[test]
    fn public_coder_layer_uses_the_port() {
        use crate::coder::{Decoder as CoderDecoder, GenericCtl};
        use crate::packet::Packet;
        use crate::{Channels, MutSignals, SampleRate};
        use std::convert::TryFrom;

        let mut dec = CoderDecoder::new(SampleRate::Hz48000, Channels::Mono).expect("decoder");
        assert_eq!(dec.size(), 191580);
        assert_eq!(dec.sample_rate().unwrap(), SampleRate::Hz48000);

        /* `nb_samples()` is OPUS_GET_SAMPLE_RATE + the packet helper. */
        let packet = [0x00u8, 1, 2, 3];
        assert_eq!(
            dec.nb_samples(Packet::try_from(&packet[..]).unwrap()).unwrap(),
            480
        );

        /* Packet loss before any packet: zeros, and the duration is recorded. */
        let mut pcm = vec![7i16; 960];
        assert_eq!(
            dec.decode(None, MutSignals::try_from(&mut pcm).unwrap(), false)
                .unwrap(),
            960
        );
        assert!(pcm.iter().all(|&v| v == 0));
        assert_eq!(dec.last_packet_duration().unwrap(), 960);

        /* Gain / pitch / phase inversion round-trip through the CTL layer. */
        assert_eq!(dec.gain().unwrap(), 0);
        dec.set_gain(-1234).unwrap();
        assert_eq!(dec.gain().unwrap(), -1234);
        assert_eq!(dec.pitch().unwrap(), 0);
        assert!(dec.phase_inversion_disabled().unwrap()); /* mono default */
        dec.set_phase_inversion_disabled(false).unwrap();
        assert!(!dec.phase_inversion_disabled().unwrap());
        let _ = dec.final_range().unwrap();

        /* `decode_float()` on a lost packet is the f32 counterpart. */
        let mut fpcm = vec![1f32; 960];
        assert_eq!(
            dec.decode_float(None, MutSignals::try_from(&mut fpcm).unwrap(), true)
                .unwrap(),
            960
        );
        assert!(fpcm.iter().all(|&v| v == 0.0));

        /* The settings survive a reset, the stream state does not. */
        dec.reset_state().unwrap();
        assert_eq!(dec.gain().unwrap(), -1234);
    }

    #[test]
    fn decode_before_any_packet_returns_zeros_matching_plc() {
        /* `mode == 0` PLC: the C returns `audiosize` zero samples. */
        let mut error = 0;
        let mut st = OpusDecoder::create(48000, 2, &mut error).expect("48 kHz stereo");
        let mut pcm = vec![7i16; 960 * 2];
        let ret = st.decode(None, 0, &mut pcm, 960, 0);
        assert_eq!(ret, 960);
        assert!(pcm[..960 * 2].iter().all(|&v| v == 0));
        assert_eq!(st.last_packet_duration, 960);
    }

    #[test]
    fn decode_user_facing_doc_mentions_osce_modes() {
        /* The extended-mode values are the C `dnn/osce.h` ones. */
        assert_eq!(OSCE_MODE_SILK_ONLY, 1000);
        assert_eq!(OSCE_MODE_HYBRID, 1001);
        assert_eq!(OSCE_MODE_CELT_ONLY, 1002);
        assert_eq!(OSCE_MODE_SILK_BBWE, 1003);
    }
}


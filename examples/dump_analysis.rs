//! Differential dump of the CELT tonality-analysis layer for acceptance
//! configuration 0 (48 kHz / stereo / AUDIO / 96 kbps / complexity 10 / 20 ms /
//! VBR) — the one oracle row whose `enc_fnv` still differs.
//!
//! Mirrors the analysis part of `opus_encode_native()` exactly:
//!
//! ```text
//! analysis_read_pos_bak      = st.analysis.read_pos;
//! analysis_read_subframe_bak = st.analysis.read_subframe;
//! run_analysis(&mut st.analysis, &st.celt_enc.mode, Some(AnalysisPcm::Float(pcm)),
//!              analysis_size=960, frame_size=960, c1=0, c2=-2, C=2, Fs=48000,
//!              lsb_depth=16, &mut info);
//! st.analysis.read_pos      = analysis_read_pos_bak;
//! st.analysis.read_subframe = analysis_read_subframe_bak;
//! tonality_get_info(&mut st.analysis, &mut info, enc_frame_size=960);
//! ```
//!
//! The matching C dumper is `compat/c/probe_analysis_dump.c`; the two outputs
//! must be byte-identical.  Floats are printed as their IEEE-754 bit patterns
//! so `diff` reports the first differing field at the first differing frame
//! without any printf float-formatting noise.
//!
//!   cargo run --release --example dump_analysis > compat/out/analysis_rs.txt

use oprs::celt::celt::AnalysisInfo;
use oprs::ffi::OPUS_APPLICATION_AUDIO;
use oprs::opus::encoder::analysis::{run_analysis, tonality_get_info, AnalysisPcm};
use oprs::opus::encoder::OpusEncoder;

const FRAMES: usize = 50;
const FS: i32 = 48000;
const CHANNELS: i32 = 2;
const FRAME_SAMPLES: i32 = 960; /* 20 ms at 48 kHz */
const FRAME_LEN: usize = FRAME_SAMPLES as usize * CHANNELS as usize;

/* ---------------------------------------------------------------- generator */

struct Rng(u32);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1103515245).wrapping_add(12345);
        self.0
    }
    fn sample(&mut self) -> f32 {
        let v = ((self.next() >> 8) as i32) - 8388608;
        v as f32 / 8388608.0
    }
}

fn envelope(i: usize) -> f32 {
    0.25 + 0.75 * (i % 9973) as f32 / 9973.0
}

/* -------------------------------------------------------------------- dump */

fn print_f32(frame: usize, name: &str, v: f32) {
    println!("frame {:2} {:<20} 0x{:08x}", frame, name, v.to_bits());
}

fn print_i32(frame: usize, name: &str, v: i32) {
    println!("frame {:2} {:<20} {}", frame, name, v);
}

fn main() {
    let mut err = 0;
    let mut enc = OpusEncoder::create(FS, CHANNELS, OPUS_APPLICATION_AUDIO, &mut err)
        .expect("opus_encoder_create failed");

    /* Same controls the oracle applies, so `st.lsb_depth` (=16) and the
       complexity gate match. */
    enc.ctl(oprs::ffi::OPUS_SET_BITRATE_REQUEST, &mut 96000);
    enc.ctl(oprs::ffi::OPUS_SET_COMPLEXITY_REQUEST, &mut 10);
    enc.ctl(oprs::ffi::OPUS_SET_VBR_REQUEST, &mut 1);
    enc.ctl(oprs::ffi::OPUS_SET_VBR_CONSTRAINT_REQUEST, &mut 0);
    enc.ctl(oprs::ffi::OPUS_SET_DTX_REQUEST, &mut 0);
    enc.ctl(oprs::ffi::OPUS_SET_INBAND_FEC_REQUEST, &mut 0);
    enc.ctl(oprs::ffi::OPUS_SET_PACKET_LOSS_PERC_REQUEST, &mut 0);
    enc.ctl(oprs::ffi::OPUS_SET_BANDWIDTH_REQUEST, &mut -1000);
    enc.ctl(oprs::ffi::OPUS_SET_SIGNAL_REQUEST, &mut -1000);
    enc.ctl(oprs::ffi::OPUS_SET_LSB_DEPTH_REQUEST, &mut 16);

    let mut rng = Rng(0x1234567u32);
    let mut pcm = vec![0.0f32; FRAME_LEN];
    let mut info = AnalysisInfo::default();

    for f in 0..FRAMES {
        for i in 0..FRAME_LEN {
            let idx = f * FRAME_LEN + i;
            pcm[i] = rng.sample() * envelope(idx);
        }

        let bak_read_pos = enc.analysis.read_pos;
        let bak_read_subframe = enc.analysis.read_subframe;

        run_analysis(
            &mut enc.analysis,
            &enc.celt_enc.mode,
            Some(AnalysisPcm::Float(&pcm)),
            FRAME_SAMPLES, /* analysis_size */
            FRAME_SAMPLES, /* frame_size */
            0,             /* c1 */
            -2,            /* c2 */
            CHANNELS,      /* C */
            FS,
            16, /* lsb_depth = IMIN(MAX_ENCODING_DEPTH, st.lsb_depth) */
            &mut info,
        );

        /* opus_encode_native() rewinds so that the per-frame loop can call
           tonality_get_info() for the actual encoding decision. */
        enc.analysis.read_pos = bak_read_pos;
        enc.analysis.read_subframe = bak_read_subframe;
        tonality_get_info(&mut enc.analysis, &mut info, FRAME_SAMPLES);

        print_i32(f, "valid", info.valid);
        print_i32(f, "bandwidth", info.bandwidth);
        print_f32(f, "activity_probability", info.activity_probability);
        print_f32(f, "tonality", info.tonality);
        print_f32(f, "tonality_slope", info.tonality_slope);
        print_f32(f, "noisiness", info.noisiness);
        print_f32(f, "activity", info.activity);
        print_f32(f, "max_pitch_ratio", info.max_pitch_ratio);
        print_f32(f, "music_prob", info.music_prob);
        print_f32(f, "music_prob_min", info.music_prob_min);
        print_f32(f, "music_prob_max", info.music_prob_max);
    }
}

//! Independent check of the Rust multistream **encoder**: reproduces the five
//! configurations of `compat/tools/ms_oracle.c` (exactly the same PCM generator
//! and ctls) and prints the per-configuration FNV-1a of the produced packets,
//! which must equal the C `enc_fnv` values in `compat/out/ms_ref2.txt`.
//!
//!   cargo build --release --example ms_enc_check
//!   ./target/release/examples/ms_enc_check
//!   # cfg N enc_fnv=... must equal the enc_fnv values in compat/out/ms_ref2.txt

use oprs::ffi;
use oprs::opus::multistream::OpusMSEncoder;
use oprs::opus::multistream_encoder::{
    opus_multistream_encode_float, opus_multistream_encoder_create,
    opus_multistream_encoder_ctl, opus_multistream_surround_encoder_create,
};

const FRAMES: usize = 30;
const FRAME: usize = 960;
const MAX_PACKET: usize = 1500;

struct Fnv(u64);

impl Fnv {
    fn new() -> Fnv {
        Fnv(1469598103934665603)
    }
    fn upd(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(1099511628211);
        }
    }
}

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

struct Cfg {
    name: &'static str,
    channels: i32,
    family: i32,
    streams: i32,
    coupled: i32,
    application: i32,
    bitrate: i32,
    vbr: i32,
    dtx: i32,
    fec: i32,
    loss: i32,
}

const CFGS: &[Cfg] = &[
    Cfg { name: "6ch/5.1/surround/voip/64k", channels: 6, family: 1, streams: 0, coupled: 0, application: ffi::OPUS_APPLICATION_VOIP, bitrate: 64000, vbr: 1, dtx: 0, fec: 0, loss: 0 },
    Cfg { name: "8ch/7.1/surround/audio/96k", channels: 8, family: 1, streams: 0, coupled: 0, application: ffi::OPUS_APPLICATION_AUDIO, bitrate: 96000, vbr: 1, dtx: 0, fec: 0, loss: 0 },
    Cfg { name: "4ch/explicit/4streams+0", channels: 4, family: 0, streams: 4, coupled: 0, application: ffi::OPUS_APPLICATION_AUDIO, bitrate: 64000, vbr: 1, dtx: 0, fec: 0, loss: 0 },
    Cfg { name: "3ch/explicit/2streams+1coup", channels: 3, family: 0, streams: 2, coupled: 1, application: ffi::OPUS_APPLICATION_VOIP, bitrate: 48000, vbr: 1, dtx: 0, fec: 0, loss: 0 },
    Cfg { name: "5ch/5.0/surround/audio/80k", channels: 5, family: 1, streams: 0, coupled: 0, application: ffi::OPUS_APPLICATION_AUDIO, bitrate: 80000, vbr: 1, dtx: 0, fec: 0, loss: 0 },
    Cfg { name: "6ch/5.1/cbr/audio/64k", channels: 6, family: 1, streams: 0, coupled: 0, application: ffi::OPUS_APPLICATION_AUDIO, bitrate: 64000, vbr: 0, dtx: 0, fec: 0, loss: 0 },
    Cfg { name: "6ch/5.1/dtx/voip/32k", channels: 6, family: 1, streams: 0, coupled: 0, application: ffi::OPUS_APPLICATION_VOIP, bitrate: 32000, vbr: 1, dtx: 1, fec: 0, loss: 0 },
    Cfg { name: "8ch/7.1/fec/loss20/64k", channels: 8, family: 1, streams: 0, coupled: 0, application: ffi::OPUS_APPLICATION_VOIP, bitrate: 64000, vbr: 1, dtx: 0, fec: 1, loss: 20 },
];

fn main() {
    let mut total = Fnv::new();
    for (c, cf) in CFGS.iter().enumerate() {
        let mut mapping = [0u8, 1, 2, 3, 4, 5, 6, 7];
        let mut streams = cf.streams;
        let mut coupled = cf.coupled;
        let mut err = 0;
        let mut enc: Box<OpusMSEncoder> = if cf.family != 0 {
            opus_multistream_surround_encoder_create(
                48000,
                cf.channels,
                cf.family,
                &mut streams,
                &mut coupled,
                &mut mapping,
                cf.application,
                &mut err,
            )
        } else {
            opus_multistream_encoder_create(
                48000,
                cf.channels,
                streams,
                coupled,
                &mapping,
                cf.application,
                &mut err,
            )
        }
        .unwrap_or_else(|| panic!("cfg {c}: create failed ({err})"));

        let mut v;
        v = cf.bitrate;
        assert_eq!(opus_multistream_encoder_ctl(&mut enc, ffi::OPUS_SET_BITRATE_REQUEST, &mut v), 0);
        v = 10;
        assert_eq!(opus_multistream_encoder_ctl(&mut enc, ffi::OPUS_SET_COMPLEXITY_REQUEST, &mut v), 0);
        v = 1;
        assert_eq!(opus_multistream_encoder_ctl(&mut enc, ffi::OPUS_SET_VBR_REQUEST, &mut v), 0);
        v = ffi::OPUS_AUTO;
        assert_eq!(opus_multistream_encoder_ctl(&mut enc, ffi::OPUS_SET_SIGNAL_REQUEST, &mut v), 0);
        v = 16;
        assert_eq!(opus_multistream_encoder_ctl(&mut enc, ffi::OPUS_SET_LSB_DEPTH_REQUEST, &mut v), 0);
        v = cf.vbr;
        assert_eq!(opus_multistream_encoder_ctl(&mut enc, ffi::OPUS_SET_VBR_REQUEST, &mut v), 0);
        v = cf.dtx;
        assert_eq!(opus_multistream_encoder_ctl(&mut enc, ffi::OPUS_SET_DTX_REQUEST, &mut v), 0);
        v = cf.fec;
        assert_eq!(opus_multistream_encoder_ctl(&mut enc, ffi::OPUS_SET_INBAND_FEC_REQUEST, &mut v), 0);
        v = cf.loss;
        assert_eq!(opus_multistream_encoder_ctl(&mut enc, ffi::OPUS_SET_PACKET_LOSS_PERC_REQUEST, &mut v), 0);

        let mut rng = Rng(0x1234567u32.wrapping_add(c as u32 * 7919));
        let ch = cf.channels as usize;
        let mut pcm = vec![0f32; FRAME * ch];
        let mut h = Fnv::new();
        let mut bytes = 0i64;
        for f in 0..FRAMES {
            for (i, s) in pcm.iter_mut().enumerate() {
                let idx = f * FRAME * ch + i;
                *s = rng.sample() * envelope(idx);
            }
            let mut out = vec![0u8; MAX_PACKET];
            let n = opus_multistream_encode_float(&mut enc, &pcm, FRAME as i32, &mut out, MAX_PACKET as i32);
            assert!(n > 0, "cfg {c}: encode failed at frame {f} ({n})");
            out.truncate(n as usize);
            h.upd(&(n as i32).to_le_bytes());
            h.upd(&out);
            bytes += n as i64;
        }
        println!("cfg {c} {}\n  enc_fnv=0x{:016x} enc_bytes={bytes}", cf.name, h.0);
        total.upd(&h.0.to_le_bytes());
    }
    println!("TOTAL enc=0x{:016x}", total.0);
}

//! Quality harness for the opt-in `fast` build.
//!
//! Bit-exactness is the contract of the default build.  `--features fast`
//! deliberately reorders floating-point operations (8-lane FMA instead of 4-lane
//! mul+add), so the acceptance criterion becomes a measurable quality bound:
//!
//! For each of the 16 oracle configurations this harness
//!
//!   1. encodes the deterministic oracle signal with libopus and with `oprs`
//!      (identical settings, identical loss/FEC pattern),
//!   2. decodes *our* packets with libopus and with `oprs`,
//!   3. decodes *libopus's* packets with libopus and with `oprs`,
//!   4. reports the SNR in dB of
//!        - `dec(our pkts)` : oprs decode vs libopus decode of **our** packets,
//!        - `dec(C pkts)`   : oprs decode vs libopus decode of **libopus's**
//!          packets,
//!        - `enc drift`     : libopus decode of our packets vs libopus decode
//!          of libopus's packets (informational: how far the encoder's
//!          rate-distortion point moved),
//!   5. reports the per-configuration packet-size drift against libopus, and
//!   6. fails if the minimum of the two decoder SNRs is below 90 dB, or if
//!      either direction of interop breaks (libopus cannot decode a packet we
//!      emitted, or we cannot decode a libopus packet).
//!
//! It links the system libopus (the same 1.6.1 the C oracle uses) through
//! `#[link(name = "opus")]`; there are no Rust dependencies.
//!
//! ```text
//! cargo run --release --example snr_harness
//! cargo run --release --features fast --example snr_harness
//! ```
//!
//! The default build prints `inf` SNR everywhere (bit-identical decode) and
//! identical packet sizes, which is the sanity check that the harness itself is
//! comparing the right streams.

use oprs::coder::{Decoder, Encoder};
use oprs::packet::Packet;
use oprs::{Application, Bandwidth, Bitrate, Channels, MutSignals, SampleRate, Signal};

const MAX_PACKET: usize = 1500;
/// Gate for the minimum decoder SNR (dB).
const MIN_SNR_DB: f64 = 90.0;

// ---------------------------------------------------------------------------
// libopus FFI (system 1.6.1)
// ---------------------------------------------------------------------------

#[repr(C)]
struct RawEncoder {
    _private: [u8; 0],
}

#[repr(C)]
struct RawDecoder {
    _private: [u8; 0],
}

// SAFETY: the declarations match `opus/opus.h` 1.6.1 for the subset used here.
#[link(name = "opus")]
unsafe extern "C" {
    fn opus_encoder_create(
        fs: i32,
        channels: i32,
        application: i32,
        error: *mut i32,
    ) -> *mut RawEncoder;
    fn opus_encoder_ctl(st: *mut RawEncoder, request: i32, ...) -> i32;
    fn opus_encode_float(
        st: *mut RawEncoder,
        pcm: *const f32,
        frame_size: i32,
        data: *mut u8,
        max_data_bytes: i32,
    ) -> i32;
    fn opus_encoder_destroy(st: *mut RawEncoder);
    fn opus_decoder_create(fs: i32, channels: i32, error: *mut i32) -> *mut RawDecoder;
    fn opus_decode_float(
        st: *mut RawDecoder,
        data: *const u8,
        len: i32,
        pcm: *mut f32,
        frame_size: i32,
        decode_fec: i32,
    ) -> i32;
    fn opus_decoder_destroy(st: *mut RawDecoder);
}

struct CEncoder {
    ptr: *mut RawEncoder,
}

impl CEncoder {
    fn new(cf: &Config) -> CEncoder {
        let mut err = 0i32;
        // SAFETY: plain create call; `err` is a valid out-parameter.
        let ptr = unsafe {
            opus_encoder_create(
                cf.rate as i32,
                cf.channels as i32,
                cf.application as i32,
                &mut err,
            )
        };
        assert!(!ptr.is_null() && err == 0, "opus_encoder_create failed: {err}");
        let enc = CEncoder { ptr };
        let loss = if cf.fec { 10 } else { 0 };
        // SAFETY: `ptr` is a live encoder; each request takes one int argument.
        unsafe {
            let ctl = |req: i32, val: i32| {
                let r = opus_encoder_ctl(enc.ptr, req, val);
                assert_eq!(r, 0, "opus_encoder_ctl({req}) failed: {r}");
            };
            ctl(oprs::ffi::OPUS_SET_BITRATE_REQUEST, cf.bitrate);
            ctl(oprs::ffi::OPUS_SET_COMPLEXITY_REQUEST, cf.complexity as i32);
            ctl(oprs::ffi::OPUS_SET_VBR_REQUEST, cf.vbr as i32);
            ctl(oprs::ffi::OPUS_SET_VBR_CONSTRAINT_REQUEST, cf.cvbr as i32);
            ctl(oprs::ffi::OPUS_SET_DTX_REQUEST, cf.dtx as i32);
            ctl(oprs::ffi::OPUS_SET_INBAND_FEC_REQUEST, cf.fec as i32);
            ctl(oprs::ffi::OPUS_SET_PACKET_LOSS_PERC_REQUEST, loss);
            ctl(oprs::ffi::OPUS_SET_BANDWIDTH_REQUEST, cf.bandwidth as i32);
            ctl(oprs::ffi::OPUS_SET_SIGNAL_REQUEST, oprs::ffi::OPUS_AUTO);
            ctl(oprs::ffi::OPUS_SET_LSB_DEPTH_REQUEST, 16);
        }
        enc
    }

    fn encode(&mut self, pcm: &[f32], frame_samples: i32, out: &mut [u8]) -> usize {
        // SAFETY: `pcm` holds `frame_samples * channels` floats and `out` holds
        // `MAX_PACKET` bytes; both outlive the call.
        let n = unsafe {
            opus_encode_float(
                self.ptr,
                pcm.as_ptr(),
                frame_samples,
                out.as_mut_ptr(),
                out.len() as i32,
            )
        };
        assert!(n >= 0, "opus_encode_float failed: {n}");
        n as usize
    }
}

impl Drop for CEncoder {
    fn drop(&mut self) {
        // SAFETY: `ptr` was created by `opus_encoder_create` and is destroyed once.
        unsafe { opus_encoder_destroy(self.ptr) };
    }
}

struct CDecoder {
    ptr: *mut RawDecoder,
}

impl CDecoder {
    fn new(cf: &Config) -> CDecoder {
        let mut err = 0i32;
        // SAFETY: plain create call; `err` is a valid out-parameter.
        let ptr = unsafe { opus_decoder_create(cf.rate as i32, cf.channels as i32, &mut err) };
        assert!(!ptr.is_null() && err == 0, "opus_decoder_create failed: {err}");
        CDecoder { ptr }
    }

    /// One decode call: `Some(packet)` or `None` for packet loss, with
    /// libopus's `decode_fec` flag.
    fn decode(&mut self, packet: Option<&[u8]>, pcm: &mut [f32], frame_samples: i32, fec: bool) -> usize {
        let (data, len) = match packet {
            Some(p) => (p.as_ptr(), p.len() as i32),
            None => (std::ptr::null(), 0),
        };
        // SAFETY: `pcm` has room for `frame_samples * channels` floats and the
        // packet slice outlives the call.
        let n = unsafe {
            opus_decode_float(
                self.ptr,
                data,
                len,
                pcm.as_mut_ptr(),
                frame_samples,
                fec as i32,
            )
        };
        assert!(n >= 0, "opus_decode_float failed: {n}");
        n as usize
    }
}

impl Drop for CDecoder {
    fn drop(&mut self) {
        // SAFETY: `ptr` was created by `opus_decoder_create` and is destroyed once.
        unsafe { opus_decoder_destroy(self.ptr) };
    }
}

// ---------------------------------------------------------------------------
// the 16 oracle configurations (same table as `reference/tools/oracle.c`)
// ---------------------------------------------------------------------------

struct Config {
    name: &'static str,
    rate: SampleRate,
    channels: Channels,
    application: Application,
    bitrate: i32,
    complexity: u8,
    vbr: bool,
    cvbr: bool,
    dtx: bool,
    fec: bool,
    bandwidth: Bandwidth,
    /// frame size in tenths of a millisecond (25 == 2.5 ms)
    frame_ms2: usize,
    frames: usize,
    drop_every: usize,
}

use Application::{Audio, LowDelay, Voip};
use Bandwidth::{Auto, Narrowband, Wideband};
use Channels::{Mono, Stereo};
use SampleRate::{Hz12000, Hz16000, Hz24000, Hz48000, Hz8000};

const CONFIGS: &[Config] = &[
    Config { name: "48k/stereo/audio/96k/c10/20ms", rate: Hz48000, channels: Stereo, application: Audio, bitrate: 96000, complexity: 10, vbr: true, cvbr: false, dtx: false, fec: false, bandwidth: Auto, frame_ms2: 200, frames: 50, drop_every: 0 },
    Config { name: "48k/stereo/voip/32k/c5/20ms", rate: Hz48000, channels: Stereo, application: Voip, bitrate: 32000, complexity: 5, vbr: true, cvbr: false, dtx: false, fec: false, bandwidth: Auto, frame_ms2: 200, frames: 50, drop_every: 0 },
    Config { name: "48k/mono/audio/64k/c10/10ms", rate: Hz48000, channels: Mono, application: Audio, bitrate: 64000, complexity: 10, vbr: true, cvbr: false, dtx: false, fec: false, bandwidth: Auto, frame_ms2: 100, frames: 100, drop_every: 0 },
    Config { name: "48k/stereo/audio/128k/c10/5ms", rate: Hz48000, channels: Stereo, application: Audio, bitrate: 128000, complexity: 10, vbr: true, cvbr: false, dtx: false, fec: false, bandwidth: Auto, frame_ms2: 50, frames: 100, drop_every: 0 },
    Config { name: "48k/stereo/lowdelay/64k/c10/2.5ms", rate: Hz48000, channels: Stereo, application: LowDelay, bitrate: 64000, complexity: 10, vbr: true, cvbr: false, dtx: false, fec: false, bandwidth: Auto, frame_ms2: 25, frames: 200, drop_every: 0 },
    Config { name: "24k/mono/voip/24k/c3/20ms", rate: Hz24000, channels: Mono, application: Voip, bitrate: 24000, complexity: 3, vbr: true, cvbr: false, dtx: false, fec: false, bandwidth: Auto, frame_ms2: 200, frames: 50, drop_every: 0 },
    Config { name: "16k/mono/voip/16k/c0/40ms", rate: Hz16000, channels: Mono, application: Voip, bitrate: 16000, complexity: 0, vbr: true, cvbr: false, dtx: false, fec: false, bandwidth: Auto, frame_ms2: 400, frames: 30, drop_every: 0 },
    Config { name: "8k/mono/voip/12k/c0/20ms", rate: Hz8000, channels: Mono, application: Voip, bitrate: 12000, complexity: 0, vbr: true, cvbr: false, dtx: false, fec: false, bandwidth: Auto, frame_ms2: 200, frames: 50, drop_every: 0 },
    Config { name: "48k/stereo/audio/24k/c10/60ms", rate: Hz48000, channels: Stereo, application: Audio, bitrate: 24000, complexity: 10, vbr: true, cvbr: false, dtx: false, fec: false, bandwidth: Auto, frame_ms2: 600, frames: 20, drop_every: 0 },
    Config { name: "48k/stereo/audio/96k/c10/20ms/cbr", rate: Hz48000, channels: Stereo, application: Audio, bitrate: 96000, complexity: 10, vbr: false, cvbr: false, dtx: false, fec: false, bandwidth: Auto, frame_ms2: 200, frames: 50, drop_every: 0 },
    Config { name: "48k/stereo/audio/96k/c10/20ms/cvbr", rate: Hz48000, channels: Stereo, application: Audio, bitrate: 96000, complexity: 10, vbr: true, cvbr: true, dtx: false, fec: false, bandwidth: Auto, frame_ms2: 200, frames: 50, drop_every: 0 },
    Config { name: "12k/mono/voip/20k/c5/20ms", rate: Hz12000, channels: Mono, application: Voip, bitrate: 20000, complexity: 5, vbr: true, cvbr: false, dtx: false, fec: false, bandwidth: Auto, frame_ms2: 200, frames: 50, drop_every: 0 },
    Config { name: "48k/stereo/voip/20k/c10/10ms/dtx", rate: Hz48000, channels: Stereo, application: Voip, bitrate: 20000, complexity: 10, vbr: true, cvbr: false, dtx: true, fec: false, bandwidth: Auto, frame_ms2: 100, frames: 100, drop_every: 0 },
    Config { name: "48k/stereo/audio/64k/c7/20ms/wb", rate: Hz48000, channels: Stereo, application: Audio, bitrate: 64000, complexity: 7, vbr: true, cvbr: false, dtx: false, fec: false, bandwidth: Wideband, frame_ms2: 200, frames: 50, drop_every: 0 },
    Config { name: "48k/stereo/audio/64k/c7/20ms/nb", rate: Hz48000, channels: Stereo, application: Audio, bitrate: 64000, complexity: 7, vbr: true, cvbr: false, dtx: false, fec: false, bandwidth: Narrowband, frame_ms2: 200, frames: 50, drop_every: 0 },
    Config { name: "48k/stereo/voip/32k/c5/20ms/fec/loss", rate: Hz48000, channels: Stereo, application: Voip, bitrate: 32000, complexity: 5, vbr: true, cvbr: false, dtx: false, fec: true, bandwidth: Auto, frame_ms2: 200, frames: 50, drop_every: 7 },
];

// ---------------------------------------------------------------------------
// deterministic signal (same generator as the C oracle)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// encode / decode with each backend
// ---------------------------------------------------------------------------

fn is_loss(cf: &Config, f: usize) -> bool {
    cf.drop_every != 0 && (f + 1) % cf.drop_every == 0
}

fn encode_ours(cf: &Config) -> Vec<Vec<u8>> {
    let mut enc = Encoder::new(cf.rate, cf.channels, cf.application).expect("Encoder::new");
    enc.set_bitrate(Bitrate::BitsPerSecond(cf.bitrate)).unwrap();
    enc.set_complexity(cf.complexity).unwrap();
    enc.set_vbr(cf.vbr).unwrap();
    enc.set_vbr_constraint(cf.cvbr).unwrap();
    enc.set_dtx(cf.dtx).unwrap();
    enc.set_inband_fec(cf.fec).unwrap();
    enc.set_packet_loss_perc(if cf.fec { 10 } else { 0 }).unwrap();
    enc.set_bandwidth(cf.bandwidth).unwrap();
    enc.set_signal(Signal::Auto).unwrap();
    enc.set_lsb_depth(16).unwrap();

    let frame_samples = cf.rate as usize * cf.frame_ms2 / 10000;
    let channels = cf.channels as usize;
    let frame_len = frame_samples * channels;
    let mut rng = Rng(0x1234567u32.wrapping_add(cf.index() as u32 * 7919));
    let mut pcm = vec![0.0f32; frame_len];
    let mut out = vec![0u8; MAX_PACKET];
    let mut packets = Vec::with_capacity(cf.frames);
    for f in 0..cf.frames {
        for i in 0..frame_len {
            pcm[i] = rng.sample() * envelope(f * frame_len + i);
        }
        let n = enc.encode_float(&pcm, &mut out).expect("encode_float");
        assert!(n > 0, "{}: empty packet at frame {f}", cf.name);
        packets.push(out[..n].to_vec());
    }
    packets
}

fn encode_libopus(cf: &Config) -> Vec<Vec<u8>> {
    let mut enc = CEncoder::new(cf);
    let frame_samples = cf.rate as usize * cf.frame_ms2 / 10000;
    let channels = cf.channels as usize;
    let frame_len = frame_samples * channels;
    let mut rng = Rng(0x1234567u32.wrapping_add(cf.index() as u32 * 7919));
    let mut pcm = vec![0.0f32; frame_len];
    let mut out = vec![0u8; MAX_PACKET];
    let mut packets = Vec::with_capacity(cf.frames);
    for f in 0..cf.frames {
        for i in 0..frame_len {
            pcm[i] = rng.sample() * envelope(f * frame_len + i);
        }
        let n = enc.encode(&pcm, frame_samples as i32, &mut out);
        assert!(n > 0, "{}: libopus empty packet at frame {f}", cf.name);
        packets.push(out[..n].to_vec());
    }
    packets
}

/// Decodes a packet sequence with this crate, applying the oracle's loss/FEC
/// pattern.  Returns the concatenated interleaved float PCM.
fn decode_ours(cf: &Config, packets: &[Vec<u8>]) -> Vec<f32> {
    let mut dec = Decoder::new(cf.rate, cf.channels).expect("Decoder::new");
    let frame_samples = cf.rate as usize * cf.frame_ms2 / 10000;
    let channels = cf.channels as usize;
    let frame_len = frame_samples * channels;
    let mut all = Vec::with_capacity(frame_len * packets.len());
    let mut out = vec![0.0f32; frame_len];
    for f in 0..packets.len() {
        let n = if is_loss(cf, f) {
            if cf.fec && f + 1 < packets.len() {
                let pkt = Packet::try_from(packets[f + 1].as_slice())
                    .expect("Packet::try_from(next)");
                dec.decode_float(
                    Some(pkt),
                    MutSignals::try_from(&mut out[..]).unwrap(),
                    true,
                )
                .expect("decode_float(fec)")
            } else {
                dec.decode_float(None, MutSignals::try_from(&mut out[..]).unwrap(), false)
                    .expect("decode_float(plc)")
            }
        } else {
            let pkt = Packet::try_from(packets[f].as_slice()).expect("Packet::try_from");
            dec.decode_float(Some(pkt), MutSignals::try_from(&mut out[..]).unwrap(), false)
                .expect("decode_float")
        };
        all.extend_from_slice(&out[..n * channels]);
    }
    all
}

/// Same sequence through the system libopus decoder.
fn decode_libopus(cf: &Config, packets: &[Vec<u8>]) -> Vec<f32> {
    let mut dec = CDecoder::new(cf);
    let frame_samples = cf.rate as usize * cf.frame_ms2 / 10000;
    let channels = cf.channels as usize;
    let frame_len = frame_samples * channels;
    let mut all = Vec::with_capacity(frame_len * packets.len());
    let mut out = vec![0.0f32; frame_len];
    for f in 0..packets.len() {
        let n = if is_loss(cf, f) {
            if cf.fec && f + 1 < packets.len() {
                dec.decode(
                    Some(packets[f + 1].as_slice()),
                    &mut out,
                    frame_samples as i32,
                    true,
                )
            } else {
                dec.decode(None, &mut out, frame_samples as i32, false)
            }
        } else {
            dec.decode(Some(packets[f].as_slice()), &mut out, frame_samples as i32, false)
        };
        all.extend_from_slice(&out[..n * channels]);
    }
    all
}

// ---------------------------------------------------------------------------
// SNR
// ---------------------------------------------------------------------------

/// `10*log10(Σref² / Σ(ref-test)²)`, `inf` for a bit-identical signal.
fn snr_db(reference: &[f32], test: &[f32]) -> Option<f64> {
    if reference.len() != test.len() {
        return None;
    }
    let mut sig = 0.0f64;
    let mut err = 0.0f64;
    for (&r, &t) in reference.iter().zip(test.iter()) {
        let r = r as f64;
        let t = t as f64;
        sig += r * r;
        let d = r - t;
        err += d * d;
    }
    if err == 0.0 {
        Some(f64::INFINITY)
    } else if sig == 0.0 {
        None
    } else {
        Some(10.0 * (sig / err).log10())
    }
}

fn fmt_db(v: f64) -> String {
    if v.is_infinite() {
        "inf".to_string()
    } else {
        format!("{v:.1}")
    }
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

impl Config {
    /// Index in the table, used to seed the deterministic RNG exactly as the C
    /// oracle does (`0x1234567 + c*7919`).
    fn index(&self) -> usize {
        CONFIGS.iter().position(|c| std::ptr::eq(c, self)).unwrap()
    }
}

fn main() {
    let fast = cfg!(feature = "fast");
    println!(
        "oprs quality harness — build: {}",
        if fast {
            "fast (AVX2+FMA-contracted kernels)"
        } else {
            "default (bit-exact bodies)"
        }
    );
    println!(
        "gate: minimum decoder SNR >= {MIN_SNR_DB:.1} dB; \
         interop both directions required"
    );
    println!();
    println!(
        "{:<38} {:>9} {:>9} {:>9} {:>12} {:>9}",
        "configuration", "dec(our)", "dec(C)", "enc drift", "bytes ours/C", "size Δ"
    );

    let mut min_snr = f64::INFINITY;
    let mut failures: Vec<String> = Vec::new();
    let mut total_ours = 0usize;
    let mut total_c = 0usize;
    let mut size_drift_frames = 0usize;

    for cf in CONFIGS {
        let our_packets = encode_ours(cf);
        let c_packets = encode_libopus(cf);
        assert_eq!(our_packets.len(), c_packets.len());

        // Interop in both directions, in both decoders, for every packet.
        let our_pcm_ours = decode_ours(cf, &our_packets);
        let our_pcm_c = decode_ours(cf, &c_packets);
        let c_pcm_ours = decode_libopus(cf, &our_packets);
        let c_pcm_c = decode_libopus(cf, &c_packets);

        // `decode_*` panics on a decode error, so reaching this point proves
        // both directions of interop.
        let dec_our_pkts = snr_db(&c_pcm_ours, &our_pcm_ours);
        let dec_c_pkts = snr_db(&c_pcm_c, &our_pcm_c);
        let enc_drift = snr_db(&c_pcm_c, &c_pcm_ours);
        let (dec_our_pkts, dec_c_pkts, enc_drift) = match (dec_our_pkts, dec_c_pkts, enc_drift) {
            (Some(a), Some(b), Some(c)) => (a, b, c),
            _ => {
                failures.push(format!(
                    "{}: decoded-sample counts differ between libopus and oprs",
                    cf.name
                ));
                continue;
            }
        };
        min_snr = min_snr.min(dec_our_pkts).min(dec_c_pkts);

        let bytes_ours: usize = our_packets.iter().map(|p| p.len()).sum();
        let bytes_c: usize = c_packets.iter().map(|p| p.len()).sum();
        total_ours += bytes_ours;
        total_c += bytes_c;
        let mut max_delta = 0i64;
        let mut differing = 0usize;
        for (a, b) in our_packets.iter().zip(c_packets.iter()) {
            if a.len() != b.len() {
                differing += 1;
                size_drift_frames += 1;
                max_delta = max_delta.max(a.len() as i64 - b.len() as i64);
            }
        }

        println!(
            "{:<38} {:>9} {:>9} {:>9} {:>7}/{:<5} {:>+9}",
            cf.name,
            fmt_db(dec_our_pkts),
            fmt_db(dec_c_pkts),
            fmt_db(enc_drift),
            bytes_ours,
            bytes_c,
            max_delta
        );
        if differing != 0 {
            // keep the information visible without widening the table
            let _ = differing;
        }
    }

    println!();
    println!(
        "packet bytes: oprs {total_ours} vs libopus {total_c} ({:+.3}%), \
         {size_drift_frames} frame(s) with a different size",
        if total_c == 0 {
            0.0
        } else {
            100.0 * (total_ours as f64 - total_c as f64) / total_c as f64
        }
    );
    println!("minimum decoder SNR: {} dB", fmt_db(min_snr));

    for f in &failures {
        println!("FAIL: {f}");
    }
    if min_snr < MIN_SNR_DB || !failures.is_empty() {
        println!(
            "QUALITY: FAIL (minimum {:.1} dB < gate {:.1} dB or interop/sample-count failure)",
            min_snr, MIN_SNR_DB
        );
        std::process::exit(1);
    }
    println!(
        "QUALITY: PASS (minimum decoder SNR {} dB >= {:.1} dB, interop both directions)",
        fmt_db(min_snr),
        MIN_SNR_DB
    );
}

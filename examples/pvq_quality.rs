//! Quality harness for the `fast` PVQ search specialisations.
//!
//! The PVQ search specialisations behind `--features fast` (`k == 1` argmax,
//! `n == 2` closed form, `n == 4` lane path, `n >= 32` / `k > 4` vectorised
//! greedy search) deliberately change *which* pulse vector the encoder picks.
//! Unlike the AVX2+FMA kernel variants, which only reorder float operations,
//! these variants move the encoder's rate-distortion point: our packets are no
//! longer byte-identical to libopus's.  The contract therefore becomes
//!
//!   * our packets must still be a **valid** PVQ bitstream (libopus decodes
//!     them, and our decode of them is identical to libopus's), so the decoder
//!     SNR of *our* packets stays at the float noise floor;
//!   * the quantisation must not get materially worse, measured both against
//!     libopus's rate-distortion point (informational) and against the
//!     **original signal** (the real quality measure);
//!   * the packet size must not drift (the pulse cost is fixed by `(N, K)`, so
//!     any size drift would indicate a changed allocation, not a changed
//!     search).
//!
//! For each of the 16 oracle configurations this harness reports
//!
//!   (a) `dec(our)`   : libopus decode of our packets vs our decode of our
//!                      packets — the decoder-consistency / interop SNR,
//!   (b) `vs C pkts`  : libopus decode of libopus's packets vs libopus decode
//!                      of our packets — how far the encoder's RD point moved,
//!   (c) `size Δ`     : packet-size drift in percent and the number of frames
//!                      whose size differs,
//!   (d)              : interop in both directions (a failed decode panics),
//!   (e) `sig SNR`    : decode(our packets) vs the **original** deterministic
//!                      signal, next to the same number for libopus's own
//!                      packets.  This is the measure that says whether the
//!                      changed search made the codec better or worse; (b)
//!                      alone cannot, because both sides are approximations.
//!
//! Gate (exit code 1 on failure):
//!
//!   * `dec(our)` >= 100 dB on **every** configuration (valid bitstream),
//!   * total packet-size drift within ±2 % on **every** configuration,
//!   * interop in both directions,
//!   * signal SNR not worse than libopus's by more than `MAX_SNR_LOSS_DB`.
//!
//! The last item is the one that makes this harness useful for judging an
//! algorithmic PVQ change: a search variant that "passes" the first three but
//! loses 3 dB of true quality against the original signal has not paid for
//! itself.  The default (bit-exact) build prints `inf` for (a)/(b), identical
//! sizes and identical signal SNR, which validates the harness itself.
//!
//! ```text
//! cargo run --release --example pvq_quality
//! cargo run --release --features fast --example pvq_quality
//! ```

use oprs::coder::{Decoder, Encoder};
use oprs::packet::Packet;
use oprs::{Application, Bandwidth, Bitrate, Channels, MutSignals, SampleRate, Signal};

const MAX_PACKET: usize = 1500;
/// Gate for the minimum decoder-consistency SNR (dB).
const MIN_SNR_DB: f64 = 100.0;
/// Gate for the total packet-size drift per configuration (percent).
const MAX_SIZE_DRIFT_PCT: f64 = 2.0;
/// Gate for the signal-SNR loss against libopus's own encoding (dB).
const MAX_SNR_LOSS_DB: f64 = 0.5;

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
        let ptr =
            unsafe { opus_encoder_create(cf.rate as i32, cf.channels as i32, cf.application as i32, &mut err) };
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
            opus_encode_float(self.ptr, pcm.as_ptr(), frame_samples, out.as_mut_ptr(), out.len() as i32)
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
        let n = unsafe { opus_decode_float(self.ptr, data, len, pcm.as_mut_ptr(), frame_samples, fec as i32) };
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
// the 16 oracle configurations (same table as `examples/snr_harness.rs`)
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
// deterministic signal (same generator as the C oracle and snr_harness)
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

/// The deterministic input signal, exactly as both encoders see it.
fn input_signal(cf: &Config) -> Vec<f32> {
    let frame_samples = cf.rate as usize * cf.frame_ms2 / 10000;
    let channels = cf.channels as usize;
    let frame_len = frame_samples * channels;
    let mut rng = Rng(0x1234567u32.wrapping_add(cf.index() as u32 * 7919));
    let mut all = Vec::with_capacity(frame_len * cf.frames);
    let mut pcm = vec![0.0f32; frame_len];
    for f in 0..cf.frames {
        for i in 0..frame_len {
            pcm[i] = rng.sample() * envelope(f * frame_len + i);
        }
        all.extend_from_slice(&pcm);
    }
    all
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
    for _f in 0..cf.frames {
        for i in 0..frame_len {
            pcm[i] = rng.sample() * envelope(_f * frame_len + i);
        }
        let n = enc.encode_float(&pcm, &mut out).expect("encode_float");
        assert!(n > 0, "{}: empty packet", cf.name);
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
        assert!(n > 0, "{}: libopus empty packet", cf.name);
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
                let pkt = Packet::try_from(packets[f + 1].as_slice()).expect("Packet::try_from(next)");
                dec.decode_float(Some(pkt), MutSignals::try_from(&mut out[..]).unwrap(), true)
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
                dec.decode(Some(packets[f + 1].as_slice()), &mut out, frame_samples as i32, true)
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
        "PVQ quality harness — build: {}",
        if fast { "fast (PVQ specialisations active)" } else { "default (bit-exact search)" }
    );
    println!(
        "gates: dec(our) >= {MIN_SNR_DB:.0} dB every config; |size drift| <= {MAX_SIZE_DRIFT_PCT:.1} %; \
         signal-SNR loss <= {MAX_SNR_LOSS_DB:.1} dB; interop both directions"
    );
    println!();
    println!(
        "{:<38} {:>8} {:>8} {:>9} {:>7} {:>8} {:>8} {:>7}",
        "configuration", "dec(our)", "vs Cpkt", "size Δ%", "Δframes", "sig our", "sig C", "Δsig"
    );

    let mut min_snr = f64::INFINITY;
    let mut max_abs_drift: f64 = 0.0;
    let mut worst_sig_loss = 0.0f64;
    let mut worst_sig_name = "";
    let mut failures: Vec<String> = Vec::new();
    let mut total_ours = 0usize;
    let mut total_c = 0usize;
    let mut size_drift_frames = 0usize;

    for cf in CONFIGS {
        let our_packets = encode_ours(cf);
        let c_packets = encode_libopus(cf);
        assert_eq!(our_packets.len(), c_packets.len());

        // Interop in both directions, in both decoders, for every packet.
        // `decode_*` panics on a decode error, so reaching the end proves it.
        let our_pcm_ours = decode_ours(cf, &our_packets);
        let our_pcm_c = decode_ours(cf, &c_packets);
        let c_pcm_ours = decode_libopus(cf, &our_packets);
        let c_pcm_c = decode_libopus(cf, &c_packets);

        let dec_our_pkts = snr_db(&c_pcm_ours, &our_pcm_ours);
        let dec_c_pkts = snr_db(&c_pcm_c, &our_pcm_c);
        let moved_rd = snr_db(&c_pcm_c, &c_pcm_ours);
        let (dec_our_pkts, dec_c_pkts, moved_rd) = match (dec_our_pkts, dec_c_pkts, moved_rd) {
            (Some(a), Some(b), Some(c)) => (a, b, c),
            _ => {
                failures.push(format!("{}: decoded-sample counts differ between libopus and oprs", cf.name));
                continue;
            }
        };

        // True quality: both packet streams are decoded by the *same* libopus
        // decoder, so the only variable is the encoder's quantisation.
        let original = input_signal(cf);
        let sig_ours = snr_db(&original, &c_pcm_ours);
        let sig_c = snr_db(&original, &c_pcm_c);

        let bytes_ours: usize = our_packets.iter().map(|p| p.len()).sum();
        let bytes_c: usize = c_packets.iter().map(|p| p.len()).sum();
        total_ours += bytes_ours;
        total_c += bytes_c;
        let drift = if bytes_c == 0 {
            0.0
        } else {
            100.0 * (bytes_ours as f64 - bytes_c as f64) / bytes_c as f64
        };
        max_abs_drift = max_abs_drift.max(drift.abs());
        let differing = our_packets.iter().zip(c_packets.iter()).filter(|(a, b)| a.len() != b.len()).count();
        size_drift_frames += differing;
        if differing != 0 {
            failures.push(format!("{}: {differing} frame(s) with a different packet size", cf.name));
        }

        min_snr = min_snr.min(dec_our_pkts).min(dec_c_pkts);

        let (sig_ours, sig_c) = match (sig_ours, sig_c) {
            (Some(a), Some(b)) => (a, b),
            _ => {
                failures.push(format!("{}: decoded length differs from the input signal", cf.name));
                continue;
            }
        };
        let d_sig = sig_ours - sig_c;
        if d_sig < worst_sig_loss {
            worst_sig_loss = d_sig;
            worst_sig_name = cf.name;
        }

        println!(
            "{:<38} {:>8} {:>8} {:>+9.3} {:>7} {:>8} {:>8} {:>+7.3}",
            cf.name,
            fmt_db(dec_our_pkts),
            fmt_db(moved_rd),
            drift,
            differing,
            fmt_db(sig_ours),
            fmt_db(sig_c),
            d_sig
        );

        if drift.abs() > MAX_SIZE_DRIFT_PCT {
            failures.push(format!("{}: size drift {drift:+.2} % exceeds ±{MAX_SIZE_DRIFT_PCT} %", cf.name));
        }
        if d_sig < -MAX_SNR_LOSS_DB {
            failures.push(format!("{}: signal SNR {d_sig:+.2} dB vs libopus is worse than -{MAX_SNR_LOSS_DB} dB", cf.name));
        }
        let _ = dec_c_pkts;
    }

    println!();
    println!(
        "packet bytes: oprs {total_ours} vs libopus {total_c} ({:+.3}%), \
         {size_drift_frames} frame(s) with a different size",
        if total_c == 0 { 0.0 } else { 100.0 * (total_ours as f64 - total_c as f64) / total_c as f64 }
    );
    println!("minimum decoder SNR: {} dB", fmt_db(min_snr));
    println!("maximum |size drift|: {max_abs_drift:.3} %");
    println!(
        "worst signal-SNR delta vs libopus: {:+.3} dB ({})",
        worst_sig_loss, worst_sig_name
    );

    for f in &failures {
        println!("FAIL: {f}");
    }
    if min_snr < MIN_SNR_DB || !failures.is_empty() {
        println!(
            "QUALITY: FAIL (min SNR {:.1} dB < {:.0} dB, or size/quality/interop failure)",
            min_snr, MIN_SNR_DB
        );
        std::process::exit(1);
    }
    println!(
        "QUALITY: PASS (min decoder SNR {} dB >= {:.0} dB, |size drift| {:.3} % <= {:.1} %, \
         signal-SNR loss {:.3} dB <= {:.1} dB, interop both directions)",
        fmt_db(min_snr),
        MIN_SNR_DB,
        max_abs_drift,
        MAX_SIZE_DRIFT_PCT,
        -worst_sig_loss,
        MAX_SNR_LOSS_DB
    );
}

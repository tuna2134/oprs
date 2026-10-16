//! Minimal oprs-only throughput probe, used to evaluate build-profile effects
//! (e.g. `lto`/`codegen-units`) independently of the full A/B benchmark in
//! `compat/bench`:
//!
//! ```text
//! cargo run --release --example bench_min
//! ```
//!
//! Prints encode and decode real-time factors (audio seconds processed per wall
//! second) for a few representative configurations.

use oprs::coder::{Decoder, Encoder};
use oprs::packet::Packet;
use oprs::{Application, Bitrate, Channels, MutSignals, SampleRate};
use std::time::Instant;

const FRAMES: usize = 2000;

fn tone(len: usize) -> Vec<f32> {
    (0..len)
        .map(|i| ((i as f32) * 0.01).sin() * 0.3 + ((i as f32) * 0.003).sin() * 0.2)
        .collect()
}

fn run(name: &str, rate: SampleRate, channels: Channels, frame: usize, app: Application, bitrate: i32) {
    let frame_len = frame * channels as usize;
    let pcm = tone(frame_len);

    let mut enc = Encoder::new(rate, channels, app).expect("encoder");
    enc.set_bitrate(Bitrate::BitsPerSecond(bitrate)).unwrap();
    enc.set_complexity(10).unwrap();
    enc.set_vbr(true).unwrap();

    let mut packets: Vec<Vec<u8>> = Vec::with_capacity(FRAMES);
    let mut out = vec![0u8; 1500];

    // warm-up
    for _ in 0..20 {
        let n = enc.encode_float(&pcm, &mut out).unwrap();
        assert!(n > 0);
    }

    let t0 = Instant::now();
    for _ in 0..FRAMES {
        let n = enc.encode_float(&pcm, &mut out).unwrap();
        packets.push(out[..n].to_vec());
    }
    let enc_t = t0.elapsed().as_secs_f64();

    let mut dec = Decoder::new(rate, channels).expect("decoder");
    let mut dec_out = vec![0i16; frame_len];
    for p in &packets[..20] {
        let pkt = Packet::try_from(p.as_slice()).unwrap();
        dec.decode(Some(pkt), MutSignals::try_from(&mut dec_out[..]).unwrap(), false)
            .unwrap();
    }

    let t1 = Instant::now();
    for p in &packets {
        let pkt = Packet::try_from(p.as_slice()).unwrap();
        let n = dec
            .decode(Some(pkt), MutSignals::try_from(&mut dec_out[..]).unwrap(), false)
            .unwrap();
        assert_eq!(n, frame);
    }
    let dec_t = t1.elapsed().as_secs_f64();

    let audio = (FRAMES * frame) as f64 / rate as usize as f64;
    println!(
        "{name:<28} enc: {:7.1}x realtime ({:8.2} ms/frame)   dec: {:7.1}x realtime ({:8.2} ms/frame)",
        audio / enc_t,
        enc_t * 1e3 / FRAMES as f64,
        audio / dec_t,
        dec_t * 1e3 / FRAMES as f64,
    );
}

fn main() {
    run("48k/stereo/audio/96k/20ms", SampleRate::Hz48000, Channels::Stereo, 960, Application::Audio, 96000);
    run("48k/mono/voip/32k/20ms", SampleRate::Hz48000, Channels::Mono, 960, Application::Voip, 32000);
    run("48k/stereo/ld/64k/2.5ms", SampleRate::Hz48000, Channels::Stereo, 120, Application::LowDelay, 64000);
    run("16k/mono/voip/16k/20ms", SampleRate::Hz16000, Channels::Mono, 320, Application::Voip, 16000);
    run("8k/mono/voip/12k/20ms", SampleRate::Hz8000, Channels::Mono, 160, Application::Voip, 12000);
}

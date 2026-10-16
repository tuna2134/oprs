//! Temporary SILK encode loop used only for gdb sampling. Delete after use.
//!
//! ```text
//! CARGO_PROFILE_RELEASE_DEBUG=1 cargo build --release --example prof_silk
//! PROF_SECS=60 PROF_RATE=8000 PROF_VOIP=1 cargo run --release --example prof_silk
//! ```

use oprs::coder::Encoder;
use oprs::{Application, Bitrate, Channels, SampleRate};
use std::time::Instant;

fn main() {
    let secs: u64 = std::env::var("PROF_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30);
    let rate = match std::env::var("PROF_RATE").ok().as_deref() {
        Some("8000") => SampleRate::Hz8000,
        Some("16000") => SampleRate::Hz16000,
        Some("12000") => SampleRate::Hz12000,
        _ => SampleRate::Hz48000,
    };
    let app = if std::env::var("PROF_VOIP").is_ok() {
        Application::Voip
    } else {
        Application::Audio
    };
    // 20 ms frame at the selected rate.
    let frame = (rate as i32 / 50) as usize;
    let bitrate = match rate {
        SampleRate::Hz8000 => 12000,
        SampleRate::Hz12000 => 16000,
        SampleRate::Hz16000 => 16000,
        _ => 32000,
    };
    let mut enc = Encoder::new(rate, Channels::Mono, app).unwrap();
    enc.set_bitrate(Bitrate::BitsPerSecond(bitrate)).unwrap();
    enc.set_complexity(10).unwrap();
    let pcm: Vec<f32> = (0..frame).map(|i| ((i as f32) * 0.01).sin() * 0.3).collect();
    let mut out = vec![0u8; 1500];

    let start = Instant::now();
    let mut frames = 0u64;
    while start.elapsed().as_secs() < secs {
        enc.encode_float(&pcm, &mut out).unwrap();
        frames += 1;
    }
    let el = start.elapsed();
    println!(
        "{frames} frames in {:?} ({:.1}x RT)",
        el,
        frames as f64 * 0.020 / el.as_secs_f64()
    );
}

//! The quick-start example from `README.md` — kept as a real example so it
//! cannot rot:
//!
//! ```text
//! cargo run --release --example quickstart
//! ```

use oprs::coder::{Decoder, Encoder};
use oprs::packet::Packet;
use oprs::{Application, Bandwidth, Bitrate, Channels, MutSignals, SampleRate, Signal};

fn main() -> oprs::Result<()> {
    let rate = SampleRate::Hz48000;
    let channels = Channels::Stereo; // Mono = 1, Stereo = 2
    let frame_samples = 960; // 20 ms @ 48 kHz
    let frame_len = frame_samples * channels as usize;

    // ---- エンコーダ ----
    let mut enc = Encoder::new(rate, channels, Application::Audio)?;
    enc.set_bitrate(Bitrate::BitsPerSecond(64000))?;
    enc.set_complexity(10)?;
    enc.set_vbr(true)?;
    enc.set_vbr_constraint(false)?; // CVBR
    enc.set_dtx(false)?;
    enc.set_inband_fec(true)?; // FEC（LBRR）
    enc.set_packet_loss_perc(10)?;
    enc.set_bandwidth(Bandwidth::Auto)?;
    enc.set_signal(Signal::Auto)?;
    enc.set_lsb_depth(16)?;

    // 入力はインターリーブ f32（-1.0 〜 1.0）。i16 なら `encode` を使う
    let pcm: Vec<f32> = (0..frame_len)
        .map(|i| (i as f32 * 0.01).sin() * 0.5)
        .collect();

    let mut packet = vec![0u8; 1500]; // 最大パケット長
    let n = enc.encode_float(&pcm, &mut packet)?;
    packet.truncate(n);
    println!("encoded {n} bytes");

    // ---- デコーダ ----
    let mut dec = Decoder::new(rate, channels)?;
    let mut out = vec![0i16; frame_len];
    let pkt = Packet::try_from(packet.as_slice())?;
    let samples = dec.decode(Some(pkt), MutSignals::try_from(&mut out[..])?, false)?;
    println!("decoded {samples} samples/channel");

    // f32 が欲しいとき:
    let mut out_f32 = vec![0f32; frame_len];
    let pkt = Packet::try_from(packet.as_slice())?;
    let samples_f = dec.decode_float(Some(pkt), MutSignals::try_from(&mut out_f32[..])?, false)?;
    println!("decoded {samples_f} samples/channel (float)");

    assert_eq!(samples, frame_samples);
    assert_eq!(samples_f, frame_samples);
    Ok(())
}

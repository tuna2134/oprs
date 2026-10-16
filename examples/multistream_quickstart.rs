//! The multistream example from `README.md` (5.1 surround encode → decode).
//!
//! ```text
//! cargo run --release --example multistream_quickstart
//! ```

use oprs::ffi;
use oprs::opus::multistream_decoder::OpusMSDecoder;
use oprs::opus::multistream_encoder::{
    opus_multistream_encode_float, opus_multistream_encoder_ctl,
    opus_multistream_surround_encoder_create,
};

fn main() {
    let channels = 6; // 5.1
    let frame = 960; // 20 ms @ 48 kHz
    let mut mapping = [0u8; 256]; // create がストリーム→チャネル対応を書き込む
    let mut streams = 0;
    let mut coupled = 0;
    let mut err = 0;

    let mut enc = opus_multistream_surround_encoder_create(
        48000,
        channels,
        1, // mapping_family = 1（surround）
        &mut streams,
        &mut coupled,
        &mut mapping,
        ffi::OPUS_APPLICATION_AUDIO,
        &mut err,
    )
    .expect("encoder create failed");

    let mut bitrate = 128000;
    assert_eq!(
        opus_multistream_encoder_ctl(&mut enc, ffi::OPUS_SET_BITRATE_REQUEST, &mut bitrate),
        0
    );

    // インターリーブ f32（ここでは簡単なトーン。無音だと数バイトのパケットになります）
    let pcm: Vec<f32> = (0..frame * channels as usize)
        .map(|i| (i as f32 * 0.02).sin() * 0.3)
        .collect();
    let mut packet = vec![0u8; 1500];
    let n = opus_multistream_encode_float(&mut enc, &pcm, frame as i32, &mut packet, 1500);
    assert!(n > 0);

    let mut dec = OpusMSDecoder::create(
        48000,
        channels,
        streams,
        coupled,
        &mapping[..channels as usize],
        &mut err,
    )
    .expect("decoder create failed");
    let mut out = vec![0i16; frame * channels as usize];
    let samples = dec.decode(Some(&packet[..n as usize]), n, &mut out, frame as i32, 0);
    assert!(samples > 0);

    println!(
        "5.1: streams={streams} coupled={coupled} mapping={:?} -> {n} bytes, {samples} samples/channel",
        &mapping[..channels as usize]
    );
}

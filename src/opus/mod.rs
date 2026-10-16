//! The Opus layer: packet framing, the encoder/decoder front ends and the
//! repacketizer.  Mirrors the `src/` directory of libopus 1.6.1.
//!
//! | C | Rust |
//! | --- | --- |
//! | `src/opus.c` | `src/opus/packet.rs` |
//! | `src/repacketizer.c` | `src/opus/repacketizer.rs` |
//! | `src/opus_decoder.c` | `src/opus/decoder.rs` |
//! | `src/opus_encoder.c` | `src/opus/encoder.rs` |
//! | `src/opus_multistream.c` | `src/opus/multistream.rs` |
//! | `src/opus_multistream_encoder.c` | `src/opus/multistream_encoder.rs` |
//! | `src/opus_multistream_decoder.c` | `src/opus/multistream_decoder.rs` |
//! | `src/opus_projection*.c` | `src/opus/projection.rs` |
#![allow(non_snake_case)]
#![allow(non_camel_case_types)]
#![allow(non_upper_case_globals)]
#![allow(unused_mut)]
#![allow(unused_assignments)]
#![allow(unused_parens)]

pub mod packet;
pub mod repacketizer;
pub mod decoder;
pub mod encoder;
pub mod multistream;
pub mod multistream_encoder;
pub mod multistream_decoder;
pub mod projection;

/// `opus_get_version_string()` (in C this lives in `src/opus.c`).
///
/// The port reproduces the behaviour of libopus 1.6.1 and therefore reports the
/// same version string.
pub fn opus_get_version_string() -> &'static str {
    "libopus 1.6.1"
}

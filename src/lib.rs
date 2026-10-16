//! `oprs` — a pure-Rust implementation of the Opus codec.
//!
//! The public API is source-compatible with the `audiopus` crate
//! (version 0.3.0-rc.0), so code written against `audiopus` can be recompiled
//! against this crate without changes.  Unlike `audiopus`, no C library is
//! linked: the codec itself is implemented in Rust.
//!
//! The port follows libopus 1.6.1 module by module; the reference
//! configuration it reproduces bit-for-bit is:
//!
//! * floating-point build (`FIXED_POINT` undefined)
//! * `FLOAT_APPROX` undefined
//! * x86 intrinsics + run-time CPU dispatch enabled (host arch level 4)
//! * `ENABLE_DEEP_PLC`, `ENABLE_DRED`, `ENABLE_OSCE`, `CUSTOM_MODES`,
//!   `ENABLE_HARDENING` defined; `ENABLE_OSCE_BWE` undefined

#![allow(clippy::missing_safety_doc)]

pub mod types;

pub mod celt;

pub mod silk;

pub mod opus;

pub mod error;
pub mod ffi;

pub mod coder;
pub mod packet;
pub mod repacketizer;
pub mod softclip;

pub use crate::error::{Error, ErrorCode, Result};

use std::convert::TryFrom;

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Signal {
    Auto = ffi::OPUS_AUTO,
    Voice = ffi::OPUS_SIGNAL_VOICE,
    Music = ffi::OPUS_SIGNAL_MUSIC,
}

impl TryFrom<i32> for Signal {
    type Error = Error;

    fn try_from(value: i32) -> Result<Self> {
        Ok(match value {
            ffi::OPUS_AUTO => Signal::Auto,
            ffi::OPUS_SIGNAL_VOICE => Signal::Voice,
            ffi::OPUS_SIGNAL_MUSIC => Signal::Music,
            _ => return Err(Error::InvalidSignal(value)),
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Bitrate {
    /// Explicit bitrate choice (in bits/second).
    BitsPerSecond(i32),
    /// Maximum bitrate allowed (up to maximum number of bytes for the packet).
    Max,
    /// Default bitrate decided by the encoder (not recommended).
    Auto,
}

impl From<Bitrate> for i32 {
    fn from(bitrate: Bitrate) -> i32 {
        match bitrate {
            Bitrate::Auto => ffi::OPUS_AUTO,
            Bitrate::Max => ffi::OPUS_BITRATE_MAX,
            Bitrate::BitsPerSecond(bits) => bits,
        }
    }
}

impl TryFrom<i32> for Bitrate {
    type Error = Error;

    fn try_from(value: i32) -> Result<Bitrate> {
        Ok(match value {
            ffi::OPUS_AUTO => Bitrate::Auto,
            ffi::OPUS_BITRATE_MAX => Bitrate::Max,
            x if x.is_positive() => Bitrate::BitsPerSecond(x),
            // `audiopus` reports `InvalidBandwidth` here; kept for compatibility.
            _ => return Err(Error::InvalidBandwidth(value)),
        })
    }
}

/// Represents possible sample rates Opus can use.
/// Values represent Hertz.
#[repr(i32)]
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub enum SampleRate {
    Hz8000 = 8000,
    Hz12000 = 12000,
    Hz16000 = 16000,
    Hz24000 = 24000,
    Hz48000 = 48000,
}

impl TryFrom<i32> for SampleRate {
    type Error = Error;

    /// Fails if a number does not map a documented Opus sample rate.
    fn try_from(value: i32) -> Result<Self> {
        Ok(match value {
            8000 => SampleRate::Hz8000,
            12000 => SampleRate::Hz12000,
            16000 => SampleRate::Hz16000,
            24000 => SampleRate::Hz24000,
            48000 => SampleRate::Hz48000,
            _ => return Err(Error::InvalidSampleRate(value)),
        })
    }
}

/// Represents possible application-types for Opus.
#[repr(i32)]
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub enum Application {
    /// Best for most VoIP/videoconference applications where listening quality
    /// and intelligibility matter most.
    Voip = ffi::OPUS_APPLICATION_VOIP,
    /// Best for broadcast/high-fidelity application where the decoded audio
    /// should be as close as possible to the input.
    Audio = ffi::OPUS_APPLICATION_AUDIO,
    /// Only use when lowest-achievable latency is what matters most.
    LowDelay = ffi::OPUS_APPLICATION_RESTRICTED_LOWDELAY,
}

impl TryFrom<i32> for Application {
    type Error = Error;

    /// Fails if a value does not match Opus' specified application-value.
    fn try_from(value: i32) -> Result<Self> {
        Ok(match value {
            ffi::OPUS_APPLICATION_VOIP => Application::Voip,
            ffi::OPUS_APPLICATION_AUDIO => Application::Audio,
            ffi::OPUS_APPLICATION_RESTRICTED_LOWDELAY => Application::LowDelay,
            _ => return Err(Error::InvalidApplication),
        })
    }
}

/// Represents possible audio channels Opus can use.
#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Channels {
    /// Not supported when constructing encoders or decoders.
    Auto = ffi::OPUS_AUTO,
    Mono = 1,
    Stereo = 2,
}

impl Channels {
    pub fn is_mono(self) -> bool {
        if let Channels::Mono = self {
            return true;
        }

        false
    }

    pub fn is_stereo(self) -> bool {
        if let Channels::Stereo = self {
            return true;
        }

        false
    }
}

impl TryFrom<i32> for Channels {
    type Error = Error;

    // Fails if a value does not match Opus' specified channel-value.
    fn try_from(value: i32) -> Result<Channels> {
        Ok(match value {
            ffi::OPUS_AUTO => Channels::Auto,
            1 => Channels::Mono,
            2 => Channels::Stereo,
            _ => return Err(Error::InvalidChannels(value)),
        })
    }
}

impl From<Channels> for i32 {
    fn from(channels: Channels) -> i32 {
        channels as i32
    }
}

/// Represents possible bandwidths of an Opus-stream.
#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Bandwidth {
    /// Pick the bandwidth automatically.
    Auto = ffi::OPUS_AUTO,
    /// A 4kHz bandwidth.
    Narrowband = ffi::OPUS_BANDWIDTH_NARROWBAND,
    /// A 6kHz bandwidth.
    Mediumband = ffi::OPUS_BANDWIDTH_MEDIUMBAND,
    /// A 8kHz bandwidth.
    Wideband = ffi::OPUS_BANDWIDTH_WIDEBAND,
    /// A 12kHz bandwidth.
    Superwideband = ffi::OPUS_BANDWIDTH_SUPERWIDEBAND,
    /// A 20kHz bandwidth.
    Fullband = ffi::OPUS_BANDWIDTH_FULLBAND,
}

impl TryFrom<i32> for Bandwidth {
    type Error = Error;

    // Fails if a value does not match Opus' specified bandwidth-value.
    fn try_from(value: i32) -> Result<Self> {
        Ok(match value {
            ffi::OPUS_AUTO => Bandwidth::Auto,
            ffi::OPUS_BANDWIDTH_NARROWBAND => Bandwidth::Narrowband,
            ffi::OPUS_BANDWIDTH_MEDIUMBAND => Bandwidth::Mediumband,
            ffi::OPUS_BANDWIDTH_WIDEBAND => Bandwidth::Wideband,
            ffi::OPUS_BANDWIDTH_SUPERWIDEBAND => Bandwidth::Superwideband,
            ffi::OPUS_BANDWIDTH_FULLBAND => Bandwidth::Fullband,
            _ => return Err(Error::InvalidBandwidth(value)),
        })
    }
}

/// A newtype wrapping around a mutable buffer. They represent mutably borrowed
/// arguments that will be filled by Opus.
#[derive(Debug)]
pub struct MutSignals<'a, T>(pub(crate) &'a mut [T]);

impl<'a, T> TryFrom<&'a mut [T]> for MutSignals<'a, T> {
    type Error = Error;

    fn try_from(value: &'a mut [T]) -> Result<Self> {
        if value.len() > i32::MAX as usize {
            return Err(Error::SignalsTooLarge);
        }

        Ok(Self(value))
    }
}

impl<'a, T> TryFrom<&'a mut Vec<T>> for MutSignals<'a, T> {
    type Error = Error;

    fn try_from(value: &'a mut Vec<T>) -> Result<Self> {
        value.as_mut_slice().try_into()
    }
}

impl<'a, T> MutSignals<'a, T> {
    /// Directly access the wrapped slice.
    ///
    /// This is an `oprs` addition (not part of the `audiopus` API): the public
    /// layer needs it to hand the buffer to the safe pure-Rust codec layer.
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        self.0
    }

    pub fn as_mut_ptr(&mut self) -> *mut T {
        self.0.as_mut_ptr()
    }

    /// Due to checking the length during construction of this newtype wrapping
    /// around an immutably borrowed buffer, we can safely cast `usize` to `i32`.
    pub fn i32_len(&self) -> i32 {
        self.0.len() as i32
    }
}

/// Gets the libopus version string.
///
/// The implementation reproduces the behaviour of `libopus 1.6.1`, and reports
/// the same version string, so that applications which inspect it (for example
/// looking for the substring `-fixed`) behave identically.
pub fn version() -> &'static str {
    crate::opus::opus_get_version_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::convert::TryFrom;

    #[test]
    fn enum_conversions_match_audiopus() {
        // `SampleRate` is matched on literal integers, the others on `ffi`
        // constants.
        assert_eq!(SampleRate::try_from(8000).unwrap(), SampleRate::Hz8000);
        assert_eq!(SampleRate::try_from(48000).unwrap(), SampleRate::Hz48000);
        assert_eq!(
            SampleRate::try_from(44100).unwrap_err(),
            Error::InvalidSampleRate(44100)
        );

        assert_eq!(Application::try_from(2048).unwrap(), Application::Voip);
        assert_eq!(Application::try_from(2049).unwrap(), Application::Audio);
        assert_eq!(Application::try_from(2051).unwrap(), Application::LowDelay);
        assert_eq!(
            Application::try_from(0).unwrap_err(),
            Error::InvalidApplication
        );

        assert_eq!(Channels::try_from(-1000).unwrap(), Channels::Auto);
        assert_eq!(Channels::try_from(1).unwrap(), Channels::Mono);
        assert_eq!(Channels::try_from(2).unwrap(), Channels::Stereo);
        assert_eq!(
            Channels::try_from(3).unwrap_err(),
            Error::InvalidChannels(3)
        );
        assert_eq!(i32::from(Channels::Stereo), 2);
        assert!(Channels::Mono.is_mono());
        assert!(!Channels::Mono.is_stereo());
        assert!(Channels::Stereo.is_stereo());

        assert_eq!(Bandwidth::try_from(-1000).unwrap(), Bandwidth::Auto);
        assert_eq!(Bandwidth::try_from(1101).unwrap(), Bandwidth::Narrowband);
        assert_eq!(Bandwidth::try_from(1105).unwrap(), Bandwidth::Fullband);
        assert_eq!(
            Bandwidth::try_from(0).unwrap_err(),
            Error::InvalidBandwidth(0)
        );

        assert_eq!(Signal::try_from(-1000).unwrap(), Signal::Auto);
        assert_eq!(Signal::try_from(3001).unwrap(), Signal::Voice);
        assert_eq!(Signal::try_from(3002).unwrap(), Signal::Music);
        assert_eq!(Signal::try_from(0).unwrap_err(), Error::InvalidSignal(0));

        // `Bitrate::try_from` reports `InvalidBandwidth`, not `InvalidBitrate`
        // (an `audiopus` quirk that is reproduced on purpose).
        assert_eq!(i32::from(Bitrate::Auto), -1000);
        assert_eq!(i32::from(Bitrate::Max), -1);
        assert_eq!(i32::from(Bitrate::BitsPerSecond(64000)), 64000);
        assert_eq!(Bitrate::try_from(-1000).unwrap(), Bitrate::Auto);
        assert_eq!(Bitrate::try_from(-1).unwrap(), Bitrate::Max);
        assert_eq!(
            Bitrate::try_from(64000).unwrap(),
            Bitrate::BitsPerSecond(64000)
        );
        assert_eq!(
            Bitrate::try_from(0).unwrap_err(),
            Error::InvalidBandwidth(0)
        );
    }

    #[test]
    fn mut_signals_conversions() {
        let mut buffer = [0i16; 4];
        let mut signals = MutSignals::try_from(&mut buffer[..]).unwrap();
        assert_eq!(signals.i32_len(), 4);
        assert!(!signals.as_mut_ptr().is_null());
        assert_eq!(signals.as_mut_slice().len(), 4);

        // Empty is allowed; only lengths above `i32::MAX` are rejected.
        let mut empty: Vec<i16> = Vec::new();
        assert!(MutSignals::try_from(&mut empty).is_ok());

        let mut buffer = vec![0.0f32; 8];
        let signals = MutSignals::try_from(&mut buffer).unwrap();
        assert_eq!(signals.i32_len(), 8);
    }
}

//! Packet newtypes and packet queries.
//!
//! This is a compatible reimplementation of `audiopus::packet` (0.3.0-rc.0):
//! the newtypes, the free functions and their validation are reproduced
//! signature for signature, but the queries call the pure-Rust port in
//! [`crate::opus::packet`] instead of the C library.

use crate::error::try_map_opus_error;
use crate::{Bandwidth, Channels, Error, Result, SampleRate};
use std::convert::TryFrom;

fn packet_len_check(packet_buffer: &[u8]) -> Result<i32> {
    match packet_buffer {
        // non-empty guarantee:
        x if x.is_empty() => Err(Error::EmptyPacket),
        // limited size guarantee:
        _ if packet_buffer.len() > std::i32::MAX as usize => Err(Error::PacketTooLarge),
        _ => Ok(packet_buffer.len() as i32),
    }
}

/// A newtype around `&[u8]` to guarantee:
/// - Minimum one element: A packet cannot be empty.
/// - Limited size: A packet's length may not exceed `std::i32::MAX`.
#[derive(Debug)]
pub struct Packet<'a>(&'a [u8]);

impl<'a> Packet<'a> {
    pub fn as_ptr(&self) -> *const u8 {
        self.0.as_ptr()
    }

    /// The underlying type is immutably borrowed and has been verified upon
    /// construction of `Packet`, thus we know casting `usize` will fit
    /// inside `i32`.
    pub fn i32_len(&self) -> i32 {
        self.0.len() as i32
    }

    /// Crate-internal access to the wrapped slice.
    ///
    /// This is **not** part of the `audiopus` API; the public layer needs it to
    /// hand the packet to the safe pure-Rust codec layer.
    pub(crate) fn as_slice(&self) -> &'a [u8] {
        self.0
    }
}

impl<'a> TryFrom<&'a Vec<u8>> for Packet<'a> {
    type Error = Error;

    fn try_from(value: &'a Vec<u8>) -> Result<Self> {
        value.as_slice().try_into()
    }
}

impl<'a> TryFrom<&'a [u8]> for Packet<'a> {
    type Error = Error;

    fn try_from(value: &'a [u8]) -> Result<Packet<'a>> {
        packet_len_check(value).map(|_| Self(value))
    }
}

/// A newtype around `&mut [u8]` to guarantee that accessing length on the
/// underlying buffer is checked each time.
#[derive(Debug)]
pub struct MutPacket<'a>(&'a mut [u8]);

impl<'a> MutPacket<'a> {
    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        self.0.as_mut_ptr()
    }

    /// Checks if the underlying buffer meets requirements.
    pub fn i32_len(&self) -> Result<i32> {
        packet_len_check(&self.0)
    }

    /// Crate-internal access to the wrapped slice.
    ///
    /// This is **not** part of the `audiopus` API; the public layer needs it to
    /// hand the buffer to the safe pure-Rust codec layer.
    pub(crate) fn as_mut_slice(&mut self) -> &mut [u8] {
        self.0
    }
}

impl<'a> TryFrom<&'a mut Vec<u8>> for MutPacket<'a> {
    type Error = Error;

    fn try_from(value: &'a mut Vec<u8>) -> Result<Self> {
        value.as_mut_slice().try_into()
    }
}

impl<'a> TryFrom<&'a mut [u8]> for MutPacket<'a> {
    type Error = Error;

    fn try_from(value: &'a mut [u8]) -> Result<Self> {
        packet_len_check(value).map(move |_| Self(value))
    }
}

/// Gets bandwidth of an Opus `packet`.
///
/// **Errors**:
/// Empty `packet` will return `Error::EmptyPacket`.
pub fn bandwidth(packet: Packet<'_>) -> Result<Bandwidth> {
    Bandwidth::try_from(crate::opus::packet::opus_packet_get_bandwidth(
        packet.as_slice(),
    ))
}

/// Gets number of samples per frame of an Opus `packet`.
///
/// **Errors**:
/// Empty `packet` will return `Error::EmptyPacket`.
pub fn samples_per_frame(packet: Packet<'_>, sample_rate: SampleRate) -> Result<usize> {
    // Like `audiopus`, the raw result is cast unconditionally: this function
    // cannot report an error.
    Ok(
        crate::opus::packet::opus_packet_get_samples_per_frame(
            packet.as_slice(),
            sample_rate as i32,
        ) as usize,
    )
}

/// Gets number of samples in an Opus `packet`.
///
/// **Errors**:
/// Empty `packet` will return `Error::EmptyPacket`.
pub fn nb_samples(packet: Packet<'_>, sample_rate: SampleRate) -> Result<usize> {
    try_map_opus_error(crate::opus::packet::opus_packet_get_nb_samples(
        packet.as_slice(),
        packet.i32_len(),
        sample_rate as i32,
    ))
    .map(|n| n as usize)
}

/// Gets number of channels in an Opus `packet`.
///
/// **Errors**:
/// Empty `packet` will return `Error::EmptyPacket`.
pub fn nb_channels(packet: Packet<'_>) -> Result<Channels> {
    Ok(Channels::try_from(
        crate::opus::packet::opus_packet_get_nb_channels(packet.as_slice()),
    )?)
}

/// Gets number of frames in an Opus `packet`.
///
/// **Errors**:
/// Empty `packet` will return [`Error::EmptyPacket`].
pub fn nb_frames(packet: Packet<'_>) -> Result<usize> {
    try_map_opus_error(crate::opus::packet::opus_packet_get_nb_frames(
        packet.as_slice(),
        packet.i32_len(),
    ))
    .map(|n| n as usize)
}

#[cfg(test)]
mod tests {
    use super::{bandwidth, nb_channels, nb_frames, nb_samples, samples_per_frame};
    use crate::packet::{MutPacket, Packet};
    use crate::{Bandwidth, Channels, Error, SampleRate};
    use std::convert::TryFrom;

    /// We verify the `TryFrom`-impl for `Packet` by creating and then
    /// converting from `Vec`s that meet and violate the contract.
    #[test]
    fn packet_bandwidth() {
        let empty_packet = vec![];
        let empty_packet_bandwidth = Packet::try_from(&empty_packet);
        assert_eq!(empty_packet_bandwidth.unwrap_err(), Error::EmptyPacket);

        let narrow_packet = vec![1, 2, 3];
        let narrow_packet_bandwidth = bandwidth(Packet::try_from(&narrow_packet).unwrap());
        assert_eq!(narrow_packet_bandwidth.unwrap(), Bandwidth::Narrowband);

        let mediumband_packet = vec![50];
        let mediumband_packet_bandwidth = bandwidth(Packet::try_from(&mediumband_packet).unwrap());
        assert_eq!(mediumband_packet_bandwidth.unwrap(), Bandwidth::Mediumband);

        let wideband_packet = vec![80];
        let wideband_packet_bandwidth = bandwidth(Packet::try_from(&wideband_packet).unwrap());
        assert_eq!(wideband_packet_bandwidth.unwrap(), Bandwidth::Wideband);

        let superwideband_packet = vec![200];
        let superwideband_bandwidth = bandwidth(Packet::try_from(&superwideband_packet).unwrap());
        assert_eq!(superwideband_bandwidth.unwrap(), Bandwidth::Superwideband);

        let fullband_packet = vec![255];
        let fullband_bandwidth = bandwidth(Packet::try_from(&fullband_packet).unwrap());
        assert_eq!(fullband_bandwidth.unwrap(), Bandwidth::Fullband);
    }

    #[test]
    fn packet_queries() {
        let packet_bytes = vec![1u8, 2, 3];
        // `Packet` is deliberately neither `Copy` nor `Clone` (as in
        // `audiopus`), so every query gets its own newtype.
        let new_packet = || Packet::try_from(&packet_bytes).unwrap();
        assert_eq!(new_packet().i32_len(), 3);
        assert!(!new_packet().as_ptr().is_null());

        // `data[0] == 1`: 10 ms SILK, code 1 (two frames), mono.
        assert_eq!(
            samples_per_frame(new_packet(), SampleRate::Hz48000).unwrap(),
            480
        );
        assert_eq!(nb_samples(new_packet(), SampleRate::Hz48000).unwrap(), 960);
        assert_eq!(nb_frames(new_packet()).unwrap(), 2);
        assert_eq!(nb_channels(new_packet()).unwrap(), Channels::Mono);
    }

    #[test]
    fn mut_packet_conversions() {
        let mut empty: Vec<u8> = Vec::new();
        assert_eq!(
            MutPacket::try_from(&mut empty).unwrap_err(),
            Error::EmptyPacket
        );

        let mut buffer = vec![1u8, 2, 3];
        let mut packet = MutPacket::try_from(&mut buffer).unwrap();
        assert_eq!(packet.i32_len().unwrap(), 3);
        assert!(!packet.as_mut_ptr().is_null());

        // The slice-based impl performs the same check.
        let mut slice = [0u8; 2];
        assert!(MutPacket::try_from(&mut slice[..]).is_ok());
        assert!(MutPacket::try_from(&mut [][..]).is_err());
    }
}

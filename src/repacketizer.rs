//! The repacketizer and the packet padding helpers.
//!
//! This is a compatible reimplementation of `audiopus::repacketizer`
//! (0.3.0-rc.0): same types and signatures, but the work is done by the
//! pure-Rust port in [`crate::opus::repacketizer`].
//!
//! `audiopus` gives the C library interior mutability through a raw
//! `*mut OpusRepacketizer`, which is why its `repacketizer_*` methods take
//! `&self`.  The pure-Rust port owns its frame buffers and needs `&mut self`
//! for `cat`/`out`/`out_range`, so the state is wrapped in a [`RefCell`] here to
//! keep the `&self` signatures.  As in `audiopus`, the type is not `Sync`.

use crate::Result;
use crate::error::try_map_opus_error;
use crate::packet::{MutPacket, Packet};
use std::cell::RefCell;
use std::fmt;

/// Returns Opus' internal `OpusRepacketizer`'s size in bytes.
pub fn repacketizer_size() -> usize {
    crate::opus::repacketizer::OpusRepacketizer::get_size() as usize
}

pub fn multistream_packet_pad(
    mut data: MutPacket<'_>,
    new_len: usize,
    nb_streams: usize,
) -> Result<()> {
    let len = data.i32_len()?;

    try_map_opus_error(crate::opus::repacketizer::opus_multistream_packet_pad(
        data.as_mut_slice(),
        len,
        new_len as i32,
        nb_streams as i32,
    ))
    .map(|_| ())
}

pub fn multistream_packet_unpad(mut data: MutPacket<'_>, nb_streams: usize) -> Result<()> {
    let len = data.i32_len()?;

    try_map_opus_error(crate::opus::repacketizer::opus_multistream_packet_unpad(
        data.as_mut_slice(),
        len,
        nb_streams as i32,
    ))
    .map(|_| ())
}

pub fn packet_pad(mut data: MutPacket<'_>, new_len: i32) -> Result<()> {
    let len = data.i32_len()?;

    try_map_opus_error(crate::opus::repacketizer::opus_packet_pad(
        data.as_mut_slice(),
        len,
        new_len,
    ))
    .map(|_| ())
}

pub fn packet_unpad(mut data: MutPacket<'_>) -> Result<()> {
    let len = data.i32_len()?;

    try_map_opus_error(crate::opus::repacketizer::opus_packet_unpad(
        data.as_mut_slice(),
        len,
    ))
    .map(|_| ())
}

/// The `audiopus` `Repacketizer`: an owned `OpusRepacketizer` behind a
/// [`RefCell`] (see the module documentation).
pub struct Repacketizer {
    state: RefCell<Box<crate::opus::repacketizer::OpusRepacketizer>>,
}

impl fmt::Debug for Repacketizer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Repacketizer")
            .field("nb_frames", &self.nb_frames())
            .finish_non_exhaustive()
    }
}

impl Default for Repacketizer {
    fn default() -> Self {
        Self::new()
    }
}

impl Repacketizer {
    pub fn new() -> Self {
        Self {
            state: RefCell::new(crate::opus::repacketizer::OpusRepacketizer::create()),
        }
    }

    pub fn nb_frames(&self) -> usize {
        self.state.borrow().get_nb_frames() as usize
    }

    pub fn repacketizer_out(&self, mut data_out: MutPacket<'_>, max_len: i32) -> Result<()> {
        let mut state = self.state.borrow_mut();

        try_map_opus_error(state.out(data_out.as_mut_slice(), max_len)).map(|_| ())
    }

    pub fn repacketizer_out_range(
        &self,
        begin: i32,
        end: i32,
        mut data_out: MutPacket<'_>,
        max_len: i32,
    ) -> Result<()> {
        let mut state = self.state.borrow_mut();

        try_map_opus_error(state.out_range(begin, end, data_out.as_mut_slice(), max_len))
            .map(|_| ())
    }

    pub fn repacketizer_cat(&self, data: Packet<'_>) -> Result<()> {
        let mut state = self.state.borrow_mut();

        try_map_opus_error(state.cat(data.as_slice(), data.i32_len())).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Repacketizer, multistream_packet_pad, multistream_packet_unpad, packet_pad, packet_unpad,
        repacketizer_size,
    };
    use crate::packet::{MutPacket, Packet};
    use std::convert::TryFrom;

    /// A single 2.5 ms CELT-only frame with a 3-byte payload (`toc = 0x80`).
    const PACKET: [u8; 4] = [0x80, 1, 2, 3];

    #[test]
    fn size_and_default() {
        assert!(repacketizer_size() > 0);

        let repacketizer = Repacketizer::default();
        assert_eq!(repacketizer.nb_frames(), 0);
        assert!(format!("{:?}", repacketizer).contains("Repacketizer"));
    }

    #[test]
    fn cat_and_out_round_trip() {
        let repacketizer = Repacketizer::new();
        let packet = Packet::try_from(&PACKET[..]).unwrap();

        repacketizer.repacketizer_cat(packet).unwrap();
        assert_eq!(repacketizer.nb_frames(), 1);

        let mut out = vec![0u8; 64];
        repacketizer
            .repacketizer_out(MutPacket::try_from(&mut out).unwrap(), 64)
            .unwrap();
        assert_eq!(&out[..PACKET.len()], &PACKET[..]);

        let mut out_range = vec![0u8; 64];
        repacketizer
            .repacketizer_out_range(0, 1, MutPacket::try_from(&mut out_range).unwrap(), 64)
            .unwrap();
        assert_eq!(&out_range[..PACKET.len()], &PACKET[..]);
    }

    #[test]
    fn cat_rejects_garbage() {
        let repacketizer = Repacketizer::new();
        // `toc = 0x80`, code 3, second byte 0x3F: 63 frames, but too short.
        let garbage = [0x83u8, 0x3F];
        let packet = Packet::try_from(&garbage[..]).unwrap();
        assert!(repacketizer.repacketizer_cat(packet).is_err());
    }

    #[test]
    fn pad_and_unpad() {
        // `MutPacket` cannot grow, so the only successful padding call is the
        // no-op one (`new_len == len`); this is the same limitation `audiopus`
        // has.
        let mut buffer = PACKET.to_vec();
        packet_pad(
            MutPacket::try_from(&mut buffer).unwrap(),
            PACKET.len() as i32,
        )
        .unwrap();

        let mut buffer = PACKET.to_vec();
        packet_unpad(MutPacket::try_from(&mut buffer).unwrap()).unwrap();
        assert_eq!(&buffer[..PACKET.len()], &PACKET[..]);

        let mut buffer = PACKET.to_vec();
        multistream_packet_pad(MutPacket::try_from(&mut buffer).unwrap(), PACKET.len(), 1).unwrap();

        let mut buffer = PACKET.to_vec();
        multistream_packet_unpad(MutPacket::try_from(&mut buffer).unwrap(), 1).unwrap();
        assert_eq!(&buffer[..PACKET.len()], &PACKET[..]);
    }
}

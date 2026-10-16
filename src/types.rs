//! Fixed-width integer types mirroring `include/opus_types.h`.
//!
//! The names are kept identical to the C originals so that transliterated code
//! can be diffed against the reference implementation line by line.

pub type opus_int8 = i8;
pub type opus_uint8 = u8;
pub type opus_int16 = i16;
pub type opus_uint16 = u16;
pub type opus_int32 = i32;
pub type opus_uint32 = u32;
pub type opus_int64 = i64;
pub type opus_uint64 = u64;

/// `opus_intptr` is only used by the multistream API.
pub type opus_intptr = isize;
pub type opus_uintptr = usize;

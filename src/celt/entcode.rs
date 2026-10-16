//! Mirror of `celt/entcode.{h,c}`.
//!
//! The range coder is integer-only and therefore identical in the float and
//! fixed-point configurations; this is the foundation everything else in CELT
//! and SILK is built on, so it is ported first and verified bit-exactly.
//!
//! The C structure stores a raw `unsigned char *buf` plus a `storage` limit.
//! Here the buffer is owned (`Vec<u8>`); the decoder copies the packet into it,
//! which costs one `memcpy` per packet and keeps the whole coder safe.
//! `buf.len() >= storage` always holds, and `storage` is the only bound used.

use crate::types::*;

/// `typedef opus_uint32 ec_window;`
pub type ec_window = opus_uint32;

/// `EC_WINDOW_SIZE`
pub const EC_WINDOW_SIZE: i32 = 32;
/// `EC_UINT_BITS`
pub const EC_UINT_BITS: i32 = 8;
/// `BITRES`
pub const BITRES: i32 = 3;

/*Constants used by the entropy encoder/decoder (celt/mfrngcod.h).*/
/// `EC_SYM_BITS`
pub const EC_SYM_BITS: i32 = 8;
/// `EC_CODE_BITS`
pub const EC_CODE_BITS: i32 = 32;
/// `EC_SYM_MAX`
pub const EC_SYM_MAX: u32 = (1u32 << EC_SYM_BITS) - 1;
/// `EC_CODE_SHIFT`
pub const EC_CODE_SHIFT: i32 = EC_CODE_BITS - EC_SYM_BITS - 1;
/// `EC_CODE_TOP`
pub const EC_CODE_TOP: u32 = 1u32 << (EC_CODE_BITS - 1);
/// `EC_CODE_BOT`
pub const EC_CODE_BOT: u32 = EC_CODE_TOP >> EC_SYM_BITS;
/// `EC_CODE_EXTRA`
pub const EC_CODE_EXTRA: i32 = (EC_CODE_BITS - 2) % EC_SYM_BITS + 1;

/// The entropy encoder/decoder context (`struct ec_ctx`).
#[derive(Clone)]
pub struct ec_ctx {
    /// Buffered input/output.
    pub buf: Vec<u8>,
    /// The size of the buffer.
    pub storage: opus_uint32,
    /// The offset at which the last byte containing raw bits was read/written.
    pub end_offs: opus_uint32,
    /// Bits that will be read from/written at the end.
    pub end_window: ec_window,
    /// Number of valid bits in `end_window`.
    pub nend_bits: i32,
    /// The total number of whole bits read/written.
    pub nbits_total: i32,
    /// The offset at which the next range coder byte will be read/written.
    pub offs: opus_uint32,
    /// The number of values in the current range.
    pub rng: opus_uint32,
    /// Decoder: the difference between the top of the current range and the
    /// input value, minus one.  Encoder: the low end of the current range.
    pub val: opus_uint32,
    /// Decoder: the saved normalization factor from `ec_decode()`.
    /// Encoder: the number of outstanding carry propagating symbols.
    pub ext: opus_uint32,
    /// A buffered input/output symbol, awaiting carry propagation.
    pub rem: i32,
    /// Nonzero if an error occurred.
    pub error: i32,
}

/// `EC_CLZ0 - EC_CLZ(x)` with `__builtin_clz`.
///
/// The C macro is undefined for `_x == 0`; the Rust version returns 0 for that
/// input, which never changes a well-defined computation.
#[inline(always)]
pub fn EC_ILOG(x: opus_uint32) -> i32 {
    (32 - x.leading_zeros()) as i32
}

impl ec_ctx {
    #[inline(always)]
    pub fn ec_range_bytes(&self) -> opus_uint32 {
        self.offs
    }

    #[inline(always)]
    pub fn ec_get_buffer(&self) -> &[u8] {
        &self.buf
    }

    #[inline(always)]
    pub fn ec_get_error(&self) -> i32 {
        self.error
    }

    /// `ec_tell()`
    #[inline(always)]
    pub fn ec_tell(&self) -> i32 {
        self.nbits_total - EC_ILOG(self.rng)
    }
}

/// `ec_tell_frac()`
pub fn ec_tell_frac(this: &ec_ctx) -> opus_uint32 {
    const CORRECTION: [u32; 8] = [
        35733, 38967, 42495, 46340, 50535, 55109, 60097, 65535,
    ];
    let nbits: u32 = (this.nbits_total as u32) << BITRES;
    let l: i32 = EC_ILOG(this.rng);
    let r: u32 = this.rng >> (l - 16);
    let mut b: u32 = (r >> 12) - 8;
    if r > CORRECTION[b as usize] {
        b += 1;
    }
    let l = (l << 3) + b as i32;
    nbits.wrapping_sub(l as u32)
}

/// `celt_udiv(n, d)`.
///
/// `USE_SMALL_DIV_TABLE` is only defined for ARM assembly builds, so on the
/// x86-64 reference build this is a plain division.
#[inline(always)]
pub fn celt_udiv(n: opus_uint32, d: opus_uint32) -> opus_uint32 {
    debug_assert!(d > 0);
    n / d
}

/// `celt_sudiv(n, d)`
#[inline(always)]
pub fn celt_sudiv(n: opus_int32, d: opus_int32) -> opus_int32 {
    debug_assert!(d > 0);
    n / d
}

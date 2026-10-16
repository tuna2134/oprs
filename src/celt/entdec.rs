//! Mirror of `celt/entdec.c`.

use super::arch::*;
use super::entcode::*;
use crate::types::*;

#[inline(always)]
fn ec_read_byte(this: &mut ec_ctx) -> i32 {
    if this.offs < this.storage {
        let b = this.buf[this.offs as usize] as i32;
        this.offs += 1;
        b
    } else {
        0
    }
}

#[inline(always)]
fn ec_read_byte_from_end(this: &mut ec_ctx) -> i32 {
    if this.end_offs < this.storage {
        this.end_offs += 1;
        this.buf[(this.storage - this.end_offs) as usize] as i32
    } else {
        0
    }
}

/// Normalizes the contents of `val` and `rng` so that `rng` lies entirely in
/// the high-order symbol.
fn ec_dec_normalize(this: &mut ec_ctx) {
    /*If the range is too small, rescale it and input some bits.*/
    while this.rng <= EC_CODE_BOT {
        let mut sym;
        this.nbits_total += EC_SYM_BITS;
        this.rng <<= EC_SYM_BITS;
        /*Use up the remaining bits from our last symbol.*/
        sym = this.rem;
        /*Read the next value from the input.*/
        this.rem = ec_read_byte(this);
        /*Take the rest of the bits we need from this new symbol.*/
        sym = (sym << EC_SYM_BITS | this.rem) >> (EC_SYM_BITS - EC_CODE_EXTRA);
        /*And subtract them from val, capped to be less than EC_CODE_TOP.*/
        this.val = ((this.val << EC_SYM_BITS) + (EC_SYM_MAX & !(sym as u32))) & (EC_CODE_TOP - 1);
    }
}

/// `ec_dec_init()`
pub fn ec_dec_init(buf: &[u8]) -> ec_ctx {
    let mut this = ec_ctx {
        buf: buf.to_vec(),
        storage: buf.len() as opus_uint32,
        end_offs: 0,
        end_window: 0,
        nend_bits: 0,
        /*This is the offset from which ec_tell() will subtract partial bits.
        The final value after the ec_dec_normalize() call will be the same as in
        the encoder, but we have to compensate for the bits that are added there.*/
        nbits_total: EC_CODE_BITS + 1
            - ((EC_CODE_BITS - EC_CODE_EXTRA) / EC_SYM_BITS) * EC_SYM_BITS,
        offs: 0,
        rng: 1u32 << EC_CODE_EXTRA,
        val: 0,
        ext: 0,
        rem: 0,
        error: 0,
    };
    this.rem = ec_read_byte(&mut this);
    this.val = this.rng - 1 - ((this.rem as u32) >> (EC_SYM_BITS - EC_CODE_EXTRA));
    /*Normalize the interval.*/
    ec_dec_normalize(&mut this);
    this
}

/// `EC_MINI(a,b)` — only `a`, `b` >= 0 are used here.
#[inline(always)]
fn EC_MINI(a: u32, b: u32) -> u32 {
    if b < a {
        b
    } else {
        a
    }
}

/// `ec_decode()`
pub fn ec_decode(this: &mut ec_ctx, ft: u32) -> u32 {
    let s: u32;
    this.ext = celt_udiv(this.rng, ft);
    s = this.val / this.ext;
    ft - EC_MINI(s + 1, ft)
}

/// `ec_decode_bin()`
pub fn ec_decode_bin(this: &mut ec_ctx, bits: u32) -> u32 {
    let s: u32;
    this.ext = this.rng >> bits;
    s = this.val / this.ext;
    (1u32 << bits) - EC_MINI(s + 1, 1u32 << bits)
}

/// `ec_dec_update()`
pub fn ec_dec_update(this: &mut ec_ctx, fl: u32, fh: u32, ft: u32) {
    let s: opus_uint32;
    s = IMUL32(this.ext, ft - fh);
    this.val -= s;
    this.rng = if fl > 0 {
        IMUL32(this.ext, fh - fl)
    } else {
        this.rng - s
    };
    ec_dec_normalize(this);
}

/// The probability of having a "one" is 1/(1<<logp).
pub fn ec_dec_bit_logp(this: &mut ec_ctx, logp: u32) -> i32 {
    let r: opus_uint32;
    let d: opus_uint32;
    let s: opus_uint32;
    let ret: i32;
    r = this.rng;
    d = this.val;
    s = r >> logp;
    ret = if d < s { 1 } else { 0 };
    if ret == 0 {
        this.val = d - s;
    }
    this.rng = if ret != 0 { s } else { r - s };
    ec_dec_normalize(this);
    ret
}

/// `ec_dec_icdf()`
pub fn ec_dec_icdf(this: &mut ec_ctx, icdf: &[u8], ftb: u32) -> i32 {
    let mut r: opus_uint32;
    let d: opus_uint32;
    let mut s: opus_uint32;
    let mut t: opus_uint32;
    let mut ret: i32;
    s = this.rng;
    d = this.val;
    r = s >> ftb;
    ret = -1;
    loop {
        t = s;
        ret += 1;
        s = IMUL32(r, icdf[ret as usize] as u32);
        if !(d < s) {
            break;
        }
    }
    this.val = d - s;
    this.rng = t - s;
    ec_dec_normalize(this);
    ret
}

/// `ec_dec_icdf16()`
pub fn ec_dec_icdf16(this: &mut ec_ctx, icdf: &[u16], ftb: u32) -> i32 {
    let mut r: opus_uint32;
    let d: opus_uint32;
    let mut s: opus_uint32;
    let mut t: opus_uint32;
    let mut ret: i32;
    s = this.rng;
    d = this.val;
    r = s >> ftb;
    ret = -1;
    loop {
        t = s;
        ret += 1;
        s = IMUL32(r, icdf[ret as usize] as u32);
        if !(d < s) {
            break;
        }
    }
    this.val = d - s;
    this.rng = t - s;
    ec_dec_normalize(this);
    ret
}

/// `ec_dec_uint()`
pub fn ec_dec_uint(this: &mut ec_ctx, ft: opus_uint32) -> opus_uint32 {
    let ft_: u32;
    let mut s: u32;
    let mut ftb: i32;
    let mut ft_in = ft;
    /*In order to optimize EC_ILOG(), it is undefined for the value 0.*/
    debug_assert!(ft_in > 1);
    ft_in -= 1;
    ftb = EC_ILOG(ft_in);
    if ftb > EC_UINT_BITS {
        let t: opus_uint32;
        ftb -= EC_UINT_BITS;
        ft_ = (ft_in >> ftb) + 1;
        s = ec_decode(this, ft_);
        ec_dec_update(this, s, s + 1, ft_);
        t = (s as opus_uint32) << ftb | ec_dec_bits(this, ftb as u32);
        if t <= ft_in {
            t
        } else {
            this.error = 1;
            ft_in
        }
    } else {
        ft_in += 1;
        s = ec_decode(this, ft_in);
        ec_dec_update(this, s, s + 1, ft_in);
        s
    }
}

/// `ec_dec_bits()`
pub fn ec_dec_bits(this: &mut ec_ctx, bits: u32) -> opus_uint32 {
    let mut window: ec_window;
    let mut available: i32;
    let ret: opus_uint32;
    window = this.end_window;
    available = this.nend_bits;
    if (available as u32) < bits {
        loop {
            window |= (ec_read_byte_from_end(this) as ec_window) << available;
            available += EC_SYM_BITS;
            if !(available <= EC_WINDOW_SIZE - EC_SYM_BITS) {
                break;
            }
        }
    }
    ret = window & (((1u32) << bits) - 1u32);
    window >>= bits;
    available -= bits as i32;
    this.end_window = window;
    this.nend_bits = available;
    this.nbits_total += bits as i32;
    ret
}

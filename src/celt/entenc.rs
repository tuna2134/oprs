//! Mirror of `celt/entenc.c`.

use super::arch::*;
use super::entcode::*;
use crate::types::*;

#[inline(always)]
fn ec_write_byte(this: &mut ec_ctx, value: u32) -> i32 {
    if this.offs + this.end_offs >= this.storage {
        return -1;
    }
    this.buf[this.offs as usize] = value as u8;
    this.offs += 1;
    0
}

#[inline(always)]
fn ec_write_byte_at_end(this: &mut ec_ctx, value: u32) -> i32 {
    if this.offs + this.end_offs >= this.storage {
        return -1;
    }
    this.end_offs += 1;
    this.buf[(this.storage - this.end_offs) as usize] = value as u8;
    0
}

/// Outputs a symbol, with a carry bit.
///
/// If there is a potential to propagate a carry over several symbols, they are
/// buffered until it can be determined whether or not an actual carry will
/// occur.
fn ec_enc_carry_out(this: &mut ec_ctx, c: i32) {
    if c != EC_SYM_MAX as i32 {
        /*No further carry propagation possible, flush buffer.*/
        let carry: i32;
        carry = c >> EC_SYM_BITS;
        /*Don't output a byte on the first write.
        This compare should be taken care of by branch-prediction thereafter.*/
        if this.rem >= 0 {
            this.error |= ec_write_byte(this, (this.rem + carry) as u32);
        }
        if this.ext > 0 {
            let sym: u32;
            sym = ((EC_SYM_MAX as i32 + carry) & EC_SYM_MAX as i32) as u32;
            loop {
                this.error |= ec_write_byte(this, sym);
                this.ext -= 1;
                if this.ext == 0 {
                    break;
                }
            }
        }
        this.rem = c & EC_SYM_MAX as i32;
    } else {
        this.ext += 1;
    }
}

#[inline(always)]
fn ec_enc_normalize(this: &mut ec_ctx) {
    /*If the range is too small, output some bits and rescale it.*/
    while this.rng <= EC_CODE_BOT {
        ec_enc_carry_out(this, (this.val >> EC_CODE_SHIFT) as i32);
        /*Move the next-to-high-order symbol into the high-order position.*/
        this.val = (this.val << EC_SYM_BITS) & (EC_CODE_TOP - 1);
        this.rng <<= EC_SYM_BITS;
        this.nbits_total += EC_SYM_BITS;
    }
}

/// `ec_enc_init()` — allocates a buffer of `size` zeroed bytes.
pub fn ec_enc_init(size: opus_uint32) -> ec_ctx {
    ec_ctx {
        buf: vec![0u8; size as usize],
        end_offs: 0,
        end_window: 0,
        nend_bits: 0,
        /*This is the offset from which ec_tell() will subtract partial bits.*/
        nbits_total: EC_CODE_BITS + 1,
        offs: 0,
        rng: EC_CODE_TOP,
        rem: -1,
        val: 0,
        ext: 0,
        storage: size,
        error: 0,
    }
}

/// `ec_encode()`
pub fn ec_encode(this: &mut ec_ctx, fl: u32, fh: u32, ft: u32) {
    let r: opus_uint32;
    r = celt_udiv(this.rng, ft);
    if fl > 0 {
        this.val = this.val.wrapping_add(this.rng - IMUL32(r, ft - fl));
        this.rng = IMUL32(r, fh - fl);
    } else {
        this.rng -= IMUL32(r, ft - fh);
    }
    ec_enc_normalize(this);
}

/// `ec_encode_bin()`
pub fn ec_encode_bin(this: &mut ec_ctx, fl: u32, fh: u32, bits: u32) {
    let r: opus_uint32;
    r = this.rng >> bits;
    if fl > 0 {
        this.val = this
            .val
            .wrapping_add(this.rng - IMUL32(r, (1u32 << bits) - fl));
        this.rng = IMUL32(r, fh - fl);
    } else {
        this.rng -= IMUL32(r, (1u32 << bits) - fh);
    }
    ec_enc_normalize(this);
}

/// The probability of having a "one" is 1/(1<<logp).
pub fn ec_enc_bit_logp(this: &mut ec_ctx, val: i32, logp: u32) {
    let r: opus_uint32;
    let s: opus_uint32;
    let l: opus_uint32;
    r = this.rng;
    l = this.val;
    s = r >> logp;
    let r2 = r - s;
    if val != 0 {
        this.val = l + r2;
    }
    this.rng = if val != 0 { s } else { r2 };
    ec_enc_normalize(this);
}

/// `ec_enc_icdf()`
pub fn ec_enc_icdf(this: &mut ec_ctx, s: i32, icdf: &[u8], ftb: u32) {
    let r: opus_uint32;
    r = this.rng >> ftb;
    if s > 0 {
        this.val = this
            .val
            .wrapping_add(this.rng - IMUL32(r, icdf[s as usize - 1] as u32));
        this.rng = IMUL32(r, (icdf[s as usize - 1] as u32) - (icdf[s as usize] as u32));
    } else {
        this.rng -= IMUL32(r, icdf[s as usize] as u32);
    }
    ec_enc_normalize(this);
}

/// `ec_enc_icdf16()`
pub fn ec_enc_icdf16(this: &mut ec_ctx, s: i32, icdf: &[u16], ftb: u32) {
    let r: opus_uint32;
    r = this.rng >> ftb;
    if s > 0 {
        this.val = this
            .val
            .wrapping_add(this.rng - IMUL32(r, icdf[s as usize - 1] as u32));
        this.rng = IMUL32(r, (icdf[s as usize - 1] as u32) - (icdf[s as usize] as u32));
    } else {
        this.rng -= IMUL32(r, icdf[s as usize] as u32);
    }
    ec_enc_normalize(this);
}

/// `ec_enc_uint()`
pub fn ec_enc_uint(this: &mut ec_ctx, fl: opus_uint32, ft: opus_uint32) {
    let ft_: u32;
    let fl_: u32;
    let mut ftb: i32;
    let mut ft_in = ft;
    /*In order to optimize EC_ILOG(), it is undefined for the value 0.*/
    debug_assert!(ft_in > 1);
    ft_in -= 1;
    ftb = EC_ILOG(ft_in);
    if ftb > EC_UINT_BITS {
        ftb -= EC_UINT_BITS;
        ft_ = (ft_in >> ftb) + 1;
        fl_ = fl >> ftb;
        ec_encode(this, fl_, fl_ + 1, ft_);
        ec_enc_bits(this, fl & (((1u32) << ftb) - 1u32), ftb as u32);
    } else {
        ec_encode(this, fl, fl + 1, ft_in + 1);
    }
}

/// `ec_enc_bits()`
pub fn ec_enc_bits(this: &mut ec_ctx, fl: opus_uint32, bits: u32) {
    let mut window: ec_window;
    let mut used: i32;
    window = this.end_window;
    used = this.nend_bits;
    debug_assert!(bits > 0);
    if used + bits as i32 > EC_WINDOW_SIZE {
        loop {
            this.error |= ec_write_byte_at_end(this, window & EC_SYM_MAX);
            window >>= EC_SYM_BITS;
            used -= EC_SYM_BITS;
            if !(used >= EC_SYM_BITS) {
                break;
            }
        }
    }
    window |= fl.wrapping_shl(used as u32);
    used += bits as i32;
    this.end_window = window;
    this.nend_bits = used;
    this.nbits_total += bits as i32;
}

/// `ec_enc_patch_initial_bits()`
pub fn ec_enc_patch_initial_bits(this: &mut ec_ctx, val: u32, nbits: u32) {
    let shift: i32;
    let mask: u32;
    debug_assert!(nbits <= EC_SYM_BITS as u32);
    shift = EC_SYM_BITS - nbits as i32;
    mask = ((1u32 << nbits) - 1) << shift;
    if this.offs > 0 {
        /*The first byte has been finalized.*/
        this.buf[0] = ((this.buf[0] as u32 & !mask) | val << shift) as u8;
    } else if this.rem >= 0 {
        /*The first byte is still awaiting carry propagation.*/
        this.rem = (this.rem as u32 & !mask | val << shift) as i32;
    } else if this.rng <= (EC_CODE_TOP >> nbits) {
        /*The renormalization loop has never been run.*/
        this.val = (this.val & !(mask << EC_CODE_SHIFT)) | val << (EC_CODE_SHIFT + shift);
    } else {
        /*The encoder hasn't even encoded _nbits of data yet.*/
        this.error = -1;
    }
}

/// `ec_enc_shrink()`
pub fn ec_enc_shrink(this: &mut ec_ctx, size: opus_uint32) {
    debug_assert!(this.offs + this.end_offs <= size);
    if this.end_offs > 0 {
        let src = (this.storage - this.end_offs) as usize;
        let dst = (size - this.end_offs) as usize;
        let n = this.end_offs as usize;
        this.buf.copy_within(src..src + n, dst);
    }
    this.storage = size;
}

/// `ec_enc_done()`
pub fn ec_enc_done(this: &mut ec_ctx) {
    let mut window: ec_window;
    let mut used: i32;
    let mut msk: opus_uint32;
    let mut end: opus_uint32;
    let mut l: i32;
    /*We output the minimum number of bits that ensures that the symbols encoded
    thus far will be decoded correctly regardless of the bits that follow.*/
    l = EC_CODE_BITS - EC_ILOG(this.rng);
    msk = (EC_CODE_TOP - 1) >> l;
    end = (this.val.wrapping_add(msk)) & !msk;
    if (end | msk) >= this.val.wrapping_add(this.rng) {
        l += 1;
        msk >>= 1;
        end = (this.val.wrapping_add(msk)) & !msk;
    }
    while l > 0 {
        ec_enc_carry_out(this, (end >> EC_CODE_SHIFT) as i32);
        end = (end << EC_SYM_BITS) & (EC_CODE_TOP - 1);
        l -= EC_SYM_BITS;
    }
    /*If we have a buffered byte flush it into the output buffer.*/
    if this.rem >= 0 || this.ext > 0 {
        ec_enc_carry_out(this, 0);
    }
    /*If we have buffered extra bits, flush them as well.*/
    window = this.end_window;
    used = this.nend_bits;
    while used >= EC_SYM_BITS {
        this.error |= ec_write_byte_at_end(this, window & EC_SYM_MAX);
        window >>= EC_SYM_BITS;
        used -= EC_SYM_BITS;
    }
    /*Clear any excess space and add any remaining extra bits to the last byte.*/
    if this.error == 0 {
        let from = this.offs as usize;
        let to = (this.storage - this.end_offs) as usize;
        for b in this.buf[from..to].iter_mut() {
            *b = 0;
        }
        if used > 0 {
            /*If there's no range coder data at all, give up.*/
            if this.end_offs >= this.storage {
                this.error = -1;
            } else {
                l = -l;
                /*If we've busted, don't add too many extra bits to the last byte; it
                would corrupt the range coder data, and that's more important.*/
                if this.offs + this.end_offs >= this.storage && l < used {
                    window &= (1u32 << l) - 1;
                    this.error = -1;
                }
                this.buf[(this.storage - this.end_offs - 1) as usize] |= window as u8;
            }
        }
    }
}

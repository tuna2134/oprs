//! Mirror of `silk/macros.h` from libopus 1.6.1, transliterated to safe Rust.
//!
//! SILK's platform macros.  `OPUS_FAST_INT64` is 1 on x86-64 (the pinned configuration), so the 64-bit-intermediate branches of the multiplicative macros are the live ones; the shift-and-add fallbacks are not ported.
//!
//! Original C copyright notice (this file is a derived work):
// /***********************************************************************
// Copyright (c) 2006-2011, Skype Limited. All rights reserved.
// Redistribution and use in source and binary forms, with or without
// modification, are permitted provided that the following conditions
// are met:
// - Redistributions of source code must retain the above copyright notice,
// this list of conditions and the following disclaimer.
// - Redistributions in binary form must reproduce the above copyright
// notice, this list of conditions and the following disclaimer in the
// documentation and/or other materials provided with the distribution.
// - Neither the name of Internet Society, IETF or IETF Trust, nor the
// names of specific contributors, may be used to endorse or promote
// products derived from this software without specific prior written
// permission.
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
// AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
// IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
// ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER OR CONTRIBUTORS BE
// LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR
// CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF
// SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS
// INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN
// CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE)
// ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE
// POSSIBILITY OF SUCH DAMAGE.
// ***********************************************************************/

use crate::celt::entcode::EC_ILOG;
use crate::silk::typedef::*;
use crate::types::*;

#[allow(unused_imports)]
pub(crate) use crate::silk::typedef::silk_assert;

// ---------------------------------------------------------------------------
// Multiply/accumulate with a 16-bit operand.
//
// C keeps the exact 32-bit result of the C `(opus_int16)` cast, which is a
// two's-complement truncation; Rust's `as i16` truncates identically.
// ---------------------------------------------------------------------------

/// `((opus_int32)(((a32) * (opus_int64)((opus_int16)(b32))) >> 16))`
#[inline(always)]
pub fn silk_SMULWB(a32: opus_int32, b32: opus_int32) -> opus_int32 {
    (((a32 as i64) * ((b32 as i16) as i64)) >> 16) as opus_int32
}

/// `((opus_int32)((a32) + (((b32) * (opus_int64)((opus_int16)(c32))) >> 16)))`
#[inline(always)]
pub fn silk_SMLAWB(a32: opus_int32, b32: opus_int32, c32: opus_int32) -> opus_int32 {
    ((a32 as i64) + (((b32 as i64) * ((c32 as i16) as i64)) >> 16)) as opus_int32
}

/// `((opus_int32)(((a32) * (opus_int64)((b32) >> 16)) >> 16))`
#[inline(always)]
pub fn silk_SMULWT(a32: opus_int32, b32: opus_int32) -> opus_int32 {
    (((a32 as i64) * ((b32 >> 16) as i64)) >> 16) as opus_int32
}

/// `((opus_int32)((a32) + (((b32) * ((opus_int64)(c32) >> 16)) >> 16)))`
#[inline(always)]
pub fn silk_SMLAWT(a32: opus_int32, b32: opus_int32, c32: opus_int32) -> opus_int32 {
    ((a32 as i64) + (((b32 as i64) * ((c32 as i64) >> 16)) >> 16)) as opus_int32
}

/// `((opus_int32)((opus_int16)(a32)) * (opus_int32)((opus_int16)(b32)))`
#[inline(always)]
pub fn silk_SMULBB(a32: opus_int32, b32: opus_int32) -> opus_int32 {
    ((a32 as i16) as i32).wrapping_mul((b32 as i16) as i32)
}

/// `((a32) + ((opus_int32)((opus_int16)(b32))) * (opus_int32)((opus_int16)(c32)))`
#[inline(always)]
pub fn silk_SMLABB(a32: opus_int32, b32: opus_int32, c32: opus_int32) -> opus_int32 {
    a32.wrapping_add(((b32 as i16) as i32).wrapping_mul((c32 as i16) as i32))
}

/// `((opus_int32)((opus_int16)(a32)) * ((b32) >> 16))`
#[inline(always)]
pub fn silk_SMULBT(a32: opus_int32, b32: opus_int32) -> opus_int32 {
    ((a32 as i16) as i32).wrapping_mul(b32 >> 16)
}

/// `((a32) + ((opus_int32)((opus_int16)(b32))) * ((c32) >> 16))`
#[inline(always)]
pub fn silk_SMLABT(a32: opus_int32, b32: opus_int32, c32: opus_int32) -> opus_int32 {
    a32.wrapping_add(((b32 as i16) as i32).wrapping_mul(c32 >> 16))
}

/// `(silk_ADD64((a64), ((opus_int64)(b32) * (opus_int64)(c32))))`
#[inline(always)]
pub fn silk_SMLAL(a64: opus_int64, b32: opus_int32, c32: opus_int32) -> opus_int64 {
    a64.wrapping_add((b32 as i64).wrapping_mul(c32 as i64))
}

/// `((opus_int32)(((opus_int64)(a32) * (b32)) >> 16))`
#[inline(always)]
pub fn silk_SMULWW(a32: opus_int32, b32: opus_int32) -> opus_int32 {
    (((a32 as i64) * (b32 as i64)) >> 16) as opus_int32
}

/// `((opus_int32)((a32) + (((opus_int64)(b32) * (c32)) >> 16)))`
#[inline(always)]
pub fn silk_SMLAWW(a32: opus_int32, b32: opus_int32, c32: opus_int32) -> opus_int32 {
    ((a32 as i64) + (((b32 as i64) * (c32 as i64)) >> 16)) as opus_int32
}

// ---------------------------------------------------------------------------
// Add/subtract with saturation.
//
// The C macros detect the overflow from the signs of the operands and of the
// wrapped result; the Rust transliteration keeps that structure (the
// non-overflowing `(a)+(b)` / `(a)-(b)` become the `wrapping_*` forms).
// ---------------------------------------------------------------------------

/// `silk_ADD_SAT32(a, b)`
#[inline(always)]
pub fn silk_ADD_SAT32(a: opus_int32, b: opus_int32) -> opus_int32 {
    if (a as u32).wrapping_add(b as u32) & 0x8000_0000 == 0 {
        if (a & b) as u32 & 0x8000_0000 != 0 {
            silk_int32_MIN
        } else {
            a.wrapping_add(b)
        }
    } else if (a | b) as u32 & 0x8000_0000 == 0 {
        silk_int32_MAX
    } else {
        a.wrapping_add(b)
    }
}

/// `silk_SUB_SAT32(a, b)`
#[inline(always)]
pub fn silk_SUB_SAT32(a: opus_int32, b: opus_int32) -> opus_int32 {
    if (a as u32).wrapping_sub(b as u32) & 0x8000_0000 == 0 {
        if (a as u32) & ((b as u32) ^ 0x8000_0000) & 0x8000_0000 != 0 {
            silk_int32_MIN
        } else {
            a.wrapping_sub(b)
        }
    } else if ((a as u32) ^ 0x8000_0000) & (b as u32) & 0x8000_0000 != 0 {
        silk_int32_MAX
    } else {
        a.wrapping_sub(b)
    }
}

// ---------------------------------------------------------------------------
// Leading-zero counts.
//
// `EC_ILOG(x) == 32 - __builtin_clz(x)` (EC_CLZ0 == 32 on x86-64), so
// `silk_CLZ32` reduces to `__builtin_clz`, which Rust spells
// `u32::leading_zeros`.
// ---------------------------------------------------------------------------

/// `static OPUS_INLINE opus_int32 silk_CLZ16(opus_int16 in16)`
#[inline(always)]
pub fn silk_CLZ16(in16: opus_int16) -> opus_int32 {
    32 - EC_ILOG(((in16 as i32).wrapping_shl(16) | 0x8000) as u32)
}

/// `static OPUS_INLINE opus_int32 silk_CLZ32(opus_int32 in32)`
#[inline(always)]
pub fn silk_CLZ32(in32: opus_int32) -> opus_int32 {
    if in32 != 0 {
        32 - EC_ILOG(in32 as u32)
    } else {
        32
    }
}

// ---------------------------------------------------------------------------
// Matrix access.
//
// The C macros return an lvalue; the Rust helpers return (or take) the element
// at the same flat index `row*N + column`.
// ---------------------------------------------------------------------------

/// `matrix_ptr(Matrix_base_adr, row, column, N)`
#[inline(always)]
pub fn matrix_ptr<T: Copy>(Matrix_base_adr: &[T], row: opus_int32, column: opus_int32, N: opus_int32) -> T {
    Matrix_base_adr[row.wrapping_mul(N).wrapping_add(column) as usize]
}

/// `matrix_adr(Matrix_base_adr, row, column, N)`
#[inline(always)]
pub fn matrix_adr<T>(Matrix_base_adr: &[T], row: opus_int32, column: opus_int32, N: opus_int32) -> &T {
    &Matrix_base_adr[row.wrapping_mul(N).wrapping_add(column) as usize]
}

/// The write-through counterpart of [`matrix_ptr`]; the C macro is an lvalue,
/// which Rust cannot express, so writes go through this helper.
#[inline(always)]
pub fn matrix_ptr_mut<T>(Matrix_base_adr: &mut [T], row: opus_int32, column: opus_int32, N: opus_int32) -> &mut T {
    let i = row.wrapping_mul(N).wrapping_add(column) as usize;
    &mut Matrix_base_adr[i]
}

/// The write-through counterpart of [`matrix_adr`].
#[inline(always)]
pub fn matrix_adr_mut<T>(Matrix_base_adr: &mut [T], row: opus_int32, column: opus_int32, N: opus_int32) -> &mut T {
    let i = row.wrapping_mul(N).wrapping_add(column) as usize;
    &mut Matrix_base_adr[i]
}

/// The write-through counterpart of [`matrix_c_ptr`].
#[inline(always)]
pub fn matrix_c_ptr_mut<T>(Matrix_base_adr: &mut [T], row: opus_int32, column: opus_int32, M: opus_int32) -> &mut T {
    let i = row.wrapping_add(M.wrapping_mul(column)) as usize;
    &mut Matrix_base_adr[i]
}

/// `matrix_c_ptr(Matrix_base_adr, row, column, M)` — column based.
#[inline(always)]
pub fn matrix_c_ptr<T: Copy>(Matrix_base_adr: &[T], row: opus_int32, column: opus_int32, M: opus_int32) -> T {
    Matrix_base_adr[row.wrapping_add(M.wrapping_mul(column)) as usize]
}

// `silk_DWORD_ALIGN` is empty on anything but EMBEDDED_ARM, so it is dropped
// here: `#[repr(C)]` already gives the fields their natural x86 alignment.
// `silk_memcpy` / `silk_memset` / `silk_memmove` map onto safe slice
// operations at their call sites.

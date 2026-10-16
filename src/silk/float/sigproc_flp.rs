//! Mirror of `silk/float/SigProc_FLP.h` from libopus 1.6.1, transliterated to safe Rust.
//!
//! `silk_float` is the same type as `crate::silk::typedef::silk_float`; `silk_DWORD_ALIGN` is empty on x86 and therefore dropped.  Only the inline helpers are ported here — the `silk_*_FLP` declarations are not needed in Rust.
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

pub use crate::silk::typedef::silk_float;

use crate::celt::arch::float2int;
use crate::silk::sigproc_fix::*;
use crate::types::*;

/// `#define PI (3.1415926536f)`
pub const PI: silk_float = 3.1415926536;

/// `silk_min_float(a, b)`
#[inline(always)]
pub fn silk_min_float(a: silk_float, b: silk_float) -> silk_float {
    if a < b {
        a
    } else {
        b
    }
}

/// `silk_max_float(a, b)`
#[inline(always)]
pub fn silk_max_float(a: silk_float, b: silk_float) -> silk_float {
    if a > b {
        a
    } else {
        b
    }
}

/// `silk_abs_float(a)` — `(silk_float)fabs(a)`.
#[inline(always)]
pub fn silk_abs_float(a: silk_float) -> silk_float {
    a.abs()
}

/// `silk_sigmoid(x)` — `(silk_float)(1.0 / (1.0 + exp(-x)))`.
#[inline(always)]
pub fn silk_sigmoid(x: silk_float) -> silk_float {
    (1.0f64 / (1.0f64 + (-(x as f64)).exp())) as silk_float
}

/// `silk_float2int(x)` — `(opus_int32)float2int(x)`.
#[inline(always)]
pub fn silk_float2int(x: silk_float) -> opus_int32 {
    float2int(x)
}

/// `silk_float2short_array(out, in, length)`
#[inline(always)]
pub fn silk_float2short_array(out: &mut [opus_int16], input: &[silk_float], length: opus_int32) {
    let mut k = length - 1;
    while k >= 0 {
        out[k as usize] = silk_SAT16(float2int(input[k as usize])) as opus_int16;
        k -= 1;
    }
}

/// `silk_short2float_array(out, in, length)`
#[inline(always)]
pub fn silk_short2float_array(out: &mut [silk_float], input: &[opus_int16], length: opus_int32) {
    let mut k = length - 1;
    while k >= 0 {
        out[k as usize] = input[k as usize] as silk_float;
        k -= 1;
    }
}

/// `silk_log2(x)` — `(silk_float)(3.32192809488736 * log10(x))`.
#[inline(always)]
pub fn silk_log2(x: f64) -> silk_float {
    (3.32192809488736 * x.log10()) as silk_float
}

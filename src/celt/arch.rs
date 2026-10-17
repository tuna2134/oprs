//! Mirror of `celt/arch.h` for the **float** configuration
//! (`FIXED_POINT` undefined, `FLOAT_APPROX` undefined, `DISABLE_FLOAT_API`
//! undefined), which is the configuration the reference `libopus` shared
//! library is built with.
//!
//! In this configuration CELT's fixed-point macros are identities, but they are
//! kept here as `#[inline(always)]` functions with the *same names as the C
//! macros* so that transliterated code can be diffed against the reference line
//! by line.  Every identity function documents the C definition it replaces.

use crate::types::*;

/// `opus_val16` in a float build.
pub type opus_val16 = f32;
/// `opus_val32` in a float build.
pub type opus_val32 = f32;
/// `opus_val64` in a float build.
pub type opus_val64 = f32;

pub type celt_sig = f32;
pub type celt_norm = f32;
pub type celt_ener = f32;
pub type celt_glog = f32;
pub type opus_res = f32;
pub type celt_coef = f32;

/// `CELT_SIG_SCALE`
pub const CELT_SIG_SCALE: f32 = 32768.0;

pub const Q15ONE: f32 = 1.0;
pub const Q31ONE: f32 = 1.0;
pub const COEF_ONE: f32 = 1.0;
pub const NORM_SCALING: f32 = 1.0;
pub const EPSILON: f32 = 1e-15;
pub const VERY_SMALL: f32 = 1e-30;
pub const VERY_LARGE16: f32 = 1e15;
pub const Q15_ONE: f32 = 1.0;
pub const SIG_SHIFT: i32 = 12;
pub const NORM_SHIFT: i32 = 24;
pub const MAX_ENCODING_DEPTH: i32 = 24;

/// Assertions are compiled out unless the *Rust* build enables debug
/// assertions; the reference library is built without `ENABLE_ASSERTIONS`, so
/// release behaviour matches the C release behaviour (no check at all).
macro_rules! celt_assert {
    ($($arg:tt)*) => { debug_assert!($($arg)*) };
}
pub(crate) use celt_assert;

macro_rules! celt_assert2 {
    ($cond:expr, $msg:expr) => { debug_assert!($cond, "{}", $msg) };
}
pub(crate) use celt_assert2;

macro_rules! celt_sig_assert {
    ($($arg:tt)*) => { debug_assert!($($arg)*) };
}
pub(crate) use celt_sig_assert;

// ---------------------------------------------------------------------------
// Type-generic min/max, mirroring the C macros.
// ---------------------------------------------------------------------------

#[inline(always)]
pub fn MIN16<T: PartialOrd>(a: T, b: T) -> T {
    if a < b {
        a
    } else {
        b
    }
}
#[inline(always)]
pub fn MAX16<T: PartialOrd>(a: T, b: T) -> T {
    if a > b {
        a
    } else {
        b
    }
}
#[inline(always)]
pub fn MIN32<T: PartialOrd>(a: T, b: T) -> T {
    if a < b {
        a
    } else {
        b
    }
}
#[inline(always)]
pub fn MAX32<T: PartialOrd>(a: T, b: T) -> T {
    if a > b {
        a
    } else {
        b
    }
}
#[inline(always)]
pub fn IMIN<T: PartialOrd>(a: T, b: T) -> T {
    if a < b {
        a
    } else {
        b
    }
}
#[inline(always)]
pub fn IMAX<T: PartialOrd>(a: T, b: T) -> T {
    if a > b {
        a
    } else {
        b
    }
}
#[inline(always)]
pub fn FMIN(a: f32, b: f32) -> f32 {
    if a < b {
        a
    } else {
        b
    }
}
#[inline(always)]
pub fn FMAX(a: f32, b: f32) -> f32 {
    if a > b {
        a
    } else {
        b
    }
}

#[inline(always)]
pub fn ABS16(x: f32) -> f32 {
    x.abs()
}
#[inline(always)]
pub fn ABS32(x: f32) -> f32 {
    x.abs()
}

// ---------------------------------------------------------------------------
// Float-configuration identities (see the `#else /* FIXED_POINT */` branch of
// celt/arch.h).
// ---------------------------------------------------------------------------

#[inline(always)]
pub fn GCONST(x: f32) -> f32 {
    x
}
#[inline(always)]
pub fn QCONST16(x: f32, _bits: i32) -> f32 {
    x
}
#[inline(always)]
pub fn QCONST32(x: f32, _bits: i32) -> f32 {
    x
}

#[inline(always)]
pub fn NEG16(x: f32) -> f32 {
    -x
}
#[inline(always)]
pub fn NEG32(x: f32) -> f32 {
    -x
}
#[inline(always)]
pub fn NEG32_ovflw(x: f32) -> f32 {
    -x
}
#[inline(always)]
pub fn EXTRACT16(x: f32) -> f32 {
    x
}
#[inline(always)]
pub fn EXTEND32(x: f32) -> f32 {
    x
}
#[inline(always)]
pub fn SHR16(a: f32, _shift: i32) -> f32 {
    a
}
#[inline(always)]
pub fn SHL16(a: f32, _shift: i32) -> f32 {
    a
}
#[inline(always)]
pub fn SHR32(a: f32, _shift: i32) -> f32 {
    a
}
#[inline(always)]
pub fn SHL32(a: f32, _shift: i32) -> f32 {
    a
}
#[inline(always)]
pub fn PSHR32(a: f32, _shift: i32) -> f32 {
    a
}
#[inline(always)]
pub fn VSHR32(a: f32, _shift: i32) -> f32 {
    a
}
#[inline(always)]
pub fn SHR64(a: f32, _shift: i32) -> f32 {
    a
}
#[inline(always)]
pub fn PSHR(a: f32, _shift: i32) -> f32 {
    a
}
#[inline(always)]
pub fn SHR(a: f32, _shift: i32) -> f32 {
    a
}
#[inline(always)]
pub fn SHL(a: f32, _shift: i32) -> f32 {
    a
}
#[inline(always)]
pub fn SATURATE(x: f32, _a: f32) -> f32 {
    x
}
#[inline(always)]
pub fn SATURATE16(x: f32) -> f32 {
    x
}
#[inline(always)]
pub fn ROUND16(a: f32, _shift: i32) -> f32 {
    a
}
#[inline(always)]
pub fn SROUND16(a: f32, _shift: i32) -> f32 {
    a
}
#[inline(always)]
pub fn HALF16(x: f32) -> f32 {
    0.5 * x
}
#[inline(always)]
pub fn HALF32(x: f32) -> f32 {
    0.5 * x
}

#[inline(always)]
pub fn ADD16(a: f32, b: f32) -> f32 {
    a + b
}
#[inline(always)]
pub fn SUB16(a: f32, b: f32) -> f32 {
    a - b
}
#[inline(always)]
pub fn ADD32(a: f32, b: f32) -> f32 {
    a + b
}
#[inline(always)]
pub fn SUB32(a: f32, b: f32) -> f32 {
    a - b
}
#[inline(always)]
pub fn ADD32_ovflw(a: f32, b: f32) -> f32 {
    a + b
}
#[inline(always)]
pub fn SUB32_ovflw(a: f32, b: f32) -> f32 {
    a - b
}
#[inline(always)]
pub fn SHL32_ovflw(a: f32, _shift: i32) -> f32 {
    a
}
#[inline(always)]
pub fn PSHR32_ovflw(a: f32, _shift: i32) -> f32 {
    a
}

#[inline(always)]
pub fn MULT16_16_16(a: f32, b: f32) -> f32 {
    a * b
}
#[inline(always)]
pub fn MULT16_16(a: f32, b: f32) -> f32 {
    a * b
}
#[inline(always)]
pub fn MAC16_16(c: f32, a: f32, b: f32) -> f32 {
    c + a * b
}

#[inline(always)]
pub fn MULT16_32_Q15(a: f32, b: f32) -> f32 {
    a * b
}
#[inline(always)]
pub fn MULT16_32_Q16(a: f32, b: f32) -> f32 {
    a * b
}
#[inline(always)]
pub fn MULT32_32_Q16(a: f32, b: f32) -> f32 {
    a * b
}
#[inline(always)]
pub fn MULT32_32_Q31(a: f32, b: f32) -> f32 {
    a * b
}
#[inline(always)]
pub fn MULT32_32_P31(a: f32, b: f32) -> f32 {
    a * b
}
#[inline(always)]
pub fn MULT32_32_P31_ovflw(a: f32, b: f32) -> f32 {
    a * b
}
#[inline(always)]
pub fn MAC16_32_Q15(c: f32, a: f32, b: f32) -> f32 {
    c + a * b
}
#[inline(always)]
pub fn MAC16_32_Q16(c: f32, a: f32, b: f32) -> f32 {
    c + a * b
}
#[inline(always)]
pub fn MAC_COEF_32_ARM(c: f32, a: f32, b: f32) -> f32 {
    c + a * b
}

#[inline(always)]
pub fn MULT16_16_Q11_32(a: f32, b: f32) -> f32 {
    a * b
}
#[inline(always)]
pub fn MULT16_16_Q11(a: f32, b: f32) -> f32 {
    a * b
}
#[inline(always)]
pub fn MULT16_16_Q13(a: f32, b: f32) -> f32 {
    a * b
}
#[inline(always)]
pub fn MULT16_16_Q14(a: f32, b: f32) -> f32 {
    a * b
}
#[inline(always)]
pub fn MULT16_16_Q15(a: f32, b: f32) -> f32 {
    a * b
}
#[inline(always)]
pub fn MULT16_16_P15(a: f32, b: f32) -> f32 {
    a * b
}
#[inline(always)]
pub fn MULT16_16_P13(a: f32, b: f32) -> f32 {
    a * b
}
#[inline(always)]
pub fn MULT16_16_P14(a: f32, b: f32) -> f32 {
    a * b
}
#[inline(always)]
pub fn MULT16_32_P16(a: f32, b: f32) -> f32 {
    a * b
}

#[inline(always)]
pub fn MULT_COEF_32(a: f32, b: f32) -> f32 {
    a * b
}
#[inline(always)]
pub fn MULT_COEF(a: f32, b: f32) -> f32 {
    a * b
}
#[inline(always)]
pub fn MULT_COEF_TAPS(a: f32, b: f32) -> f32 {
    a * b
}
#[inline(always)]
pub fn COEF2VAL16(x: f32) -> f32 {
    x
}

#[inline(always)]
pub fn DIV32_16(a: f32, b: f32) -> f32 {
    a / b
}
#[inline(always)]
pub fn DIV32(a: f32, b: f32) -> f32 {
    a / b
}

#[inline(always)]
pub fn SIG2RES(a: f32) -> f32 {
    (1.0 / CELT_SIG_SCALE) * a
}
#[inline(always)]
pub fn RES2SIG(a: f32) -> f32 {
    CELT_SIG_SCALE * a
}
#[inline(always)]
pub fn RES2FLOAT(a: f32) -> f32 {
    a
}
#[inline(always)]
pub fn INT16TORES(a: i16) -> f32 {
    (a as f32) * (1.0 / CELT_SIG_SCALE)
}
#[inline(always)]
pub fn INT24TORES(a: i32) -> f32 {
    (1.0 / 32768.0 / 256.0) * (a as f32)
}
#[inline(always)]
pub fn ADD_RES(a: f32, b: f32) -> f32 {
    ADD32(a, b)
}
#[inline(always)]
pub fn FLOAT2RES(a: f32) -> f32 {
    a
}
#[inline(always)]
pub fn RES2VAL16(a: f32) -> f32 {
    a
}
#[inline(always)]
pub fn FLOAT2SIG(a: f32) -> f32 {
    a * CELT_SIG_SCALE
}
#[inline(always)]
pub fn INT16TOSIG(a: i16) -> f32 {
    a as f32
}
#[inline(always)]
pub fn INT24TOSIG(a: i32) -> f32 {
    (a as f32) * (1.0 / 256.0)
}
#[inline(always)]
pub fn MULT16_RES_Q15(a: f32, b: f32) -> f32 {
    MULT16_16_Q15(a, b)
}

/// `celt/float_cast.h:68`: with GCC and SSE the reference builds `float2int()`
/// as `_mm_cvt_ss2si(_mm_set_ss(x))` — one `cvtss2si`, which rounds according
/// to `MXCSR` (ties-to-even by default, i.e. `rint` semantics) and yields
/// `INT_MIN` for NaN and for values outside the `opus_int32` range.
///
/// The portable branch below is the identical function written with
/// `round_ties_even()`; on the `x86-64` baseline LLVM does *not* fold that
/// into `cvtss2si`, so it becomes an out-of-line libm `rintf` call that shows
/// up on the decode path (~5 % of a 48 kHz stereo frame and of an 8 kHz SILK
/// frame).  `sse` is part of the x86-64 baseline, so the intrinsic is
/// unconditionally available there and emits the single instruction the C does.
#[inline(always)]
pub fn float2int(x: f32) -> i32 {
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: SSE (hence `cvtss2si`) is part of the x86-64 baseline that
        // this crate is compiled for.
        unsafe { core::arch::x86_64::_mm_cvtss_si32(core::arch::x86_64::_mm_set_ss(x)) }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        if x.is_nan() {
            i32::MIN
        } else if x >= 2147483648.0 {
            i32::MIN
        } else if x < -2147483648.0 {
            i32::MIN
        } else {
            x.round_ties_even() as i32
        }
    }
}

#[inline(always)]
pub fn FLOAT2INT16(x: f32) -> i16 {
    let mut x = x * CELT_SIG_SCALE;
    x = MAX32(x, -32768.0);
    x = MIN32(x, 32767.0);
    float2int(x) as i16
}

#[inline(always)]
pub fn FLOAT2INT24(x: f32) -> i32 {
    let mut x = x * (CELT_SIG_SCALE * 256.0);
    x = MAX32(x, -16777216.0);
    x = MIN32(x, 16777216.0);
    float2int(x)
}

#[inline(always)]
pub fn RES2INT16(a: f32) -> i16 {
    FLOAT2INT16(a)
}

#[inline(always)]
pub fn RES2INT24(a: f32) -> i32 {
    float2int(32768.0 * 256.0 * a)
}

/// `SAT16`
#[inline(always)]
pub fn SAT16(x: i32) -> i16 {
    if x > 32767 {
        32767
    } else if x < -32768 {
        -32768
    } else {
        x as i16
    }
}

/// `celt_isnan` (`(x)!=(x)` when `FLOAT_APPROX` is undefined)
#[inline(always)]
pub fn celt_isnan(x: f32) -> bool {
    x != x
}

/// `IMUL32(a,b)` — 32-bit wrapping multiply.
#[inline(always)]
pub fn IMUL32(a: u32, b: u32) -> u32 {
    a.wrapping_mul(b)
}

//! Mirror of `silk/typedef.h` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Fixed-width and SILK-specific type and limit definitions.
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

use crate::types::*;

// ---------------------------------------------------------------------------
// Types from `include/opus_types.h` that `crate::types` does not mirror.
// ---------------------------------------------------------------------------

/// `#define opus_int         int` — used for counters etc.; at least 16 bits.
pub type opus_int = i32;
/// `#define opus_uint        unsigned int` — used for counters etc.
pub type opus_uint = u32;

/// `# define silk_float float` (the `#ifndef FIXED_POINT` branch).
pub type silk_float = f32;
/// `# define silk_float_MAX FLT_MAX`
pub const silk_float_MAX: silk_float = f32::MAX;

// ---------------------------------------------------------------------------
// Limits.
// ---------------------------------------------------------------------------

/// `((opus_int64)0x7FFFFFFFFFFFFFFFLL)`  /* 2^63 - 1 */
pub const silk_int64_MAX: opus_int64 = i64::MAX;
/// `((opus_int64)0x8000000000000000LL)`  /* -2^63 */
pub const silk_int64_MIN: opus_int64 = i64::MIN;
/// `0x7FFFFFFF`  /* 2^31 - 1 */
pub const silk_int32_MAX: opus_int32 = i32::MAX;
/// `((opus_int32)0x80000000)`  /* -2^31 */
pub const silk_int32_MIN: opus_int32 = i32::MIN;
/// `0x7FFF`  /* 2^15 - 1 */
pub const silk_int16_MAX: opus_int16 = i16::MAX;
/// `((opus_int16)0x8000)`  /* -2^15 */
pub const silk_int16_MIN: opus_int16 = i16::MIN;
/// `0x7F`  /* 2^7 - 1 */
pub const silk_int8_MAX: opus_int8 = i8::MAX;
/// `((opus_int8)0x80)`  /* -2^7 */
pub const silk_int8_MIN: opus_int8 = i8::MIN;
/// `0xFF`  /* 2^8 - 1 */
pub const silk_uint8_MAX: opus_uint8 = u8::MAX;

/// `silk_TRUE`
pub const silk_TRUE: opus_int = 1;
/// `silk_FALSE`
pub const silk_FALSE: opus_int = 0;

// `silk_assert(COND)` is `((void)(COND))` when `ENABLE_ASSERTIONS` is
// undefined, which is the reference configuration.  Like `celt_assert!` it is
// mapped onto `debug_assert!`, so it can only ever fire in a Rust debug build
// (the probes always build release).  `silk_fatal()` only exists in the
// `ENABLE_ASSERTIONS` build, which is not the configuration being ported.
macro_rules! silk_assert {
    ($($arg:tt)*) => { debug_assert!($($arg)*) };
}
pub(crate) use silk_assert;

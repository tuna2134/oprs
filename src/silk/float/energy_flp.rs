//! Mirror of `silk/float/energy_FLP.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Sum of squares of a `silk_float` array, with the result as a `double`.
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

use crate::silk::float::sigproc_flp::silk_float;
use crate::silk::typedef::*;

/// sum of squares of a `silk_float` array, with result as double
pub fn silk_energy_FLP(data: &[silk_float], dataSize: opus_int) -> f64 {
    let mut i: opus_int;
    let mut result: f64;

    /* 4x unrolled loop */
    result = 0.0;
    i = 0;
    while i < dataSize - 3 {
        result += data[i as usize] as f64 * data[i as usize] as f64
            + data[(i + 1) as usize] as f64 * data[(i + 1) as usize] as f64
            + data[(i + 2) as usize] as f64 * data[(i + 2) as usize] as f64
            + data[(i + 3) as usize] as f64 * data[(i + 3) as usize] as f64;
        i += 4;
    }

    /* add any remaining products */
    while i < dataSize {
        result += data[i as usize] as f64 * data[i as usize] as f64;
        i += 1;
    }

    silk_assert!(result >= 0.0);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_single() {
        assert_eq!(silk_energy_FLP(&[], 0), 0.0);
        assert_eq!(silk_energy_FLP(&[3.0], 1), 9.0);
    }

    #[test]
    fn matches_left_to_right_sum() {
        let v: [f32; 9] = [1.0, -2.0, 3.0, 4.0, -0.5, 0.25, 8.0, 16.0, 32.0];
        let mut expect = 0.0f64;
        let mut i = 0;
        while i < 9 {
            expect += v[i] as f64 * v[i] as f64;
            i += 1;
        }
        assert_eq!(silk_energy_FLP(&v, 9), expect);
    }
}

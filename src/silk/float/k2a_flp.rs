//! Mirror of `silk/float/k2a_FLP.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Step-up function: reflection coefficients to prediction coefficients.
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
use crate::types::*;

/// step up function, converts reflection coefficients to prediction coefficients
pub fn silk_k2a_FLP(A: &mut [silk_float], rc: &[silk_float], order: opus_int32) {
    let mut k: opus_int;
    let mut n: opus_int;

    k = 0;
    while k < order {
        let rck = rc[k as usize];
        n = 0;
        while n < (k + 1) >> 1 {
            let tmp1 = A[n as usize];
            let tmp2 = A[(k - n - 1) as usize];
            A[n as usize] = tmp1 + tmp2 * rck;
            A[(k - n - 1) as usize] = tmp2 + tmp1 * rck;
            n += 1;
        }
        A[k as usize] = -rck;
        k += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_order_is_negated_reflection_coefficient() {
        let mut a = [0.0f32; 4];
        let rc = [0.25f32, 0.0, 0.0, 0.0];
        silk_k2a_FLP(&mut a, &rc, 4);
        assert_eq!(a[0], -0.25);
        assert_eq!(a[1], 0.0);
    }
}

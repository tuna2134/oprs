//! Mirror of `silk/NLSF_VQ_weights_laroia.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! R. Laroia, N. Phamdo and N. Farvardin, "Robust and Efficient Quantization
//! of Speech LSP Parameters Using Structured Vector Quantization", Proc. IEEE
//! Int. Conf. Acoust., Speech, Signal Processing, pp. 641-644, 1991.
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

use crate::celt::arch::celt_assert;
use crate::silk::define::*;
use crate::silk::sigproc_fix::*;
use crate::silk::typedef::silk_assert;
use crate::silk::typedef::*;
use crate::types::*;

/* Laroia low complexity NLSF weights */
pub fn silk_NLSF_VQ_weights_laroia(
    pNLSFW_Q_OUT: &mut [opus_int16], /* O     Pointer to input vector weights [D]                        */
    pNLSF_Q15: &[opus_int16],        /* I     Pointer to input vector         [D]                        */
    D: opus_int,                     /* I     Input vector dimension (even)                              */
) {
    celt_assert!(D > 0);
    celt_assert!((D & 1) == 0);

    /* First value */
    let mut tmp1_int = silk_max_int(pNLSF_Q15[0] as opus_int32, 1);
    tmp1_int = silk_DIV32_16(1i32 << (15 + NLSF_W_Q), tmp1_int);
    let mut tmp2_int = silk_max_int(pNLSF_Q15[1] as opus_int32 - pNLSF_Q15[0] as opus_int32, 1);
    tmp2_int = silk_DIV32_16(1i32 << (15 + NLSF_W_Q), tmp2_int);
    pNLSFW_Q_OUT[0] = silk_min_int(tmp1_int + tmp2_int, silk_int16_MAX as opus_int32) as opus_int16;
    silk_assert!(pNLSFW_Q_OUT[0] > 0);

    /* Main loop */
    let mut k: opus_int = 1;
    while k < D - 1 {
        tmp1_int = silk_max_int(
            pNLSF_Q15[(k + 1) as usize] as opus_int32 - pNLSF_Q15[k as usize] as opus_int32,
            1,
        );
        tmp1_int = silk_DIV32_16(1i32 << (15 + NLSF_W_Q), tmp1_int);
        pNLSFW_Q_OUT[k as usize] =
            silk_min_int(tmp1_int + tmp2_int, silk_int16_MAX as opus_int32) as opus_int16;
        silk_assert!(pNLSFW_Q_OUT[k as usize] > 0);

        tmp2_int = silk_max_int(
            pNLSF_Q15[(k + 2) as usize] as opus_int32 - pNLSF_Q15[(k + 1) as usize] as opus_int32,
            1,
        );
        tmp2_int = silk_DIV32_16(1i32 << (15 + NLSF_W_Q), tmp2_int);
        pNLSFW_Q_OUT[(k + 1) as usize] =
            silk_min_int(tmp1_int + tmp2_int, silk_int16_MAX as opus_int32) as opus_int16;
        silk_assert!(pNLSFW_Q_OUT[(k + 1) as usize] > 0);

        k += 2;
    }

    /* Last value */
    tmp1_int = silk_max_int((1 << 15) - pNLSF_Q15[(D - 1) as usize] as opus_int32, 1);
    tmp1_int = silk_DIV32_16(1i32 << (15 + NLSF_W_Q), tmp1_int);
    pNLSFW_Q_OUT[(D - 1) as usize] =
        silk_min_int(tmp1_int + tmp2_int, silk_int16_MAX as opus_int32) as opus_int16;
    silk_assert!(pNLSFW_Q_OUT[(D - 1) as usize] > 0);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Weights are strictly positive and the two halves of the loop are
    /// symmetric around the input gaps.
    #[test]
    fn weights_positive_and_symmetric() {
        let nlsf: Vec<i16> = vec![2000, 6000, 9000, 15000, 18000, 24000];
        let mut w = vec![0i16; 6];
        silk_NLSF_VQ_weights_laroia(&mut w, &nlsf, 6);
        for &x in w.iter() {
            assert!(x > 0);
        }
        /* w[k] = 2^17 / gap(k, k+1) + 2^17 / gap(k-1, k) */
        assert_eq!(
            w[2],
            ((1i32 << 17) / (15000 - 9000) + (1i32 << 17) / (9000 - 6000)) as i16
        );
    }

    /// Out-of-range but monotone inputs still produce saturating weights.
    #[test]
    fn weights_saturate() {
        let nlsf = [1i16, 1, 1, 1];
        let mut w = [0i16; 4];
        silk_NLSF_VQ_weights_laroia(&mut w, &nlsf, 4);
        for &x in w.iter() {
            assert_eq!(x, i16::MAX);
        }
    }
}

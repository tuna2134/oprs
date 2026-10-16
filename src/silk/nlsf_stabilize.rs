//! Mirror of `silk/NLSF_stabilize.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! NLSF stabilizer:
//!
//! - Moves NLSFs further apart if they are too close
//! - Moves NLSFs away from borders if they are too close
//! - High effort to achieve a modification with minimum Euclidean distance to input vector
//! - Output are sorted NLSF coefficients
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

use crate::silk::sigproc_fix::*;
use crate::silk::sort::*;
use crate::silk::typedef::*;
use crate::types::*;

/* Constant Definitions */
const MAX_LOOPS: opus_int = 20;

/// `silk_NLSF_stabilize(NLSF_Q15, NDeltaMin_Q15, L)`
pub fn silk_NLSF_stabilize(
    NLSF_Q15: &mut [opus_int16],
    NDeltaMin_Q15: &[opus_int16],
    L: opus_int,
) {
    let mut i: opus_int;
    let mut I: opus_int = 0;
    let mut k: opus_int;
    let mut loops: opus_int;
    let mut center_freq_Q15: opus_int16;
    let mut diff_Q15: opus_int32;
    let mut min_diff_Q15: opus_int32;
    let mut min_center_Q15: opus_int32;
    let mut max_center_Q15: opus_int32;

    /* This is necessary to ensure an output within range of a opus_int16 */
    silk_assert!(NDeltaMin_Q15[L as usize] >= 1);

    loops = 0;
    while loops < MAX_LOOPS {
        /**************************/
        /* Find smallest distance */
        /**************************/
        /* First element */
        min_diff_Q15 = NLSF_Q15[0] as opus_int32 - NDeltaMin_Q15[0] as opus_int32;
        I = 0;
        /* Middle elements */
        for i in 1..=L - 1 {
            diff_Q15 = NLSF_Q15[i as usize] as opus_int32
                - (NLSF_Q15[(i - 1) as usize] as opus_int32 + NDeltaMin_Q15[i as usize] as opus_int32);
            if diff_Q15 < min_diff_Q15 {
                min_diff_Q15 = diff_Q15;
                I = i;
            }
        }
        /* Last element */
        diff_Q15 = (1 << 15)
            - (NLSF_Q15[(L - 1) as usize] as opus_int32 + NDeltaMin_Q15[L as usize] as opus_int32);
        if diff_Q15 < min_diff_Q15 {
            min_diff_Q15 = diff_Q15;
            I = L;
        }

        /***************************************************/
        /* Now check if the smallest distance non-negative */
        /***************************************************/
        if min_diff_Q15 >= 0 {
            return;
        }

        if I == 0 {
            /* Move away from lower limit */
            NLSF_Q15[0] = NDeltaMin_Q15[0];
        } else if I == L {
            /* Move away from higher limit */
            NLSF_Q15[(L - 1) as usize] = ((1 << 15) - NDeltaMin_Q15[L as usize] as opus_int32) as opus_int16;
        } else {
            /* Find the lower extreme for the location of the current center frequency */
            min_center_Q15 = 0;
            for k in 0..I {
                min_center_Q15 += NDeltaMin_Q15[k as usize] as opus_int32;
            }
            min_center_Q15 += silk_RSHIFT(NDeltaMin_Q15[I as usize] as opus_int32, 1);

            /* Find the upper extreme for the location of the current center frequency */
            max_center_Q15 = 1 << 15;
            k = L;
            while k > I {
                max_center_Q15 -= NDeltaMin_Q15[k as usize] as opus_int32;
                k -= 1;
            }
            max_center_Q15 -= silk_RSHIFT(NDeltaMin_Q15[I as usize] as opus_int32, 1);

            /* Move apart, sorted by value, keeping the same center frequency */
            center_freq_Q15 = silk_LIMIT(
                silk_RSHIFT_ROUND(
                    NLSF_Q15[(I - 1) as usize] as opus_int32
                        + NLSF_Q15[I as usize] as opus_int32,
                    1,
                ),
                min_center_Q15,
                max_center_Q15,
            ) as opus_int16;
            NLSF_Q15[(I - 1) as usize] =
                (center_freq_Q15 as opus_int32 - silk_RSHIFT(NDeltaMin_Q15[I as usize] as opus_int32, 1))
                    as opus_int16;
            NLSF_Q15[I as usize] = (NLSF_Q15[(I - 1) as usize] as opus_int32
                + NDeltaMin_Q15[I as usize] as opus_int32)
                as opus_int16;
        }
        loops += 1;
    }

    /* Safe and simple fall back method, which is less ideal than the above */
    if loops == MAX_LOOPS {
        /* Insertion sort (fast for already almost sorted arrays):   */
        /* Best case:  O(n)   for an already sorted array            */
        /* Worst case: O(n^2) for an inversely sorted array          */
        silk_insertion_sort_increasing_all_values_int16(&mut NLSF_Q15[0..], L);

        /* First NLSF should be no less than NDeltaMin[0] */
        NLSF_Q15[0] = silk_max_int(NLSF_Q15[0] as opus_int, NDeltaMin_Q15[0] as opus_int) as opus_int16;

        /* Keep delta_min distance between the NLSFs */
        for i in 1..L {
            NLSF_Q15[i as usize] = silk_max_int(
                NLSF_Q15[i as usize] as opus_int,
                silk_ADD_SAT16(NLSF_Q15[(i - 1) as usize], NDeltaMin_Q15[i as usize]) as opus_int,
            ) as opus_int16;
        }

        /* Last NLSF should be no higher than 1 - NDeltaMin[L] */
        NLSF_Q15[(L - 1) as usize] = silk_min_int(
            NLSF_Q15[(L - 1) as usize] as opus_int,
            (1 << 15) - NDeltaMin_Q15[L as usize] as opus_int,
        ) as opus_int16;

        /* Keep NDeltaMin distance between the NLSFs */
        i = L - 2;
        while i >= 0 {
            NLSF_Q15[i as usize] = silk_min_int(
                NLSF_Q15[i as usize] as opus_int,
                NLSF_Q15[(i + 1) as usize] as opus_int - NDeltaMin_Q15[(i + 1) as usize] as opus_int,
            ) as opus_int16;
            i -= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::silk::structs::silk_NLSF_CB_struct;
    use crate::silk::tables_nlsf_cb_nb_mb::silk_NLSF_CB_NB_MB;
    use crate::silk::tables_nlsf_cb_wb::silk_NLSF_CB_WB;

    fn check(cb: &silk_NLSF_CB_struct, nlsf_in: &[i16; 16]) {
        let order = cb.order as usize;
        let mut nlsf = *nlsf_in;
        silk_NLSF_stabilize(&mut nlsf, cb.deltaMin_Q15, order as i32);
        assert!(nlsf[0] >= cb.deltaMin_Q15[0]);
        for i in 1..order {
            assert!((nlsf[i] as i32) >= nlsf[i - 1] as i32 + cb.deltaMin_Q15[i] as i32);
        }
        assert!((nlsf[order - 1] as i32) <= 32768 - cb.deltaMin_Q15[order] as i32);
    }

    #[test]
    fn separates_equal_values() {
        let nlsf = [1000i16; 16];
        check(&silk_NLSF_CB_NB_MB, &nlsf);
        let nlsf = [0i16; 16];
        check(&silk_NLSF_CB_NB_MB, &nlsf);
    }

    #[test]
    fn handles_inverted_input() {
        let mut nlsf = [0i16; 16];
        for (i, v) in nlsf.iter_mut().enumerate() {
            *v = (30000 - 1900 * i as i32) as i16;
        }
        check(&silk_NLSF_CB_WB, &nlsf);
    }
}

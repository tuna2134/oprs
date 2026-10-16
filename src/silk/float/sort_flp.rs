//! Mirror of `silk/float/sort_FLP.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Insertion sort (decreasing), fast for already almost sorted arrays.
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
use crate::silk::float::sigproc_flp::silk_float;
use crate::silk::typedef::*;

/// Insertion sort, decreasing order.
///
/// `idx` receives the index vector of the sorted elements; only the first `K`
/// entries are guaranteed to be correct.
pub fn silk_insertion_sort_decreasing_FLP(
    a: &mut [silk_float],
    idx: &mut [opus_int],
    L: opus_int,
    K: opus_int,
) {
    let mut value: silk_float;
    let mut i: opus_int;
    let mut j: opus_int;

    /* Safety checks */
    celt_assert!(K > 0);
    celt_assert!(L > 0);
    celt_assert!(L >= K);

    /* Write start indices in index vector */
    i = 0;
    while i < K {
        idx[i as usize] = i;
        i += 1;
    }

    /* Sort vector elements by value, decreasing order */
    i = 1;
    while i < K {
        value = a[i as usize];
        j = i - 1;
        while (j >= 0) && (value > a[j as usize]) {
            a[(j + 1) as usize] = a[j as usize]; /* Shift value */
            idx[(j + 1) as usize] = idx[j as usize]; /* Shift index */
            j -= 1;
        }
        a[(j + 1) as usize] = value; /* Write value */
        idx[(j + 1) as usize] = i; /* Write index */
        i += 1;
    }

    /* If less than L values are asked check the remaining values,      */
    /* but only spend CPU to ensure that the K first values are correct */
    i = K;
    while i < L {
        value = a[i as usize];
        if value > a[(K - 1) as usize] {
            j = K - 2;
            while (j >= 0) && (value > a[j as usize]) {
                a[(j + 1) as usize] = a[j as usize]; /* Shift value */
                idx[(j + 1) as usize] = idx[j as usize]; /* Shift index */
                j -= 1;
            }
            a[(j + 1) as usize] = value; /* Write value */
            idx[(j + 1) as usize] = i; /* Write index */
        }
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorts_decreasing_and_tracks_indices() {
        let mut a = [1.0f32, 5.0, 2.0, 4.0, 3.0];
        let mut idx = [0i32; 5];
        silk_insertion_sort_decreasing_FLP(&mut a, &mut idx, 5, 4);
        assert_eq!(&a[..4], &[5.0, 4.0, 3.0, 2.0]);
        assert_eq!(&idx[..4], &[1, 3, 4, 2]);
    }

    #[test]
    fn partial_sort_keeps_first_k_correct() {
        let mut a = [9.0f32, 1.0, 8.0, 2.0, 7.0, 3.0];
        let mut idx = [0i32; 6];
        silk_insertion_sort_decreasing_FLP(&mut a, &mut idx, 6, 3);
        assert_eq!(&a[..3], &[9.0, 8.0, 7.0]);
        assert_eq!(&idx[..3], &[0, 2, 4]);
    }
}

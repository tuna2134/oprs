//! Mirror of `silk/sort.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Insertion sort helpers.  The `#ifdef FIXED_POINT` function
//! (`silk_insertion_sort_decreasing_int16`) is only used by the fixed-point
//! build, so it is not ported.
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

use crate::silk::typedef::*;
use crate::types::*;

/* Insertion sort (fast for already almost sorted arrays):   */
/* Best case:  O(n)   for an already sorted array            */
/* Worst case: O(n^2) for an inversely sorted array          */
/*                                                           */
/* Shell short:    https://en.wikipedia.org/wiki/Shell_sort  */

/// `silk_insertion_sort_increasing(a, idx, L, K)`
pub fn silk_insertion_sort_increasing(
    a: &mut [opus_int32],
    idx: &mut [opus_int],
    L: opus_int,
    K: opus_int,
) {
    let mut value: opus_int32;
    let mut j: opus_int;

    /* Safety checks */
    silk_assert!(K > 0);
    silk_assert!(L > 0);
    silk_assert!(L >= K);

    /* Write start indices in index vector */
    for i in 0..K {
        idx[i as usize] = i;
    }

    /* Sort vector elements by value, increasing order */
    for i in 1..K {
        value = a[i as usize];
        j = i - 1;
        while j >= 0 && value < a[j as usize] {
            a[(j + 1) as usize] = a[j as usize]; /* Shift value */
            idx[(j + 1) as usize] = idx[j as usize]; /* Shift index */
            j -= 1;
        }
        a[(j + 1) as usize] = value; /* Write value */
        idx[(j + 1) as usize] = i; /* Write index */
    }

    /* If less than L values are asked for, check the remaining values, */
    /* but only spend CPU to ensure that the K first values are correct */
    for i in K..L {
        value = a[i as usize];
        if value < a[(K - 1) as usize] {
            j = K - 2;
            while j >= 0 && value < a[j as usize] {
                a[(j + 1) as usize] = a[j as usize]; /* Shift value */
                idx[(j + 1) as usize] = idx[j as usize]; /* Shift index */
                j -= 1;
            }
            a[(j + 1) as usize] = value; /* Write value */
            idx[(j + 1) as usize] = i; /* Write index */
        }
    }
}

/// `silk_insertion_sort_increasing_all_values_int16(a, L)`
pub fn silk_insertion_sort_increasing_all_values_int16(a: &mut [opus_int16], L: opus_int) {
    let mut value: opus_int;
    let mut j: opus_int;

    /* Safety checks */
    silk_assert!(L > 0);

    /* Sort vector elements by value, increasing order */
    for i in 1..L {
        value = a[i as usize] as opus_int;
        j = i - 1;
        while j >= 0 && value < a[j as usize] as opus_int {
            a[(j + 1) as usize] = a[j as usize]; /* Shift value */
            j -= 1;
        }
        a[(j + 1) as usize] = value as opus_int16; /* Write value */
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn increasing_with_indices() {
        let mut a = [5i32, 1, 4, 2, 3];
        let mut idx = [0i32; 5];
        silk_insertion_sort_increasing(&mut a, &mut idx, 5, 5);
        assert_eq!(a, [1, 2, 3, 4, 5]);
        assert_eq!(idx, [1, 3, 4, 2, 0]);
    }

    #[test]
    fn increasing_all_values_int16() {
        let mut a = [3i16, -1, 2, 2, 0];
        silk_insertion_sort_increasing_all_values_int16(&mut a, 5);
        assert_eq!(a, [-1, 0, 2, 2, 3]);
    }
}

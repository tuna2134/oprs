//! Mirror of `silk/LP_variable_cutoff.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Elliptic/Cauer filters designed with 0.1 dB passband ripple, 80 dB minimum
//! stopband attenuation, and [0.95 : 0.15 : 0.35] normalized cut off
//! frequencies.  Low-pass filter with variable cutoff frequency based on
//! piece-wise linear interpolation between elliptic filters.
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

use crate::silk::biquad_alt::*;
use crate::silk::define::*;
use crate::silk::macros::*;
use crate::silk::sigproc_fix::*;
use crate::silk::structs::*;
use crate::silk::tables_other::*;
use crate::silk::typedef::*;
use crate::types::*;

/* Helper function, interpolates the filter taps */
fn silk_LP_interpolate_filter_taps(
    B_Q28: &mut [opus_int32],    /* O    [ TRANSITION_NB ] */
    A_Q28: &mut [opus_int32],    /* O    [ TRANSITION_NA ] */
    ind: opus_int,
    fac_Q16: opus_int32,
) {
    if ind < TRANSITION_INT_NUM - 1 {
        if fac_Q16 > 0 {
            if fac_Q16 < 32768 {
                /* fac_Q16 is in range of a 16-bit int */
                /* Piece-wise linear interpolation of B and A */
                for nb in 0..TRANSITION_NB {
                    B_Q28[nb as usize] = silk_SMLAWB(
                        silk_Transition_LP_B_Q28[ind as usize][nb as usize],
                        silk_Transition_LP_B_Q28[(ind + 1) as usize][nb as usize]
                            - silk_Transition_LP_B_Q28[ind as usize][nb as usize],
                        fac_Q16,
                    );
                }
                for na in 0..TRANSITION_NA {
                    A_Q28[na as usize] = silk_SMLAWB(
                        silk_Transition_LP_A_Q28[ind as usize][na as usize],
                        silk_Transition_LP_A_Q28[(ind + 1) as usize][na as usize]
                            - silk_Transition_LP_A_Q28[ind as usize][na as usize],
                        fac_Q16,
                    );
                }
            } else {
                /* ( fac_Q16 - ( 1 << 16 ) ) is in range of a 16-bit int */
                silk_assert!(fac_Q16 - (1 << 16) == silk_SAT16(fac_Q16 - (1 << 16)));
                /* Piece-wise linear interpolation of B and A */
                for nb in 0..TRANSITION_NB {
                    B_Q28[nb as usize] = silk_SMLAWB(
                        silk_Transition_LP_B_Q28[(ind + 1) as usize][nb as usize],
                        silk_Transition_LP_B_Q28[(ind + 1) as usize][nb as usize]
                            - silk_Transition_LP_B_Q28[ind as usize][nb as usize],
                        fac_Q16 - (1 << 16),
                    );
                }
                for na in 0..TRANSITION_NA {
                    A_Q28[na as usize] = silk_SMLAWB(
                        silk_Transition_LP_A_Q28[(ind + 1) as usize][na as usize],
                        silk_Transition_LP_A_Q28[(ind + 1) as usize][na as usize]
                            - silk_Transition_LP_A_Q28[ind as usize][na as usize],
                        fac_Q16 - (1 << 16),
                    );
                }
            }
        } else {
            B_Q28[..TRANSITION_NB as usize]
                .copy_from_slice(&silk_Transition_LP_B_Q28[ind as usize]);
            A_Q28[..TRANSITION_NA as usize]
                .copy_from_slice(&silk_Transition_LP_A_Q28[ind as usize]);
        }
    } else {
        B_Q28[..TRANSITION_NB as usize]
            .copy_from_slice(&silk_Transition_LP_B_Q28[(TRANSITION_INT_NUM - 1) as usize]);
        A_Q28[..TRANSITION_NA as usize]
            .copy_from_slice(&silk_Transition_LP_A_Q28[(TRANSITION_INT_NUM - 1) as usize]);
    }
}

/* Low-pass filter with variable cutoff frequency based on  */
/* piece-wise linear interpolation between elliptic filters */
/* Start by setting psEncC->mode <> 0;                      */
/* Deactivate by setting psEncC->mode = 0;                  */
/// `silk_LP_variable_cutoff(psLP, frame, frame_length)`
pub fn silk_LP_variable_cutoff(
    psLP: &mut silk_LP_state,   /* I/O  LP filter state */
    frame: &mut [opus_int16],   /* I/O  Low-pass filtered output signal */
    frame_length: opus_int,     /* I    Frame length */
) {
    let mut B_Q28 = [0 as opus_int32; TRANSITION_NB as usize];
    let mut A_Q28 = [0 as opus_int32; TRANSITION_NA as usize];
    let mut fac_Q16: opus_int32 = 0;
    let mut ind: opus_int = 0;

    silk_assert!(psLP.transition_frame_no >= 0 && psLP.transition_frame_no <= TRANSITION_FRAMES);

    /* Run filter if needed */
    if psLP.mode != 0 {
        /* Calculate index and interpolation factor for interpolation */
        fac_Q16 = silk_LSHIFT(TRANSITION_FRAMES - psLP.transition_frame_no, 16 - 6);
        ind = silk_RSHIFT(fac_Q16, 16);
        fac_Q16 -= silk_LSHIFT(ind, 16);

        silk_assert!(ind >= 0);
        silk_assert!(ind < TRANSITION_INT_NUM);

        /* Interpolate filter coefficients */
        silk_LP_interpolate_filter_taps(&mut B_Q28, &mut A_Q28, ind, fac_Q16);

        /* Update transition frame number for next frame */
        psLP.transition_frame_no = silk_LIMIT(psLP.transition_frame_no + psLP.mode, 0, TRANSITION_FRAMES);

        /* ARMA low-pass filtering */
        silk_assert!(TRANSITION_NB == 3 && TRANSITION_NA == 2);
        /* The C aliases `in` and `out` here (in-place filter); the port keeps a
           copy of the input so the borrow checker permits the same arithmetic. */
        let in_buf: Vec<opus_int16> = frame[..frame_length as usize].to_vec();
        silk_biquad_alt_stride1(
            &in_buf,
            &B_Q28,
            &A_Q28,
            &mut psLP.In_LP_State,
            frame,
            frame_length,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_zero_is_a_noop() {
        let mut st = silk_LP_state::default();
        let frame0 = [123i16; 320];
        let mut frame = frame0;
        silk_LP_variable_cutoff(&mut st, &mut frame, 320);
        assert_eq!(frame, frame0);
        assert_eq!(st.transition_frame_no, 0);
        assert_eq!(st.In_LP_State, [0, 0]);
    }
}

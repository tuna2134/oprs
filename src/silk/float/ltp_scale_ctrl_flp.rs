//! Mirror of `silk/float/LTP_scale_ctrl_FLP.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Calculation of LTP state scaling.
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

use crate::silk::define::{CODE_INDEPENDENTLY, CODE_INDEPENDENTLY_NO_LTP_SCALING};
use crate::silk::float::main_flp::{silk_encoder_control_FLP, silk_encoder_state_FLP};
use crate::silk::float::sigproc_flp::silk_float;
use crate::silk::log2lin::silk_log2lin;
use crate::silk::macros::silk_SMULBB;
use crate::silk::tables_other::silk_LTPScales_table_Q14;
use crate::silk::typedef::*;
use crate::types::*;

pub fn silk_LTP_scale_ctrl_FLP(
    psEnc: &mut silk_encoder_state_FLP,
    psEncCtrl: &mut silk_encoder_control_FLP,
    condCoding: opus_int,
) {
    let mut round_loss: opus_int;

    if condCoding == CODE_INDEPENDENTLY {
        /* Only scale if first frame in packet */
        round_loss = psEnc.sCmn.PacketLoss_perc * psEnc.sCmn.nFramesPerPacket;
        if psEnc.sCmn.LBRR_flag != 0 {
            /* LBRR reduces the effective loss. In practice, it does not square the loss because
               losses aren't independent, but that still seems to work best. We also never go below 2%. */
            round_loss = 2 + silk_SMULBB(round_loss, round_loss) / 100;
        }
        psEnc.sCmn.indices.LTP_scaleIndex = (silk_SMULBB(
            psEncCtrl.LTPredCodGain as opus_int32,
            round_loss,
        ) > silk_log2lin(2900 - psEnc.sCmn.SNR_dB_Q7)) as opus_int8;
        psEnc.sCmn.indices.LTP_scaleIndex += (silk_SMULBB(
            psEncCtrl.LTPredCodGain as opus_int32,
            round_loss,
        ) > silk_log2lin(3900 - psEnc.sCmn.SNR_dB_Q7))
            as opus_int8;
    } else {
        /* Default is minimum scaling */
        psEnc.sCmn.indices.LTP_scaleIndex = 0;
    }

    psEncCtrl.LTP_scale = silk_LTPScales_table_Q14[psEnc.sCmn.indices.LTP_scaleIndex as usize]
        as silk_float
        / 16384.0f32;
    let _ = CODE_INDEPENDENTLY_NO_LTP_SCALING;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conditional_coding_uses_minimum_scaling() {
        let mut enc = silk_encoder_state_FLP::default();
        let mut ctrl = silk_encoder_control_FLP::default();
        enc.sCmn.PacketLoss_perc = 10;
        enc.sCmn.nFramesPerPacket = 2;
        silk_LTP_scale_ctrl_FLP(&mut enc, &mut ctrl, 2 /* CODE_CONDITIONALLY */);
        assert_eq!(enc.sCmn.indices.LTP_scaleIndex, 0);
        assert_eq!(ctrl.LTP_scale, silk_LTPScales_table_Q14[0] as f32 / 16384.0);
    }

    #[test]
    fn high_ltp_gain_and_loss_selects_larger_index() {
        let mut enc = silk_encoder_state_FLP::default();
        let mut ctrl = silk_encoder_control_FLP::default();
        enc.sCmn.PacketLoss_perc = 100;
        enc.sCmn.nFramesPerPacket = 3;
        /* a high SNR pushes both silk_log2lin() thresholds to their minimum,
         * so the large LTP prediction gain trips both index increments */
        enc.sCmn.SNR_dB_Q7 = 100 * 128;
        ctrl.LTPredCodGain = 100.0;
        silk_LTP_scale_ctrl_FLP(&mut enc, &mut ctrl, 0 /* CODE_INDEPENDENTLY */);
        assert_eq!(enc.sCmn.indices.LTP_scaleIndex, 2);
        assert_eq!(ctrl.LTP_scale, silk_LTPScales_table_Q14[2] as f32 / 16384.0);
    }
}

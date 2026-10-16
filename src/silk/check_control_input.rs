/* Copyright (c) 2006-2011 Skype Limited. All Rights Reserved
   Ported to Rust from libopus 1.6.1 (silk/check_control_input.c).
   This file is a transliteration of the original C; the original copyright
   and licence (BSD-3-Clause, see the libopus COPYING file) apply. */
//! Mirror of `silk/check_control_input.c`.

use crate::silk::control::silk_EncControlStruct;
use crate::silk::define::*;
use crate::silk::typedef::opus_int;

/// Check encoder control struct
pub fn check_control_input(encControl: &silk_EncControlStruct) -> opus_int {
    if ((encControl.API_sampleRate != 8000)
        && (encControl.API_sampleRate != 12000)
        && (encControl.API_sampleRate != 16000)
        && (encControl.API_sampleRate != 24000)
        && (encControl.API_sampleRate != 32000)
        && (encControl.API_sampleRate != 44100)
        && (encControl.API_sampleRate != 48000))
        || ((encControl.desiredInternalSampleRate != 8000)
            && (encControl.desiredInternalSampleRate != 12000)
            && (encControl.desiredInternalSampleRate != 16000))
        || ((encControl.maxInternalSampleRate != 8000)
            && (encControl.maxInternalSampleRate != 12000)
            && (encControl.maxInternalSampleRate != 16000))
        || ((encControl.minInternalSampleRate != 8000)
            && (encControl.minInternalSampleRate != 12000)
            && (encControl.minInternalSampleRate != 16000))
        || (encControl.minInternalSampleRate > encControl.desiredInternalSampleRate)
        || (encControl.maxInternalSampleRate < encControl.desiredInternalSampleRate)
        || (encControl.minInternalSampleRate > encControl.maxInternalSampleRate)
    {
        debug_assert!(false);
        return SILK_ENC_FS_NOT_SUPPORTED;
    }
    if encControl.payloadSize_ms != 10
        && encControl.payloadSize_ms != 20
        && encControl.payloadSize_ms != 40
        && encControl.payloadSize_ms != 60
    {
        debug_assert!(false);
        return SILK_ENC_PACKET_SIZE_NOT_SUPPORTED;
    }
    if encControl.packetLossPercentage < 0 || encControl.packetLossPercentage > 100 {
        debug_assert!(false);
        return SILK_ENC_INVALID_LOSS_RATE;
    }
    if encControl.useDTX < 0 || encControl.useDTX > 1 {
        debug_assert!(false);
        return SILK_ENC_INVALID_DTX_SETTING;
    }
    if encControl.useCBR < 0 || encControl.useCBR > 1 {
        debug_assert!(false);
        return SILK_ENC_INVALID_CBR_SETTING;
    }
    if encControl.useInBandFEC < 0 || encControl.useInBandFEC > 1 {
        debug_assert!(false);
        return SILK_ENC_INVALID_INBAND_FEC_SETTING;
    }
    if encControl.nChannelsAPI < 1 || encControl.nChannelsAPI > ENCODER_NUM_CHANNELS {
        debug_assert!(false);
        return SILK_ENC_INVALID_NUMBER_OF_CHANNELS_ERROR;
    }
    if encControl.nChannelsInternal < 1 || encControl.nChannelsInternal > ENCODER_NUM_CHANNELS {
        debug_assert!(false);
        return SILK_ENC_INVALID_NUMBER_OF_CHANNELS_ERROR;
    }
    if encControl.nChannelsInternal > encControl.nChannelsAPI {
        debug_assert!(false);
        return SILK_ENC_INVALID_NUMBER_OF_CHANNELS_ERROR;
    }
    if encControl.complexity < 0 || encControl.complexity > 10 {
        debug_assert!(false);
        return SILK_ENC_INVALID_COMPLEXITY_SETTING;
    }

    SILK_NO_ERROR
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> silk_EncControlStruct {
        let mut c: silk_EncControlStruct = unsafe { core::mem::zeroed() };
        c.nChannelsAPI = 2;
        c.nChannelsInternal = 2;
        c.API_sampleRate = 48000;
        c.maxInternalSampleRate = 16000;
        c.minInternalSampleRate = 16000;
        c.desiredInternalSampleRate = 16000;
        c.payloadSize_ms = 20;
        c.packetLossPercentage = 0;
        c.complexity = 10;
        c.useInBandFEC = 0;
        c.useDTX = 0;
        c.useCBR = 0;
        c
    }

    #[test]
    fn accepts_a_valid_configuration() {
        assert_eq!(check_control_input(&base()), SILK_NO_ERROR);
    }

    /// The reference build defines `ENABLE_HARDENING`, so the C
    /// `celt_assert(0)` in front of each of these returns is a **live abort**.
    /// The Rust port maps `celt_assert!` to `debug_assert!`, so a debug build
    /// panics here (exactly like the C reference aborts) and only a release
    /// build returns the error code.  The codes are therefore checked only
    /// when debug assertions are compiled out.
    #[test]
    #[cfg(not(debug_assertions))]
    fn rejects_invalid_settings() {
        let mut c = base();
        c.API_sampleRate = 44101;
        assert_eq!(check_control_input(&c), SILK_ENC_FS_NOT_SUPPORTED);
        let mut c = base();
        c.payloadSize_ms = 30;
        assert_eq!(check_control_input(&c), SILK_ENC_PACKET_SIZE_NOT_SUPPORTED);
        let mut c = base();
        c.packetLossPercentage = 101;
        assert_eq!(check_control_input(&c), SILK_ENC_INVALID_LOSS_RATE);
        let mut c = base();
        c.nChannelsInternal = 3;
        assert_eq!(
            check_control_input(&c),
            SILK_ENC_INVALID_NUMBER_OF_CHANNELS_ERROR
        );
        let mut c = base();
        c.complexity = 11;
        assert_eq!(check_control_input(&c), SILK_ENC_INVALID_COMPLEXITY_SETTING);
    }
}

//! Mirror of `silk/PLC.h` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Packet-loss-concealment constants.  The function declarations of the header are skipped (Rust does not need them).
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

/// `BWE_COEF` (`0.99`, a double literal in the C).
pub const BWE_COEF: f64 = 0.99;
/// `V_PITCH_GAIN_START_MIN_Q14` — 0.7 in Q14.
pub const V_PITCH_GAIN_START_MIN_Q14: opus_int = 11469;
/// `V_PITCH_GAIN_START_MAX_Q14` — 0.95 in Q14.
pub const V_PITCH_GAIN_START_MAX_Q14: opus_int = 15565;
/// `MAX_PITCH_LAG_MS`
pub const MAX_PITCH_LAG_MS: opus_int = 18;
/// `RAND_BUF_SIZE`
pub const RAND_BUF_SIZE: opus_int = 128;
/// `RAND_BUF_MASK`
pub const RAND_BUF_MASK: opus_int = RAND_BUF_SIZE - 1;
/// `LOG2_INV_LPC_GAIN_HIGH_THRES` — 2^3 = 8 dB LPC gain.
pub const LOG2_INV_LPC_GAIN_HIGH_THRES: opus_int = 3;
/// `LOG2_INV_LPC_GAIN_LOW_THRES` — 2^8 = 24 dB LPC gain.
pub const LOG2_INV_LPC_GAIN_LOW_THRES: opus_int = 8;
/// `PITCH_DRIFT_FAC_Q16` — 0.01 in Q16.
pub const PITCH_DRIFT_FAC_Q16: opus_int = 655;

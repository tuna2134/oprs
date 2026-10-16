//! Mirror of `silk/API.h` from libopus 1.6.1, transliterated to safe Rust.
//!
//! The SILK public API's type definitions.  `silk_TOC_struct` and `SILK_MAX_FRAMES_PER_PACKET` are the only definitions in the header; every function declaration is skipped (Rust does not need it).
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

/// `SILK_MAX_FRAMES_PER_PACKET`
pub const SILK_MAX_FRAMES_PER_PACKET: opus_int = 3;

/// Struct for TOC (Table of Contents) — `silk_TOC_struct`.
#[derive(Clone, Copy, Debug, Default)]
pub struct silk_TOC_struct {
    /// Voice activity for packet
    pub VADFlag: opus_int,
    /// Voice activity for each frame in packet
    pub VADFlags: [opus_int; SILK_MAX_FRAMES_PER_PACKET as usize],
    /// Flag indicating if packet contains in-band FEC
    pub inbandFECFlag: opus_int,
}

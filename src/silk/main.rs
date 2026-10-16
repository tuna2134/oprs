//! Mirror of `silk/main.h` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Declarations only.  `main.h` contains no types, enums or constants of its own — every definition it mentions lives in `structs.h`, `control.h`, `tables.h` or `PLC.h`, which have their own modules here; the function prototypes are not needed in Rust.
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

//! This module intentionally contains no items: everything `main.h` includes
//! is ported in its own module.
//!
//! The dispatch macros defined next to the prototypes map onto the plain-C
//! implementations:
//!   `silk_VQ_WMat_EC(..., arch)`  -> `silk_VQ_WMat_EC_c(...)`
//!   `silk_NSQ(..., arch)`         -> `silk_NSQ_c(...)`
//!   `silk_NSQ_del_dec(..., arch)` -> `silk_NSQ_del_dec_c(...)`
//!   `silk_VAD_GetSA_Q8(psEncC, pIn, arch)` -> `silk_VAD_GetSA_Q8_c(...)`
//!
//! `silk_decode_frame` and `silk_PLC` gain the `ENABLE_DEEP_PLC` /
//! `ENABLE_OSCE` parameters (`LPCNetPLCState *lpcnet`, `OSCEModel *osce_model`)
//! in the reference configuration; those DNN types are declared in
//! `structs.rs` / `fixed/structs_fix.rs` as opaque state.

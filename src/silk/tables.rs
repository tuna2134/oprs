//! Mirror of `silk/tables.h` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Runtime declarations only.  Every table named by `tables.h` is defined (as `pub static`) in the per-file data module that mirrors the corresponding `tables_*.c` / `pitch_est_tables.c` / `table_LSF_cos.c` / `resampler_rom.c`:
//!
//! | C file | Rust module |
//! | --- | --- |
//! | `tables_gain.c` | `tables_gain` |
//! | `tables_LTP.c` | `tables_ltp` |
//! | `tables_NLSF_CB_NB_MB.c` | `tables_nlsf_cb_nb_mb` |
//! | `tables_NLSF_CB_WB.c` | `tables_nlsf_cb_wb` |
//! | `tables_other.c` | `tables_other` |
//! | `tables_pitch_lag.c` | `tables_pitch_lag` |
//! | `tables_pulses_per_block.c` | `tables_pulses_per_block` |
//! | `pitch_est_tables.c` | `pitch_est_tables` |
//! | `table_LSF_cos.c` | `table_lsf_cos` |
//! | `resampler_rom.c` | `resampler_rom` |
//!
//! The C `extern` declarations are not needed in Rust, so this module carries no
//! items.  The tables whose C declarations use a *symbolic* dimension
//! (`N_LEVELS_QGAIN / 8`, `MAX_DELTA_GAIN_QUANT - MIN_DELTA_GAIN_QUANT + 1`,
//! `TRANSITION_INT_NUM`, `RESAMPLER_*_ORDER_FIR*`, ...) are reshaped by hand
//! in those modules using the constants from `define.rs` / `resampler_rom.rs`.
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

//! This module intentionally contains no items (see the module documentation).

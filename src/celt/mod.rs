//! CELT: the low-delay transform codec half of Opus.
//!
//! Module layout mirrors `celt/` from libopus 1.6.1.  The port targets the
//! *float* configuration (`FIXED_POINT` undefined), which is what the
//! reference `libopus` shared library is built as.
#![allow(non_snake_case)]
#![allow(non_camel_case_types)]
#![allow(non_upper_case_globals)]
#![allow(unused_mut)]
#![allow(unused_assignments)]
#![allow(unused_parens)]
#![allow(clippy::excessive_precision)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::needless_range_loop)]
#![allow(clippy::identity_op)]
#![allow(clippy::manual_range_contains)]

pub mod arch;
pub mod kernels;
pub mod entcode;
pub mod entdec;
pub mod entenc;
pub mod laplace;
pub mod cwrs;
pub mod mathops;
pub mod kiss_fft;
pub mod mdct;
pub mod modes;
pub mod rate;
pub mod pitch;
pub mod celt_lpc;
pub mod quant_bands;
pub mod vq;
pub mod bands;
pub mod celt;
pub mod celt_decoder;
pub mod celt_encoder;

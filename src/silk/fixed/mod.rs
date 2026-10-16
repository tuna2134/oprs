//! Mirror of `silk/fixed/` from libopus 1.6.1.
#![allow(non_snake_case)]
#![allow(non_camel_case_types)]
#![allow(non_upper_case_globals)]
#![allow(unused_mut)]
#![allow(unused_assignments)]
#![allow(unused_parens)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::needless_range_loop)]

pub mod apply_sine_window_fix;
pub mod autocorr_fix;
pub mod burg_modified_fix;
pub mod corrmatrix_fix;
pub mod encode_frame_fix;
pub mod find_lpc_fix;
pub mod find_ltp_fix;
pub mod find_pitch_lags_fix;
pub mod find_pred_coefs_fix;
pub mod k2a_fix;
pub mod k2a_q16_fix;
pub mod ltp_analysis_filter_fix;
pub mod ltp_scale_ctrl_fix;
pub mod noise_shape_analysis_fix;
pub mod pitch_analysis_core_fix;
pub mod process_gains_fix;
pub mod regularize_correlations_fix;
pub mod residual_energy16_fix;
pub mod residual_energy_fix;
pub mod schur64_fix;
pub mod schur_fix;
pub mod structs_fix;
pub mod vector_ops_fix;
pub mod warped_autocorrelation_fix;

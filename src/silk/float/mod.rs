//! Mirror of `silk/float/` from libopus 1.6.1.
#![allow(non_snake_case)]
#![allow(non_camel_case_types)]
#![allow(non_upper_case_globals)]
#![allow(unused_mut)]
#![allow(unused_assignments)]
#![allow(unused_parens)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::needless_range_loop)]

pub mod apply_sine_window_flp;
pub mod autocorrelation_flp;
pub mod burg_modified_flp;
pub mod bwexpander_flp;
pub mod corrmatrix_flp;
pub mod encode_frame_flp;
pub mod energy_flp;
pub mod find_lpc_flp;
pub mod find_ltp_flp;
pub mod find_pitch_lags_flp;
pub mod find_pred_coefs_flp;
pub mod inner_product_flp;
pub mod k2a_flp;
pub mod lpc_analysis_filter_flp;
pub mod lpc_inv_pred_gain_flp;
pub mod ltp_analysis_filter_flp;
pub mod ltp_scale_ctrl_flp;
pub mod main_flp;
pub mod noise_shape_analysis_flp;
pub mod pitch_analysis_core_flp;
pub mod process_gains_flp;
pub mod regularize_correlations_flp;
pub mod residual_energy_flp;
pub mod scale_copy_vector_flp;
pub mod scale_vector_flp;
pub mod schur_flp;
pub mod sigproc_flp;
pub mod sort_flp;
pub mod warped_autocorrelation_flp;
pub mod wrappers_flp;

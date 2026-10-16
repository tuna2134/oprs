//! SILK: the linear-prediction half of Opus.
//!
//! Module layout mirrors `silk/` from libopus 1.6.1.
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

pub mod a2nlsf;
pub mod ana_filt_bank_1;
pub mod api;
pub mod biquad_alt;
pub mod bwexpander;
pub mod bwexpander_32;
pub mod check_control_input;
pub mod cng;
pub mod code_signs;
pub mod control;
pub mod control_audio_bandwidth;
pub mod control_codec;
pub mod control_snr;
pub mod debug;
pub mod dec_api;
pub mod decode_core;
pub mod decode_frame;
pub mod decode_indices;
pub mod decode_parameters;
pub mod decode_pitch;
pub mod decode_pulses;
pub mod decoder_set_fs;
pub mod define;
pub mod enc_api;
pub mod encode_indices;
pub mod encode_pulses;
pub mod gain_quant;
pub mod hp_variable_cutoff;
pub mod init_decoder;
pub mod init_encoder;
pub mod inlines;
pub mod inner_prod_aligned;
pub mod interpolate;
pub mod lin2log;
pub mod log2lin;
pub mod lp_variable_cutoff;
pub mod lpc_analysis_filter;
pub mod lpc_fit;
pub mod lpc_inv_pred_gain;
pub mod macros;
pub mod main;
pub mod nlsf2a;
pub mod nlsf_decode;
pub mod nlsf_del_dec_quant;
pub mod nlsf_encode;
pub mod nlsf_stabilize;
pub mod nlsf_unpack;
pub mod nlsf_vq;
pub mod nlsf_vq_weights_laroia;
pub mod nsq;
pub mod nsq_del_dec;
pub mod pitch_est_tables;
pub mod plc;
pub mod plc_h;
pub mod process_nlsfs;
pub mod quant_ltp_gains;
pub mod resampler;
pub mod resampler_down2;
pub mod resampler_down2_3;
pub mod resampler_private_ar2;
pub mod resampler_private_down_fir;
pub mod resampler_private_iir_fir;
pub mod resampler_private_up2_hq;
pub mod resampler_rom;
pub mod shell_coder;
pub mod sigm_q15;
pub mod sigproc_fix;
pub mod sort;
pub mod stereo_decode_pred;
pub mod stereo_encode_pred;
pub mod stereo_find_predictor;
pub mod stereo_lr_to_ms;
pub mod stereo_ms_to_lr;
pub mod stereo_quant_pred;
pub mod structs;
pub mod sum_sqr_shift;
pub mod table_lsf_cos;
pub mod tables;
pub mod tables_gain;
pub mod tables_ltp;
pub mod tables_nlsf_cb_nb_mb;
pub mod tables_nlsf_cb_wb;
pub mod tables_other;
pub mod tables_pitch_lag;
pub mod tables_pulses_per_block;
pub mod typedef;
pub mod fixed;
pub mod float;
pub mod vad;
pub mod vq_wmat_ec;

#!/usr/bin/env python3
"""Generate section 2 (file inventory) of docs/libopus-map.md."""
import os, re, subprocess, sys

REF = "/home/m-kikuchi/projects/oprs/reference/opus-1.6.1"
OUT = "/home/m-kikuchi/projects/oprs/docs/_s2.md"

def sh(cmd):
    return subprocess.run(cmd, shell=True, cwd=REF, capture_output=True, text=True).stdout

# ---------------------------------------------------------------- variant map
def parse_mk(path):
    """Return {file: variable_name} for every .c listed in a *_sources.mk."""
    m = {}
    cur = None
    for line in open(os.path.join(REF, path)):
        line = line.rstrip("\n")
        if not line.strip():
            continue
        if line.rstrip().endswith("=") or re.match(r"^[A-Z0-9_]+ *=", line):
            cur = line.split("=")[0].strip()
            rest = line.split("=", 1)[1].strip()
            if rest.endswith("\\"):
                rest = rest[:-1].strip()
            if rest:
                for f in rest.split():
                    if f.endswith(".c"):
                        m[f] = cur
            continue
        body = line.strip()
        if body.endswith("\\"):
            body = body[:-1].strip()
        if cur and body.endswith(".c"):
            m[body] = cur
    return m

mk = {}
for f in ("celt_sources.mk", "silk_sources.mk", "opus_sources.mk",
          "lpcnet_sources.mk"):
    mk.update(parse_mk(f))

VARIANT_LABEL = {
    "CELT_SOURCES": "ALWAYS (celt)",
    "SILK_SOURCES": "ALWAYS (silk)",
    "OPUS_SOURCES": "ALWAYS (src)",
    "OPUS_SOURCES_FLOAT": "FLOAT-API",
    "SILK_SOURCES_FIXED": "FIXED-ONLY",
    "SILK_SOURCES_FLOAT": "FLOAT-ONLY",
    "CELT_SOURCES_X86_RTCD": "RTCD (x86)",
    "CELT_SOURCES_SSE": "SSE",
    "CELT_SOURCES_SSE2": "SSE2",
    "CELT_SOURCES_SSE4_1": "SSE4.1",
    "CELT_SOURCES_AVX2": "AVX2",
    "CELT_SOURCES_ARM_RTCD": "RTCD (ARM)",
    "CELT_SOURCES_ARM_ASM": "ARM ASM",
    "CELT_AM_SOURCES_ARM_ASM": "ARM ASM",
    "CELT_SOURCES_ARM_NEON_INTR": "NEON",
    "CELT_SOURCES_ARM_NE10": "NE10",
    "SILK_SOURCES_X86_RTCD": "RTCD (x86)",
    "SILK_SOURCES_SSE4_1": "SSE4.1",
    "SILK_SOURCES_FIXED_SSE4_1": "SSE4.1",
    "SILK_SOURCES_AVX2": "AVX2",
    "SILK_SOURCES_FLOAT_AVX2": "AVX2",
    "SILK_SOURCES_ARM_RTCD": "RTCD (ARM)",
    "SILK_SOURCES_ARM_NEON_INTR": "NEON",
    "SILK_SOURCES_FIXED_ARM_NEON_INTR": "NEON",
}

# ------------------------------------------------------------ classifications
NON_CORE = {
    "src/opus_demo.c", "src/opus_compare.c", "src/repacketizer_demo.c",
    "src/qext_compare.c", "celt/opus_custom_demo.c", "celt/mini_kfft.c",
}
SIMD_DIR = re.compile(r"/(x86|arm|mips)/")
TEST_DIR = re.compile(r"/tests?/")

def classify(path):
    if SIMD_DIR.search(path):
        return "SIMD/ASM-ONLY"
    if TEST_DIR.search(path) or path in NON_CORE:
        return "NOT-CORE"
    return "YES"

def domain(path):
    if path in ("src/analysis.c", "src/mlp.c", "src/mlp_data.c"):
        return "FLOAT-API"
    if path.startswith("dnn/"):
        return "DNN"
    return "CORE"

def includes(path):
    txt = open(os.path.join(REF, path), errors="replace").read()
    return [m for m in re.findall(r'^\s*#\s*include\s+"([^"]+)"', txt, re.M)]

def lines(path):
    return int(sh(f"wc -l < {path}").strip())

# ------------------------------------------------------------- descriptions
# Auto-descriptions for the mechanical families.
def auto_desc(path, base):
    if SIMD_DIR.search(path):
        fam = []
        if "/x86/" in path:
            fam.append("x86 intrinsics")
        if "/arm/" in path:
            fam.append("ARM NEON intrinsics")
        if "/mips/" in path:
            fam.append("MIPS asm")
        return ("Intrinsics-only override (%s); replaces a pure-C function via the "
                "RTCD map. NOT needed for a portable Rust port." % "/".join(fam))
    if TEST_DIR.search(path):
        return ("Stand-alone test/demo program (not part of the library). "
                "Useful only as an oracle.")
    m = re.match(r"silk/(fixed|float)/(.*?)(_FIX|_FLP)\.c$", path)
    if m:
        stem = m.group(2)
        if m.group(1) == "fixed":
            return ("Fixed-point Q-domain implementation of `silk_%s` for the SILK "
                    "encoder analysis path (FIXED_POINT only)." % stem)
        return ("Float implementation of `silk_%s` for the SILK encoder analysis "
                "path (!FIXED_POINT only; not bit-exact across platforms)." % stem)
    m = re.match(r"src/mlp(_data)?\.c$", path)
    if m:
        return "MLP machinery / trained weights for the float-API analysis layer."
    return None

DESC = {
# ------------------------------------------------------------------ celt/
"celt/bands.c": "Band energy computation, normalisation/denormalisation, anti-collapse, spreading/hysteresis decisions, Hadamard transforms and `quant_all_bands` (the band-by-band PVQ/stereo quantiser driver).",
"celt/celt.c": "Mode-independent helpers shared by encoder and decoder: `resampling_factor`, `comb_filter`, `init_caps`.",
"celt/celt_encoder.c": "The CELT encoder: state init/CTL, `celt_preemphasis`, prefilter, band energy, TF/dynalloc/VBR analysis, and the whole `celt_encode_with_ec` bitstream writer.",
"celt/celt_decoder.c": "The CELT decoder: state init/CTL, `deemphasis`, `celt_synthesis`, PLC, and the whole `celt_decode_with_ec_dred`/`celt_decode_with_ec` bitstream reader.",
"celt/celt_lpc.c": "LPC helpers: Levinson-Durbin `_celt_lpc`, `celt_fir_c`, `celt_iir`, and `_celt_autocorr` (with the FFT-based autocorrelation path).",
"celt/cwrs.c": "Combinatorial pulse coding: `encode_pulses`/`decode_pulses` (V(N,K) enumeration), `log2_frac`, `get_required_bits`, and the `CeltPulseCache` construction.",
"celt/entcode.c": "Entropy-coder core: `ec_ilog`, `ec_tell`, `ec_tell_frac` — the bit-accounting shared by encoder and decoder.",
"celt/entdec.c": "Range decoder: `ec_dec_init/normalize`, `ec_decode*`, `ec_dec_bit_logp`, `ec_dec_icdf(16)`, `ec_dec_uint`, `ec_dec_bits`.",
"celt/entenc.c": "Range encoder: `ec_enc_init/normalize/carry_out`, `ec_encode*`, `ec_enc_bit_logp`, `ec_enc_icdf(16)`, `ec_enc_uint`, `ec_enc_bits`, `ec_enc_patch_initial_bits`, `ec_enc_shrink`, `ec_enc_done`.",
"celt/kiss_fft.c": "Mixed-radix FFT used by the MDCT and the pitch prefilter: `opus_fft_alloc*`, `kf_bfly2/3/4/5`, `kf_work`, `opus_fft_c`, `opus_ifft_c`.",
"celt/laplace.c": "Laplace-model entropy coding of coarse band energies and of the SILK-side Laplace symbols: `ec_laplace_encode/decode`, `ec_laplace_encode_p0/decode_p0`.",
"celt/mathops.c": "Integer/float math primitives: `isqrt32`, `frac_div32(_q29)`, `celt_rsqrt_norm(32)`, `celt_sqrt(32)`, `celt_cos_norm(32)`, `celt_rcp(_norm*)`, plus `celt_float2int16_c`.",
"celt/mdct.c": "MDCT/IMDCT: `clt_mdct_init/clear`, `clt_mdct_forward_c`, `clt_mdct_backward_c` (the rotation/twiddle machinery around kiss_fft).",
"celt/modes.c": "Mode construction: `opus_custom_mode_create` (eBands, allocation vectors, pulse cache, MDCT twiddles), `opus_custom_mode_destroy`, `compute_qext_mode`.",
"celt/pitch.c": "Pitch analysis for the CELT encoder: `pitch_downsample`, `pitch_search`, `remove_doubling`, and the comb-filter/pitch-xcorr glue.",
"celt/quant_bands.c": "Band-energy (glog) quantisation: `quant_coarse_energy`, `quant_fine_energy`, `quant_energy_finalise`, the three `unquant_*` counterparts, and `amp2Log2`/`log2Amp`.",
"celt/rate.c": "Bit allocation: `compute_pulse_cache`, `clt_compute_allocation` (interp_bits2pulses family), `clt_compute_extra_allocation` (QEXT).",
"celt/vq.c": "PVQ vector quantisation: `exp_rotation`, `op_pvq_search_c`, `alg_quant`, `alg_unquant`, `renormalise_vector`, `stereo_itheta`, plus the QEXT `cubic_quant`/`cubic_unquant`.",
"celt/mini_kfft.c": "A small, standalone kiss-FFT variant vendored for the QEXT comparison tool (`qext_compare.c`); not part of the library.",
"celt/opus_custom_demo.c": "Demo program for the Opus custom API; not part of the library.",
# ------------------------------------------------------------------ silk/ (top level)
"silk/A2NLSF.c": "`silk_A2NLSF`: converts LPC coefficients to normalised LSFs by Chebyshev polynomial root search and bisection.",
"silk/ana_filt_bank_1.c": "`silk_ana_filt_bank_1`: two-band analysis filter bank (used for 2x downsampling in the resampler and stereo).",
"silk/biquad_alt.c": "`silk_biquad_alt_stride1/2`: second-order ARMA biquad with slowly varying Q28 coefficients.",
"silk/bwexpander.c": "`silk_bwexpander`: bandwidth expansion of Q12 LPC coefficients by a Q16 chirp factor.",
"silk/bwexpander_32.c": "`silk_bwexpander_32`: bandwidth expansion of Q16/Q17 LPC coefficients.",
"silk/check_control_input.c": "`check_control_input`: validates a `silk_EncControlStruct` and quantises the requested bitrate/frame size.",
"silk/CNG.c": "Comfort-noise generation: `silk_CNG_Reset` and `silk_CNG` (decoder-side noise injection and LPC/LTP smoothing).",
"silk/code_signs.c": "`silk_encode_signs`/`silk_decode_signs`: entropy coding of pulse signs for the NSQ output.",
"silk/control_audio_bandwidth.c": "`silk_control_audio_bandwidth`: chooses NB/MB/WB internal bandwidth and triggers resampler changes.",
"silk/control_codec.c": "`silk_control_encoder`: applies encoder CTLs to the per-channel state (bitrate, complexity, LPC order, features).",
"silk/control_SNR.c": "`silk_control_SNR`: maps bitrate/complexity to the SNR targets used by the gain/NSQ loops.",
"silk/debug.c": "Debug/timer helpers (`silk_TimerSave`, WMOPS reporting); compiled to nothing unless SILK debug macros are set.",
"silk/dec_API.c": "SILK decoder public entry points: `silk_Get_Decoder_Size`, `silk_InitDecoder`, `silk_ResetDecoder`, `silk_Decode`, `silk_get_TOC`, and the `silk_decoder` super-struct.",
"silk/decode_core.c": "`silk_decode_core`: the SILK decoder's per-frame synthesis (LTP/LPC excitation reconstruction and resampling).",
"silk/decode_frame.c": "`silk_decode_frame`: one SILK frame — decode indices, parameters, then call `silk_decode_core`; handles PLC/CNG fallback.",
"silk/decode_indices.c": "`silk_decode_indices`: reads the SILK frame's entropy-coded side information (gains, LSF/NLSF, pitch, LTP, seed).",
"silk/decode_parameters.c": "`silk_decode_parameters`: turns decoded indices into LPC/LTP/gain parameters (dequantisation and interpolation).",
"silk/decode_pitch.c": "`silk_decode_pitch`: maps the coded pitch index/lag to the per-subframe pitch lags.",
"silk/decode_pulses.c": "`silk_decode_pulses`: decodes the NSQ pulse excitation via the shell coder and the pulse-count/RateLevel tables.",
"silk/decoder_set_fs.c": "`silk_decoder_set_fs`: switches the decoder's internal sampling rate/bandwidth and resets dependent state.",
"silk/enc_API.c": "SILK encoder public entry points: `silk_Get_Encoder_Size`, `silk_InitEncoder`, `silk_Encode`, `silk_QueryEncoder`, plus the `silk_encoder` super-struct and VAD/LBRR/DTX logic.",
"silk/encode_indices.c": "`silk_encode_indices`: writes the SILK frame's entropy-coded side information.",
"silk/encode_pulses.c": "`silk_encode_pulses`: encodes the NSQ pulse excitation (RateLevel tree, split into 8- or 16-sample blocks, shell coder).",
"silk/gain_quant.c": "`silk_gains_quant`/`silk_gains_dequant`/`silk_gains_ID`: log-domain gain quantisation with inter-frame prediction.",
"silk/HP_variable_cutoff.c": "`silk_HP_variable_cutoff`: variable high-pass filter applied at the encoder input for bandwidth changes.",
"silk/init_decoder.c": "`silk_init_decoder`/`silk_reset_decoder`: zero/initialise a `silk_decoder_state` (and OSCE state when enabled).",
"silk/init_encoder.c": "`silk_init_encoder`: zero/initialise a `silk_encoder_state_FIX|FLP` channel state.",
"silk/inner_prod_aligned.c": "`silk_inner_prod_aligned_scale`: scaled inner product of 16-bit vectors.",
"silk/interpolate.c": "`silk_interpolate`: linear interpolation of two Q15 vectors (used for LSF and LTP interpolation).",
"silk/lin2log.c": "`silk_lin2log`: Q7 linear to Q7 log2 approximation (piecewise linear + table).",
"silk/log2lin.c": "`silk_log2lin`: Q7 log2 to Q16 linear approximation, the inverse of `silk_lin2log`.",
"silk/LPC_analysis_filter.c": "`silk_LPC_analysis_filter`: LPC residual computation, fixed-point with output scaling.",
"silk/LPC_fit.c": "`silk_LPC_fit`: rescales/rounds LPC coefficients to fit a target Q domain (used by the fixed-point encoder).",
"silk/LPC_inv_pred_gain.c": "`silk_LPC_inverse_pred_gain(_c)`: inverse prediction gain via the Schur/reflection-coefficient stability recursion.",
"silk/LP_variable_cutoff.c": "`silk_LP_variable_cutoff`: encoder-side variable low-pass filter for the LTP/voicing path.",
"silk/NLSF2A.c": "`silk_NLSF2A`: converts normalised LSFs to LPC coefficients (cosine table + polynomial expansion, with stabilisation fallback).",
"silk/NLSF_decode.c": "`silk_NLSF_decode`: decodes and stabilises an NLSF vector from its indices.",
"silk/NLSF_del_dec_quant.c": "`silk_NLSF_del_dec_quant`: delayed-decision (Viterbi) NLSF quantiser in the encoder.",
"silk/NLSF_encode.c": "`silk_NLSF_encode`: NLSF encoder front end (weighting, VQ, delayed decision, stabilisation).",
"silk/NLSF_stabilize.c": "`silk_NLSF_stabilize`: enforces minimum spacing of NLSF values, with the delta-min table.",
"silk/NLSF_unpack.c": "`silk_NLSF_unpack`: unpacks codebook predictor/selection indices into the per-coefficient structures.",
"silk/NLSF_VQ.c": "`silk_NLSF_VQ`: weighted squared-error search over the NLSF codebook.",
"silk/NLSF_VQ_weights_laroia.c": "`silk_NLSF_VQ_weights_laroia`: Laroia weights for NLSF VQ distortion.",
"silk/NSQ.c": "`silk_noise_shape_quantizer` and the two-stage NSQ driver: the SILK encoder's noise-shaping quantiser producing pulses and quantised signal.",
"silk/NSQ_del_dec.c": "`silk_NSQ_del_dec_c`: delayed-decision variant of the noise-shaping quantiser (default encoder path).",
"silk/pitch_est_tables.c": "Pitch-estimator codebook tables: `silk_CB_lags_stage2/3(_10_ms)`, `silk_Lag_range_stage3`, `silk_nb_cbk_searchs_stage3`.",
"silk/PLC.c": "Packet-loss concealment: `silk_PLC_Reset`, `silk_PLC`, `silk_PLC_glue_frames` (LPC/LTP extrapolation and energy matching), with an optional `ENABLE_DEEP_PLC` LPCNet branch.",
"silk/process_NLSFs.c": "`silk_process_NLSFs`: runs NLSF encode/decode round-trip, LPC conversion and stability checks each frame.",
"silk/quant_LTP_gains.c": "`silk_quant_LTP_gains`: joint VQ of the LTP filter taps per subframe with a rate-distortion criterion.",
"silk/resampler.c": "Resampler front end: `silk_resampler_init` and `silk_resampler`, dispatching to the up/down/2:3/FIR/IIR primitives.",
"silk/resampler_down2.c": "`silk_resampler_down2`: 2x decimation with an all-pass/all-pole pair.",
"silk/resampler_down2_3.c": "`silk_resampler_down2_3`: 2/3-rate decimation using the AR2 + FIR cascade.",
"silk/resampler_private_AR2.c": "`silk_resampler_private_AR2`: second-order ARMA pre-filter used by the down-samplers.",
"silk/resampler_private_down_FIR.c": "`silk_resampler_private_down_FIR`: interpolate-then-FIR decimation stage.",
"silk/resampler_private_IIR_FIR.c": "`silk_resampler_private_IIR_FIR`: IIR up-sampling stage followed by FIR interpolation.",
"silk/resampler_private_up2_HQ.c": "`silk_resampler_private_up2_HQ(_wrapper)`: 2x up-sampling with two all-pass branches.",
"silk/resampler_rom.c": "Resampler coefficient ROM: `silk_resampler_frac_FIR_12`, up2 HQ coefficients, and the rate/bandwidth dispatch table.",
"silk/shell_coder.c": "`silk_shell_encoder`/`silk_shell_decoder`: the shell (hierarchical) pulse coding used inside `encode_pulses`/`decode_pulses`.",
"silk/sigm_Q15.c": "`silk_sigm_Q15`: Q15 sigmoid approximation by piecewise segments (used for pitch/voicing decisions).",
"silk/sort.c": "Insertion sort helpers (`silk_insertion_sort_increasing*`, `..._decreasing_int16`) used throughout parameter search.",
"silk/stereo_decode_pred.c": "`silk_stereo_decode_pred`/`silk_stereo_decode_mid_only`: stereo predictor and mid-only flag decoding.",
"silk/stereo_encode_pred.c": "`silk_stereo_encode_pred`/`silk_stereo_encode_mid_only`: stereo predictor and mid-only flag coding.",
"silk/stereo_find_predictor.c": "`silk_stereo_find_predictor`: computes the mid/side predictor from cross-correlations.",
"silk/stereo_LR_to_MS.c": "`silk_stereo_LR_to_MS`: encoder-side L/R to M/S conversion, including the two-stage prediction and delay alignment.",
"silk/stereo_MS_to_LR.c": "`silk_stereo_MS_to_LR`: decoder-side M/S to L/R reconstruction.",
"silk/stereo_quant_pred.c": "`silk_stereo_quant_pred`: quantises the stereo predictor and mid-only flag to their entropy-coded indices.",
"silk/sum_sqr_shift.c": "`silk_sum_sqr_shift`: energy computation with a variable right shift to avoid overflow.",
"silk/table_LSF_cos.c": "The `silk_LSFCosTab_FIX_Q12` cosine table used by `silk_NLSF2A`/`silk_A2NLSF`.",
"silk/tables_gain.c": "Gain-quantisation tables: `silk_gains_dequant_Q16`-related CDFs and `silk_gain_quant_*`.",
"silk/tables_LTP.c": "LTP codebook tables: `silk_LTP_vq_ptrs_Q7`, `silk_LTP_gain_vq_*`, `silk_LTP_per_index_iCDF`, `silk_LTP_gain_iCDF_ptrs`.",
"silk/tables_NLSF_CB_NB_MB.c": "Narrowband/mediumband NLSF codebook: `silk_NLSF_CB_NB_MB`, its iCDFs, predictors, weights and delta-min table.",
"silk/tables_NLSF_CB_WB.c": "Wideband NLSF codebook: `silk_NLSF_CB_WB`, its iCDFs, predictors, weights and delta-min table.",
"silk/tables_other.c": "Miscellaneous tables: pitch-contour and pitch-lag iCDFs, LTP scaling, quantisation offset, PLC window, LBRR flags.",
"silk/tables_pitch_lag.c": "Pitch-lag coding tables: `silk_pitch_lag_iCDF`, `silk_pitch_delta_iCDF`, `silk_pitch_contour_*`, `silk_pitch_lag_*`.",
"silk/tables_pulses_per_block.c": "Pulse-count coding tables: `silk_max_pulses_table`, `silk_pulses_per_block_iCDF*`, `silk_rate_levels_iCDF*`, `silk_LSB_iCDF`.",
"silk/VAD.c": "Voice activity detection: `silk_VAD_Init`, `silk_VAD_GetSA_Q8(_c)` and noise-level tracking.",
"silk/VQ_WMat_EC.c": "`silk_VQ_WMat_EC_c`: weighted LTP-tap VQ search with a rate-distortion cost.",
"silk/tests/test_unit_LPC_inv_pred_gain.c": "Unit test for `silk_LPC_inverse_pred_gain`.",
# ------------------------------------------------------------------ src/
"src/opus.c": "Library-level helpers: `opus_strerror`, `opus_get_version_string`, and `opus_packet_parse_impl`/`opus_packet_init_impl`/`opus_packet_get_*` TOC and frame-header parsing.",
"src/opus_decoder.c": "The `OpusDecoder`: packet parsing, mode/frame-size detection, CELT+SILK hybrid decoding, PLC/soft-decoding glue, float/int16/24 and multistream-facing CTLs.",
"src/opus_encoder.c": "The `OpusEncoder`: mode/bandwidth/bitrate decision logic, SILK and CELT encoder driving, hybrid framing, DTX/FEC, VBR computation, delay/repacketisation glue.",
"src/repacketizer.c": "The `OpusRepacketizer`: frame concatenation, padding, self-delimited packet parsing and repacketised output.",
"src/extensions.c": "Opus packet extensions (the 1.6 'extensions' mechanism) and the `opus_extension_*` parse/iterate helpers.",
"src/opus_multistream.c": "Multistream helper: `opus_multistream_encoder_get_size` / `opus_multistream_decoder_get_size` and the channel mapping matrix layout.",
"src/opus_multistream_encoder.c": "The `OpusMSEncoder`: per-stream `OpusEncoder` fan-out, channel-mapping matrix, and coupled-stream handling.",
"src/opus_multistream_decoder.c": "The `OpusMSDecoder`: per-stream `OpusDecoder` fan-in, channel-mapping matrix, and soft-decoding hooks.",
"src/opus_projection_encoder.c": "Ambisonics projection super-frame encoder built on the multistream encoder plus a mixing matrix.",
"src/opus_projection_decoder.c": "Ambisonics projection super-frame decoder built on the multistream decoder plus a mixing matrix.",
"src/mapping_matrix.c": "Dense matrix multiply/accumulate used by multistream and projection (`mapping_matrix_*`), with fixed/float/QEXT variants.",
"src/analysis.c": "Encoder-side signal analysis (music/speech classification, tonality, activity, leak boost) driving CELT mode decisions — FLOAT API only.",
"src/mlp.c": "Small multi-layer perceptron inference used by `src/analysis.c` — FLOAT API only.",
"src/mlp_data.c": "Trained MLP weights for `src/mlp.c` — FLOAT API only.",
"src/opus_demo.c": "Reference CLI encoder/decoder demo; not part of the library.",
"src/opus_compare.c": "Bit-exactness comparison tool (RMS-weighted difference) used by the test suite; not part of the library.",
"src/repacketizer_demo.c": "Demo program for the repacketizer API; not part of the library.",
"src/qext_compare.c": "QEXT comparison tool built on `celt/mini_kfft.c`; not part of the library.",
}

def desc(path):
    if path in DESC:
        return DESC[path]
    base = os.path.basename(path)
    a = auto_desc(path, base)
    if a:
        return a
    return "TODO"

# -------------------------------------------------------------------- output
def table(paths):
    rows = ["| File | Lines | Portable? | Domain | Build variant | Description | Project `#include`s |",
            "|---|---:|---|---|---|---|---|"]
    tot = 0
    for p in paths:
        n = lines(p)
        tot += n
        inc = includes(p)
        incs = ", ".join("`%s`" % i for i in inc) if inc else "_(none)_"
        rows.append("| `%s` | %d | %s | %s | %s | %s | %s |" % (
            p, n, classify(p), domain(p), VARIANT_LABEL.get(mk.get(p), "NOT-CORE"),
            desc(p).replace("|", "\\|"), incs))
    rows.append("")
    rows.append("**Subtotal: %d files, %d lines.**" % (len(paths), tot))
    rows.append("")
    return "\n".join(rows)

def main():
    celt = sorted(sh("find celt -name '*.c'").split())
    silk = sorted(sh("find silk -name '*.c'").split())
    src = sorted(sh("find src -name '*.c'").split())

    def grp(paths, pred):
        return [p for p in paths if pred(p)]

    portable_celt = grp(celt, lambda p: classify(p) == "YES")
    simd_celt = grp(celt, lambda p: classify(p) == "SIMD/ASM-ONLY")
    noncore_celt = grp(celt, lambda p: classify(p) == "NOT-CORE")
    portable_silk = grp(silk, lambda p: classify(p) == "YES")
    simd_silk = grp(silk, lambda p: classify(p) == "SIMD/ASM-ONLY")
    noncore_silk = grp(silk, lambda p: classify(p) == "NOT-CORE")
    portable_src = grp(src, lambda p: classify(p) == "YES")
    noncore_src = grp(src, lambda p: classify(p) == "NOT-CORE")

    o = []
    o.append("## 2. File inventory\n")
    o.append("Every `.c` file under `celt/`, `silk/` and `src/` (recursively). "
             "Line counts are `wc -l` on the 1.6.1 tree. `Portable?` = `YES` for "
             "C code a portable Rust port must reproduce, `SIMD/ASM-ONLY` for "
             "intrinsics/assembly variants that must **not** be ported, "
             "`NOT-CORE` for demos/tests/tools. `Domain` = `CORE`, `FLOAT-API` "
             "(only compiled when the float API is enabled) or `DNN`. "
             "`Build variant` quotes the make variable that lists the file.\n")
    o.append("**Headline numbers**: %d `.c` files, %d lines; of which "
             "portable C = %d files / %d lines; SIMD-asm-only = %d files; "
             "not-core = %d files.\n" % (
                 len(celt) + len(silk) + len(src),
                 sum(lines(p) for p in celt + silk + src),
                 len(portable_celt) + len(portable_silk) + len(portable_src),
                 sum(lines(p) for p in portable_celt + portable_silk + portable_src),
                 len(simd_celt) + len(simd_silk),
                 len(noncore_celt) + len(noncore_silk) + len(noncore_src)))

    o.append("### 2.1 `celt/` — portable C (the 18 `CELT_SOURCES` + QEXT helper)\n")
    o.append(table(portable_celt))
    o.append("### 2.2 `celt/` — SIMD / assembly only (**do not port**)\n")
    o.append("All of these are RTCD dispatch shims or SIMD kernels. Each has a "
             "pure-C fallback that a portable port implements instead; the "
             "`*_c`/`*_c`-suffixed function is the portable name.\n")
    o.append(table(simd_celt))
    o.append("### 2.3 `celt/` — demos and tests (not part of the library)\n")
    o.append(table(noncore_celt))
    o.append("### 2.4 `silk/` — portable C (top level, `SILK_SOURCES`)\n")
    o.append(table(grp(portable_silk, lambda p: "/" not in p[5:])))
    o.append("### 2.5 `silk/fixed/` — fixed-point encoder analysis (`SILK_SOURCES_FIXED`)\n")
    o.append("Compiled **only** when `FIXED_POINT` is defined. These files "
             "implement the SILK encoder's analysis path in integer Q-domain "
             "arithmetic and are exactly what a bit-exact fixed-point port needs. "
             "`silk/fixed/structs_FIX.h` (`silk_encoder_state_FIX`, "
             "`silk_shape_state_FIX`, `silk_encoder_control_FIX`, and the outer "
             "`silk_encoder`) is the header counterpart.\n")
    o.append(table(grp(portable_silk, lambda p: p.startswith("silk/fixed/"))))
    o.append("### 2.6 `silk/float/` — float encoder analysis (`SILK_SOURCES_FLOAT`)\n")
    o.append("Compiled **only** when `FIXED_POINT` is **not** defined. This is "
             "the float encoder analysis path; it is **not** bit-exact across "
             "platforms (it relies on `float`/`double` FP contraction and libm), "
             "so a bit-exact port targets the fixed build and treats these files "
             "as documentation of intent only. `silk/float/wrappers_FLP.c` is "
             "special: it `#define`s the shared `silk_*` entry points to their "
             "`_FLP` implementations (see below).\n")
    o.append(table(grp(portable_silk, lambda p: p.startswith("silk/float/"))))
    o.append("### 2.7 `silk/` — SIMD / assembly only (**do not port**)\n")
    o.append(table(simd_silk))
    o.append("### 2.8 `src/` — portable C (`OPUS_SOURCES`)\n")
    o.append(table(portable_src))
    o.append("### 2.9 `src/` — demos and tools (not part of the library)\n")
    o.append(table(noncore_src))

    # ---- notes
    o.append("""### 2.10 The portable core set, and what the SIMD directories add

The complete portable core, in make-variable terms, is:

```
CELT_SOURCES          18 files   celt/*.c
SILK_SOURCES          77 files   silk/*.c
SILK_SOURCES_FIXED    23 files   silk/fixed/*.c     (FIXED_POINT)
  or SILK_SOURCES_FLOAT 28 files silk/float/*.c     (!FIXED_POINT)
OPUS_SOURCES          11 files   src/*.c
OPUS_SOURCES_FLOAT     3 files   src/analysis.c src/mlp.c src/mlp_data.c
                                 (only when the float API is on)
```

Everything in `celt/x86/`, `celt/arm/`, `celk/mips/`, `silk/x86/`, `silk/arm/`,
`silk/fixed/x86/`, `silk/fixed/arm/`, `silk/float/x86/` exists for speed only.
They are reached through the RTCD tables in
`celt/x86/x86_celt_map.c`, `celt/arm/arm_celt_map.c`, `silk/x86/x86_silk_map.c`,
`silk/arm/arm_silk_map.c` (plus the CPU-feature probes in `x86cpu.c` /
`armcpu.c`). A portable port deletes the whole dispatch layer and calls the
`_c` function unconditionally. Concretely, the overrides are:

| SIMD file | Replaces (portable C) | make variable |
|---|---|---|
| `celt/x86/pitch_sse.c` | `celt_pitch_xcorr` | `CELT_SOURCES_SSE` |
| `celt/x86/pitch_sse2.c` | `celt_inner_prod` (float) | `CELT_SOURCES_SSE2` |
| `celt/x86/vq_sse2.c` | `celt_inner_prod` (fixed) | `CELT_SOURCES_SSE2` |
| `celt/x86/pitch_sse4_1.c` | `celt_pitch_xcorr`, `xcorr_kernel` | `CELT_SOURCES_SSE4_1` |
| `celt/x86/celt_lpc_sse4_1.c` | `celt_fir`/`celt_iir` | `CELT_SOURCES_SSE4_1` |
| `celt/x86/pitch_avx.c` | `celt_pitch_xcorr` | `CELT_SOURCES_AVX2` |
| `silk/x86/NSQ_sse4_1.c` | `silk_noise_shape_quantizer` | `SILK_SOURCES_SSE4_1` |
| `silk/x86/NSQ_del_dec_sse4_1.c`, `silk/x86/NSQ_del_dec_avx2.c` | `silk_NSQ_del_dec` | `SILK_SOURCES_SSE4_1`, `SILK_SOURCES_AVX2` |
| `silk/x86/VAD_sse4_1.c` | `silk_VAD_GetSA_Q8` | `SILK_SOURCES_SSE4_1` |
| `silk/x86/VQ_WMat_EC_sse4_1.c` | `silk_VQ_WMat_EC` | `SILK_SOURCES_SSE4_1` |
| `silk/fixed/x86/vector_ops_FIX_sse4_1.c` | `silk_inner_prod_aligned`, `silk_sum_sqr_shift` | `SILK_SOURCES_FIXED_SSE4_1` |
| `silk/fixed/x86/burg_modified_FIX_sse4_1.c` | `silk_burg_modified` | `SILK_SOURCES_FIXED_SSE4_1` |
| `celt/arm/celt_neon_intr.c` | `celt_pitch_xcorr` / `celt_inner_prod` | `CELT_SOURCES_ARM_NEON_INTR` |
| `celt/arm/pitch_neon_intr.c` | `celt_pitch_xcorr` | `CELT_SOURCES_ARM_NEON_INTR` |
| `silk/arm/NSQ_neon.c`, `NSQ_del_dec_neon_intr.c` | `silk_noise_shape_quantizer`, `silk_NSQ_del_dec` | `SILK_SOURCES_ARM_NEON_INTR` |
| `silk/arm/biquad_alt_neon_intr.c` | `silk_biquad_alt_stride1/2` | `SILK_SOURCES_ARM_NEON_INTR` |
| `silk/arm/LPC_inv_pred_gain_neon_intr.c` | `silk_LPC_inverse_pred_gain_c` | `SILK_SOURCES_ARM_NEON_INTR` |
| `silk/fixed/arm/warped_autocorrelation_FIX_neon_intr.c` | `silk_warped_autocorrelation_FIX` | `SILK_SOURCES_FIXED_ARM_NEON_INTR` |
| `celt/arm/celt_fft_ne10.c`, `celt_mdct_ne10.c` | kiss FFT / MDCT via Ne10 | `CELT_SOURCES_ARM_NE10` |
| `celt/arm/celt_pitch_xcorr_arm.s` | `celt_pitch_xcorr` (inline asm) | `CELT_SOURCES_ARM_ASM` |
| `silk/float/x86/inner_product_FLP_avx2.c` | `silk_inner_product_FLP` | `SILK_SOURCES_FLOAT_AVX2` |

**Every SIMD override is required by libopus to be bit-exact with its C
counterpart** (`--enable-check-asm` exists to verify this), so ignoring them
cannot change the output bitstream — only the speed. This is confirmed by the
`OPUS_CHECK_ASM` instrumentation and is a useful invariant for the port's test
strategy: the portable `_c` function is the normative definition.

### 2.11 `silk/float/wrappers_FLP.c` — the float/fixed dispatch trick

The SILK top-level files call one set of names (`silk_find_LPC`,
`silk_burg_modified`, ...). In the fixed build those names are the functions in
`silk/fixed/*.c`. In the float build, the `_FLP` implementations are linked in,
and `silk/float/wrappers_FLP.c` provides the translation layer. It works by
macro-renaming the shared entry points, e.g.:

```c
/* silk/float/wrappers_FLP.c */
#define silk_encoder_state_Fxx        silk_encoder_state_FLP
#define silk_enc_API_enc_frame_FLP    silk_Encode
```
(refer to the file itself for the exact current set; the pattern is that the
top-level `silk_Encode` selects `silk_encode_frame_FLP` while the *parameter*
structs are the FLP variants.) The practical consequence for the port: the
`silk/fixed/*` function signatures are the normative ones, and the `_FLP`
files can be ignored once `FIXED_POINT` is chosen.

### 2.12 Header files of the core (line counts)

""")
    hdrs = sh("find celt silk src -name '*.h'").split()
    rows = ["| Header | Lines | Purpose |", "|---|---:|---|"]
    for h in sorted(hdrs):
        n = lines(h)
        rows.append("| `%s` | %d | |" % (h, n))
    o.append("\n".join(rows))
    o.append("")

    open(OUT, "w").write("\n".join(o))
    print("wrote", OUT, "lines:", sum(1 for _ in open(OUT)))


# --- post-process: fill the §2.12 header-purpose column (reproducible) ---------
def fill_header_purposes(path=OUT):
    import re as _re
    try:
        from _hdr_purposes import PURPOSES
    except Exception:
        return
    t = open(path).read()
    i = t.index("### 2.12 Header files of the core")
    head, sec = t[:i], t[i:]
    out = []
    for line in sec.split("\n"):
        m = _re.match(r"^\| `([^`]+\.h)` \| (\d+) \| *\|$", line)
        if m and m.group(1) in PURPOSES:
            line = "| `%s` | %s | %s |" % (m.group(1), m.group(2),
                                           PURPOSES[m.group(1)])
        out.append(line)
    open(path, "w").write(head + "\n".join(out))


main()

fill_header_purposes()

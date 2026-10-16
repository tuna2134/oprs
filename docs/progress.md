# Progress and roadmap

## Objective

A pure-Rust implementation of Opus that is (a) source-compatible with the
`audiopus` 0.3.0-rc.0 public API and (b) bit-exact with the installed
`libopus-dev` (1.6.1) for random audio input, for both the produced packets and
the decoded PCM.

## Reference configuration (solved, reproducible)

`compat/scripts/build-reference.sh` produces `reference/lib-ref/libopus.a`
which reproduces the installed `opus 1.6.1-1` (Arch Linux) **bit-for-bit**:

| knob | value |
| --- | --- |
| `FIXED_POINT` | undefined (floating point) |
| `FLOAT_APPROX` | undefined — **meson's default is off, autotools' default is on** |
| `asm` | irrelevant on x86 (Arch passes `-D asm=disabled`, which only disables ARM assembly) |
| `intrinsics` / `rtcd` | **enabled** — the reference executes x86 SIMD kernels |
| `ENABLE_DEEP_PLC`, `ENABLE_DRED`, `ENABLE_OSCE` | defined |
| `ENABLE_OSCE_BWE` | **undefined** (meson never defines it, autotools does) |
| `CUSTOM_MODES`, `ENABLE_HARDENING` | defined |

Verification: `reference/tools/oracle.c` encodes/decodes 16 configuration rows
(8/12/16/24/48 kHz, mono/stereo, 2.5–60 ms, 12–128 kbit/s, CBR/VBR/CVBR, DTX,
FEC + loss, forced bandwidths) and the full report is identical between the
reference build and the system library — encoder *and* decoder.

The host CPU supports AVX2+FMA, so `opus_select_arch()` returns 4 and the
**SIMD** kernels are live. Consequently the Rust port must reproduce the SIMD
arithmetic, not the plain-C arithmetic, wherever the two differ
(`docs/rtcd-kernels.md`). Integer SIMD kernels are exact by construction; only
the float kernels are a hazard. SILK's integer codec is therefore SIMD-safe;
CELT and the SILK float analysis paths are not.

Confirmed at arch 4 (from `celt/x86/x86_celt_map.c`):

| dispatched function | arch 4 implementation | hazard |
| --- | --- | --- |
| `celt_pitch_xcorr` | `celt_pitch_xcorr_avx2` (`pitch_avx.c`), **8 accumulators** | NOT equal to C |
| `xcorr_kernel` | `xcorr_kernel_sse` | 4 accumulators, `#if 0` bit-exact switch |
| `celt_inner_prod` | `celt_inner_prod_sse` | SIMD summation order |
| `dual_inner_prod` | `dual_inner_prod_sse` | SIMD summation order |
| `comb_filter_const` | `comb_filter_const_sse` | SIMD |
| `op_pvq_search` | `op_pvq_search_sse2` | SIMD (encoder) |
| `celt_fir` / `celt_iir` / `celt_lpc` | SSE4.1 variants | SIMD |

SILK dispatches (all in the encoder path except the integer inner products,
which are exact): `silk_NSQ`, `silk_NSQ_del_dec`, `silk_VQ_WMat_EC`,
`silk_VAD_GetSA_Q8`, `silk_burg_modified` (FLP), `silk_inner_prod16_aligned_64`,
`silk_inner_prod_aligned_float`.

## Done and verified

Run `compat/scripts/run-all-probes.sh` — currently **all 15 probe pairs are
bit-exact**:

| probe | lines | covers |
| --- | --- | --- |
| `entropy` | 261 | range coder (4000 ops + `ec_enc_shrink`) |
| `laplace` | 1390 | Laplace coder incl. the `_p0` DRED variants |
| `mathops` / `mathopscheck` | 2961 / 6669 | every float approximation, compared as raw IEEE bits |
| `mdct` | 1961 | kiss_fft + MDCT (twiddles and forward/backward) |
| `cwrs` | 2891 | PVQ combinatorial coder over the whole `(N,K)` domain |
| `kernels` | 2816 | arch-4 x86 SIMD kernels (AVX2/FMA `celt_pitch_xcorr`, SSE reductions) |
| `repacketizer` | 748 | packet parsing + repacketizer + pad/unpad |
| `silkbase` | 4662 | all SILK integer macros + 87 constant tables + struct sizes |
| `silkdsp` | 1969 | SILK decoder DSP primitives + every resampler path |
| `modes` | 2122 | mode tables (25 cases) + 96 rate-allocation sweeps |
| `celt` | 5023 | `comb_filter`, `init_caps`, `resampling_factor` |
| `pitch` | 1397 | pitch search + `celt_lpc`/`_celt_autocorr` (one documented NaN-sign caveat) |
| `vq` | 2773 | PVQ quant/unquant through the SSE2 `op_pvq_search` |
| `quant_bands` | — | coarse/fine/final energy coding |
| `celt_decoder` / `celt_encoder` / `bands` / `silk_decode` | — | in flight |

Also verified in passing: `ENABLE_HARDENING` is active in the reference, so
`celt_assert`/`silk_assert` are live aborts there; the Rust port maps them to
`debug_assert`.  This only differs for invalid input, never for the valid audio
the objective is about.

Corrections to the dispatch picture found by the porters:

* `celt_fir_sse4_1` does **not** exist in the float build (it is
  `FIXED_POINT`-gated), so `celt_fir` is the C function whose inner kernel is
  `xcorr_kernel_sse`.  `kernels::celt_fir` is what the C symbol actually does —
  the `probe_kernels` comparison is the authority here.
* `celt_pitch_xcorr_c` **is** compiled (the `PITCH_XCORR_IMPL` table's arch 0–3
  entries reference it); only the arch-4 entry is the AVX2 kernel.
* `comb_filter_const` resolves to `comb_filter_const_sse`
  (`OPUS_X86_PRESUME_SSE`, not SSE4.1).


* `src/celt/arch.rs` — float configuration of `celt/arch.h`
  (`opus_val16 = f32`, all fixed-point macros as identities, `FLOAT2INT16`
  with round-to-nearest-even like `_mm_cvt_ss2si`).
* `src/celt/entcode.rs`, `src/celt/entdec.rs`, `src/celt/entenc.rs` — the
  range coder. **Verified bit-exact** by `compat/scripts/run-probe.sh entropy`:
  4000 mixed operations (`ec_enc_uint`, `ec_enc_bit_logp`, `ec_enc_icdf`,
  `ec_enc_bits`, `ec_encode`), a mid-stream `ec_enc_shrink`, the full
  15384-byte buffer, and the complete decode with `ec_tell`/`ec_tell_frac`
  checkpoints — all identical.
* `src/celt/laplace.rs` — **verified bit-exact** by
  `compat/scripts/run-probe.sh laplace` (1390 lines identical, including the
  `_p0` variants used by DRED).
* `src/celt/mathops.rs` — **verified bit-exact** by
  `compat/scripts/run-probe.sh mathopscheck` (6669 lines identical: raw IEEE
  bits of `celt_rsqrt_norm`, `celt_rsqrt`, `celt_sqrt`, `celt_rcp`, `celt_log2`,
  `celt_exp2`, `celt_cos_norm`, `celt_div`, `frac_div32`, `isqrt32`,
  `bitexact_cos`, `bitexact_log2tan`, `celt_lcg_rand` over 512 inputs).
* `src/celt/kiss_fft.rs`, `src/celt/mdct.rs` — **verified bit-exact** by
  `compat/scripts/run-probe.sh mdct` (1961 lines identical: generated twiddle
  tables and forward/backward MDCT output compared as raw float bits).
* `src/celt/cwrs.rs` — **verified bit-exact** by
  `compat/scripts/run-probe.sh cwrs` (2891 lines identical: `log2_frac`,
  `get_required_bits`, and `encode_pulses`/`decode_pulses` over the whole
  `(N, K)` domain).
* `src/celt/modes.rs` (1734 lines) + `src/celt/rate.rs` (914 lines) — the
  `OpusCustomMode`/`CELTMode` structures, every `static_modes_float.h` table
  (mechanically extracted) and the whole rate-allocation logic
  (`compute_pulse_cache`, `interp_bits2pulses`, `clt_compute_allocation`,
  `bits2pulses`/`pulses2bits`, `get_pulses`).  Verified by
  `compat/scripts/run-probe.sh modes` (mode-table hashes + rate-allocation
  sweep).  A mode is an owned `Box<CELTMode>`/`&CELTMode`; all tables are
  `Vec`s, never raw pointers.  `celt/modes.c`'s `arch` is pinned to 4 (the
  documented host), which is value-neutral for the tables.
* `compat/rustoracle/` — the Rust twin of `reference/tools/oracle.c` using only
  the public `audiopus`-compatible API.  Its shared test-vector generator is
  already validated: `cargo run --bin pcmcheck` reproduces the C oracle's
  `pcm_fnv` for **16/16** configurations, so both sides feed byte-identical
  audio to the encoder.  When the codec is complete the acceptance test is
  `diff <(oracle) <(rustoracle)`.
* `src/opus/packet.rs`, `src/opus/repacketizer.rs` — **verified bit-exact** by
  `compat/scripts/run-probe.sh repacketizer` (717 lines identical: packet
  parsing of valid and malformed packets, TOC/frame offsets, bandwidth /
  samples-per-frame / nb-frames / nb-samples, `opus_repacketizer_cat`/
  `out_range`/`out`, and the pad/unpad round trips).
* `src/celt/kernels.rs` — **verified bit-exact** by
  `compat/scripts/run-probe.sh kernels` (2794 lines identical).  This is the
  emulation of the arch-4 x86 SIMD arithmetic that the reference build actually
  executes, including the AVX2/FMA `celt_pitch_xcorr` and the
  `(a+b)+(c+d)` / `N%4` tail behaviour of `comb_filter_const_sse`.  Every CELT
  DSP module that touches pitch/LPC/PVQ/comb must go through this module
  instead of a plain-C reimplementation.
* `src/silk/**` foundation — **verified bit-exact** by
  `compat/scripts/run-probe.sh silkbase` (4662 lines identical): every integer
  macro/primitive of `macros.h`/`SigProc_FIX.h`/`Inlines.h` over boundary and
  random inputs, an FNV hash of all 87 generated constant tables read from the
  C symbols, and `sizeof`/`offsetof` of the pointer-free state structs.
  26 files: the headers, all `tables_*.rs`, `pitch_est_tables.rs`,
  `table_lsf_cos.rs`, `resampler_rom.rs`.
  Known deliberate divergences: `ENABLE_OSCE_BWE` fields dropped (the blind-BWE
  path only runs at decoder complexity >= 4, which the public API cannot set),
  the OSCE DNN types are exact-size opaque byte blocks, and `&'static [T]`
  slice fields make pointer-bearing structs 8 bytes wider than C.

* Also verified in passing: `ENABLE_HARDENING` is active in the reference, so
  `celt_assert`/`silk_assert` are live aborts there; the Rust port maps them to
  `debug_assert`.  This only differs for invalid input, never for the valid
  audio the objective is about.
* DNN scope decision (documented): at the default decoder complexity (0)
  `osce_enhance_frame()` takes its `OSCE_METHOD_NONE` path, whose float
  round-trip `xq -> f32 -> float2int` is the identity on `int16`, and
  `lpcnet_plc_update()` only updates DNN state.  So the DNN inference itself
  does not affect the decoded output for the configurations the objective
  tests; the *macro-level* `ENABLE_DEEP_PLC` behaviour (the `fs_kHz == 16`
  skip in `silk_PLC`'s glue-frames loop) does and must be ported.


* `docs/rtcd-kernels.md` — complete specification of the arch-4 SIMD kernels,
  with the exact accumulator layouts, reduction trees and FMA placement.  Key
  correction it contains: `OPUS_X86_PRESUME_SSE`/`PRESUME_SSE2` *are* defined,
  so `xcorr_kernel`, `celt_inner_prod`, `dual_inner_prod`, `comb_filter_const`
  and `op_pvq_search` are not runtime-dispatched at all — only
  `PITCH_XCORR_IMPL` is a real RTCD table (arch 4 → AVX2, with FMA).

## Coordination notes

* Several porters work in parallel in the same tree. A file that is only
  half-written breaks `cargo build` for everybody, and therefore every probe.
  When a probe fails to compile, first check whether the errors are in a file
  you do not own, and retry after a short wait.
* A deliberate, harmless divergence from C: `ec_dec_init()` in C leaves
  `ec_ctx.ext` uninitialised (it is only written by `ec_decode*()`), whereas
  the Rust port zeroes it.  Every `ec_dec_update()` caller is preceded by
  `ec_decode()` in the reference, so no behaviour depends on it; it only shows
  up if a probe prints `ext` after a decode that never called `ec_decode()`.

## Module ownership (do not duplicate these elsewhere)

Everything below is in flight or done; `compat/scripts/port-status.sh` prints
the authoritative "what is still missing" list (currently 36 of 100 compiled
C files).

| module(s) | owner | state |
| --- | --- | --- |
| `celt/{modes,rate,quant_bands,bands,celt,pitch,celt_lpc,vq}.rs` | the respective agents | **done**, all probes IDENTICAL |
| `silk/*` decoder DSP (34 files) + foundation | silk-dsp / silk-foundation agents | **done**, `probe_silkdsp`/`probe_silkbase` IDENTICAL |
| public API (`src/coder/**`, `packet.rs`, `repacketizer.rs`, `softclip.rs`) | api agent | **done**; `compat/apicompat` compiles both crates in one binary |
| `celt/celt_decoder.rs` | celt-decoder agent | porting |
| `celt/celt_encoder.rs` | celt-encoder agent | porting |
| SILK decoder core (`decode_core`, `dec_API`, `decode_frame`, `decode_indices`, `decode_parameters`, `decode_pulses`, `decoder_set_fs`, `init_decoder`, `PLC`, `CNG`) | silk-decode agent | porting |
| SILK encoder integer core (`NSQ`, `NSQ_del_dec` **AVX2 path**, `VAD`, `NLSF_encode`/`VQ`/`del_dec_quant`/`VQ_weights_laroia`, `process_NLSFs`, `quant_LTP_gains`, `VQ_WMat_EC`, `A2NLSF`, `encode_indices`, `encode_pulses`, `init_encoder`, `control_*`, `LPC_analysis_filter`, `stereo_*`) | silk-enc-core agent | porting |
| SILK float analysis (`silk/float/*_FLP.c`) | silk-float agent | porting the 24 self-contained files; the 4 that call the encoder core (`find_lpc`, `find_pred_coefs`, `wrappers`, `encode_frame`) wait for the core, by decision — writing them early would leave the crate uncompilable |

Still unassigned: `src/opus/encoder.rs`, `src/opus/decoder.rs` (the real
`opus_encoder.c` / `opus_decoder.c` front ends; the placeholders currently
return `OPUS_INTERNAL_ERROR`) and `silk/enc_API.c` integration.

### What the `src/opus` front ends need (extracted from the C)

* `opus_decoder.c` calls: `celt_decoder_{get_size,init,ctl}`,
  `celt_decode_with_ec`, `celt_decode_with_ec_dred`, `silk_Get_Decoder_Size`,
  `silk_InitDecoder`, `silk_Decode`, `silk_ResetDecoder`, `silk_LoadOSCEModels`,
  the `opus_dred_*` family, `opus_extension_iterator_*`, `opus_packet_parse*`
  (already ported), `opus_pcm_soft_clip_impl`, `opus_select_arch`,
  `celt_exp2`, `celt_float2int16`.
* `opus_encoder.c` calls: `celt_encoder_{get_size,init,ctl}`,
  `celt_encode_with_ec`, `silk_Get_Encoder_Size`, `silk_InitEncoder`,
  `silk_Encode`, `silk_biquad_alt_stride{1,2}`, `silk_{lin2log,log2lin}` and the
  usual `silk_*` integer macros (all already ported).

### DNN/DRED scoping for the front ends

`ENABLE_DRED` is defined, so `opus_decoder.c` references the `opus_dred_*`
family and `opus_encoder.c` can emit DRED.  The objective's packets never carry
DRED, because the encoder only produces it when `OPUS_SET_DRED_DURATION` is
set — a control `audiopus` never issues.  Likewise `silk_LoadOSCEModels` and the
`lpcnet`/OSCE inference only matter above the default decoder complexity.
So the front ends must port the *control flow* faithfully and implement those
DNN/DRED entry points as clearly-marked stubs that behave exactly as the C does
for packets with no DRED extension / no OSCE model loaded.  Any divergence in
that area is not observable for the configurations the acceptance test uses,
and `run-oracle.sh` will confirm it.



### SILK variant selection (verified against the built archive)

`nm reference/lib-ref/libopus.a` shows that the float build compiles **only the
`silx/float/*_FLP.c`** analysis files: `silk_autocorrelation_FLP`,
`silk_burg_modified_FLP`, `silk_pitch_analysis_core_FLP`,
`silk_find_pitch_lags_FLP`, `silk_find_LPC_FLP`, `silk_noise_shape_analysis_FLP`,
`silk_encode_frame_FLP`, `silk_LPC_analysis_filter_FLP`, … —
the entire `silk/fixed/*_FIX.c` set is **not linked at all** (only
`fixed/structs_FIX.h` matters, as a header).  So the SILK encoder port needs:
the common integer files (`NSQ.c`, `NSQ_del_dec.c`, `NLSF_*.c`,
`process_NLSFs.c`, `quant_LTP_gains.c`, `VQ_WMat_EC.c`, `VAD.c`, `enc_API.c`,
`init_encoder.c`, `control_*.c`, `encode_indices.c`, `encode_pulses.c`, …) plus
`silk/float/*_FLP.c`, and none of the `_FIX` implementations.



## Already done outside the codec

* `src/error.rs`, the crate-root newtypes (`Signal`, `Bitrate`, `SampleRate`,
  `Application`, `Channels`, `Bandwidth`, `MutSignals`, `version()`) and
  `src/ffi.rs` — the `audiopus`-compatible surface that does not depend on the
  codec internals.
* `compat/apicompat/` builds `audiopus 0.3.0-rc.0` against the system libopus;
  it will become the drop-in compatibility test (the *same* test source
  compiled against both crates, comparing packets byte for byte).

## Roadmap (bottom-up)

1. CELT foundation: laplace, cwrs, mathops, kiss_fft, mdct, **modes**, **rate**.
2. CELT DSP: pitch, celt_lpc (SIMD-equivalent), quant_bands, vq, bands.
3. CELT decoder, then encoder (`celt/` only, testable via the low-delay
   2.5 ms configuration and the `opus_custom_*` entry points).
4. SILK foundation headers (`typedef.h`, `define.h`, `SigProc_FIX.h`,
   `macros.h`, `structs.h`, `tables.h`), then the decoder, then the encoder.
   The SILK encoder needs the float/SIMD kernels from `docs/rtcd-kernels.md`.
5. Opus layer: `opus_decoder.c`, `opus_encoder.c`, multistream/repacketizer,
   `opus_pcm_soft_clip`.
6. `audiopus`-compatible public API on top (`docs/audiopus-api.md`,
   `docs/internal-api.md`), then a compatibility test that compiles the *same*
   test code against `audiopus` and `oprs`.
7. Final acceptance: the 16-configuration oracle, but run through the Rust
   implementation and diffed against `reference/tools/report-system.txt`.

## Tools

| script | purpose |
| --- | --- |
| `compat/scripts/build-reference.sh` | build the bit-exact reference `libopus.a` |
| `compat/scripts/run-probe.sh <name>` | diff a C probe against its Rust twin |
| `compat/scripts/gen-ffi.py` | regenerate `src/ffi.rs` from `opus_defines.h` |
| `reference/tools/build-oracle.sh` | demonstrate encoder/decoder equality with the system lib |
| `reference/tools/oracle.c` | the 16-configuration bit-exactness oracle |

## Notes

* `reference/` is git-ignored (it holds the 30 MB libopus source tree and the
  built libraries).
* The library must stay dependency-free.
* Every ported module keeps the C names and the original copyright header.

## How to continue

```bash
bash compat/scripts/port-status.sh          # what is still missing (0 of 100 files)
bash compat/scripts/run-all-probes.sh       # every C-vs-Rust probe (21/23 IDENTICAL, see CURRENT STATE)
cargo test --lib                            # 158 unit tests
bash compat/scripts/run-oracle.sh           # THE acceptance test vs the system libopus
bash compat/scripts/run-oracle.sh --reference   # same, vs reference/lib-ref
cargo build --manifest-path compat/apicompat/Cargo.toml && ./compat/apicompat/target/debug/apicompat
```

The last command is the drop-in compatibility proof: one binary instantiating
the same suite against `audiopus` and `oprs`.  It prints
`[audiopus] all API checks passed` and, until the codec internals are ported,
`[oprs] FAILED: … Internal error inside Opus occured` — expected, and it
disappears when `src/opus/{encoder,decoder}.rs` are replaced by the real ports.

Order of work from here: finish `celt_decoder` + `celt_encoder`, then
`src/opus/{encoder,decoder}.rs` → first end-to-end match on the 2.5 ms
low-delay (CELT-only) configuration; then the SILK decoder core and the SILK
encoder (integer core + the four remaining `_FLP` files + `enc_API`) → the
remaining 15 configurations.

## CURRENT STATE — **OBJECTIVE ACHIEVED** (round 113, 2026-10-17 03:50 JST)

* **All 100 compiled C files have a ported Rust module** (`compat/scripts/port-status.sh` reports 0 missing).
* `src/opus/decoder.rs` is a **real port** (1658 lines) and is **bit-exact** (`probe_opus_decoder: IDENTICAL (29149 lines)`).
* `src/opus/encoder.rs` has **landed**: the port of `src/opus_encoder.c` plus its `analysis` sub-module (the port of the out-of-scope-but-required `src/analysis.c` + `src/mlp.c` + `src/mlp_data.c`; `run_analysis` really is needed — `opus_encode_native` runs it whenever `silk_mode.complexity >= 7` and `Fs ∈ [16k,48k]`, i.e. for 9 of the 16 acceptance configurations, and its output drives `CELT_SET_ANALYSIS`/CELT encoder decisions).  `cargo build --release` is green.  The 11 constant tables (`dct_table`, `analysis_window`, `tbands`, `std_feature_bias` + the 7 MLP arrays) are generated mechanically by `compat/scripts/gen-analysis-tables.py` into `compat/out/analysis_tables_rust_paste.txt`.
* `bash compat/scripts/run-acceptance.sh` (new) runs every gate in order: build → `cargo test --lib` → 23 probes → decode half → `run-oracle.sh` (the acceptance criterion).
* **The decoded-PCM half of the ACCEPTANCE TEST is now proven**: `bash compat/scripts/run-dec-half.sh` (new) encodes the 16 `reference/tools/oracle.c` configurations with the **system libopus 1.6.1**, freezes the packets + decode plan into `compat/out/dechalf_trace.bin`, and decodes them both with libopus (`compat/tools/dechalf_dump.c`) and with the Rust port (`compat/rustoracle/src/bin/dec_half.rs`, through the public audiopus-compatible API).  Result at 03:20: `DECODE HALF: BIT-EXACT`, and the C side's 16 per-config `dec_fnv` values equal the full C oracle's (`compat/out/oracle_c.txt`), so the comparison is on exactly the acceptance configurations.  Only the Opus *encoder* is still missing for the full diff.
* **All 23 C-vs-Rust probes are bit-exact** (verified in the 03:25 acceptance run): entropy, laplace, mathops, mathopscheck, mdct, cwrs, kernels, modes, repacketizer, silkbase, silkdsp, silk_decode (2900), silkenc (1587), silk_encode (4803), silkfloat (10250, with the `CHECK_AVX2_STATE` gate back ON so the AVX2 state path is covered again), celt, pitch, vq, quant_bands, bands, celt_decoder, celt_encoder and opus_decoder (29149).
* **ACCEPTANCE STATUS: 15 of the 16 configurations are byte-identical.**  `bash compat/scripts/run-oracle.sh` reports `ACCEPTANCE: DIFFERENT — 4 differing lines`, i.e. only **cfg 0** (`48k/stereo/audio/96k/c10/20ms`): `enc_fnv`/`dec_fnv` differ while `enc_bytes=10982` is identical.  CBR (cfg 9) and CVBR (cfg 10) variants of the same configuration are bit-exact.
* **The cfg-0 divergence is localised to ONE frame**: `compat/tools/enctrace_cfg0.c` + `compat/rustoracle/src/bin/enc_trace.rs` print per-frame `n`/TOC/FNV/packet-hex on both sides and reproduce the oracle's cfg-0 `enc_fnv` exactly (`0x70add2eab788d266` C vs `0xeb719e78e97ed4b9` Rust).  `compat/out/enctrace_diff.txt`: only **frame 42 of 50** differs; frames 0-41 and 43-49 are byte-identical, `n=179` and the TOC `0xfc` (CELT-only stereo 20 ms FB) are identical, payload bytes 0-8 are identical and the divergence starts at **byte offset 9**.  Prime suspects (all VBR-only): the `st->analysis.valid`/`equiv_rate` clamp of `signalBandwidth` in `celt/celt_encoder.c:2604-2620`, the `intra_ener`/`force_intra` and `CELT_SET_ANALYSIS` handoff, and the unconstrained-VBR reservoir arithmetic (`st->vbr_reservoir`/`st->vbr_drift`/`vbr_rate`).
  **Ruled out**: `intra_ener`/`force_intra` (the reference only writes `st->force_intra` from the `CELT_SET_FORCE_INTRA` ctl, which audiopus never calls, so it is a constant 0) and the `compute_equiv_rate` formula (diffed line-for-line against the C, identical).  Because the `signalBandwidth` clamp uses `equiv_rate = st->bitrate_bps - (40*C+20)*((Fs>>3)-20)` and `st->bitrate_bps` is overwritten every frame by `CELT_SET_BITRATE` with the VBR-adjusted rate, the first suspect is now the per-frame VBR rate / `CELT_SET_BITRATE` handoff (instrument `src/celt/celt_encoder.rs`, which has no active owner).
* **Transient divergence ⇒ per-frame analysis output.**  Frame 42 differs but frame 43 is byte-identical again, so the differing quantity cannot be a carried coding state (`lastCodedBands`, `oldBandE`, prefilter, MDCT memory) — it must be recomputed every frame and cross a threshold only at 42.  That points at `tonality_get_info` → `AnalysisInfo` and its consumers: `celt/celt_encoder.c:2611` `signalBandwidth = IMAX(st->analysis.bandwidth, min_bandwidth)` (note **IMAX**, so a larger `analysis_info.bandwidth` in the port changes the coded bands), `celt_encoder.c:2042` (`analysis.tonality`) and `src/opus_encoder.c:1279-1312` (`valid`/`bandwidth`/`music_prob*`/`activity_probability`).  The stateful mask/hysteresis loop that produces `bandwidth` was diffed against the port (`encoder.rs:4884-4940`) and **matches the C line for line**, so the suspicion is the `tonality_get_info` decision logic (`analysis.c:246-310`, the `bandwidth_span = 6` look-around, `valid`/`activity_probability`, `lowECount`/`Etracker`) and the MLP feature path that feeds `tonality`/`music_prob`.
* The public API layer is complete, and `compat/apicompat` compiles the *same* suite against `audiopus` and `oprs` in one binary; `[oprs]` still fails only inside the un-buildable `src/opus/encoder.rs`.

### Round-91 resolutions (the two SILK encoder bugs, both fixed)

* **`probe_silk_encode` (arch 0) — FIXED.**  The la_shape diagnostic was exactly right: `silk_encode_frame_FLP` used `LA_SHAPE_MS * fs_kHz` (5*fs) for both the frame-insertion offset *and* the `x` pointer handed to `silk_noise_shape_analysis_FLP`, but `noise_shape_analysis` internally does `x_ptr = x - psEnc->sCmn.la_shape` and `la_shape` is `3*fs_kHz` for complexity 0 and 2 (`control_codec.c:318,338`).  The window was read `2*fs_kHz` samples too early at cx 0/2, perturbing AR/Gains → `GainsIndices` → everything.  Fix split the two offsets in `src/silk/float/encode_frame_flp.rs` (`la_shape_win = LA_SHAPE_MS*fs_kHz` for insertion/dither/memmove, `la_shape = psEnc.sCmn.la_shape` for the noise-shape offset).
* **`probe_silkenc` / `probe_silkfloat` (arch 4) — FIXED at the arithmetic level.**  Root cause in `src/silk/nsq_del_dec.rs:1687`: the port used one `mm_sign()` for two different sign semantics.  C `silk/x86/NSQ_del_dec_avx2.c:789` calls the **intrinsic** `_mm_sign_epi32(_mm_set1_epi32(QUANT_LEVEL_ADJUST_Q10), q1_Q0)`, whose semantics are `b<0 ? -a : (b==0 ? 0 : a)`, while lines 773/785/822 call the file-local `silk_mm_sign_epi32`, whose semantics are `cond<0 ? -num : num` (`cond==0` keeps `num`, because the helper ORs the condition with 1).  With `q1_Q0 == 0` the port produced `+80` instead of `0`, moving `q1_Q10`/`q2_Q10` from `{100,1044}` to `{20,964}`.  The bias feeds `exc_Q14`/`sxq_Q14`, hence `sLPC_Q14`/`sLF_AR_Q14`/`sDiff_Q14`/`sAR2_Q14` — which is why `pulses`/`Seed`/`lagPrev` matched while the carried state did not.  Fix: added `mm_sign_epi32()` (intrinsic semantics, `nsq_del_dec.rs:1141-1155`) and used it at 1687, keeping `mm_sign`/`mm256_sign` elsewhere.  `probe_silkenc` section 18 covers the AVX2 path (nStates 3/4, shaping order 20/24, 10/20 ms, the `Lambda > 2048` RDO branch, voiced/unvoiced, the gain case that exposed it) and is IDENTICAL.  The `perm`/`shfl` translation and `_mm256_castsi128_si256`'s undefined upper lanes were both checked against C and are correct (only `low128(SS_Q_Q10)` is consumed).

### Round-90 diagnostics that led to those fixes (private runs, outputs kept)

* `probe_silk_encode` **forces `arch = 0` on both sides** (`PROBE_ARCH 0`).  Fails exactly the **15 cases whose complexity ∈ {0, 2}** and is bit-exact for all 63 cases with complexity 1 and 3..10.  First divergence (case 1 = 16 kHz/mono/20 ms/24 kbps/complexity 0, frame 0): every pre-state line, `state`/`flags`/`misc`/`shape`/`ctl`/`stereo` and `nlsf_fn` are identical, then
  `idx Gains=[42 14 4 4]` (C) vs `[26 29 5 4]` (Rust) and `nsq ... sLF_AR=-1448/-2416, sDiff=-656/-1616, rand_seed=1772010356/2062666343`.
  `GainsIndices` are written by `silk_process_gains_FLP`/`silk_gains_quant` **before** NSQ, so the bug is upstream of the NSQ kernels, in the gain chain (`noise_shape_analysis_FLP` → `process_gains_FLP`) or its inputs.  Complexity 0/2 are the rows with `pitchEstimationComplexity = SILK_PE_MIN_COMPLEX`, `shapingLPCOrder = 12`, `la_shape = 3*fs_kHz`, `NLSF_MSVQ_Survivors = 2`, `pitchEstimationLPCOrder = 6` (`silk/control_codec.c:316-345`); note `la_shape = 3*fs_kHz` makes `slope_part = (shapeWinLength - 3*fs)/2` **zero** in `silk/float/noise_shape_analysis_FLP.c:234-243`, i.e. the two `silk_apply_sine_window_FLP` calls have `length == 0`.
* `probe_silkfloat` (arch = 4, i.e. the real dispatch) `case_pred_chain` (complexity 9): `find_pitch_lags_FLP`, `noise_shape_analysis_FLP`, `find_pred_coefs_FLP` and `process_gains_FLP` are **all identical**, and after `silk_NSQ_wrapper_FLP` the `pulses`, `Seed`, `rand_seed` and `lagPrev` are identical too — but `sNSQ.sLF_AR_shp_Q14` 2160 vs 2488, `sNSQ.sDiff_shp_Q14` 1600 vs 68896 and the whole of `sLPC_Q14`/`sAR2_Q14` differ.  That isolates an **AVX2 `silk_NSQ_del_dec` state-update/rounding** bug (nStates = 4); the complexity-2 case (`nStates = 2` → the AVX2 wrapper's `_c` fallback) is identical end to end.
* `probe_silkfloat` `case_encode_frame` (arch = 4) diverged for cfg 0-3 (complexity 2, 2, 8, 10) and passed cfg 4 (8 kHz, complexity 3, silent input).
* Both probes were bit-exact earlier in the session, and `src/silk/nsq_del_dec.rs`, `src/silk/float/encode_frame_flp.rs`, `src/silk/float/find_pred_coefs_flp.rs` and `src/silk/nsq.rs` were all edited between 02:47 and 03:04 — these were in-flight regressions, not a stable baseline.
* `cargo test --lib` had five failures in SILK files; all five were test-expectation bugs (not code) and are now fixed by their owner: `silk::nsq::rand_is_wrapping_mla` (`silk_RAND(seed) = RAND_INCREMENT + seed*RAND_MULTIPLIER`, arguments were swapped), `silk::nsq_del_dec::{hmin_hmax_broadcast, mask_del_dec_matches_c_shift}` (the C macro's real output for nStates=3 is `[0,0,0,-1]`), `silk::a2nlsf::trans_poly_small_case` and `silk::stereo_lr_to_ms::identical_channels_have_no_side` (both asserted one sample past the written range).

### Verified SILK dispatch facts for this host (arch = 4, Intel Xeon E5-2690 v4 / Broadwell)

`reference/opus-1.6.1/config.h`: `OPUS_HAVE_RTCD 1`, `OPUS_X86_MAY_HAVE_AVX2 1`, `OPUS_X86_PRESUME_SSE 1`, `OPUS_X86_PRESUME_SSE2 1`, **`OPUS_X86_PRESUME_SSE4_1` undefined**, `OPUS_X86_PRESUME_AVX2` undefined.  Therefore:

* `silk_NSQ` → `SILK_NSQ_IMPL[4]` = **`silk_NSQ_sse4_1`** (there is no AVX2 NSQ; `silk/x86/main_sse.h:104-141`).
* `silk_NSQ_del_dec` → `SILK_NSQ_DEL_DEC_IMPL[4]` = **`silk_NSQ_del_dec_avx2`**, whose `verify_assumptions` (`silk/x86/NSQ_del_dec_avx2.c:45-70`) needs `fs_kHz ∈ {8,12,16}` and `nStatesDelayedDecision ∈ {3,4}`; for `nStatesDelayedDecision ∈ {1,2}` it calls **`silk_NSQ_del_dec_c`**.  So `silk_NSQ_del_dec_sse4_1` is unreachable at arch 4.
* `nStatesDelayedDecision` / `shapingLPCOrder` by complexity (`silk/control_codec.c:316-385`): 0 → 1/12, 1 → 1/14, 2 → 2/12, 3 → 2/14, 4 → 2/14, 5-6 → 2/16, 7 → 3/20, ≥8 → 4/24.
* **`probe_opus_encoder` (new, owner-created) narrows cfg 0 frame 42 to the coding stage.**  Private run outputs `compat/out/oc_openc_{c,rs}.txt` + `oc_openc_diff.txt`.  At `CE 42` every printed decision/state field is identical on both sides (`ta=10 lcb=20 hfa=0 td=0 pp=624 pg=0 pt=0 ct=0 vr=0 vd=0 vo=0 vc=43 om=3edacc02 ss=ba1f87e4 int=19 sp=4021cacb obe=6ad3738d... ole=...`) and only `rng`/`rf` (the range coder) differ — so no *decision* is wrong; the divergence is inside the coding of that frame after the decisions, with a float input that does not change stored state (next suspects to print: `offsets[]`, `cap[]`, `pulses[]`, `fine_quant[]`, `alloc_trim`, `balance`, `codedBands`, and `quant_fine_energy`'s `error`).  The same probe also exposes (a) a **real off-by-one in constrained-VBR drift** (`vd`/`vo` 1453 vs 1454 while the packet is identical; update at `celt/celt_encoder.c:2515-2516`, suspects `celt_rcp`/`MULT16_32_Q15`/`delta*(1<<lm_diff)`), and (b) a ctl-surface divergence where C returns `OPUS_UNIMPLEMENTED (-5)` for request ids 13..31 that Rust reports as implemented (`GET ... -> 0 v=...`).
* **ROOT-CAUSE MECHANISM for cfg 0 frame 42 (identified): a 1-ULP difference in the float `AnalysisInfo.tonality_slope` flips a *locally coded* value.**  `alloc_trim` is a local, coded with `ec_enc_icdf(enc, alloc_trim, trim_icdf, 7)` at `celt/celt_encoder.c:2420` (byte 9 of the packet is exactly this region), and `alloc_trim_analysis` (`celt_encoder.c:865-949`, called at :2416) subtracts `MAX16(-QCONST16(2.f,8), MIN16(QCONST16(2.f,8), QCONST16(2.f,8)*(analysis->tonality_slope+.05f)))` at line 938 before `trim_index = floor(.5f+trim)` (clamped 0..10).  Because `alloc_trim` is a local, it is absent from the `CE` state dump — which is exactly why every printed field of the new `probe_opus_encoder` matches while only the range coder differs.  A 1-ULP change flips the rounding only when `trim` sits next to a .5 boundary, hence one frame (42), no other state change, and frame 43 clean again; the function is also VBR-sensitive through `equiv_rate` (lines 878-882), matching CBR/CVBR (`cfg 9`/`cfg 10`) being clean.  `tonality_slope` comes from `src/analysis.c:890` (`slope /= 8*8`, accumulating `band_tonality[b]*(b-8)`; Rust `src/opus/encoder.rs:4790`, `:4975-4976`), so the first differing float is expected in the per-band tonality/leakage/band-energy block (`analysis.c` ~780-860, Rust `encoder.rs` ~4780-4880) — check float contraction/accumulation order there (this port emulates FMA with explicit `mul_add`).  Next: dump the raw `f32` bits of `tonality_slope`/`tonality` and `alloc_trim`/`trim` for frames 38-44 on both sides.
* **ANALYSIS LAYER ELIMINATED (round 101).**  `compat/c/probe_analysis_dump.c` + `examples/dump_analysis.rs` dump the raw bits of `run_analysis`/`tonality_get_info` for cfg 0's exact PCM (550 lines: `valid/bandwidth/activity_probability/tonality/tonality_slope/music_prob*`): **C and Rust are IDENTICAL**.  Independently, the complexity sweep (`compat/out/enctrace_cx_{c,rs}.txt`) shows the divergence also at **complexity 6** (frames 42 *and* 44), where `run_analysis` does not run at all (`src/opus_encoder.c:1254` needs `complexity >= 7`), and at complexity 7/8/9/10 at frame 42 only.  So the earlier `tonality_slope` mechanism is **wrong** and the bug is in the **CELT encoder / opus→CELT handoff** at all complexities ≥ 6.  Remaining suspects are the *locally coded* values in the byte-9 region whose inputs are not in the `CE` dump: `alloc_trim` (`celt_encoder.c:2420`, so its non-analysis inputs `X`, `bandLogE`, `st->stereo_saving`, `tf_estimate`, `surround_trim`, `intensity`, `equiv_rate` at `:865-949`), and everything sharing the VBR block that also produces the `vbr_drift` ±1 (`:2509-2526`).  Next instrumentation: raw f32 bits of those quantities for frames 41-45 in `src/celt/celt_encoder.rs` (unowned), then fix and re-check both cfg 0 and the cx-6 sweep.
* **FAILING QUANTITY PINNED: `alloc_trim` (round 105).**  The instrumented reference build (`compat/out/cx_instr_stderr.txt`, from a print-instrumented copy of `celt_encoder.c` whose stdout is byte-identical to the plain run) prints `TR cx0..cx4 f42 alloc_trim 3` and `tell_after_trim 647` for **all five complexity sections (complexity 6..10)**.  `alloc_trim` is a local coded by `ec_enc_icdf(enc, alloc_trim, trim_icdf, 7)` at `celt/celt_encoder.c:2420` (byte-9 region) and is absent from the `CE` state dump, so a wrong Rust `alloc_trim` reproduces every observation (transient single frame, identical state, analysis-independent at cx 6).  Prime hypothesis: a plain porting bug in the pure function `alloc_trim_analysis` (`celt_encoder.c:865-949`, called at `:2416`) — wrong constant/Q-format/loop bound/sign.  Confirmation: print `alloc_trim`/`trim` in `src/celt/celt_encoder.rs` and check it equals 3 at frame 42.
* **TF STAGE PINPOINTED (round 108).**  The instrumented Rust/C traces (`compat/out/tr_c.txt` / `tr_rs.txt`, 2650 lines each, produced with the print-instrumented reference build and a matching instrumented `src/celt/celt_encoder.rs`) show: `tell_after_trim` **matches** (so the earlier `alloc_trim` hypothesis is wrong), while at the failing frame the first divergences are `tf_res_h` (the TF/transient decision hash, C `214171cf` vs Rust `debe8e31`) and consequently `tell_after_tf` (581 vs 590) and `tell_after_dynalloc` (617 vs 626) — a 9-bit difference carried forward.  So the bug is in **`tf_analysis`** (`celt/celt_encoder.c`, a pure function of `bandLogE`/`oldBandE`/`analysis`/`LM`/`C`/`N`/`lambda`/`tf_estimate`), i.e. in the TF resolution/`tf_select` decision — not in `alloc_trim`, not in the analysis layer (550-line dump identical) and not in the VBR rate.  Also in the same diff: `cx4 f0 prefilter_period` 15 (C) vs 0 (Rust), almost certainly an instrumentation-point mismatch since frame 0's packet is byte-identical in the uninstrumented run.
* **FIRST DIFFERING DECISION FOUND: `tf_chan` (round 109).**  The regenerated instrumented traces (`compat/out/tr_diff.txt`) show, at the failing frame, `tf_chan` C `1` vs Rust `0` (chosen in `tf_analysis`, `celt/celt_encoder.c:268`/`:440`, by the strict `unmask > mask_metric` comparison inside the per-channel loop), and everything downstream follows: `tf_res_pre_h`, `tf_res_h` (`214171cf`/`debe8e31`), `tell_after_tf` 581/590, `tell_after_dynalloc` 617/626, `tell_after_trim` 647/655, `bits` 10744/10736, `pulses_h`, `rng`, `ec_tell_frac`, `ec_nbits_total`, `ec_rem`.  The Rust block `src/celt/celt_encoder.rs:854-892` was compared against C `celt_encoder.c:404-442` and is equivalent (`mean = celt_sqrt(mean*maxE*.5*len2)`, `norm = len2/(1e-15+mean)` because `SHL32`/`SHR32` are float no-ops, the `floor`+0/127 clamps, the integer `unmask = 64*unmask*4/(6*(len2-17))`, strict `>`, `mask_metric` from 0, and `EPSILON == 1e-15f` on both sides), so the flip must come from the per-channel `tmp[]`/`mean`/`maxE` computation upstream in the same function — next step: dump those per channel at the failing frame.  Also in the same diff: `X_pre_h` differs in every frame of that section and `prefilter_period` at f0 (15 vs 0) — both are instrumentation artifacts to verify, since those frames' packets are byte-identical in the uninstrumented run.

* **ALL GATES PASS** (`bash compat/scripts/run-acceptance.sh` -> `ACCEPTANCE SUITE: all gates passed.`):
  * `cargo build --release`: OK;  `cargo test --lib --release`: 303 passed / 0 failed;  debug `cargo test --lib`: 302 passed / 0 failed.
  * **all 24 C-vs-Rust probes bit-exact** (entropy … opus_decoder 29149, opus_encoder 2856).
  * decoded-PCM half over the 16 configurations: BIT-EXACT.
  * **`ACCEPTANCE: BIT-EXACT` — the 16-configuration oracle diff is empty against BOTH the system libopus 1.6.1 and the pinned `reference/lib-ref` build: identical packets AND identical decoded PCM for every configuration** (`enc=0xdbce15d74e829a43 dec=0xf73254ad00d572f1` on both sides).
  * `compat/apicompat`: `[audiopus] all API checks passed` and `[oprs] all API checks passed` — the same suite instantiated against both crates.
  * `compat/scripts/port-status.sh`: 0 of 100 compiled C files without a ported Rust module.
* **THE FINAL BUG (cfg 0 frame 42)**: a single wrong table entry — `inv_table[110]` was `4` in the Rust port, `3` in libopus (`src/celt/celt_encoder.rs` vs `celt/celt_encoder.c:293`).  It changed `unmask` (65 vs 66), flipping `tf_chan` (C 1 / Rust 0) at the one frame where `unmask > mask_metric` sat on the boundary; `tf_res` then differed (a 9-bit shift), and frame 43 reconverged because `pf_on == 0` there.  Diagnosed with a print-instrumented copy of `celt_encoder.c` linked into a private archive (`compat/out/libopus-instr.a`) diffed against temporary prints in the Rust encoder; full write-up in `compat/out/FINDING_inv_table.md`.  Fix applied and verified: oracle BIT-EXACT with the one-line change.
* Two probe-side artifacts were also fixed: the `probe_opus_encoder` delay-buffer hash covered floats C does not allocate for mono/RESTRICTED encoders, and `silk::stereo_lr_to_ms`'s test used `(i*97) % 4000 - 2000` on a `usize` (debug-only underflow).

### What is left

1. **Fix the cfg-0 frame-42 divergence** in `src/opus/encoder.rs` (owner working from the localisation above).
2. Fix the two failing `cargo test --lib` expectations in the same file: `opus::encoder::tests::gen_toc_matches_c` (asserts 240 == 64) and `compute_equiv_rate_matches_c`.
3. `probe_opus_encoder` still to be produced (the `run-oracle.sh` diff covers the same ground, so it is lower priority).
4. Then `bash compat/scripts/run-acceptance.sh` must report `ACCEPTANCE: BIT-EXACT` (gate 5) with every other gate green.

Useful API notes collected from the porters: `silk_stereo_LR_to_MS` takes the whole `inputBuf` slice; `silk_NSQ`/`silk_NSQ_del_dec` take the whole 2x16 `PredCoef_Q12` array; `celt_encoder` owns its `Box<CELTMode>` and `celt_encoder_init` creates it internally; `celt_encoder_ctl` splits the variadic C call into `&mut i32` plus separate `_analysis`/`_silk_info`/`_energy_mask` entry points; `RESYNTH` is **not** compiled, so the CELT encoder has no dependency on the decoder.

## Multistream (active goal `goal-5afa77b0`, round 1)

**Why:** `audiopus 0.3.0-rc.0` declares a `multistream = []` Cargo feature but ships **no** multistream code — `src/coder/` only has `encoder.rs`/`decoder.rs`, and the only multistream item in the crate is `repacketizer::multistream_packet_pad/unpad` (already ported and bit-exact in `probe_repacketizer`).  So the audiopus-compatible surface is complete (`compat/apicompat` passes for both crates); multistream is the one *practical* gap, implemented here as an extension.

**Porting (in progress by agent `1456efb2`):** `reference/opus-1.6.1/src/opus_multistream.c` (91 lines: `ChannelLayout`, `validate_layout`, `get_{left,right,mono}_channel`) + `opus_multistream_encoder.c` (1363: `surround_analysis`, `*_get_size`, `*_init`, `*_create` and the `surround_*` variants, `opus_multistream_encode_native`, `encode`/`encode24`/`encode_float`, `ctl`, `destroy`) + `opus_multistream_decoder.c` (565: `*_get_size`/`_init`/`_create`, `opus_multistream_decode_native`, `decode`/`decode24`/`decode_float`, `ctl`, `destroy`) → `src/opus/multistream{,_encoder,_decoder}.rs`, verified by the new `compat/c/probe_multistream.c` + `examples/probe_multistream.rs` (`bash compat/scripts/run-probe.sh multistream`) over 3-8 channels, explicit mappings, 5.1/7.1 surround (`mapping_family` 0/1), 2.5-60 ms, 8-48 kHz, VOIP/AUDIO/LOWDELAY, VBR/CBR/CVBR, complexity 0/5/10, DTX/FEC, forced bandwidth, the whole ctl switch, and decoder FEC/loss/`decode_float`/`decode24` plus error return codes.  `reference/lib-ref` exports the 12 `opus_multistream_*` symbols, so the C oracle is available.

**Deliberately out of scope (with reasons), to be stated in the docs/README:**
* **DRED / LPCNet deep PLC / OSCE / LossGen** — `reference/opus-1.6.1/dnn/` is **470,060 lines**, almost all trained weight tables.  Decoder-side deep PLC / OSCE / blind-BWE is **unreachable** through the audiopus API (its `Decoder` exposes no ctl passthrough: only `new`/`decode`/`decode_float`/`nb_samples`/`last_packet_duration`/`pitch`/`gain`/`set_gain`/`size`).  **DRED is reachable** via the public generic passthrough `Encoder::set_encoder_ctl_request(OPUS_SET_DRED_DURATION_REQUEST, n)`; enabling it produces DRED latents from stubs, so packets would differ from libopus — this must be documented explicitly rather than silently divergent.
* **projection API** (`mapping_matrix.c`, 997 lines) — not exposed by audiopus either.
* `OSCE_BWE` blind-BWE DSP — no audiopus API to enable it.

**Known measurable cosmetic difference (left as is deliberately):** `Decoder::size()` / `coder::size()` return the *autotools reference build* layout (191,580 / 200,348) because `probe_opus_decoder` compares `OpusDecoder::get_size` directly against `reference/lib-ref`; audiopus-on-this-machine (meson system libopus) reports 154,828 / 163,596 (the meson config drops the `ENABLE_OSCE_BWE` fields).  No acceptance path compares the value.  Making the *public* `coder::size()` return the system numbers is possible but would disagree with the internal reference-pinned value, so it is left documented instead.

### Multistream round 5 status (files landed and independently verified)

* **Step 1 landed**: `src/opus/multistream.rs` (10,988 bytes) now holds `ChannelLayout` + `validate_layout` + `get_left_channel`/`get_right_channel`/`get_mono_channel` + the `OpusMSEncoder`/`OpusMSDecoder` struct definitions, and `src/opus/mod.rs` declares `pub mod multistream;` with the C→Rust mapping row.  `cargo build --release` is green and the landed module causes no regression (`run-oracle.sh` → `ACCEPTANCE: BIT-EXACT`; `probe_opus_encoder` → `IDENTICAL (2856 lines)`).
* **Independently verified line-by-line against C** (`reference/opus-1.6.1/src/opus_multistream.c:41-91`): all four ChannelLayout helpers are identical (`max_channel = nb_streams + nb_coupled_streams`, `>255` → 0, `mapping[i] >= max_channel && != 255` → 0; the three `get_*_channel` helpers use `prev<0 ? 0 : prev+1` and the `stream_id*2`, `stream_id*2+1`, `stream_id + nb_coupled_streams` predicates, returning -1 when absent), and `ChannelLayout.mapping` is `[u8; 256]`, so the `255` sentinel is representable.
* **Ready-made verification assets** (all produced this session, none touching the agent's files):
  * `compat/tools/ms_oracle.c` + `compat/scripts/multistream-system-check.sh` → `MULTISTREAM BASELINE: OK`: `reference/lib-ref` and the **system libopus 1.6.1** produce identical multistream packets AND decoded PCM for 5 configurations, so reference-build equality implies system-libopus equality.
  * Reference `enc_fnv`/`dec_fnv` targets for the Rust port (see `compat/out/ms_ref.txt`): cfg 0 6ch/5.1 `5a3f8509f1b9f024`/`c75855ec0f20e5bf`; cfg 1 8ch/7.1 `60cf796ba1425def`/`45c214acb56a8df2`; cfg 2 4ch explicit `365bd0830f316c06`/`fc1a4f53cd07082c`; cfg 3 3ch coupled `ca5b1fb526a75d4a`/`b33d9c9d45080045`; cfg 4 5ch/5.0 `98c185870f05997e`/`7f5ee114a2a7eddf`.
  * `compat/tools/ms_sizes.c` + `compat/out/ms_sizes_{ref,sys}.txt`: the `get_size` expectation tables.  **Encoder sizes agree between the two libraries; decoder sizes differ** (they embed one `sizeof(OpusDecoder)` per stream, and the meson system build drops the `ENABLE_OSCE_BWE` fields) — the probe links lib-ref, so the port must match the *reference* numbers (consistent with our reference-pinned `OpusDecoder::get_size`).

### Multistream round 8: the Rust decoder is verified bit-exact (independently of the probe)

`bash compat/scripts/multistream-dec-check.sh` → **`MULTISTREAM DECODER: BIT-EXACT (5 configurations)`**.

* `compat/tools/ms_oracle.c` (public API, linked against `reference/lib-ref`) encodes 6ch/5.1, 8ch/7.1, 4ch-explicit, 3ch-coupled and 5ch/5.0 with the oracle's deterministic PCM and (via `MS_TRACE=...`) dumps the packets to `compat/out/ms_trace.bin`; the same tool prints the C `dec_fnv` values.  `examples/ms_dec_check.rs` reads that trace, creates the Rust `OpusMSDecoder` and decodes: all five `dec_fnv` match C exactly (`c75855ec0f20e5bf`, `45c214acb56a8df2`, `fc1a4f53cd07082c`, `b33d9c9d45080045`, `7f5ee114a2a7eddf`, 28800 samples each).  `MS_TRACE` unset leaves the tool's output byte-identical, so the baseline check is unaffected.
* This verifies the multistream demux / coupled+mono channel distribution path before the Rust encoder exists.  Still to cover there (the agent's `probe_multistream`): FEC (`decode_fec=1`), packet loss (`None`), `decode_float`/`decode24`, malformed packets, decoder ctls.
* Wiring note: `OpusMSDecoder` is `pub struct` in `src/opus/multistream.rs:233` while its `impl` lives in `src/opus/multistream_decoder.rs` (imported privately), so the type is currently reachable only as `oprs::opus::multistream::OpusMSDecoder`.

### Multistream round 10: BOTH halves are bit-exact (independently verified)

* **`bash compat/scripts/multistream-enc-check.sh` -> `MULTISTREAM ENCODER: BIT-EXACT (5 configurations)`.**  `compat/tools/ms_oracle.c` (C, linked against `reference/lib-ref`) and `examples/ms_enc_check.rs` (Rust port, identical PCM/ctls) produce **byte-identical packets** for 6ch/5.1, 8ch/7.1, 4ch-explicit, 3ch-coupled and 5ch/5.0: `enc_fnv` `5a3f8509f1b9f024` / `60cf796ba1425def` / `365bd0830f316c06` / `ca5b1fb526a75d4a` / `98c185870f05997e`, with `enc_bytes` 8768 / 10096 / 4702 / 3493 / 6059 all equal.  This covers `opus_multistream_surround_encoder_create` (mapping/streams/coupled computation), `surround_analysis`, the per-stream `OpusEncoder` fan-out and the shared range coder.
* **`bash compat/scripts/multistream-dec-check.sh` -> `MULTISTREAM DECODER: BIT-EXACT (5 configurations)`** (round 8): the Rust decoder reproduces the C decoded PCM exactly for the same five configurations (`c75855ec0f20e5bf`, `45c214acb56a8df2`, `fc1a4f53cd07082c`, `b33d9c9d45080045`, `7f5ee114a2a7eddf`).
* **No regression** from adding the modules: `run-oracle.sh` -> `ACCEPTANCE: BIT-EXACT` (16 configurations), `probe_opus_encoder` -> `IDENTICAL (2856 lines)`.
* `bash compat/scripts/run-acceptance.sh` now also runs the multistream baseline + encoder + decoder checks as gates.
* Remaining for the goal: the full `compat/c/probe_multistream.c` + `examples/probe_multistream.rs` matrix (2.5-60 ms, VBR/CBR/CVBR, DTX, FEC, decoder FEC/loss, malformed packets, the whole ctl switch, error return codes), unit tests, and the final green `run-acceptance.sh` with the new probe as #25.

### Multistream round 13: the independent checks now cover CBR / DTX / FEC (8 configurations)

`compat/tools/ms_oracle.c` and `examples/ms_enc_check.rs` were extended with three more configurations and the matching ctls (`OPUS_SET_VBR`, `OPUS_SET_DTX`, `OPUS_SET_INBAND_FEC`, `OPUS_SET_PACKET_LOSS_PERC`); the two check scripts no longer truncate to the first five rows:

* `MULTISTREAM ENCODER: BIT-EXACT (8 configurations)` — 6ch/5.1 CBR 64k (VBR off), 6ch/5.1 DTX VOIP 32k, 8ch/7.1 in-band FEC + 20 % packet loss, on top of the original five.
* `MULTISTREAM DECODER: BIT-EXACT (8 configurations)` — the Rust decoder reproduces C's PCM for those packets too (including DTX packets and FEC/LBRR-bearing packets).
* `MULTISTREAM BASELINE: OK` — and the **system libopus 1.6.1 agrees with the reference build on all 8** (packets + decoded PCM), so the CBR/DTX/FEC multistream paths are covered by the reference-vs-system equivalence as well.
* Remaining coverage to come from the agent's `probe_multistream`: frame sizes 2.5-60 ms (the independent checks use 20 ms), CVBR, forced bandwidth, decoder `decode_fec=1` recovery and `NULL`-packet concealment, malformed packets, the full ctl switch and error return codes.

### Multistream goal COMPLETE (round 19) — final acceptance

`bash compat/scripts/run-acceptance.sh` -> **`ACCEPTANCE SUITE: all gates passed`** (exit 0):

```
-- cargo build --release: OK
-- cargo test --lib --release: OK            (315 passed)
-- all C-vs-Rust probes: OK                  (all 25 probes bit-exact, incl. probe_multistream: IDENTICAL (1754 lines))
-- decoded-PCM half (16 configurations): OK  (BIT-EXACT)
-- multistream baseline (reference build == system libopus): OK
-- multistream encoder vs C reference: OK     (BIT-EXACT, 8 configurations)
-- multistream decoder vs C packets: OK       (BIT-EXACT, 8 configurations)
-- ACCEPTANCE: 16-configuration oracle diff: OK  (BIT-EXACT)
ACCEPTANCE SUITE: all gates passed.
```

* `probe_multistream` (new, C+Rust): 68-config row table + `get_size`/error paths + `msctl` + decoder FEC/PLC/float/int24/get_state + malformed inputs, `IDENTICAL (1754 lines)`.  The C side was verified deterministic (two runs identical) before the Rust twin landed.
* **One follow-up defect fixed in the harness**: `multistream-system-check.sh` step 2 compared the probe output between the reference build and the system libopus byte-for-byte, which fails only on the `decg …` lines — `opus_multistream_decoder_get_size()` embeds one `sizeof(OpusDecoder)` per stream, and the installed meson libopus drops the `ENABLE_OSCE_BWE` fields (ref 191,856 vs system 155,104 for 1 stream), exactly the divergence already documented for the single-stream `size()`.  Step 2 now filters only those 37 `decg` size lines and reports them as the documented difference; every behavioural line must still agree.  (Found by the porting agent, not by me: my earlier confirmation had suppressed stderr.)
* `run-acceptance.sh`'s probe-gate label was corrected from "23 C-vs-Rust probes" to "all C-vs-Rust probes" (the count is 25).

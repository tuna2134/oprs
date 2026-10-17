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

## Performance goal (goal-59ea80ae): audiopus (native libopus) vs oprs — measured state

Harness: `compat/bench` (standalone crate, both backends linked through one macro-instantiated workload body, ABBA-interleaved, adaptive reps ≥120 ms per row, ratio = oprs median / audiopus median, sorted summary; oprs-only multistream section).  Evidence files (all with provenance: revision, `src/` sha256, rustflags, rustc/libopus versions, repetition counts):
* `compat/out/bench_baseline.txt` — frozen `git HEAD` (`99dd5b0`, archived via `git archive` into `compat/bench/frozen/oprs`, runnable via `compat/bench/frozen/run.sh`), default flags: **overall geomean 2.381**
* `compat/out/bench_after.txt` — current working tree, default flags: **2.420** (i.e. no gain; noise)
* `compat/out/bench_native.txt` — current working tree, `RUSTFLAGS="-C target-cpu=native"`: **1.833**

Per-workload medians (frozen HEAD, default flags): `enc_i16` 160 rows median 2.720 worst 5.189; `enc_f32` 36 rows median 2.484 worst 5.254; `dec_i16` 57 rows median **2.928** worst 4.218 (`48k/2ch/20ms/audio/128000bps/fec`); `dec_f32` median 1.633; `plc_i16/f32` ≈1.55–1.63; `repacketizer` 3.117 (its isolated `Repacketizer::new()+drop` is 5.447 on a 258 ns op); **`packet_parse` 0.739 and `softclip` 0.790 are FASTER in oprs**.  340/343 rows slower.

Ruled out by measurement: **heap allocations** (CELT encode went 999 → 384 allocs/frame, 473 KB → 363 KB, with *no* throughput change; the C uses `VARDECL`/stack, the port had used `vec![…]`), **LTO / codegen-units** (no effect), and small structural/bounds-check tweaks (below the ~5 % measurement noise).  Established positive lever: **`RUSTFLAGS="-C target-cpu=native"` ≈ +34–64 % on CELT/hybrid encode rows and +3 % on SILK-only rows, with bit-exactness preserved** (verified on `probe_celt_encoder`, `probe_celt`, `probe_vq`, `probe_kernels`, `probe_silkenc`); documented in `README.md`.

Remaining gap (1.83x) is compute: the port emulates libopus' x86 kernels with scalar lane arrays, while the C executes real intrinsics.  Planned order of intrinsic transcription (one kernel per batch, revert if the dominated `bench_min` row does not move ≥10 %, gates = 25 probes + oracle + 315 tests after every batch, plus `probe_silkenc`/`probe_silk_encode` for the SILK ones): `celt_pitch_xcorr_avx2` → `celt_inner_prod_sse`/`dual_inner_prod` → `comb_filter_const_sse` → `silk_inner_product_FLP_avx2` (keep the `f64` `mul_add`s — load-bearing) → `silk_NSQ_del_dec_avx2` (the 5x SILK rows) → `op_pvq_search_sse`.

Diagnostics built for this goal (keep): `examples/bench_min.rs` (oprs-only RTF per row), `examples/alloc_count.rs` (allocs/frame), `examples/alloc_trace.rs` (backtrace at the N-th allocation — this is how `interleave_hadamard`/`op_pvq_search_sse`/`clt_mdct_forward_c`/`comb_filter_impl` were located).  One regression was caught and fixed during this work: an `nsq_del_dec.rs` change made `probe_silkenc` differ in `nsq.sLTP_shp_Q14_frame` and was reverted by the owner.

## Decode-performance goal (goal-3234eca7): state at hand-off

**Target**: the decoder is the largest remaining gap after the CELT `mul_add`/`fmaf` fix (`compat/out/bench_fma.txt`, overall geomean 2.420 -> 2.064; encode median 1.97, **decode median 2.95**, worst `dec_i16 48k/2ch/20ms/audio/128000bps+fec` 4.245, `dec_f32` median 1.59).

**Hypothesis tested and FALSIFIED**: per-frame heap allocation.  The decode path had ~257 allocations/frame (253 KB) while a decode call costs only ~9.2 µs in oprs vs ~2.2 µs in libopus, so 257 malloc/free pairs looked like the whole gap.  A batch converting the per-band allocators (`op_pvq_search*`, `alg_unquant`, `celt_iir`) plus `celt.rs`/`vq.rs`/`celt_decoder.rs`/`opus/decoder.rs` temporaries to stack arrays cut decode allocations **257 -> 37 (-86 %)** while decode throughput moved only ~+5 % (inside the ±5–10 % noise of `examples/bench_min.rs`).  Same conclusion as for encode: allocation is not the bottleneck for this port.

**Therefore the remaining decode gap is compute**, consistent with the CELT FMA fix not helping decode: the decoder's hot kernels are the ones the C implements with **SSE**, not FMA — `celt_inner_prod_sse`, `dual_inner_prod`, `comb_filter_const_sse` (all `src/celt/kernels.rs`), plus the SILK-decoder dispatched helpers (`silk_LPC_analysis_filter`, `silk_inner_prod_aligned`, resampler).

**Next step (agreed, not yet started)**: transcribe those SSE bodies with `core::arch::x86_64` intrinsics and runtime dispatch (`is_x86_feature_detected!("sse4.1"/"sse2")`), keeping the scalar emulation as the fallback — one kernel per batch, a `bench_min` decode row must move ≥10 % or the batch is reverted, and the gate set is 25 probes + 16-config oracle + 315 tests (plus `probe_kernels`/`probe_celt`/`probe_celt_decoder`/`probe_bands`/`probe_mdct`/`probe_silk_decode`/`probe_silkdsp`).  Also worth checking `nm target/release/liboprs.rlib | grep fmaf` for any decode-path `mul_add` still lowered to a libm call.

**Verification discipline learned in this session**: while the optimisation agent is editing, `run-all-probes.sh` intermittently reports failures that vanish on a re-run (three occurrences); always re-run before treating a red probe as a regression.  Confirmed green at hand-off: build, all 25 probes, `ACCEPTANCE: BIT-EXACT`, 315 tests, decode allocations 37/frame.

## Remaining-performance-items goal (goal-28de2839): scope and state

Four follow-ups requested by the user after the decode investigation, with the acceptance rules agreed with the agents (gates: 25 probes + 16-config oracle + 315 tests after every batch, plus the module-specific probes; a batch counts only if `bench_min`/`compat/bench` moves the dominated row ≥10 %, except item 3 where verified-but-small wins are accepted; `src/silk/nsq_del_dec.rs` is owned exclusively by one agent because two earlier attempts there broke bit-exactness).

1. **Real-decode rows in the harness** (`compat/bench`, owner: benchmark agent).  The A/B `dec_i16`/`dec_f32` rows measure `decode(..., fec=true)` — the **concealment/PLC path** — not real packet decoding, so "decode is 2.95x" is a PLC ratio (locally: real decode 88.5 µs/frame = 226x, `fec=true` 52.4 µs/frame = 381x, and neither matches the harness absolutes, which may not be a full 20 ms frame).  Task: add separate real-decode (`fec=false`) rows, keep the FEC/PLC rows, state the exact call shape/frame size per row in the header, and regenerate `bench_baseline.txt` (frozen HEAD — HEAD has moved to `2b58d7a`), `bench_after.txt` and `bench_native.txt` with provenance.  **State: harness sources being edited, output files not yet regenerated (their header still says `decode_fec : true on every decode call`).**
2. **`silk_NSQ_del_dec_avx2` full intrinsic transcription** (`src/silk/nsq_del_dec.rs`, owner: new agent `f68394ad`).  Instrumented cost ≈200 µs/call ≈ 2/3 of the 8 kHz SILK frame, and the SILK-only 8/12 kHz encode rows are the worst in the A/B (2.6–3.6, worst 5.19).  Approach: transcribe `silk/x86/NSQ_del_dec_avx2.c` intrinsic-for-intrinsic with `core::arch::x86_64`, `#[target_feature]` + cached `is_x86_feature_detected!` dispatch, emulation kept as the fallback; sub-kernel at a time with `probe_silkenc`/`probe_silk_encode`/`probe_silkfloat` after each.  **State: not started (file untouched at 08:35).**
3. **SILK-float FMA dispatch + remaining per-frame allocations** (owner: optimisation agent).  The FMA dispatch for `silk_inner_product_FLP_avx2` (+3.4 % bench_min, +4.5–5 % tight loop) is **applied and verified**: `probe_silkfloat` 10250 IDENTICAL and, run by the orchestrator, all 25 probes bit-exact + `ACCEPTANCE: BIT-EXACT` + 315 tests.  Remaining allocation sites to convert: `celt::entenc::ec_enc_init`, `analysis::tonality_analysis`, `celt::bands::quant_all_bands`'s `ec_ctx::clone` (theta_rdo path), `celt::mdct::clt_mdct_forward_c`.  **State: in progress (`bands.rs` being edited; `alloc_count` still 98 encode / 37 decode per frame).**
4. **`Repacketizer::new()` cost** (owner: optimisation agent).  A/B: 251 ns vs libopus 47 ns (ratio 5.3) for `new()`+drop, paid every `repacketizer` iteration; `probe_repacketizer` must stay IDENTICAL and a measured breakdown is an acceptable answer.  **State: not started.**

Cumulative performance position before these four items (all verified): CELT `mul_add`→libm `fmaf` fix recovered the `-C target-cpu=native` CELT/hybrid gain in the default build (A/B overall geomean 2.381 frozen → **2.064**; `enc_i16` median 2.753 → **1.967**); allocation batches cut encode 999 → **98** and decode 327 → **37** allocs/frame; decode on the 48k stereo row improved 208.9 → **~238x RT** (+14 %).

## Residual-performance goal (goal-505bb50a): state

Third optimisation goal, opened after the harness v2 (457 rows: real decode / FEC / PLC separated) gave the current worst rows at src `6b069266` — they are all **encode** rows plus repacketizer:
`enc_f32 8k/mono/20ms/lowdelay` 2.617 · `enc_i16 8k/mono/20ms/lowdelay` 2.513 · `enc_* 8k/mono/10ms/lowdelay` 2.450 · `enc_i16 12k/mono/{20,40,60}ms/audio` 2.387–2.405 · `repacketizer` 2.997/2.887 · `repkt_new` 3.951. No decode row is in the worst-10 any more (real-decode medians: `dec_i16` 1.303, `dec_f32` 1.229; oprs is *faster* on 16 kHz hybrid 0.43–0.56 and 48 kHz mono SILK 0.83).

**DONE — `Repacketizer` container redesign (owner: optimisation agent).** `[Vec<u8>; 48]`×2 → first `[Option<Box<[u8]>>; 48]`×2 (241 → 151.6 ns, 1888 B), then the sparse container form. Final measurement, run by the orchestrator from the agent's scratch profiler (`examples/prof_repkt.rs`, min of 7, release):
```
size_of::<OpusRepacketizer>() = 400 B      (was 2656 B → 1888 B; libopus layout is 1120 B)
Repacketizer::new() + drop              25.8 ns/op   (was 241 ns; libopus measures 47 ns)
OpusRepacketizer::default() + drop       7.7 ns/op
new() + cat3 + out_range + drop        521.7 ns/op   (wrapper overhead ≈44 ns)
```
Verified by the orchestrator on this change: `probe_repacketizer` IDENTICAL (748 lines), all 25 probes bit-exact, `ACCEPTANCE: BIT-EXACT`, 315 tests. Still to do for this item: re-run `compat/bench` to record the `repacketizer`/`repkt_new` rows (expected below 1.0 now).

**PENDING (assigned, not started at hand-off).**
* Encode residuals (owner: optimisation agent `e6839d36`): the 8 kHz `/lowdelay` rows are `MODE_CELT_ONLY` and the `12k/.../audio` rows share the CELT-only timing signature, so this is CELT-side, not SILK NSQ. Suggested order: check `nm target/release/liboprs.rlib | grep fmaf` for a libm call on *this* path (the CELT `xcorr` FMA fix may not be reached because the 8 kHz CELT-only prefilter does not run), then `clt_mdct_forward/backward`, then `clt_compute_allocation`/`quant_all_bands`/`alg_quant`, then use `objdump` to find scalar where C has SSE.
* Decode gaps (owner: new agent `44430378`): 48 kHz fullband stereo CELT 1.25, 8 kHz narrowband 1.30, 8 kHz 2ch 2.5 ms 1.4–1.5. Attribution phase (gdb sampling + a scratch decode loop, since `perf` is unavailable). Known: allocations are not the cause (327 → 17/frame gave ~+5 %); `celt_inner_prod`/`dual_inner_prod` are already packed SSE; `comb_filter_const` is scalar but ~2 % of decode; `celt_decode_with_ec_dred` is 82–86 % of decode time. Baseline `bench_min` decoders: 48k stereo 236.6x, 48k mono voip 487.9x, 48k LD 166.8x, 16k mono 1996.9x, 8k mono 3812x.

**Housekeeping before the next commit**: the agents' scratch files are untracked and should be removed or ignored — `examples/prof_repkt.rs`, `examples/scratch_dec_loop.rs`, `prof-scratch/`, `repacketizer.rs.bak` (earlier scratch of this kind was deleted by its owner at the end of each task).

Also committed in `8d53a14` (by the user): the README performance section was rewritten by the orchestrator to match the measurements — decode paths must be measured separately (with `fec=true` both libraries conceal instead of decoding), the A/B per-workload medians, the corrected `-C target-cpu=native` claim (**now ~3–5 % on encode and nothing on decode**, because the tree carries explicit runtime-dispatched SIMD), and the allocation numbers (encode 999 → 14, decode 327 → 17 per frame).

### goal-505bb50a — final numbers for the two closed items

**Repacketizer (DONE).** `frames`/`paddings` are now `Vec<Option<Box<[u8]>>>` grown lazily (`grow_frames`/`grow_paddings` at the single `cat` write cluster, sized `base + max(parsed.count, curr_nb_frames)` so the `cnf` loop and the `while i < nb_frames` loops in `opus_packet_unpad`/`opus_multistream_packet_unpad` stay in range; reads via `.get(i).and_then(|o| o.as_deref()).unwrap_or(&[])` at the four sites).  Struct 2656 → 1888 → **400 B**.  `new()`+drop 241 → 151.6 → **42.7 ns** (min of 7; the orchestrator's run of the same scratch loop measured 25.8 ns — the two agree that construction is now *faster than libopus' 47 ns*).  Harness `BENCH_FILTER=repacketizer` (`compat/out/bench_repkt_after.txt`): `repkt_new` **0.726** (oprs 41 ns vs audiopus 56 ns), `repacketizer` rows **2.997 → 2.399** and **2.887 → 2.421**.  Probes `repacketizer` 748 / `multistream` 1754 / `opus_encoder` 2856 / `opus_decoder` 29149 IDENTICAL; 25 probes + oracle + 315 tests green.  **Remaining 2.4–2.5x** on that row is `cat`'s per-frame `to_vec().into_boxed_slice()`; going below needs the C's borrowed-pointer contract or one shared backing buffer (not attempted).

**8 kHz `/lowdelay` + 12 kHz `/audio` encode residuals (localised, NOT fixed).**  Codegen is ruled out by measurement: forcing `+avx2,+fma` crate-wide buys ≤5 % (8k/20ms −2.8 %, 8k/10ms −9.4 %, 12k/20ms −5.0 %, 12k/60ms −3.5 %) and `-C target-cpu=native` is no better — all inside the ±10 % noise of this shared box.  A control sweep at the same internal CELT `N = 960` shows 8k **147 µs** < 16k **181 µs** < 48k **195 µs**: there is **no 8 kHz pathology**, and the excess over libopus grows with the number of *real input samples* (8k: aud 51 571 vs opr 134 946 ns → 83 µs excess).  The suspect is therefore the **per-input-sample opus-layer work**, not the rate-independent CELT kernels: the `upsample = 6` slow `celt_preemphasis` branch plus its 960-sample zero-fill, the analysis FFT, and the delay-`pcm_buf` copies.  No stage-level breakdown was obtained (instrumentation scripting failed twice); the anchors for whoever picks it up: timers at `celt_encode_with_ec` entry, before `/* Find pitch period and gain */`, after the prefilter block, before the `freq`/`bandE` declarations, before `/* Residual quantisation */`, and before `let error_code = ec.get_error()`, accumulating into a `pub static [AtomicU64; 5]` every 2000 calls.

State at hand-off: gates green (25 probes, oracle BIT-EXACT, 315 tests), `alloc_count` 14.01 encode / 17 decode per frame at 48 k stereo, `bench_min` 48k/stereo/audio 43.9x enc / 198.1x dec, 48k/mono/voip 89.4 / 466.8, 8k/mono/voip 169.8 / 3433.  **The only uncommitted `src` delta is `src/opus/repacketizer.rs`.**  Untracked leftovers to remove before the next commit: `examples/scratch_dec_loop.rs`, `prof-scratch/` (the decode agent still owns these), and any remaining scratch from this batch.

### goal-505bb50a — decode attribution (item B, "identify" half DONE)

Measured by the decode agent on the real `bench` corpus with temporary `Instant`/stage instrumentation (oprs side only so far; a C-side SIGPROF/`ITIMER_PROF` RIP-sampling profiler in a throwaway crate linking audiopus was being built to get the symbol-by-symbol comparison):

**48 kHz fullband stereo** (`dec_i16 48k/2ch/20ms/audio`, A/B ratio 1.25):
```
celt_decode_with_ec_dred 100 %
  quant_all_bands      66.6 %   <- largest
    alg_unquant        37.6 %   <- single largest (PVQ decode)
    quant_band/partition 29 %
  celt_synthesis       21.3 %   (clt_mdct_backward 17.7 %)
  deemphasis            4.6 %
  clt_compute_allocation 2.2 %
  comb_filter          ~0 %     (postfilter gains are 0 on real packets — unlike synthetic transients)
```
**8 kHz narrowband** (`dec_i16 8k/mono/20ms/voip`, A/B ratio 1.30):
```
silk_Decode  83 % of the frame
  silk_decode_pulses  40.9 %
  silk_decode_core    35.3 %
  decode_parameters    9.9 %
  decode_indices       4.4 %
  (the CELT wrapper is irrelevant at 8 kHz)
```

So the two decode gaps have **different** targets: 48 kHz is `alg_unquant` (PVQ) + `clt_mdct_backward`, 8 kHz is the SILK decoder body (`silk_decode_pulses` + `silk_decode_core` = 76 %).  Note the earlier assumption that `comb_filter`/`comb_filter_const` mattered was based on synthetic transients; on real packets it is ~0 % and should be deprioritised.  No fix was landed at hand-off.  Scratch to remove when the agent finishes: `prof-scratch/`, `examples/scratch_dec_loop.rs`, `asm_scratch.txt`.

#### Symbol-level confirmation (RIP sampling, 48 kHz stereo decode)

The decode agent's SIGPROF/`ITIMER_PROF` sampler (throwaway crate in `prof-scratch/sampler/`, links `oprs`) produced 2,985 RIP samples (`rips_oprs_48s.txt` + `base_oprs_48s.txt`).  Symbolised with `addr2line -f -C -e prof-scratch/sampler/target/release/sampler <rip-base>`:

```
4.9 %  oprs::celt::vq::exp_rotation                                  (5 addresses — largest cluster; inside alg_unquant)
2.7 %  oprs::celt::cwrs::decode_pulses
2.6 %  oprs::celt::celt_decoder::deemphasis
1.8 %  <oprs::opus::decoder::OpusDecoder>::decode
0.8 %  compiler_builtins::math::libm_math::arch::x86::detect::load_x86_features
```

This confirms the stage attribution (`alg_unquant` 37.6 %, `clt_mdct_backward` 17.7 %) and adds a **Rust-only, arithmetic-free lead**: CPU-feature detection is being executed **repeatedly inside the decode hot path** (the C caches `opus_select_arch()`), i.e. 0.8 % of samples in `load_x86_features`.  Fix to try first: cache the `is_x86_feature_detected!` result per kernel group (`OnceLock`/`AtomicU8`) or hoist it out of the per-band loop wherever the decode dispatch sits — also check for a `mul_add`→`fmaf`-driven call into compiler-builtins per sample.  Priority order for the 48 kHz path after that: `vq::exp_rotation` (≈5 %, part of `alg_unquant`), then `clt_mdct_backward`.  The 8 kHz narrowband path is a different target (SILK decoder: `silk_decode_pulses` 40.9 % + `silk_decode_core` 35.3 %).

#### C-side sampling and the C-vs-oprs asymmetry (48 kHz stereo decode)

The same agent built a **C-side** sampler with the symbol-bearing static libopus from `reference/` (`prof-scratch/sampler-c/target/release/sampler-c`, RIP samples `rips_cstatic_48s_{1s,4s}.txt` + `maps_cstatic_48s_1s.txt`).  Both sides symbolised (C 2,978 samples; oprs 2,985):

```
C (libopus)                              oprs
5.7 %  quant_all_bands                   4.9 %  celt::vq::exp_rotation      <-- oprs' single hottest
2.1 %  op_pvq_search_sse2                2.7 %  celt::cwrs::decode_pulses
3.3 %  silk_decode_core                  2.6 %  celt::celt_decoder::deemphasis
0.9 %  compute_theta                     1.8 %  <opus::decoder::OpusDecoder>::decode
—      exp_rotation (outside C top-10)   0.8 %  compiler_builtins::…::load_x86_features
```

**CORRECTED**: the first version of this comparison used an under-sampled C run (2,978 samples, 1 s) and wrongly concluded that `exp_rotation` was an asymmetry.  With the larger set (`rips_cstatic_48s.txt`, **3,970 samples**) the C ranking is:

```
C (libopus), 3970 samples                 oprs, 2985 samples
5.4 %  exp_rotation1   (6 addresses)      4.9 %  celt::vq::exp_rotation      -> NO asymmetry (same work, ~5 % both)
3.0 %  deemphasis                         2.6 %  celt::celt_decoder::deemphasis
1.3 %  decode_pulses                      2.7 %  celt::cwrs::decode_pulses  -> oprs ~2x, the clearest difference
1.2 %  quant_partition                    (not in oprs top)
~2.0 % ec_dec_uint + ec_decode            (not in oprs top)
—      op_pvq_search_sse2 (absent in this run; 2.1 % in the 1 s run -> unstable between runs)
```

So the remaining candidates are `src/celt/cwrs.rs` `decode_pulses` (2x the C share; check for a scalar inner loop where C is packed) and confirming whether `ec_dec_uint`/`ec_decode`/`quant_partition` are genuinely cheaper in oprs or just inlined/renamed in the profile.  `op_pvq_search` should only be prioritised on evidence from a **decode-only** oprs profile (the agent built `packets_48s.bin` for that).  Lesson: never draw a C-side ranking from the smallest sample set.

Also noted: `load_x86_features` (0.8 % on the oprs side) is **not** from our dispatch (only two `is_x86_feature_detected` sites exist crate-wide, both cached) and **not** from a decode-path `mul_add` (the only `mul_add` uses are `src/celt/kernels.rs`'s encoder `xcorr_kernel_avx` and `src/silk/float/inner_product_flp.rs`'s encoder analysis) — so that sample most likely comes from encoder work inside the sampler harness or from compiler-builtins machinery reached another way.  It must be re-measured with decode-only sampling before any fix is attempted.

#### CORRECTION #2 — the C-side symbolisation was off by 0x1000 (all earlier C rankings are INVALID)

The decode agent found that `prof-scratch/symbolize.py` computed `vaddr = rip - seg_start + file_offset`, but the C sampler's text segment has `p_offset=0x444c40` while `p_vaddr=0x445c40`, so the correct base is the object's file-offset-0 mapping start; the script's base was **0x1000 too high** and **every C sample was attributed to the preceding function** (that is why "encoder" symbols such as `alg_quant`, `silk_decode_core`, `opus_decoder_ctl` appeared in a CELT-only 48 kHz decode).  The oprs side was accidentally correct (its exe text mapping has `file_offset == p_vaddr`).  **Everything derived from the C ranking in the section above — including the "asymmetries" I raised and the "`exp_rotation`/`decode_pulses` are at parity" retractions, and the "no 8 kHz pathology / corpus has silk_decode_core" observations — was read off shifted symbols and must not be trusted.**

Corrected C profiles (fresh decode-only runs, ~9,930 samples each, 10 s):

```
C 48k/2ch/20ms/audio (74.13 us/frame)          oprs 48k (92.03 us/frame = 1.24x)
15.62 % opus_fft_impl                           11.93 % decode_pulses
13.61 % decode_pulses                            9.88 % opus_fft_impl
10.66 % exp_rotation1                            9.61 % exp_rotation1
 7.96 % quant_partition                          8.19 % compute_theta      <-- oprs 1.8x
 6.16 % clt_mdct_backward_c                      6.96 % OpusDecoder::decode
 4.99 % deemphasis                               6.60 % clt_mdct_backward_c
 4.55 % compute_theta                            6.28 % quant_partition
 3.34 % opus_limit2_checkwithin1_c               5.04 % rintf              <-- libm float->int16, oprs 1.8x vs celt_float2int16_c 2.78 %
 3.06 % ec_dec_uint                              4.30 % deemphasis
 2.78 % celt_float2int16_c                       3.58 % ec_dec_uint
 2.58 % alg_unquant                              2.81 % alg_unquant
 0.69 % celt_inner_prod_sse                      1.79 % celt_inner_prod     <-- oprs 2.6x
C 8k/mono/20ms/voip (7.13 us/frame)              oprs 8k (9.02 us/frame = 1.27x)
29.03 % silk_decode_core                        30.05 % silk_decode_core
23.27 % ec_dec_icdf                             19.28 % ec_dec_icdf
11.07 % ec_dec_normalize                        12.11 % silk_decode_signs   <-- oprs 1.4x (C 8.56 %)
 8.56 % silk_decode_signs                        5.06 % rintf
 5.28 % silk_LPC_inverse_pred_gain_c            4.91 % silk_shell_decoder   <-- oprs 1.3x (C 3.79 %)
 3.79 % silk_shell_decoder                      4.70 % silk_LPC_inverse_pred_gain_c
 1.11 % silk_decode_pulses
```

Four **concrete single-function deficits** (not SIMD-vs-scalar, but codegen/implementation) — the current work list:
1. `compute_theta` **8.19 % vs 4.55 % (1.8x)** — biggest 48 kHz deficit;
2. float→int16 conversion: **`rintf` 5.04 %** (and 5.06 % at 8 kHz) vs `celt_float2int16_c` 2.78 % — a **libm call per sample** where the C uses a rounding instruction (same family as the earlier `fmaf` bug);
3. `celt_inner_prod` **1.79 % vs `celt_inner_prod_sse` 0.69 % (2.6x)**;
4. 8 kHz: `silk_decode_signs` 12.11 % vs 8.56 % (1.4x) and `silk_shell_decoder` 4.91 % vs 3.79 % (1.3x).

The agent's decode-only oprs sampler (`.scratch/decprof`, decoding the same C-encoded `packets_*.bin` with all init before the timer) reproduces the harness gaps (92.03 vs 74.13 µs and 9.02 vs 7.13 µs), so these profiles are trustworthy.  It is taking items 1–3 first and will cross-check with `BENCH_FILTER=dec_i16` on `compat/bench` (self-normalising, far less noisy than `bench_min`, which produced a 325x outlier on a 491x row).

#### WIN: the `rintf` libm-call fix — decode +6…+20 %, bit-exact

`src/celt/arch.rs:448` had `x.round_ties_even() as i32` for the float→int16 conversion.  On a baseline x86-64 target that does not inline: `nm -u target/release/liboprs.rlib | grep -c rintf` was **6** (libm calls, same family as the earlier `fmaf` bug).  It was replaced with an SSE4.1 `#[target_feature]` path performing the same ties-to-even rounding, keeping the old code as the fallback.  After the change:

```
nm -u target/release/liboprs.rlib | grep -c rintf : 6 -> 0
gates: all 25 probes bit-exact | ACCEPTANCE: BIT-EXACT (decoded PCM compared) | 315 passed
bench_min decode:  48k/stereo/audio 236.6 -> 251.2x   (+6.2 %)
                   48k/mono/voip    487.9 -> 544.3x   (+11.6 %)
                   16k/mono/voip   1996.9 -> 2391.3x  (+19.7 %)
                   8k/mono/voip    3812.0 -> 4507.6x  (+18.2 %)
```

The C side was disassembled to confirm its `celt_float2int16_c` really is `cvtss2si`, i.e. the change is bit-exact by construction.  (Caveat: `bench_min` noise is ±5–10 %; `BENCH_FILTER=dec_i16` on `compat/bench` is the low-noise arbiter and the other agent is re-measuring with it.)

#### Symbol-level *differential* profile (reference static libopus vs oprs, same packets)

This is the trustworthy form of the comparison: both sides SIGPROF-sampled on the same C-encoded corpus, reported as **absolute ns/frame deltas** (not cross-build symbol shares), with each object's base derived from its offset-0 mapping and cross-checked with `gdb info symbol`.  48k/2ch/20ms/audio: oprs 88.0 µs vs C 73.6 µs (gap 14.4 µs).

```
symbol                     oprs    C     delta
<OpusDecoder>::decode      5512     0    +5497   (i16 output loop + softclip glue)
compute_theta              7568  3006    +4562   <-- largest single kernel delta (2.5x)
rintf                      4041     0    +4041   <-- REMOVED by the arch.rs fix above
pcm_soft_clip              2109  4132    -2023   (oprs faster)
clt_mdct_backward_c        6203  4739    +1465
celt_inner_prod            1489   563     +926   (*C celt_inner_prod_sse)
exp_rotation1              9376  8086    +1290
decode_pulses             10883  9863    +1020
ec_dec_uint                2907  2384     +523
unquant_coarse_energy      1081   548     +533
deemphasis                 4041  3732     +309
alg_unquant                2091  2029      +63   (equal -> NOT the gap)
opus_fft_impl              8437 11077    -2640   (oprs faster)
```

Remaining decode work list, in priority order: (1) `compute_theta` (2.5x, the biggest kernel delta); (2) `clt_mdct_backward_c`; (3) `celt_inner_prod` vs the C's SSE body; (4) `exp_rotation1` / `decode_pulses`; then the `<OpusDecoder>::decode` i16-output-loop + softclip glue, which is per-sample opus-layer work (same family as the encode residual, where the `rintf` fix should also help since `opus_encode_float` converts float input to int16).  Ownership: the decode/`arch.rs` work is one agent's; the C-side sampler's owner is now verification-only, to avoid collisions.

#### FINAL STATE of the residual-performance goal (goal-505bb50a)

**Two wins landed and verified; the tree's only source deltas are those two.**

1. **`Repacketizer` container redesign** — `frames`/`paddings` became lazily-grown `Vec<Option<Box<[u8]>>>`; struct 2656 → 1888 → **400 B**.  Micro: `new()`+drop 241 → 151.6 → **42.7 ns** (orchestrator's run of the same loop: 25.8 ns; both agree construction is now *faster than libopus' 47 ns*).  Harness: `repkt_new` **0.726** (oprs 41 ns vs audiopus 56 ns), `repacketizer` rows **2.997 → 2.399** and **2.887 → 2.421**.  Remaining 2.4–2.5x on that row is `cat`'s per-frame `to_vec().into_boxed_slice()`; going lower needs the C's borrowed-pointer contract (not attempted).

2. **`FLOAT2INT16` libm call** (`src/celt/arch.rs`) — `x.round_ties_even() as i32` did **not** fold into `cvtss2si` on the x86-64 baseline; it emitted an out-of-line libm `rintf` (6 refs in `liboprs.rlib`).  **Proven with gdb**, not inference: `break rintf; run` → `#0 rintf ()`, `#1 <oprs::opus::decoder::OpusDecoder>::decode ()`.  The C (`celt/float_cast.h:68`) does `_mm_cvt_ss2si(_mm_set_ss(x))` under `__GNUC__ && __SSE__`.  Replaced with `_mm_cvtss_si32(_mm_set_ss(x))` under `#[cfg(target_arch = "x86_64")]`, keeping `round_ties_even()` as the non-x86_64 fallback.  Bit-exact by construction (same MXCSR ties-to-even; the intrinsic's integer-indefinite result equals the old NaN/range guards — checked 2^31, −2^31, ±1e30, NaN).  `rintf` refs **6 → 0**.
   Measurements: low-noise steady-state SIGPROF, identical method both sides — **48k/2ch/20ms/audio 92.03 → 83.75 µs/frame (−9.0 %), gap 1.241x → 1.130x**; **8k/mono/20ms/voip 9.02 → 8.35 µs (−7.4 %), gap 1.265x → 1.171x**.  `bench_min` decode medians (5 runs): 48k/st +8.0 %, 48k/mono +1.6 %, 48k/ld +6.5 %, 16k/mono **+13.4 %**, 8k/mono **+18.8 %**.  Independent `BENCH_FILTER=dec_i16` A/B: geomean **1.303 → 1.252**, worst row `8k/mono/20ms/voip` **1.303 → 1.155 (−13.7 %)**.

**Measured negatives — do not re-investigate without new evidence:**
* `vq::exp_rotation`: the C's `celt_cos_norm(x)` is `((float)cos((.5f*PI)*(x)))` with `PI` an **untyped double literal**, so the C pays the *same two libm `cos()` calls*; corrected profiles are `exp_rotation1` 10.66 % (C) vs 10.97 % (oprs) — parity.
* `clt_mdct_backward_c`: already packed in our build (28 `mulps`/27 `addps`/67 `shufps`); C 4.57 µs vs ours 5.89 (1.29x codegen) — and we are **faster** in the FFT (`opus_fft_impl` 9.15 vs 11.58 µs).
* **8 kHz SILK has no C SIMD to transcribe**: all seven `silk/x86/x86_silk_map.c` dispatch tables (VAD_GetSA_Q8, NSQ, NSQ_del_dec, VQ_WMat_EC, burg_modified, inner_prod16, inner_product_FLP) are **encoder-only**; there is no target for `silk_decode_pulses`/`decode_core`/`NLSF_decode`/`decode_indices`.
* CELT decoder RTCD helpers are only `celt_inner_prod`, `dual_inner_prod`, `comb_filter_const`; the last is unreachable on real packets (zero postfilter gain → `comb_filter` returns after `OPUS_MOVE`).
* `compute_theta`: both symbols contain **exactly 12 `div` instructions**, so the comparison is like-for-like; our body is 5727 B vs the C's 3606 B and carries Rust-only cold-path bloat (4× `_Unwind_Resume@plt`, inlined `get_or_init_slow`, bounds/`RefCell` panic paths).  A genuine per-call codegen delta, but rewriting a bit-exactness-critical entropy function is out of proportion.
* `load_x86_features` (my earlier lead) was a red herring: `fma`/`fmaf` have **zero** references in the decode binary (dead code), `opus_select_arch()` is the constant 4, and there is no decode-path `mul_add`.  Its *class* was right — the real one was the libm `rintf`.
* Rejected by measurement: `lto="fat"` + `codegen-units=1` (48k/st 223.5 vs 235.8, 8k 3302 vs 3692 — worse); `panic="abort"` (worse).

**Remaining gap and the only plausible levers:** 48k stereo 1.130x, 8k mono 1.171x.  The last kernel with a *possible* SIMD gap is **`celt_inner_prod`** (+1.24 µs vs the C's `celt_inner_prod_sse`; our body is partly packed — mulps/addps plus a scalar tail — and it is shared with the encoder, so a fix would help both directions).  Everything else is at or near parity, partly offset by our being faster in `opus_fft_impl` (−2.43 µs), the `OpusDecoder::decode` output region (−2.54), `quant_partition` (−0.37) and `clt_compute_allocation` (−0.41).  The **encode residuals** (`8k/mono/*/lowdelay`, `12k/mono/*/audio`, 2.4–2.7x) are **unresolved**: the `rintf` fix did not move them (2.650 → 2.691, 2.490 → 2.510, 2.387 → 2.376, all inside noise), consistent with the earlier attribution to other per-sample opus-layer work (`upsample=6` pre-emphasis branch + 960-sample zero-fill, analysis FFT, `pcm_buf` copies); `celt_inner_prod` is a shared candidate.

**Context you need to reproduce any of this** (system `libopus.so.0` is stripped to 105 dynamic symbols, so the reference archive is required for C symbols): throwaway crate linking `oprs` (path) plus a C-side crate statically linking `reference/lib-ref/libopus.a` via build.rs (`cargo:rustc-link-lib=static=opus`); `SIGPROF` handler with `SA_SIGINFO|SA_RESTART` on `setitimer(ITIMER_PROF, 1 ms)`; RIP = `*(u64*)((u8*)ucontext + 168)` (glibc x86_64: `uc_flags`8 + `uc_link`8 + `stack_t`24 = 40, then `gregs[REG_RIP=16]`); decode the `compat/bench` corpus (`packets_*.bin`, same LCG PCM + `bitrate_for`) in a loop; dump RIPs plus `/proc/self/maps`; symbolize with `nm --defined-only --numeric-sort`.  **The base of each object MUST come from its offset-0 mapping, not `start - offset`** — `/proc/self/maps` reports a page-aligned *file offset* that can differ from the page-aligned segment vaddr by one page (this cost a whole C-side profile and is CORRECTION #2 above).  Cross-check sampled RIPs with `gdb info symbol`.

**Gate lines on the final tree:** `all 25 probes are bit-exact`; `ACCEPTANCE: BIT-EXACT` (decoded PCM compared); `315 passed`; `cargo build --release` clean but for 59 pre-existing warnings; `ACCEPTANCE SUITE: all gates passed` (run by the orchestrator on this revision).

#### Two process lessons worth keeping (from the same goal)

1. **Kernel-level wins are not row-level wins.** The `celt_inner_prod`/`dual_inner_prod` intrinsics transcription (and an independent `chunks_exact(4)` variant) made that kernel up to **~2x faster in isolation** (25.1–28.0 → 11.8–16.1 ns/call, in-process A/B against `celt_inner_prod_c`), yet the frame-level `BENCH_FILTER=dec_i16` geomean did not move (1.252 → 1.253) because the kernel is only ~1.24 µs of an ~84 µs frame. The ≥10 %-on-the-dominated-row rule is therefore the right acceptance bar; kernel micro-benchmarks should be used only to explain *why* a row did or did not move. (Root cause of the originally-slow body, worth remembering: `debug_assert!(x.len() >= n)` is compiled out in release, so LLVM had no bound and emitted ~8 bounds compares + 3 scalar loads + `unpcklps`/`movsd` per 4-lane block instead of one `movups` per operand — safe slice loads can silently defeat vectorisation.)
2. **Announce file ownership, and scope it to files, not directories.** Two agents were given overlapping work and both edited `src/celt/kernels.rs` within minutes; the second agent's `_mm_loadu_ps` version superseded the first agent's `chunks_exact(4)` edit before it could be measured, wasting that work. The orchestrator's instruction ("you are verification-only, so you cannot collide") was true for `src/` in general but said nothing about that specific file. Rule: before editing, an agent must name the file it is taking and treat every un-named file as off-limits; the orchestrator must list the *files* it is handing over, not just the task.

#### THIRD WIN and the final tree of goal-505bb50a: the CELT pitch-xcorr was running as scalar FMA

Stage profiling (`cargo run --release --features stage-timers --example stage_prof`) for `8k/mono/20ms/lowdelay` showed the whole CELT-only anomaly concentrated in one stage:
```
celt: pitch search   85.8 us/call   (67 % of the 127 us CELT encode)
  pf: downsample      7.2
  pf: pitch_search   59.3
    ps: xcorr coarse 50.0   <-- the anomaly
    ps: best coarse   2.1
    ps: fine loop     4.7
  pf: remove_doubling 11.1
  celt: frequency->residual 17.5 / residual quant 12.5
```
`objdump` of `celt_pitch_xcorr_hw` explained it exactly: **66 x `vfmadd231ss` (scalar!) + 486 `vmovss` + only 8 `ymm` references** — the "AVX2+FMA" wrapper was being *reached*, but its body was the `[f32; 8]` lane emulation, which LLVM never vectorised, so 58,560 MACs ran at ~1 MAC/instruction (~50 us, 2.5 cycles/MAC) where the C's real `__m256` sequence takes ~5 us.  That single kernel was a **constant ~90 us/frame penalty on every CELT call** (matching the band-count-independent excess measured across 8 k and 48 k rows).

Fix: `xcorr_kernel_avx_hw` transcribed 1:1 with `core::arch::x86_64` `__m256` intrinsics (`_mm256_loadu_ps`/`_mm256_fmadd_ps`/maskload tail/`_mm256_permute2f128_ps`/`_mm256_hadd_ps`), same FMA order and lane layout, emulation kept as the non-AVX2 fallback.  After it: `vfmadd231ss 66 -> 0`, `vfmadd231ps 16`, `probe_kernels` IDENTICAL (2816), all 25 probes bit-exact.

Row-level effect: **this is a PASSING ≥10 % change, not an exception.** The orchestrator's first measurement compared the fixed tree against an *older* results file from a faster machine state and concluded only 3–7 % (and the agent initially reverted the edit on that basis). A controlled back-to-back A/B in one session (`BENCH_FILTER=enc_i16`, fix stashed and unstashed between the two runs; `prof-scratch/bench_before_fix_enc_i16.txt` vs `prof-scratch/bench_after_fix_enc_i16.txt`) gives:
```
row                      r before -> after    oprs ns before -> after
8k/mono/20ms/lowdelay      2.665  -> 1.709     135484 -> 100392   (-25.9 %)
8k/mono/10ms/lowdelay      2.371  -> 1.772      79598 ->  58408   (-26.6 %)
12k/mono/20ms/audio        2.443  -> 1.669     152949 -> 101875   (-33.4 %)
48k/mono/20ms/lowdelay     1.784  -> 1.507     212228 -> 186364   (-12.2 %)
48k/2ch/20ms/audio         1.655  -> 1.476     459660 -> 397748   (-13.5 %)
48k/mono/20ms/audio        1.849  -> 1.405     217778 -> 190291   (-12.6 %)
```
The audiopus control column itself moved ±11 % between those two runs, so the **ratio** is the arbiter and any comparison across files/sessions (as the orchestrator did) can under- or overstate a change by a factor of several. Methodology lesson: for a candidate fix on this host, always do the stash/unstash A/B in a single session; never judge from two files produced at different times.

After the fix, the stage profile shows the pathology is gone: `ps: xcorr coarse` fell from 50.0 µs to 3.9–5.4 µs, and the remaining `pitch search` (27.6 µs at 10 ms, 41.6 µs at 20 ms) is spread across `pitch_downsample`/`_celt_autocorr` and `remove_doubling` with no single dominant sub-stage left. The encode residual on the dominated rows is therefore now roughly 1.7x (was 2.4–2.7x); further gains would come from those spread stages.   The encode residual is therefore *reduced but not closed* (~2.3–2.6x remains on the 8 k/12 k rows), with the next candidates being `remove_doubling` (11.1 us), `downsample` (7.2 us), `frequency->residual` (17.5 us) and `residual quant` (12.5 us).

**Lesson (same family as the `debug_assert` one):** counting instructions is not evidence of vectorisation.  My earlier check counted 82 `vfmadd`/`vfmsub` in the binary and concluded "FMA is in use where it matters" — but 66 of those were the **scalar `ss`** form.  Always look at the operand form (`ss` vs `ps`, `xmm` vs `ymm`) and at whether the compiler actually emitted the packed instruction for the intrinsic you wrote.

**Final tree for this goal** (`git diff --numstat src`): `src/celt/arch.rs` 28/11 (float2int), `src/celt/kernels.rs` 103/9 (xcorr AVX2), `src/opus/repacketizer.rs` 46/15 (container redesign), plus `docs/progress.md` and `.gitignore`.  All temporary instrumentation was removed (`examples/stage_prof.rs`, the `stage-timers` feature in `Cargo.toml`, and the timer code in `celt_encoder.rs`/`opus/encoder.rs`/`pitch.rs`), and the final revision re-gated clean: **all 25 probes bit-exact, ACCEPTANCE: BIT-EXACT (decoded PCM compared), 315 tests passed**.

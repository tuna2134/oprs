# Does the SILK NSQ AVX2 kernel belong to `fast`?

**Item ② — reorganise the SILK NSQ SIMD so `fast` owns it consistently, and measure
what that costs the default build.**

Scope: `src/silk/nsq_del_dec.rs`, new `examples/nsq_ab.rs`, this report. Nothing
else was touched by this work. (`git status` also lists `src/celt/kernels.rs` and
`examples/pvq_quality.rs`; those are the parallel CELT agent's, and they were
present identically in both sides of every A/B below, so they cannot bias the
comparisons.)

---

## 0. Verdict (short)

**No — the arch-4 AVX2 body is the reference, so it stays in the default build.
`fast` gains nothing here, and the attempt to give it the kernel is measurably
harmful.**

| default body | SILK rows (end to end) | 16-config oracle | 25 probes | per NSQ call |
| --- | --- | --- | --- | --- |
| **AVX2 (kept)** | baseline | `BIT-EXACT`, `TOTAL enc=0xdbce15d74e829a43` | `all 25 probes are bit-exact` | 1.26–1.69x faster than the C body (geomean **1.48x**) |
| C body ("provable exactness") | **+21.5 % … +35.5 %** on the five requested rows | **13 of 16 configurations change** (`enc_bytes` identical, packet bytes differ) | **24/25** (`probe_silkenc` fails on the C's own AVX2-vs-C divergence) | baseline |

The C body is *not* an exactness gain: it is the fallback libopus itself uses only
on a host without AVX2 (or for `nStatesDelayedDecision < 3`). The reference we
have always matched is libopus 1.6.1 as built and running here, where
`opus_select_arch() == 4` dispatches `silk_NSQ_del_dec_avx2`.
`docs/rtcd-kernels.md` §6's "NO, not provably bit-identical" is a statement about
the **fallback**, not about the reference. §6 should say so explicitly.

`fast`-side work was attempted and is a measured negative (see §5): the kernel is
integer Q-domain arithmetic, so there is no FMA to add, and the one bit-exact
re-association candidate (a 4-accumulator LPC tap chain) measured **+0.26 %**
(slower) and was reverted.

---

## 1. The restructure

`src/silk/nsq_del_dec.rs` keeps all three implementations:

| implementation | what it is | compiled | selected by |
| --- | --- | --- | --- |
| `silk_NSQ_del_dec_c` | the bit-exact C body, 1:1 transcription | always | every non-arch-4 case, and the internal `nStatesDelayedDecision < 3` fallback |
| `silk_NSQ_del_dec_avx2` | the arch-4 body; **intrinsics** on x86-64, lane-vector **emulation** elsewhere | always | `use_avx2_nsq(arch)` |
| `mod avx2_hw` | the intrinsic-for-intrinsic transcription of `silk/x86/NSQ_del_dec_avx2.c` | `cfg(target_arch = "x86_64")` | only through `silk_noise_shape_quantizer_del_dec_avx2`'s run-time `have_avx2()` branch |

The policy is one function, so the decision is one line:

```rust
fn use_avx2_nsq(arch: opus_int) -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        (arch & 7) == 4 && have_avx2()          // <- the C's RTCD arch-4 slot
    }
    ...
}
```

* It is **the same in both builds** (this is the decision taken after the
  measurements below): the run-time `have_avx2()` guard is kept, so a CPU without
  AVX2 still executes the C body and never SIGILLs, and `silk_NSQ_del_dec_avx2`
  still falls back to the C body for `nStatesDelayedDecision` ∈ {1, 2}.
* `mod avx2_hw` is **not** `cfg(feature = "fast")`: the default build needs it.
  During the experiment it *was* gated on `fast`; that only made sense while the
  default was not allowed to dispatch there.
* The lane-vector emulation is compiled in both builds on purpose: it is the
  executable specification of the intrinsics, it is what five unit tests and
  `examples/nsq_ab.rs` exercise, and keeping it compiled is why the test count
  stays **315 in both builds** with no test edits.
* `--features fast` is left with an explicitly *empty* NSQ scope. That is the
  finding, not an omission: see §5.

Documentation of the policy and of the measurement that chose it is in the module
header and on `use_avx2_nsq` / `silk_NSQ_del_dec`, so the next person does not
redo this experiment.

---

## 2. Method

### 2.1 Per call — `examples/nsq_ab.rs` (new)

Times `silk_NSQ_del_dec_c` against `silk_NSQ_del_dec_avx2` **on identical input**
inside one process:

* 8 cases: `nStatesDelayedDecision` 3 and 4, voiced/unvoiced, warping on/off,
  8/12/16 kHz internal rate, 20 ms frames, deterministic inputs from the same LCG
  as `probe_silkenc` (`x16` ±8000, LPC ±1500, gains 65536..400000, λ 1024/3000);
* every timed call starts from a byte-identical `silk_nsq_state` /
  `SideInfoIndices` snapshot, so the measured region is exactly one NSQ call;
* C and AVX2 bodies are alternated **call by call** so host drift cancels;
* the reported number is the **min of 300 repetitions** (plus the mean). On this
  shared host the mean moves by ±10–20 % between runs while the min repeats to
  ~0.1–0.3 %; the min is what makes the small fast-side deltas readable.

It also prints whether the two bodies agree on the observable output (`pulses`,
`xq`, the LPC/shaping states, the seed) and what the **dispatcher** costs.

### 2.2 End to end — `compat/bench`

`BENCH_FILTER=enc_i16 BENCH_TARGET_MS=60`, 160 encode rows, **three alternating
pairs** of full runs in one session (`A B A B A B`), where the tree differs only in
`use_avx2_nsq`: `A` = AVX2 default, `B` = C-body default. Each side is built from
source by `compat/bench/run.sh`, which also stamps and double-checks the `src/`
digest (no run spanned a tree change: 0 digest warnings).

The arbiter is the **in-process ratio** `r_med = oprs/audiopus` ns per frame, so
the host's own drift cancels row by row. The five requested rows are reported both
as that ratio and as absolute `oprs` ns/frame.

**Noise floor**, measured first on the identical tree: the spread (max/min) of the
three same-state `r_med` values is a median **6.6 %** across the 160 rows. The
effects below are 3–5x that.

Provenance: the six runs were back to back in one session on the working tree,
which included the parallel CELT agent's in-flight `src/celt/kernels.rs` on both
sides (0 digest warnings in every run, i.e. no run spanned a source change). The
A/B therefore isolates `use_avx2_nsq` and nothing else.

---

## 3. What "default = C body" costs — the measurement

### 3.1 It is not an exactness gain (the important part)

Oracle (`bash compat/scripts/run-oracle.sh`, full 16 configurations / 50–200 frames
each):

| tree | result |
| --- | --- |
| HEAD / default = AVX2 | `ACCEPTANCE: BIT-EXACT`, `TOTAL enc=0xdbce15d74e829a43 dec=0xf73254ad00d572f1` |
| default = C body | `ACCEPTANCE: DIFFERENT`, **13 of 16** configurations changed, `TOTAL enc=0x607257776c94b0cf` |

Same `enc_bytes` on every row (10982, 3782, … identical) but different packet
bytes: a real bitstream change, not a size change. The three unchanged
configurations are exactly the low-complexity ones (`16k/mono/voip/c0/40ms`,
`8k/mono/voip/c0/20ms`, `12k/mono/voip/c5/20ms`) where the encoder already picked
`nStatesDelayedDecision < 3` and the AVX2 entry had been falling back to the C body
anyway.

Probes (`bash compat/scripts/run-all-probes.sh`): **24 of 25**. `probe_silkenc`
differs, and classifying all 130 differing lines is what proves the point:

* **0** differing lines carry a `_c` suffix. `deldec_cmp.pulses_c`,
  `deldec_cmp.nsq_c.*`, `deldec_cmp.seed_c` — i.e. the probe's explicit
  `silk_NSQ_del_dec_c` run — are **byte-identical between the C library and this
  port**. Our C body is exactly the C body.
* All differences are `*_dispatched`, `nsq.*` and `nsq.ec.*` on **3 of 14** NSQ
  cases (section 18 cases 7, 9, 10 and 2 of 4 in section 19): the places where the
  C's own `silk_NSQ_del_dec_avx2` and `silk_NSQ_del_dec_c` disagree.

So the divergence §6 calls "NO, not provably bit-identical … rare cases" is real
and reachable: with those inputs it changes 3 of 14 synthetic NSQ cases and, on
real encoder content, 13 of 16 oracle configurations. Our AVX2 transcription
reproduces the C's AVX2 output on all of it (HEAD is bit-exact), so the
transcription is right; it is libopus that is not bit-identical between its own
two kernels.

### 3.2 Per NSQ call

`examples/nsq_ab.rs`, fast build, min of 300, C body vs AVX2 intrinsics:

| case | C body ns | AVX2 ns | C/AVX2 | outputs |
| --- | ---: | ---: | ---: | --- |
| 16k / 4 states / voiced / warp | 170 279 | 103 724 | **1.64** | pulses/state differ |
| 16k / 4 states / unvoiced | 169 880 | 103 368 | **1.64** | pulses/state differ |
| 16k / 3 states / voiced | 122 385 | 91 574 | **1.34** | pulses/state differ |
| 16k / 3 states / unvoiced / warp | 122 459 | 91 624 | **1.34** | bit-identical |
| 12k / 4 states / voiced / warp | 122 599 | 77 555 | **1.58** | bit-identical |
| 12k / 3 states / unvoiced | 87 658 | 68 303 | **1.28** | bit-identical |
| 8k / 4 states / voiced | 82 094 | 52 476 | **1.56** | pulses/state differ |
| 8k / 3 states / unvoiced / warp | 57 896 | 45 911 | **1.26** | bit-identical |
| **geomean** | | | **1.48x** | |

(The last column is a *finding*, not a bug: on half of the probe-like cases the two
references — AVX2 and plain C — genuinely produce different pulses. The geomean
repeats to **1.45–1.48x** across three full runs of the example.)

The dispatcher adds **1.00x** on top of the body it selects (all 8 cases), i.e. the
arch-4 branch and the cached `have_avx2()` load are free.

For reference, the lane-vector emulation (what a non-AVX2 host would otherwise
run) is **3.6–4.3x slower than the C body** (C/B = 0.23–0.30 on the same eight
cases). This is why the run-time AVX2 check must fall back to the *C body*, not to
the emulation.

### 3.3 End to end — the five requested rows

`enc_i16`, `c10`, VBR, mono, 20 ms; `A` = AVX2 default (kept), `B` = C-body default
(the move). `r_med` is the in-process ratio oprs/libopus (lower is better), median
of three runs each; "cost" is `B` vs `A`.

| row | A `r_med` | B `r_med` | cost | A ns/frame | B ns/frame | cost (abs) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `8k/mono/20ms/voip/16000bps` | 1.204 | 1.516 | **+25.9 %** | 155 570 | 193 899 | **+24.6 %** |
| `12k/mono/20ms/voip/24000bps` | 1.170 | 1.585 | **+35.5 %** | 221 735 | 282 124 | **+27.2 %** |
| `16k/mono/20ms/voip/24000bps` | 1.126 | 1.511 | **+34.2 %** | 327 524 | 423 900 | **+29.4 %** |
| `48k/mono/20ms/voip/24000bps` | 1.188 | 1.443 | **+21.5 %** | 398 613 | 471 972 | **+18.4 %** |
| `8k/mono/20ms/audio/16000bps` | 1.140 | 1.503 | **+31.8 %** | 142 096 | 197 833 | **+39.2 %** |

That is the price of the move on exactly the rows the AVX2 kernel was introduced
for: **~20–35 % slower**, i.e. the AVX2 body is worth **1.2–1.4x end to end** on
these rows. (In `r_min` terms the same rows read +28.1 %, +33.9 %, +39.1 %,
+34.4 %, +31.2 %.)

Wider views from the same runs:

| set | n | cost of the C body |
| --- | ---: | --- |
| the five rows above | 5 | +21.5 % … +35.5 % |
| SILK-only rows (`8k/`+`12k/`, ≥ 10 ms) | 40 | median **+13.3 %**, geomean +14.5 % |
| whole `enc_i16` set | 160 | median +2.1 %, geomean **+7.9 %** |

The `enc_i16` distribution has two clear clusters, which is the useful
interpretation:

* **Rows that really run the delayed-decision NSQ** (any 20 ms VOIP row at every
  rate, plus `8k/mono/20ms/audio`): **+20 … +40 %**.
* **Rows where the encoder picks `nStatesDelayedDecision < 3`, a CELT-only
  configuration, or a short frame** (all `lowdelay` rows, most 48 kHz
  audio/hybrid rows, 2.5/5 ms frames): **−4.0 … +3.7 %**, i.e. inside the ±6.6 %
  noise floor — both sides run the C body there, so there is nothing to change.

Cross-check with `examples/bench_min.rs` (default build, 3 runs, nsq_ab in
parentheses): `8k/mono/voip` 0.12–0.14 ms/frame where one NSQ call is 52 µs
(AVX2) / 82 µs (C) → +30 µs on a ~130 µs frame = **+23 %**, matching the table;
`16k/mono/voip` 0.24–0.26 ms/frame where NSQ is 104 / 173 µs → **+28 %**,
matching too. The bench rows and the kernel-level numbers agree, which is the
sanity check that the row deltas are NSQ and not noise.

`bench_min`, default vs `fast`: all SILK rows are within its own ±10–15 % noise
(the NSQ code is identical in the two builds, so this is expected).

---

## 4. Why the "default = C body" idea is wrong (recorded so it is not retried)

1. The C body and the AVX2 body are **different codecs** by construction: the AVX2
   file is a Q14 rewrite whose `silk_sar_round_smulww` omits the intermediate int32
   truncation. Whatever the exact trigger, it is not the rare-corpus curiosity §6
   implies: on ordinary encoder audio it fires often enough to change **13 of the
   16** oracle configurations, and on the probe's synthetic gains it changes 3 of
   14 NSQ cases.
2. libopus dispatches arch 4 on any AVX2 host, so **the AVX2 semantics are the
   observable libopus 1.6.1 semantics** on the reference machine. Matching the C
   body means matching libopus-on-a-machine-without-AVX2.
3. Therefore the C body buys no exactness: it trades "matches libopus's observable
   output" for "matches libopus's fallback", reddens both acceptance gates, and
   costs 20–35 % on exactly the rows the kernel exists for.

The only honest way to describe the default build's contract is the one now in the
module docs: *bit-exact with libopus 1.6.1 as dispatched on this host* (arch 4);
the C body is the exact fallback for `nStatesDelayedDecision < 3` and for hosts
without AVX2. It is not "exactness by construction" and cannot be made so without
changing which library we match.

---

## 5. Can `fast` gain anything here? (attempted, measured, rejected)

### 5.1 AVX2+FMA variants — not applicable

The whole NSQ body is **integer** Q-domain arithmetic: `_mm256_mullo_epi32`,
`_mm256_mul_epi32`, `_mm256_mulhrs_epi16`, `_mm256_madd_epi16`, `_mm256_slli/
srai_epi32`, saturating add/sub, plus the scalar `SMULWB`/`SMULWW` taps. There is
**no `_mm*_ps`/`_mm*_pd` operation anywhere in the kernel** (checked over the 83
`_mm*` call sites), so there is nothing for an FMA contraction to fuse:
`target_feature(enable = "fma")` cannot change integer codegen. Adding
`avx2,fma` to the hot functions is a no-op by construction, and a variant that
*wanted* an FMA would need floating-point arithmetic the Q14 body does not have.

### 5.2 The integer analogue: re-associate the LPC tap chain — **+0.26 %, reverted**

`h_short_prediction_x4` accumulates `order` (10 or 16) taps into one
`_mm256_add_epi32` chain. That chain *is* re-associable **bit-exactly**: each tap
is a wrapping 32-bit lane add, so every lane is `(bias + Σ products) mod 2^32` and
the high 32 bits `h_cvtepi64_epi32_high` selects are unchanged. So this candidate
is exact (it could have gone in the default, not just `fast`) and looked like
latency headroom.

Measured with two binaries built from the same tree with `--features fast`,
alternating, min-of-300, five runs each:

| case | single chain (C's order) | 4 chains | 4ch / single |
| --- | ---: | ---: | ---: |
| 12k / 3 states / unvoiced | 68 266 | 68 324 | 1.0008 |
| 12k / 4 states / voiced / warp | 77 350 | 77 448 | 1.0013 |
| 16k / 3 states / unvoiced / warp | 91 810 | 92 445 | 1.0069 |
| 16k / 3 states / voiced | 91 518 | 92 023 | 1.0055 |
| 16k / 4 states / unvoiced | 103 407 | 103 869 | 1.0045 |
| 16k / 4 states / voiced / warp | 103 902 | 104 128 | 1.0022 |
| 8k / 3 states / unvoiced / warp | 45 887 | 45 882 | 0.9999 |
| 8k / 4 states / voiced | 52 513 | 52 518 | 1.0001 |
| **geomean** | | | **1.0026** |

**0.3 % slower**, consistently — the tap chain is throughput-bound (independent
`vpmuldq`s), not latency-bound, and the extra accumulators cost registers. Reverted
to the C's single chain, which is also what keeps the transcription literal.

### 5.3 Dispatcher / dispatch overhead — nothing there

`silk_NSQ_del_dec_avx2` calls the intrinsic body through a run-time `have_avx2()`
check; measured cost is **1.00x** of the body itself on all 8 cases (cached
`cpuid`, a relaxed atomic load). Hoisting it, caching it in a `static`, or making
it compile-time under `fast` buys nothing measurable.

### 5.4 What is left (for the record, not attempted)

A *semantics-changing* AVX2 variant (e.g. 8 lanes across two candidate sets,
`vpmaddwd`-style 16-bit packing of the prediction, or the emulation's
fewer-serial-dependency order) would change the bitstream and would need its own
quality study on `snr_harness` — and on this corpus the encoder's bitstream is
already identical to libopus's at the current speed, so any such variant would
have to beat a 1.2–1.4x end-to-end gap while *losing* the bit-exactness contract.
That is a product decision, not a kernel tweak, and the numbers above say the
current kernel is not the thing to trade.

---

## 6. Recommendation

1. **Keep `default = AVX2`** (implemented): arch 4 + `have_avx2()` → the AVX2 body,
   C body for everything else (including `nStatesDelayedDecision` 1/2). This is
   what the gates, the oracle corpus and libopus on this host all agree with.
   Everything else costs 20–35 % on the SILK rows and reddens both acceptance
   gates.
2. **`fast` does not own this kernel.** Its NSQ scope stays empty; the feature's
   existing CELT kernels are unaffected.
3. **Fix the story in `docs/rtcd-kernels.md` §6**: the AVX2 kernel is not a
   "maybe" relative to the reference — it *is* the reference at arch 4, and the C
   body is the fallback. The measured evidence (13/16 oracle configurations, 3/14
   probe NSQ cases, `deldec_cmp.*_c` byte-identical) is in §3.1 above.
4. The flip, if anyone ever wants the C-body default for a non-AVX2-targeted build,
   is one line in `use_avx2_nsq` (`return false`), and this report + the module docs
   say what it costs.

---

## 7. Reproduce

```text
cargo run --release  --example nsq_ab                      # C body vs arch-4 body
cargo run --release --features fast --example nsq_ab       # same NSQ, per-call A/B

BENCH_FILTER=enc_i16 BENCH_TARGET_MS=60 bash compat/bench/run.sh /tmp/ab.txt
#   A = tree as committed, B = use_avx2_nsq -> false; alternate A B A B A B

cargo run --release --example bench_min
cargo run --release --features fast --example bench_min

bash compat/scripts/run-all-probes.sh
bash compat/scripts/run-oracle.sh
cargo test --lib --release
cargo test --lib --release --features fast
cargo run --release --features fast --example snr_harness
```

---

## 8. Gate results on the final tree

| gate | result |
| --- | --- |
| `cargo build --release` | clean (57 pre-existing warnings, same count as HEAD) |
| `cargo build --release --features fast` | clean (57 pre-existing warnings) |
| `bash compat/scripts/run-all-probes.sh` | `all 25 probes are bit-exact` |
| `bash compat/scripts/run-oracle.sh` | `ACCEPTANCE: BIT-EXACT — every configuration produced identical packets and identical decoded PCM.` (`TOTAL enc=0xdbce15d74e829a43 dec=0xf73254ad00d572f1`) |
| `cargo test --lib --release` | `315 passed; 0 failed` |
| `cargo test --lib --release --features fast` | `315 passed; 0 failed` |
| `cargo run --release --features fast --example snr_harness` | `QUALITY: PASS` — minimum decoder SNR **129.1 dB** (gate 90), packet bytes **95104 vs 95104**, **0** frames of different size, interop both directions. Every SILK-only configuration is still `inf` (bit-identical decode), exactly as before: the NSQ code is identical in the two builds. Two CELT rows show a non-`inf` *informational* `enc drift` (19.6 dB `48k/stereo/audio/128k/c10/5ms`, 14.6 dB `48k/stereo/lowdelay/64k/c10/2.5ms`); that is the `fast` CELT/FMA work owned by the parallel agent, not this kernel |
| `cargo run --release --example snr_harness` (sanity) | `QUALITY: PASS`, minimum decoder SNR `inf` on all 16 configurations |

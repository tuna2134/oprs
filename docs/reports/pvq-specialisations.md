# PVQ search specialisations under `--features fast`

Status: implemented, measured, kept (with one revert). Default build untouched
and still bit-exact. Owner: item ① (PVQ search), files
`src/celt/kernels.rs` (+ `examples/pvq_quality.rs`, this report).

## 1. Summary

| variant | dispatch condition | bit-exact vs reference? | kernel speed-up (measured) | decision |
|---|---|---|---|---|
| `K == 1` argmax | `k == 1` | no (tie-breaking only) | ≈10–20x per call (scalar argmax; small share of the work) | **kept** |
| `N == 2` closed form, exact `1.0/sum` | `n == 2` | no (different pulses, equal quality) | **2.3–3.2x** | **kept** |
| `N == 4` four-lane body | `n == 4` | yes | **2.3–3.2x** | **kept** |
| general body, scratch not zero-filled | everything else | yes | **1.02–1.39x** (1.10–1.39x for `n <= 32`) | **kept** |
| eight-lane AVX2 scan (`pvq_search_avx2` shape) | `n >= 16` | yes | **0.87–0.97x for `n <= 64`**, 1.10x @120, 1.17x @176 | **reverted** |
| batched `fast_select` (top-B per pass) | `n >= 32` | no | rejected on analysis (§4.6), not implemented | not added |

Only `K == 1` and `N == 2` change the bitstream. Both were measured end to end
(§5): minimum decoder SNR **129.1 dB**, **0.000 %** packet-size drift,
**−0.001 dB** worst signal-SNR delta against libopus's own encoding. 14 of the
16 oracle configurations still produce **byte-identical** packets.

The four kept variants make the PVQ search 1.0–3.2x faster per call at the band
sizes the 16 oracle configurations actually use; §6 gives the end-to-end effect
on the six `enc_i16` control rows.

## 2. What was added

`src/celt/kernels.rs`, new `[fast] PVQ search specialisations` section
(`#[cfg(all(feature = "fast", target_arch = "x86_64"))]`), plus one dispatch
hook at the top of `op_pvq_search_sse2`:

```rust
#[cfg(all(feature = "fast", target_arch = "x86_64"))]
if let Some(yy) = op_pvq_search_specialised(x, iy, k, nn) { return yy; }
```

`op_pvq_search_specialised` is the dispatcher (the `restsend/opus-rs`
`src/pvq.rs:100-147` shape):

```text
k == 1            -> op_pvq_search_k1        (argmax)
n == 2            -> op_pvq_search_n2        (closed form, exact reciprocal)
n == 4            -> op_pvq_search_n4_hw     (four-lane SSE body)
otherwise         -> op_pvq_search_fast_body (reference arithmetic, scratch
                                              written instead of zero-filled)
```

The dispatch is inside `op_pvq_search_sse2` rather than in `vq.rs` so that both
`alg_quant` and the existing `examples/kernel_ab.rs` (which calls the kernel
directly) exercise it. `src/celt/vq.rs` therefore needed no code change (only a
cross-reference comment in `alg_quant`).

The default path is untouched: the only non-`cfg(fast)` edit is that
`op_pvq_search_sse2_hw` now takes the three scratch pointers instead of slices
(same function, same instruction sequence, called from the same place), so the
fast body can hand it uninitialised storage. Probes and oracle confirm the
default build is unchanged.

There are no new dependencies; `examples/pvq_quality.rs` links the system
libopus exactly like `examples/snr_harness.rs`.

## 3. Measurement method

* **Builds.** Baseline = `git HEAD` (`src/celt/kernels.rs` reverted) built with
  `--features fast`; new = this tree built with `--features fast`; both
  `cargo build --release` and the same `compat/bench` build
  (`cargo build --release --offline --features oprs/fast`). The two bench
  binaries and the two `kernel_ab` binaries were built once, kept side by side
  and then run **alternating A/B/A/B in one session** (three rounds each), so
  no number here compares runs from different sessions. The host is shared with
  other work: per-run noise on the absolute times is 5–10 %, which is why the
  tables below report three rounds and medians, and why the kernel table uses a
  best-of-7 micro-benchmark instead of one end-to-end run.
* **Kernel.** `examples/kernel_ab.rs` (rows `op_pvq_search N=64 K=48` and
  `N=32 K=20`), plus an ad-hoc harness that timed the dispatch on a
  `(n, k)` grid with the baseline body selected through a temporary
  kill-switch (the switch and the harness were removed before hand-off; the
  numbers are reproduced in §4).
* **Quality.** `examples/pvq_quality.rs` (modelled on `snr_harness`) against the
  system libopus 1.6.1 on the 16 oracle configurations: decoder SNR of our
  packets, SNR against libopus's own packets, packet-size drift, interop both
  directions, and — the number that decides an algorithmic change — the
  **signal SNR** of `decode(our packets)` and `decode(libopus packets)` against
  the original deterministic signal.
* **Where PVQ time goes.** A temporary per-call histogram (removed) counted
  `(n, k)` over real encodes. For 48 kHz/20 ms/mono at 64 kbit/s, 50 frames:

  | band size | calls | Σ k·n (share of search work) |
  |---|---|---|
  | `n <= 4` | 332 | 13 578 (12 %) |
  | `5..16` | 675 | 69 527 (62 %) |
  | `17..32` | 245 | 27 180 (24 %; cumulative 98 %) |
  | `33..64` | 11 | 2 544 (2 %) |
  | `65..96` | 0 | 0 |
  | `> 96` | 0 | 0 |

  Stereo 48 kHz/20 ms/128 kbit/s has the same shape (n ≤ 32 = 92 % of the
  work), 48 kHz/60 ms reaches `33..64` (Σ k·n 31 872, the dominant bucket), and
  the 8 kHz/12 kHz VoIP oracle configurations issue **zero** PVQ calls (pure
  SILK frames). That is why only the three 48 kHz control rows can move at all,
  and why an eight-lane path sold as "for wide vectors" has nothing to do
  (§4.5).

## 4. Per-variant results

### 4.1 `K == 1`: argmax

The reference body already resolves `K == 1` to an `|x|` argmax (the greedy
loop's single pass has `xy == 0`, `yy == 1`, so the score is exactly `|x_j|`),
so this is a pure specialisation — except for the tie-break. The reference's
`_mm_max_epi16` bookkeeping only raises `pos` on a *strict* improvement over the
running per-lane maximum, so among equal magnitudes it picks the largest index
of the **earliest four-lane group** that reached the maximum; this body picks
the largest index overall. Measured with a targeted harness:

| input (`n = 39`, `k = 1`) | reference index | this body |
|---|---|---|
| single spike at 19 | 19 | 19 |
| ramp (unique max at 38) | 38 | 38 |
| all `|x|` equal | 3 | 38 |
| alternating ±0.25 | 3 | 38 |
| half zeros / half 0.7 | 22 | 38 |

Both always land on a maximum-magnitude coefficient; they differ only when the
maximum is tied. End-to-end that is **0.000 %** size drift and a **−0.001 dB**
signal-SNR delta (§5), i.e. an equally good quantisation.

Speed: replacing the vector pass with a scalar O(n) argmax removes the whole
call overhead — for `n = 8..16` the reference body costs 141–286 ns (§4.4,
including the 4.3 KiB zero-fill) while the argmax is a handful of
comparisons, i.e. ~10–20x on those calls. `K == 1` is however a small share of
the search *work* (it only appears on the highest bands / lowest rates), so it
does not move the end-to-end number on its own; it is kept because it is
strictly cheaper and quality-neutral.

### 4.2 `N == 2`: closed form with the exact reciprocal

For two dimensions the PVQ optimum is `y ∝ |X|`, so `y0 = round(K·|x0|/sum)`,
`y1 = K − y0` with an exact `1.0/sum` is the best integer split. The reference
projects with `_mm_rcp_ps`, truncates, and then runs the greedy loop. The pulse
vector differs (it is a different, equally valid codeword); both stay near
optimal, and the measured signal-SNR delta is **+0.004 dB** on the 5 ms
configuration where the change is concentrated.

Speed (ns/call, best of 7×40 000 calls):

| `n/k` | reference body | `N == 2` body | speed-up |
|---|---|---|---|
| 2/3 | 79.8 | 26.1 | 3.05x |
| 2/20 | 75.1 | 25.0 | 3.01x |

`N <= 4` — the union of the `N == 2` and `N == 4` bodies — is 25 % of the PVQ
*calls* and 13 % of the work at 48 kHz/20 ms (§3); most of the `N == 2` win is
the avoided 4.3 KiB scratch zero-fill, see §4.4.

### 4.3 `N == 4`: four-lane body

The whole search runs on one `__m128`: `X` and `2·iy` stay in registers, the
presearch and its reduction order are the reference's, and the per-pulse argmax
is `_mm_movemask_ps` + leading-zero instead of the `_mm_max_epi16` position
vector. Result: **bit-identical** pulse vectors and `yy`.

| `n/k` | reference body | `N == 4` body | speed-up |
|---|---|---|---|
| 4/3 | 111.7 | 49.0 | 2.28x |
| 4/20 | 90.6 | 28.6 | 3.16x |
| 4/48 | 108.1 | 46.7 | 2.32x |

(The `N <= 4` bucket as a whole is 25 % of PVQ calls / 13 % of the work at
48 kHz/20 ms; `N == 4` is the larger part of it.)

### 4.4 General body: same arithmetic, scratch not zero-filled

This turned out to be the real win, and it is not a wider vector. The reference
wrapper allocates three `[0.0f32; MAX_PVQ_N + 3]` arrays — 4.3 KiB — and
zero-fills them **on every call**; the C `ALLOC` is an `alloca` and does not
initialise. For a small band the zero-fill costs more than the search itself.
`op_pvq_search_fast_body` keeps the reference's four-lane arithmetic but uses
`[MaybeUninit<f32>; MAX_PVQ_N + 3]` storage and writes only the lanes it reads
(`X[0..N]` copied in, `X[N..N+3] = 0` as the reference does, `y`/`signy` written
by the first pass before any read). Result: **bit-identical**.

| `n/k` | reference body (zero-filled scratch) | fast body | speed-up |
|---|---|---|---|
| 8/4 | 140.5 | 101.3 | 1.39x |
| 8/20 | 154.6 | 111.4 | 1.39x |
| 12/9 | 182.3 | 143.8 | 1.27x |
| 16/9 | 285.9 | 252.3 | 1.13x |
| 16/20 | 211.8 | 182.8 | 1.16x |
| 17/12 | 238.3 | 199.6 | 1.19x |
| 24/12 | 352.3 | 320.4 | 1.10x |
| 32/20 | 408.7 | 350.1 | 1.17x |
| 48/24 | 841.1 | 823.7 | 1.02x |
| 64/48 | 1023.2 | 994.3 | 1.03x |
| 120/40 | 2410.9 | 2406.8 | 1.00x |
| 176/12 | 1164.6 | 1113.4 | 1.05x |

`examples/kernel_ab.rs` (three alternating rounds, ns/call, extracted from the
job log; the row names are the ones the harness prints):

| round | `op_pvq_search N=64 K=48` baseline (ns) | new (ns) | `N=32 K=20` baseline (ns) | new (ns) |
|---|---|---|---|---|
| 1 | 1034.4 | 982.1 | 372.1 | 328.5 |
| 2 | 1017.5 | 979.1 | 370.6 | 338.3 |
| 3 | 1005.9 | 1047.8 | 382.6 | 335.2 |
| median | 1017.5 | **982.1** | 372.1 | **335.2** |
| speed-up | | **1.036x** | | **1.110x** |

The harness's checksum column is identical between the two builds
(`9240000.0000` / `9100000.0000`), which is the expected result for a
bit-identical specialisation. Both `kernel_ab` sizes are larger than the bands
that dominate a real frame, so these are the *least* favourable of the kept
variants; §4 shows 1.1–1.4x at `n = 8..32`.

### 4.5 Eight-lane AVX2 scan — implemented, measured, **reverted**

`restsend/opus-rs`'s `pvq_search_avx2` widens the scan to eight lanes. An
AVX2+FMA eight-lane version of the *same* score function and tie-break was
implemented (so it was bit-identical for every case outside the `N == 2` and
`K == 1` bodies: a 2 912-case `(n, k)` sweep against the reference body was
exact) and measured against the reference body:

| `n/k` | reference body | 8-lane AVX2 | ratio (ref/8-lane) |
|---|---|---|---|
| 16/9 | 269.1 | 309.3 | 0.87 |
| 16/20 | 208.7 | 219.5 | 0.95 |
| 17/12 | 223.8 | 246.3 | 0.91 |
| 24/12 | 363.5 | 410.3 | 0.89 |
| 32/20 | 416.6 | 438.4 | 0.95 |
| 48/24 | 880.9 | 908.5 | 0.97 |
| 64/48 | 1052.4 | 1078.3 | 0.98 |
| 120/40 | 2606.5 | 2370.0 | **1.10** |
| 176/12 | 1173.9 | 1007.9 | **1.17** |

It only pays for `n >= ~96`, and **no oracle configuration reaches `n > 96`**
(§3: the largest bucket with any calls is `65..96`, one call in the 60 ms
configuration). Worse, that table already flatters the eight-lane body: the
reference column pays the 4.3 KiB zero-fill, while the eight-lane kernel used
`MaybeUninit` scratch. Against the kept fast body (§4.4) it would lose
everywhere except `n >= 120`. Cause: the per-pulse cost is not the scan loop —
it is the serial `xy`/`yy` update, the `y[best] += 2` scalar store (which makes
the next 8-lane load of `y` a store-forwarding stall), and the horizontal argmax
reduction (~65 cycles per pulse at `n = 32, k = 20`). Halving the loop count
removes about a third of the work and adds a wider reduction.

**Reverted.** The code is not in the tree.

### 4.6 Batched `fast_select` — rejected on the measured bottleneck

`restsend/opus-rs`'s `n >= 32` path (`pvq_search_fast_select`) selects up to
four pulses per pass from one score evaluation (with `select_nth_unstable_by`
for the large-`k` case), which amortises exactly the per-pulse costs above. It
was evaluated and not added, because:

* at `n >= 32` it only reaches 2–5 % of the PVQ work in the oracle
  configurations (`33..64` bucket: 2 % at 20 ms, dominant only at 60 ms);
* the four pulses of a batch are chosen from a stale `xy`/`yy`, and the batch
  must use distinct positions, which is precisely the regime where CELT's PVQ
  concentrates many pulses on one coefficient — a measurable RD loss for at
  most ~10 % of the search time that is itself 5–10 % of an encode frame;
* the divide-based score (`rxy*rxy/ryy`, their large-`k` branch) and the
  `select_nth` partition are not cheaper than three extra `rsqrt` passes at
  these sizes on this host.

Recorded here so the option is not re-tried blindly.

## 5. Quality: 16 oracle configurations (fast build, all kept variants)

`cargo run --release --features fast --example pvq_quality`:

| configuration | dec(our) dB | vs C pkts dB | size Δ % | Δframes | sig our dB | sig C dB | Δsig dB |
|---|---|---|---|---|---|---|---|
| 48k/stereo/audio/96k/c10/20ms | 131.1 | inf | +0.000 | 0 | −2.6 | −2.6 | +0.000 |
| 48k/stereo/voip/32k/c5/20ms | 129.1 | inf | +0.000 | 0 | −2.3 | −2.3 | +0.000 |
| 48k/mono/audio/64k/c10/10ms | 134.6 | inf | +0.000 | 0 | −2.6 | −2.6 | +0.000 |
| 48k/stereo/audio/128k/c10/5ms | 136.7 | 19.6 | +0.000 | 0 | −2.6 | −2.6 | +0.004 |
| 48k/stereo/lowdelay/64k/c10/2.5ms | inf | 14.6 | +0.000 | 0 | −2.8 | −2.8 | −0.001 |
| 24k/mono/voip/24k/c3/20ms | inf | inf | +0.000 | 0 | −2.2 | −2.2 | +0.000 |
| 16k/mono/voip/16k/c0/40ms | inf | inf | +0.000 | 0 | −1.8 | −1.8 | +0.000 |
| 8k/mono/voip/12k/c0/20ms | inf | inf | +0.000 | 0 | −1.8 | −1.8 | +0.000 |
| 48k/stereo/audio/24k/c10/60ms | 130.1 | inf | +0.000 | 0 | −2.3 | −2.3 | +0.000 |
| 48k/stereo/audio/96k/c10/20ms/cbr | 130.7 | inf | +0.000 | 0 | −2.6 | −2.6 | +0.000 |
| 48k/stereo/audio/96k/c10/20ms/cvbr | 130.5 | inf | +0.000 | 0 | −2.6 | −2.6 | +0.000 |
| 12k/mono/voip/20k/c5/20ms | inf | inf | +0.000 | 0 | −1.8 | −1.8 | +0.000 |
| 48k/stereo/voip/20k/c10/10ms/dtx | 130.6 | inf | +0.000 | 0 | −1.1 | −1.1 | +0.000 |
| 48k/stereo/audio/64k/c7/20ms/wb | 153.5 | inf | +0.000 | 0 | −1.3 | −1.3 | +0.000 |
| 48k/stereo/audio/64k/c7/20ms/nb | 166.8 | inf | +0.000 | 0 | −0.7 | −0.7 | +0.000 |
| 48k/stereo/voip/32k/c5/20ms/fec/loss | 134.2 | inf | +0.000 | 0 | −1.2 | −1.2 | +0.000 |

* **minimum decoder SNR 129.1 dB** (gate: ≥ 100 dB) — identical to the
  bit-exact build's 129.1 dB, i.e. the changed packets decode exactly as well in
  libopus as in this crate (this is the "encode/decode consistency" gate: it
  would collapse if a specialisation emitted an invalid codeword).
* **maximum |size drift| 0.000 %** (gate: ±2 %), and **0** frames with a
  different size: the pulse cost is fixed by `(N, K)`, so a size change would
  have meant a *changed allocation*, not a changed search.
* **worst signal-SNR delta −0.001 dB** (2.5 ms configuration), and **+0.004 dB**
  on the 5 ms configuration: against the original signal the changed
  quantisation is as good as libopus's, which is what allows keeping it.
* Interop holds in both directions for every packet of every configuration
  (`decode_*` panics otherwise), including the FEC/loss configuration.
* `vs C pkts` is `inf` for 14/16 configurations: the kept specialisations
  reproduce libopus's pulse vectors exactly there. The two non-`inf` rows are
  the small-frame configurations (2.5 ms / 5 ms), where `N == 2` / `N == 4` /
  `K == 1` bands dominate — that is the expected footprint of §4.1 and §4.2.

`cargo run --release --features fast --example snr_harness` → `QUALITY: PASS`
(minimum decoder SNR 129.1 dB, interop both directions), unchanged from the
baseline fast build.

## 6. End-to-end speed: the six `enc_i16` control rows

`compat/bench`, `BENCH_FILTER=enc_i16`, three alternating A/B/A/B rounds of the
baseline and new `--features fast` binaries in one session. `r_med` is the
`oprs`/`audiopus` median ratio the task's control rows use; `opr ns/op` is the
absolute per-frame time of our build, which is the less noisy of the two on a
shared host. Rows are given as `r_med (baseline → new)` with the median of the
three rounds.

| control row | `r_med` baseline (3 rounds) | `r_med` new (3 rounds) | median `r_med` base → new | `opr ns/op` median base → new | PVQ calls? |
|---|---|---|---|---|---|
| 8k/mono/10ms/lowdelay | 1.337, 1.344, 1.319 | 1.303, 1.433, 1.312 | 1.337 → 1.312 | 45 236 → 44 617 | no (SILK) |
| 8k/mono/20ms/lowdelay | 1.181, 1.224, 1.227 | 1.182, 1.326, 1.200 | 1.224 → 1.200 | 67 523 → 67 248 | no (SILK) |
| 12k/mono/20ms/audio | 1.209, 1.183, 1.218 | 1.258, 1.232, 1.171 | 1.209 → 1.232 | 78 812 → 81 105 | no (12 kHz is SILK-only) |
| 48k/mono/20ms/audio | 1.110, 1.091, 1.090 | 1.090, 1.107, 1.076 | 1.091 → 1.090 | 131 083 → 135 723 | yes |
| 48k/mono/20ms/lowdelay | 1.078, 1.079, 1.077 | 1.059, 1.064, 1.074 | 1.078 → 1.064 | 124 684 → 130 085 | yes |
| 48k/2ch/20ms/audio | 1.051, 1.065, 1.070 | 1.014, 1.125, 1.019 | 1.065 → 1.019 | 299 780 → 298 058 | yes |

Interpretation, honestly: the six control rows are **noise-limited on this
shared host**. The three non-CELT rows cannot move (zero PVQ calls: 8 kHz
and 12 kHz streams are SILK-only) and their apparent −2 %…+2 % spread is pure
noise, which calibrates the method: three
rounds of `r_med` resolve nothing smaller than ~5 %. On the three CELT rows the
`r_med` medians move in the right direction (48k/2ch by 4 %), but the per-round
spread is larger than the PVQ share itself, and the absolute `opr ns/op`
medians move the other way on two rows — an artefact of the per-run load, not
of the build: the two builds emit **byte-identical packets** for these
configurations (verified by hashing every packet, see below), so the amount of
work per frame is identical.

A quieter, encode-only measurement (our encoder alone, 1000 frames per rep,
min of 12 reps, five alternating rounds, `taskset -c 3`) resolves it:

| configuration | packet bytes hash | baseline ns/frame (5 rounds) | new ns/frame (5 rounds) | median speed-up |
|---|---|---|---|---|
| 48k/mono/20ms/audio/64k | identical (`5b7bc3ed…`) | 116 602 / 126 567 / 117 517 / 120 965 / 120 592 | 119 959 / 115 836 / 115 517 / 120 197 / 119 009 | **1.013x** |
| 48k/mono/20ms/lowdelay/52k | identical (`3df83ed4…`) | 117 878 / 116 002 / 117 552 / 122 396 / 114 721 | 119 091 / 116 321 / 114 392 / 113 915 / 115 278 | **1.020x** |
| 48k/2ch/20ms/audio/128k | identical (`48d8e3e8…`) | 186 371 / 192 126 / 190 992 / 189 578 / 189 480 | 188 884 / 191 203 / 190 961 / 186 777 / 183 801 | **1.004x** |
| 48k/mono/10ms/audio/64k | identical (`e9e18181…`) | 75 677 / 74 085 / 73 125 / 75 934 / 75 604 | 73 630 / 72 353 / 74 144 / 71 488 / 73 692 | **1.027x** |

So the end-to-end gain is **0.4–2.7 % per encode frame** when the bitstream does
not move, which is exactly what the PVQ share predicts: the histogram (§3)
gives ~3.4 % of a 48 kHz/20 ms frame in the search, and the kept variants make
that search ~10–39 % faster at the dominant band sizes. The 10 ms row gains most
because it has twice as many small bands per second.

The 2.5 ms/5 ms configurations (where `N == 2`/`N == 4` dominate and the
bitstream does change) are not in the six control rows; for them the same
arithmetic gives a larger share (25 % of the calls are `N <= 4`, 2.3–3.2x
faster), and the quality harness confirms the packets stay valid and the
signal SNR unchanged.

## 7. Gates

| gate | command | result |
|---|---|---|
| default build | `cargo build --release` | **ok** |
| default probes | `bash compat/scripts/run-all-probes.sh` | **all 25 probes are bit-exact** (`probe_vq: IDENTICAL`, 2773 lines) |
| default oracle | `bash compat/scripts/run-oracle.sh` | **ACCEPTANCE: BIT-EXACT** (`enc=0xdbce15d74e829a43 dec=0xf73254ad00d572f1` for both) |
| default tests | `cargo test --lib --release` | **315 passed; 0 failed** |
| fast build | `cargo build --release --features fast` | **ok** |
| fast tests | `cargo test --lib --release --features fast` | **315 passed; 0 failed** |
| fast quality gate | `cargo run --release --features fast --example snr_harness` | **QUALITY: PASS** (min 129.1 dB, interop both directions) |
| new PVQ quality gate | `cargo run --release --features fast --example pvq_quality` | **QUALITY: PASS** (min 129.1 dB, max size drift 0.000 %, signal-SNR loss 0.001 dB) |
| new harness sanity (default) | `cargo run --release --example pvq_quality` | `inf` decoder SNR, 0.000 % drift, `+0.000 dB` signal delta |

The default-build gates ran with the parallel SILK work present in
`src/silk/**` (not touched here); the bit-exactness of the default build is
unaffected by the `--features fast` code, which is entirely `cfg`-gated, and the
only default-visible edit is the pointer signature of the (unchanged)
`op_pvq_search_sse2_hw`.

## 8. Files

* `src/celt/kernels.rs` — `[fast] PVQ search specialisations` section and the
  dispatch hook in `op_pvq_search_sse2`; `op_pvq_search_sse2_hw` now takes raw
  pointers (default arithmetic unchanged).
* `examples/pvq_quality.rs` — new quality harness (16 oracle configurations,
  decoder SNR, size drift, signal SNR, interop).
* `docs/reports/pvq-specialisations.md` — this report.
* `src/celt/vq.rs` — one comment in `alg_quant` pointing at the dispatch (which
  lives in `kernels.rs` so both `vq.rs` and `examples/kernel_ab.rs` exercise
  it); no code change.

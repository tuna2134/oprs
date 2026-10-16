# Porting conventions

The port is a **faithful transliteration** of libopus 1.6.1 into safe Rust.
Bit-exactness with the installed `libopus` is the acceptance criterion, so
readability-by-diff against the C source outranks idiomatic style.

## Reference configuration (pinned)

`compat/scripts/build-reference.sh` builds the reference library into
`reference/lib-ref/libopus.a`.  Its configuration reproduces the installed
`opus 1.6.1-1` (Arch Linux) package **bit-for-bit** on all 16 oracle
configurations:

```
floating point (FIXED_POINT undefined)
FLOAT_APPROX undefined
x86 intrinsics + RTCD enabled  (host arch level 4: AVX2/FMA capable CPU)
ENABLE_DEEP_PLC, ENABLE_DRED, ENABLE_OSCE, CUSTOM_MODES, ENABLE_HARDENING
```

`ENABLE_OSCE_BWE`: the Arch/meson-built *system* library does **not** define it,
while the autotools reference build produced by `build-reference.sh` **does**.
The two are still bit-identical on the whole 16-configuration oracle, because
the blind-BWE path only activates at decoder complexity >= 4 and the decoder
complexity is 0 by default (and `audiopus` does not expose a decoder complexity
control).  The port therefore targets the autotools reference; where a struct
layout matters for a C probe, note that the C side is compiled with
`ENABLE_OSCE_BWE` defined and the Rust structs currently omit those fields.

`ENABLE_HARDENING` is active, so `celt_assert`/`silk_assert` are **live aborts**
in the reference, not no-ops.

The host CPU selects `arch == 4`.  **The reference therefore executes the x86
SIMD kernels, not the plain C fallbacks.**  Where a dispatched SIMD kernel is
not arithmetically identical to the C version (see `docs/rtcd-kernels.md`) the
Rust port must reproduce the *SIMD* arithmetic, not the C arithmetic.

## Module layout

One Rust module per C file, under the same directory name:

| C | Rust |
| --- | --- |
| `celt/foo.c`, `celt/foo.h` | `src/celt/foo.rs` |
| `silk/foo.c`, `silk/foo.h` | `src/silk/foo.rs` |
| `src/foo.c` | `src/opus/foo.rs` |

## Rules

1. **Keep the C names.**  Every module starts with
   `#![allow(non_snake_case)]`-style allowances (already set on the parent
   module).  Functions, structs, fields, constants and macros keep their C
   names so that a diff against the C file is meaningful.
2. **Keep the original copyright header.**  The port is a derived work of the
   BSD-3-Clause libopus sources; copy the copyright notice of the C file into
   the Rust file and note that it is a transliteration.
3. **Types.**  Use the aliases from `crate::types` (`opus_int16`, `opus_uint32`,
   …).  In this configuration `opus_val16`, `opus_val32`, `opus_val64`,
   `celt_sig`, `celt_norm`, `celt_ener`, `opus_res`, `celt_coef` are all `f32`
   (re-exported from `crate::celt::arch`).
4. **Macros.**  The C macros that do arithmetic in a float build are available
   as `#[inline(always)]` functions in `crate::celt::arch` with their C names
   (`MULT16_16`, `MAC16_16`, `SHR32`, `SATURATE`, `HALF16`, `ADD32`, …).  Use
   them verbatim so the code matches the C.  Do **not** "simplify" them.
5. **Integer arithmetic.**  C relies on wrapping for unsigned types and on
   implementation-defined behaviour for some signed shifts.  Use explicit
   `wrapping_add` / `wrapping_sub` / `wrapping_mul` / `wrapping_shl` wherever
   C could wrap, so that debug builds do not panic and release builds match C.
   Never use a `usize` where C uses a 32-bit type.
6. **Pointers and buffers.**  Replace `T *p` + length with `&[T]` / `&mut [T]`,
   or with an owned `Vec<T>` when the C code allocates through `ALLOC`.
   `VARDECL(T, x); ALLOC(x, n, T);` becomes `let mut x = vec![T::default(); n];`
   (or a stack array when `n` is a small compile-time constant).  C code that
   aliases through raw pointers occasionally has to be rewritten carefully —
   keep the arithmetic order identical.
7. **Assertions.**  `celt_assert!`, `celt_assert2!`, `celt_sig_assert!` are
   available from `crate::celt::arch`; they compile to `debug_assert!`, which
   matches the reference (built without `ENABLE_ASSERTIONS`).
8. **No external crates.**  The library must stay dependency-free.  `libm`
   calls must go through `f32`/`f64` inherent methods so that the system libm
   is used, exactly like the C.
9. **Compile-time configuration.**  The port targets one configuration only
   (see above).  Dead `#ifdef` branches for other configurations are dropped;
   branches that are active in the reference configuration are ported.
   `ENABLE_OSCE`, `ENABLE_DEEP_PLC`, `ENABLE_DRED` are active: keep their
   `#ifdef` blocks.  `ENABLE_OSCE_BWE` is *not* active: drop those blocks.
10. **No panics.**  Index out of bounds, unwrap, and integer overflow in debug
    builds must never be reachable for valid input.

## Verification is mandatory

Every ported module must be accompanied by a probe that proves bit-exactness
against the reference C library.  The recipe:

1. Write `compat/c/probe_<name>.c` that exercises the C function(s) with a
   deterministic pseudo-random input and prints every intermediate state
   (state structs, output samples, tell values, …).
2. Write `examples/probe_<name>.rs` that runs the identical sequence with the
   Rust port and prints the identical format.  Use the same LCG
   (`state = state*1103515245 + 12345`) so both sides generate identical inputs.
3. Run `bash compat/scripts/run-probe.sh <name>`; it builds both, runs both and
   diffs the output.  The only acceptable result is `IDENTICAL`.

The C probe can call *internal* libopus functions (they are ordinary external
symbols in the static library); pass `-I reference/opus-1.6.1/{include,celt,silk,src,dnn}`
as `run-probe.sh` does.

### Probe pitfalls that have already cost time

* **C macros evaluate their arguments more than once.**  A line like
  `cap[j] = IMIN(cap[j], (int)(rnd()%256));` calls `rnd()` **twice** whenever
  the second branch of the ternary is taken, so the C and Rust LCG streams
  drift apart and every later case differs.  Always draw random values into a
  local first.
* **C reuses stack buffers between iterations.**  Arrays that are only written
  on `[start, end)` hold stale values from the previous case in C, while a
  fresh Rust `Vec` is zeroed.  Clear the buffers explicitly on both sides at
  the start of each case.
* **Probes must print the inputs, not just the outputs.**  The `rate 13`
  divergence above looked like an allocation bug for a while; it was obvious
  once the differing line showed the *inputs* already differed.
* The main crate is edited by several agents at once, so `cargo build` (and
  therefore every probe) can fail transiently in a file you do not own.  Check
  whether the errors are yours before reacting; re-run after a short wait.


Printing the full state after every operation is strongly preferred over a
final hash: when the two sides disagree the first differing line localises the
bug immediately.

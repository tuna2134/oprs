#!/usr/bin/env python3
"""Section 5: state structs, verbatim."""
import os, re, subprocess

REF = "/home/m-kikuchi/projects/oprs/reference/opus-1.6.1"
OUT = "/home/m-kikuchi/projects/oprs/docs/_s5.md"

def lines(p):
    return open(os.path.join(REF, p), errors="replace").read().split("\n")

def grab(p, a, b):
    ls = lines(p)
    return "\n".join(ls[a - 1:b])

def fence(p, a, b, note=None):
    s = "`%s:%d-%d`\n\n```c\n%s\n```\n" % (p, a, b, grab(p, a, b))
    if note:
        s += "\n" + note + "\n"
    return s + "\n"

# (level, heading, prose, [(file, a, b, note), ...])
SECTIONS = [
(3, "5.1 `OpusEncoder` (`src/opus_encoder.c`)",
 "Defined at `src/opus_encoder.c:76-146`. The struct is preceded by "
 "`StereoWidthState` (`:70-74`) and the `MAX_ENCODER_BUFFER` macro "
 "(`:62-67`), which is `960` under `ENABLE_QEXT` and `480` otherwise.",
 [("src/opus_encoder.c", 62, 74, None),
  ("src/opus_encoder.c", 76, 146,
   "**Porting notes.**\n"
   "* `celt_enc_offset`/`silk_enc_offset` are *byte offsets into the same "
   "allocation*: `opus_encoder_init` (`src/opus_encoder.c:205-245`) computes\n"
   "  `base_size = align(sizeof(OpusEncoder))`, subtracts part of "
   "`delay_buffer` for 1-channel / restricted applications, then lays out "
   "`[OpusEncoder][silk_encoder][OpusCustomEncoder]` and stores the offsets. "
   "So the C struct is a variable-size arena. A Rust port can use owned "
   "sub-structs, but `opus_encoder_get_size()` must still return the same "
   "number if it is exposed.\n"
   "* `#define OPUS_ENCODER_RESET_START stream_channels` marks the first field "
   "cleared by `opus_encoder_ctl(OPUS_RESET_STATE)`; every field from "
   "`stream_channels` down is reset, everything above is retained. The Rust "
   "port must reproduce that split exactly (`reset_state_preserving_prefix`).\n"
   "* `delay_buffer[MAX_ENCODER_BUFFER*2]` is the last field and is "
   "deliberately over-sized: for `application == "
   "OPUS_APPLICATION_RESTRICTED_*` it is partially or wholly omitted. It is "
   "the resampler-delay compensation ring buffer.\n"
   "* `energy_masking` is a pointer (`celt_glog *`) into a caller-provided "
   "buffer set via `OPUS_SET_ENERGY_MASK`; it is **not** owned by the state.\n"
   "* `#ifndef DISABLE_FLOAT_API` fields (`analysis`, `detected_bandwidth`) "
   "are only present when the float API is compiled in. `analysis` is a large "
   "inline `TonalityAnalysisState` — its size changes `opus_encoder_get_size()`."),
 ]),

(3, "5.2 `OpusDecoder` (`src/opus_decoder.c`)",
 "Defined at `src/opus_decoder.c:65-94`.",
 [("src/opus_decoder.c", 65, 94,
   "**Porting notes.**\n"
   "* Same arena trick as the encoder: `opus_decoder_init` sets "
   "`st->silk_dec_offset = align(sizeof(OpusDecoder));` and "
   "`st->celt_dec_offset = st->silk_dec_offset + silkDecSizeBytes;`, so the "
   "allocation is `[OpusDecoder][silk_decoder][OpusCustomDecoder]`.\n"
   "* `#define OPUS_DECODER_RESET_START stream_channels` is the reset split.\n"
   "* `softclip_mem[2]` exists only in the **non**-fixed build; in the "
   "fixed build `opus_pcm_soft_clip` is a no-op and the field is absent.\n"
   "* `LPCNetPLCState lpcnet` is a large inline DNN state present only under "
   "`ENABLE_DEEP_PLC`; it dominates `opus_decoder_get_size()` in that build."),
  ("src/opus_decoder.c", 1339, 1346,
   "`OpusDREDDecoder` (DRED only; DNN, out of scope).")]),

(3, "5.3 `silk_encoder_state` and friends (`silk/structs.h`)",
 "These are the **per-channel** SILK state structs shared by the fixed and "
 "float builds. `silk/structs.h:146-239` is `silk_encoder_state`; "
 "`silk_decoder_state` is `:286-341`; `silk_decoder_control` is `:346-354`.",
 [("silk/structs.h", 56, 69, None),
  ("silk/structs.h", 74, 127, None),
  ("silk/structs.h", 129, 141,
   "`SideInfoIndices` — the decoded/encoded index set passed between index "
   "coding and parameter reconstruction."),
  ("silk/structs.h", 146, 239,
   "`silk_encoder_state`. Note the `#ifdef _WIN32` float variants, "
   "`silk_DWORD_ALIGN` on `predCoef_Q12`/`LTPCoef_Q12`/`AR_Q13`, and the "
   "many `MAX_*`-sized arrays (see `silk/define.h`). Fields with "
   "`FIXED_POINT`-conditional types (`silk_float` vs Q-domain ints) must be "
   "resolved by choosing the fixed build."),
  ("silk/structs.h", 243, 252,
   "`ENABLE_OSCE` only (DNN, out of scope)."),
  ("silk/structs.h", 256, 281, None),
  ("silk/structs.h", 286, 341, None),
  ("silk/structs.h", 346, 354, None)]),

(3, "5.4 `silk_encoder` / `silk_decoder` super-structs",
 "These are the **top-level** SILK state objects that `OpusEncoder`/`OpusDecoder` "
 "embed. `silk_encoder` is **not** in `silk/structs.h`: it is defined "
 "separately for the fixed and float builds in `silk/fixed/structs_FIX.h` and "
 "`silk/float/structs_FLP.h`. `silk_decoder` is defined locally in "
 "`silk/dec_API.c`.",
 [("silk/fixed/structs_FIX.h", 30, 109,
   "The whole fixed-build header: `silk_shape_state_FIX` (`:39-44`), "
   "`silk_encoder_state_FIX` (`:49-57`), `silk_encoder_control_FIX` "
   "(`:62-89`) and `silk_encoder` (`:94-106`). **This is the state a "
   "bit-exact fixed-point port must reproduce.**"),
  ("silk/float/structs_FLP.h", 30, 105,
   "The float-build counterpart, for reference only (not bit-exact). Note it "
   "also defines a type named `silk_encoder`, so the two headers are mutually "
   "exclusive — exactly as `FIXED_POINT` selects."),
  ("silk/dec_API.c", 44, 56,
   "`silk_decoder`. `DECODER_NUM_CHANNELS` is 2. Under `ENABLE_OSCE` an "
   "`OSCEModel osce_model` field is appended.")]),

(3, "5.5 `OpusCustomEncoder` / `OpusCustomDecoder` (= `CELTEncoder` / `CELTDecoder`)",
 "`celt/celt.h:57-59` defines the aliases:\n"
 "```c\n#define CELTEncoder OpusCustomEncoder\n#define CELTDecoder OpusCustomDecoder\n"
 "#define CELTMode OpusCustomMode\n```\n"
 "The struct bodies live in the encoder/decoder `.c` files, not in the header.",
 [("celt/celt_encoder.c", 63, 142,
   "`struct OpusCustomEncoder`. It embeds an `AnalysisInfo analysis` and, "
   "under `ENABLE_QEXT`, a `SILKInfo silkinfo`; it also embeds "
   "`OpusCustomMode`-independent state plus `prefilter_period_old`, etc. "
   "Under `ENABLE_DEEP_PLC`, DNN members are appended."),
  ("celt/celt_decoder.c", 87, 139, "`struct OpusCustomDecoder`.")]),

(3, "5.6 `OpusCustomMode` / `CELTMode` and `PulseCache` (`celt/modes.h`)",
 None,
 [("celt/modes.h", 44, 49, "`PulseCache`."),
  ("celt/modes.h", 54, 77,
   "`struct OpusCustomMode`. `ENABLE_QEXT` appends the QEXT fields. The "
   "`opus_*` API never sees this struct, but the opus layer reads its fields "
   "directly (see §4.10).")]),

(3, "5.7 FFT and MDCT state (`celt/kiss_fft.h`, `celt/mdct.h`)",
 None,
 [("celt/kiss_fft.h", 78, 110, None),
  ("celt/mdct.h", 49, 54, None)]),

(3, "5.8 `OpusRepacketizer` (`src/opus_private.h`)",
 None,
 [("src/opus_private.h", 39, 48, None)]),

(3, "5.9 Supporting state structs a port must also reproduce",
 "These are all declared in `silk/structs.h` unless noted, and every one of "
 "them is embedded (by value) in `silk_encoder_state`, `silk_decoder_state` or "
 "`silk_encoder`, so their exact layout and element counts matter.",
 [("silk/structs.h", 88, 93, "`silk_LP_state`."),
  ("silk/structs.h", 96, 109,
   "`silk_NLSF_CB_struct` — the pointer table that binds a codebook "
   "(`silk_NLSF_CB_NB_MB` / `silk_NLSF_CB_WB`) to its derived vectors."),
  ("silk/resampler_structs.h", 30, 50,
   "`silk_resampler_state_struct`. `#ifdef USE_SMALL_DIV_TABLE` changes the "
   "table reference; the coefficient table pointer must point at "
   "`silk_resampler_frac_FIR_12` / the up2-HQ arrays from "
   "`silk/resampler_rom.c`."),
  ("silk/control.h", 42, 120,
   "`silk_EncControlStruct` — **the** encoder/decoder control block that "
   "`src/opus_encoder.c` fills in and `silk_Encode` consumes. Its fields are "
   "the actual cross-layer contract (`nChannelsAPI`, `nChannelsInternal`, "
   "`API_sampleRate`, `maxInternalSampleRate`, `minInternalSampleRate`, "
   "`desiredInternalSampleRate`, `payloadSize_ms`, `bitRate`, "
   "`packetLossPercentage`, `complexity`, `useInBandFEC`, `useDTX`, "
   "`useCBR`, `maxBits`, `toMono`, `opaque`, `useHardRedundancy`, ...)."),
  ("silk/control.h", 125, 162, "`silk_DecControlStruct`."),
  ("silk/API.h", 44, 48, "`silk_TOC_struct`."),
  ("celt/celt.h", 65, 84,
   "`AnalysisInfo` and `SILKInfo` — embedded in the CELT encoder state; "
   "`AnalysisInfo` is filled by the float-API analysis layer and influences "
   "CELT mode decisions. `leak_boost[LEAK_BANDS]` is `unsigned char` even in "
   "the float build (\"Store as Q6 char to save space\")."),
  ("src/mapping_matrix.h", 39, 45,
   "`MappingMatrix` — needed only for the multistream/projection layer.")]),

(3, "5.10 The entropy coder: one struct for both directions",
 "`celt/entcode.h:46-48` declares a **single** struct used for both encoder "
 "and decoder:\n"
 "```ctypedef struct ec_ctx         ec_ctx;\ntypedef struct ec_ctx         ec_enc;\n"
 "typedef struct ec_ctx         ec_dec;\n```\n"
 "The union-of-roles is deliberate (\"We use the same structure for both, so "
 "that common functions like ec_tell() can be used on either one\"). A Rust "
 "port should mirror this with one `EcCtx` type used by both `enc` and `dec` "
 "modules, so that `ec_tell` can be shared.",
 [("celt/entcode.h", 42, 91,
   "Includes the exact `#define`s: `EC_WINDOW_SIZE`, `EC_UINT_BITS (8)`, "
   "`BITRES (3)`, and the `USE_SMALL_DIV_TABLE` switch.")]),
]

# extra: EC constants elsewhere
EXTRA = """
### 5.11 Entropy-coder bit-exact invariants

The following invariants are what make the range coder reproducible. They are
derived from `celt/entcode.h`, `celt/mfrngcod.h`, `celt/entenc.c`,
`celt/entdec.c`; the exact code is quoted in §7.4.

| Field | Encoder meaning | Decoder meaning | Invariant |
|---|---|---|---|
| `buf` | output buffer | input buffer | never reallocated while in use; `ec_enc_shrink` only lowers `storage`. |
| `storage` | usable bytes | usable bytes | encoder: `storage` may be *decreased* by `ec_enc_shrink`; decoder: negative-byte reads are clamped and set `error`. |
| `end_offs` | offset of the last byte written with raw bits, counting **down from `storage`** | offset of the last byte read for raw bits, counting down from `storage` | raw bits are stored at the **end** of the buffer, range-coded bits from the front; the two halves must not collide. `end_offs` grows downward. |
| `end_window` | accumulator of raw bits not yet flushed | accumulator of raw bits not yet consumed | `EC_WINDOW_SIZE == sizeof(ec_window)*CHAR_BIT == 32`. `ec_enc_bits`/`ec_dec_bits` must fill/drain in exactly the same order. |
| `nend_bits` | number of valid bits in `end_window` | idem | `0 <= nend_bits < EC_WINDOW_SIZE` after each call; `ec_enc_done`/`ec_dec_init` must leave it 0. |
| `nbits_total` | total whole bits written (excludes partial bits in the range coder) | total whole bits read | monotone non-decreasing; `ec_tell()` = `nbits_total - EC_ILOG(rng)`. |
| `offs` | offset of the next range-coder byte to write | offset of the next byte to read | Writes are **refused** once `offs+end_offs >= storage` (`ec_write_byte`/`ec_write_byte_at_end` return `-1`, and the caller folds that into `error`), so `offs <= storage` always. The decoder returns `0` for reads past `storage` (`ec_dec_read_byte`) and records `error`. API contract: `nbCompressedBytes >= 2` (`celt/celt_encoder.c:1831`), `max_data_bytes >= 3` (`src/opus_encoder.c:1340`). |
| `rng` | size of the current range | size of the current range | `EC_CODE_TOP <= rng <= 2^32-1` after normalisation; `rng` drives `ec_tell` via `EC_ILOG`. |
| `val` | low end of the current range | `EC_CODE_TOP - 1 - (value - low)` | encoder `val` only enters `ec_enc_carry_out`; decoder `val` is *subtracted* in `ec_decode`. |
| `rem` | buffered byte awaiting carry propagation | (unused by the decoder's arithmetic; still present) | `rem` must be flushed by `ec_enc_carry_out` exactly once; `-1` means "no buffered byte". |
| `ext` | number of outstanding carry-propagating symbols | saved normalisation factor from `ec_decode` | `ec_encode` must set `ext = rng/_ft` and the `while (rng <= EC_CODE_BOTTOM)` loop must run the same number of iterations as C. |
| `error` | (not used) | set on malformed input | must be sticky: once set, it stays set; `ec_dec_bits`/`ec_dec_icdf` must degrade identically. |

Constants (exact text in §7.4): `EC_SYM_BITS = 8`, `EC_CODE_BITS = 32`,
`EC_SYM_MAX = 0xFF`, `EC_CODE_TOP = 1u<<(EC_CODE_BITS-1)`,
`EC_CODE_BOT = EC_CODE_TOP>>EC_SYM_BITS`,
`EC_CODE_EXTRA = (EC_CODE_BITS-2)%EC_SYM_BITS+1`,
`EC_CODE_BOTTOM = EC_CODE_BOT<<EC_CODE_EXTRA`, `BITRES = 3`,
`EC_UINT_BITS = 8`, `EC_WINDOW_SIZE = 32`.

`ec_tell()`/`ec_tell_frac()` are *derived* values, so they must not be stored:
```c
static OPUS_INLINE int ec_tell(ec_ctx *_this){
  return _this->nbits_total-EC_ILOG(_this->rng);
}
```
and `ec_tell_frac` (`celt/entcode.c:69-112`) is a 3-bit fractional refinement
built from `rng` alone — see §7.4 for the exact loop.

### 5.12 Allocation / ownership summary

| Struct | Allocation in C | Rust recommendation |
|---|---|---|
| `OpusEncoder` | variable-size arena `[OpusEncoder][silk_encoder][OpusCustomEncoder]`, offsets stored in the struct | owned `Box<OpusEncoder>` containing `silk: SilkEncoder`, `celt: CeltEncoder` fields (or `Box`ed); replicate `OPUS_ENCODER_RESET_START`. |
| `OpusDecoder` | arena `[OpusDecoder][silk_decoder][OpusCustomDecoder]` | same pattern; replicate `OPUS_DECODER_RESET_START`. |
| `silk_encoder` | `state_Fxx[MAX_NUM_CHANNELS]` inline, plus `sStereo` | inline array of 2 `SilkEncoderState`, indexed by channel. |
| `silk_decoder` | `channel_state[DECODER_NUM_CHANNELS]` inline | inline array of 2 `SilkDecoderState`. |
| `silk_resampler_state_struct` | inline in `silk_encoder_state`/`silk_decoder_state` | inline; the `sCoefs`/`sFIR`/`sIIR` unions are plain fixed arrays. |
| `ec_ctx` | inline in `OpusEncoder`? — **no**: created on the stack in `opus_encode_native`/`opus_decode_frame` | `EcCtx` value on the stack, passed as `&mut`. |
| `CELTMode`/`OpusCustomMode` | one per rate, created by `opus_custom_mode_create`, cached in `static` pointers in `src/opus_encoder.c`/`opus_decoder.c` | lazily-initialised `static`/`OnceLock` per sampling rate, matching the C static cache. |
| mode tables (`static_modes_fixed.h`) | `static const` arrays in the mode object; some in read-only data, some allocated by `opus_custom_mode_create` | `&'static` tables from generated Rust, plus heap-allocated `allocVectors`/`cache` where C allocates. |

---
"""

o = []
o.append("## 5. State structs\n")
o.append("Every struct below is reproduced **verbatim** from the 1.6.1 tree, with "
         "the exact field declarations (including `#ifdef` guards, array bounds "
         "and alignment attributes), because a bit-exact port must reproduce the "
         "field set — and, where the layout is observable (arena offsets, "
         "`opus_*_get_size()`), the byte layout too.\n")

for lvl, head, prose, grabs in SECTIONS:
    o.append("#" * lvl + " " + head + "\n")
    if prose:
        o.append(prose + "\n")
    for (p, a, b, note) in grabs:
        o.append(fence(p, a, b, note))

o.append(EXTRA)
open(OUT, "w").write("\n".join(o))
print("wrote", OUT, sum(1 for _ in open(OUT)), "lines")

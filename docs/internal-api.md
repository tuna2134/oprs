# Internal Opus-layer API (Rust)

This is the contract between the ported codec (`src/celt`, `src/silk`,
`src/opus`) and the `audiopus`-compatible public API (`src/lib.rs`,
`src/error.rs`, `src/coder/**`, `src/packet.rs`, `src/repacketizer.rs`,
`src/softclip.rs`).

The internal layer mirrors the C API names and semantics one-to-one, but with
safe Rust signatures: no raw pointers, no `unsafe`, errors returned as the
original negative `OPUS_*` codes.  The public layer is responsible for
converting those codes into `Error`/`ErrorCode`.

## `src/opus/encoder.rs` (mirrors `src/opus_encoder.c`)

```rust
pub struct OpusEncoder { /* ported state */ }

impl OpusEncoder {
    pub fn get_size(channels: i32) -> i32;
    pub fn create(fs: i32, channels: i32, application: i32, error: &mut i32) -> Option<Box<OpusEncoder>>;
    pub fn init(&mut self, fs: i32, channels: i32, application: i32) -> i32;
    pub fn destroy(self) {}
    /// Replaces `opus_encoder_ctl(st, request, value)`.  `value` is an in/out
    /// integer; for GET requests the result is written back into it.
    pub fn ctl(&mut self, request: i32, value: &mut i32) -> i32;
    pub fn encode(&mut self, pcm: &[i16], frame_size: i32, data: &mut [u8], max_data_bytes: i32) -> i32;
    pub fn encode_float(&mut self, pcm: &[f32], frame_size: i32, data: &mut [u8], max_data_bytes: i32) -> i32;
}

/// `opus_get_version_string()`
pub fn opus_get_version_string() -> &'static str;
```

## `src/opus/decoder.rs` (mirrors `src/opus_decoder.c`)

```rust
pub struct OpusDecoder { /* ported state */ }

impl OpusDecoder {
    pub fn get_size(channels: i32) -> i32;
    pub fn create(fs: i32, channels: i32, error: &mut i32) -> Option<Box<OpusDecoder>>;
    pub fn init(&mut self, fs: i32, channels: i32) -> i32;
    pub fn destroy(self) {}
    pub fn ctl(&mut self, request: i32, value: &mut i32) -> i32;
    /// `data == None` means packet loss (PLC); `len` is the C `len` argument.
    pub fn decode(&mut self, data: Option<&[u8]>, len: i32, pcm: &mut [i16], frame_size: i32, decode_fec: i32) -> i32;
    /// `opus_decode_float()`: the `f32` counterpart of `decode()`; the packet is
    /// still bytes, only the output type changes.
    pub fn decode_float(&mut self, data: Option<&[u8]>, len: i32, pcm: &mut [f32], frame_size: i32, decode_fec: i32) -> i32;
}
```

> **Correction (API layer).** An earlier revision of this document spelled
> `decode_float` as `data: Option<&[f32]>`, which cannot be right: the packet to
> decode is bytes.  The public layer (`src/coder/decoder.rs`) calls it with
> `data: Option<&[u8]>` and `pcm: &mut [f32]`, matching `opus_decode_float()` in
> C, and the placeholder in `src/opus/decoder.rs` declares exactly that.
>
> `opus_decoder_get_nb_samples(const OpusDecoder*, const unsigned char*, opus_int32)`
> is **not** part of the contract above (it is missing from this document).  The
> public layer emulates it with `OPUS_GET_SAMPLE_RATE_REQUEST` +
> `opus_packet_get_nb_samples()`, which is what the C does
> (`opus_packet_get_nb_samples(packet, len, dec->Fs)`), so no extra method is
> required.  If the port would rather expose
> `OpusDecoder::get_nb_samples(&self, packet: &[u8], len: i32) -> i32`, the API
> layer can be switched to it, but the documented set of methods must not grow
> silently.

## `src/opus/packet.rs` (mirrors `src/opus.c`) — **implemented and verified**

```rust
pub fn opus_packet_get_bandwidth(data: &[u8]) -> i32;
pub fn opus_packet_get_samples_per_frame(data: &[u8], Fs: i32) -> i32;
pub fn opus_packet_get_nb_channels(data: &[u8]) -> i32;
pub fn opus_packet_get_nb_frames(packet: &[u8], len: i32) -> i32;
pub fn opus_packet_get_nb_samples(packet: &[u8], len: i32, Fs: i32) -> i32;
pub fn opus_packet_get_mode(data: &[u8]) -> i32;
pub fn opus_packet_has_lbrr(packet: &[u8], len: i32) -> i32;
pub fn opus_packet_parse<'a>(packet: &'a [u8], len: i32, out_toc: &mut u8,
                             frames: &mut [&'a [u8]], size: &mut [i16], payload_offset: &mut i32) -> i32;
pub fn opus_pcm_soft_clip(pcm: &mut [f32], frame_size: i32, channels: i32, softclip_mem: &mut [f32]);
// plus the helper API used by the C probe: encode_size, parse_size,
// opus_packet_parse_impl, and the extension iterator/count/parse/generate.
```

`frames` must have room for 48 entries; `opus_packet_parse` fills the leading
entries and returns the number of frames (or a negative error).  `size` holds
the per-frame lengths.  The explicit `'a` lifetime is required to tie the frame
sub-slices to the packet.

## `src/opus/repacketizer.rs` (mirrors `src/repacketizer.c`) — **implemented and verified**

```rust
pub struct OpusRepacketizer { /* owns its frames: no raw pointers */ }
impl Default for OpusRepacketizer { .. }

impl OpusRepacketizer {
    pub fn get_size() -> i32;               // 1120, matches sizeof(OpusRepacketizer)
    pub fn init(&mut self) -> i32;          // the C version returns the pointer
    pub fn create() -> Box<OpusRepacketizer>;
    pub fn ctl(&mut self, request: i32, value: &mut i32) -> i32;  // C has no such call; returns OPUS_UNIMPLEMENTED
    pub fn cat(&mut self, data: &[u8], len: i32) -> i32;
    pub fn get_nb_frames(&self) -> i32;
    pub fn out_range(&mut self, begin: i32, end: i32, data: &mut [u8], maxlen: i32) -> i32;
    pub fn out(&mut self, data: &mut [u8], maxlen: i32) -> i32;
}

pub fn opus_packet_pad(data: &mut [u8], len: i32, new_len: i32) -> i32;
pub fn opus_packet_unpad(data: &mut [u8], len: i32) -> i32;
pub fn opus_multistream_packet_pad(data: &mut [u8], len: i32, new_len: i32, nb_streams: i32) -> i32;
pub fn opus_multistream_packet_unpad(data: &mut [u8], len: i32, nb_streams: i32) -> i32;
```

The repacketizer deliberately owns each frame in a `Vec<u8>` rather than
aliasing the caller's packet (which removes the in-place `OPUS_MOVE` overlap C
relies on); the produced bytes are identical.


## Conventions

* Negative return values are the C `OPUS_*` error codes; the public layer maps
  them through `ErrorCode::from`.
* Every function must accept the same *edge cases* as C (empty slices, `len`
  larger than the slice, negative frame sizes) and return the same code.  The
  public layer performs the same validation `audiopus` does, no more.
* No panics for any input reachable from the public API.

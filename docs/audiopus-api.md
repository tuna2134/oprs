# `audiopus` 0.3.0-rc.0 — Exhaustive Public API Inventory

Reference source (read-only):
`/home/m-kikuchi/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/audiopus-0.3.0-rc.0/`

Files covered completely by this inventory:

| File | Lines |
| --- | --- |
| `src/lib.rs` | 339 |
| `src/error.rs` | 146 |
| `src/coder.rs` | 27 |
| `src/coder/encoder.rs` | 853 |
| `src/coder/decoder.rs` | 273 |
| `src/packet.rs` | 178 |
| `src/repacketizer.rs` | 108 |
| `src/softclip.rs` | 53 |

> **Goal of this document.** Everything below is copied verbatim from the source
> where a signature is given. Anything stated as "gap", "bug" or "surprise" is an
> observation about the reference implementation, not a proposed change.

---

## 1. Crate metadata

### 1.1 `Cargo.toml` (normalised, as published)

```toml
[package]
edition = "2018"
name = "audiopus"
version = "0.3.0-rc.0"
authors = ["Lakelezz <lakelezz@protonmail.ch>"]
description = "High-level binding of the Opus Codec library."
documentation = "https://docs.rs/audiopus"
readme = "README.md"
keywords = ["audio", "opus", "codec"]
categories = ["api-bindings", "compression", "encoding", "multimedia::audio", "multimedia::encoding"]
license = "ISC"
repository = "https://github.com/lakelezz/audiopus.git"

[dependencies.audiopus_sys]
version = "0.2.2"

[dev-dependencies.matches]
version = "0.1.8"

[features]
coder = ["encoder", "decoder"]
decoder = ["packet"]
default_features = ["coder"]
encoder = ["packet"]
multistream = []
packet = []
repacketizer = ["packet"]
```

* **Edition:** `2018`.
* **Runtime dependency:** `audiopus_sys = "0.2.2"` (re-exported to users as `audiopus::ffi`).
* **Dev-dependency:** `matches = "0.1.8"`.
* There is **no `build.rs`** and no `[package] links` in audiopus itself.
* Rust version advertised in `README.md`: 1.51+.

### 1.2 Crate-level attributes (`src/lib.rs:33–39`)

```rust
#![deny(rust_2018_idioms)]
#![deny(clippy::all)]
#![deny(clippy::pedantic)]
#![deny(clippy::nursery)]
#![deny(clippy::cargo)]
// TODO: Document all public items.
// #![deny(missing_docs)]
```

`missing_docs` is **not** enabled (commented out) — hence several public items
carry no documentation (see §11).

### 1.3 Feature flags — and the critical finding

The `[features]` table declares `coder`, `decoder`, `encoder`, `packet`,
`repacketizer`, `multistream` and `default_features`.

**There is no `#[cfg(feature = ...)]` anywhere in the crate.**
An exhaustive search of `src/` finds only `#[cfg(test)]` (5 occurrences, one per
test module). Consequently:

* Every feature gates **exactly nothing**. All five public modules and all public
  items are compiled unconditionally, regardless of which features are enabled.
* Feature dependency edges declared in `[features]` (`coder → encoder, decoder`,
  `decoder → packet`, `encoder → packet`, `repacketizer → packet`) are inert with
  respect to the source.

**`default` feature set:** there is **no** `default = [...]` key. The key that is
present is spelled `default_features = ["coder"]`, which in Cargo defines an
ordinary feature *named* `default_features` (Cargo only special-cases the exact
name `default`). Therefore:

* The default feature set is **empty**.
* `features = ["default_features"]` would enable `coder` (and transitively
  `encoder`/`decoder`/`packet`), but nothing in the code changes.
* For a drop-in replacement, the feature **names** in the table above must keep
  resolving so downstream `audiopus = { features = [...] }` declarations do not
  fail; reproducing the gating behaviour means reproducing "no gating".

### 1.4 Module tree and re-exports

```rust
pub mod coder;
pub mod error;
pub mod packet;
pub mod repacketizer;
pub mod softclip;

pub use crate::error::{Error, ErrorCode, Result};
pub use audiopus_sys as ffi;
```

* `oprs::ffi` is a **public re-export of the entire `audiopus_sys` crate**.
* `coder`, `error`, `packet`, `repacketizer`, `softclip` are all `pub mod`.
* `coder` internally has `mod decoder;` and `mod encoder;` — **private** modules —
  and re-exports selected items (see §7).
* `use std::{convert::{TryFrom, TryInto}, ffi::CStr};` in `lib.rs` is a **private**
  `use`. `crate::TryFrom` / `crate::TryInto` therefore exist inside the crate
  (and are used by `packet.rs` via `use crate::{... TryFrom, TryInto}`), but are
  **not** public API. The only reachable `TryFrom` is `std::convert::TryFrom`.

---

## 2. Crate root items (`src/lib.rs`)

### 2.1 `Signal`

```rust
#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Signal {
    Auto = ffi::OPUS_AUTO,
    Voice = ffi::OPUS_SIGNAL_VOICE,
    Music = ffi::OPUS_SIGNAL_MUSIC,
}
```

Resolved discriminants: `Auto = -1000`, `Voice = 3001`, `Music = 3002`.

Conversion:

```rust
impl TryFrom<i32> for Signal {
    type Error = Error;

    fn try_from(value: i32) -> Result<Self> {
        Ok(match value {
            ffi::OPUS_AUTO => Signal::Auto,
            ffi::OPUS_SIGNAL_VOICE => Signal::Voice,
            ffi::OPUS_SIGNAL_MUSIC => Signal::Music,
            _ => return Err(Error::InvalidSignal(value)),
        })
    }
}
```

*No* `From<Signal> for i32`; the crate uses `signal as i32` (valid because of `#[repr(i32)]`).

### 2.2 `Bitrate`

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Bitrate {
    /// Explicit bitrate choice (in bits/second).
    BitsPerSecond(i32),
    /// Maximum bitrate allowed (up to maximum number of bytes for the packet).
    Max,
    /// Default bitrate decided by the encoder (not recommended).
    Auto,
}
```

No `#[repr(...)]` attribute; `Max`/`Auto` are not encoded by discriminant.

```rust
impl From<Bitrate> for i32 {
    fn from(bitrate: Bitrate) -> i32 {
        match bitrate {
            Bitrate::Auto => ffi::OPUS_AUTO,
            Bitrate::Max => ffi::OPUS_BITRATE_MAX,
            Bitrate::BitsPerSecond(bits) => bits,
        }
    }
}

impl TryFrom<i32> for Bitrate {
    type Error = Error;

    fn try_from(value: i32) -> Result<Bitrate> {
        Ok(match value {
            ffi::OPUS_AUTO => Bitrate::Auto,
            ffi::OPUS_BITRATE_MAX => Bitrate::Max,
            x if x.is_positive() => Bitrate::BitsPerSecond(x),
            _ => return Err(Error::InvalidBandwidth(value)),
        })
    }
}
```

Resolved: `OPUS_AUTO = -1000`, `OPUS_BITRATE_MAX = -1`.

> **Bug / surprise:** the failure arm returns `Error::InvalidBandwidth(value)`,
> not `Error::InvalidBitrate(value)`. `Error::InvalidBitrate` is never constructed
> anywhere in the crate. Values `<= 0` other than the two sentinels (including
> `0`) produce `Error::InvalidBandwidth`.

### 2.3 `SampleRate`

```rust
/// Represents possible sample rates Opus can use.
/// Values represent Hertz.
#[repr(i32)]
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub enum SampleRate {
    Hz8000 = 8000,
    Hz12000 = 12000,
    Hz16000 = 16000,
    Hz24000 = 24000,
    Hz48000 = 48000,
}

impl TryFrom<i32> for SampleRate {
    type Error = Error;

    /// Fails if a number does not map a documented Opus sample rate.
    fn try_from(value: i32) -> Result<Self> {
        Ok(match value {
            8000 => SampleRate::Hz8000,
            12000 => SampleRate::Hz12000,
            16000 => SampleRate::Hz16000,
            24000 => SampleRate::Hz24000,
            48000 => SampleRate::Hz48000,
            _ => return Err(Error::InvalidSampleRate(value)),
        })
    }
}
```

*Note the derive order differs from other enums (`Debug, Clone, Copy, ...`), and it
is the only enum whose `TryFrom` matches literal integers instead of `ffi` constants.*
No `From<SampleRate> for i32`; the crate uses `sample_rate as i32`.

### 2.4 `Application`

```rust
/// Represents possible application-types for Opus.
#[repr(i32)]
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub enum Application {
    /// Best for most VoIP/videoconference applications where listening quality
    /// and intelligibility matter most.
    Voip = ffi::OPUS_APPLICATION_VOIP,
    /// Best for broadcast/high-fidelity application where the decoded audio
    /// should be as close as possible to the input.
    Audio = ffi::OPUS_APPLICATION_AUDIO,
    /// Only use when lowest-achievable latency is what matters most.
    LowDelay = ffi::OPUS_APPLICATION_RESTRICTED_LOWDELAY,
}

impl TryFrom<i32> for Application {
    type Error = Error;

    /// Fails if a value does not match Opus' specified application-value.
    fn try_from(value: i32) -> Result<Self> {
        Ok(match value {
            ffi::OPUS_APPLICATION_VOIP => Application::Voip,
            ffi::OPUS_APPLICATION_AUDIO => Application::Audio,
            ffi::OPUS_APPLICATION_RESTRICTED_LOWDELAY => Application::LowDelay,
            _ => return Err(Error::InvalidApplication),
        })
    }
}
```

Resolved: `Voip = 2048`, `Audio = 2049`, `LowDelay = 2051`.
The enum itself has **no** doc comment (only its variants do).
No `From<Application> for i32`; crate uses `application as i32`.

### 2.5 `Channels`

```rust
/// Represents possible audio channels Opus can use.
#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Channels {
    /// Not supported when constructing encoders or decoders.
    Auto = ffi::OPUS_AUTO,
    Mono = 1,
    Stereo = 2,
}

impl Channels {
    pub fn is_mono(self) -> bool {
        if let Channels::Mono = self {
            return true;
        }

        false
    }

    pub fn is_stereo(self) -> bool {
        if let Channels::Stereo = self {
            return true;
        }

        false
    }
}

impl TryFrom<i32> for Channels {
    type Error = Error;

    // Fails if a value does not match Opus' specified channel-value.
    fn try_from(value: i32) -> Result<Channels> {
        Ok(match value {
            ffi::OPUS_AUTO => Channels::Auto,
            1 => Channels::Mono,
            2 => Channels::Stereo,
            _ => return Err(Error::InvalidChannels(value)),
        })
    }
}

impl From<Channels> for i32 {
    fn from(channels: Channels) -> i32 {
        channels as i32
    }
}
```

Resolved: `Auto = -1000`, `Mono = 1`, `Stereo = 2`.
`is_mono`/`is_stereo` take `self` **by value** and are undocumented.

### 2.6 `Bandwidth`

```rust
/// Represents possible bandwidths of an Opus-stream.
#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Bandwidth {
    /// Pick the bandwidth automatically.
    Auto = ffi::OPUS_AUTO,
    /// A 4kHz bandwidth.
    Narrowband = ffi::OPUS_BANDWIDTH_NARROWBAND,
    /// A 6kHz bandwidth.
    Mediumband = ffi::OPUS_BANDWIDTH_MEDIUMBAND,
    /// A 8kHz bandwidth.
    Wideband = ffi::OPUS_BANDWIDTH_WIDEBAND,
    /// A 12kHz bandwidth.
    Superwideband = ffi::OPUS_BANDWIDTH_SUPERWIDEBAND,
    /// A 20kHz bandwidth.
    Fullband = ffi::OPUS_BANDWIDTH_FULLBAND,
}

impl TryFrom<i32> for Bandwidth {
    type Error = Error;

    // Fails if a value does not match Opus' specified bandwidth-value.
    fn try_from(value: i32) -> Result<Self> {
        Ok(match value {
            ffi::OPUS_AUTO => Bandwidth::Auto,
            ffi::OPUS_BANDWIDTH_NARROWBAND => Bandwidth::Narrowband,
            ffi::OPUS_BANDWIDTH_MEDIUMBAND => Bandwidth::Mediumband,
            ffi::OPUS_BANDWIDTH_WIDEBAND => Bandwidth::Wideband,
            ffi::OPUS_BANDWIDTH_SUPERWIDEBAND => Bandwidth::Superwideband,
            ffi::OPUS_BANDWIDTH_FULLBAND => Bandwidth::Fullband,
            _ => return Err(Error::InvalidBandwidth(value)),
        })
    }
}
```

Resolved: `Auto = -1000`, `Narrowband = 1101`, `Mediumband = 1102`,
`Wideband = 1103`, `Superwideband = 1104`, `Fullband = 1105`.
No `From<Bandwidth> for i32`; crate uses `bandwidth as i32`.

> **Gap:** there is **no `BandwidthError` type** anywhere in the crate. The only
> bandwidth error is the `Error::InvalidBandwidth(i32)` variant of `Error`.

### 2.7 `MutSignals<'a, T>`

```rust
/// A newtype wrapping around a mutable buffer. They represent mutably borrowed
/// arguments that will be filled by Opus.
/// E.g. you pass this to an encode-method and Opus encodes data into the
/// underlying buffer.
///
/// **Info**:
/// This type is only verifying that Opus' requirement are not violated.
#[derive(Debug)]
pub struct MutSignals<'a, T>(&'a mut [T]);
```

The tuple field is **private** (no `pub`). Derives only `Debug` (no `Clone`).

```rust
impl<'a, T> TryFrom<&'a mut [T]> for MutSignals<'a, T> {
    type Error = Error;

    fn try_from(value: &'a mut [T]) -> Result<Self> {
        if value.len() > std::i32::MAX as usize {
            return Err(Error::SignalsTooLarge);
        }

        Ok(Self(value))
    }
}

impl<'a, T> TryFrom<&'a mut Vec<T>> for MutSignals<'a, T> {
    type Error = Error;

    fn try_from(value: &'a mut Vec<T>) -> Result<Self> {
        value.as_mut_slice().try_into()
    }
}

impl<'a, T> MutSignals<'a, T> {
    pub fn as_mut_ptr(&mut self) -> *mut T {
        self.0.as_mut_ptr()
    }

    /// Due to checking the length during construction of this newtype wrapping
    /// around a immutably borrowed buffer, we can safely cast `usize` to `i32`
    /// without worrying about `usize` being too large for `i32`.
    pub fn i32_len(&self) -> i32 {
        self.0.len() as i32
    }
}
```

Public methods:

```rust
pub fn as_mut_ptr(&mut self) -> *mut T
pub fn i32_len(&self) -> i32
```

`as_mut_ptr` is undocumented.

### 2.8 Crate-root free function

```rust
/// Gets the libopus version string.
///
/// Applications may look for the substring "-fixed" in the version string to
/// determine whether they have a fixed-point or floating-point build at runtime.
pub fn version() -> &'static str {
    // The pointer given from the `opus_get_version_string` function will be valid
    // therefore we can create a `CStr` from this pointer.
    unsafe { CStr::from_ptr(ffi::opus_get_version_string()) }
        .to_str()
        .unwrap()
}
```

Maps to libopus `opus_get_version_string()`. It **can panic** (`.unwrap()` on a
non-UTF-8 `&str` conversion), despite the crate-level claim to "never panic".

---

## 3. `error` module (`src/error.rs`)

### 3.1 `Result` alias

```rust
pub type Result<T> = std::result::Result<T, Error>;
```

This is the crate-wide `Result` and is re-exported at the root:
`audiopus::Result`, `audiopus::error::Result`.

### 3.2 `Error`

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Error {
    /// A value failed to match a documented [`Application`].
    ///
    /// [`Application`]: ../enum.Application.html
    InvalidApplication,
    /// A value failed to match a documented [`Bandwidth`].
    ///
    /// [`Bandwidth`]: ../enum.Application.html
    InvalidBandwidth(i32),
    /// A value failed to match a documented [`Bitrate`],
    /// negative values are invalid.
    ///
    /// [`Bitrate`]: ../enum.Bitrate.html
    InvalidBitrate(i32),
    /// A value failed to match a documented [`Signal`].
    ///
    /// [`Signal`]: ../enum.Signal.html
    InvalidSignal(i32),
    /// Complexity was lower than 1 or higher than 10.
    InvalidComplexity(i32),
    /// A value failed to match a documented [`SampleRate`].
    ///
    /// [`SampleRate`]: ../enum.SampleRate.html
    InvalidSampleRate(i32),
    /// A value failed to match a documented [`Channels`].
    ///
    /// [`Channels`]: ../enum.Channels.html
    InvalidChannels(i32),
    /// An error returned from Opus containing an [`ErrorCode`] describing
    /// the cause.
    Opus(ErrorCode),
    /// Opus is not operating with empty packets.
    EmptyPacket,
    /// Opus' maximum `Vec` or slice length of `std::i32::MAX` has been
    /// exceeded.
    SignalsTooLarge,
    /// Opus' maximum `Vec` or slice length of `std::i32::MAX` has been
    /// exceeded.
    PacketTooLarge,
    /// A `Vec` representing a mapping exceeded the expected value.
    MappingExpectedLen(usize),
}
```

All 12 variants, in declaration order, with payload types. `Error` is `Copy`.
There is **no `Error::new`** associated function/constructor anywhere; the crate
constructs `Error` values directly as enum variants, and converts from
`ErrorCode` via `From` (see below).

Trait impls:

```rust
impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Error::Opus(err) => Some(err),
            _ => None,
        }
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            Error::InvalidApplication => f.write_str("Invalid Application"),
            Error::InvalidBandwidth(bandwidth) => write!(f, "Invalid Bandwitdh: {}", bandwidth),
            Error::InvalidSignal(signal) => write!(f, "Invalid Signal: {}", signal),
            Error::InvalidComplexity(complexity) => write!(f, "Invalid Complexity: {}", complexity),
            Error::InvalidSampleRate(rate) => write!(f, "Invalid Sample Rate: {}", rate),
            Error::InvalidChannels(channels) => write!(f, "Invalid Channels: {}", channels),
            Error::Opus(error_code) => write!(f, "{}", error_code),
            Error::EmptyPacket => f.write_str("Passed packet contained no elements"),
            Error::SignalsTooLarge => f.write_str("Signals' length exceeded `i32::MAX`"),
            Error::PacketTooLarge => f.write_str("Packet's length exceeded `i32::MAX`"),
            Error::InvalidBitrate(rate) => write!(f, "Invalid Bitrate: {}", rate),
            Error::MappingExpectedLen(len) => write!(f, "Wrong channel length, expected: {}", len),
        }
    }
}

impl From<ErrorCode> for Error {
    fn from(error_code: ErrorCode) -> Error {
        Error::Opus(error_code)
    }
}
```

Exact `Display` strings (note the misspelling `"Bandwitdh"`):

| Variant | Display output |
| --- | --- |
| `InvalidApplication` | `Invalid Application` |
| `InvalidBandwidth(b)` | `Invalid Bandwitdh: {b}` |
| `InvalidSignal(s)` | `Invalid Signal: {s}` |
| `InvalidComplexity(c)` | `Invalid Complexity: {c}` |
| `InvalidSampleRate(r)` | `Invalid Sample Rate: {r}` |
| `InvalidChannels(c)` | `Invalid Channels: {c}` |
| `Opus(e)` | `{e}` (delegates to `ErrorCode`'s `Display`) |
| `EmptyPacket` | `Passed packet contained no elements` |
| `SignalsTooLarge` | `Signals' length exceeded `i32::MAX`` |
| `PacketTooLarge` | `Packet's length exceeded `i32::MAX`` |
| `InvalidBitrate(r)` | `Invalid Bitrate: {r}` |
| `MappingExpectedLen(l)` | `Wrong channel length, expected: {l}` |

### 3.3 `ErrorCode`

```rust
#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum ErrorCode {
    BadArgument = ffi::OPUS_BAD_ARG,
    BufferTooSmall = ffi::OPUS_BUFFER_TOO_SMALL,
    InternalError = ffi::OPUS_INTERNAL_ERROR,
    InvalidPacket = ffi::OPUS_INVALID_PACKET,
    Unimplemented = ffi::OPUS_UNIMPLEMENTED,
    InvalidState = ffi::OPUS_INVALID_STATE,
    AllocFail = ffi::OPUS_ALLOC_FAIL,
    /// Occurs when Opus sends an error value that is not documented.
    /// `0` is unrelated to Opus and just a mere marker by this crate to
    /// differentiate between Opus' errors (all of them are negative).
    Unknown = 0,
}
```

Resolved discriminants: `BadArgument = -1`, `BufferTooSmall = -2`,
`InternalError = -3`, `InvalidPacket = -4`, `Unimplemented = -5`,
`InvalidState = -6`, `AllocFail = -7`, `Unknown = 0`.

`Unknown` is a **crate-local sentinel**; it does not exist in libopus.

```rust
impl Display for ErrorCode {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        let s = match self {
            ErrorCode::BadArgument => "Passed argument violated Opus' specified requirements",
            ErrorCode::BufferTooSmall => "Passed buffer was too small",
            ErrorCode::InternalError => "Internal error inside Opus occured",
            ErrorCode::InvalidPacket => "Opus received a packet violating requirements",
            ErrorCode::Unimplemented => "Unimplemented code branch was attempted to be executed",
            ErrorCode::InvalidState => "Opus-type instance is in an invalid state",
            ErrorCode::AllocFail => "Opus was unable to allocate memory",
            ErrorCode::Unknown => {
                "Opus returned a non-negative error, this might be a Audiopus or Opus bug"
            }
        };

        write!(f, "{}", s)
    }
}

impl StdError for ErrorCode {}

impl From<i32> for ErrorCode {
    fn from(number: i32) -> ErrorCode {
        match number {
            ffi::OPUS_BAD_ARG => ErrorCode::BadArgument,
            ffi::OPUS_BUFFER_TOO_SMALL => ErrorCode::BufferTooSmall,
            ffi::OPUS_INTERNAL_ERROR => ErrorCode::InternalError,
            ffi::OPUS_INVALID_PACKET => ErrorCode::InvalidPacket,
            ffi::OPUS_UNIMPLEMENTED => ErrorCode::Unimplemented,
            ffi::OPUS_INVALID_STATE => ErrorCode::InvalidState,
            ffi::OPUS_ALLOC_FAIL => ErrorCode::AllocFail,
            _ => ErrorCode::Unknown,
        }
    }
}
```

`StdError for ErrorCode` is an **empty** impl (no `source`, no `description`).

### 3.4 `try_map_opus_error` (public, `error` module only)

```rust
/// Checks if the `ffi_return_value` is documented by Opus.
/// Returns `Error` if value is negative.
pub fn try_map_opus_error(ffi_return_value: i32) -> Result<i32> {
    match ffi_return_value {
        v if v < 0 => Err(Error::from(ErrorCode::from(v))),
        _ => Ok(ffi_return_value),
    }
}
```

* Path: `audiopus::error::try_map_opus_error` — **not** re-exported at the crate root.
* This is the crate's canonical "negative return value ⇒ `Error::Opus`" mapper.

---

## 4. `packet` module (`src/packet.rs`)

### 4.1 private helper

```rust
fn packet_len_check(packet_buffer: &[u8]) -> Result<i32> {
    match packet_buffer {
        // non-empty guarantee:
        x if x.is_empty() => Err(Error::EmptyPacket),
        // limited size guarantee:
        _ if packet_buffer.len() > std::i32::MAX as usize => Err(Error::PacketTooLarge),
        _ => Ok(packet_buffer.len() as i32),
    }
}
```

Not public. Used by all four `TryFrom` impls of `Packet`/`MutPacket` and by
`MutPacket::i32_len`, so the invariants are re-checked on every `i32_len` call
for `MutPacket`.

### 4.2 `Packet<'a>`

```rust
/// A newtype around `&[u8]` to guarantee:
/// - Minimum one element: A packet cannot be empty.
/// - Limited size: A packet's length may not exceed `std::i32::MAX`.
#[derive(Debug)]
pub struct Packet<'a>(&'a [u8]);
```

Private tuple field; only `Debug` is derived.

```rust
impl<'a> Packet<'a> {
    pub fn as_ptr(&self) -> *const u8 {
        self.0.as_ptr()
    }

    /// The underlying type is immutably borrowed and has been verified upon
    /// construction of `Packet`, thus we know casting `usize` will fit
    /// inside `i32`.
    pub fn i32_len(&self) -> i32 {
        self.0.len() as i32
    }
}

impl<'a> TryFrom<&'a Vec<u8>> for Packet<'a> {
    type Error = Error;

    fn try_from(value: &'a Vec<u8>) -> Result<Self> {
        value.as_slice().try_into()
    }
}

impl<'a> TryFrom<&'a [u8]> for Packet<'a> {
    type Error = Error;

    fn try_from(value: &'a [u8]) -> Result<Packet<'a>> {
        packet_len_check(value).map(|_| Self(value))
    }
}
```

Public methods: `as_ptr(&self) -> *const u8` (undocumented), `i32_len(&self) -> i32`.

### 4.3 `MutPacket<'a>`

```rust
/// A newtype around `&mut [u8]` to guarantee that accessing length on the
/// underlying buffer is checked each time.
#[derive(Debug)]
pub struct MutPacket<'a>(&'a mut [u8]);
```

Private tuple field; only `Debug` is derived.

```rust
impl<'a> MutPacket<'a> {
    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        self.0.as_mut_ptr()
    }

    /// Checks if the underlying buffer meets requirements.
    pub fn i32_len(&self) -> Result<i32> {
        packet_len_check(&self.0)
    }
}

impl<'a> TryFrom<&'a mut Vec<u8>> for MutPacket<'a> {
    type Error = Error;

    fn try_from(value: &'a mut Vec<u8>) -> Result<Self> {
        value.as_mut_slice().try_into()
    }
}

impl<'a> TryFrom<&'a mut [u8]> for MutPacket<'a> {
    type Error = Error;

    fn try_from(value: &'a mut [u8]) -> Result<Self> {
        packet_len_check(value).map(move |_| Self(value))
    }
}
```

Note the asymmetry with `Packet`: `MutPacket::i32_len` returns `Result<i32>`,
`Packet::i32_len` returns `i32`. `as_mut_ptr` takes `&mut self` and is undocumented.

### 4.4 `packet` free functions

Exact signatures and the libopus call each wraps:

```rust
/// Gets bandwidth of an Opus `packet`.
///
/// **Errors**:
/// Empty `packet` will return `Error::EmptyPacket`.
pub fn bandwidth(packet: Packet<'_>) -> Result<Bandwidth>
```
→ `opus_packet_get_bandwidth(const unsigned char *data)`; return value converted
with `TryFrom<i32> for Bandwidth`.

```rust
/// Gets number of samples per frame of an Opus `packet`.
///
/// **Errors**:
/// Empty `packet` will return `Error::EmptyPacket`.
pub fn samples_per_frame(packet: Packet<'_>, sample_rate: SampleRate) -> Result<usize>
```
→ `opus_packet_get_samples_per_frame(const unsigned char *data, opus_int32 Fs)`.
**Never returns `Err`** in practice: the raw `i32` result is cast `as usize` and
wrapped in `Ok(...)` unconditionally.

```rust
/// Gets number of samples in an Opus `packet`.
///
/// **Errors**:
/// Empty `packet` will return `Error::EmptyPacket`.
pub fn nb_samples(packet: Packet<'_>, sample_rate: SampleRate) -> Result<usize>
```
→ `opus_packet_get_nb_samples(const unsigned char *packet, opus_int32 len, opus_int32 Fs)`;
negative result mapped through `try_map_opus_error`, then `as usize`.

```rust
/// Gets number of channels in an Opus `packet`.
///
/// **Errors**:
/// Empty `packet` will return `Error::EmptyPacket`.
pub fn nb_channels(packet: Packet<'_>) -> Result<Channels>
```
→ `opus_packet_get_nb_channels(const unsigned char *data)`; result fed to
`Channels::try_from(i32)` (so an unexpected value also yields an error).

```rust
/// Gets number of frames in an Opus `packet`.
///
/// **Errors**:
/// Empty `packet` will return [`Error::EmptyPacket`].
pub fn nb_frames(packet: Packet<'_>) -> Result<usize>
```
→ `opus_packet_get_nb_frames(const unsigned char *packet, opus_int32 len)`;
negative result mapped through `try_map_opus_error`, then `as usize`.

The doc-comment claim "Empty `packet` will return `Error::EmptyPacket`" is only
reachable **at construction time** (`Packet::try_from`), since a constructed
`Packet` is non-empty by invariant.

---

## 5. `repacketizer` module (`src/repacketizer.rs`)

Imports: `error::try_map_opus_error`, `ffi`, `packet::{MutPacket, Packet}`, `Result`.

### 5.1 Free functions

```rust
/// Returns Opus' internal `OpusRepacketizer`'s size in bytes.
pub fn repacketizer_size() -> usize {
    unsafe { ffi::opus_repacketizer_get_size() as usize }
}
```
→ `opus_repacketizer_get_size(void)`.

```rust
pub fn multistream_packet_pad(
    mut data: MutPacket<'_>,
    new_len: usize,
    nb_streams: usize,
) -> Result<()>
```
→ `opus_multistream_packet_pad(unsigned char *data, opus_int32 len, opus_int32 new_len, int nb_streams)`;
negative result mapped via `try_map_opus_error`, then `.map(|_| ())`.
`new_len`/`nb_streams` are `usize` and cast `as i32` internally (no range check).

```rust
pub fn multistream_packet_unpad(mut data: MutPacket<'_>, nb_streams: usize) -> Result<()>
```
→ `opus_multistream_packet_unpad(unsigned char *data, opus_int32 len, int nb_streams)`;
negative result mapped, then `.map(|_| ())`.

```rust
pub fn packet_pad(mut data: MutPacket<'_>, new_len: i32) -> Result<()>
```
→ `opus_packet_pad(unsigned char *data, opus_int32 len, opus_int32 new_len)`;
negative result mapped, then `.map(|_| ())`.

```rust
pub fn packet_unpad(mut data: MutPacket<'_>) -> Result<()>
```
→ `opus_packet_unpad(unsigned char *data, opus_int32 len)`;
negative result mapped, then `.map(|_| ())`.

All four are **undocumented**. Note the `mut data` receiver parameter in the
signature (taken by value; the `mut` is part of the parameter binding, not the
public type). Padding cannot change the caller-visible length, since `MutPacket`
borrows a fixed-length slice; the new length is only known to Opus.

### 5.2 `Repacketizer`

```rust
#[derive(Debug)]
pub struct Repacketizer {
    pointer: *mut ffi::OpusRepacketizer,
}
```

Field is **private**. Only `Debug` is derived. No `Send`/`Sync` impl is provided
(the raw pointer makes it `!Send + !Sync` by default).

```rust
impl Default for Repacketizer {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Repacketizer {
    /// We have to ensure that the resource our wrapping Opus-struct is pointing
    /// to is deallocated properly.
    fn drop(&mut self) {
        unsafe { ffi::opus_repacketizer_destroy(self.pointer) }
    }
}

impl Repacketizer {
    pub fn new() -> Self {
        let pointer = unsafe { ffi::opus_repacketizer_create() };

        Self { pointer }
    }

    pub fn nb_frames(&self) -> usize {
        unsafe { ffi::opus_repacketizer_get_nb_frames(self.pointer) as usize }
    }

    pub fn repacketizer_out(&self, mut data_out: MutPacket<'_>, max_len: i32) -> Result<()> {
        try_map_opus_error(unsafe {
            ffi::opus_repacketizer_out(self.pointer, data_out.as_mut_ptr(), max_len)
        })
        .map(|_| ())
    }

    pub fn repacketizer_out_range(
        &self,
        begin: i32,
        end: i32,
        mut data_out: MutPacket<'_>,
        max_len: i32,
    ) -> Result<()> {
        try_map_opus_error(unsafe {
            ffi::opus_repacketizer_out_range(
                self.pointer,
                begin,
                end,
                data_out.as_mut_ptr(),
                max_len,
            )
        })
        .map(|_| ())
    }

    pub fn repacketizer_cat(&self, data: Packet<'_>) -> Result<()> {
        try_map_opus_error(unsafe {
            ffi::opus_repacketizer_cat(self.pointer, data.as_ptr(), data.i32_len())
        })
        .map(|_| ())
    }
}
```

Mapping to libopus calls:

| Method | libopus call |
| --- | --- |
| `Repacketizer::new` | `opus_repacketizer_create(void)` |
| `Repacketizer::nb_frames` | `opus_repacketizer_get_nb_frames(OpusRepacketizer *rp)` |
| `Repacketizer::repacketizer_out` | `opus_repacketizer_out(OpusRepacketizer *rp, unsigned char *data, opus_int32 maxlen)` |
| `Repacketizer::repacketizer_out_range` | `opus_repacketizer_out_range(OpusRepacketizer *rp, int begin, int end, unsigned char *data, opus_int32 maxlen)` |
| `Repacketizer::repacketizer_cat` | `opus_repacketizer_cat(OpusRepacketizer *rp, const unsigned char *data, opus_int32 len)` |
| `Drop for Repacketizer` | `opus_repacketizer_destroy(OpusRepacketizer *rp)` |

`Repacketizer::new` does **not** check the returned pointer for null and does
**not** observe any error code (`opus_repacketizer_create` returns a pointer only).
Note that `repacketizer_out`/`repacketizer_out_range`'s `max_len` is passed to
Opus verbatim and is **not** cross-checked against `data_out`'s actual length.

---

## 6. `softclip` module (`src/softclip.rs`)

```rust
#[derive(Clone, Debug)]
pub struct SoftClip {
    channels: Channels,
    memory: [f32; 2],
}

impl SoftClip {
    pub fn new(channels: Channels) -> Self {
        Self {
            channels,
            memory: [0.0; 2],
        }
    }

    /// Opus applies soft-clipping to bring a f32 signal within the
    /// [-1,1] range.
    pub fn apply(&mut self, mut signals: MutSignals<'_, f32>) -> Result<()> {
        unsafe {
            ffi::opus_pcm_soft_clip(
                signals.as_mut_ptr(),
                signals.i32_len() / self.channels as i32,
                self.channels as i32,
                self.memory.as_mut_ptr(),
            )
        };

        Ok(())
    }
}
```

* Both fields are **private**. Derives `Clone, Debug` (cloning copies the
  per-channel clipping `memory` state as well).
* `SoftClip` itself is **undocumented**; `new` is **undocumented**.
* Maps to `opus_pcm_soft_clip(float *pcm, int frame_size, int channels, float *softclip_mem)`.
* **The FFI return value is discarded** and `apply` unconditionally returns
  `Ok(())` — the `Result` is effectively always `Ok`. (`opus_pcm_soft_clip`
  returns `void` in audiopus_sys 0.2.2, hence the `;` after the block.)
* No validation of `signals` length, of the channel count, or that
  `signals.len()` is a multiple of the channel count. `Channels::Auto`
  (`-1000`) would make `self.channels as i32` negative.
* `memory` is fixed at 2 `f32`s, so only mono/stereo operation is meaningful.

---

## 7. `coder` module (`src/coder.rs`, `src/coder/encoder.rs`, `src/coder/decoder.rs`)

### 7.1 Module exports

Entire content of `src/coder.rs`:

```rust
use crate::{Error, SampleRate};

pub use self::{
    decoder::{size, Decoder},
    encoder::Encoder,
};

mod decoder;
mod encoder;

/// A set of methods that both `Encoder` and `Decoder` have implemented.
///
/// **Info**:
/// This does not include `set_sample_rate` as it returns unimplemented on
/// [`Decoder`].
///
/// [`Decoder`]: decoder/struct.Decoder.html
pub trait GenericCtl {
    fn final_range(&self) -> Result<u32, Error>;

    fn phase_inversion_disabled(&self) -> Result<bool, Error>;
    fn set_phase_inversion_disabled(&mut self, disabled: bool) -> Result<(), Error>;

    fn sample_rate(&self) -> Result<SampleRate, Error>;

    fn reset_state(&mut self) -> Result<(), Error>;
}
```

Public paths exposed:

* `audiopus::coder::Encoder` (re-export of private `coder::encoder::Encoder`)
* `audiopus::coder::Decoder` (re-export of private `coder::decoder::Decoder`)
* `audiopus::coder::size` (re-export of private `coder::decoder::size`)
* `audiopus::coder::GenericCtl`

The modules `coder::encoder` and `coder::decoder` themselves are private
(`mod`, not `pub mod`), so e.g. `audiopus::coder::encoder::Encoder` is **not**
a valid external path.

`GenericCtl` is **not** re-exported at the crate root. Its methods are declared
with fully-written `Result<T, Error>` (the trait does **not** use the crate's
`Result<T>` alias, though it is the same type).

Both `Encoder` and `Decoder` implement `GenericCtl`.

### 7.2 `Encoder`

```rust
/// `Encoder` calls to Opus and offers method to encode and issue
/// requests to Opus.
#[derive(Debug)]
pub struct Encoder {
    pointer: *mut ffi::OpusEncoder,
    channels: Channels,
}

/// The Opus encoder can be sent between threads unless the Opus library
/// has been compiled with `NONTHREADSAFE_PSEUDOSTACK` to disallow encoding in
/// parallel.
unsafe impl Send for Encoder {}
```

Both fields are **private**. `Debug` prints the raw pointer address.

#### `impl GenericCtl for Encoder`

```rust
impl GenericCtl for Encoder {
    /// Gets the final state of the codec's entropy coder.
    ///
    /// This is used for testing purposes. The encoder state should
    /// be identical after coding a payload, assuming no data corruption or
    /// software bugs.
    fn final_range(&self) -> Result<u32> {
        self.encoder_ctl_request(ffi::OPUS_GET_FINAL_RANGE_REQUEST)
            .map(|v| v as u32)
    }

    /// Gets the encoder's configured phase inversion status.
    fn phase_inversion_disabled(&self) -> Result<bool> {
        self.encoder_ctl_request(ffi::OPUS_SET_PHASE_INVERSION_DISABLED_REQUEST)
            .map(|b| b == 1)
    }

    /// If set to `true`, disables the use of phase inversion for intensity
    /// stereo, improving the quality of mono downmixes, but slightly reducing
    /// normal stereo quality.
    ///
    /// Disabling phase inversion in the decoder does not comply with RFC 6716,
    /// although it does not cause any interoperability issue and is expected
    /// to become part of the Opus standard once RFC 6716 is updated by
    /// draft-ietf-codec-opus-update.
    fn set_phase_inversion_disabled(&mut self, disabled: bool) -> Result<()> {
        let disable_phase_inversion = if disabled { 1 } else { 0 };
        self.set_encoder_ctl_request(
            ffi::OPUS_SET_PHASE_INVERSION_DISABLED_REQUEST,
            disable_phase_inversion,
        )
        .map(|_| ())
    }

    /// Gets the sampling rate the encoder or decoder was initialized with.
    ///
    /// This simply returns the Fs value passed to [`Encoder::new`].
    ///
    /// [`Encoder::new`]: struct.Encoder.html#method.new
    fn sample_rate(&self) -> Result<SampleRate> {
        self.encoder_ctl_request(ffi::OPUS_GET_SAMPLE_RATE_REQUEST)
            .and_then(SampleRate::try_from)
    }

    /// Resets the codec state to be equivalent to a freshly initialized state.
    ///
    /// This should be called when switching streams in order to prevent the
    /// back to back decoding from giving different results from one at a
    /// time decoding.
    fn reset_state(&mut self) -> Result<()> {
        self.encoder_ctl_request(ffi::OPUS_RESET_STATE).map(|_| ())
    }
}
```

> **Bug / surprise:** `Encoder::phase_inversion_disabled` (a *getter*) issues
> `ffi::OPUS_SET_PHASE_INVERSION_DISABLED_REQUEST` through the *get* path, not
> `OPUS_GET_PHASE_INVERSION_DISABLED_REQUEST`. The `Decoder` version correctly
> uses `OPUS_GET_...`.

#### `impl Encoder` (inherent)

Every signature verbatim, in source order:

```rust
/// Creates a new Opus encoder.
///
/// **Warning**:
/// If `channels` is set to [`Channels::Auto`] the function will
/// return [`BadArgument`].
///
/// [`Channels::Auto`]: ../enum.Channels.html#variant.Auto
/// [`BadArgument`]: ../error/enum.ErrorCode.html#variant.BadArgument
pub fn new(sample_rate: SampleRate, channels: Channels, mode: Application) -> Result<Encoder>
```
→ `opus_encoder_create(opus_int32 Fs, int channels, int application, int *error)`.
Success test is `if opus_code == ffi::OPUS_OK || !pointer.is_null()` — note the
**`||`**: a non-null pointer is accepted even when `opus_code != OPUS_OK`. On
failure: `Err(ErrorCode::from(opus_code).into())` i.e. `Error::Opus(...)`.

```rust
/// Issues a CTL get-`request` to Opus.
/// If Opus returns a negative value it indicates an error.
///
/// **Info**:
/// As [`Encoder`]'s methods cover all possible CTLs, it is recommended
/// to use them instead.
///
/// [`Encoder`]: struct.Encoder.html
pub fn encoder_ctl_request(&self, request: i32) -> Result<i32>
```
→ `opus_encoder_ctl(OpusEncoder *st, int request, ...)` with an `&mut i32` out-arg.
This is the **only** public raw-CTL entry point among the two coders.

```rust
/// Issues a CTL set-`request` to Opus and sets the `Encoder`'s setting to
/// `value` based on sent `request`.
/// If Opus returns a negative value it indicates an error.
///
/// **Info**:
/// As [`Encoder`]'s methods cover all possible CTLs, it is recommended
/// to use them instead.
///
/// [`Encoder`]: struct.Encoder.html
pub fn set_encoder_ctl_request(&mut self, request: i32, value: i32) -> Result<()>
```
→ `opus_encoder_ctl(OpusEncoder *st, int request, value)`; negative mapped, else `Ok(())`.

```rust
/// Encodes an Opus frame.
///
/// The `input` signal (interleaved if 2 channels) will be encoded into the
/// `output` payload and on success returns the length of the
/// encoded packet.
pub fn encode(&self, input: &[i16], output: &mut [u8]) -> Result<usize>
```
→ `opus_encode(OpusEncoder *st, const opus_int16 *pcm, int frame_size, unsigned char *data, opus_int32 max_data_bytes)`.
`frame_size` is computed as `input.len() as i32 / self.channels as i32`;
`max_data_bytes` is `output.len() as i32`. **No length validation** (both casts can
truncate/overflow for lengths > `i32::MAX`).

```rust
/// Encodes an Opus frame from floating point input.
///
/// The `input` signal (interleaved if 2 channels) will be encoded into the
/// `output` payload and on success, returns the length of the
/// encoded packet.
pub fn encode_float(&self, input: &[f32], output: &mut [u8]) -> Result<usize>
```
→ `opus_encode_float(OpusEncoder *st, const float *pcm, int frame_size, unsigned char *data, opus_int32 max_data_bytes)`.
Same `input.len() as i32 / channels as i32` logic; no validation.

```rust
/// Gets the encoder's complexity configuration.
pub fn complexity(&self) -> Result<u8>
```
→ CTL `OPUS_GET_COMPLEXITY_REQUEST`, `v as u8`.

```rust
/// Configures the encoder's computational complexity.
///
/// **Warning**:
/// If `complexity` exceeds 10, [`BadArgument`] will be returned.
///
/// [`BadArgument`]: ../error/enum.ErrorCode.html#variant.BadArgument
pub fn set_complexity(&mut self, complexity: u8) -> Result<()>
```
→ CTL `OPUS_SET_COMPLEXITY_REQUEST` with `i32::from(complexity)`.
(The `Error::InvalidComplexity` variant is not used; Opus returns `BadArgument`.)

```rust
/// Gets the encoder's configured application.
pub fn application(&self) -> Result<Application>
```
→ CTL `OPUS_GET_APPLICATION_REQUEST`, then `Application::try_from`.

```rust
/// Configures the encoder's intended application.
///
/// The initial value is a mandatory argument in the [`new`]-function.
///
/// [`new`]: struct.Encoder.html#method.new
pub fn set_application(&mut self, application: Application) -> Result<()>
```
→ CTL `OPUS_SET_APPLICATION_REQUEST` with `application as i32`.

```rust
/// Configures the bitrate in the encoder.
///
/// Rates from 500 to 512000 bits per second are meaningful,
/// as well as the special values [`Bitrate::Auto`] and [`Bitrate::Max`].
/// [`Bitrate::Max`] can be used to cause the codec to use
/// as much rate as it can, which is useful for controlling the rate by
/// adjusting the output buffer size.
pub fn set_bitrate(&mut self, bitrate: Bitrate) -> Result<()>
```
→ CTL `OPUS_SET_BITRATE_REQUEST` with `bitrate.into()` (i.e. `i32::from(bitrate)`).

```rust
/// Gets the encoder's configured bandpass.
pub fn bitrate(&self) -> Result<Bitrate>
```
→ CTL `OPUS_GET_BITRATE_REQUEST`, then `Bitrate::try_from`.
(Doc-comment is copy-paste wrong: it says "bandpass".)

```rust
/// Enables variable bitrate (VBR) in the encoder.
///
/// The configured bitrate may not be met exactly because frames must be an
/// integer number of bytes in length.
///
/// **Warning**:
/// Only the MDCT mode of Opus currently heeds the constraint.
/// Speech mode ignores it completely,
/// hybrid mode may fail to obey it if the LPC layer uses more bitrate
/// than the constraint would have permitted.
pub fn enable_vbr_constraint(&mut self) -> Result<()>
```
Calls `self.set_vbr_constraint(true)`.

```rust
/// Disables variable bitrate (VBR) in the encoder.
/// ... (same warning text)
pub fn disable_vbr_constraint(&mut self) -> Result<()>
```
Calls `self.set_vbr_constraint(false)`.

```rust
/// Sets variable bitrate (VBR) in the encoder.
/// ... (same warning text)
pub fn set_vbr_constraint(&mut self, enable: bool) -> Result<()>
```
→ CTL `OPUS_SET_VBR_CONSTRAINT_REQUEST` with `if enable { 1 } else { 0 }`.

```rust
/// Determine if constrained VBR is enabled in the encoder.
pub fn vbr_constraint(&self) -> Result<bool>
```
→ CTL `OPUS_GET_VBR_CONSTRAINT_REQUEST`, `b == 1`.

```rust
/// Enables variable bitrate (VBR) in the encoder.
///
/// The configured bitrate may not be met exactly because frames must be an
/// integer number of bytes in length.
pub fn enable_vbr(&mut self) -> Result<()>
```
Calls `self.set_vbr(true)`.

```rust
/// Disables variable bitrate (VBR) in the encoder.
///
/// The configured bitrate may not be met exactly because frames must be an
/// integer number of bytes in length.
pub fn disable_vbr(&mut self) -> Result<()>
```
Calls `self.set_vbr(false)`.

```rust
/// Sets variable bitrate (VBR) in the encoder.
///
/// The configured bitrate may not be met exactly because frames must be an
/// integer number of bytes in length.
pub fn set_vbr(&mut self, enable: bool) -> Result<()>
```
→ CTL `OPUS_SET_VBR_REQUEST` with `if enable { 1 } else { 0 }`.

```rust
/// Determine if variable bitrate (VBR) is enabled in the encoder.
pub fn vbr(&self) -> Result<bool>
```
→ CTL `OPUS_GET_VBR_REQUEST`, `b == 1`.

```rust
/// Configures the encoder's use of inband forward error correction (FEC).
pub fn set_inband_fec(&mut self, enable: bool) -> Result<()>
```
→ CTL `OPUS_SET_INBAND_FEC_REQUEST`.

```rust
/// Enables the encoder's use of inband forward error correction (FEC).
pub fn enable_inband_fec(&mut self) -> Result<()>
```
Calls `self.set_inband_fec(true)`.

```rust
/// Disables the encoder's use of inband forward error correction (FEC).
pub fn disable_inband_fec(&mut self) -> Result<()>
```
Calls `self.set_inband_fec(false)`.

```rust
/// Gets encoder's configured use of inband forward error correction.
pub fn inband_fec(&self) -> Result<bool>
```
→ CTL `OPUS_GET_INBAND_FEC_REQUEST`, `n == 1`.

```rust
/// Gets the encoder's configured packet loss percentage.
pub fn packet_loss_perc(&self) -> Result<u8>
```
→ CTL `OPUS_GET_PACKET_LOSS_PERC_REQUEST`, `n as u8`.

```rust
/// Higher values trigger progressively more loss resistant behavior in the
/// encoder at the expense of quality at a given bitrate in the absence of
/// packet loss, but greater quality under loss.
///
/// Configures the encoder's expected packet loss percentage.
pub fn set_packet_loss_perc(&mut self, percentage: u8) -> Result<()>
```
→ CTL `OPUS_SET_PACKET_LOSS_PERC_REQUEST` with `i32::from(percentage)`.

```rust
/// Gets the total samples of delay added by the entire codec.
///
/// This can be queried by the encoder and then the provided number of
/// samples can be skipped on from the start of the decoder's output to
/// provide time aligned input and output. From the perspective of a
/// decoding application the real data begins this many samples late.
///
/// The decoder contribution to this delay is identical for all decoders,
/// but the encoder portion of the delay may vary from implementation to
/// implementation, version to version, or even depend on the encoder's
/// initial configuration.
/// Applications needing delay compensation should call this method
/// rather than hard-coding a value.
pub fn lookahead(&self) -> Result<u32>
```
→ CTL `OPUS_GET_LOOKAHEAD_REQUEST`, `n as u32`.

```rust
/// Configures mono/stereo forcing in the encoder.
///
/// This can force the encoder to produce packets encoded as either
/// mono or stereo, regardless of the format of the input audio.
/// This is useful when the caller knows that the input signal is
/// currently a mono source embedded in a stereo stream.
pub fn set_force_channels(&mut self, channels: Channels) -> Result<()>
```
→ CTL `OPUS_SET_FORCE_CHANNELS_REQUEST` with `channels as i32`.

```rust
/// Gets the encoder's forced channel configuration.
pub fn force_channels(&self) -> Result<Channels>
```
→ CTL `OPUS_GET_FORCE_CHANNELS_REQUEST`, then `Channels::try_from`.

```rust
/// Gets the encoder's configured maximum allowed bandpass.
pub fn max_bandwidth(&self) -> Result<Bandwidth>
```
→ CTL `OPUS_GET_MAX_BANDWIDTH_REQUEST`, then `Bandwidth::try_from`.

```rust
/// Configures the maximum bandpass that the encoder will select automatically.
///
/// Applications should normally use this instead of [`set_bandwidth`]
/// (leaving that set to the default, [`Bandwidth::Auto`]).
///
/// This allows the application to set an upper bound based on the type of
/// input it is providing, but still gives the encoder the freedom to reduce
/// the bandpass when the bitrate becomes too low,
/// for better overall quality.
///
/// **Warning**:
/// [`Bandwidth::Auto`] will return [`BadArgument`] as it is not
/// accepted by Opus as `bandwidth` value.
///
/// [`set_bandwidth`]: struct.Encoder.html#method.set_bandwidth.html
/// [`Bandwidth::Auto`]: ../enum.Bandwidth.html#variant.Auto
/// [`BadArgument`]: ../error/enum.ErrorCode.html#variant.BadArgument.html
pub fn set_max_bandwidth(&mut self, bandwidth: Bandwidth) -> Result<()>
```
→ CTL `OPUS_SET_MAX_BANDWIDTH_REQUEST` with `bandwidth as i32`.

```rust
/// Gets the encoder's configured prediction status.
pub fn prediction_disabled(&self) -> Result<bool>
```
→ CTL `OPUS_GET_PREDICTION_DISABLED_REQUEST`, `n == 1`.

```rust
/// If set `prediction_disabled` to `true`, disables almost all use of
/// prediction, making frames almost completely independent.
///
/// This reduces quality.
pub fn set_prediction_disabled(&mut self, prediction_disabled: bool) -> Result<()>
```
→ CTL `OPUS_SET_PREDICTION_DISABLED_REQUEST`.

```rust
/// Gets the encoder's configured signal type.
pub fn signal(&self) -> Result<Signal>
```
→ CTL `OPUS_GET_SIGNAL_REQUEST`, then `Signal::try_from`.

```rust
/// Configures the type of signal being encoded.
///
/// This is a hint which helps the encoder's mode selection.
pub fn set_signal(&mut self, signal: Signal) -> Result<()>
```
→ CTL `OPUS_SET_SIGNAL_REQUEST` with `signal as i32`.

```rust
/// Gets the encoder's configured bandpass.
pub fn bandwidth(&self) -> Result<Bandwidth>
```
→ CTL `OPUS_GET_BANDWIDTH_REQUEST`, then `Bandwidth::try_from`.

```rust
/// Sets the encoder's bandpass to a specific value.
///
/// This prevents the encoder from automatically selecting the bandpass
/// based on the available bitrate.
/// If an application knows the bandpass of the input audio it is providing,
/// it should normally use [`set_max_bandwidth`] instead,
/// which still gives the encoder the freedom to reduce the bandpass when
/// the bitrate becomes too low, for better overall quality.
///
/// [`set_max_bandwidth`]: struct.Encoder.html#method.set_max_bandwidth
pub fn set_bandwidth(&mut self, bandwidth: Bandwidth) -> Result<()>
```
→ CTL `OPUS_SET_BANDWIDTH_REQUEST` with `bandwidth as i32`.

```rust
/// Gets encoder's configured use of discontinuous transmission.
pub fn dtx(&self) -> Result<bool>
```
→ CTL `OPUS_GET_DTX_REQUEST`, `n == 1`.

```rust
/// Configures the encoder's use of discontinuous transmission (DTX).
pub fn set_dtx(&mut self, dtx: bool) -> Result<()>
```
→ CTL `OPUS_SET_DTX_REQUEST`.

```rust
/// Enables the encoder's use of discontinuous transmission (DTX).
pub fn enable_dtx(&mut self) -> Result<()>
```
Calls `self.set_dtx(true)`.

```rust
/// Disables the encoder's use of discontinuous transmission (DTX).
pub fn disable_dtx(&mut self) -> Result<()>
```
Calls `self.set_dtx(false)`.

```rust
/// Gets the encoder's configured signal depth.
pub fn lsb_depth(&self) -> Result<u8>
```
→ CTL `OPUS_GET_LSB_DEPTH_REQUEST`, `n as u8`.

```rust
/// Configures the depth of signal being encoded.
///
/// This is a hint which helps the encoder identify silence and near-silence.
/// It represents the number of significant bits of linear intensity below
/// which the signal contains ignorable quantisation or other noise.
///
/// For example, a depth of 14 would be an appropriate setting for G.711
/// u-law input. A depth of 16 would be appropriate for 16-bit linear pcm
/// input with `encode_float()`.
///
/// When using `encode()` instead of `encode_float()`, or when libopus is
/// compiled for fixed-point, the encoder uses the minimum of the value set
/// here and the value 16.
pub fn set_lsb_depth(&mut self, lsb_depth: u8) -> Result<()>
```
→ CTL `OPUS_SET_LSB_DEPTH_REQUEST` with `i32::from(lsb_depth)`.
(No Rust-side bit-depth validation; Opus rejects `<8`/`>24` with `BadArgument`.)

#### `impl Drop for Encoder`

```rust
impl Drop for Encoder {
    /// We have to ensure that the resource our wrapping Opus-struct is pointing
    /// to is deallocated properly.
    fn drop(&mut self) {
        unsafe { ffi::opus_encoder_destroy(self.pointer) }
    }
}
```

#### Complete list of `Encoder` CTL constants used

| Method | CTL request constant |
| --- | --- |
| `GenericCtl::final_range` | `OPUS_GET_FINAL_RANGE_REQUEST` |
| `GenericCtl::phase_inversion_disabled` | `OPUS_SET_PHASE_INVERSION_DISABLED_REQUEST` *(sic)* |
| `GenericCtl::set_phase_inversion_disabled` | `OPUS_SET_PHASE_INVERSION_DISABLED_REQUEST` |
| `GenericCtl::sample_rate` | `OPUS_GET_SAMPLE_RATE_REQUEST` |
| `GenericCtl::reset_state` | `OPUS_RESET_STATE` (passed as a "request") |
| `complexity` / `set_complexity` | `OPUS_GET_COMPLEXITY_REQUEST` / `OPUS_SET_COMPLEXITY_REQUEST` |
| `application` / `set_application` | `OPUS_GET_APPLICATION_REQUEST` / `OPUS_SET_APPLICATION_REQUEST` |
| `bitrate` / `set_bitrate` | `OPUS_GET_BITRATE_REQUEST` / `OPUS_SET_BITRATE_REQUEST` |
| `vbr_constraint` / `set_vbr_constraint` | `OPUS_GET_VBR_CONSTRAINT_REQUEST` / `OPUS_SET_VBR_CONSTRAINT_REQUEST` |
| `vbr` / `set_vbr` | `OPUS_GET_VBR_REQUEST` / `OPUS_SET_VBR_REQUEST` |
| `inband_fec` / `set_inband_fec` | `OPUS_GET_INBAND_FEC_REQUEST` / `OPUS_SET_INBAND_FEC_REQUEST` |
| `packet_loss_perc` / `set_packet_loss_perc` | `OPUS_GET_PACKET_LOSS_PERC_REQUEST` / `OPUS_SET_PACKET_LOSS_PERC_REQUEST` |
| `lookahead` | `OPUS_GET_LOOKAHEAD_REQUEST` |
| `force_channels` / `set_force_channels` | `OPUS_GET_FORCE_CHANNELS_REQUEST` / `OPUS_SET_FORCE_CHANNELS_REQUEST` |
| `max_bandwidth` / `set_max_bandwidth` | `OPUS_GET_MAX_BANDWIDTH_REQUEST` / `OPUS_SET_MAX_BANDWIDTH_REQUEST` |
| `prediction_disabled` / `set_prediction_disabled` | `OPUS_GET_PREDICTION_DISABLED_REQUEST` / `OPUS_SET_PREDICTION_DISABLED_REQUEST` |
| `signal` / `set_signal` | `OPUS_GET_SIGNAL_REQUEST` / `OPUS_SET_SIGNAL_REQUEST` |
| `bandwidth` / `set_bandwidth` | `OPUS_GET_BANDWIDTH_REQUEST` / `OPUS_SET_BANDWIDTH_REQUEST` |
| `dtx` / `set_dtx` | `OPUS_GET_DTX_REQUEST` / `OPUS_SET_DTX_REQUEST` |
| `lsb_depth` / `set_lsb_depth` | `OPUS_GET_LSB_DEPTH_REQUEST` / `OPUS_SET_LSB_DEPTH_REQUEST` |

There is **no** `set_sample_rate` method (the `GenericCtl` doc-comment explains
that it is excluded because it is unimplemented on `Decoder`).

### 7.3 `Decoder`

```rust
/// `Decoder` to decode.
#[derive(Debug)]
pub struct Decoder {
    pointer: *mut ffi::OpusDecoder,
    channels: Channels,
}

/// The Opus decoder can be sent between threads unless the Opus library
/// has been compiled with `NONTHREADSAFE_PSEUDOSTACK` to disallow decoding in
/// parallel.
unsafe impl Send for Decoder {}
```

Both fields are **private**. Only `Debug` is derived.

#### `impl GenericCtl for Decoder`

```rust
impl GenericCtl for Decoder {
    fn final_range(&self) -> Result<u32> {
        self.decoder_ctl_request(ffi::OPUS_GET_FINAL_RANGE_REQUEST)
            .map(|v| v as u32)
    }

    fn phase_inversion_disabled(&self) -> Result<bool> {
        self.decoder_ctl_request(ffi::OPUS_GET_PHASE_INVERSION_DISABLED_REQUEST)
            .map(|b| b == 1)
    }

    fn set_phase_inversion_disabled(&mut self, disabled: bool) -> Result<()> {
        let disable_phase_inversion = if disabled { 1 } else { 0 };
        self.set_decoder_ctl_request(
            ffi::OPUS_SET_PHASE_INVERSION_DISABLED_REQUEST,
            disable_phase_inversion,
        )
        .map(|_| ())
    }

    fn sample_rate(&self) -> Result<SampleRate> {
        self.decoder_ctl_request(ffi::OPUS_GET_SAMPLE_RATE_REQUEST)
            .and_then(SampleRate::try_from)
    }

    fn reset_state(&mut self) -> Result<()> {
        self.decoder_ctl_request(ffi::OPUS_RESET_STATE).map(|_| ())
    }
}
```

Note: the `Decoder` impl has **no doc comments** on any of the five methods
(unlike `Encoder`'s impl).

#### `impl Decoder` (inherent)

```rust
/// Creates a new Opus decoder.
pub fn new(sample_rate: SampleRate, channels: Channels) -> Result<Decoder>
```
→ `opus_decoder_create(opus_int32 Fs, int channels, int *error)`. Same
`if opus_code == ffi::OPUS_OK || !pointer.is_null()` acceptance test and
`Err(ErrorCode::from(opus_code).into())` on failure.

```rust
/// Decodes an Opus packet as `input` and writes decoded data into `output`.
/// Passing `None` as `input` indicates a packet loss.
///
/// **Errors**:
/// Returns [Error::Opus] when Opus encountered a problem.
///
/// [Error::Opus]: crate::error::Error::Opus
pub fn decode(
    &mut self,
    input: Option<Packet<'_>>,
    mut output: MutSignals<'_, i16>,
    fec: bool,
) -> Result<usize>
```
→ `opus_decode(OpusDecoder *st, const unsigned char *data, opus_int32 len, opus_int16 *pcm, int frame_size, int decode_fec)`.
`None` passes `(std::ptr::null(), 0)`. `frame_size` is
`output.i32_len() / self.channels as i32`; `fec as i32`. No length checks.

```rust
/// Decodes an Opus frame from floating point input.
///
/// The `input` signal (interleaved if 2 channels) will be encoded into the
/// `output` payload and on success, returns the length of the
/// encoded packet.
///
/// **Errors**:
/// Returns [Error::Opus] when Opus encountered a problem.
///
/// [Error::Opus]: crate::error::Error::Opus
pub fn decode_float(
    &mut self,
    input: Option<Packet<'_>>,
    mut output: MutSignals<'_, f32>,
    fec: bool,
) -> Result<usize>
```
→ `opus_decode_float(OpusDecoder *st, const unsigned char *data, opus_int32 len, float *pcm, int frame_size, int decode_fec)`.

```rust
/// Gets the number of samples of an Opus packet.
///
/// **Errors**:
/// Returns [Error::Opus] when Opus encountered a problem.
///
/// [Error::Opus]: crate::error::Error::Opus
pub fn nb_samples(&self, input: Packet<'_>) -> Result<usize>
```
→ `opus_decoder_get_nb_samples(const OpusDecoder *dec, const unsigned char packet[], opus_int32 len)`.

Private CTL helpers (not public API, documented in source):

```rust
/// Issues a CTL `request` to Opus without argument used to
/// request a value.
/// If Opus returns a value smaller than 0, it indicates an error.
///
/// **Errors**:
/// Returns [Error::Opus] when Opus encountered a problem
///
/// [Error::Opus]: crate::error::Error::Opus
fn decoder_ctl_request(&self, request: i32) -> Result<i32>

/// Issues a CTL `request` to Opus accepting an additional argument used
/// to set the `decoder`'s setting to `value`.
/// If Opus returns a value smaller than 0, it indicates an error.
///
/// **Errors**:
/// Returns [Error::Opus] when Opus encountered a problem
///
/// [Error::Opus]: crate::error::Error::Opus
fn set_decoder_ctl_request(&self, request: i32, value: i32) -> Result<()>
```
Both use `ffi::opus_decoder_ctl(self.pointer, ...)`.

> Contrast with `Encoder`: the decoder CTL helpers are **private**, while
> `Encoder::encoder_ctl_request`/`set_encoder_ctl_request` are public. There is
> no public raw-CTL escape hatch on `Decoder` in this release.

```rust
/// Gets the duration (in samples) of the last packet successfully decoded
/// or concealed.
pub fn last_packet_duration(&self) -> Result<u32>
```
→ CTL `OPUS_GET_LAST_PACKET_DURATION_REQUEST`, `v as u32`.

```rust
/// Gets the pitch period at 48 kHz of the last decoded frame, if available.
///
/// This can be used for any post-processing algorithm requiring the use of
/// pitch, e.g. time stretching/shortening.
/// If the last frame was not voiced, or if the pitch was not coded in the
/// frame, then zero is returned.
pub fn pitch(&self) -> Result<i32>
```
→ CTL `OPUS_GET_PITCH_REQUEST`.

```rust
/// Gets the decoder's configured amount to scale PCM signal by
/// in Q8 dB units.
pub fn gain(&self) -> Result<i32>
```
→ CTL `OPUS_GET_GAIN_REQUEST`.

```rust
/// Configures decoder gain adjustment.
///
/// Scales the decoded output by a factor of `gain` specified in
/// Q8 dB units.
///
/// **Warning**:
/// This has a maximum range of -32768 to 32767 inclusive, and returns
/// [`BadArgument`] otherwise.
/// The default is 0 indicating no adjustment.
///
/// **Info**:
/// This setting survives decoder reset.
///
/// [`BadArgument`]: ../error/enum.ErrorCode.html#variant.BadArgument
pub fn set_gain(&self, gain: i32) -> Result<()>
```
→ CTL `OPUS_SET_GAIN_REQUEST`. **Takes `&self`, not `&mut self`** (the only
mutating decoder setter with a shared receiver; `set_decoder_ctl_request` itself
also takes `&self`).

```rust
/// Gets size of self's underlying Opus-decoder in bytes.
pub fn size(&self) -> usize
```
→ `opus_decoder_get_size(int channels)`; uses `self.channels as i32`.

#### Free function `size` (re-exported as `coder::size`)

```rust
/// Gets size of an Opus-decoder in bytes.
pub fn size(channels: Channels) -> usize {
    unsafe { ffi::opus_decoder_get_size(channels as i32) as usize }
}
```
→ `opus_decoder_get_size(int channels)`. There is **no** analogous public
`Encoder` size function.

#### `impl Drop for Decoder`

```rust
impl Drop for Decoder {
    /// We have to ensure that the resource our wrapping Opus-struct is pointing
    /// to is deallocated properly.
    fn drop(&mut self) {
        unsafe { ffi::opus_decoder_destroy(self.pointer) }
    }
}
```

#### Complete list of `Decoder` CTL constants used

| Method | CTL request constant |
| --- | --- |
| `GenericCtl::final_range` | `OPUS_GET_FINAL_RANGE_REQUEST` |
| `GenericCtl::phase_inversion_disabled` | `OPUS_GET_PHASE_INVERSION_DISABLED_REQUEST` |
| `GenericCtl::set_phase_inversion_disabled` | `OPUS_SET_PHASE_INVERSION_DISABLED_REQUEST` |
| `GenericCtl::sample_rate` | `OPUS_GET_SAMPLE_RATE_REQUEST` |
| `GenericCtl::reset_state` | `OPUS_RESET_STATE` |
| `last_packet_duration` | `OPUS_GET_LAST_PACKET_DURATION_REQUEST` |
| `pitch` | `OPUS_GET_PITCH_REQUEST` |
| `gain` | `OPUS_GET_GAIN_REQUEST` |
| `set_gain` | `OPUS_SET_GAIN_REQUEST` |

---

## 8. Argument-validation semantics (exact)

These are the **only** Rust-side checks performed before/around calling into the
C library. Everything else is delegated to libopus.

1. **Packet buffer length** (`packet::packet_len_check`, private):
   * `len == 0` → `Err(Error::EmptyPacket)`.
   * `len > std::i32::MAX as usize` (i.e. `> 2147483647`) → `Err(Error::PacketTooLarge)`.
   * otherwise `Ok(len as i32)`.
   Applied by `TryFrom<&[u8]> for Packet`, `TryFrom<&mut [u8]> for MutPacket`
   (and via delegation the `Vec` variants), and again by `MutPacket::i32_len`.

2. **Signals buffer length** (`TryFrom<&mut [T]> for MutSignals`):
   * `len > std::i32::MAX as usize` → `Err(Error::SignalsTooLarge)`.
   * **Empty is allowed.** No alignment/type-size check. The `Vec` variant
     delegates to the slice variant.

3. **Sample rate ↔ `i32`**:
   * `SampleRate → i32`: implicit `as i32` on the `#[repr(i32)]` enum. No `From`
     impl. Accepted values are exactly `{8000, 12000, 16000, 24000, 48000}`.
   * `i32 → SampleRate`: `SampleRate::try_from`; any other integer →
     `Err(Error::InvalidSampleRate(value))`.
   * `Encoder::new` / `Decoder::new` pass `sample_rate as i32` straight to
     `opus_*_create`; no extra check.

4. **Channel count**:
   * `Channels → i32`: `From<Channels> for i32` (also `as i32` casts elsewhere).
   * `i32 → Channels`: `Channels::try_from` accepts `-1000` (`Auto`), `1`
     (`Mono`), `2` (`Stereo`); anything else →
     `Err(Error::InvalidChannels(value))`.
   * `Channels::Auto` is documented as "Not supported when constructing encoders
     or decoders"; `Encoder::new`/`Decoder::new` do not pre-check it — libopus
     returns `OPUS_BAD_ARG`, surfaced as `Error::Opus(ErrorCode::BadArgument)`
     (verified by the in-crate test `encoder_construction`).
   * No check that `input.len()`/`output.len()` is divisible by the channel count
     in `encode`/`encode_float`/`decode`/`decode_float`/`SoftClip::apply`.

5. **crate `Result` alias**: `pub type Result<T> = std::result::Result<T, Error>;`
   defined in `error.rs` and re-exported at the crate root. `GenericCtl` spells
   its returns as `Result<T, Error>` where `Result` there is the alias (so the
   error type is still `Error`).

6. **"`Error::new`"**: **does not exist.** Error construction paths are:
   * direct enum literals (`Error::EmptyPacket`, `Error::InvalidSignal(v)`, …);
   * `Error::from(error_code)` via `impl From<ErrorCode> for Error`;
   * `ErrorCode::from(number)` via `impl From<i32> for ErrorCode`;
   * `try_map_opus_error(i32) -> Result<i32>` for negative libopus returns.

7. **`Encoder::new` / `Decoder::new` success test**: `opus_code == OPUS_OK || !pointer.is_null()`.
   Because of `||`, the pointer is always accepted when non-null; if the pointer
   is null and `opus_code != OPUS_OK`, `Error::Opus(ErrorCode::from(opus_code))`
   is returned. If the pointer is null **and** `opus_code == OPUS_OK`, an
   `Encoder`/`Decoder` wrapping a null pointer is returned and later calls are UB.

8. **Raw-CTL validation**: `encoder_ctl_request`/`set_encoder_ctl_request` and the
   private decoder equivalents perform no argument checking; they only map
   negative returns to `Error::Opus`.

9. **Type-level newtype path independence**: `Encoder::encode` and
   `Encoder::encode_float` accept plain `&[i16]` / `&[f32]` and `&mut [u8]`
   directly, bypassing `MutSignals`/`MutPacket` (and therefore their checks).
   `Decoder::decode`/`decode_float` accept `MutSignals<..>` but a plain
   `Option<Packet<'_>>` for input, so the packet check is inherited from
   `Packet`'s constructor. Padding functions take `MutPacket`, so they do
   re-check via `i32_len()`.

---

## 9. Unsafe code, raw pointers and slices

* **There are no `pub unsafe fn` items.** Every public function is a safe `fn`;
  all `unsafe` usage is inside function bodies, wrapping `audiopus_sys` calls.
* Raw-pointer-returning public methods:
  * `Packet::as_ptr(&self) -> *const u8`
  * `MutPacket::as_mut_ptr(&mut self) -> *mut u8`
  * `MutSignals::as_mut_ptr(&mut self) -> *mut T`
* Raw pointers held in private fields: `Encoder.pointer: *mut ffi::OpusEncoder`,
  `Decoder.pointer: *mut ffi::OpusDecoder`,
  `Repacketizer.pointer: *mut ffi::OpusRepacketizer`.
* `unsafe impl Send for Encoder {}` and `unsafe impl Send for Decoder {}` — the
  only unsafe trait impls. No `Sync` impls anywhere; `Repacketizer` gets no
  `Send`.
* Unsafe blocks appear in: `version` (`CStr::from_ptr`), all of `packet.rs` free
  functions, all of `repacketizer.rs`, `softclip.rs`, every encoder/decoder
  method that calls `ffi::*`, and all three `Drop` impls.
* Slices are used as raw pointer + `i32` length pairs; the lengths are obtained
  from `Packet::i32_len`/`MutPacket::i32_len`/`MutSignals::i32_len`. The only
  `i32` conversions that are *guaranteed* in-bounds by construction are the
  `i32_len` accessors; the ad-hoc `as i32` casts in `encode`, `encode_float`,
  `decode`, `decode_float`, `size`, `set_*` are **not** guarded.

---

## 10. FFI surfaces used from `audiopus_sys` 0.2.2

Functions referenced by the crate (all under `audiopus::ffi`):

| Function | Used by |
| --- | --- |
| `opus_get_version_string` | `version()` |
| `opus_encoder_create` | `Encoder::new` |
| `opus_encoder_destroy` | `Drop for Encoder` |
| `opus_encoder_ctl` | `Encoder::encoder_ctl_request` / `set_encoder_ctl_request` |
| `opus_encode` | `Encoder::encode` |
| `opus_encode_float` | `Encoder::encode_float` |
| `opus_decoder_create` | `Decoder::new` |
| `opus_decoder_destroy` | `Drop for Decoder` |
| `opus_decoder_ctl` | decoder CTL helpers |
| `opus_decoder_get_size` | `Decoder::size`, `coder::size` |
| `opus_decoder_get_nb_samples` | `Decoder::nb_samples` |
| `opus_decode` | `Decoder::decode` |
| `opus_decode_float` | `Decoder::decode_float` |
| `opus_packet_get_bandwidth` | `packet::bandwidth` |
| `opus_packet_get_samples_per_frame` | `packet::samples_per_frame` |
| `opus_packet_get_nb_samples` | `packet::nb_samples` |
| `opus_packet_get_nb_channels` | `packet::nb_channels` |
| `opus_packet_get_nb_frames` | `packet::nb_frames` |
| `opus_pcm_soft_clip` | `SoftClip::apply` |
| `opus_repacketizer_get_size` | `repacketizer::repacketizer_size` |
| `opus_repacketizer_create` | `Repacketizer::new` |
| `opus_repacketizer_destroy` | `Drop for Repacketizer` |
| `opus_repacketizer_get_nb_frames` | `Repacketizer::nb_frames` |
| `opus_repacketizer_out` | `Repacketizer::repacketizer_out` |
| `opus_repacketizer_out_range` | `Repacketizer::repacketizer_out_range` |
| `opus_repacketizer_cat` | `Repacketizer::repacketizer_cat` |
| `opus_packet_pad` | `repacketizer::packet_pad` |
| `opus_packet_unpad` | `repacketizer::packet_unpad` |
| `opus_multistream_packet_pad` | `repacketizer::multistream_packet_pad` |
| `opus_multistream_packet_unpad` | `repacketizer::multistream_packet_unpad` |

Referenced `audiopus_sys` constants (values verified from the vendored binding):

```rust
pub const OPUS_OK: i32 = 0;
pub const OPUS_BAD_ARG: i32 = -1;
pub const OPUS_BUFFER_TOO_SMALL: i32 = -2;
pub const OPUS_INTERNAL_ERROR: i32 = -3;
pub const OPUS_INVALID_PACKET: i32 = -4;
pub const OPUS_UNIMPLEMENTED: i32 = -5;
pub const OPUS_INVALID_STATE: i32 = -6;
pub const OPUS_ALLOC_FAIL: i32 = -7;
pub const OPUS_AUTO: i32 = -1000;
pub const OPUS_BITRATE_MAX: i32 = -1;
pub const OPUS_APPLICATION_VOIP: i32 = 2048;
pub const OPUS_APPLICATION_AUDIO: i32 = 2049;
pub const OPUS_APPLICATION_RESTRICTED_LOWDELAY: i32 = 2051;
pub const OPUS_SIGNAL_VOICE: i32 = 3001;
pub const OPUS_SIGNAL_MUSIC: i32 = 3002;
pub const OPUS_BANDWIDTH_NARROWBAND: i32 = 1101;
pub const OPUS_BANDWIDTH_MEDIUMBAND: i32 = 1102;
pub const OPUS_BANDWIDTH_WIDEBAND: i32 = 1103;
pub const OPUS_BANDWIDTH_SUPERWIDEBAND: i32 = 1104;
pub const OPUS_BANDWIDTH_FULLBAND: i32 = 1105;
```

Plus the many `OPUS_*_REQUEST` / `OPUS_RESET_STATE` CTL constants listed in §7.

---

## 11. Public-but-undocumented / surprising items

### 11.1 Undocumented public items

`#![deny(missing_docs)]` is disabled, and these public items carry no doc
comment:

* `Channels::is_mono`, `Channels::is_stereo`
* `MutSignals::as_mut_ptr`
* `MutPacket::as_mut_ptr`
* `SoftClip` (the struct itself) and `SoftClip::new`
* `Repacketizer` (the struct itself) and `Repacketizer::new`,
  `Repacketizer::nb_frames`, `Repacketizer::repacketizer_out`,
  `Repacketizer::repacketizer_out_range`, `Repacketizer::repacketizer_cat`
* `repacketizer::multistream_packet_pad`, `multistream_packet_unpad`,
  `packet_pad`, `packet_unpad`
* `GenericCtl::final_range`, `phase_inversion_disabled`,
  `set_phase_inversion_disabled`, `sample_rate`, `reset_state` (the trait is
  documented, the methods are not)
* `Application`, `Signal`, `Bitrate`, `ErrorCode` enum declarations themselves
  (their variants are documented except `Signal`'s and `Bitrate`'s docs cover
  only parts; `Application` has no top-level doc)
* `error::Result` type alias
* `packet::Packet::as_ptr`
* `Decoder::size` has a doc; `coder::size` has a doc.

### 11.2 Derived / manual trait impls inventory

| Type | Derives | Manual impls |
| --- | --- | --- |
| `Signal` | `Clone, Copy, Debug, Eq, PartialEq, Hash` | `TryFrom<i32>` |
| `Bitrate` | `Clone, Copy, Debug, Eq, PartialEq, Hash` | `From<Bitrate> for i32`, `TryFrom<i32>` |
| `SampleRate` | `Debug, Clone, Copy, Eq, PartialEq, Hash` | `TryFrom<i32>` |
| `Application` | `Debug, Clone, Copy, Eq, PartialEq, Hash` | `TryFrom<i32>` |
| `Channels` | `Clone, Copy, Debug, Eq, PartialEq, Hash` | `From<Channels> for i32`, `TryFrom<i32>` |
| `Bandwidth` | `Clone, Copy, Debug, Eq, PartialEq, Hash` | `TryFrom<i32>` |
| `MutSignals<'a, T>` | `Debug` | `TryFrom<&'a mut [T]>`, `TryFrom<&'a mut Vec<T>>` |
| `Error` | `Clone, Copy, Debug, Eq, PartialEq, Hash` | `StdError`, `Display`, `From<ErrorCode>` |
| `ErrorCode` | `Clone, Copy, Debug, Eq, PartialEq, Hash` | `Display`, `StdError` (empty), `From<i32>` |
| `Packet<'a>` | `Debug` | `TryFrom<&'a Vec<u8>>`, `TryFrom<&'a [u8]>` |
| `MutPacket<'a>` | `Debug` | `TryFrom<&'a mut Vec<u8>>`, `TryFrom<&'a mut [u8]>` |
| `SoftClip` | `Clone, Debug` | — |
| `Repacketizer` | `Debug` | `Default`, `Drop` |
| `Encoder` | `Debug` | `unsafe impl Send`, `GenericCtl`, `Drop` |
| `Decoder` | `Debug` | `unsafe impl Send`, `GenericCtl`, `Drop` |

* `Default` is implemented **only** for `Repacketizer` (delegating to `new()`).
* There are **no blanket impls** anywhere in the crate.
* `Decoder`'s `Debug` output includes the raw pointer value and the `channels` field.
* `SoftClip`'s `Clone` copies the accumulated `memory` (not a fresh state).

### 11.3 Surprising behaviours / likely bugs

1. `Bitrate::try_from(i32)` returns `Error::InvalidBandwidth` instead of
   `Error::InvalidBitrate` (lib.rs:104).
2. `Encoder::phase_inversion_disabled` uses
   `OPUS_SET_PHASE_INVERSION_DISABLED_REQUEST` in the getter path
   (encoder.rs:33). The decoder counterpart uses the `GET` constant.
3. `Error::InvalidBitrate(i32)`, `Error::InvalidComplexity(i32)` and
   `Error::MappingExpectedLen(usize)` are **never constructed** by the crate —
   dead variants that are nonetheless public and part of the API.
4. `Display for Error` contains the typo `"Invalid Bandwitdh: {}"`.
5. `Display for ErrorCode` contains `"Internal error inside Opus occured"` (typo)
   and `"...Audiopus..."` (typo for "audiopus").
6. `version()` can panic via `.unwrap()` on `CStr::to_str()`.
7. `SoftClip::apply` always returns `Ok(())` and discards the FFI result; its
   `Result<()>` return type is effectively infallible.
8. `Repacketizer` is `!Send`/`!Sync` while `Encoder`/`Decoder` are `Send`
   (unsafe impl). This asymmetry is easy to miss.
9. `Decoder::set_gain(&self, …)` mutates decoder state through `&self`
   (interior mutability is actually provided by the C library); the same applies
   to the private `set_decoder_ctl_request(&self, …)`. `Encoder`'s setters all
   take `&mut self`.
10. `GenericCtl::phase_inversion_disabled` is declared on a trait shared with
    `Decoder`, so the encoder bug in (2) is observable through the trait.
11. `samples_per_frame` returns `Result<usize>` but cannot fail
    (it does `Ok(... as usize)` unconditionally).
12. `Encoder::encode`/`encode_float` accept raw slices and do not enforce the
    crate's own `MutSignals`/`MutPacket` newtype invariant; lengths are cast
    `as i32` unchecked.
13. `Encoder::new`/`Decoder::new` accept a non-null pointer regardless of the
    returned error code because of `||` rather than `&&` (lib.rs of encoder/decoder).
14. `packet_pad`/`multistream_packet_pad` take a length parameter (`new_len`)
    that is **not** validated against the actual `MutPacket` slice length.
15. `Channels::is_mono`/`is_stereo` take `self` by value rather than `&self`,
    and the crate never uses them internally.
16. Crate-internal `use std::convert::{TryFrom, TryInto}` is private, so a
    reimplementation must expose the same *set* of `TryFrom` impls but need not
    (and must not) re-export `TryFrom` itself.
17. The `Reader`-facing typos in doc-links
    (`../enum.Application.html` used for `Bandwidth`, `..BadArgument.html`
    suffixes with `.html`) are cosmetic only.
18. `Cargo.toml`'s `default_features = ["coder"]` is not a valid `default`
    feature definition; the true default feature set is empty.

---

## 12. Machine-checkable checklist of the public API

Types (public):

* `coder::GenericCtl` (trait)
* `coder::Encoder` (struct)
* `coder::Decoder` (struct)
* `coder::size` (fn)
* `error::Error` (enum)
* `error::ErrorCode` (enum)
* `error::Result<T>` (type alias)
* `error::try_map_opus_error` (fn)
* `packet::Packet<'a>` (struct)
* `packet::MutPacket<'a>` (struct)
* `packet::bandwidth`, `packet::samples_per_frame`, `packet::nb_samples`,
  `packet::nb_channels`, `packet::nb_frames` (fns)
* `repacketizer::Repacketizer` (struct)
* `repacketizer::repacketizer_size`, `multistream_packet_pad`,
  `multistream_packet_unpad`, `packet_pad`, `packet_unpad` (fns)
* `softclip::SoftClip` (struct)
* `Signal`, `Bitrate`, `SampleRate`, `Application`, `Channels`, `Bandwidth`,
  `MutSignals<'a, T>` (crate root)
* `version` (crate root fn)
* Re-exports: `Error`, `ErrorCode`, `Result` at crate root; `audiopus_sys as ffi`
  at crate root.

There is **no** `Encoder::size`, **no** `set_sample_rate`, **no** multistream
encoder/decoder type, **no** `BandwidthError`, and **no** `Error::new`.

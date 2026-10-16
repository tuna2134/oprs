//! The Opus decoder.
//!
//! Compatible reimplementation of `audiopus::coder::Decoder` (0.3.0-rc.0).  The
//! public signatures and the interior-mutability quirks (`set_gain` and the
//! private `set_decoder_ctl_request` take `&self`) are reproduced verbatim; the
//! state is an owned pure-Rust [`crate::opus::decoder::OpusDecoder`] rather than
//! a C pointer.

use super::GenericCtl;
use crate::{
    Channels, ErrorCode, MutSignals, Result, SampleRate, error::try_map_opus_error, ffi,
    packet::Packet,
};
use std::cell::RefCell;
use std::fmt;

/// `Decoder` to decode.
pub struct Decoder {
    /// The ported decoder state (see [`super::encoder::Encoder`] for why this
    /// is a [`RefCell`]).
    state: RefCell<Box<crate::opus::decoder::OpusDecoder>>,
    channels: Channels,
}

/// The Opus decoder can be sent between threads unless the Opus library
/// has been compiled with `NONTHREADSAFE_PSEUDOSTACK` to disallow decoding in
/// parallel.
unsafe impl Send for Decoder {}

impl fmt::Debug for Decoder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `audiopus` prints its raw pointer here; an owned state has no pointer
        // to print, so only the observable configuration is shown.
        f.debug_struct("Decoder")
            .field("channels", &self.channels)
            .finish_non_exhaustive()
    }
}

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

impl Decoder {
    /// Creates a new Opus decoder.
    pub fn new(sample_rate: SampleRate, channels: Channels) -> Result<Decoder> {
        let mut opus_code = 0;

        let state = crate::opus::decoder::OpusDecoder::create(
            sample_rate as i32,
            channels as i32,
            &mut opus_code,
        );

        // Same acceptance test as `audiopus` (`opus_code == OPUS_OK || !pointer.is_null()`),
        // expressed with the owned `Option` state.
        match state {
            Some(state) => Ok(Decoder {
                state: RefCell::new(state),
                channels,
            }),
            None => Err(ErrorCode::from(opus_code).into()),
        }
    }

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
    ) -> Result<usize> {
        let (input_slice, input_len) = match input {
            Some(value) => (Some(value.as_slice()), value.i32_len()),
            None => (None, 0),
        };

        let frame_size = output.i32_len() / self.channels as i32;

        try_map_opus_error(self.state.borrow_mut().decode(
            input_slice,
            input_len,
            output.as_mut_slice(),
            frame_size,
            fec as i32,
        ))
        .map(|n| n as usize)
    }

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
    ) -> Result<usize> {
        let (input_slice, input_len) = match input {
            Some(value) => (Some(value.as_slice()), value.i32_len()),
            None => (None, 0),
        };

        let frame_size = output.i32_len() / self.channels as i32;

        try_map_opus_error(self.state.borrow_mut().decode_float(
            input_slice,
            input_len,
            output.as_mut_slice(),
            frame_size,
            fec as i32,
        ))
        .map(|n| n as usize)
    }

    /// Gets the number of samples of an Opus packet.
    ///
    /// **Errors**:
    /// Returns [Error::Opus] when Opus encountered a problem.
    ///
    /// [Error::Opus]: crate::error::Error::Opus
    pub fn nb_samples(&self, input: Packet<'_>) -> Result<usize> {
        // The C `opus_decoder_get_nb_samples()` is exactly
        // `opus_packet_get_nb_samples(packet, len, dec->Fs)`; `Fs` is queried
        // through the documented CTL so no extra internal method is needed.
        let fs = self.decoder_ctl_request(ffi::OPUS_GET_SAMPLE_RATE_REQUEST)?;

        try_map_opus_error(crate::opus::packet::opus_packet_get_nb_samples(
            input.as_slice(),
            input.i32_len(),
            fs,
        ))
        .map(|n| n as usize)
    }

    /// Issues a CTL `request` to Opus without argument used to
    /// request a value.
    /// If Opus returns a value smaller than 0, it indicates an error.
    ///
    /// **Errors**:
    /// Returns [Error::Opus] when Opus encountered a problem
    ///
    /// [Error::Opus]: crate::error::Error::Opus
    fn decoder_ctl_request(&self, request: i32) -> Result<i32> {
        let mut value = 0;

        let ffi_result = self.state.borrow_mut().ctl(request, &mut value);

        try_map_opus_error(ffi_result)?;

        Ok(value)
    }

    /// Issues a CTL `request` to Opus accepting an additional argument used
    /// to set the `decoder`'s setting to `value`.
    /// If Opus returns a value smaller than 0, it indicates an error.
    ///
    /// **Errors**:
    /// Returns [Error::Opus] when Opus encountered a problem
    ///
    /// [Error::Opus]: crate::error::Error::Opus
    fn set_decoder_ctl_request(&self, request: i32, value: i32) -> Result<()> {
        let mut value = value;

        try_map_opus_error(self.state.borrow_mut().ctl(request, &mut value))?;

        Ok(())
    }

    /// Gets the duration (in samples) of the last packet successfully decoded
    /// or concealed.
    pub fn last_packet_duration(&self) -> Result<u32> {
        self.decoder_ctl_request(ffi::OPUS_GET_LAST_PACKET_DURATION_REQUEST)
            .map(|v| v as u32)
    }

    /// Gets the pitch period at 48 kHz of the last decoded frame, if available.
    ///
    /// This can be used for any post-processing algorithm requiring the use of
    /// pitch, e.g. time stretching/shortening.
    /// If the last frame was not voiced, or if the pitch was not coded in the
    /// frame, then zero is returned.
    pub fn pitch(&self) -> Result<i32> {
        self.decoder_ctl_request(ffi::OPUS_GET_PITCH_REQUEST)
    }

    /// Gets the decoder's configured amount to scale PCM signal by
    /// in Q8 dB units.
    pub fn gain(&self) -> Result<i32> {
        self.decoder_ctl_request(ffi::OPUS_GET_GAIN_REQUEST)
    }

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
    pub fn set_gain(&self, gain: i32) -> Result<()> {
        self.set_decoder_ctl_request(ffi::OPUS_SET_GAIN_REQUEST, gain)
    }

    /// Gets size of self's underlying Opus-decoder in bytes.
    pub fn size(&self) -> usize {
        crate::opus::decoder::OpusDecoder::get_size(self.channels as i32) as usize
    }
}

/// Gets size of an Opus-decoder in bytes.
pub fn size(channels: Channels) -> usize {
    crate::opus::decoder::OpusDecoder::get_size(channels as i32) as usize
}

#[cfg(test)]
mod tests {
    use super::{Decoder, size};
    use crate::coder::GenericCtl;
    use crate::packet::Packet;
    use crate::{Channels, MutSignals, SampleRate};
    use std::convert::TryFrom;

    fn assert_send<T: Send>() {}
    fn assert_debug<T: std::fmt::Debug>() {}

    /// `audiopus` marks both coders `Send` (and leaves them `!Sync`).
    #[test]
    fn decoder_is_send_and_debug() {
        assert_send::<Decoder>();
        assert_debug::<Decoder>();
    }

    /// The `GenericCtl` surface must be implemented by `Decoder`.
    #[test]
    fn decoder_implements_generic_ctl() {
        fn assert_generic_ctl<T: GenericCtl>() {}
        assert_generic_ctl::<Decoder>();
    }

    /// `coder::size` mirrors `opus_decoder_get_size`; the placeholder returns
    /// `OPUS_INTERNAL_ERROR`, so only the call is exercised here.
    #[test]
    fn size_is_callable() {
        let _ = size(Channels::Mono);
        let _ = size(Channels::Stereo);
    }

    /// Compile-time check of every public `Decoder` signature against the
    /// `audiopus` 0.3.0-rc.0 inventory (names, argument types, `&self`/`&mut
    /// self`, return types).  The function is never called: the codec internals
    /// are still placeholders, so these calls cannot run yet — they only have to
    /// type-check.
    #[allow(dead_code)]
    #[allow(unused_mut)]
    fn decoder_signature_check(
        mut decoder: Decoder,
        packet_bytes: &[u8],
        pcm: &mut [i16],
        pcm_float: &mut [f32],
    ) {
        let _ = Decoder::new(SampleRate::Hz48000, Channels::Stereo);
        let _ = decoder.decode(
            Some(Packet::try_from(packet_bytes).unwrap()),
            MutSignals::try_from(&mut pcm[..]).unwrap(),
            false,
        );
        let _ = decoder.decode(None, MutSignals::try_from(&mut pcm[..]).unwrap(), true);
        let _ = decoder.decode_float(
            Some(Packet::try_from(packet_bytes).unwrap()),
            MutSignals::try_from(&mut pcm_float[..]).unwrap(),
            false,
        );
        let _ = decoder.decode_float(
            None,
            MutSignals::try_from(&mut pcm_float[..]).unwrap(),
            true,
        );
        let _ = decoder.nb_samples(Packet::try_from(packet_bytes).unwrap());
        let _ = decoder.last_packet_duration();
        let _ = decoder.pitch();
        let _ = decoder.gain();
        let _ = decoder.set_gain(0);
        let _ = decoder.size();
        let _ = size(Channels::Mono);

        // `GenericCtl` (the trait is in scope).
        let _ = decoder.final_range();
        let _ = decoder.phase_inversion_disabled();
        let _ = decoder.set_phase_inversion_disabled(true);
        let _ = decoder.sample_rate();
        let _ = decoder.reset_state();
    }
}

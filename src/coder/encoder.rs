//! The Opus encoder.
//!
//! Compatible reimplementation of `audiopus::coder::Encoder` (0.3.0-rc.0).  The
//! public signatures, the CTL requests and the validation quirks (including the
//! `OPUS_SET_PHASE_INVERSION_DISABLED_REQUEST` used by the
//! [`GenericCtl::phase_inversion_disabled`] *getter*) are reproduced verbatim;
//! the state is an owned pure-Rust [`crate::opus::encoder::OpusEncoder`] rather
//! than a C pointer.

use super::GenericCtl;
use crate::{
    Application, Bandwidth, Bitrate, Channels, ErrorCode, Result, SampleRate, Signal,
    error::try_map_opus_error, ffi,
};
use std::cell::RefCell;
use std::fmt;

/// `Encoder` calls to Opus and offers method to encode and issue
/// requests to Opus.
pub struct Encoder {
    /// The ported encoder state.
    ///
    /// `audiopus` stores a `*mut OpusEncoder` and lets the C library provide the
    /// interior mutability that the `&self` entry points (`encode`,
    /// `encode_float`, the getters) need.  The pure-Rust port owns its state, so
    /// a [`RefCell`] provides the same interior mutability while keeping the
    /// `audiopus` signatures.
    state: RefCell<Box<crate::opus::encoder::OpusEncoder>>,
    channels: Channels,
}

/// The Opus encoder can be sent between threads unless the Opus library
/// has been compiled with `NONTHREADSAFE_PSEUDOSTACK` to disallow encoding in
/// parallel.
unsafe impl Send for Encoder {}

impl fmt::Debug for Encoder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `audiopus` prints its raw pointer here; an owned state has no pointer
        // to print, so only the observable configuration is shown.
        f.debug_struct("Encoder")
            .field("channels", &self.channels)
            .finish_non_exhaustive()
    }
}

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
    ///
    /// **Note**: `audiopus` passes the *set* request here (a known bug); it is
    /// kept verbatim so the observable behaviour is the same.
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

impl Encoder {
    /// Creates a new Opus encoder.
    ///
    /// **Warning**:
    /// If `channels` is set to [`Channels::Auto`] the function will
    /// return [`BadArgument`].
    ///
    /// [`Channels::Auto`]: ../enum.Channels.html#variant.Auto
    /// [`BadArgument`]: ../error/enum.ErrorCode.html#variant.BadArgument
    pub fn new(sample_rate: SampleRate, channels: Channels, mode: Application) -> Result<Encoder> {
        let mut opus_code = 0;

        let state = crate::opus::encoder::OpusEncoder::create(
            sample_rate as i32,
            channels as i32,
            mode as i32,
            &mut opus_code,
        );

        // `audiopus` accepts the result when `opus_code == OPUS_OK || !pointer.is_null()`,
        // so a non-null pointer is accepted even when `opus_code` is an error.
        // `Some`/`None` is the owned equivalent of the pointer test; the C never
        // returns a pointer together with a non-`OPUS_OK` code, so this is exact.
        match state {
            Some(state) => Ok(Encoder {
                state: RefCell::new(state),
                channels,
            }),
            None => Err(ErrorCode::from(opus_code).into()),
        }
    }

    /// Issues a CTL get-`request` to Opus.
    /// If Opus returns a negative value it indicates an error.
    ///
    /// **Info**:
    /// As [`Encoder`]'s methods cover all possible CTLs, it is recommended
    /// to use them instead.
    ///
    /// [`Encoder`]: struct.Encoder.html
    pub fn encoder_ctl_request(&self, request: i32) -> Result<i32> {
        let mut value = 0;

        let ffi_result = self.state.borrow_mut().ctl(request, &mut value);
        try_map_opus_error(ffi_result)?;

        Ok(value)
    }

    /// Issues a CTL set-`request` to Opus and sets the `Encoder`'s setting to
    /// `value` based on sent `request`.
    /// If Opus returns a negative value it indicates an error.
    ///
    /// **Info**:
    /// As [`Encoder`]'s methods cover all possible CTLs, it is recommended
    /// to use them instead.
    ///
    /// [`Encoder`]: struct.Encoder.html
    pub fn set_encoder_ctl_request(&mut self, request: i32, value: i32) -> Result<()> {
        let mut value = value;

        try_map_opus_error(self.state.borrow_mut().ctl(request, &mut value))?;

        Ok(())
    }

    /// Encodes an Opus frame.
    ///
    /// The `input` signal (interleaved if 2 channels) will be encoded into the
    /// `output` payload and on success returns the length of the
    /// encoded packet.
    pub fn encode(&self, input: &[i16], output: &mut [u8]) -> Result<usize> {
        let frame_size = input.len() as i32 / self.channels as i32;
        let max_data_bytes = output.len() as i32;

        try_map_opus_error(self.state.borrow_mut().encode(
            input,
            frame_size,
            output,
            max_data_bytes,
        ))
        .map(|n| n as usize)
    }

    /// Encodes an Opus frame from floating point input.
    ///
    /// The `input` signal (interleaved if 2 channels) will be encoded into the
    /// `output` payload and on success, returns the length of the
    /// encoded packet.
    pub fn encode_float(&self, input: &[f32], output: &mut [u8]) -> Result<usize> {
        let frame_size = input.len() as i32 / self.channels as i32;
        let max_data_bytes = output.len() as i32;

        try_map_opus_error(self.state.borrow_mut().encode_float(
            input,
            frame_size,
            output,
            max_data_bytes,
        ))
        .map(|n| n as usize)
    }

    /// Gets the encoder's complexity configuration.
    pub fn complexity(&self) -> Result<u8> {
        self.encoder_ctl_request(ffi::OPUS_GET_COMPLEXITY_REQUEST)
            .map(|v| v as u8)
    }

    /// Configures the encoder's computational complexity.
    ///
    /// **Warning**:
    /// If `complexity` exceeds 10, [`BadArgument`] will be returned.
    ///
    /// [`BadArgument`]: ../error/enum.ErrorCode.html#variant.BadArgument.html
    pub fn set_complexity(&mut self, complexity: u8) -> Result<()> {
        self.set_encoder_ctl_request(ffi::OPUS_SET_COMPLEXITY_REQUEST, i32::from(complexity))
    }

    /// Gets the encoder's configured application.
    pub fn application(&self) -> Result<Application> {
        self.encoder_ctl_request(ffi::OPUS_GET_APPLICATION_REQUEST)
            .and_then(Application::try_from)
    }

    /// Configures the encoder's intended application.
    ///
    /// The initial value is a mandatory argument in the [`new`]-function.
    ///
    /// [`new`]: struct.Encoder.html#method.new
    pub fn set_application(&mut self, application: Application) -> Result<()> {
        self.set_encoder_ctl_request(ffi::OPUS_SET_APPLICATION_REQUEST, application as i32)
            .map(|_| ())
    }

    /// Configures the bitrate in the encoder.
    ///
    /// Rates from 500 to 512000 bits per second are meaningful,
    /// as well as the special values [`Bitrate::Auto`] and [`Bitrate::Max`].
    /// [`Bitrate::Max`] can be used to cause the codec to use
    /// as much rate as it can, which is useful for controlling the rate by
    /// adjusting the output buffer size.
    pub fn set_bitrate(&mut self, bitrate: Bitrate) -> Result<()> {
        self.set_encoder_ctl_request(ffi::OPUS_SET_BITRATE_REQUEST, bitrate.into())?;

        Ok(())
    }

    /// Gets the encoder's configured bandpass.
    pub fn bitrate(&self) -> Result<Bitrate> {
        self.encoder_ctl_request(ffi::OPUS_GET_BITRATE_REQUEST)
            .and_then(Bitrate::try_from)
    }

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
    pub fn enable_vbr_constraint(&mut self) -> Result<()> {
        self.set_vbr_constraint(true)
    }

    /// Disables variable bitrate (VBR) in the encoder.
    ///
    /// The configured bitrate may not be met exactly because frames must be an
    /// integer number of bytes in length.
    ///
    /// **Warning**:
    /// Only the MDCT mode of Opus currently heeds the constraint.
    /// Speech mode ignores it completely,
    /// hybrid mode may fail to obey it if the LPC layer uses more bitrate
    /// than the constraint would have permitted.
    pub fn disable_vbr_constraint(&mut self) -> Result<()> {
        self.set_vbr_constraint(false)
    }

    /// Sets variable bitrate (VBR) in the encoder.
    ///
    /// The configured bitrate may not be met exactly because frames must be an
    /// integer number of bytes in length.
    ///
    /// **Warning**:
    /// Only the MDCT mode of Opus currently heeds the constraint.
    /// Speech mode ignores it completely,
    /// hybrid mode may fail to obey it if the LPC layer uses more bitrate
    /// than the constraint would have permitted.
    pub fn set_vbr_constraint(&mut self, enable: bool) -> Result<()> {
        let if_vbr_shall_be_enabled = if enable { 1 } else { 0 };

        self.set_encoder_ctl_request(
            ffi::OPUS_SET_VBR_CONSTRAINT_REQUEST,
            if_vbr_shall_be_enabled,
        )
        .map(|_| ())
    }

    /// Determine if constrained VBR is enabled in the encoder.
    pub fn vbr_constraint(&self) -> Result<bool> {
        self.encoder_ctl_request(ffi::OPUS_GET_VBR_CONSTRAINT_REQUEST)
            .map(|b| b == 1)
    }

    /// Enables variable bitrate (VBR) in the encoder.
    ///
    /// The configured bitrate may not be met exactly because frames must be an
    /// integer number of bytes in length.
    pub fn enable_vbr(&mut self) -> Result<()> {
        self.set_vbr(true)
    }

    /// Disables variable bitrate (VBR) in the encoder.
    ///
    /// The configured bitrate may not be met exactly because frames must be an
    /// integer number of bytes in length.
    pub fn disable_vbr(&mut self) -> Result<()> {
        self.set_vbr(false)
    }

    /// Sets variable bitrate (VBR) in the encoder.
    ///
    /// The configured bitrate may not be met exactly because frames must be an
    /// integer number of bytes in length.
    pub fn set_vbr(&mut self, enable: bool) -> Result<()> {
        let if_vbr_shall_be_enabled = if enable { 1 } else { 0 };

        self.set_encoder_ctl_request(ffi::OPUS_SET_VBR_REQUEST, if_vbr_shall_be_enabled)
            .map(|_| ())
    }

    /// Determine if variable bitrate (VBR) is enabled in the encoder.
    pub fn vbr(&self) -> Result<bool> {
        self.encoder_ctl_request(ffi::OPUS_GET_VBR_REQUEST)
            .map(|b| b == 1)
    }

    /// Configures the encoder's use of inband forward error correction (FEC).
    pub fn set_inband_fec(&mut self, enable: bool) -> Result<()> {
        let if_inband_fec_shall_be_enabled = if enable { 1 } else { 0 };

        self.set_encoder_ctl_request(
            ffi::OPUS_SET_INBAND_FEC_REQUEST,
            if_inband_fec_shall_be_enabled,
        )
        .map(|_| ())
    }

    /// Enables the encoder's use of inband forward error correction (FEC).
    pub fn enable_inband_fec(&mut self) -> Result<()> {
        self.set_inband_fec(true)
    }

    /// Disables the encoder's use of inband forward error correction (FEC).
    pub fn disable_inband_fec(&mut self) -> Result<()> {
        self.set_inband_fec(false)
    }

    /// Gets encoder's configured use of inband forward error correction.
    pub fn inband_fec(&self) -> Result<bool> {
        self.encoder_ctl_request(ffi::OPUS_GET_INBAND_FEC_REQUEST)
            .map(|n| n == 1)
    }

    /// Gets the encoder's configured packet loss percentage.
    pub fn packet_loss_perc(&self) -> Result<u8> {
        self.encoder_ctl_request(ffi::OPUS_GET_PACKET_LOSS_PERC_REQUEST)
            .map(|n| n as u8)
    }

    /// Higher values trigger progressively more loss resistant behavior in the
    /// encoder at the expense of quality at a given bitrate in the absence of
    /// packet loss, but greater quality under loss.
    ///
    /// Configures the encoder's expected packet loss percentage.
    pub fn set_packet_loss_perc(&mut self, percentage: u8) -> Result<()> {
        self.set_encoder_ctl_request(
            ffi::OPUS_SET_PACKET_LOSS_PERC_REQUEST,
            i32::from(percentage),
        )
        .map(|_| ())
    }

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
    pub fn lookahead(&self) -> Result<u32> {
        self.encoder_ctl_request(ffi::OPUS_GET_LOOKAHEAD_REQUEST)
            .map(|n| n as u32)
    }

    /// Configures mono/stereo forcing in the encoder.
    ///
    /// This can force the encoder to produce packets encoded as either
    /// mono or stereo, regardless of the format of the input audio.
    /// This is useful when the caller knows that the input signal is
    /// currently a mono source embedded in a stereo stream.
    pub fn set_force_channels(&mut self, channels: Channels) -> Result<()> {
        self.set_encoder_ctl_request(ffi::OPUS_SET_FORCE_CHANNELS_REQUEST, channels as i32)
            .map(|_| ())
    }

    /// Gets the encoder's forced channel configuration.
    pub fn force_channels(&self) -> Result<Channels> {
        self.encoder_ctl_request(ffi::OPUS_GET_FORCE_CHANNELS_REQUEST)
            .and_then(Channels::try_from)
    }

    /// Gets the encoder's configured maximum allowed bandpass.
    pub fn max_bandwidth(&self) -> Result<Bandwidth> {
        self.encoder_ctl_request(ffi::OPUS_GET_MAX_BANDWIDTH_REQUEST)
            .and_then(Bandwidth::try_from)
    }

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
    pub fn set_max_bandwidth(&mut self, bandwidth: Bandwidth) -> Result<()> {
        self.set_encoder_ctl_request(ffi::OPUS_SET_MAX_BANDWIDTH_REQUEST, bandwidth as i32)
    }

    /// Gets the encoder's configured prediction status.
    pub fn prediction_disabled(&self) -> Result<bool> {
        self.encoder_ctl_request(ffi::OPUS_GET_PREDICTION_DISABLED_REQUEST)
            .map(|n| n == 1)
    }

    /// If set `prediction_disabled` to `true`, disables almost all use of
    /// prediction, making frames almost completely independent.
    ///
    /// This reduces quality.
    pub fn set_prediction_disabled(&mut self, prediction_disabled: bool) -> Result<()> {
        let prediction_disabled = if prediction_disabled { 1 } else { 0 };

        self.set_encoder_ctl_request(
            ffi::OPUS_SET_PREDICTION_DISABLED_REQUEST,
            prediction_disabled,
        )
        .map(|_| ())
    }

    /// Gets the encoder's configured signal type.
    pub fn signal(&self) -> Result<Signal> {
        self.encoder_ctl_request(ffi::OPUS_GET_SIGNAL_REQUEST)
            .and_then(Signal::try_from)
    }

    /// Configures the type of signal being encoded.
    ///
    /// This is a hint which helps the encoder's mode selection.
    pub fn set_signal(&mut self, signal: Signal) -> Result<()> {
        self.set_encoder_ctl_request(ffi::OPUS_SET_SIGNAL_REQUEST, signal as i32)
            .map(|_| ())
    }

    /// Gets the encoder's configured bandpass.
    pub fn bandwidth(&self) -> Result<Bandwidth> {
        self.encoder_ctl_request(ffi::OPUS_GET_BANDWIDTH_REQUEST)
            .and_then(Bandwidth::try_from)
    }

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
    pub fn set_bandwidth(&mut self, bandwidth: Bandwidth) -> Result<()> {
        self.set_encoder_ctl_request(ffi::OPUS_SET_BANDWIDTH_REQUEST, bandwidth as i32)
    }

    /// Gets encoder's configured use of discontinuous transmission.
    pub fn dtx(&self) -> Result<bool> {
        self.encoder_ctl_request(ffi::OPUS_GET_DTX_REQUEST)
            .map(|n| n == 1)
    }

    /// Configures the encoder's use of discontinuous transmission (DTX).
    pub fn set_dtx(&mut self, dtx: bool) -> Result<()> {
        let dtx_shall_be_enabled = if dtx { 1 } else { 0 };

        self.set_encoder_ctl_request(ffi::OPUS_SET_DTX_REQUEST, dtx_shall_be_enabled)
            .map(|_| ())
    }

    /// Enables the encoder's use of discontinuous transmission (DTX).
    pub fn enable_dtx(&mut self) -> Result<()> {
        self.set_dtx(true)
    }

    /// Disables the encoder's use of discontinuous transmission (DTX).
    pub fn disable_dtx(&mut self) -> Result<()> {
        self.set_dtx(false)
    }

    /// Gets the encoder's configured signal depth.
    pub fn lsb_depth(&self) -> Result<u8> {
        self.encoder_ctl_request(ffi::OPUS_GET_LSB_DEPTH_REQUEST)
            .map(|n| n as u8)
    }

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
    pub fn set_lsb_depth(&mut self, lsb_depth: u8) -> Result<()> {
        self.set_encoder_ctl_request(ffi::OPUS_SET_LSB_DEPTH_REQUEST, i32::from(lsb_depth))
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::{Application, Bandwidth, Bitrate, Channels, Encoder, SampleRate, Signal, ffi};
    use crate::coder::GenericCtl;

    fn assert_send<T: Send>() {}
    fn assert_debug<T: std::fmt::Debug>() {}

    /// `audiopus` marks both coders `Send` (and leaves them `!Sync`).
    #[test]
    fn encoder_is_send_and_debug() {
        assert_send::<Encoder>();
        assert_debug::<Encoder>();
    }

    /// The `GenericCtl` surface must be implemented by `Encoder`.
    #[test]
    fn encoder_implements_generic_ctl() {
        fn assert_generic_ctl<T: GenericCtl>() {}
        assert_generic_ctl::<Encoder>();
    }

    /// Compile-time check of every public `Encoder` signature against the
    /// `audiopus` 0.3.0-rc.0 inventory (names, argument types, `&self`/`&mut
    /// self`, return types).  The function is never called: the codec internals
    /// are still placeholders, so these calls cannot run yet — they only have to
    /// type-check.
    #[allow(dead_code)]
    #[allow(unused_mut)]
    fn encoder_signature_check(mut encoder: Encoder) {
        let _ = Encoder::new(SampleRate::Hz48000, Channels::Stereo, Application::Audio);
        let _ = encoder.encoder_ctl_request(ffi::OPUS_GET_BITRATE_REQUEST);
        let _ = encoder.set_encoder_ctl_request(ffi::OPUS_SET_BITRATE_REQUEST, 64000);
        let _ = encoder.encode(&[0i16], &mut [0u8]);
        let _ = encoder.encode_float(&[0.0f32], &mut [0u8]);

        let _ = encoder.complexity();
        let _ = encoder.set_complexity(10);
        let _ = encoder.application();
        let _ = encoder.set_application(Application::Audio);
        let _ = encoder.bitrate();
        let _ = encoder.set_bitrate(Bitrate::Auto);
        let _ = encoder.enable_vbr_constraint();
        let _ = encoder.disable_vbr_constraint();
        let _ = encoder.set_vbr_constraint(true);
        let _ = encoder.vbr_constraint();
        let _ = encoder.enable_vbr();
        let _ = encoder.disable_vbr();
        let _ = encoder.set_vbr(true);
        let _ = encoder.vbr();
        let _ = encoder.set_inband_fec(true);
        let _ = encoder.enable_inband_fec();
        let _ = encoder.disable_inband_fec();
        let _ = encoder.inband_fec();
        let _ = encoder.packet_loss_perc();
        let _ = encoder.set_packet_loss_perc(0);
        let _ = encoder.lookahead();
        let _ = encoder.set_force_channels(Channels::Auto);
        let _ = encoder.force_channels();
        let _ = encoder.max_bandwidth();
        let _ = encoder.set_max_bandwidth(Bandwidth::Auto);
        let _ = encoder.prediction_disabled();
        let _ = encoder.set_prediction_disabled(true);
        let _ = encoder.signal();
        let _ = encoder.set_signal(Signal::Auto);
        let _ = encoder.bandwidth();
        let _ = encoder.set_bandwidth(Bandwidth::Auto);
        let _ = encoder.dtx();
        let _ = encoder.set_dtx(true);
        let _ = encoder.enable_dtx();
        let _ = encoder.disable_dtx();
        let _ = encoder.lsb_depth();
        let _ = encoder.set_lsb_depth(16);

        // `GenericCtl` (the trait is in scope).
        let _ = encoder.final_range();
        let _ = encoder.phase_inversion_disabled();
        let _ = encoder.set_phase_inversion_disabled(true);
        let _ = encoder.sample_rate();
        let _ = encoder.reset_state();
    }
}

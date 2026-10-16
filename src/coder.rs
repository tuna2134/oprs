//! The encoder/decoder front ends.
//!
//! This is a compatible reimplementation of `audiopus::coder` (0.3.0-rc.0):
//! [`Encoder`], [`Decoder`], [`size`] and the [`GenericCtl`] trait keep the
//! same paths, signatures and validation quirks, but the work is done by the
//! pure-Rust port in [`crate::opus::encoder`] / [`crate::opus::decoder`].

use crate::{Error, SampleRate};

pub use self::{
    decoder::{Decoder, size},
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

#[cfg(test)]
mod tests {
    use super::GenericCtl;
    use crate::{Error, SampleRate};

    /// Compile-time check of the `GenericCtl` signatures; the trait spells its
    /// returns as `Result<T, Error>` (the `std` `Result`), not as the crate
    /// alias, exactly like `audiopus`.
    #[allow(dead_code)]
    fn generic_ctl_signature_check<T: GenericCtl>(ctl: &mut T) {
        let _: Result<u32, Error> = ctl.final_range();
        let _: Result<bool, Error> = ctl.phase_inversion_disabled();
        let _: Result<(), Error> = ctl.set_phase_inversion_disabled(true);
        let _: Result<SampleRate, Error> = ctl.sample_rate();
        let _: Result<(), Error> = ctl.reset_state();
    }
}

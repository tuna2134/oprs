//! Soft clipping of `f32` signals.
//!
//! This is a compatible reimplementation of `audiopus::softclip` (0.3.0-rc.0):
//! same type, same signatures, same behaviour — including that [`SoftClip::apply`]
//! discards the underlying result and always returns `Ok(())`.

use crate::{Channels, MutSignals, Result};

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
    ///
    /// Like `audiopus`, the return value of the underlying
    /// `opus_pcm_soft_clip` is ignored and `Ok(())` is always returned.
    pub fn apply(&mut self, mut signals: MutSignals<'_, f32>) -> Result<()> {
        let frame_size = signals.i32_len() / self.channels as i32;
        let channels = self.channels as i32;

        crate::opus::packet::opus_pcm_soft_clip(
            signals.as_mut_slice(),
            frame_size,
            channels,
            &mut self.memory,
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::SoftClip;
    use crate::{Channels, MutSignals};
    use std::convert::TryFrom;

    #[test]
    fn soft_clip() {
        let mut soft_clip = SoftClip::new(Channels::Stereo);

        let mut signals: Vec<f32> = vec![];
        soft_clip.apply((&mut signals).try_into().unwrap()).unwrap();

        signals.push(5.0);
        signals.push(-5000.3453);
        soft_clip.apply((&mut signals).try_into().unwrap()).unwrap();

        assert!(signals[0] <= 1.0 && signals[0] >= -1.0);
        assert!(signals[1] <= 1.0 && signals[1] >= -1.0);
    }

    #[test]
    fn soft_clip_clones_its_memory() {
        let mut soft_clip = SoftClip::new(Channels::Mono);
        let mut signals = vec![2.0f32, -3.0, 0.5];
        soft_clip
            .apply(MutSignals::try_from(&mut signals[..]).unwrap())
            .unwrap();
        assert!(signals.iter().all(|s| *s >= -1.0 && *s <= 1.0));

        let clone = soft_clip.clone();
        assert!(format!("{:?}", clone).contains("SoftClip"));
    }

    #[test]
    fn apply_always_succeeds() {
        // `Channels::Auto` (`-1000`) is nonsense here, but `apply` still reports
        // success, exactly like `audiopus`.
        let mut soft_clip = SoftClip::new(Channels::Auto);
        let mut signals = vec![0.0f32; 4];
        assert!(
            soft_clip
                .apply(MutSignals::try_from(&mut signals[..]).unwrap())
                .is_ok()
        );
    }
}

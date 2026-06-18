// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Non-bitstream runtime configuration for the encoder.

use core::fmt;

use splot_parallel::ThreadCount;

/// Runtime speed preset for future encoder decisions.
///
/// Lower values reserve room for slower, more exhaustive decisions; higher
/// values reserve room for faster decisions. The current encoder does not emit
/// packets, so the preset is stored as runtime policy only.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SpeedPreset(u8);

impl SpeedPreset {
    /// Slowest accepted preset value.
    pub const MIN_VALUE: u8 = 0;
    /// Fastest accepted preset value.
    pub const MAX_VALUE: u8 = 10;
    /// Default preset value.
    pub const DEFAULT_VALUE: u8 = 6;

    /// Creates a speed preset from its numeric value.
    ///
    /// # Errors
    /// Returns [`SpeedPresetError`] when `value` is outside the accepted range.
    pub const fn try_from_u8(value: u8) -> Result<Self, SpeedPresetError> {
        if value <= Self::MAX_VALUE {
            Ok(Self(value))
        } else {
            Err(SpeedPresetError { value })
        }
    }

    /// Returns the numeric preset value.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

impl Default for SpeedPreset {
    fn default() -> Self {
        Self(Self::DEFAULT_VALUE)
    }
}

impl TryFrom<u8> for SpeedPreset {
    type Error = SpeedPresetError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Self::try_from_u8(value)
    }
}

/// Error returned for unsupported encoder speed presets.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpeedPresetError {
    value: u8,
}

impl SpeedPresetError {
    /// Returns the unsupported numeric preset value.
    #[must_use]
    pub const fn value(self) -> u8 {
        self.value
    }
}

impl fmt::Display for SpeedPresetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "encoder speed preset {} is outside the supported range {}..={}",
            self.value,
            SpeedPreset::MIN_VALUE,
            SpeedPreset::MAX_VALUE
        )
    }
}

impl std::error::Error for SpeedPresetError {}

/// Runtime (non-bitstream) encoder knobs.
///
/// This is deliberately separate from [`crate::EncoderConfig`], which holds
/// only bitstream-affecting settings. Thread count and speed preset must never
/// influence whether emitted syntax is valid.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub struct EncoderRuntimeConfig {
    /// Worker-thread policy. Defaults to [`ThreadCount::Auto`].
    pub thread_count: ThreadCount,
    /// Runtime speed preset. Defaults to [`SpeedPreset::DEFAULT_VALUE`].
    pub speed_preset: SpeedPreset,
}

impl EncoderRuntimeConfig {
    /// Builds a runtime config with the given thread-count policy.
    #[must_use]
    pub fn new(thread_count: ThreadCount) -> Self {
        Self {
            thread_count,
            speed_preset: SpeedPreset::default(),
        }
    }

    /// Returns a copy of this runtime config with a different speed preset.
    #[must_use]
    pub const fn with_speed_preset(mut self, speed_preset: SpeedPreset) -> Self {
        self.speed_preset = speed_preset;
        self
    }

    /// Returns the configured runtime speed preset.
    #[must_use]
    pub const fn speed_preset(self) -> SpeedPreset {
        self.speed_preset
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn speed_preset_accepts_documented_range() {
        assert_eq!(
            SpeedPreset::try_from_u8(SpeedPreset::MIN_VALUE)
                .unwrap()
                .get(),
            SpeedPreset::MIN_VALUE
        );
        assert_eq!(
            SpeedPreset::try_from_u8(SpeedPreset::MAX_VALUE)
                .unwrap()
                .get(),
            SpeedPreset::MAX_VALUE
        );
    }

    #[test]
    fn speed_preset_rejects_out_of_range_value() {
        let err = SpeedPreset::try_from_u8(SpeedPreset::MAX_VALUE + 1).unwrap_err();
        assert_eq!(err.value(), SpeedPreset::MAX_VALUE + 1);
        assert!(err.to_string().contains("supported range 0..=10"));
    }

    #[test]
    fn runtime_config_defaults_and_with_speed_preserve_thread_policy() {
        let runtime = EncoderRuntimeConfig::new(ThreadCount::from(3_usize));
        assert_eq!(runtime.thread_count, ThreadCount::from(3_usize));
        assert_eq!(runtime.speed_preset(), SpeedPreset::default());

        let speed = SpeedPreset::try_from_u8(2).unwrap();
        let runtime = runtime.with_speed_preset(speed);
        assert_eq!(runtime.thread_count, ThreadCount::from(3_usize));
        assert_eq!(runtime.speed_preset(), speed);
    }
}

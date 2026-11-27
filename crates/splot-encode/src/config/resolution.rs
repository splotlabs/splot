// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use std::collections::BTreeMap;

use super::Config;
use crate::error::Result;
use crate::intra_frame::check_frame_setup;
use crate::preset::{PresetContext, PresetField, PresetParams, ResolutionClass};

/// The source of a resolved preset value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PresetOrigin {
    /// The selected speed preset supplies the value.
    Preset,
    /// The screen-content row supplies a value that differs from the speed preset.
    ScreenContent,
    /// The tune supplies the value.
    Tune,
    /// Fast decode supplies the value.
    FastDecode,
    /// An explicit override supplies the value.
    Explicit,
    /// Lossless coding forces the value.
    Lossless,
}

/// A resolved preset row.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ResolvedPreset {
    /// The resolved preset values.
    pub params: PresetParams,
    base: &'static PresetParams,
    screen_base: Option<&'static PresetParams>,
    overrides: BTreeMap<String, PresetOrigin>,
}

impl ResolvedPreset {
    /// Returns the source of the field value in this context.
    #[must_use]
    pub fn origin(&self, field: &PresetField, context: PresetContext) -> PresetOrigin {
        if let Some(&origin) = self.overrides.get(field.name) {
            origin
        } else if self
            .screen_base
            .is_some_and(|screen| field.value(screen, context) != field.value(self.base, context))
        {
            PresetOrigin::ScreenContent
        } else {
            PresetOrigin::Preset
        }
    }
}

/// Effective configuration before the encoder receives frame samples.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ResolvedConfig {
    /// The frame width in luma samples after resize.
    pub coded_width: u32,
    /// The frame height in luma samples after resize.
    pub coded_height: u32,
    /// The resolution class of the coded frame.
    pub resolution: ResolutionClass,
    /// The preset row the encoder uses before it checks the first frame.
    pub normal: ResolvedPreset,
    /// The preset row the frame coder can select when it detects screen content.
    ///
    /// This field is empty when screen detection is off. The first frame determines
    /// whether the frame coder selects this row. Sequence setup, lookahead and GOP planning use the normal row.
    pub screen: Option<ResolvedPreset>,
}

impl Config {
    /// Validates the configuration and resolves both possible preset rows.
    ///
    /// Frame kind, coefficient level and content detection depend on the encode.
    /// Each row contains values for all preset contexts.
    ///
    /// # Errors
    /// Returns the same configuration errors as [`Self::validate`].
    /// Returns a tile layout or coding error if the encoder cannot set up the coded frame.
    pub fn resolve(&self) -> Result<ResolvedConfig> {
        let mut overrides = BTreeMap::new();
        let [normal, screen] = self.validated_presets(|key, origin| {
            overrides.insert(key.to_owned(), origin);
        })?;
        let coded = self.coded();
        check_frame_setup(&coded, &normal)?;
        let base = self.preset.params();
        Ok(ResolvedConfig {
            coded_width: coded.width,
            coded_height: coded.height,
            resolution: ResolutionClass::of(coded.width, coded.height),
            screen: normal.screen_detect.then(|| ResolvedPreset {
                params: screen,
                base,
                screen_base: Some(self.preset.screen_params()),
                overrides: overrides.clone(),
            }),
            normal: ResolvedPreset {
                params: normal,
                base,
                screen_base: None,
                overrides,
            },
        })
    }
}

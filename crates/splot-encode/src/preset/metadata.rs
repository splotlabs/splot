// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::{FieldValue, PresetContext, PresetParams};

/// A group in the preset table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PresetGroup {
    /// Block partition search.
    Partition,
    /// Intra prediction.
    Intra,
    /// Transforms and quantization.
    Transform,
    /// Inter prediction and frame references.
    Inter,
    /// Loop filters.
    Filters,
    /// Screen content tools.
    ScreenContent,
    /// Lossless tools.
    Lossless,
    /// Group of pictures and lookahead.
    Gop,
    /// Mode decision.
    ModeDecision,
    /// Parallel encoding.
    Threading,
}

impl PresetGroup {
    /// The stable name of this group.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Partition => "partition",
            Self::Intra => "intra",
            Self::Transform => "transform",
            Self::Inter => "inter",
            Self::Filters => "filters",
            Self::ScreenContent => "screen_content",
            Self::Lossless => "lossless",
            Self::Gop => "gop",
            Self::ModeDecision => "mode_decision",
            Self::Threading => "threading",
        }
    }
}

/// The type of one preset value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresetValueKind {
    /// A Boolean flag.
    Bool,
    /// An unsigned 8-bit integer.
    U8,
    /// A signed 8-bit integer.
    I8,
    /// An unsigned 16-bit integer.
    U16,
    /// Three Boolean flags.
    BoolTriple,
    /// Three unsigned 8-bit integers.
    U8Triple,
}

/// How one preset field varies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresetFieldShape {
    /// One value for all contexts.
    Scalar(PresetValueKind),
    /// One stored value per context. Overrides accept one value or one per frame kind.
    Contextual(PresetValueKind),
}

/// The value of a field in one context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresetFieldValue {
    /// A Boolean flag.
    Bool(bool),
    /// An unsigned 8-bit integer.
    U8(u8),
    /// A signed 8-bit integer.
    I8(i8),
    /// An unsigned 16-bit integer.
    U16(u16),
    /// Three Boolean flags.
    BoolTriple([bool; 3]),
    /// Three unsigned 8-bit integers.
    U8Triple([u8; 3]),
}

/// Inclusive bounds for an integer or each member of an integer triple.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresetIntegerRange {
    /// The smallest allowed value.
    pub min: i32,
    /// The largest allowed value.
    pub max: i32,
}

impl PresetIntegerRange {
    pub(super) const fn new(min: i32, max: i32) -> Self {
        Self { min, max }
    }
}

/// A field in the preset table.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct PresetField {
    /// The exact key that [`PresetParams::set`] accepts.
    pub name: &'static str,
    /// The field's group in the preset table.
    pub group: PresetGroup,
    /// The field's type and contextual behavior.
    pub shape: PresetFieldShape,
    /// The field's purpose and any conditions that affect its use.
    pub help: &'static str,
    /// The input format that [`PresetParams::set`] accepts.
    pub expected: &'static str,
    /// Static integer bounds. [`PresetParams::validate`] checks configuration-dependent bounds.
    pub integer_range: Option<PresetIntegerRange>,
    read: fn(&PresetParams, PresetContext) -> PresetFieldValue,
}

impl PresetField {
    pub(super) const fn new<T: FieldValue>(
        name: &'static str,
        group: PresetGroup,
        help: &'static str,
        range: Option<PresetIntegerRange>,
        _: fn(&PresetParams) -> &T,
        read: fn(&PresetParams, PresetContext) -> PresetFieldValue,
    ) -> Self {
        let integer_range = match range {
            Some(range) => Some(range),
            None => T::INTEGER_RANGE,
        };
        Self {
            name,
            group,
            shape: T::SHAPE,
            help,
            expected: T::EXPECTED,
            integer_range,
            read,
        }
    }

    /// Reads this field from `params` in the specified context.
    #[must_use]
    pub fn value(&self, params: &PresetParams, context: PresetContext) -> PresetFieldValue {
        (self.read)(params, context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preset::{FrameKind, PRESETS, PresetError, ResolutionClass};

    #[test]
    fn descriptors_match_the_setter() -> core::result::Result<(), PresetError> {
        use PresetFieldValue::{Bool, BoolTriple, I8, U8, U8Triple, U16};

        let fields = PresetParams::FIELD_DESCRIPTORS;
        assert_eq!(fields.len(), PresetParams::FIELDS.len());
        assert!(PresetParams::field("unknown").is_none());

        for (field, &name) in fields.iter().zip(PresetParams::FIELDS) {
            assert_eq!(field.name, name);
            assert!(!field.help.is_empty());
            assert_eq!(PresetParams::field(name).map(|f| f.name), Some(name));
            for preset in PRESETS {
                let mut overridden = preset;
                for context in PresetContext::ALL {
                    let value = field.value(&preset, context);
                    let (kind, encoded, numbers) = match value {
                        Bool(value) => (PresetValueKind::Bool, value.to_string(), vec![]),
                        I8(value) => (
                            PresetValueKind::I8,
                            value.to_string(),
                            vec![i32::from(value)],
                        ),
                        U8(value) => (
                            PresetValueKind::U8,
                            value.to_string(),
                            vec![i32::from(value)],
                        ),
                        U16(value) => (
                            PresetValueKind::U16,
                            value.to_string(),
                            vec![i32::from(value)],
                        ),
                        BoolTriple([a, b, c]) => {
                            (PresetValueKind::BoolTriple, format!("{a},{b},{c}"), vec![])
                        }
                        U8Triple([a, b, c]) => (
                            PresetValueKind::U8Triple,
                            format!("{a},{b},{c}"),
                            vec![i32::from(a), i32::from(b), i32::from(c)],
                        ),
                    };
                    assert!(matches!(
                        field.shape,
                        PresetFieldShape::Scalar(k) | PresetFieldShape::Contextual(k) if k == kind
                    ));
                    assert_eq!(field.integer_range.is_none(), numbers.is_empty(), "{name}");
                    if let Some(range) = field.integer_range {
                        assert!(
                            numbers.iter().all(|v| (range.min..=range.max).contains(v)),
                            "{name} {value:?}"
                        );
                    }
                    overridden.set(name, &encoded)?;
                    assert_eq!(field.value(&overridden, context), value, "{name}");
                }
            }
        }
        Ok(())
    }

    #[test]
    fn contextual_values_preserve_preset_and_override_cells()
    -> core::result::Result<(), PresetError> {
        let mut preset = PRESETS[6];
        let field = PresetParams::field("key_qindex_boost").unwrap();
        assert_eq!(
            field.shape,
            PresetFieldShape::Contextual(PresetValueKind::U8)
        );
        let sd_key = PresetContext::default();
        let hd_key = PresetContext {
            resolution: ResolutionClass::Hd,
            ..sd_key
        };
        assert_eq!(field.value(&preset, sd_key), PresetFieldValue::U8(4));
        assert_eq!(field.value(&preset, hd_key), PresetFieldValue::U8(0));

        preset.set("key_qindex_boost", "5,6,7,8")?;
        for context in PresetContext::ALL {
            assert_eq!(
                field.value(&preset, context),
                PresetFieldValue::U8(match context.frame {
                    FrameKind::Key => 5,
                    FrameKind::Base => 6,
                    FrameKind::Mid => 7,
                    FrameKind::Leaf => 8,
                })
            );
        }
        Ok(())
    }
}

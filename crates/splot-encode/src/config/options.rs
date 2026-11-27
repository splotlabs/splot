// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Descriptions of public encoder configuration fields.
//!
//! The keys identify fields within the encoder option schema. Omitted settings
//! use [`Config::new`] defaults. Source fields take values from the input when
//! available. Absent source metadata uses the source field defaults. An empty
//! first-pass list selects one-pass encoding.
//! Use [`Config::validate`] to check a populated configuration. Descriptions
//! include input constraints that depend on other fields or encoder state.
//! Each new `Config` field requires a corresponding entry in [`OPTIONS`].

use splot_recon::{BitDepth, PixelFormat};

use crate::rate_control::{Crf, MAX_BITRATE_KBPS, MAX_BUFFER_MS, max_qindex};

use super::{Config, MAX_FRAME_DIMENSION, Preset, TileLayout};

/// Where a field obtains its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionClass {
    /// The caller selects a value or uses the encoder default.
    Setting,
    /// The value describes the input samples or source metadata.
    Source,
    /// The value comes from a separate encode pass.
    FirstPass,
    /// The field holds an ordered list of preset overrides.
    PresetOverrides,
    /// The record contains fields from more than one class.
    Mixed,
}

/// An integer limit for an encoder option.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limit {
    /// The same limit for every configuration.
    Fixed(u64),
    /// The maximum quantizer index at the source bit depth.
    QuantizerIndex,
    /// The value of the named field in the same record.
    Field(&'static str),
}

impl Limit {
    /// Returns the limit when the bit depth is known.
    ///
    /// A limit that depends on a sibling field requires the caller to supply
    /// the value of that field.
    #[must_use]
    pub fn at_depth(self, depth: BitDepth) -> Option<u64> {
        match self {
            Self::Fixed(value) => Some(value),
            Self::QuantizerIndex => Some(u64::from(max_qindex(depth))),
            Self::Field(_) => None,
        }
    }
}

/// The input shape of a public field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueShape {
    /// A Boolean switch.
    Boolean,
    /// An inclusive integer range and a unit, when applicable.
    Integer {
        /// The minimum value.
        min: u64,
        /// The maximum value.
        max: Limit,
        /// The unit of the value.
        unit: Option<&'static str>,
    },
    /// A positive ratio with two unsigned 32-bit terms.
    Ratio,
    /// A byte payload.
    Bytes,
    /// An optional value with an explicit absent state.
    Optional(&'static Self),
    /// A record with named members.
    Record(&'static [OptionDescription]),
    /// A tagged choice. Some variants take named fields.
    Choice(&'static [VariantDescription]),
    /// A list of values.
    List(&'static Self),
    /// An ordered list of key and value pairs accepted by [`crate::PresetParams::set`].
    Overrides,
}

/// A public field in the encoder option schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OptionDescription {
    /// The canonical key within the enclosing record.
    pub key: &'static str,
    /// The source of the field value.
    pub class: OptionClass,
    /// The input type and its limits.
    pub shape: ValueShape,
    /// A short description of the field.
    pub help: &'static str,
    /// Conditions that affect availability or validation.
    pub applicability: &'static str,
}

/// A variant in a tagged encoder option.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VariantDescription {
    /// The canonical variant key.
    pub key: &'static str,
    /// The fields of the variant.
    pub fields: &'static [OptionDescription],
    /// A short description of the variant.
    pub help: &'static str,
}

macro_rules! options {
    ($($field:ident, $class:ident, $shape:expr, $help:literal $(, $conditions:literal)?;)*) => {
        &[$(OptionDescription {
            key: stringify!($field),
            class: OptionClass::$class,
            shape: $shape,
            help: $help,
            applicability: concat!($($conditions)?),
        },)*]
    };
}

macro_rules! config_options {
    ($($field:ident, $class:ident, $shape:expr, $help:literal $(, $conditions:literal)?;)*) => {
        /// The public fields of [`Config`].
        pub const OPTIONS: &[OptionDescription] =
            options!($($field, $class, $shape, $help $(, $conditions)?;)*);

        const _: fn(&Config) = |config| {
            let Config { $($field: _,)* } = config;
        };
    };
}

const fn integer(min: u64, max: u64, unit: &'static str) -> ValueShape {
    ValueShape::Integer {
        min,
        max: Limit::Fixed(max),
        unit: if unit.is_empty() { None } else { Some(unit) },
    }
}

const fn variant(
    key: &'static str,
    fields: &'static [OptionDescription],
    help: &'static str,
) -> VariantDescription {
    VariantDescription { key, fields, help }
}

const U8_MAX: u64 = u8::MAX as u64;
const U16_MAX: u64 = u16::MAX as u64;
const U32_MAX: u64 = u32::MAX as u64;
const CRF: ValueShape = integer(1, Crf::MAX as u64, "");
const BITRATE: ValueShape = integer(1, MAX_BITRATE_KBPS as u64, "kilobits per second");
const BUFFER_MS: ValueShape = integer(1, MAX_BUFFER_MS as u64, "milliseconds");
const INITIAL_MS: ValueShape = ValueShape::Integer {
    min: 0,
    max: Limit::Field("buffer_ms"),
    unit: Some("milliseconds"),
};

/// The rate-control variants and their inputs.
pub const RATE_CONTROL_MODES: &[VariantDescription] = &[
    variant(
        "constant_qp",
        options! {
            qp, Setting, ValueShape::Integer { min: 0, max: Limit::QuantizerIndex, unit: None },
                "Set the frame quantizer index. Zero selects lossless coding.";
        },
        "Use the same quantizer index for every frame.",
    ),
    variant(
        "crf",
        options! { crf, Setting, CRF, "Set the rate factor. A higher value uses fewer bits."; },
        "Target constant quality.",
    ),
    variant(
        "capped_crf",
        options! {
            crf, Setting, CRF, "Set the rate factor. A higher value uses fewer bits.";
            max_bitrate_kbps, Setting, BITRATE, "Set the maximum bitrate.";
            buffer_ms, Setting, BUFFER_MS, "Set the decoder buffer size.";
            initial_ms, Setting, INITIAL_MS, "Set the initial decoder buffer level.";
        },
        "Target constant quality within a bitrate cap.",
    ),
    variant(
        "vbr",
        options! { bitrate_kbps, Setting, BITRATE, "Set the target average bitrate."; },
        "Vary the bitrate around a target average.",
    ),
    variant(
        "cbr",
        options! {
            bitrate_kbps, Setting, BITRATE, "Set the target bitrate.";
            buffer_ms, Setting, BUFFER_MS, "Set the decoder buffer size.";
            initial_ms, Setting, INITIAL_MS, "Set the initial decoder buffer level.";
        },
        "Target a constant bitrate with a decoder buffer.",
    ),
];

const TILE_LOG2: ValueShape = integer(0, TileLayout::MAX_LOG2 as u64, "log2 of tile count");
const TILE_SIZES: ValueShape = ValueShape::List(&integer(1, U32_MAX, "superblocks"));

/// The tile layout variants and their inputs.
pub const TILE_LAYOUTS: &[VariantDescription] = &[
    variant(
        "uniform",
        options! {
            cols_log2, Setting, TILE_LOG2, "Set the number of tile columns.";
            rows_log2, Setting, TILE_LOG2, "Set the number of tile rows.";
        },
        "Use uniform spacing. The coded frame size can limit the number of tiles.",
    ),
    variant(
        "explicit",
        options! {
            widths, Setting, TILE_SIZES, "Set the tile column widths.";
            heights, Setting, TILE_SIZES, "Set the tile row heights.";
        },
        "Set individual tile sizes. A positive seq_tile_info requires uniform tiles.",
    ),
];

/// The supported tune values.
pub const TUNES: &[VariantDescription] = &[
    variant("psnr", &[], "Use squared error."),
    variant(
        "ssim",
        &[],
        "Use adaptive quantization based on local variance.",
    ),
    variant(
        "vq",
        &[],
        "Use adaptive quantization and retain source AC energy.",
    ),
];

const CHROMA: &[VariantDescription] = &[
    variant(PixelFormat::Monochrome.as_str(), &[], "Luma only."),
    variant(PixelFormat::Yuv420.as_str(), &[], "4:2:0 chroma."),
    variant(PixelFormat::Yuv422.as_str(), &[], "4:2:2 chroma."),
    variant(PixelFormat::Yuv444.as_str(), &[], "4:4:4 chroma."),
];

const CHROMA_SAMPLE_POSITIONS: &[VariantDescription] = &[
    variant("left", &[], "On a luma column."),
    variant("center", &[], "Between two luma columns."),
    variant("top_left", &[], "On the upper-left luma sample."),
    variant("top", &[], "Between the two upper luma samples."),
    variant("bottom_left", &[], "On the lower-left luma sample."),
    variant("bottom", &[], "Between the two lower luma samples."),
];

const CHROMATICITY: ValueShape = ValueShape::Record(options! {
    x, Source, integer(0, U16_MAX, "0.00002"), "CIE x coordinate.";
    y, Source, integer(0, U16_MAX, "0.00002"), "CIE y coordinate.";
});
const DISPLAY: ValueShape = ValueShape::Record(options! {
    primaries, Source, ValueShape::List(&CHROMATICITY), "Three display primaries in green, blue, red order.";
    white_point, Source, CHROMATICITY, "White point of the mastering display.";
    luminance_max, Source, integer(0, U32_MAX, "0.0001 cd/m²"), "Maximum mastering luminance.";
    luminance_min, Source, integer(0, U32_MAX, "0.0001 cd/m²"), "Minimum mastering luminance.";
});
const LIGHT_LEVEL: ValueShape = ValueShape::Record(options! {
    max_cll, Source, integer(0, U16_MAX, "cd/m²"), "Maximum sample light level. Zero means no bound.";
    max_fall, Source, integer(0, U16_MAX, "cd/m²"), "Maximum frame average light level. Zero means no bound.";
});
const CONTENT: ValueShape = ValueShape::Record(options! {
    color_primaries, Source, integer(0, U8_MAX, "H.273 code point"), "Colour primaries of the source.";
    transfer_characteristics, Source, integer(0, U8_MAX, "H.273 code point"), "Transfer characteristics of the source.";
    matrix_coefficients, Source, integer(0, U8_MAX, "H.273 code point"), "Matrix coefficients of the source.";
    full_range, Source, ValueShape::Boolean, "Declare that the source samples use full range.";
    chroma_sample_position, Source, ValueShape::Optional(&ValueShape::Choice(CHROMA_SAMPLE_POSITIONS)),
        "Declare the chroma sample position.", "4:2:0 supports every position. 4:2:2 supports left and center.";
    sample_aspect_ratio, Source, ValueShape::Optional(&ValueShape::Ratio), "Sample width divided by sample height.";
    timing_info, Setting, ValueShape::Boolean, "Signal the frame rate in the stream.";
    mastering_display, Source, ValueShape::Optional(&DISPLAY), "Mastering display colour volume.", "AV2 limits the coordinates and luminance.";
    content_light_level, Source, ValueShape::Optional(&LIGHT_LEVEL), "Upper bounds for the light level.";
});
const KEY_FRAME_METADATA: ValueShape = ValueShape::Record(options! {
    icc_profile, Setting, ValueShape::Optional(&ValueShape::Bytes), "Copy an ICC profile before each key frame.", "Use a payload with a 128-byte ICC header.";
    itut_t35, Setting, ValueShape::Optional(&ValueShape::Bytes), "Copy T.35 data before each key frame.", "Supply the country code and terminal provider code.";
    user_data_unregistered, Setting, ValueShape::Optional(&ValueShape::Bytes), "Copy unregistered user data before each key frame.", "Start the payload with a 16-byte UUID.";
});
const FRAME_HASH: ValueShape = ValueShape::Choice(&[
    variant("frame", &[], "Hash the Y, U and V samples together."),
    variant("per_plane", &[], "Hash each plane separately."),
]);
const FRAME_METADATA: ValueShape = ValueShape::Record(options! {
    timecode, Setting, ValueShape::Boolean, "Write a timecode before each output frame.", "The frame rate is at most 512 frames per second.";
    temporal_point_info, Setting, ValueShape::Boolean, "Write the presentation time before each output frame.", "Requires timing info.";
    scan_type, Setting, ValueShape::Boolean, "Mark each output frame as progressive.";
    banding_hints, Setting, ValueShape::Optional(&ValueShape::Bytes), "Copy banding hints before each output frame.", "Supply one metadata_banding_hints() padded to a byte.";
    decoded_frame_hash, Setting, ValueShape::Optional(&FRAME_HASH), "Write an MD5 hash after each output frame.";
});
const CROP_OFFSET: ValueShape = integer(0, U32_MAX, "coded luma samples");
const CROP: ValueShape = ValueShape::Record(options! {
    left, Setting, CROP_OFFSET, "Set the offset from the left edge.";
    right, Setting, CROP_OFFSET, "Set the offset from the right edge.";
    top, Setting, CROP_OFFSET, "Set the offset from the top edge.";
    bottom, Setting, CROP_OFFSET, "Set the offset from the bottom edge.";
});
const PROFILES: &[VariantDescription] = &[
    variant("0", &[], "Main_420_10_IP0."),
    variant("1", &[], "Main_420_10_IP1."),
    variant("2", &[], "Main_420_10_IP2."),
    variant("3", &[], "Main_422_10_IP1."),
    variant("4", &[], "Main_444_10_IP1."),
    variant("31", &[], "Configurable."),
];
const TIERS: &[VariantDescription] = &[
    variant("main", &[], "Main tier."),
    variant("high", &[], "High tier."),
];
const THREADS: ValueShape = ValueShape::Choice(&[
    variant("auto", &[], "Use the host worker count."),
    variant(
        "fixed",
        options! { count, Setting, integer(1, usize::MAX as u64, "workers"), "Set the number of worker threads."; },
        "Use a fixed worker count.",
    ),
]);

config_options! {
    width, Source, integer(1, MAX_FRAME_DIMENSION as u64, "luma samples"), "Width of the input frame.";
    height, Source, integer(1, MAX_FRAME_DIMENSION as u64, "luma samples"), "Height of the input frame.";
    bit_depth, Source, ValueShape::Choice(&[variant("8", &[], "8 bits."), variant("10", &[], "10 bits.")]),
        "Bit depth of the input samples.";
    chroma, Source, ValueShape::Choice(CHROMA), "Chroma format of the input frames.";
    frame_rate, Source, ValueShape::Ratio, "Frame rate in frames per second.";
    content, Mixed, CONTENT, "Describe the input samples and control timing metadata.";
    key_frame_metadata, Setting, KEY_FRAME_METADATA, "Set prefix metadata for each key frame.";
    frame_metadata, Setting, FRAME_METADATA, "Set prefix metadata for each output frame.";
    rate_control, Setting, ValueShape::Choice(RATE_CONTROL_MODES), "Select how the encoder controls quality and bitrate.";
    preset, Setting, integer(0, Preset::MAX as u64, ""), "Select a speed preset.";
    preset_overrides, PresetOverrides, ValueShape::Overrides, "Apply overrides in order after tune and fast decode.";
    threads, Setting, THREADS, "Set the worker-thread policy.";
    tiles, Setting, ValueShape::Choice(TILE_LAYOUTS), "Set the tile layout for each frame.";
    tile_groups, Setting, integer(1, U32_MAX, "groups per frame"), "Set the number of tile groups.",
        "The encoder limits the number of groups to the number of coded tiles.";
    disable_loopfilters_across_tiles, Setting, ValueShape::Boolean, "Stop the in-loop filters at tile edges.";
    keyint, Setting, integer(1, U32_MAX, "frames"), "Set the key-frame interval.";
    resize_denominator, Setting, ValueShape::Optional(&integer(
        *Config::RESIZE_DENOMINATORS.start() as u64, *Config::RESIZE_DENOMINATORS.end() as u64, "",
    )), "Encode each axis at eight divided by this value.", "Lossless coding requires the original size.";
    crop, Setting, ValueShape::Optional(&CROP), "Set the crop window after resize.",
        "The window must fit the coded frame. Its origin must align with the chroma grid.";
    profile, Setting, ValueShape::Optional(&ValueShape::Choice(PROFILES)), "Request a seq_profile_idc.",
        "The profile must allow the chroma format. Absent selects the lowest profile that allows it.";
    level, Setting, ValueShape::Optional(&integer(0, 31, "seq_level_idx")), "Request a seq_level_idx.",
        "The default is 31, without level limits. Absent selects the lowest level that holds. 22 to 30 are reserved. The encoder checks the picture size, tiles, display rate, header rate, reference frames and target bitrate; the caller is responsible for the other Annex A.4 limits.";
    tier, Setting, ValueShape::Optional(&ValueShape::Choice(TIERS)), "Request a seq_tier.",
        "The default is the main tier. The high tier needs level 4.0 or above. Absent tries the main tier first.";
    film_grain, Setting, ValueShape::Boolean, "Estimate a grain model from the source.",
        "Lossless coding does not support film grain.";
    cbr_padding, Setting, ValueShape::Boolean, "Add padding to maintain the target bitrate.", "Requires CBR.";
    tune, Setting, ValueShape::Choice(TUNES), "Select the quality metric.";
    fast_decode, Setting, ValueShape::Boolean, "Apply the fast-decode preset overrides.";
    multi_frame_header, Setting, ValueShape::Boolean, "Write a multi-frame header with each key frame.";
    first_pass, FirstPass, ValueShape::List(&integer(0, u64::MAX, "bits per frame")),
        "Supply the coded frame sizes from the first pass.", "Only VBR uses these values. An empty list selects one pass.";
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    fn check_fields(fields: &[OptionDescription]) {
        let mut keys = BTreeSet::new();
        for field in fields {
            assert!(keys.insert(field.key), "duplicate key {}", field.key);
            assert!(!field.help.is_empty());
            check_shape(field.shape);
        }
    }

    fn check_shape(shape: ValueShape) {
        match shape {
            ValueShape::Record(fields) => check_fields(fields),
            ValueShape::Optional(inner) | ValueShape::List(inner) => check_shape(*inner),
            ValueShape::Choice(variants) => {
                let mut keys = BTreeSet::new();
                for variant in variants {
                    assert!(keys.insert(variant.key), "duplicate key {}", variant.key);
                    assert!(!variant.help.is_empty());
                    check_fields(variant.fields);
                }
            }
            ValueShape::Integer {
                min,
                max: Limit::Fixed(max),
                ..
            } => assert!(min <= max),
            _ => {}
        }
    }

    #[test]
    fn option_keys_are_unique_within_each_scope() {
        check_fields(OPTIONS);
    }

    #[test]
    fn quantizer_bound_matches_config_validation_at_both_depths() {
        use crate::RateControlMode;

        let ValueShape::Integer { max, .. } = RATE_CONTROL_MODES[0].fields[0].shape else {
            panic!("Constant QP requires an integer index");
        };
        for depth in [BitDepth::Eight, BitDepth::Ten] {
            let qp = max.at_depth(depth).unwrap() as u16;
            let mut config = Config::new(64, 64);
            config.bit_depth = depth;
            config.rate_control = RateControlMode::ConstantQp { qp };
            assert!(config.validate().is_ok());
            config.rate_control = RateControlMode::ConstantQp { qp: qp + 1 };
            assert!(config.validate().is_err());
        }
    }
}

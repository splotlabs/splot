// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 `seg_info(numSegments)` segment-information syntax (AV2 v1.0.0 § 5.4.9).
//!
//! This is the reusable segmentation-feature parser shared by the sequence segment
//! config (§ 5.4.4) and the multi-frame header (§ 5.7). It reads syntax only and
//! performs no segmentation decoding; clipped feature values are stored as the spec
//! specifies (clipping is not, by itself, a conformance error).

use crate::bitio::BitReader;
use crate::error::Result;

/// `MAX_SEGMENTS`: number of segments allowed in segmentation (AV2 v1.0.0 § 3).
pub const MAX_SEGMENTS: usize = 16;

/// `SEG_LVL_MAX`: number of segment features (AV2 v1.0.0 § 3).
pub const SEG_LVL_MAX: usize = 3;

/// `Segmentation_Feature_Bits[SEG_LVL_MAX]` (AV2 v1.0.0 § 5.4.9): the number of value
/// bits coded for each enabled feature.
const SEGMENTATION_FEATURE_BITS: [u32; SEG_LVL_MAX] = [9, 0, 0];

/// `Segmentation_Feature_Signed[SEG_LVL_MAX]` (AV2 v1.0.0 § 5.4.9): whether each
/// feature value is signed (`su`) or unsigned (`f`).
const SEGMENTATION_FEATURE_SIGNED: [bool; SEG_LVL_MAX] = [true, false, false];

/// `MAXQ_BITS = MAXQ_8_BITS + 4 * MAXQ_OFFSET = 255 + 4 * 24` (AV2 v1.0.0 § 3): the
/// maximum quantizer irrespective of bit depth, used as the `SEG_LVL_ALT_Q` clip.
const MAXQ_BITS: i32 = 255 + 4 * 24;

/// `Segmentation_Feature_Max[SEG_LVL_MAX]` (AV2 v1.0.0 § 5.4.9): the clipping limit for
/// each feature value.
const SEGMENTATION_FEATURE_MAX: [i32; SEG_LVL_MAX] = [MAXQ_BITS, 0, 0];

/// One segment feature `(enabled, data)` pair (AV2 v1.0.0 § 5.4.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SegmentFeature {
    /// `FeatureEnabled[i][j]` (`feature_enabled`).
    pub enabled: bool,
    /// `FeatureData[i][j]`, the clipped feature value (0 when disabled).
    pub data: i32,
}

impl SegmentFeature {
    /// A disabled feature with zero data.
    pub const DISABLED: Self = Self {
        enabled: false,
        data: 0,
    };
}

/// Parsed `seg_info(numSegments)` (AV2 v1.0.0 § 5.4.9).
///
/// `features[i][j]` holds the `(enabled, data)` pair for segment `i` and feature `j`;
/// segments beyond [`SegmentInfo::num_segments`] are disabled with zero data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SegmentInfo {
    /// `numSegments` actually parsed (8 or 16 at the sequence/MFH call sites).
    pub num_segments: u8,
    /// `(FeatureEnabled, FeatureData)` for every segment and feature.
    pub features: [[SegmentFeature; SEG_LVL_MAX]; MAX_SEGMENTS],
}

impl SegmentInfo {
    /// Returns the parsed features for segment `i`, if `i < num_segments`.
    #[must_use]
    pub fn segment(&self, i: usize) -> Option<&[SegmentFeature; SEG_LVL_MAX]> {
        if i < self.num_segments as usize {
            self.features.get(i)
        } else {
            None
        }
    }
}

/// Clips `value` to the inclusive range `[low, high]` (AV2 `Clip3`).
const fn clip3(low: i32, high: i32, value: i32) -> i32 {
    if value < low {
        low
    } else if value > high {
        high
    } else {
        value
    }
}

/// Parses `seg_info(numSegments)` (AV2 v1.0.0 § 5.4.9).
///
/// `num_segments` is `MaxSegments` from the caller (8 or 16). It is capped at
/// [`MAX_SEGMENTS`] so an out-of-range caller value cannot index out of bounds; the
/// sequence and multi-frame-header call sites only ever pass 8 or 16. Unused feature
/// slots stay disabled with zero data.
///
/// # Errors
/// Returns descriptor errors or
/// [`Error::UnexpectedEof`](crate::error::Error::UnexpectedEof) if the payload ends
/// mid-field.
pub fn parse_seg_info(reader: &mut BitReader<'_>, num_segments: u8) -> Result<SegmentInfo> {
    let count = (num_segments as usize).min(MAX_SEGMENTS);
    let mut features = [[SegmentFeature::DISABLED; SEG_LVL_MAX]; MAX_SEGMENTS];

    for segment in features.iter_mut().take(count) {
        for (j, feature) in segment.iter_mut().enumerate() {
            let enabled = reader.read_flag()?;
            let data = if enabled {
                let bits_to_read = SEGMENTATION_FEATURE_BITS[j];
                let limit = SEGMENTATION_FEATURE_MAX[j];
                if SEGMENTATION_FEATURE_SIGNED[j] {
                    // AV2 § 5.4.9: n = 1 + bitsToRead; feature_value = su(n).
                    let value = reader.read_su(1 + bits_to_read)?;
                    clip3(-limit, limit, value)
                } else {
                    // f(bitsToRead); for j = 1, 2 bitsToRead is 0 (no value bits).
                    let value = reader.read_bits(bits_to_read)?;
                    // bits_to_read <= 9 here, so the value fits in i32 without loss.
                    clip3(0, limit, value as i32)
                }
            } else {
                0
            };
            *feature = SegmentFeature { enabled, data };
        }
    }

    Ok(SegmentInfo {
        // count <= MAX_SEGMENTS (16), so it fits in u8.
        num_segments: count as u8,
        features,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::error::Error;
    use crate::span::ByteOffset;

    use crate::test_bits::Bits;

    #[test]
    fn all_disabled_eight_segments_consumes_one_bit_per_feature() {
        // 8 segments * SEG_LVL_MAX (3) feature_enabled bits, all 0.
        let mut bits = Bits::default();
        for _ in 0..(8 * SEG_LVL_MAX) {
            bits.bit(0);
        }
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let info = parse_seg_info(&mut reader, 8).unwrap();
        assert_eq!(info.num_segments, 8);
        assert_eq!(reader.consumed_bits(), (8 * SEG_LVL_MAX) as u64);
        for i in 0..8 {
            for j in 0..SEG_LVL_MAX {
                assert_eq!(info.features[i][j], SegmentFeature::DISABLED);
            }
        }
        // Segments beyond num_segments are disabled too.
        assert_eq!(info.segment(8), None);
        assert_eq!(info.features[15][0], SegmentFeature::DISABLED);
    }

    #[test]
    fn all_disabled_sixteen_segments() {
        let mut bits = Bits::default();
        for _ in 0..(16 * SEG_LVL_MAX) {
            bits.bit(0);
        }
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let info = parse_seg_info(&mut reader, 16).unwrap();
        assert_eq!(info.num_segments, 16);
        assert_eq!(reader.consumed_bits(), (16 * SEG_LVL_MAX) as u64);
        assert!(
            info.features
                .iter()
                .flatten()
                .all(|f| *f == SegmentFeature::DISABLED)
        );
    }

    #[test]
    fn signed_quantizer_feature_is_read_via_su_and_clipped() {
        // Segment 0, feature 0 (SEG_LVL_ALT_Q) enabled with a negative su(10) value;
        // remaining features disabled. su(10) of 0b10_0000_0000 = -512, clipped to
        // -351 (Segmentation_Feature_Max[0] = MAXQ_BITS = 351).
        let mut bits = Bits::default();
        bits.bit(1); // feature_enabled[0][0]
        bits.f(0b10_0000_0000, 10); // su(10) value = -512
        bits.bit(0); // feature_enabled[0][1]
        bits.bit(0); // feature_enabled[0][2]
        // Segment 1: all disabled (3 bits) -> exercise num_segments = 2.
        bits.bit(0);
        bits.bit(0);
        bits.bit(0);
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let info = parse_seg_info(&mut reader, 2).unwrap();
        assert_eq!(info.num_segments, 2);
        assert!(info.features[0][0].enabled);
        assert_eq!(info.features[0][0].data, -351);
        assert_eq!(info.features[0][1], SegmentFeature::DISABLED);
        assert_eq!(info.features[1][0], SegmentFeature::DISABLED);
    }

    #[test]
    fn signed_quantizer_feature_within_limit_is_not_clipped() {
        // su(10) value 100 (positive, within +/-351) is stored unclipped.
        let mut bits = Bits::default();
        bits.bit(1); // feature_enabled[0][0]
        bits.f(100, 10); // su(10) value = 100
        bits.bit(0); // feature_enabled[0][1]
        bits.bit(0); // feature_enabled[0][2]
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let info = parse_seg_info(&mut reader, 1).unwrap();
        assert!(info.features[0][0].enabled);
        assert_eq!(info.features[0][0].data, 100);
    }

    #[test]
    fn reports_eof_without_panicking() {
        let mut reader = BitReader::new(&[], ByteOffset::new(0));
        assert!(matches!(
            parse_seg_info(&mut reader, 8),
            Err(Error::UnexpectedEof { .. })
        ));
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use crate::span::ByteOffset;
    use proptest::prelude::*;

    proptest! {
        /// `seg_info()` must never panic on arbitrary input or segment counts.
        #[test]
        fn parse_seg_info_never_panics(
            data in proptest::collection::vec(any::<u8>(), 0..64),
            num_segments in any::<u8>(),
        ) {
            let mut reader = BitReader::new(&data, ByteOffset::new(0));
            let _ = parse_seg_info(&mut reader, num_segments);
        }
    }
}

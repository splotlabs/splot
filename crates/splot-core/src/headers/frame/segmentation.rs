// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Frame-header segmentation parameters (AV2 v1.0.0 § 5.18.7.1,
//! `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-1`).
//!
//! Models `segmentation_params()` on the **intra path with a direct sequence
//! reference** (`cur_mfh_id == 0`): there `DerivedPrimaryRefFrame ==
//! PRIMARY_REF_NONE`, so `segmentation_update_map` is inferred `1` and
//! `segmentation_temporal_update` is inferred `0` without reading bits, and the
//! `haveSegParams` / `allowChange` derivation uses the sequence state
//! (`seq_seg_info_present_flag` / `seq_allow_seg_info_change`, AV2 § 5.4.4) only.
//! The `cur_mfh_id > 0` branch consults multi-frame-header fields
//! (`mfh_seg_info_present_flag`, `mfh_ext_seg_flag`, `mfh_allow_seg_info_change`)
//! that the validator's MFH record does not carry yet, so the caller stops with an
//! explicit partial status before reaching this parser on that path.
//!
//! Fresh feature data is parsed with the existing `seg_info(MaxSegments)` helper
//! ([`crate::segment::parse_seg_info`], AV2 § 5.4.9).

use crate::bitio::BitReader;
use crate::error::Result;
use crate::headers::sequence::SequenceSegmentConfig;
use crate::segment::{MAX_SEGMENTS, SEG_LVL_MAX, SegmentFeature, SegmentInfo, parse_seg_info};

/// `SEG_LVL_SKIP`: index of the skip segment feature (AV2 v1.0.0 § 3,
/// `docs/spec/av2/1.0.0/03-symbols.md`), the `SegIdPreSkip` threshold in
/// § 5.18.7.1.
const SEG_LVL_SKIP: usize = 1;

/// Sequence-derived inputs for `segmentation_params()` (AV2 v1.0.0 § 5.18.7.1),
/// gathered from `sequence_segment_config()` (AV2 § 5.4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoreSeqSegView {
    /// `seq_seg_info_present_flag` (AV2 § 5.4.4): selects the sequence branch of
    /// the `haveSegParams` derivation (§ 5.18.7.1).
    pub seq_seg_info_present_flag: bool,
    /// `seq_allow_seg_info_change` (AV2 § 5.4.4). Only consulted when
    /// `seq_seg_info_present_flag` is set (`false` here when not signalled).
    pub seq_allow_seg_info_change: bool,
    /// `enable_ext_seg` (AV2 § 5.4.4), used by the `cur_mfh_id > 0` branch's
    /// `mfh_ext_seg_flag == enable_ext_seg` comparison (§ 5.18.7.1).
    pub enable_ext_seg: bool,
    /// `MaxSegments` (AV2 § 5.4.4: `16` when `enable_ext_seg`, else `8`), the
    /// `seg_info()` argument and the loop bound for `LastActiveSegId` (§ 5.18.7.1)
    /// and the § 5.18.2 lossless derivation.
    pub max_segments: u8,
    /// The stored sequence feature data (`SeqFeatureEnabled` / `SeqFeatureData`,
    /// AV2 § 5.4.4 / § 5.4.9), reused when `reuse_seg_info` is `1` with
    /// `mfhId == 0` (§ 5.18.7.1). Present only when `seq_seg_info_present_flag`.
    pub seq_segment_info: Option<SegmentInfo>,
}

impl CoreSeqSegView {
    /// Builds the segmentation view from the parsed `sequence_segment_config()`
    /// (AV2 v1.0.0 § 5.4.4).
    #[must_use]
    pub(crate) fn from_sequence_config(segment: &SequenceSegmentConfig) -> Self {
        Self {
            seq_seg_info_present_flag: segment.seq_seg_info_present_flag,
            seq_allow_seg_info_change: segment.seq_allow_seg_info_change.unwrap_or(false),
            enable_ext_seg: segment.enable_ext_seg,
            max_segments: segment.max_segments,
            seq_segment_info: segment.segment_info,
        }
    }
}

/// Parsed `segmentation_params()` (AV2 v1.0.0 § 5.18.7.1,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-1`) on the intra path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct SegmentationParams {
    /// `segmentation_enabled`.
    pub segmentation_enabled: bool,
    /// `reuse_seg_info` (read when `allowChange`, else inferred `haveSegParams`;
    /// meaningful only when `segmentation_enabled`).
    pub reuse_seg_info: bool,
    /// `FeatureEnabled` / `FeatureData` per segment and feature: all-zero when
    /// segmentation is disabled, the stored sequence data when `reuse_seg_info`,
    /// or freshly parsed `seg_info(MaxSegments)` (AV2 § 5.4.9) otherwise.
    pub features: [[SegmentFeature; SEG_LVL_MAX]; MAX_SEGMENTS],
    /// `segmentation_update_map` (intra path: inferred `1` when
    /// `segmentation_enabled`, since `DerivedPrimaryRefFrame == PRIMARY_REF_NONE`).
    pub segmentation_update_map: bool,
    /// `segmentation_temporal_update` (intra path: inferred `0`).
    pub segmentation_temporal_update: bool,
    /// `SegIdPreSkip`: whether any enabled feature index is `>= SEG_LVL_SKIP`.
    pub seg_id_pre_skip: bool,
    /// `LastActiveSegId`: the highest segment id with any enabled feature.
    pub last_active_seg_id: u8,
}

/// Parses `segmentation_params()` (AV2 v1.0.0 § 5.18.7.1,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-1`) on the intra path
/// with `cur_mfh_id == 0` (the caller stops before this parser on the
/// MFH-resolved path; see the module docs).
///
/// # Errors
/// Returns [`Error::UnexpectedEof`](crate::error::Error::UnexpectedEof) or a typed
/// descriptor error if the payload ends or is malformed mid-field.
pub fn parse_segmentation_params(
    reader: &mut BitReader<'_>,
    seg: &CoreSeqSegView,
) -> Result<SegmentationParams> {
    // AV2 § 5.18.7.1: segmentation_enabled f(1).
    let segmentation_enabled = reader.read_bit()? != 0;

    let mut features = [[SegmentFeature::DISABLED; SEG_LVL_MAX]; MAX_SEGMENTS];
    let mut reuse_seg_info = false;
    let mut segmentation_update_map = false;
    // AV2 § 5.18.7.1: segmentation_temporal_update = 0 on every path this parser
    // models (intra, DerivedPrimaryRefFrame == PRIMARY_REF_NONE).
    let segmentation_temporal_update = false;

    if segmentation_enabled {
        // AV2 § 5.18.7.1 haveSegParams / allowChange / mfhId derivation. This
        // parser only runs with cur_mfh_id == 0 (see module docs), so the
        // `cur_mfh_id > 0 && mfh_seg_info_present_flag[cur_mfh_id]` branch is
        // never taken: only the sequence branch and the zero fallback apply, and
        // mfhId == 0 throughout.
        let (have_seg_params, allow_change) = if seg.seq_seg_info_present_flag {
            (true, seg.seq_allow_seg_info_change)
        } else {
            (false, false)
        };

        // AV2 § 5.18.7.1: reuse_seg_info f(1) when allowChange, else inferred
        // reuse_seg_info = haveSegParams.
        reuse_seg_info = if allow_change {
            reader.read_bit()? != 0
        } else {
            have_seg_params
        };

        if reuse_seg_info {
            // AV2 § 5.18.7.1, mfhId == 0: FeatureEnabled / FeatureData copy
            // SeqFeatureEnabled / SeqFeatureData (§ 5.4.4) for all MAX_SEGMENTS
            // segments. reuse_seg_info == 1 implies haveSegParams == 1, i.e.
            // seq_seg_info_present_flag, so a view built by
            // `from_sequence_config` always carries the stored data here; an
            // inconsistent hand-built view keeps the all-disabled default
            // instead of panicking.
            if let Some(info) = seg.seq_segment_info {
                features = info.features;
            }
        } else {
            // AV2 § 5.18.7.1: (FeatureEnabled, FeatureData) =
            // seg_info(MaxSegments) (§ 5.4.9).
            features = parse_seg_info(reader, seg.max_segments)?.features;
        }

        // AV2 § 5.18.7.1: DerivedPrimaryRefFrame == PRIMARY_REF_NONE on the
        // intra path, so segmentation_update_map = 1 and
        // segmentation_temporal_update = 0 are inferred without reading bits.
        segmentation_update_map = true;
    }
    // AV2 § 5.18.7.1 else-branch: FeatureEnabled / FeatureData stay all zero.

    // AV2 § 5.18.7.1: SegIdPreSkip / LastActiveSegId derivation over
    // 0 <= i < MaxSegments (capped at MAX_SEGMENTS like seg_info(), § 5.4.9).
    let max_segments = (seg.max_segments as usize).min(MAX_SEGMENTS);
    let mut seg_id_pre_skip = false;
    let mut last_active_seg_id = 0u8;
    for (i, segment) in features.iter().enumerate().take(max_segments) {
        for (j, feature) in segment.iter().enumerate() {
            if feature.enabled {
                // i < MAX_SEGMENTS (16), so it fits in u8.
                last_active_seg_id = i as u8;
                if j >= SEG_LVL_SKIP {
                    seg_id_pre_skip = true;
                }
            }
        }
    }

    Ok(SegmentationParams {
        segmentation_enabled,
        reuse_seg_info,
        features,
        segmentation_update_map,
        segmentation_temporal_update,
        seg_id_pre_skip,
        last_active_seg_id,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::error::Error;
    use crate::span::ByteOffset;

    #[derive(Default)]
    struct Bits {
        bits: Vec<u8>,
    }

    impl Bits {
        fn bit(&mut self, bit: u8) {
            self.bits.push(bit & 1);
        }

        fn f(&mut self, value: u32, width: u32) {
            for shift in (0..width).rev() {
                self.bit(((value >> shift) & 1) as u8);
            }
        }

        fn into_bytes(self) -> Vec<u8> {
            let mut bytes = Vec::new();
            for chunk in self.bits.chunks(8) {
                let mut byte = 0u8;
                for (i, bit) in chunk.iter().enumerate() {
                    byte |= *bit << (7 - i);
                }
                bytes.push(byte);
            }
            bytes
        }
    }

    /// A view with no sequence segment info signalled (§ 5.4.4
    /// `seq_seg_info_present_flag == 0`).
    fn no_seq_info_view(max_segments: u8) -> CoreSeqSegView {
        CoreSeqSegView {
            seq_seg_info_present_flag: false,
            seq_allow_seg_info_change: false,
            enable_ext_seg: max_segments == 16,
            max_segments,
            seq_segment_info: None,
        }
    }

    /// A view carrying stored sequence feature data (§ 5.4.4
    /// `seq_seg_info_present_flag == 1`) with segment 7, feature 2 enabled.
    fn seq_info_view(allow_change: bool) -> CoreSeqSegView {
        let mut features = [[SegmentFeature::DISABLED; SEG_LVL_MAX]; MAX_SEGMENTS];
        features[7][2] = SegmentFeature {
            enabled: true,
            data: 0,
        };
        CoreSeqSegView {
            seq_seg_info_present_flag: true,
            seq_allow_seg_info_change: allow_change,
            enable_ext_seg: false,
            max_segments: 8,
            seq_segment_info: Some(SegmentInfo {
                num_segments: 8,
                features,
            }),
        }
    }

    #[test]
    fn disabled_segmentation_zeroes_features_and_reads_one_bit() {
        // § 5.18.7.1 else-branch: all FeatureEnabled/FeatureData zero, no
        // further bits read.
        let mut bits = Bits::default();
        bits.bit(0); // segmentation_enabled
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let params = parse_segmentation_params(&mut reader, &no_seq_info_view(8)).unwrap();
        assert_eq!(reader.consumed_bits(), 1);
        assert!(!params.segmentation_enabled);
        assert!(!params.reuse_seg_info);
        assert!(
            params
                .features
                .iter()
                .flatten()
                .all(|f| *f == SegmentFeature::DISABLED)
        );
        assert!(!params.segmentation_update_map);
        assert!(!params.segmentation_temporal_update);
        assert!(!params.seg_id_pre_skip);
        assert_eq!(params.last_active_seg_id, 0);
    }

    #[test]
    fn fresh_seg_info_quantizer_feature_sets_last_active_without_pre_skip() {
        // No sequence info: haveSegParams = 0, allowChange = 0, so
        // reuse_seg_info is inferred 0 (no bit) and seg_info(8) is parsed.
        // Segment 2 feature 0 (SEG_LVL_ALT_Q, j < SEG_LVL_SKIP) enabled.
        let mut bits = Bits::default();
        bits.bit(1); // segmentation_enabled
        for _ in 0..2 {
            bits.f(0, 3); // segments 0..2: all features disabled
        }
        bits.bit(1); // segment 2: feature_enabled[2][0]
        bits.f(100, 10); // su(10) feature value = 100
        bits.bit(0); // feature_enabled[2][1]
        bits.bit(0); // feature_enabled[2][2]
        for _ in 0..5 {
            bits.f(0, 3); // segments 3..8: all features disabled
        }
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let params = parse_segmentation_params(&mut reader, &no_seq_info_view(8)).unwrap();
        // 1 + 8 * SEG_LVL_MAX enabled bits + 10 value bits.
        assert_eq!(reader.consumed_bits(), 35);
        assert!(params.segmentation_enabled);
        assert!(!params.reuse_seg_info);
        assert!(params.features[2][0].enabled);
        assert_eq!(params.features[2][0].data, 100);
        // Intra path: update_map inferred 1, temporal_update inferred 0.
        assert!(params.segmentation_update_map);
        assert!(!params.segmentation_temporal_update);
        assert_eq!(params.last_active_seg_id, 2);
        assert!(!params.seg_id_pre_skip);
    }

    #[test]
    fn fresh_seg_info_skip_feature_sets_pre_skip() {
        // Segment 5 feature 1 (j >= SEG_LVL_SKIP) enabled: no value bits
        // (Segmentation_Feature_Bits[1] = 0), SegIdPreSkip = 1.
        let mut bits = Bits::default();
        bits.bit(1); // segmentation_enabled
        for _ in 0..5 {
            bits.f(0, 3); // segments 0..5 disabled
        }
        bits.bit(0); // feature_enabled[5][0]
        bits.bit(1); // feature_enabled[5][1]
        bits.bit(0); // feature_enabled[5][2]
        for _ in 0..2 {
            bits.f(0, 3); // segments 6..8 disabled
        }
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let params = parse_segmentation_params(&mut reader, &no_seq_info_view(8)).unwrap();
        assert_eq!(reader.consumed_bits(), 25);
        assert!(params.features[5][1].enabled);
        assert_eq!(params.last_active_seg_id, 5);
        assert!(params.seg_id_pre_skip);
    }

    #[test]
    fn fresh_seg_info_with_ext_seg_reaches_segment_fifteen() {
        // enable_ext_seg: MaxSegments = 16; segment 15 feature 0 enabled
        // exercises the full derivation loop bound.
        let mut bits = Bits::default();
        bits.bit(1); // segmentation_enabled
        for _ in 0..15 {
            bits.f(0, 3); // segments 0..15 disabled
        }
        bits.bit(1); // feature_enabled[15][0]
        bits.f(1, 10); // su(10) feature value = 1
        bits.bit(0); // feature_enabled[15][1]
        bits.bit(0); // feature_enabled[15][2]
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let params = parse_segmentation_params(&mut reader, &no_seq_info_view(16)).unwrap();
        assert_eq!(reader.consumed_bits(), 1 + 16 * 3 + 10);
        assert_eq!(params.last_active_seg_id, 15);
        assert!(!params.seg_id_pre_skip);
    }

    #[test]
    fn sequence_reuse_is_inferred_without_a_bit_when_change_not_allowed() {
        // seq_seg_info_present_flag = 1, seq_allow_seg_info_change = 0:
        // reuse_seg_info inferred = haveSegParams = 1, only 1 bit read.
        let view = seq_info_view(false);
        let mut bits = Bits::default();
        bits.bit(1); // segmentation_enabled
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let params = parse_segmentation_params(&mut reader, &view).unwrap();
        assert_eq!(reader.consumed_bits(), 1);
        assert!(params.reuse_seg_info);
        assert_eq!(
            params.features,
            view.seq_segment_info.unwrap().features,
            "reuse copies SeqFeatureEnabled/SeqFeatureData"
        );
        assert!(params.segmentation_update_map);
        assert!(!params.segmentation_temporal_update);
        // Stored feature: segment 7, feature 2 (j >= SEG_LVL_SKIP).
        assert_eq!(params.last_active_seg_id, 7);
        assert!(params.seg_id_pre_skip);
    }

    #[test]
    fn sequence_reuse_bit_is_read_when_change_allowed() {
        // seq_allow_seg_info_change = 1: reuse_seg_info f(1) read as 1.
        let view = seq_info_view(true);
        let mut bits = Bits::default();
        bits.bit(1); // segmentation_enabled
        bits.bit(1); // reuse_seg_info
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let params = parse_segmentation_params(&mut reader, &view).unwrap();
        assert_eq!(reader.consumed_bits(), 2);
        assert!(params.reuse_seg_info);
        assert_eq!(params.features, view.seq_segment_info.unwrap().features);
        assert_eq!(params.last_active_seg_id, 7);
    }

    #[test]
    fn declined_reuse_parses_fresh_seg_info_instead_of_stored_data() {
        // reuse_seg_info read as 0: seg_info(8) is parsed and the stored
        // sequence data is ignored.
        let view = seq_info_view(true);
        let mut bits = Bits::default();
        bits.bit(1); // segmentation_enabled
        bits.bit(0); // reuse_seg_info
        for _ in 0..8 {
            bits.f(0, 3); // seg_info(8): all features disabled
        }
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let params = parse_segmentation_params(&mut reader, &view).unwrap();
        assert_eq!(reader.consumed_bits(), 2 + 8 * 3);
        assert!(!params.reuse_seg_info);
        assert!(
            params
                .features
                .iter()
                .flatten()
                .all(|f| *f == SegmentFeature::DISABLED)
        );
        assert_eq!(params.last_active_seg_id, 0);
        assert!(!params.seg_id_pre_skip);
    }

    #[test]
    fn empty_input_reports_eof_without_panicking() {
        let mut reader = BitReader::new(&[], ByteOffset::new(0));
        assert!(matches!(
            parse_segmentation_params(&mut reader, &no_seq_info_view(8)),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn truncation_inside_seg_info_reports_eof() {
        // segmentation_enabled = 1 then only 7 more bits, but seg_info(8)
        // needs at least 24.
        let data = [0b1000_0000u8];
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        assert!(matches!(
            parse_segmentation_params(&mut reader, &no_seq_info_view(8)),
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
        /// `segmentation_params()` must never panic, for arbitrary payloads and
        /// arbitrary (even internally inconsistent) sequence views.
        #[test]
        fn parse_segmentation_params_never_panics(
            data in proptest::collection::vec(any::<u8>(), 0..96),
            seq_seg_info_present_flag in any::<bool>(),
            seq_allow_seg_info_change in any::<bool>(),
            enable_ext_seg in any::<bool>(),
            max_segments in any::<u8>(),
            has_stored_info in any::<bool>(),
            stored_segment in 0..MAX_SEGMENTS,
            stored_feature in 0..SEG_LVL_MAX,
            stored_data in any::<i32>(),
        ) {
            let mut features = [[SegmentFeature::DISABLED; SEG_LVL_MAX]; MAX_SEGMENTS];
            features[stored_segment][stored_feature] = SegmentFeature {
                enabled: true,
                data: stored_data,
            };
            let seg = CoreSeqSegView {
                seq_seg_info_present_flag,
                seq_allow_seg_info_change,
                enable_ext_seg,
                max_segments,
                seq_segment_info: has_stored_info.then_some(SegmentInfo {
                    num_segments: max_segments.min(MAX_SEGMENTS as u8),
                    features,
                }),
            };
            let mut reader = BitReader::new(&data, ByteOffset::new(0));
            let _ = parse_segmentation_params(&mut reader, &seg);
        }
    }
}

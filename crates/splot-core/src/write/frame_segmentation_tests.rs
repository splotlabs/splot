// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>


#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::too_many_lines)]
mod tests {
    use super::*;
    use crate::bitio::BitReader;
    use crate::headers::frame::parse_segmentation_params;
    use crate::span::ByteOffset;

    use crate::test_bits::Bits;

    /// A view with no sequence segment info signalled (§ 5.4.4
    /// `seq_seg_info_present_flag == 0`), mirroring the parser's own helper.
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
        let mut features = ALL_DISABLED;
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

    /// An MFH segmentation view carrying stored feature data with segment 3, feature 0
    /// (SEG_LVL_ALT_Q) enabled.
    fn mfh_seg_view(ext_seg: bool, allow_change: bool) -> MfhSegView {
        let mut features = ALL_DISABLED;
        features[3][0] = SegmentFeature {
            enabled: true,
            data: 9,
        };
        MfhSegView {
            mfh_ext_seg_flag: ext_seg,
            mfh_allow_seg_info_change: allow_change,
            mfh_segment_info: SegmentInfo {
                num_segments: 8,
                features,
            },
        }
    }

    fn parse(bytes: &[u8], seg: &CoreSeqSegView, mfh: Option<&MfhSegView>) -> SegmentationParams {
        let mut reader = BitReader::new(bytes, ByteOffset::new(0));
        parse_segmentation_params(&mut reader, seg, mfh).unwrap()
    }

    fn write(
        params: &SegmentationParams,
        seg: &CoreSeqSegView,
        mfh: Option<&MfhSegView>,
    ) -> Vec<u8> {
        let mut writer = BitWriter::new();
        write_segmentation_params(&mut writer, params, seg, mfh).unwrap();
        writer.into_bytes()
    }

    /// Asserts the semantic round-trip `parse(write(params)) == params` and byte-stability
    /// against the same hand-built fixture bytes.
    fn assert_roundtrip(
        bytes: &[u8],
        seg: &CoreSeqSegView,
        mfh: Option<&MfhSegView>,
    ) -> SegmentationParams {
        let params = parse(bytes, seg, mfh);
        let written = write(&params, seg, mfh);
        let reparsed = parse(&written, seg, mfh);
        assert_eq!(reparsed, params, "parse(write(params)) != params");
        let rewritten = write(&reparsed, seg, mfh);
        assert_eq!(rewritten, written, "write not idempotent");
        params
    }


    #[test]
    fn disabled_round_trips_byte_exact() {
        let mut bits = Bits::default();
        bits.bit(0); // segmentation_enabled
        let data = bits.into_bytes();
        let seg = no_seq_info_view(8);
        let params = assert_roundtrip(&data, &seg, None);
        assert!(!params.segmentation_enabled);
        assert_eq!(write(&params, &seg, None), data, "not byte-exact");
    }

    #[test]
    fn fresh_seg_info_quantizer_feature_round_trips() {
        let mut bits = Bits::default();
        bits.bit(1); // segmentation_enabled
        for _ in 0..2 {
            bits.f(0, 3); // segments 0..2 disabled
        }
        bits.bit(1); // feature_enabled[2][0]
        bits.f(100, 10); // su(10) feature value = 100
        bits.bit(0); // feature_enabled[2][1]
        bits.bit(0); // feature_enabled[2][2]
        for _ in 0..5 {
            bits.f(0, 3); // segments 3..8 disabled
        }
        let data = bits.into_bytes();
        let seg = no_seq_info_view(8);
        let params = assert_roundtrip(&data, &seg, None);
        assert!(params.features[2][0].enabled);
        assert_eq!(params.features[2][0].data, 100);
        assert!(params.segmentation_update_map);
        assert!(!params.segmentation_temporal_update);
        assert_eq!(params.last_active_seg_id, 2);
        assert!(!params.seg_id_pre_skip);
        assert_eq!(write(&params, &seg, None), data, "not byte-exact");
    }

    #[test]
    fn fresh_seg_info_skip_feature_round_trips_with_pre_skip() {
        let mut bits = Bits::default();
        bits.bit(1); // segmentation_enabled
        for _ in 0..5 {
            bits.f(0, 3); // segments 0..5 disabled
        }
        bits.bit(0); // [5][0]
        bits.bit(1); // [5][1] enabled
        bits.bit(0); // [5][2]
        for _ in 0..2 {
            bits.f(0, 3); // segments 6..8 disabled
        }
        let data = bits.into_bytes();
        let seg = no_seq_info_view(8);
        let params = assert_roundtrip(&data, &seg, None);
        assert!(params.features[5][1].enabled);
        assert!(params.seg_id_pre_skip);
        assert_eq!(params.last_active_seg_id, 5);
        assert_eq!(write(&params, &seg, None), data, "not byte-exact");
    }

    #[test]
    fn fresh_seg_info_ext_seg_reaches_segment_fifteen_round_trips() {
        let mut bits = Bits::default();
        bits.bit(1); // segmentation_enabled
        for _ in 0..15 {
            bits.f(0, 3); // segments 0..15 disabled
        }
        bits.bit(1); // [15][0]
        bits.f(1, 10); // su(10) value = 1
        bits.bit(0); // [15][1]
        bits.bit(0); // [15][2]
        let data = bits.into_bytes();
        let seg = no_seq_info_view(16);
        let params = assert_roundtrip(&data, &seg, None);
        assert_eq!(params.last_active_seg_id, 15);
        assert!(!params.seg_id_pre_skip);
        assert_eq!(write(&params, &seg, None), data, "not byte-exact");
    }

    #[test]
    fn sequence_reuse_inferred_round_trips_without_a_bit() {
        let seg = seq_info_view(false);
        let mut bits = Bits::default();
        bits.bit(1); // segmentation_enabled
        let data = bits.into_bytes();
        let params = assert_roundtrip(&data, &seg, None);
        assert!(params.reuse_seg_info);
        assert_eq!(params.features, seg.seq_segment_info.unwrap().features);
        assert_eq!(params.last_active_seg_id, 7);
        assert!(params.seg_id_pre_skip);
        assert_eq!(write(&params, &seg, None), data, "not byte-exact");
    }

    #[test]
    fn sequence_reuse_signaled_round_trips_with_a_bit() {
        let seg = seq_info_view(true);
        let mut bits = Bits::default();
        bits.bit(1); // segmentation_enabled
        bits.bit(1); // reuse_seg_info
        let data = bits.into_bytes();
        let params = assert_roundtrip(&data, &seg, None);
        assert!(params.reuse_seg_info);
        assert_eq!(params.features, seg.seq_segment_info.unwrap().features);
        assert_eq!(write(&params, &seg, None), data, "not byte-exact");
    }

    #[test]
    fn sequence_declined_reuse_signaled_round_trips_fresh() {
        let seg = seq_info_view(true);
        let mut bits = Bits::default();
        bits.bit(1); // segmentation_enabled
        bits.bit(0); // reuse_seg_info
        for _ in 0..8 {
            bits.f(0, 3); // fresh seg_info(8): all disabled
        }
        let data = bits.into_bytes();
        let params = assert_roundtrip(&data, &seg, None);
        assert!(!params.reuse_seg_info);
        assert!(params.features.iter().flatten().all(|f| !f.enabled));
        assert_eq!(write(&params, &seg, None), data, "not byte-exact");
    }

    #[test]
    fn mfh_arm_reuse_inferred_round_trips() {
        let seg = no_seq_info_view(8);
        let mfh = mfh_seg_view(false, false);
        let mut bits = Bits::default();
        bits.bit(1); // segmentation_enabled
        let data = bits.into_bytes();
        let params = assert_roundtrip(&data, &seg, Some(&mfh));
        assert!(params.reuse_seg_info);
        assert!(params.features[3][0].enabled);
        assert_eq!(params.features[3][0].data, 9);
        assert_eq!(params.last_active_seg_id, 3);
        assert_eq!(write(&params, &seg, Some(&mfh)), data, "not byte-exact");
    }

    #[test]
    fn mfh_arm_reuse_signaled_round_trips() {
        let seg = no_seq_info_view(8);
        let mfh = mfh_seg_view(false, true);
        let mut bits = Bits::default();
        bits.bit(1); // segmentation_enabled
        bits.bit(1); // reuse_seg_info
        let data = bits.into_bytes();
        let params = assert_roundtrip(&data, &seg, Some(&mfh));
        assert!(params.reuse_seg_info);
        assert!(params.features[3][0].enabled);
        assert_eq!(write(&params, &seg, Some(&mfh)), data, "not byte-exact");
    }

    #[test]
    fn mfh_arm_ext_seg_mismatch_round_trips_fresh() {
        let seg = no_seq_info_view(8); // enable_ext_seg == false
        let mfh = mfh_seg_view(true, true); // mfh_ext_seg_flag == true != false
        let mut bits = Bits::default();
        bits.bit(1); // segmentation_enabled
        for _ in 0..8 {
            bits.f(0, 3); // fresh seg_info(8): all disabled
        }
        let data = bits.into_bytes();
        let params = assert_roundtrip(&data, &seg, Some(&mfh));
        assert!(!params.reuse_seg_info);
        assert!(params.features.iter().flatten().all(|f| !f.enabled));
        assert_eq!(write(&params, &seg, Some(&mfh)), data, "not byte-exact");
    }

    #[test]
    fn mfh_arm_takes_priority_over_sequence_branch_round_trips() {
        let seg = seq_info_view(false); // seq segment 7, feature 2
        let mfh = mfh_seg_view(false, false); // MFH segment 3, feature 0
        let mut bits = Bits::default();
        bits.bit(1); // segmentation_enabled
        let data = bits.into_bytes();
        let params = assert_roundtrip(&data, &seg, Some(&mfh));
        assert!(params.features[3][0].enabled, "MFH branch wins");
        assert!(!params.features[7][2].enabled, "sequence data unused");
        assert_eq!(write(&params, &seg, Some(&mfh)), data, "not byte-exact");
    }


    /// Builds a canonical enabled+fresh model with all-disabled features (the writer's most
    /// permissive starting point for mutation tests).
    fn canonical_fresh(seg: &CoreSeqSegView) -> SegmentationParams {
        let mut bits = Bits::default();
        bits.bit(1); // segmentation_enabled
        for _ in 0..(seg.max_segments.min(16)) {
            bits.f(0, 3);
        }
        parse(&bits.into_bytes(), seg, None)
    }

    fn assert_rejected(
        params: &SegmentationParams,
        seg: &CoreSeqSegView,
        mfh: Option<&MfhSegView>,
        what: &'static str,
    ) {
        let mut writer = BitWriter::new();
        let err = write_segmentation_params(&mut writer, params, seg, mfh).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader { what },
            "unexpected error variant"
        );
        assert_eq!(writer.bit_len(), 0, "reject must not write any bit");
    }

    #[test]
    fn rejects_inferred_reuse_disagreeing_with_have_seg_params() {
        let seg = no_seq_info_view(8);
        let mut params = canonical_fresh(&seg);
        params.reuse_seg_info = true;
        assert_rejected(&params, &seg, None, "segmentation_reuse_seg_info");
    }

    #[test]
    fn rejects_reuse_features_not_matching_source() {
        let seg = seq_info_view(false);
        let mut bits = Bits::default();
        bits.bit(1); // segmentation_enabled
        let mut params = parse(&bits.into_bytes(), &seg, None);
        assert!(params.reuse_seg_info);
        params.features[7][2] = SegmentFeature::DISABLED;
        params.features[6][2] = SegmentFeature {
            enabled: true,
            data: 0,
        };
        let (pre_skip, last) = derive_seg_id_state(&params.features, seg.max_segments);
        params.seg_id_pre_skip = pre_skip;
        params.last_active_seg_id = last;
        assert_rejected(&params, &seg, None, "segmentation_reuse_features");
    }

    #[test]
    fn rejects_enabled_segmentation_update_map_false() {
        let seg = no_seq_info_view(8);
        let mut params = canonical_fresh(&seg);
        params.segmentation_update_map = false;
        assert_rejected(&params, &seg, None, "segmentation_update_map");
    }

    #[test]
    fn rejects_enabled_segmentation_temporal_update_true() {
        let seg = no_seq_info_view(8);
        let mut params = canonical_fresh(&seg);
        params.segmentation_temporal_update = true;
        assert_rejected(&params, &seg, None, "segmentation_temporal_update");
    }

    #[test]
    fn rejects_disabled_with_reuse_seg_info_set() {
        let seg = no_seq_info_view(8);
        let mut params = parse(&[0u8], &seg, None);
        assert!(!params.segmentation_enabled);
        params.reuse_seg_info = true;
        assert_rejected(&params, &seg, None, "segmentation_disabled_reuse_seg_info");
    }

    #[test]
    fn rejects_disabled_with_non_default_features() {
        let seg = no_seq_info_view(8);
        let mut params = parse(&[0u8], &seg, None);
        params.features[1][0] = SegmentFeature {
            enabled: true,
            data: 0,
        };
        let (pre_skip, last) = derive_seg_id_state(&params.features, seg.max_segments);
        params.seg_id_pre_skip = pre_skip;
        params.last_active_seg_id = last;
        assert_rejected(&params, &seg, None, "segmentation_disabled_features");
    }

    #[test]
    fn rejects_disabled_with_update_map_set() {
        let seg = no_seq_info_view(8);
        let mut params = parse(&[0u8], &seg, None);
        params.segmentation_update_map = true;
        assert_rejected(&params, &seg, None, "segmentation_disabled_update_map");
    }

    #[test]
    fn rejects_disabled_with_temporal_update_set() {
        let seg = no_seq_info_view(8);
        let mut params = parse(&[0u8], &seg, None);
        params.segmentation_temporal_update = true;
        assert_rejected(
            &params,
            &seg,
            None,
            "segmentation_disabled_temporal_update",
        );
    }

    #[test]
    fn rejects_wrong_derived_seg_id_pre_skip() {
        let seg = no_seq_info_view(8);
        let mut params = canonical_fresh(&seg);
        params.seg_id_pre_skip = true; // table is all-disabled -> derives false
        assert_rejected(&params, &seg, None, "segmentation_seg_id_pre_skip");
    }

    #[test]
    fn rejects_wrong_derived_last_active_seg_id() {
        let seg = no_seq_info_view(8);
        let mut params = canonical_fresh(&seg);
        params.last_active_seg_id = 5; // table is all-disabled -> derives 0
        assert_rejected(&params, &seg, None, "segmentation_last_active_seg_id");
    }

    #[test]
    fn rejects_fresh_seg_info_with_unencodable_feature() {
        let seg = no_seq_info_view(8);
        let mut params = canonical_fresh(&seg);
        params.features[0][0] = SegmentFeature {
            enabled: true,
            data: i32::MAX, // far outside the +/-351 clip window
        };
        let (pre_skip, last) = derive_seg_id_state(&params.features, seg.max_segments);
        params.seg_id_pre_skip = pre_skip;
        params.last_active_seg_id = last;
        let mut writer = BitWriter::new();
        let err = write_segmentation_params(&mut writer, &params, &seg, None).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalSequenceValue {
                what: "seg_info_signed_data"
            }
        );
        assert_eq!(writer.bit_len(), 0, "reject must not write any bit");
    }

    #[test]
    fn hostile_max_segments_does_not_panic_and_round_trips_bounded() {
        let seg = CoreSeqSegView {
            seq_seg_info_present_flag: false,
            seq_allow_seg_info_change: false,
            enable_ext_seg: false,
            max_segments: 255, // hostile
            seq_segment_info: None,
        };
        let mut bits = Bits::default();
        bits.bit(1); // segmentation_enabled
        for _ in 0..16 {
            bits.f(0, 3);
        }
        let data = bits.into_bytes();
        let params = parse(&data, &seg, None);
        let written = write(&params, &seg, None);
        let reparsed = parse(&written, &seg, None);
        assert_eq!(reparsed, params);
    }
}

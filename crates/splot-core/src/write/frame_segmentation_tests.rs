// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

// Unit, round-trip, and rejection tests for the § 5.18.7.1 `segmentation_params()` writer.

// `include!`d into `crate::write::frame_segmentation` so `super::*` resolves to its writer
// and private helpers (the property tests live in the sibling
// `frame_segmentation_proptests.rs`).

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::too_many_lines)]
mod tests {
    use super::*;
    use crate::bitio::BitReader;
    use crate::headers::frame::parse_segmentation_params;
    use crate::span::ByteOffset;

    /// MSB-first bit builder mirroring the parser's own `Bits` helper, so this module reuses
    /// the same hand-built, spec-grounded fixtures for the parse-then-write round-trips.
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

    // ----- Round-trip tests (one per syntax branch) -----

    #[test]
    fn disabled_round_trips_byte_exact() {
        // § 5.18.7.1 else-branch: segmentation_enabled == 0, one bit, all-zero features.
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
        // No sequence info -> haveSegParams 0, allowChange 0, reuse inferred 0, seg_info(8)
        // parsed fresh. Segment 2 feature 0 (j < SEG_LVL_SKIP) enabled.
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
        // Segment 5 feature 1 (j >= SEG_LVL_SKIP) enabled: SegIdPreSkip = 1, no value bits.
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
        // enable_ext_seg -> MaxSegments 16; segment 15 feature 0 enabled exercises the full
        // derivation loop bound.
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
        // seq_seg_info_present_flag 1, allow_change 0: reuse inferred 1, only 1 bit.
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
        // seq_allow_seg_info_change 1: reuse_seg_info f(1) coded as 1.
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
        // allow_change 1, reuse coded as 0: fresh seg_info(8) is parsed.
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
        // MFH branch: ext_seg matches -> haveSegParams 1; allow_change 0 -> reuse inferred 1.
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
        // mfh_allow_seg_info_change 1 -> allowChange 1 -> reuse_seg_info f(1) coded as 1.
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
        // ext_seg mismatch -> haveSegParams 0, allowChange 0, reuse inferred 0; fresh
        // seg_info(8) parsed, ignoring the MFH data.
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
        // Both arms could supply data; § 5.18.7.1 selects the MFH branch first.
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

    // ----- Rejection tests (one per WriteError reject path; bit_len()==0) -----

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
        // No seq info -> haveSegParams 0, allowChange 0; reuse_seg_info is inferred 0. A
        // model claiming reuse_seg_info == true would reparse as false.
        let seg = no_seq_info_view(8);
        let mut params = canonical_fresh(&seg);
        params.reuse_seg_info = true;
        assert_rejected(&params, &seg, None, "segmentation_reuse_seg_info");
    }

    #[test]
    fn rejects_reuse_features_not_matching_source() {
        // Sequence reuse arm (allow_change 0): features must equal the stored source.
        let seg = seq_info_view(false);
        let mut bits = Bits::default();
        bits.bit(1); // segmentation_enabled
        let mut params = parse(&bits.into_bytes(), &seg, None);
        assert!(params.reuse_seg_info);
        // Corrupt one stored feature so it no longer matches the reuse source. Keep the
        // derived seg-id state consistent so this hits the reuse-features check, not the
        // derivation check.
        params.features[7][2] = SegmentFeature::DISABLED;
        params.features[6][2] = SegmentFeature {
            enabled: true,
            data: 0,
        };
        // Recompute the correct derived values for the mutated table so the derivation check
        // passes and the reuse-features check fires instead.
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
        // Keep the derived seg-id state consistent with the mutated table so the disabled
        // features check fires (not the derivation check).
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
        // Constructed model: a correct enabled table but a flipped SegIdPreSkip would reparse
        // with the re-derived value.
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
        // The fresh branch validates the seg_info(MaxSegments) body up front; an out-of-clip
        // signed value is rejected by check_seg_info_encodable (propagated, bit_len 0).
        let seg = no_seq_info_view(8);
        let mut params = canonical_fresh(&seg);
        params.features[0][0] = SegmentFeature {
            enabled: true,
            data: i32::MAX, // far outside the +/-351 clip window
        };
        // Keep the derived seg-id state consistent so the seg_info body check fires first.
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
        // A constructed CoreSeqSegView with an out-of-range max_segments must not panic; the
        // writer clamps the seg-id loop and seg_info count at MAX_SEGMENTS. seg_info() itself
        // also clamps, so a parser-produced model round-trips with the clamped count.
        let seg = CoreSeqSegView {
            seq_seg_info_present_flag: false,
            seq_allow_seg_info_change: false,
            enable_ext_seg: false,
            max_segments: 255, // hostile
            seq_segment_info: None,
        };
        // 16 segments worth of all-disabled seg_info bits (the parser clamps to 16).
        let mut bits = Bits::default();
        bits.bit(1); // segmentation_enabled
        for _ in 0..16 {
            bits.f(0, 3);
        }
        let data = bits.into_bytes();
        let params = parse(&data, &seg, None);
        // Re-emission must succeed and reparse identically.
        let written = write(&params, &seg, None);
        let reparsed = parse(&written, &seg, None);
        assert_eq!(reparsed, params);
    }
}

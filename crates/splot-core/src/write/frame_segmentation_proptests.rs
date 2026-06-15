// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

// Property tests for the § 5.18.7.1 `segmentation_params()` writer.

// `include!`d into `crate::write::frame_segmentation` so `super::*` resolves to its writer
// and private helpers.

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod proptests {
    use super::*;
    use crate::bitio::BitReader;
    use crate::headers::frame::parse_segmentation_params;
    use crate::span::ByteOffset;
    use proptest::prelude::*;

    /// Builds the same arbitrary (even internally inconsistent) sequence and MFH views the
    /// parser's own proptest exercises, so the writer is driven by genuine parser output.
    /// The sequence and MFH inputs are grouped into two nested tuples to stay within
    /// proptest's per-tuple arity.
    fn views() -> impl Strategy<Value = (CoreSeqSegView, Option<MfhSegView>)> {
        let seq = (
            any::<bool>(),
            any::<bool>(),
            any::<bool>(),
            prop_oneof![Just(8u8), Just(16u8), any::<u8>()],
            any::<bool>(),
            0..MAX_SEGMENTS,
            0..SEG_LVL_MAX,
            any::<i32>(),
        );
        let mfh = (
            any::<bool>(),
            any::<bool>(),
            any::<bool>(),
            0..MAX_SEGMENTS,
            0..SEG_LVL_MAX,
            any::<i32>(),
        );
        (seq, mfh).prop_map(
            |(
                (
                    seq_present,
                    seq_allow,
                    enable_ext_seg,
                    max_segments,
                    has_stored,
                    stored_seg,
                    stored_feat,
                    stored_data,
                ),
                (with_mfh, mfh_ext, mfh_allow, mfh_seg, mfh_feat, mfh_data),
            )| {
                let mut features = ALL_DISABLED;
                features[stored_seg][stored_feat] = SegmentFeature {
                    enabled: true,
                    data: stored_data,
                };
                let seg = CoreSeqSegView {
                    seq_seg_info_present_flag: seq_present,
                    seq_allow_seg_info_change: seq_allow,
                    enable_ext_seg,
                    max_segments,
                    seq_segment_info: has_stored.then_some(SegmentInfo {
                        num_segments: max_segments.min(MAX_SEGMENTS as u8),
                        features,
                    }),
                };
                let mut mfh_features = ALL_DISABLED;
                mfh_features[mfh_seg][mfh_feat] = SegmentFeature {
                    enabled: true,
                    data: mfh_data,
                };
                let mfh = with_mfh.then_some(MfhSegView {
                    mfh_ext_seg_flag: mfh_ext,
                    mfh_allow_seg_info_change: mfh_allow,
                    mfh_segment_info: SegmentInfo {
                        num_segments: max_segments.min(MAX_SEGMENTS as u8),
                        features: mfh_features,
                    },
                });
                (seg, mfh)
            },
        )
    }

    proptest! {
        /// Every parser-reachable `segmentation_params()` round-trips: parsing random bits
        /// under a random gating view, re-emitting the model, and reparsing yields the same
        /// model, and re-emission is byte-stable.
        #[test]
        fn roundtrip_segmentation_params(
            data in proptest::collection::vec(any::<u8>(), 0..96),
            (seg, mfh) in views(),
        ) {
            let mut reader = BitReader::new(&data, ByteOffset::new(0));
            let Ok(params) = parse_segmentation_params(&mut reader, &seg, mfh.as_ref()) else {
                return Ok(());
            };
            // The model came from the parser under these exact inputs, so the writer must
            // accept it and produce a byte-stable, reparse-identical encoding.
            let mut writer = BitWriter::new();
            write_segmentation_params(&mut writer, &params, &seg, mfh.as_ref())
                .expect("parser-produced model must be writable");
            let bytes = writer.into_bytes();
            let mut r2 = BitReader::new(&bytes, ByteOffset::new(0));
            let reparsed = parse_segmentation_params(&mut r2, &seg, mfh.as_ref())
                .expect("written bytes must reparse");
            prop_assert_eq!(reparsed, params);
            let mut w2 = BitWriter::new();
            write_segmentation_params(&mut w2, &reparsed, &seg, mfh.as_ref()).unwrap();
            prop_assert_eq!(w2.into_bytes(), bytes);
        }

        /// The writer never panics on an arbitrary constructed model and arbitrary (even
        /// internally inconsistent) views, including a hostile `max_segments`.
        #[test]
        fn writer_never_panics_on_constructed_models(
            enabled in any::<bool>(),
            reuse in any::<bool>(),
            update_map in any::<bool>(),
            temporal in any::<bool>(),
            pre_skip in any::<bool>(),
            last_active in any::<u8>(),
            seg_idx in 0..MAX_SEGMENTS,
            feat_idx in 0..SEG_LVL_MAX,
            feat_data in any::<i32>(),
            feat_enabled in any::<bool>(),
            (seg, mfh) in views(),
        ) {
            let mut features = ALL_DISABLED;
            features[seg_idx][feat_idx] = SegmentFeature {
                enabled: feat_enabled,
                data: feat_data,
            };
            let params = SegmentationParams {
                segmentation_enabled: enabled,
                reuse_seg_info: reuse,
                features,
                segmentation_update_map: update_map,
                segmentation_temporal_update: temporal,
                seg_id_pre_skip: pre_skip,
                last_active_seg_id: last_active,
            };
            let mut writer = BitWriter::new();
            // Either accepted or rejected; never a panic, and a reject leaves bit_len 0.
            if write_segmentation_params(&mut writer, &params, &seg, mfh.as_ref()).is_err() {
                prop_assert_eq!(writer.bit_len(), 0);
            }
        }
    }
}

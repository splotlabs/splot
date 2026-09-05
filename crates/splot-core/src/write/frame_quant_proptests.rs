// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod proptests {
    use super::*;
    use crate::bitio::BitReader;
    use crate::headers::frame::{
        QmSetLevels, parse_delta_q_params, parse_lossless_info, parse_quantization_params,
        parse_setup_qm_params, read_delta_q,
    };
    use crate::segment::{SEG_LVL_MAX, SegmentFeature};
    use crate::span::ByteOffset;
    use proptest::prelude::*;

    fn arbitrary_quant_view() -> impl Strategy<Value = CoreSeqQuantView> {
        (
            prop_oneof![Just(8u8), Just(10u8)],
            prop_oneof![Just(1u8), Just(3u8)],
            any::<[bool; 5]>(),
            (-4i32..=4, -4i32..=4, -4i32..=4),
            any::<[bool; 3]>(),
        )
            .prop_map(
                |(bit_depth, num_planes, flags, bases, tcq)| CoreSeqQuantView {
                    bit_depth,
                    num_planes,
                    separate_uv_delta_q: flags[0],
                    equal_ac_dc_q: flags[1],
                    y_dc_delta_q_enabled: flags[2],
                    uv_dc_delta_q_enabled: flags[3],
                    uv_ac_delta_q_enabled: flags[4],
                    base_y_dc_delta_q: bases.0,
                    base_uv_dc_delta_q: bases.1,
                    base_uv_ac_delta_q: bases.2,
                    enable_tcq: tcq[0],
                    choose_tcq_per_frame: tcq[1],
                    enable_parity_hiding: tcq[2],
                },
            )
    }

    fn pack(bits: &[bool]) -> Vec<u8> {
        let mut out = crate::test_bits::Bits::default();
        for &bit in bits {
            out.bit(u8::from(bit));
        }
        let mut bytes = out.into_bytes();
        bytes.extend_from_slice(&[0u8; 8]);
        bytes
    }

    fn reader(bytes: &[u8]) -> BitReader<'_> {
        BitReader::new(bytes, ByteOffset::new(0))
    }

    proptest! {
        /// Every `read_delta_q` value in the su(7) domain round-trips byte-exactly.
        #[test]
        fn read_delta_q_round_trips(value in -64i32..=63) {
            let mut writer = BitWriter::new();
            write_read_delta_q(&mut writer, value).unwrap();
            let bytes = writer.into_bytes();
            prop_assert_eq!(read_delta_q(&mut reader(&bytes)).unwrap(), value);
        }

        /// Every parser-reachable quantization_params round-trips: parse random bits +
        /// gating, then re-emit and reparse to the same model.
        #[test]
        fn quantization_params_round_trips(
            quant in arbitrary_quant_view(),
            tip in any::<bool>(),
            bits in proptest::collection::vec(any::<bool>(), 0..40),
        ) {
            let packed = pack(&bits);
            if let Ok(params) = parse_quantization_params(&mut reader(&packed), &quant, tip) {
                let mut writer = BitWriter::new();
                write_quantization_params(&mut writer, &params, &quant, tip).unwrap();
                let written = writer.into_bytes();
                let reparsed =
                    parse_quantization_params(&mut reader(&written), &quant, tip).unwrap();
                prop_assert_eq!(reparsed, params);
            }
        }

        /// Every parser-reachable setup_qm_params round-trips.
        #[test]
        fn setup_qm_params_round_trips(
            quant in arbitrary_quant_view(),
            seg_enabled in any::<bool>(),
            bits in proptest::collection::vec(any::<bool>(), 0..40),
        ) {
            let packed = pack(&bits);
            if let Ok(qm) = parse_setup_qm_params(&mut reader(&packed), &quant, seg_enabled) {
                let mut writer = BitWriter::new();
                write_setup_qm_params(&mut writer, &qm, &quant, seg_enabled).unwrap();
                let written = writer.into_bytes();
                let reparsed =
                    parse_setup_qm_params(&mut reader(&written), &quant, seg_enabled).unwrap();
                prop_assert_eq!(reparsed, qm);
            }
        }

        /// Every parser-reachable delta_q_params round-trips.
        #[test]
        fn delta_q_params_round_trips(
            base_q_idx in any::<u32>(),
            bits in proptest::collection::vec(any::<bool>(), 0..8),
        ) {
            let packed = pack(&bits);
            if let Ok(dq) = parse_delta_q_params(&mut reader(&packed), base_q_idx) {
                let mut writer = BitWriter::new();
                write_delta_q_params(&mut writer, &dq, base_q_idx).unwrap();
                let written = writer.into_bytes();
                let reparsed = parse_delta_q_params(&mut reader(&written), base_q_idx).unwrap();
                prop_assert_eq!(reparsed, dq);
            }
        }

        /// Every parser-reachable lossless_info round-trips, across using_qmatrix on/off,
        /// random segment ALT_Q features, and the allow_tcq / allow_parity_hiding gates.
        #[test]
        fn lossless_info_round_trips(
            quant in arbitrary_quant_view(),
            base_q_idx in 0u32..=64,
            deltas in (-4i32..=4, -4i32..=4, -4i32..=4, -4i32..=4, -4i32..=4),
            using_qmatrix in any::<bool>(),
            pic_qm_num_minus_1 in 0u8..4,
            seg_enabled in any::<bool>(),
            alt_q in (any::<bool>(), -64i32..=64),
            delta_q_present in any::<bool>(),
            qm_levels in proptest::collection::vec((0u8..16, 0u8..16, 0u8..16), 4),
            max_segments in prop_oneof![Just(8u8), Just(16u8)],
            bits in proptest::collection::vec(any::<bool>(), 0..40),
        ) {
            let quantization = QuantizationParams {
                base_q_idx,
                delta_q_y_dc: deltas.0,
                delta_q_u_dc: deltas.1,
                delta_q_u_ac: deltas.2,
                delta_q_v_dc: deltas.3,
                delta_q_v_ac: deltas.4,
                diff_uv_delta: false,
            };
            let pic_qm_num_minus_1 = if using_qmatrix && seg_enabled {
                pic_qm_num_minus_1
            } else {
                0
            };
            let qm_num = usize::from(pic_qm_num_minus_1) + 1;
            let mut levels = [QmSetLevels::default(); MAX_PIC_QM_NUM];
            if using_qmatrix {
                for (i, (slot, (y, u, v))) in levels.iter_mut().zip(qm_levels.iter()).enumerate() {
                    if i >= qm_num {
                        break; // levels at/beyond qmNum stay the zeroed default
                    }
                    let (qm_y, mut qm_u, mut qm_v) = (*y, *u, *v);
                    if quant.num_planes <= 1 {
                        qm_u = 0;
                        qm_v = 0;
                    } else if !quant.separate_uv_delta_q {
                        qm_v = qm_u;
                    }
                    *slot = QmSetLevels { qm_y, qm_u, qm_v };
                }
            }
            let qm = SetupQmParams {
                using_qmatrix,
                pic_qm_num_minus_1,
                levels,
            };
            let delta_q = DeltaQParams { delta_q_present, delta_q_res: 0 };
            let mut segmentation = SegmentationParams {
                segmentation_enabled: seg_enabled,
                reuse_seg_info: false,
                features: [[SegmentFeature::DISABLED; SEG_LVL_MAX]; MAX_SEGMENTS],
                segmentation_update_map: seg_enabled,
                segmentation_temporal_update: false,
                seg_id_pre_skip: false,
                last_active_seg_id: 0,
            };
            segmentation.features[0][0] = SegmentFeature { enabled: alt_q.0, data: alt_q.1 };

            let packed = pack(&bits);
            if let Ok(info) = parse_lossless_info(
                &mut reader(&packed),
                &quant,
                &quantization,
                &qm,
                &delta_q,
                &segmentation,
                max_segments,
            ) {
                let mut writer = BitWriter::new();
                write_lossless_info(
                    &mut writer,
                    &info,
                    &quant,
                    &quantization,
                    &qm,
                    &delta_q,
                    &segmentation,
                    max_segments,
                )
                .unwrap();
                let written = writer.into_bytes();
                let reparsed = parse_lossless_info(
                    &mut reader(&written),
                    &quant,
                    &quantization,
                    &qm,
                    &delta_q,
                    &segmentation,
                    max_segments,
                )
                .unwrap();
                prop_assert_eq!(reparsed, info);
            }
        }
    }
}

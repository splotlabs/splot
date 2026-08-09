// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>


#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod proptests {
    use super::*;
    use crate::bitio::BitReader;
    use crate::headers::frame::{
        CcsoPlaneParams, LrPlaneParams, parse_ccso_params, parse_lr_params,
    };
    use crate::headers::sequence::{ChromaFormatIdc, SuperblockSize};
    use crate::span::ByteOffset;
    use proptest::prelude::*;

    fn pack(bits: &[bool]) -> Vec<u8> {
        let mut out = Vec::new();
        for chunk in bits.chunks(8) {
            let mut byte = 0u8;
            for (i, b) in chunk.iter().enumerate() {
                byte |= u8::from(*b) << (7 - i);
            }
            out.push(byte);
        }
        out.extend_from_slice(&[0u8; 16]); // pad so the parser never hits EOF mid-field
        out
    }

    fn reader(bytes: &[u8]) -> BitReader<'_> {
        BitReader::new(bytes, ByteOffset::new(0))
    }

    fn arbitrary_sb_size() -> impl Strategy<Value = SuperblockSize> {
        prop_oneof![
            Just(SuperblockSize::Block64x64),
            Just(SuperblockSize::Block128x128),
            Just(SuperblockSize::Block256x256),
        ]
    }

    fn arbitrary_chroma() -> impl Strategy<Value = ChromaFormatIdc> {
        prop_oneof![
            Just(ChromaFormatIdc::Yuv420),
            Just(ChromaFormatIdc::Monochrome),
            Just(ChromaFormatIdc::Yuv444),
            Just(ChromaFormatIdc::Yuv422),
        ]
    }

    proptest! {

        /// Every parser-reachable lr_params on the writer-supported surface round-trips: parse
        /// random bits + gating, then re-emit and reparse to the same model. Parsed
        /// frame-level Wiener NS banks are skipped because the writer still treats
        /// `frame_filters_on` as a hard residual.
        #[test]
        fn lr_round_trips(
            coded_lossless in any::<bool>(),
            enable_restoration in any::<bool>(),
            num_planes in prop_oneof![Just(1u8), Just(3u8)],
            lr_pc_wiener_disabled in any::<bool>(),
            lr_wiener_nonsep_disabled in any::<bool>(),
            lr_uv_wiener_nonsep_disabled in any::<bool>(),
            sb_size in arbitrary_sb_size(),
            chroma in arbitrary_chroma(),
            bits in proptest::collection::vec(any::<bool>(), 0..24),
        ) {
            let view = CoreSeqRestorationView {
                enable_restoration,
                lr_pc_wiener_disabled,
                lr_wiener_nonsep_disabled,
                lr_uv_pc_wiener_disabled: enable_restoration,
                lr_uv_wiener_nonsep_disabled,
            };
            let geometry = LrGeometry::new(sb_size, chroma);
            let packed = pack(&bits);
            if let Ok(params) = parse_lr_params(
                &mut reader(&packed),
                coded_lossless,
                num_planes,
                &view,
                geometry,
            ) {
                if params.planes.iter().any(|plane| {
                    plane.frame_filters_on || plane.frame_filter_bank.is_some()
                }) {
                    return Ok(());
                }
                let mut writer = BitWriter::new();
                write_lr_params(
                    &mut writer,
                    &params,
                    coded_lossless,
                    num_planes,
                    &view,
                    geometry,
                )
                .unwrap();
                let written = writer.into_bytes();
                let reparsed = parse_lr_params(
                    &mut reader(&written),
                    coded_lossless,
                    num_planes,
                    &view,
                    geometry,
                )
                .unwrap();
                prop_assert_eq!(reparsed, params);
            }
        }

        /// The LR writer never panics on an arbitrary (possibly invalid) model + gating, and on
        /// Err leaves the writer empty.
        #[test]
        fn lr_writer_never_panics_on_constructed_models(
            uses_lr in any::<bool>(),
            planes in proptest::collection::vec(
                (
                    prop_oneof![
                        Just(FrameRestorationType::None),
                        Just(FrameRestorationType::PcWiener),
                        Just(FrameRestorationType::WienerNonsep),
                        Just(FrameRestorationType::Switchable),
                    ],
                    any::<bool>(),
                    proptest::option::of(any::<u8>()),
                ),
                0..4,
            ),
            loop_restoration_size in any::<[u32; 3]>(),
            coded_lossless in any::<bool>(),
            enable_restoration in any::<bool>(),
            num_planes in prop_oneof![Just(1u8), Just(3u8)],
            lr_pc_wiener_disabled in any::<bool>(),
            lr_wiener_nonsep_disabled in any::<bool>(),
            lr_uv_pc_wiener_disabled in any::<bool>(),
            lr_uv_wiener_nonsep_disabled in any::<bool>(),
            sb_size in arbitrary_sb_size(),
            chroma in arbitrary_chroma(),
        ) {
            let view = CoreSeqRestorationView {
                enable_restoration,
                lr_pc_wiener_disabled,
                lr_wiener_nonsep_disabled,
                lr_uv_pc_wiener_disabled,
                lr_uv_wiener_nonsep_disabled,
            };
            let geometry = LrGeometry::new(sb_size, chroma);
            let planes = planes
                .into_iter()
                .map(|(restoration_type, frame_filters_on, num_filter_classes)| LrPlaneParams {
                    restoration_type,
                    frame_filters_on,
                    num_filter_classes,
                    frame_filter_bank: None,
                })
                .collect();
            let params = LrParams {
                uses_lr,
                planes,
                loop_restoration_size,
            };
            let mut writer = BitWriter::new();
            let result = write_lr_params(
                &mut writer,
                &params,
                coded_lossless,
                num_planes,
                &view,
                geometry,
            );
            if result.is_err() {
                prop_assert_eq!(writer.bit_len(), 0);
            }
        }


        /// Every parser-reachable ccso_params round-trips: parse random bits + gating, then
        /// re-emit and reparse to the same model.
        #[test]
        fn ccso_round_trips(
            coded_lossless in any::<bool>(),
            enable_ccso in any::<bool>(),
            num_planes in prop_oneof![Just(1u8), Just(3u8)],
            single_picture_header_flag in any::<bool>(),
            bits in proptest::collection::vec(any::<bool>(), 0..160),
        ) {
            let view = CoreSeqCcsoView {
                enable_ccso,
                single_picture_header_flag,
            };
            let packed = pack(&bits);
            if let Ok(params) =
                parse_ccso_params(&mut reader(&packed), coded_lossless, num_planes, &view)
            {
                let mut writer = BitWriter::new();
                write_ccso_params(&mut writer, &params, coded_lossless, num_planes, &view).unwrap();
                let written = writer.into_bytes();
                let reparsed =
                    parse_ccso_params(&mut reader(&written), coded_lossless, num_planes, &view)
                        .unwrap();
                prop_assert_eq!(reparsed, params);
            }
        }

        /// The CCSO writer never panics on an arbitrary (possibly invalid) model + gating, and on
        /// Err leaves the writer empty.
        #[test]
        fn ccso_writer_never_panics_on_constructed_models(
            ccso_frame_flag in proptest::option::of(any::<bool>()),
            planes in proptest::collection::vec(
                (
                    any::<bool>(),
                    proptest::option::of(any::<bool>()),
                    proptest::option::of(any::<u8>()),
                    proptest::option::of(any::<u8>()),
                    proptest::option::of(any::<u8>()),
                    proptest::option::of(any::<bool>()),
                    proptest::option::of(any::<u8>()),
                    proptest::collection::vec(any::<u8>(), 0..16),
                ),
                0..4,
            ),
            coded_lossless in any::<bool>(),
            enable_ccso in any::<bool>(),
            num_planes in prop_oneof![Just(1u8), Just(3u8)],
            single_picture_header_flag in any::<bool>(),
        ) {
            let view = CoreSeqCcsoView {
                enable_ccso,
                single_picture_header_flag,
            };
            let planes = planes
                .into_iter()
                .map(
                    |(
                        ccso_planes,
                        ccso_bo_only,
                        ccso_scale_idx,
                        ccso_quant_idx,
                        ccso_ext_filter,
                        ccso_edge_clf,
                        ccso_max_band_log2,
                        ccso_offset_idx,
                    )| CcsoPlaneParams {
                        reuse_ccso: false,
                        sb_reuse_ccso: false,
                        ccso_ref_idx: None,
                        ccso_planes,
                        ccso_bo_only,
                        ccso_scale_idx,
                        ccso_quant_idx,
                        ccso_ext_filter,
                        ccso_edge_clf,
                        ccso_max_band_log2,
                        ccso_offset_idx,
                    },
                )
                .collect();
            let params = CcsoParams {
                ccso_frame_flag,
                planes,
            };
            let mut writer = BitWriter::new();
            let result =
                write_ccso_params(&mut writer, &params, coded_lossless, num_planes, &view);
            if result.is_err() {
                prop_assert_eq!(writer.bit_len(), 0);
            }
        }
    }
}

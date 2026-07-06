// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>


#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::too_many_lines)]
mod tests {
    use super::*;
    use crate::bitio::BitReader;
    use crate::headers::frame::{
        CcsoPlaneParams, LrParseOutcome, LrPlaneParams, parse_ccso_params, parse_lr_params,
        WienerNsFrameFilterBank, WienerNsFrameFilterClass,
    };
    use crate::headers::sequence::{ChromaFormatIdc, SuperblockSize};
    use crate::span::ByteOffset;

    use crate::test_bits::Bits;

    fn reader(bytes: &[u8]) -> BitReader<'_> {
        BitReader::new(bytes, ByteOffset::new(0))
    }

    fn restoration_enabled() -> CoreSeqRestorationView {
        CoreSeqRestorationView {
            enable_restoration: true,
            lr_pc_wiener_disabled: false,
            lr_wiener_nonsep_disabled: false,
            lr_uv_pc_wiener_disabled: true,
            lr_uv_wiener_nonsep_disabled: false,
        }
    }

    fn ccso_enabled() -> CoreSeqCcsoView {
        CoreSeqCcsoView {
            enable_ccso: true,
            single_picture_header_flag: false,
        }
    }

    fn geom(sb: SuperblockSize, chroma: ChromaFormatIdc) -> LrGeometry {
        LrGeometry::new(sb, chroma)
    }

    /// Parse `data` as `lr_params()`, expecting a complete `Parsed`, re-emit it via the writer,
    /// and reparse the bytes — asserting the model round-trips and the bytes are byte-exact.
    fn lr_round_trip(
        data: &[u8],
        num_planes: u8,
        view: CoreSeqRestorationView,
        geometry: LrGeometry,
    ) -> LrParams {
        let parsed = match parse_lr_params(&mut reader(data), false, num_planes, &view, geometry, 99)
            .unwrap()
        {
            LrParseOutcome::Parsed(p) => p,
            other @ LrParseOutcome::StoppedBeforeWienerNsFilter { .. } => {
                panic!("expected Parsed, got {other:?}")
            }
        };
        let mut writer = BitWriter::new();
        write_lr_params(&mut writer, &parsed, false, num_planes, &view, geometry, 99).unwrap();
        let written = writer.into_bytes();
        let reparsed =
            match parse_lr_params(&mut reader(&written), false, num_planes, &view, geometry, 99)
                .unwrap()
            {
                LrParseOutcome::Parsed(p) => p,
                other @ LrParseOutcome::StoppedBeforeWienerNsFilter { .. } => {
                    panic!("reparse expected Parsed, got {other:?}")
                }
            };
        assert_eq!(reparsed, parsed);
        parsed
    }

    fn lr_plane(restoration_type: FrameRestorationType) -> LrPlaneParams {
        LrPlaneParams {
            restoration_type,
            frame_filters_on: false,
            num_filter_classes: None,
            frame_filter_bank: None,
        }
    }

    fn lr_params(
        uses_lr: bool,
        planes: Vec<LrPlaneParams>,
        loop_restoration_size: [u32; 3],
    ) -> LrParams {
        LrParams {
            uses_lr,
            planes,
            loop_restoration_size,
        }
    }

    fn assert_lr_rejected(
        params: &LrParams,
        num_planes: u8,
        geometry: LrGeometry,
        expected: &'static str,
    ) {
        let mut writer = BitWriter::new();
        let result = write_lr_params(
            &mut writer,
            params,
            false,
            num_planes,
            &restoration_enabled(),
            geometry,
            0,
        );
        assert!(
            matches!(
                &result,
                Err(WriteError::NonCanonicalFrameHeader { what }) if *what == expected
            ),
            "expected {expected}, got {result:?}"
        );
        assert_eq!(writer.bit_len(), 0);
    }


    #[test]
    fn lr_disabled_writes_no_bits() {
        for (coded_lossless, enable) in [(true, true), (false, false)] {
            let mut view = restoration_enabled();
            view.enable_restoration = enable;
            let geometry = geom(SuperblockSize::Block128x128, ChromaFormatIdc::Yuv420);
            let params = LrParams {
                uses_lr: false,
                planes: Vec::new(),
                loop_restoration_size: default_restoration_size(geometry),
            };
            let mut writer = BitWriter::new();
            write_lr_params(&mut writer, &params, coded_lossless, 3, &view, geometry, 7).unwrap();
            assert_eq!(writer.bit_len(), 0);
        }
    }

    #[test]
    fn lr_disabled_non_default_size_is_rejected() {
        let view = {
            let mut v = restoration_enabled();
            v.enable_restoration = false;
            v
        };
        let geometry = geom(SuperblockSize::Block128x128, ChromaFormatIdc::Yuv420);
        let mut bad = default_restoration_size(geometry);
        bad[0] += 1;
        let params = LrParams {
            uses_lr: false,
            planes: Vec::new(),
            loop_restoration_size: bad,
        };
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_lr_params(&mut writer, &params, false, 3, &view, geometry, 0),
            Err(WriteError::NonCanonicalFrameHeader {
                what: "lr_disabled"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }


    #[test]
    fn lr_all_restore_none_round_trips() {
        let mut bits = Bits::default();
        bits.ns(0, 4); // plane 0 RESTORE_NONE
        bits.ns(0, 2); // plane 1 RESTORE_NONE
        bits.ns(0, 2); // plane 2 RESTORE_NONE
        let data = bits.into_bytes();
        let geometry = geom(SuperblockSize::Block128x128, ChromaFormatIdc::Yuv420);
        let p = lr_round_trip(&data, 3, restoration_enabled(), geometry);
        assert!(!p.uses_lr);
    }


    #[test]
    fn lr_luma_size_shifts_round_trip_each_sb_size() {
        struct Case {
            sb: SuperblockSize,
            flags: &'static [u8],
        }
        let block256 = [
            Case { sb: SuperblockSize::Block256x256, flags: &[1] },       // shift 1
            Case { sb: SuperblockSize::Block256x256, flags: &[0] },       // shift 0
        ];
        let block128 = [
            Case { sb: SuperblockSize::Block128x128, flags: &[1] },       // shift 1
            Case { sb: SuperblockSize::Block128x128, flags: &[0, 1] },    // shift 0
            Case { sb: SuperblockSize::Block128x128, flags: &[0, 0] },    // shift 2
        ];
        let block64 = [
            Case { sb: SuperblockSize::Block64x64, flags: &[1] },         // shift 1
            Case { sb: SuperblockSize::Block64x64, flags: &[0, 1] },      // shift 0
            Case { sb: SuperblockSize::Block64x64, flags: &[0, 0, 1] },   // shift 2
            Case { sb: SuperblockSize::Block64x64, flags: &[0, 0, 0] },   // shift 3
        ];
        let all: Vec<&Case> = block256
            .iter()
            .chain(block128.iter())
            .chain(block64.iter())
            .collect();
        for case in all {
            let mut bits = Bits::default();
            bits.ns(1, 4); // plane 0 PC_WIENER -> usesLumaLr
            bits.ns(0, 2); // plane 1 NONE
            bits.ns(0, 2); // plane 2 NONE
            for &flag in case.flags {
                bits.bit(flag);
            }
            let data = bits.into_bytes();
            let geometry = geom(case.sb, ChromaFormatIdc::Yuv420);
            let p = lr_round_trip(&data, 3, restoration_enabled(), geometry);
            assert_eq!(
                p.planes[0].restoration_type,
                FrameRestorationType::PcWiener
            );
        }
    }

    #[test]
    fn lr_chroma_only_round_trips() {
        let mut bits = Bits::default();
        bits.ns(0, 4); // plane 0 NONE (luma)
        bits.ns(1, 3); // plane 1 WIENER_NONSEP (chroma)
        bits.bit(0); // frame_filters_on[1] = 0
        bits.ns(0, 3); // plane 2 NONE (chroma)
        bits.bit(1);
        let data = bits.into_bytes();
        let geometry = geom(SuperblockSize::Block128x128, ChromaFormatIdc::Yuv420);
        let p = lr_round_trip(&data, 3, restoration_enabled(), geometry);
        assert!(p.uses_lr);
        assert_eq!(p.planes[0].restoration_type, FrameRestorationType::None);
        assert_eq!(
            p.planes[1].restoration_type,
            FrameRestorationType::WienerNonsep
        );
    }

    #[test]
    fn lr_luma_and_chroma_round_trips() {
        let mut bits = Bits::default();
        bits.ns(1, 4); // plane 0 PC_WIENER
        bits.ns(1, 3); // plane 1 WIENER_NONSEP (chroma indexToTool n = 3)
        bits.bit(0); // frame_filters_on[1] = 0
        bits.ns(0, 3); // plane 2 NONE
        bits.bit(1); // luma half size
        bits.bit(1); // chroma half size
        let data = bits.into_bytes();
        let geometry = geom(SuperblockSize::Block128x128, ChromaFormatIdc::Yuv420);
        let p = lr_round_trip(&data, 3, restoration_enabled(), geometry);
        assert!(p.uses_lr);
    }

    #[test]
    fn lr_monochrome_single_plane_round_trips() {
        let mut bits = Bits::default();
        bits.ns(1, 4); // plane 0 PC_WIENER
        bits.bit(1); // luma half size
        let data = bits.into_bytes();
        let geometry = geom(SuperblockSize::Block128x128, ChromaFormatIdc::Monochrome);
        let p = lr_round_trip(&data, 1, restoration_enabled(), geometry);
        assert_eq!(p.planes.len(), 1);
    }

    #[test]
    fn lr_switchable_writes_frame_filters_zero_bit() {
        let mut bits = Bits::default();
        bits.ns(3, 4); // plane 0 SWITCHABLE (indexToTool[3])
        bits.bit(0); // frame_filters_on[0] = 0
        bits.ns(0, 2); // plane 1 NONE
        bits.ns(0, 2); // plane 2 NONE
        bits.bit(1); // luma half size
        let data = bits.into_bytes();
        let geometry = geom(SuperblockSize::Block64x64, ChromaFormatIdc::Yuv420);
        let p = lr_round_trip(&data, 3, restoration_enabled(), geometry);
        assert_eq!(
            p.planes[0].restoration_type,
            FrameRestorationType::Switchable
        );
    }


    #[test]
    fn lr_frame_filters_on_is_rejected_hard_residual() {
        let geometry = geom(SuperblockSize::Block128x128, ChromaFormatIdc::Yuv420);
        let mut plane = lr_plane(FrameRestorationType::WienerNonsep);
        plane.frame_filters_on = true;
        let params = lr_params(
            true,
            vec![
                plane,
                lr_plane(FrameRestorationType::None),
                lr_plane(FrameRestorationType::None),
            ],
            [256, 32, 32],
        );
        assert_lr_rejected(&params, 3, geometry, "lr_frame_filters_on");
    }

    #[test]
    fn lr_frame_filter_bank_is_rejected_until_writer_support() {
        let geometry = geom(SuperblockSize::Block128x128, ChromaFormatIdc::Yuv420);
        let params = LrParams {
            uses_lr: false,
            planes: vec![LrPlaneParams {
                restoration_type: FrameRestorationType::None,
                frame_filters_on: false,
                num_filter_classes: None,
                frame_filter_bank: Some(WienerNsFrameFilterBank {
                    classes: vec![WienerNsFrameFilterClass {
                        match_index: 0,
                        merged: true,
                        ref_bank: 0,
                        subset: None,
                        wiener_ns_uv_sym: false,
                        coeffs: vec![0; 16],
                    }],
                }),
            }],
            loop_restoration_size: [64, 32, 32],
        };
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_lr_params(&mut writer, &params, false, 1, &restoration_enabled(), geometry, 0),
            Err(WriteError::NonCanonicalFrameHeader {
                what: "lr_frame_filter_bank"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn lr_num_planes_mismatch_is_rejected() {
        let geometry = geom(SuperblockSize::Block128x128, ChromaFormatIdc::Yuv420);
        let params = LrParams {
            uses_lr: false,
            planes: vec![LrPlaneParams {
                restoration_type: FrameRestorationType::None,
                frame_filters_on: false,
                num_filter_classes: None,
                frame_filter_bank: None,
            }],
            loop_restoration_size: [64, 32, 32],
        };
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_lr_params(&mut writer, &params, false, 3, &restoration_enabled(), geometry, 0),
            Err(WriteError::NonCanonicalFrameHeader {
                what: "lr_num_planes"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn lr_tool_not_in_table_is_rejected() {
        let geometry = geom(SuperblockSize::Block128x128, ChromaFormatIdc::Yuv420);
        let params = lr_params(
            true,
            vec![
                lr_plane(FrameRestorationType::None),
                lr_plane(FrameRestorationType::PcWiener),
                lr_plane(FrameRestorationType::None),
            ],
            [64, 32, 32],
        );
        assert_lr_rejected(&params, 3, geometry, "lr_tool_index");
    }

    #[test]
    fn lr_switchable_when_not_allowed_is_rejected() {
        let geometry = geom(SuperblockSize::Block128x128, ChromaFormatIdc::Yuv420);
        let params = lr_params(
            true,
            vec![
                lr_plane(FrameRestorationType::None),
                lr_plane(FrameRestorationType::Switchable),
                lr_plane(FrameRestorationType::None),
            ],
            [64, 32, 32],
        );
        assert_lr_rejected(&params, 3, geometry, "lr_tool_index");
    }

    #[test]
    fn lr_out_of_range_subsampling_geometry_is_rejected() {
        let geometry = LrGeometry {
            sb_size: SuperblockSize::Block128x128,
            subsampling_x: 200,
            subsampling_y: 200,
        };
        let params = LrParams {
            uses_lr: false,
            planes: vec![
                LrPlaneParams {
                    restoration_type: FrameRestorationType::None,
                    frame_filters_on: false,
                    num_filter_classes: None,
                    frame_filter_bank: None,
                },
                LrPlaneParams {
                    restoration_type: FrameRestorationType::None,
                    frame_filters_on: false,
                    num_filter_classes: None,
                    frame_filter_bank: None,
                },
                LrPlaneParams {
                    restoration_type: FrameRestorationType::None,
                    frame_filters_on: false,
                    num_filter_classes: None,
                    frame_filter_bank: None,
                },
            ],
            loop_restoration_size: [64, 32, 32],
        };
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_lr_params(&mut writer, &params, false, 3, &restoration_enabled(), geometry, 0),
            Err(WriteError::NonCanonicalFrameHeader { what: "lr_size" })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn lr_num_filter_classes_some_is_rejected() {
        let geometry = geom(SuperblockSize::Block128x128, ChromaFormatIdc::Yuv420);
        let mut plane = lr_plane(FrameRestorationType::WienerNonsep);
        plane.num_filter_classes = Some(6);
        let params = lr_params(
            true,
            vec![
                plane,
                lr_plane(FrameRestorationType::None),
                lr_plane(FrameRestorationType::None),
            ],
            [256, 32, 32],
        );
        assert_lr_rejected(&params, 3, geometry, "lr_num_filter_classes");
    }

    #[test]
    fn lr_unreachable_shift_for_sb_size_is_rejected() {
        let geometry = geom(SuperblockSize::Block256x256, ChromaFormatIdc::Yuv420);
        let params = lr_params(
            true,
            vec![
                lr_plane(FrameRestorationType::PcWiener),
                lr_plane(FrameRestorationType::None),
                lr_plane(FrameRestorationType::None),
            ],
            [128, 32, 32],
        );
        assert_lr_rejected(&params, 3, geometry, "lr_size");
    }

    #[test]
    fn lr_non_power_of_two_size_is_rejected() {
        let geometry = geom(SuperblockSize::Block64x64, ChromaFormatIdc::Yuv420);
        let params = lr_params(
            true,
            vec![
                lr_plane(FrameRestorationType::PcWiener),
                lr_plane(FrameRestorationType::None),
                lr_plane(FrameRestorationType::None),
            ],
            [300, 32, 32],
        );
        assert_lr_rejected(&params, 3, geometry, "lr_size");
    }

    #[test]
    fn lr_uses_lr_mismatch_is_rejected() {
        let geometry = geom(SuperblockSize::Block128x128, ChromaFormatIdc::Yuv420);
        let params = lr_params(
            true,
            vec![
                lr_plane(FrameRestorationType::None),
                lr_plane(FrameRestorationType::None),
                lr_plane(FrameRestorationType::None),
            ],
            [64, 32, 32],
        );
        assert_lr_rejected(&params, 3, geometry, "lr_uses_lr");
    }

    #[test]
    fn lr_plane2_size_mismatch_is_rejected() {
        // loop_restoration_size[2] must equal [1]; all NONE so both unused/default.
        let geometry = geom(SuperblockSize::Block128x128, ChromaFormatIdc::Yuv420);
        let params = LrParams {
            uses_lr: false,
            planes: vec![
                LrPlaneParams {
                    restoration_type: FrameRestorationType::None,
                    frame_filters_on: false,
                    num_filter_classes: None,
                    frame_filter_bank: None,
                },
                LrPlaneParams {
                    restoration_type: FrameRestorationType::None,
                    frame_filters_on: false,
                    num_filter_classes: None,
                    frame_filter_bank: None,
                },
                LrPlaneParams {
                    restoration_type: FrameRestorationType::None,
                    frame_filters_on: false,
                    num_filter_classes: None,
                    frame_filter_bank: None,
                },
            ],
            loop_restoration_size: [64, 32, 16],
        };
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_lr_params(&mut writer, &params, false, 3, &restoration_enabled(), geometry, 0),
            Err(WriteError::NonCanonicalFrameHeader { what: "lr_size" })
        ));
        assert_eq!(writer.bit_len(), 0);
    }


    /// Parse `data` as `ccso_params()`, re-emit, and reparse — asserting the model round-trips.
    fn ccso_round_trip(
        data: &[u8],
        num_planes: u8,
        view: CoreSeqCcsoView,
    ) -> CcsoParams {
        let parsed = parse_ccso_params(&mut reader(data), false, num_planes, &view).unwrap();
        let mut writer = BitWriter::new();
        write_ccso_params(&mut writer, &parsed, false, num_planes, &view).unwrap();
        let written = writer.into_bytes();
        let reparsed =
            parse_ccso_params(&mut reader(&written), false, num_planes, &view).unwrap();
        assert_eq!(reparsed, parsed);
        parsed
    }

    #[test]
    fn ccso_disabled_writes_no_bits() {
        for (coded_lossless, enable) in [(true, true), (false, false)] {
            let mut view = ccso_enabled();
            view.enable_ccso = enable;
            let params = CcsoParams {
                ccso_frame_flag: None,
                planes: Vec::new(),
            };
            let mut writer = BitWriter::new();
            write_ccso_params(&mut writer, &params, coded_lossless, 3, &view).unwrap();
            assert_eq!(writer.bit_len(), 0);
        }
    }

    #[test]
    fn ccso_frame_flag_zero_round_trips() {
        let mut bits = Bits::default();
        bits.bit(0); // ccso_frame_flag == 0
        let data = bits.into_bytes();
        let p = ccso_round_trip(&data, 3, ccso_enabled());
        assert_eq!(p.ccso_frame_flag, Some(false));
    }

    #[test]
    fn ccso_single_picture_inferred_round_trips() {
        let mut view = ccso_enabled();
        view.single_picture_header_flag = true;
        let mut bits = Bits::default();
        bits.bit(0); // ccso_planes[0]
        bits.bit(0); // ccso_planes[1]
        bits.bit(0); // ccso_planes[2]
        let data = bits.into_bytes();
        let p = ccso_round_trip(&data, 3, view);
        assert_eq!(p.ccso_frame_flag, Some(true));
        let mut writer = BitWriter::new();
        write_ccso_params(&mut writer, &p, false, 3, &view).unwrap();
        assert_eq!(writer.bit_len(), 3);
    }

    #[test]
    fn ccso_all_planes_off_round_trips() {
        let mut bits = Bits::default();
        bits.bit(1); // ccso_frame_flag
        bits.bit(0); // ccso_planes[0]
        bits.bit(0); // ccso_planes[1]
        bits.bit(0); // ccso_planes[2]
        let data = bits.into_bytes();
        ccso_round_trip(&data, 3, ccso_enabled());
    }

    #[test]
    fn ccso_bo_only_plane_round_trips() {
        let mut bits = Bits::default();
        bits.bit(1); // ccso_frame_flag
        bits.bit(1); // ccso_planes[0]
        bits.bit(1); // ccso_bo_only[0]
        bits.f(0, 2); // ccso_scale_idx[0]
        bits.f(2, 3); // ccso_max_band_log2[0] == 2 (n = 3) -> maxBand 4
        bits.tu(0, 7);
        bits.tu(1, 7);
        bits.tu(2, 7);
        bits.tu(7, 7);
        bits.bit(0); // ccso_planes[1]
        bits.bit(0); // ccso_planes[2]
        let data = bits.into_bytes();
        let p = ccso_round_trip(&data, 3, ccso_enabled());
        assert_eq!(p.planes[0].ccso_offset_idx, vec![0, 1, 2, 7]);
    }

    #[test]
    fn ccso_full_arm_plane_round_trips() {
        let mut bits = Bits::default();
        bits.bit(1); // ccso_frame_flag
        bits.bit(1); // ccso_planes[0]
        bits.bit(0); // ccso_bo_only == 0
        bits.f(1, 2); // ccso_scale_idx == 1
        bits.f(0, 2); // ccso_quant_idx == 0 -> CCSO_Quant_Sz[1][0] == 56 != 0
        bits.f(5, 3); // ccso_ext_filter == 5
        bits.bit(1); // ccso_edge_clf == 1
        bits.f(0, 2); // ccso_max_band_log2 == 0 -> maxBand 1
        for _ in 0..4 {
            bits.tu(3, 7); // maxEdgeInterval 2 * 2 * 1 = 4 offsets
        }
        bits.bit(0); // ccso_planes[1]
        bits.bit(0); // ccso_planes[2]
        let data = bits.into_bytes();
        let p = ccso_round_trip(&data, 3, ccso_enabled());
        assert_eq!(p.planes[0].ccso_edge_clf, Some(true));
        assert_eq!(p.planes[0].ccso_offset_idx.len(), 4);
    }

    #[test]
    fn ccso_quant_step_zero_suppresses_edge_clf_round_trips() {
        let mut bits = Bits::default();
        bits.bit(1); // ccso_frame_flag
        bits.bit(1); // ccso_planes[0]
        bits.bit(0); // ccso_bo_only == 0
        bits.f(0, 2); // ccso_scale_idx == 0
        bits.f(3, 2); // ccso_quant_idx == 3 -> quantStep 0
        bits.f(0, 3); // ccso_ext_filter
        bits.f(0, 2); // ccso_max_band_log2 == 0 -> maxBand 1
        for _ in 0..9 {
            bits.tu(0, 7); // 3*3*1 = 9 offsets
        }
        bits.bit(0); // ccso_planes[1]
        bits.bit(0); // ccso_planes[2]
        let data = bits.into_bytes();
        let p = ccso_round_trip(&data, 3, ccso_enabled());
        assert_eq!(p.planes[0].ccso_edge_clf, Some(false));
        assert_eq!(p.planes[0].ccso_offset_idx.len(), 9);
    }

    #[test]
    fn ccso_monochrome_single_plane_round_trips() {
        let mut bits = Bits::default();
        bits.bit(1); // ccso_frame_flag
        bits.bit(1); // ccso_planes[0]
        bits.bit(1); // ccso_bo_only
        bits.f(0, 2); // ccso_scale_idx
        bits.f(0, 3); // ccso_max_band_log2 == 0 -> maxBand 1
        bits.tu(4, 7); // 1 offset
        let data = bits.into_bytes();
        let p = ccso_round_trip(&data, 1, ccso_enabled());
        assert_eq!(p.planes.len(), 1);
    }


    fn ccso_off_plane() -> CcsoPlaneParams {
        CcsoPlaneParams {
            reuse_ccso: false,
            sb_reuse_ccso: false,
            ccso_ref_idx: None,
            ccso_planes: false,
            ccso_bo_only: None,
            ccso_scale_idx: None,
            ccso_quant_idx: None,
            ccso_ext_filter: None,
            ccso_edge_clf: None,
            ccso_max_band_log2: None,
            ccso_offset_idx: Vec::new(),
        }
    }

    fn ccso_bo_plane(max_band_log2: u8, offsets: Vec<u8>) -> CcsoPlaneParams {
        CcsoPlaneParams {
            reuse_ccso: false,
            sb_reuse_ccso: false,
            ccso_ref_idx: None,
            ccso_planes: true,
            ccso_bo_only: Some(true),
            ccso_scale_idx: Some(0),
            ccso_quant_idx: Some(0),
            ccso_ext_filter: Some(0),
            ccso_edge_clf: Some(false),
            ccso_max_band_log2: Some(max_band_log2),
            ccso_offset_idx: offsets,
        }
    }

    #[test]
    fn ccso_disabled_with_state_is_rejected() {
        let mut view = ccso_enabled();
        view.enable_ccso = false;
        let params = CcsoParams {
            ccso_frame_flag: Some(true),
            planes: Vec::new(),
        };
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_ccso_params(&mut writer, &params, false, 3, &view),
            Err(WriteError::NonCanonicalFrameHeader {
                what: "ccso_disabled"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn ccso_single_picture_false_flag_is_rejected() {
        let mut view = ccso_enabled();
        view.single_picture_header_flag = true;
        let params = CcsoParams {
            ccso_frame_flag: Some(false),
            planes: Vec::new(),
        };
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_ccso_params(&mut writer, &params, false, 3, &view),
            Err(WriteError::NonCanonicalFrameHeader {
                what: "ccso_frame_flag"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn ccso_missing_frame_flag_is_rejected() {
        let params = CcsoParams {
            ccso_frame_flag: None,
            planes: Vec::new(),
        };
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_ccso_params(&mut writer, &params, false, 3, &ccso_enabled()),
            Err(WriteError::NonCanonicalFrameHeader {
                what: "ccso_frame_flag"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn ccso_num_planes_mismatch_is_rejected() {
        let params = CcsoParams {
            ccso_frame_flag: Some(true),
            planes: vec![ccso_off_plane()],
        };
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_ccso_params(&mut writer, &params, false, 3, &ccso_enabled()),
            Err(WriteError::NonCanonicalFrameHeader {
                what: "ccso_num_planes"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn ccso_some_field_on_disabled_plane_is_rejected() {
        let mut plane = ccso_off_plane();
        plane.ccso_scale_idx = Some(1);
        let params = CcsoParams {
            ccso_frame_flag: Some(true),
            planes: vec![plane, ccso_off_plane(), ccso_off_plane()],
        };
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_ccso_params(&mut writer, &params, false, 3, &ccso_enabled()),
            Err(WriteError::NonCanonicalFrameHeader {
                what: "ccso_plane_fields"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn ccso_none_field_on_enabled_plane_is_rejected() {
        let mut plane = ccso_bo_plane(0, vec![0]);
        plane.ccso_max_band_log2 = None;
        let params = CcsoParams {
            ccso_frame_flag: Some(true),
            planes: vec![plane, ccso_off_plane(), ccso_off_plane()],
        };
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_ccso_params(&mut writer, &params, false, 3, &ccso_enabled()),
            Err(WriteError::NonCanonicalFrameHeader {
                what: "ccso_plane_fields"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn ccso_out_of_domain_scale_idx_is_rejected() {
        let mut plane = ccso_bo_plane(0, vec![0]);
        plane.ccso_scale_idx = Some(4); // f(2) domain is 0..=3
        let params = CcsoParams {
            ccso_frame_flag: Some(true),
            planes: vec![plane, ccso_off_plane(), ccso_off_plane()],
        };
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_ccso_params(&mut writer, &params, false, 3, &ccso_enabled()),
            Err(WriteError::NonCanonicalFrameHeader {
                what: "ccso_scale_idx"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn ccso_offset_idx_len_mismatch_is_rejected() {
        let plane = ccso_bo_plane(0, vec![0, 1]);
        let params = CcsoParams {
            ccso_frame_flag: Some(true),
            planes: vec![plane, ccso_off_plane(), ccso_off_plane()],
        };
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_ccso_params(&mut writer, &params, false, 3, &ccso_enabled()),
            Err(WriteError::NonCanonicalFrameHeader {
                what: "ccso_offset_idx_len"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn ccso_offset_value_above_seven_is_rejected() {
        let plane = ccso_bo_plane(0, vec![8]); // tu(7) max is 7
        let params = CcsoParams {
            ccso_frame_flag: Some(true),
            planes: vec![plane, ccso_off_plane(), ccso_off_plane()],
        };
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_ccso_params(&mut writer, &params, false, 3, &ccso_enabled()),
            Err(WriteError::NonCanonicalFrameHeader {
                what: "ccso_offset_idx"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn ccso_bo_only_nonzero_inferred_field_is_rejected() {
        let mut plane = ccso_bo_plane(0, vec![0]);
        plane.ccso_quant_idx = Some(1); // bo_only infers 0
        let params = CcsoParams {
            ccso_frame_flag: Some(true),
            planes: vec![plane, ccso_off_plane(), ccso_off_plane()],
        };
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_ccso_params(&mut writer, &params, false, 3, &ccso_enabled()),
            Err(WriteError::NonCanonicalFrameHeader {
                what: "ccso_bo_only_fields"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    fn ccso_full_plane() -> CcsoPlaneParams {
        CcsoPlaneParams {
            reuse_ccso: false,
            sb_reuse_ccso: false,
            ccso_ref_idx: None,
            ccso_planes: true,
            ccso_bo_only: Some(false),
            ccso_scale_idx: Some(0),
            ccso_quant_idx: Some(0),
            ccso_ext_filter: Some(0),
            ccso_edge_clf: Some(false),
            ccso_max_band_log2: Some(0),
            ccso_offset_idx: vec![0; 9],
        }
    }

    #[test]
    fn ccso_frame_disabled_with_planes_is_rejected() {
        let params = CcsoParams {
            ccso_frame_flag: Some(false),
            planes: vec![ccso_off_plane(); 3],
        };
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_ccso_params(&mut writer, &params, false, 3, &ccso_enabled()),
            Err(WriteError::NonCanonicalFrameHeader {
                what: "ccso_frame_disabled"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn ccso_inter_reuse_flags_are_rejected() {
        for set_reuse in [true, false] {
            let mut plane = ccso_full_plane();
            if set_reuse {
                plane.reuse_ccso = true;
            } else {
                plane.sb_reuse_ccso = true;
            }
            let params = CcsoParams {
                ccso_frame_flag: Some(true),
                planes: vec![plane, ccso_off_plane(), ccso_off_plane()],
            };
            let mut writer = BitWriter::new();
            assert!(matches!(
                write_ccso_params(&mut writer, &params, false, 3, &ccso_enabled()),
                Err(WriteError::NonCanonicalFrameHeader {
                    what: "ccso_inter_reuse"
                })
            ));
            assert_eq!(writer.bit_len(), 0);
        }
    }

    #[test]
    fn ccso_out_of_domain_quant_idx_is_rejected() {
        let mut plane = ccso_full_plane();
        plane.ccso_quant_idx = Some(4); // f(2) domain is 0..=3
        let params = CcsoParams {
            ccso_frame_flag: Some(true),
            planes: vec![plane, ccso_off_plane(), ccso_off_plane()],
        };
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_ccso_params(&mut writer, &params, false, 3, &ccso_enabled()),
            Err(WriteError::NonCanonicalFrameHeader {
                what: "ccso_quant_idx"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn ccso_out_of_domain_ext_filter_is_rejected() {
        let mut plane = ccso_full_plane();
        plane.ccso_ext_filter = Some(8); // f(3) domain is 0..=7
        let params = CcsoParams {
            ccso_frame_flag: Some(true),
            planes: vec![plane, ccso_off_plane(), ccso_off_plane()],
        };
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_ccso_params(&mut writer, &params, false, 3, &ccso_enabled()),
            Err(WriteError::NonCanonicalFrameHeader {
                what: "ccso_ext_filter"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn ccso_edge_clf_set_when_quant_step_zero_is_rejected() {
        // CCSO_Quant_Sz[0][3] == 0 suppresses ccso_edge_clf (inferred 0, no bit); a model with
        // edge_clf == Some(true) could not have been produced.
        let mut plane = ccso_full_plane();
        plane.ccso_quant_idx = Some(3); // CCSO_Quant_Sz[0][3] == 0
        plane.ccso_edge_clf = Some(true);
        let params = CcsoParams {
            ccso_frame_flag: Some(true),
            planes: vec![plane, ccso_off_plane(), ccso_off_plane()],
        };
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_ccso_params(&mut writer, &params, false, 3, &ccso_enabled()),
            Err(WriteError::NonCanonicalFrameHeader {
                what: "ccso_edge_clf"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn ccso_out_of_domain_max_band_log2_is_rejected() {
        let plane = ccso_bo_plane(8, vec![0]);
        let params = CcsoParams {
            ccso_frame_flag: Some(true),
            planes: vec![plane, ccso_off_plane(), ccso_off_plane()],
        };
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_ccso_params(&mut writer, &params, false, 3, &ccso_enabled()),
            Err(WriteError::NonCanonicalFrameHeader {
                what: "ccso_max_band_log2"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }
}

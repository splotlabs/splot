// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::expect_used)]

use super::*;
use crate::bitstream::tile_payload::{
    CflMultiDirection, CflParams, FrameCdfSubset, GeneralIntraChromaBlockMode, TileCdfSelector,
};
use crate::{DecodeDiagnosticDetails, DecodeDiagnosticReport, DecodeSourceIssueKind};
use splot_core::symbol::SymbolDecoder;

#[test]
fn invalid_uv_mode_is_malformed_tile_syntax() {
    let error = general_intra_block_mode_error(
        GeneralIntraBlockModeError::InvalidUvMode { uv_mode: 13 },
        ByteOffset::new(42),
    );
    assert!(matches!(
        &error,
        DecodeError::MalformedSource { issue }
            if issue.kind() == DecodeSourceIssueKind::TilePayloadParseError
                && issue.rule_id().is_none()
                && issue.spec_section() == Some(GENERAL_INTRA_MODE_SPEC_SECTION)
                && issue.offset() == Some(ByteOffset::new(42))
                && issue.message().contains("out-of-range uv_mode 13")
    ));

    let report = DecodeDiagnosticReport::from_decode_error(&error)
        .expect("malformed source has a diagnostic report");
    assert_eq!(
        report.diagnostic.spec_section,
        Some(GENERAL_INTRA_MODE_SPEC_SECTION)
    );
    assert!(matches!(
        report.details,
        DecodeDiagnosticDetails::MalformedSource(_)
    ));
}

#[test]
fn impossible_mhccp_direction_is_typed_internal_state() {
    let error = general_intra_block_mode_error(
        GeneralIntraBlockModeError::InvalidCflMhDirection { direction: 3 },
        ByteOffset::new(42),
    );
    assert!(matches!(
        error,
        DecodeError::HeaderState {
            source: crate::DecodeHeaderStateError::InvalidGeneralIntraMhccpDirection,
        }
    ));
    assert!(DecodeDiagnosticReport::from_decode_error(&error).is_none());
}

#[test]
fn luma_mode_source_eof_contract_is_malformed_at_each_syntax_boundary() {
    let offset = ByteOffset::new(42);
    let source_offset = ByteOffset::new(47);
    let failures = [
        GeneralIntraBlockModeError::SymbolRead {
            reason: "intra_y_mode_set",
            source: BlockSymbolTraceReadError::Symbol(splot_core::Error::UnexpectedEof {
                offset: source_offset,
                needed: 1,
            }),
        },
        GeneralIntraBlockModeError::SymbolRead {
            reason: "intra_y_mode_offset",
            source: BlockSymbolTraceReadError::Symbol(splot_core::Error::UnexpectedEof {
                offset: source_offset,
                needed: 1,
            }),
        },
        GeneralIntraBlockModeError::Literal {
            reason: "intra_y_second_mode",
            source: splot_core::Error::UnexpectedEof {
                offset: source_offset,
                needed: 1,
            },
        },
    ];

    for failure in failures {
        let error = general_intra_block_mode_error(failure, offset);
        assert!(matches!(
            &error,
            DecodeError::MalformedSource { issue }
                if issue.kind() == DecodeSourceIssueKind::TilePayloadParseError
                    && issue.spec_section() == Some(GENERAL_INTRA_MODE_SPEC_SECTION)
                    && issue.offset() == Some(offset)
                    && issue.message().contains("unexpected end of input at byte 47")
        ));
        assert!(matches!(
            DecodeDiagnosticReport::from_decode_error(&error).map(|report| report.details),
            Some(DecodeDiagnosticDetails::MalformedSource(_))
        ));
    }
}

#[test]
fn general_intra_cdf_failures_are_typed_internal_and_do_not_mutate_rows() {
    let offset = ByteOffset::new(42);
    let mut selector_tile = FrameCdfSubset::from_defaults().tile_copy();
    let selector_before = selector_tile.clone();
    let mut selector_symbols = SymbolDecoder::new(&[0x80]).expect("symbol decoder");
    let selector_source = selector_tile
        .read_block_symbol_trace(
            TileCdfSelector::YModeIndex { ctx: usize::MAX },
            &mut selector_symbols,
        )
        .expect_err("out-of-range selector");
    assert_eq!(selector_tile, selector_before);
    let selector_error = general_intra_block_mode_error(
        GeneralIntraBlockModeError::SymbolRead {
            reason: "intra_y_mode_index",
            source: selector_source,
        },
        offset,
    );
    assert!(matches!(
        &selector_error,
        DecodeError::HeaderState {
            source: crate::DecodeHeaderStateError::InvalidGeneralIntraModeState,
        }
    ));
    assert!(DecodeDiagnosticReport::from_decode_error(&selector_error).is_none());

    let selector = TileCdfSelector::YModeSet;
    let mut invalid_tile = FrameCdfSubset::from_defaults().tile_copy();
    invalid_tile
        .with_row_mut(selector, |row| row[0] = 0)
        .expect("YModeSet selector");
    let invalid_before = invalid_tile.clone();
    let mut invalid_symbols = SymbolDecoder::new(&[0x80]).expect("symbol decoder");
    let invalid_source = invalid_tile
        .read_block_symbol_trace(selector, &mut invalid_symbols)
        .expect_err("invalid CDF row");
    assert_eq!(invalid_tile, invalid_before);
    let invalid_error = general_intra_block_mode_error(
        GeneralIntraBlockModeError::SymbolRead {
            reason: "intra_y_mode_set",
            source: invalid_source,
        },
        offset,
    );
    assert!(matches!(
        &invalid_error,
        DecodeError::HeaderState {
            source: crate::DecodeHeaderStateError::InvalidGeneralIntraModeState,
        }
    ));
    assert!(DecodeDiagnosticReport::from_decode_error(&invalid_error).is_none());

    let state_error = general_intra_block_mode_error(
        GeneralIntraBlockModeError::SymbolRead {
            reason: "intra_y_mode_set",
            source: BlockSymbolTraceReadError::Symbol(
                splot_core::Error::InvalidSymbolDecoderState {
                    offset: ByteOffset::new(48),
                    bit_offset: splot_core::span::BitOffset::from_bits(3),
                    kind: splot_core::error::SymbolDecoderErrorKind::InvalidArithmeticRange,
                },
            ),
        },
        offset,
    );
    assert!(matches!(
        &state_error,
        DecodeError::HeaderState {
            source: crate::DecodeHeaderStateError::InvalidGeneralIntraModeState,
        }
    ));
    assert!(DecodeDiagnosticReport::from_decode_error(&state_error).is_none());
}

#[test]
fn palette_source_failures_are_malformed_at_the_palette_token_boundary() {
    let offset = ByteOffset::new(42);
    let source_offset = ByteOffset::new(47);
    let failures = [
        GeneralIntraResidualError::PaletteSymbolRead {
            source: BlockSymbolTraceReadError::Symbol(splot_core::Error::UnexpectedEof {
                offset: source_offset,
                needed: 1,
            }),
        },
        GeneralIntraResidualError::PaletteLiteral {
            reason: "palette_direction",
            source: splot_core::Error::UnexpectedEof {
                offset: source_offset,
                needed: 1,
            },
        },
    ];

    for failure in failures {
        let error = general_intra_residual_error(failure, offset);
        assert!(matches!(
            &error,
            DecodeError::MalformedSource { issue }
                if issue.kind() == DecodeSourceIssueKind::TilePayloadParseError
                    && issue.rule_id().is_none()
                    && issue.spec_section() == Some("5.20.8.4")
                    && issue.offset() == Some(offset)
                    && issue.message().contains("unexpected end of input at byte 47")
        ));
        assert!(matches!(
            DecodeDiagnosticReport::from_decode_error(&error).map(|report| report.details),
            Some(DecodeDiagnosticDetails::MalformedSource(_))
        ));
    }
}

#[test]
fn palette_first_line_copy_is_a_typed_conformance_error() {
    let offset = ByteOffset::new(42);
    let error =
        general_intra_residual_error(GeneralIntraResidualError::PaletteInvalidIdentityRow, offset);

    assert!(matches!(
        &error,
        DecodeError::MalformedSource { issue }
            if issue.kind() == DecodeSourceIssueKind::TilePayloadParseError
                && issue.rule_id().is_none()
                && issue.spec_section() == Some("6.19.8.3")
                && issue.offset() == Some(offset)
                && issue.message().contains("invalid on the first row")
    ));
    assert!(matches!(
        DecodeDiagnosticReport::from_decode_error(&error).map(|report| report.details),
        Some(DecodeDiagnosticDetails::MalformedSource(_))
    ));
}

#[test]
fn invalid_transform_partition_dimensions_are_a_typed_conformance_error() {
    let offset = ByteOffset::new(42);
    let error = general_intra_residual_error(
        GeneralIntraResidualError::InvalidTransformPartitionDimensions {
            width: 4,
            height: 1,
        },
        offset,
    );

    assert!(matches!(
        &error,
        DecodeError::MalformedSource { issue }
            if issue.kind() == DecodeSourceIssueKind::TilePayloadParseError
                && issue.rule_id().is_none()
                && issue.spec_section() == Some("6.19.6.3")
                && issue.offset() == Some(offset)
                && issue.message().contains("invalid dimensions 4x1")
    ));
    assert!(matches!(
        DecodeDiagnosticReport::from_decode_error(&error).map(|report| report.details),
        Some(DecodeDiagnosticDetails::MalformedSource(_))
    ));
}

#[test]
fn palette_entropy_failures_are_typed_internal_and_fail_atomic() {
    let offset = ByteOffset::new(42);
    let selector = TileCdfSelector::IdentityRowY { ctx: usize::MAX };
    let mut selector_tile = FrameCdfSubset::from_defaults().tile_copy();
    let selector_before = selector_tile.clone();
    let mut selector_symbols = SymbolDecoder::new(&[0x80]).expect("symbol decoder");
    let selector_source = selector_tile
        .read_block_symbol_trace(selector, &mut selector_symbols)
        .expect_err("out-of-range selector");
    assert_eq!(selector_tile, selector_before);
    let selector_error = general_intra_residual_error(
        GeneralIntraResidualError::PaletteSymbolRead {
            source: selector_source,
        },
        offset,
    );
    assert!(matches!(
        selector_error,
        DecodeError::HeaderState {
            source: crate::DecodeHeaderStateError::InvalidGeneralIntraPaletteEntropyState,
        }
    ));

    let selector = TileCdfSelector::IdentityRowY { ctx: 0 };
    let mut invalid_tile = FrameCdfSubset::from_defaults().tile_copy();
    invalid_tile
        .with_row_mut(selector, |row| row[0] = 0)
        .expect("identity-row selector");
    let invalid_before = invalid_tile.clone();
    let mut invalid_symbols = SymbolDecoder::new(&[0x80]).expect("symbol decoder");
    let invalid_source = invalid_tile
        .read_block_symbol_trace(selector, &mut invalid_symbols)
        .expect_err("invalid CDF row");
    assert_eq!(invalid_tile, invalid_before);
    let invalid_error = general_intra_residual_error(
        GeneralIntraResidualError::PaletteSymbolRead {
            source: invalid_source,
        },
        offset,
    );
    assert!(matches!(
        invalid_error,
        DecodeError::HeaderState {
            source: crate::DecodeHeaderStateError::InvalidGeneralIntraPaletteEntropyState,
        }
    ));

    let state_error = general_intra_residual_error(
        GeneralIntraResidualError::PaletteLiteral {
            reason: "palette_direction",
            source: splot_core::Error::InvalidSymbolDecoderState {
                offset: ByteOffset::new(48),
                bit_offset: splot_core::span::BitOffset::from_bits(3),
                kind: splot_core::error::SymbolDecoderErrorKind::InvalidArithmeticRange,
            },
        },
        offset,
    );
    assert!(matches!(
        state_error,
        DecodeError::HeaderState {
            source: crate::DecodeHeaderStateError::InvalidGeneralIntraPaletteEntropyState,
        }
    ));
}

#[test]
fn palette_color_index_escape_is_typed_internal_state() {
    let error = general_intra_residual_error(
        GeneralIntraResidualError::PaletteColorIndex {
            color_index: 3,
            palette_size: 2,
        },
        ByteOffset::new(42),
    );

    assert!(matches!(
        &error,
        DecodeError::HeaderState {
            source: crate::DecodeHeaderStateError::InvalidGeneralIntraPaletteColorState,
        }
    ));
    assert!(DecodeDiagnosticReport::from_decode_error(&error).is_none());
}

#[test]
fn residual_plan_failures_keep_internal_and_resource_taxonomy() {
    let geometry =
        general_intra_residual_plan_error(ResidualPlanError::InvalidGeometry, ByteOffset::new(42));
    assert!(matches!(
        geometry,
        DecodeError::HeaderState {
            source: crate::DecodeHeaderStateError::InvalidBlockGeometry,
        }
    ));
    assert!(DecodeDiagnosticReport::from_decode_error(&geometry).is_none());

    let allocation = general_intra_residual_plan_error(
        ResidualPlanError::Allocation { plane: PlaneId::U },
        ByteOffset::new(42),
    );
    assert!(matches!(
        allocation,
        DecodeError::Reconstruction {
            source: splot_recon::ReconError::WorkspaceAllocationFailed {
                plane: PlaneId::U,
                context: "general-intra residual plan",
            },
        }
    ));
    assert!(DecodeDiagnosticReport::from_decode_error(&allocation).is_none());
}

#[test]
fn unexpected_general_intra_branch_is_internal_state() {
    let error = general_intra_residual_error(
        GeneralIntraResidualError::UnexpectedBranch,
        ByteOffset::new(42),
    );

    assert!(matches!(
        error,
        DecodeError::InternalState {
            reason: "general_intra_luma_coeff_unexpected_branch",
            byte_offset,
        } if byte_offset == ByteOffset::new(42)
    ));
}

#[test]
fn general_intra_recon_command_is_send() {
    fn assert_send<T: Send>() {}

    assert_send::<GeneralIntraReconCommand>();
}

fn ctx(row4: usize, col4: usize, width4: usize, height4: usize) -> BlockCtx {
    ctx_with_bit_depth(row4, col4, width4, height4, BitDepth::Ten)
}

fn ctx_with_bit_depth(
    row4: usize,
    col4: usize,
    width4: usize,
    height4: usize,
    bit_depth: BitDepth,
) -> BlockCtx {
    BlockCtx::new(
        BlockRect::new(row4, col4, width4, height4),
        TxShape::from_luma_4x4(width4, height4).expect("valid transform shape"),
        480,
        270,
        bit_depth,
        ChromaSampling::Yuv420,
    )
}

fn assert_rect_chroma_plan(
    mode: SupportedChromaMode,
    angle_delta: i8,
    expected: RectChromaPlan,
    label: &str,
) {
    assert_eq!(
        rect_chroma_plan_for_mode(mode, angle_delta, None),
        expected,
        "{label}"
    );
}

fn assert_rect_luma_plan_for_parts(
    mode: IntraYMode,
    directional_p_angle: Option<u16>,
    expected: RectLumaPlan,
    label: &str,
) {
    assert_eq!(
        rect_luma_plan_for_parts(mode, directional_p_angle, false),
        Ok(expected),
        "{label}"
    );
}

fn rect_luma_plan_for_parts(
    mode: IntraYMode,
    directional_p_angle: Option<u16>,
    use_tcq: bool,
) -> core::result::Result<RectLumaPlan, crate::DecodeHeaderStateError> {
    rect_luma_plan_for_mode(mode, directional_p_angle, use_tcq)
}

fn assert_rect_luma_mrl_plan(
    y_mode: IntraYMode,
    angle_delta_y: i8,
    mrl_index: u8,
    mrl_sec_index: Option<u8>,
    block: BlockCtx,
    expected: RectLumaPlan,
    label: &str,
) {
    let mrl = MrlSelection::from_symbols(mrl_index, mrl_sec_index).expect("valid test MRL");
    assert_eq!(
        rect_luma_mrl_plan_for_parts(y_mode, angle_delta_y, mrl, block, false, 32,),
        Ok(expected),
        "{label}"
    );
}

fn luma_modes(y_mode: IntraYMode) -> GeneralIntraBlockModes {
    luma_modes_with_angle(y_mode, 0)
}

fn luma_modes_with_angle(y_mode: IntraYMode, angle_delta_y: i8) -> GeneralIntraBlockModes {
    luma_modes_with_parts(y_mode, angle_delta_y, 0, 0)
}

fn luma_modes_with_parts(
    y_mode: IntraYMode,
    angle_delta_y: i8,
    use_dpcm_y: u8,
    dpcm_mode_y: u8,
) -> GeneralIntraBlockModes {
    GeneralIntraBlockModes::luma_only(crate::bitstream::tile_payload::GeneralIntraLumaBlockMode {
        y_mode,
        angle_delta_y,
        intra_joint_mode: crate::bitstream::tile_payload::IntraJointMode::DC,
        mrl: MrlSelection::Disabled,
        fsc_mode: 0,
        use_dip: 0,
        dip_transpose: 0,
        dip_mode: 0,
        use_dpcm_y,
        dpcm_mode_y,
    })
}

#[test]
fn luma_tx_partition_context_uses_current_block_lossless_flag() {
    let block_size = BlockSize::new(6).expect("valid block size");

    assert_eq!(
        luma_tx_partition_context(Some(TxMode::Select), block_size, false),
        Some(LumaTransformPartitionContext::new(block_size))
    );
    assert_eq!(
        luma_tx_partition_context(Some(TxMode::Select), block_size, true),
        None
    );
    assert_eq!(
        luma_tx_partition_context(Some(TxMode::Largest), block_size, false),
        None
    );
}

/// Luma 4x4 units spanning a full AV2 superblock axis.
const FULL_SB_N4_LUMA: usize = 16;

#[test]
fn lossless_luma_uses_generic_prediction_planner() {
    for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
        let block_ctx = ctx_with_bit_depth(0, 0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA, bit_depth);
        for mode in [
            IntraYMode::Vertical,
            IntraYMode::Horizontal,
            IntraYMode::D45,
            IntraYMode::D67,
            IntraYMode::D113,
            IntraYMode::D135,
            IntraYMode::D157,
            IntraYMode::D203,
            IntraYMode::Smooth,
        ] {
            let modes = luma_modes(mode);
            assert!(
                rect_luma_plan(&modes, block_ctx, false, FULL_SB_N4_LUMA).is_ok(),
                "lossless {bit_depth:?} {mode:?}"
            );
        }
    }
}

#[test]
fn lossless_adjusted_directional_luma_uses_rect_planner() {
    let block_ctx = ctx_with_bit_depth(0, 0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA, BitDepth::Eight);

    for (mode, angle_delta_y, p_angle) in [
        (IntraYMode::Vertical, 1, 93),
        (IntraYMode::Horizontal, -1, 177),
        (IntraYMode::D135, -1, 132),
        (IntraYMode::D135, 1, 138),
    ] {
        let modes = luma_modes_with_angle(mode, angle_delta_y);

        assert_eq!(
            rect_luma_plan(&modes, block_ctx, false, FULL_SB_N4_LUMA),
            Ok(RectLumaPlan::Middle {
                p_angle,
                use_tcq: false,
            })
        );
    }
}

#[test]
fn chroma_part_cfl_reaches_cfl_plan() {
    let params = CflParams::Explicit {
        alpha_u: 1,
        alpha_v: -1,
    };
    let chroma = GeneralIntraChromaBlockMode::cfl_for_test(params);

    assert_eq!(
        chroma_plan_for_parts(chroma, IntraYMode::Horizontal, 0, 1, 32),
        RectChromaPlan::Cfl {
            params,
            cfl_ds_filter_index: 1,
            sb_mib: 32,
        }
    );
}

#[test]
fn lossless_chroma_block_cfl_reaches_cfl_plan() {
    let params = CflParams::Multi {
        direction: CflMultiDirection::Left,
    };
    let luma = crate::bitstream::tile_payload::GeneralIntraLumaBlockMode {
        y_mode: IntraYMode::Horizontal,
        angle_delta_y: 0,
        intra_joint_mode: crate::bitstream::tile_payload::IntraJointMode::DC,
        mrl: MrlSelection::Disabled,
        fsc_mode: 1,
        use_dip: 0,
        dip_transpose: 0,
        dip_mode: 0,
        use_dpcm_y: 0,
        dpcm_mode_y: 0,
    };
    let chroma = GeneralIntraChromaBlockMode::cfl_for_test(params);
    let modes = GeneralIntraBlockModes::from_luma_chroma_palette(luma, chroma, None);

    let result = rect_chroma_plan(&modes, 1, 16);
    assert_eq!(
        result,
        Some(RectChromaPlan::Cfl {
            params,
            cfl_ds_filter_index: 1,
            sb_mib: 16,
        })
    );
}

#[test]
fn directional_luma_always_plans_through_the_rect_planner() {
    let shapes = [
        ctx_with_bit_depth(0, 0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA, BitDepth::Eight),
        ctx_with_bit_depth(0, 0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA, BitDepth::Ten),
        ctx(0, 0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA),
        ctx_with_bit_depth(0, 0, 4, 4, BitDepth::Eight),
        ctx_with_bit_depth(FULL_SB_N4_LUMA, FULL_SB_N4_LUMA, 4, 4, BitDepth::Ten),
    ];
    let modes = [
        IntraYMode::Vertical,
        IntraYMode::Horizontal,
        IntraYMode::D45,
        IntraYMode::D67,
        IntraYMode::D113,
        IntraYMode::D135,
        IntraYMode::D157,
        IntraYMode::D203,
    ];

    for block_ctx in shapes {
        for mode in modes {
            for angle_delta_y in [-3, 0, 2] {
                let modes = luma_modes_with_angle(mode, angle_delta_y);
                assert!(
                    rect_luma_plan(&modes, block_ctx, false, FULL_SB_N4_LUMA).is_ok(),
                    "rect planner must serve directional luma {mode:?} {angle_delta_y}"
                );
            }
        }
    }
}

#[test]
fn plans_every_chroma_mode() {
    for mode in [
        SupportedChromaMode::Dc,
        SupportedChromaMode::Smooth,
        SupportedChromaMode::D135Follow,
        SupportedChromaMode::D113Follow,
        SupportedChromaMode::D157Follow,
        SupportedChromaMode::VerticalFollow,
        SupportedChromaMode::Vertical,
        SupportedChromaMode::HorizontalFollow,
        SupportedChromaMode::Horizontal,
        SupportedChromaMode::D45Follow,
        SupportedChromaMode::D67Follow,
        SupportedChromaMode::D45,
        SupportedChromaMode::D67,
        SupportedChromaMode::D135,
        SupportedChromaMode::D113,
        SupportedChromaMode::D203Follow,
        SupportedChromaMode::D203,
        SupportedChromaMode::D157,
        SupportedChromaMode::Paeth,
        SupportedChromaMode::SmoothVertical,
        SupportedChromaMode::SmoothHorizontal,
    ] {
        let planned = match rect_chroma_plan_for_mode(mode, 0, None) {
            RectChromaPlan::Mode(planned, None)
            | RectChromaPlan::Directional {
                mode: planned,
                angle_delta_uv: 0,
                dpcm: None,
            } => Some(planned),
            _ => None,
        };
        assert_eq!(planned, Some(mode));
    }
}

#[test]
fn shared_and_chroma_part_planners_are_total_over_typed_chroma_states() {
    let luma = crate::bitstream::tile_payload::GeneralIntraLumaBlockMode {
        y_mode: IntraYMode::Horizontal,
        angle_delta_y: -2,
        intra_joint_mode: crate::bitstream::tile_payload::IntraJointMode::DC,
        mrl: MrlSelection::Disabled,
        fsc_mode: 0,
        use_dip: 0,
        dip_transpose: 0,
        dip_mode: 0,
        use_dpcm_y: 0,
        dpcm_mode_y: 0,
    };
    let prediction = GeneralIntraChromaBlockMode::Prediction {
        mode: SupportedChromaMode::HorizontalFollow,
        coeff_uv_mode: IntraYMode::Horizontal.value() as u8,
        dpcm: None,
    };
    let expected_prediction = RectChromaPlan::Directional {
        mode: SupportedChromaMode::HorizontalFollow,
        angle_delta_uv: -2,
        dpcm: None,
    };
    assert_eq!(
        chroma_plan_for_parts(prediction, luma.y_mode, luma.angle_delta_y, 1, 32),
        expected_prediction
    );
    let modes = GeneralIntraBlockModes::from_luma_chroma_palette(luma, prediction, None);
    assert_eq!(rect_chroma_plan(&modes, 1, 32), Some(expected_prediction));

    for params in [
        CflParams::Explicit {
            alpha_u: 3,
            alpha_v: -4,
        },
        CflParams::DerivedAlpha,
        CflParams::Multi {
            direction: CflMultiDirection::Direct,
        },
        CflParams::Multi {
            direction: CflMultiDirection::Above,
        },
        CflParams::Multi {
            direction: CflMultiDirection::Left,
        },
    ] {
        let chroma = GeneralIntraChromaBlockMode::Cfl(params);
        let expected = RectChromaPlan::Cfl {
            params,
            cfl_ds_filter_index: 1,
            sb_mib: 32,
        };
        assert_eq!(
            chroma_plan_for_parts(chroma, luma.y_mode, luma.angle_delta_y, 1, 32),
            expected
        );
        let modes = GeneralIntraBlockModes::from_luma_chroma_palette(luma, chroma, None);
        assert_eq!(rect_chroma_plan(&modes, 1, 32), Some(expected));
    }

    assert_eq!(
        rect_chroma_plan(&GeneralIntraBlockModes::luma_only(luma), 1, 32),
        None
    );
}

#[test]
fn classifies_smooth_luma_plans() {
    for (label, y_mode, mode) in [
        (
            "smooth vertical",
            IntraYMode::SmoothVertical,
            crate::bitstream::tile_payload::SupportedNonDcLumaMode::SmoothVertical,
        ),
        (
            "smooth",
            IntraYMode::Smooth,
            crate::bitstream::tile_payload::SupportedNonDcLumaMode::Smooth,
        ),
        (
            "smooth horizontal",
            IntraYMode::SmoothHorizontal,
            crate::bitstream::tile_payload::SupportedNonDcLumaMode::SmoothHorizontal,
        ),
    ] {
        assert_rect_luma_plan_for_parts(
            y_mode,
            None,
            RectLumaPlan::Smooth {
                mode,
                use_tcq: false,
            },
            label,
        );
    }
}

#[test]
fn classifies_paeth_luma_plan() {
    assert_eq!(
        rect_luma_plan_for_mode(IntraYMode::Paeth, None, false),
        Ok(RectLumaPlan::Paeth { use_tcq: false })
    );
}

#[test]
fn active_dip_luma_routes_before_dc() {
    let modes = GeneralIntraBlockModes::luma_only(
        crate::bitstream::tile_payload::GeneralIntraLumaBlockMode {
            y_mode: IntraYMode::Dc,
            angle_delta_y: 0,
            intra_joint_mode: crate::bitstream::tile_payload::IntraJointMode::DC,
            mrl: MrlSelection::Disabled,
            fsc_mode: 0,
            use_dip: 1,
            dip_transpose: 1,
            dip_mode: 2,
            use_dpcm_y: 0,
            dpcm_mode_y: 0,
        },
    );
    let block = ctx(0, 10, 2, 4);

    assert_eq!(
        rect_luma_plan(&modes, block, true, 16),
        Ok(RectLumaPlan::Dip {
            mode: 2,
            transpose: true,
            use_tcq: true,
        })
    );
}

#[test]
fn admits_rect_luma_mrl_cases() {
    for (label, y_mode, angle_delta_y, mrl_index, mrl_sec_index, block, expected) in [
        (
            "small rect d135 middle",
            IntraYMode::D135,
            0,
            3,
            Some(0),
            ctx(20, 216, 1, 4),
            RectLumaPlan::MiddleMrl {
                p_angle: 135,
                mrl_index: 3,
                above_mrl_index: 3,
                is_sb_boundary: false,
                secondary_mrl: false,
                use_tcq: false,
            },
        ),
        (
            "small square vertical cardinal secondary",
            IntraYMode::Vertical,
            0,
            3,
            Some(1),
            ctx(16, 264, 4, 4),
            RectLumaPlan::CardinalMrl {
                direction: IntraCardinalDirection::Vertical,
                mrl_index: 3,
                above_mrl_index: 3,
                secondary_mrl: true,
                use_tcq: false,
            },
        ),
        (
            "d45 one-sided above sb boundary",
            IntraYMode::D45,
            -2,
            1,
            Some(0),
            ctx(32, 216, 4, 4),
            RectLumaPlan::OneSidedAboveMrl {
                p_angle: 40,
                mrl_index: 1,
                above_mrl_index: 0,
                secondary_mrl: false,
                use_tcq: false,
            },
        ),
        (
            "small square d157 middle",
            IntraYMode::D157,
            3,
            1,
            Some(0),
            ctx(26, 222, 2, 2),
            RectLumaPlan::MiddleMrl {
                p_angle: 167,
                mrl_index: 1,
                above_mrl_index: 1,
                is_sb_boundary: false,
                secondary_mrl: false,
                use_tcq: false,
            },
        ),
        (
            "top-row rect d113 middle",
            IntraYMode::D113,
            -1,
            2,
            Some(1),
            ctx(0, 316, 4, 8),
            RectLumaPlan::MiddleMrl {
                p_angle: 109,
                mrl_index: 2,
                above_mrl_index: 0,
                is_sb_boundary: true,
                secondary_mrl: true,
                use_tcq: false,
            },
        ),
        (
            "left-edge active-mrl vpred middle",
            IntraYMode::Vertical,
            0,
            1,
            Some(0),
            ctx(4, 0, 1, 4),
            RectLumaPlan::MiddleMrl {
                p_angle: 91,
                mrl_index: 1,
                above_mrl_index: 1,
                is_sb_boundary: false,
                secondary_mrl: false,
                use_tcq: false,
            },
        ),
        (
            "top-row rect d67 one-sided above from left edge",
            IntraYMode::D67,
            -1,
            3,
            Some(0),
            ctx(0, 8, 8, 2),
            RectLumaPlan::OneSidedAboveMrl {
                p_angle: 64,
                mrl_index: 3,
                above_mrl_index: 0,
                secondary_mrl: false,
                use_tcq: false,
            },
        ),
        (
            "rect d67 one-sided left after wide-angle mapping",
            IntraYMode::D67,
            0,
            2,
            Some(0),
            ctx(22, 313, 2, 8),
            RectLumaPlan::OneSidedLeftMrl {
                p_angle: 246,
                mrl_index: 2,
                above_mrl_index: 2,
                is_sb_boundary: false,
                secondary_mrl: false,
                use_tcq: false,
            },
        ),
        (
            "top-left vertical cardinal without neighbours",
            IntraYMode::Vertical,
            0,
            3,
            Some(0),
            ctx(0, 0, 4, 4),
            RectLumaPlan::CardinalMrl {
                direction: IntraCardinalDirection::Vertical,
                mrl_index: 3,
                above_mrl_index: 0,
                secondary_mrl: false,
                use_tcq: false,
            },
        ),
        (
            "top-left horizontal cardinal without neighbours",
            IntraYMode::Horizontal,
            0,
            3,
            Some(0),
            ctx(0, 0, 4, 4),
            RectLumaPlan::CardinalMrl {
                direction: IntraCardinalDirection::Horizontal,
                mrl_index: 3,
                above_mrl_index: 0,
                secondary_mrl: false,
                use_tcq: false,
            },
        ),
        (
            "top-left d45 without neighbours",
            IntraYMode::D45,
            0,
            3,
            Some(0),
            ctx(0, 0, 4, 4),
            RectLumaPlan::OneSidedAboveMrl {
                p_angle: 45,
                mrl_index: 3,
                above_mrl_index: 0,
                secondary_mrl: false,
                use_tcq: false,
            },
        ),
        (
            "top-left d135 without neighbours",
            IntraYMode::D135,
            0,
            3,
            Some(0),
            ctx(0, 0, 4, 4),
            RectLumaPlan::MiddleMrl {
                p_angle: 135,
                mrl_index: 3,
                above_mrl_index: 0,
                is_sb_boundary: true,
                secondary_mrl: false,
                use_tcq: false,
            },
        ),
        (
            "top-left d203 without neighbours",
            IntraYMode::D203,
            0,
            3,
            Some(0),
            ctx(0, 0, 4, 4),
            RectLumaPlan::OneSidedLeftMrl {
                p_angle: 203,
                mrl_index: 3,
                above_mrl_index: 0,
                is_sb_boundary: true,
                secondary_mrl: false,
                use_tcq: false,
            },
        ),
    ] {
        assert_rect_luma_mrl_plan(
            y_mode,
            angle_delta_y,
            mrl_index,
            mrl_sec_index,
            block,
            expected,
            label,
        );
    }
}

#[test]
fn maps_4x8_d45_to_one_sided_left_luma() {
    let modes = luma_modes(IntraYMode::D45);

    assert_eq!(
        rect_luma_plan(&modes, ctx(8, 479, 1, 2), false, FULL_SB_N4_LUMA),
        Ok(RectLumaPlan::OneSidedLeft {
            p_angle: 225,
            use_tcq: false,
        })
    );
}

#[test]
fn plans_rect_cardinal_luma_from_modes_and_geometry() {
    for (label, mode, block, direction) in [
        (
            "128x64 vertical",
            IntraYMode::Vertical,
            ctx(FULL_SB_N4_LUMA, 256, 32, FULL_SB_N4_LUMA),
            IntraCardinalDirection::Vertical,
        ),
        (
            "64x64 vertical",
            IntraYMode::Vertical,
            ctx(0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA),
            IntraCardinalDirection::Vertical,
        ),
        (
            "64x64 horizontal",
            IntraYMode::Horizontal,
            ctx(80, 0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA),
            IntraCardinalDirection::Horizontal,
        ),
        (
            "4x8 vertical",
            IntraYMode::Vertical,
            ctx(24, 204, 1, 2),
            IntraCardinalDirection::Vertical,
        ),
        (
            "16x64 vertical",
            IntraYMode::Vertical,
            ctx(0, 0, 4, FULL_SB_N4_LUMA),
            IntraCardinalDirection::Vertical,
        ),
        (
            "64x16 horizontal",
            IntraYMode::Horizontal,
            ctx(0, 0, FULL_SB_N4_LUMA, 4),
            IntraCardinalDirection::Horizontal,
        ),
    ] {
        assert_eq!(
            rect_luma_plan(&luma_modes(mode), block, false, FULL_SB_N4_LUMA),
            Ok(RectLumaPlan::Cardinal {
                direction,
                use_tcq: false,
            }),
            "{label}"
        );
    }
}

#[test]
fn plans_rect_directional_luma_from_modes_and_geometry() {
    for (label, mode, angle_delta, block, expected) in [
        (
            "8x16 d67 +3",
            IntraYMode::D67,
            3,
            ctx(28, 216, 2, 4),
            RectLumaPlan::OneSidedAbove {
                p_angle: 76,
                use_tcq: false,
            },
        ),
        (
            "32x16 horizontal +2",
            IntraYMode::Horizontal,
            2,
            ctx(0, 264, 8, 4),
            RectLumaPlan::OneSidedLeft {
                p_angle: 186,
                use_tcq: false,
            },
        ),
        (
            "64x16 horizontal +1",
            IntraYMode::Horizontal,
            1,
            ctx(80, 0, FULL_SB_N4_LUMA, 4),
            RectLumaPlan::OneSidedLeft {
                p_angle: 183,
                use_tcq: false,
            },
        ),
        (
            "16x8 d203",
            IntraYMode::D203,
            0,
            ctx(46, 0, 4, 2),
            RectLumaPlan::OneSidedLeft {
                p_angle: 203,
                use_tcq: false,
            },
        ),
        (
            "4x8 d157 -2",
            IntraYMode::D157,
            -2,
            ctx(26, 204, 1, 2),
            RectLumaPlan::Middle {
                p_angle: 151,
                use_tcq: false,
            },
        ),
        (
            "64x32 d67 -2",
            IntraYMode::D67,
            -2,
            ctx(8, 336, FULL_SB_N4_LUMA, 8),
            RectLumaPlan::OneSidedAbove {
                p_angle: 61,
                use_tcq: false,
            },
        ),
        (
            "32x64 d135 -3",
            IntraYMode::D135,
            -3,
            ctx(FULL_SB_N4_LUMA, 320, 8, FULL_SB_N4_LUMA),
            RectLumaPlan::Middle {
                p_angle: 126,
                use_tcq: false,
            },
        ),
        (
            "32x16 vertical +1",
            IntraYMode::Vertical,
            1,
            ctx(4, 0, 8, 4),
            RectLumaPlan::Middle {
                p_angle: 93,
                use_tcq: false,
            },
        ),
        (
            "8x4 d157",
            IntraYMode::D157,
            0,
            ctx(0, 4, 2, 1),
            RectLumaPlan::Middle {
                p_angle: 157,
                use_tcq: false,
            },
        ),
    ] {
        assert_eq!(
            rect_luma_plan(
                &luma_modes_with_angle(mode, angle_delta),
                block,
                false,
                FULL_SB_N4_LUMA,
            ),
            Ok(expected),
            "{label}"
        );
    }
}

#[test]
fn classifies_rect_directional_luma_angles() {
    for (p_angle, expected) in [
        (
            45,
            RectLumaPlan::OneSidedAbove {
                p_angle: 45,
                use_tcq: false,
            },
        ),
        (
            157,
            RectLumaPlan::Middle {
                p_angle: 157,
                use_tcq: false,
            },
        ),
        (
            203,
            RectLumaPlan::OneSidedLeft {
                p_angle: 203,
                use_tcq: false,
            },
        ),
    ] {
        assert_eq!(
            rect_luma_plan_for_parts(IntraYMode::D135, Some(p_angle), false),
            Ok(expected),
        );
    }
}

#[test]
fn square_d67_angle_delta_uses_rect_residual_path_when_square_plan_rejects() {
    let first_col_block = ctx(128, 0, 32, 32);
    let modes = GeneralIntraBlockModes::luma_only(
        crate::bitstream::tile_payload::GeneralIntraLumaBlockMode {
            y_mode: IntraYMode::D67,
            angle_delta_y: -1,
            intra_joint_mode: crate::bitstream::tile_payload::IntraJointMode::DC,
            mrl: MrlSelection::Disabled,
            fsc_mode: 0,
            use_dip: 0,
            dip_transpose: 0,
            dip_mode: 0,
            use_dpcm_y: 0,
            dpcm_mode_y: 0,
        },
    );

    assert_eq!(
        rect_luma_plan(&modes, first_col_block, false, 32),
        Ok(RectLumaPlan::OneSidedAbove {
            p_angle: 64,
            use_tcq: false,
        })
    );
}

#[test]
fn retains_rect_directional_chroma_context() {
    for (label, mode, angle_delta, expected) in [
        (
            "above-left d135 follow",
            SupportedChromaMode::D135Follow,
            -3,
            RectChromaPlan::Directional {
                mode: SupportedChromaMode::D135Follow,
                angle_delta_uv: -3,
                dpcm: None,
            },
        ),
        (
            "top-row d113 follow with left-only edge",
            SupportedChromaMode::D113Follow,
            -1,
            RectChromaPlan::Directional {
                mode: SupportedChromaMode::D113Follow,
                angle_delta_uv: -1,
                dpcm: None,
            },
        ),
        (
            "top-row d157 follow with left-only edge",
            SupportedChromaMode::D157Follow,
            -1,
            RectChromaPlan::Directional {
                mode: SupportedChromaMode::D157Follow,
                angle_delta_uv: -1,
                dpcm: None,
            },
        ),
        (
            "top-row d135 with left-only edge",
            SupportedChromaMode::D135,
            0,
            RectChromaPlan::Directional {
                mode: SupportedChromaMode::D135,
                angle_delta_uv: 0,
                dpcm: None,
            },
        ),
    ] {
        assert_rect_chroma_plan(mode, angle_delta, expected, label);
    }
}

#[test]
fn follows_luma_angle_delta_for_directional_chroma() {
    assert_eq!(
        rect_chroma_plan_for_mode(SupportedChromaMode::VerticalFollow, -1, None),
        RectChromaPlan::Directional {
            mode: SupportedChromaMode::VerticalFollow,
            angle_delta_uv: -1,
            dpcm: None,
        }
    );
    assert_eq!(
        rect_chroma_plan_for_mode(SupportedChromaMode::VerticalFollow, 1, None),
        RectChromaPlan::Directional {
            mode: SupportedChromaMode::VerticalFollow,
            angle_delta_uv: 1,
            dpcm: None,
        }
    );
    assert_eq!(
        rect_chroma_plan_for_mode(SupportedChromaMode::Vertical, 0, None),
        RectChromaPlan::Directional {
            mode: SupportedChromaMode::Vertical,
            angle_delta_uv: 0,
            dpcm: None,
        }
    );
    assert_eq!(
        rect_chroma_plan_for_mode(
            SupportedChromaMode::Vertical,
            0,
            Some(DpcmDirection::Vertical),
        ),
        RectChromaPlan::Directional {
            mode: SupportedChromaMode::Vertical,
            angle_delta_uv: 0,
            dpcm: Some(DpcmDirection::Vertical),
        }
    );

    let horizontal_dpcm = Some(DpcmDirection::Horizontal);
    let angle_delta =
        inherited_chroma_angle_delta(IntraYMode::Horizontal.value(), IntraYMode::Horizontal, 2);
    assert_eq!(
        rect_chroma_plan_for_mode(
            SupportedChromaMode::Horizontal,
            angle_delta,
            horizontal_dpcm,
        ),
        RectChromaPlan::Directional {
            mode: SupportedChromaMode::Horizontal,
            angle_delta_uv: 2,
            dpcm: horizontal_dpcm,
        }
    );
}

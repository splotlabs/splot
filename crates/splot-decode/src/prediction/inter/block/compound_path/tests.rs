// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use splot_core::symbol::{CdfUpdateMode, Symbol, SymbolDecoderConfig};
use splot_core::symbol_encoder::{SymbolEncoder, SymbolEncoderConfig};

use super::*;
use crate::bitstream::tile_payload::FrameCdfSubset;

const TILE_OFFSET: ByteOffset = ByteOffset::new(0);

#[test]
fn refinemv_deblock_subpu_geometry_preserves_rectangular_blocks() {
    use crate::filters::deblock::DeblockSubPuSize;

    assert_eq!(
        compound_deblock_sub_pu_size(false, true, 32, 32),
        Some(DeblockSubPuSize::new(16, 16))
    );
    assert_eq!(
        compound_deblock_sub_pu_size(false, true, 8, 32),
        Some(DeblockSubPuSize::new(8, 16))
    );
    assert_eq!(
        compound_deblock_sub_pu_size(false, true, 32, 8),
        Some(DeblockSubPuSize::new(16, 8))
    );
}

#[test]
fn optflow_deblock_subpu_geometry_takes_precedence_over_refinemv() {
    use crate::filters::deblock::DeblockSubPuSize;

    assert_eq!(
        compound_deblock_sub_pu_size(true, true, 8, 16),
        Some(DeblockSubPuSize::square(8))
    );
}

fn encode_wedge_compound_blend() -> Vec<u8> {
    let mut tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut encoder = SymbolEncoder::with_config(
        SymbolEncoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Enabled),
    );
    tile.with_row_mut(TileCdfSelector::CompGroupIdx { ctx: 0 }, |row| {
        encoder.write_symbol_u16(row, Symbol::new(1))
    })
    .unwrap()
    .unwrap();
    tile.with_row_mut(TileCdfSelector::CompoundType, |row| {
        encoder.write_symbol_u16(row, Symbol::new(0))
    })
    .unwrap()
    .unwrap();
    tile.with_row_mut(TileCdfSelector::WedgeQuad, |row| {
        encoder.write_symbol_u16(row, Symbol::new(0))
    })
    .unwrap()
    .unwrap();
    tile.with_row_mut(TileCdfSelector::WedgeAngle { quad: 0 }, |row| {
        encoder.write_symbol_u16(row, Symbol::new(0))
    })
    .unwrap()
    .unwrap();
    tile.with_row_mut(TileCdfSelector::WedgeDist2, |row| {
        encoder.write_symbol_u16(row, Symbol::new(0))
    })
    .unwrap()
    .unwrap();
    encoder.write_bool(true).unwrap();
    encoder.finish().unwrap().into_bytes()
}

fn symbol_decoder(payload: &[u8]) -> SymbolDecoder<'_> {
    SymbolDecoder::with_base_and_config(
        payload,
        TILE_OFFSET,
        SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Enabled),
    )
    .unwrap()
}

fn compound_motion_grid_and_block(mv0: Mv, mv1: Mv) -> (NeighbourMvGrid, MvBlockContext) {
    let mut grid = NeighbourMvGrid::new(16, 16).unwrap();
    grid.record_compound_block(
        0,
        0,
        2,
        2,
        0,
        1,
        true,
        true,
        mv0,
        mv1,
        false,
        0,
        false,
        false,
        mc::CWP_EQUAL,
        false,
        BlockPrecisionRecord::default(),
        [None, None],
    );
    let block = MvBlockContext {
        mi_row: 0,
        mi_col: 2,
        bw4: 2,
        bh4: 2,
        sb_h4: 32,
        ref_frame0: 0,
        ref_frame1: Some(1),
        mi_rows: 16,
        mi_cols: 16,
    };
    (grid, block)
}

fn compound_motion_contexts() -> (
    crate::prediction::inter::find_mv_stack::ModeContext,
    BlockNeighbourContext,
) {
    let (grid, block) = compound_motion_grid_and_block(Mv::ZERO, Mv::ZERO);
    (
        find_mode_ctx(&grid, &block),
        block_neighbour_ctx(&grid, &block),
    )
}

#[test]
fn compound_ref_contexts_cover_every_valid_reference_count() {
    let (_, block) = compound_motion_grid_and_block(Mv::ZERO, Mv::ZERO);
    let grid = NeighbourMvGrid::new(16, 16).unwrap();
    let neighbour_ctx = block_neighbour_ctx(&grid, &block);

    for count in 0..=7 {
        let contexts = compound_ref_contexts(&neighbour_ctx, count).unwrap();
        assert_eq!(contexts[..count], [1; 7][..count]);
        assert_eq!(contexts[count..], [0; 7][count..]);
    }
}

#[test]
fn compound_ref_contexts_keep_invalid_reference_count_fail_closed() {
    let (_, block) = compound_motion_grid_and_block(Mv::ZERO, Mv::ZERO);
    let grid = NeighbourMvGrid::new(16, 16).unwrap();
    let neighbour_ctx = block_neighbour_ctx(&grid, &block);

    let error = compound_ref_contexts(&neighbour_ctx, 8).unwrap_err();
    assert!(matches!(
        error,
        crate::error::DecodeError::HeaderState {
            source: crate::DecodeHeaderStateError::InvalidInterReferenceMap
        }
    ));
}

#[test]
fn compound_ref_distance_signs_cover_every_valid_reference_count() {
    let mut reference = InterReferenceState::<u8>::empty().unwrap();
    reference.ref_order_hint = vec![9, 11, 10, 10, 10, 10, 10];
    let ref_frame_idx = [0, 1, 2, 3, 4, 5, 6];
    let expected = [true, false, true, true, true, true, true];

    for count in 0..=7 {
        let signs = compound_ref_distance_signs(&ref_frame_idx, &reference, 10, count).unwrap();
        assert_eq!(signs[..count], expected[..count]);
        assert_eq!(signs[count..], [true; 7][count..]);
    }
}

#[test]
fn compound_ref_distance_signs_keep_invalid_reference_map_fail_closed() {
    let reference = InterReferenceState::<u8>::empty().unwrap();
    let error = compound_ref_distance_signs(&[], &reference, 10, 1).unwrap_err();
    assert!(matches!(
        error,
        crate::error::DecodeError::HeaderState {
            source: crate::DecodeHeaderStateError::InvalidInterReferenceMap
        }
    ));
}

#[test]
fn compound_ref_distance_signs_keep_missing_order_hint_fail_closed() {
    let reference = InterReferenceState::<u8>::empty().unwrap();
    let error = compound_ref_distance_signs(&[0], &reference, 10, 1).unwrap_err();

    assert!(matches!(
        error,
        crate::error::DecodeError::ReferenceState {
            source: crate::DecodeReferenceStateError::SlotOutOfRange {
                slot: 0,
                slot_count: 0,
            }
        }
    ));
}

#[test]
fn compound_opfl_consumers_keep_missing_header_state_fail_closed() {
    let fixture = include_bytes!(
        "../../../../../../../tests/conformance/vectors/valid/syn-2frame-inter-64x64.ivf"
    );
    let (_, mut core, _) =
        crate::prediction::inter::tests::parse_inter_core_for_validation(fixture).unwrap();
    core.inter.as_mut().unwrap().opfl_refine_type = None;

    let error = compound_opfl_refine_type(&core, TILE_OFFSET).unwrap_err();
    assert!(matches!(
        error,
        crate::error::DecodeError::InternalState {
            reason: "compound_missing_opfl_refine_type",
            byte_offset: TILE_OFFSET,
        }
    ));

    let compound = crate::prediction::inter::compound::CompoundBlockSyntax {
        y_mode: CompoundYMode::NearNear,
        use_optflow: false,
        ref_frame0: 0,
        ref_frame1: 1,
        mv0: Mv::ZERO,
        mv1: Mv::ZERO,
    };
    let error = compound_refinemv_mode_allowed(&core, compound, TILE_OFFSET).unwrap_err();
    assert!(matches!(
        error,
        crate::error::DecodeError::InternalState {
            reason: "compound_refinemv_missing_opfl_refine_type",
            byte_offset: TILE_OFFSET,
        }
    ));

    core.frame_size = None;
    let reference = InterReferenceState::<u8>::empty().unwrap();
    let error = compound_sized_reference_distances(
        &core,
        &reference,
        &[],
        compound,
        CompoundReferencePath::Opfl,
        TILE_OFFSET,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        crate::error::DecodeError::InternalState {
            reason: "compound_opfl_missing_frame_size",
            byte_offset: TILE_OFFSET,
        }
    ));
}

#[test]
fn compound_reference_order_hint_covers_every_valid_reference_index() {
    let mut reference = InterReferenceState::<u8>::empty().unwrap();
    reference.ref_order_hint = vec![9, 11, 13, 15, 17, 19, 21];
    let ref_frame_idx = [0, 1, 2, 3, 4, 5, 6];

    for (ref_frame, expected) in reference.ref_order_hint.iter().copied().enumerate() {
        assert_eq!(
            compound_reference_order_hint(&reference, &ref_frame_idx, ref_frame as i8).unwrap(),
            CompoundOrderHint::Value(i64::from(expected))
        );
    }
}

#[test]
fn compound_reference_order_hint_keeps_reference_list_bounds_fail_closed() {
    let mut reference = InterReferenceState::<u8>::empty().unwrap();
    reference.ref_order_hint = vec![9];
    let error = compound_reference_order_hint(&reference, &[0], 1).unwrap_err();

    assert!(matches!(
        error,
        crate::error::DecodeError::ReferenceState {
            source: crate::DecodeReferenceStateError::ReferenceListIndexOutOfRange {
                index: 1,
                list_len: 1,
            }
        }
    ));
}

#[test]
fn compound_reference_order_hint_keeps_negative_reference_index_fail_closed() {
    let mut reference = InterReferenceState::<u8>::empty().unwrap();
    reference.ref_order_hint = vec![9];
    let error = compound_reference_order_hint(&reference, &[0], -1).unwrap_err();

    assert!(matches!(
        error,
        crate::error::DecodeError::ReferenceState {
            source: crate::DecodeReferenceStateError::ReferenceListIndexOutOfRange {
                index: -1,
                list_len: 1,
            }
        }
    ));
}

#[test]
fn compound_reference_order_hint_keeps_slot_conversion_and_bounds_fail_closed() {
    let mut reference = InterReferenceState::<u8>::empty().unwrap();
    reference.ref_order_hint = vec![9];
    let error = compound_reference_order_hint(&reference, &[u32::MAX], 0).unwrap_err();
    let expected_slot = usize::try_from(u32::MAX).unwrap_or(usize::MAX);

    assert!(matches!(
        error,
        crate::error::DecodeError::ReferenceState {
            source: crate::DecodeReferenceStateError::SlotOutOfRange {
                slot,
                slot_count: 1,
            }
        } if slot == expected_slot
    ));
}

#[test]
fn compound_reference_facts_keep_missing_width_fail_closed() {
    let mut reference = InterReferenceState::<u8>::empty().unwrap();
    reference.ref_order_hint.push(9);
    let error = compound_reference_facts(&reference, &[0], 0, TILE_OFFSET).unwrap_err();

    assert!(matches!(
        error,
        crate::error::DecodeError::ReferenceState {
            source: crate::DecodeReferenceStateError::SlotOutOfRange {
                slot: 0,
                slot_count: 0,
            }
        }
    ));
}

#[test]
fn compound_reference_facts_keep_missing_height_fail_closed() {
    let mut reference = InterReferenceState::<u8>::empty().unwrap();
    reference.ref_order_hint.push(9);
    reference.ref_frame_width.push(64);
    let error = compound_reference_facts(&reference, &[0], 0, TILE_OFFSET).unwrap_err();

    assert!(matches!(
        error,
        crate::error::DecodeError::ReferenceState {
            source: crate::DecodeReferenceStateError::SlotOutOfRange {
                slot: 0,
                slot_count: 0,
            }
        }
    ));
}

#[test]
fn compound_reference_order_hint_maps_the_full_relative_distance_domain() {
    let mut reference = InterReferenceState::<u8>::empty().unwrap();
    reference.ref_order_hint = vec![u32::MAX, i32::MAX as u32 + 1];

    assert_eq!(
        compound_reference_order_hint(&reference, &[0, 1], 0).unwrap(),
        CompoundOrderHint::Restricted
    );
    assert_eq!(
        compound_reference_order_hint(&reference, &[0, 1], 1).unwrap(),
        CompoundOrderHint::Value(i64::from(i32::MAX) + 1)
    );
    assert_eq!(
        CompoundOrderHint::Value(i64::from(i32::MAX) + 1)
            .relative_dist(CompoundOrderHint::current(i32::MAX)),
        1
    );
}

#[test]
fn compound_furthest_future_ref_excludes_restricted_references() {
    let mut reference = InterReferenceState::<u8>::empty().unwrap();
    reference.ref_order_hint = vec![u32::MAX, 15];

    assert_eq!(
        compound_furthest_future_ref(&reference, &[0, 1], CompoundOrderHint::current(10), 2)
            .unwrap(),
        Some(1)
    );
}

#[test]
fn compound_furthest_future_ref_ranks_by_raw_order_hint() {
    let mut reference = InterReferenceState::<u8>::empty().unwrap();
    reference.ref_order_hint = vec![i32::MAX as u32 + 128, i32::MAX as u32 + 129];

    assert_eq!(
        compound_furthest_future_ref(&reference, &[0, 1], CompoundOrderHint::current(i32::MAX), 2)
            .unwrap(),
        Some(1)
    );
}

#[test]
fn compound_joint_projection_preserves_restricted_reference_semantics() {
    let current = CompoundOrderHint::current(10);
    let ordinary = CompoundOrderHint::Value(200);
    let restricted = CompoundOrderHint::Restricted;
    let projection = compound_joint_mv_projection_from_order_hints(current, ordinary, restricted);

    assert_eq!(projection.base_list, 1);
    assert_eq!(projection.first_dist, 127);
    assert_eq!(projection.second_dist, -127);
    assert!(compound_references_same_side(
        current,
        restricted,
        CompoundOrderHint::Value(0),
    ));
}

fn encode_compound_local_warp(ctx: usize, enabled: bool) -> Vec<u8> {
    let mut tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut encoder = SymbolEncoder::with_config(
        SymbolEncoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Enabled),
    );
    tile.with_row_mut(TileCdfSelector::UseLocalWarp { ctx }, |row| {
        encoder.write_symbol_u16(row, Symbol::new(u8::from(enabled)))
    })
    .unwrap()
    .unwrap();
    encoder.finish().unwrap().into_bytes()
}

#[test]
fn compound_blend_reads_wedge_index_and_sign() {
    let payload = encode_wedge_compound_blend();
    let mut tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut symbols = symbol_decoder(&payload);

    let blend = read_compound_blend_syntax(
        &mut tile,
        &mut symbols,
        CompoundBlendToolConfig {
            masked_enabled: true,
            implicit_mask: false,
        },
        CompoundBlendInput {
            skip_mode: false,
            use_optflow: false,
            joint_amvd: false,
            switchable_refinemv_on: false,
            n4w: 2,
            n4h: 2,
            block_size_index: 3,
            comp_group_idx_ctx: 0,
        },
        TILE_OFFSET,
    )
    .unwrap();

    assert_eq!(
        blend,
        mc::CompoundBlend::Wedge {
            index: 0,
            sign: true,
        }
    );
}

#[test]
fn compound_optflow_forces_average_without_blend_symbols() {
    let payload = encode_wedge_compound_blend();
    let mut tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut symbols = symbol_decoder(&payload);
    let before = symbols.checkpoint();

    let blend = read_compound_blend_syntax(
        &mut tile,
        &mut symbols,
        CompoundBlendToolConfig {
            masked_enabled: true,
            implicit_mask: true,
        },
        CompoundBlendInput {
            skip_mode: false,
            use_optflow: true,
            joint_amvd: false,
            switchable_refinemv_on: false,
            n4w: 2,
            n4h: 2,
            block_size_index: 3,
            comp_group_idx_ctx: 0,
        },
        TILE_OFFSET,
    )
    .unwrap();

    assert_eq!(blend, mc::CompoundBlend::average_with_implicit_mask(true));
    assert_eq!(symbols.checkpoint(), before);
}

/// AV2 § 5.20.7.6: a joint-AMVD compound mode (JOINT_NEWMV/JOINT_NEWMV_OPTFLOW
/// with `use_amvd`) does not signal `comp_group_idx`, so the blend stays average
/// and no `S()` symbol is consumed even when masked compound is enabled.
#[test]
fn compound_joint_amvd_forces_average_without_blend_symbols() {
    let payload = encode_wedge_compound_blend();
    let mut tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut symbols = symbol_decoder(&payload);
    let before = symbols.checkpoint();

    let blend = read_compound_blend_syntax(
        &mut tile,
        &mut symbols,
        CompoundBlendToolConfig {
            masked_enabled: true,
            implicit_mask: true,
        },
        CompoundBlendInput {
            skip_mode: false,
            use_optflow: false,
            joint_amvd: true,
            switchable_refinemv_on: false,
            n4w: 2,
            n4h: 2,
            block_size_index: 3,
            comp_group_idx_ctx: 0,
        },
        TILE_OFFSET,
    )
    .unwrap();

    assert_eq!(blend, mc::CompoundBlend::average_with_implicit_mask(true));
    assert_eq!(symbols.checkpoint(), before);
}

#[test]
fn compound_optflow_forces_sharp_interp_without_symbols() {
    let payload = encode_compound_local_warp(0, false);
    let mut tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut symbols = symbol_decoder(&payload);
    let before = symbols.checkpoint();

    let interp = resolve_compound_interp_filter(
        &mut tile,
        &mut symbols,
        FrameInterpolationFilter::Switchable,
        true,
        true,
        0,
        TILE_OFFSET,
    )
    .unwrap();

    assert_eq!(interp, ReconInterpolationFilter::EightTapSharp);
    assert_eq!(symbols.checkpoint(), before);
}

#[test]
fn narrow_global_compound_block_needs_interp_filter() {
    assert!(compound_needs_interp_filter(
        1,
        4,
        CompoundYMode::GlobalGlobal,
        false
    ));
    assert!(!compound_needs_interp_filter(
        2,
        2,
        CompoundYMode::GlobalGlobal,
        false
    ));
}

#[test]
fn compound_simple_motion_consumes_local_warp_gate() {
    let (mode_ctx, neighbour_ctx) = compound_motion_contexts();
    assert!(mode_ctx.warp_sample_found && mode_ctx.warp_sample_found1);
    let ctx = neighbour_ctx.use_local_warp_ctx();
    let payload = encode_compound_local_warp(ctx, false);
    let mut tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut symbols = symbol_decoder(&payload);

    let local_warp = read_compound_motion_mode_syntax(
        &mut tile,
        &mut symbols,
        true,
        &neighbour_ctx,
        TILE_OFFSET,
    )
    .unwrap();

    assert!(!local_warp);
    symbols.exit_symbol().unwrap();
}

#[test]
fn compound_local_warp_symbol_reports_localwarp_when_set() {
    let (_, neighbour_ctx) = compound_motion_contexts();
    let ctx = neighbour_ctx.use_local_warp_ctx();
    let payload = encode_compound_local_warp(ctx, true);
    let mut tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut symbols = symbol_decoder(&payload);

    let local_warp = read_compound_motion_mode_syntax(
        &mut tile,
        &mut symbols,
        true,
        &neighbour_ctx,
        TILE_OFFSET,
    )
    .unwrap();

    assert!(local_warp);
    symbols.exit_symbol().unwrap();
}

#[test]
fn compound_local_warp_derives_a_model_per_explicit_reference_list() {
    let (grid, mut block) =
        compound_motion_grid_and_block(Mv { row: 24, col: -8 }, Mv { row: -16, col: 40 });
    block.ref_frame1 = None;
    let models = compound_local_warp_models(
        &grid,
        &block,
        1,
        Mv { row: 24, col: -8 },
        Mv { row: -16, col: 40 },
        block.mi_row,
        block.mi_col,
        block.bw4,
        block.bh4,
        TILE_OFFSET,
    )
    .unwrap();

    assert!(models[0].is_some(), "list-0 warp model");
    assert!(models[1].is_some(), "list-1 warp model");
}

#[test]
fn compound_local_warp_uses_translation_for_a_list_without_samples() {
    let mut grid = NeighbourMvGrid::new(16, 16).unwrap();
    grid.record_block(
        0,
        0,
        2,
        2,
        true,
        0,
        None,
        true,
        Mv { row: 24, col: -8 },
        false,
        0,
        false,
        BlockPrecisionRecord::default(),
    );
    let block = MvBlockContext {
        mi_row: 0,
        mi_col: 2,
        bw4: 2,
        bh4: 2,
        sb_h4: 32,
        ref_frame0: 0,
        ref_frame1: Some(1),
        mi_rows: 16,
        mi_cols: 16,
    };
    let models = compound_local_warp_models(
        &grid,
        &block,
        1,
        Mv { row: 24, col: -8 },
        Mv::ZERO,
        0,
        2,
        2,
        2,
        TILE_OFFSET,
    )
    .unwrap();

    assert!(models[0].is_some(), "list-0 warp model");
    assert_eq!(models[1], None, "list-1 translational fallback");
}

#[test]
fn compound_warp_blend_forces_plain_average_but_keeps_masks() {
    let average = mc::CompoundBlend::average_with_implicit_mask(true);
    assert_eq!(
        compound_warp_blend(average, true),
        mc::CompoundBlend::Average {
            implicit_mask: false,
            cwp_weight: mc::CWP_EQUAL,
        }
    );
    assert_eq!(compound_warp_blend(average, false), average);
    let wedge = mc::CompoundBlend::Wedge {
        index: 3,
        sign: true,
    };
    assert_eq!(compound_warp_blend(wedge, true), wedge);
}

#[test]
fn compound_local_warp_motion_suppresses_cwp() {
    assert!(!compound_cwp_signal_allowed(
        true,
        CompoundCwpInput {
            y_mode: CompoundYMode::NearNear,
            jmvd_scale_mode: 0,
            skip_mode: false,
            use_optflow: false,
            use_refinemv: false,
            motion_simple: false,
            ref_frame0: 0,
            ref_frame1: 1,
            blend: mc::CompoundBlend::default(),
        },
    ));
}

#[test]
fn compound_optflow_suppresses_local_warp_gate() {
    let compound = super::super::super::compound::CompoundBlockSyntax {
        y_mode: CompoundYMode::NewNew,
        use_optflow: true,
        ref_frame0: 0,
        ref_frame1: 1,
        mv0: Mv::ZERO,
        mv1: Mv::ZERO,
    };

    assert!(!compound_local_warp_signal_allowed(
        compound,
        2,
        2,
        false,
        REFINE_SWITCHABLE,
        [true; 2],
        true,
    ));
}

#[test]
fn joint_mvd_projection_uses_reference_distance_ratio() {
    assert_eq!(
        project_joint_mvd(Mv { row: 96, col: -48 }, 1, 2),
        Mv { row: 48, col: -24 }
    );
    assert_eq!(
        project_joint_mvd(Mv { row: 96, col: -48 }, -1, 2),
        Mv { row: -48, col: 24 }
    );
}

#[test]
fn joint_mvd_scale_mode_matches_amvd_and_non_amvd_axes() {
    assert_eq!(
        scale_joint_projected_mvd(Mv { row: 10, col: 6 }, 1, true),
        Mv { row: 20, col: 12 }
    );
    assert_eq!(
        scale_joint_projected_mvd(Mv { row: 10, col: 6 }, 2, false),
        Mv { row: 10, col: 12 }
    );
    assert_eq!(
        scale_joint_projected_mvd(Mv { row: 10, col: 6 }, 4, false),
        Mv { row: 10, col: 3 }
    );
}

#[test]
fn compound_cwp_mode_allows_unscaled_joint_newmv() {
    assert!(compound_cwp_mode_allowed(CompoundYMode::NearNear, 4));
    assert!(compound_cwp_mode_allowed(CompoundYMode::JointNew, 0));
    assert!(!compound_cwp_mode_allowed(CompoundYMode::JointNew, 1));
    assert!(!compound_cwp_mode_allowed(CompoundYMode::NearNew, 0));
}

#[test]
fn compound_optflow_suppresses_cwp_symbols() {
    assert!(!compound_cwp_signal_allowed(
        true,
        CompoundCwpInput {
            y_mode: CompoundYMode::NearNear,
            jmvd_scale_mode: 0,
            skip_mode: false,
            use_optflow: true,
            use_refinemv: false,
            motion_simple: true,
            ref_frame0: 0,
            ref_frame1: 1,
            blend: mc::CompoundBlend::default(),
        },
    ));
}

#[test]
fn compound_refinemv_suppresses_cwp_symbols() {
    assert!(!compound_cwp_signal_allowed(
        true,
        CompoundCwpInput {
            y_mode: CompoundYMode::NearNear,
            jmvd_scale_mode: 0,
            skip_mode: false,
            use_optflow: false,
            use_refinemv: true,
            motion_simple: true,
            ref_frame0: 0,
            ref_frame1: 1,
            blend: mc::CompoundBlend::default(),
        },
    ));
}

#[test]
fn refine_all_requires_equal_weight_non_global_average() {
    let equal = mc::CompoundBlend::default();
    let unequal = equal.average_with_cwp_weight(12);
    let masked = mc::CompoundBlend::DiffWeighted { inverse: false };
    let mut compound = crate::prediction::inter::compound::CompoundBlockSyntax {
        y_mode: CompoundYMode::NearNear,
        use_optflow: false,
        ref_frame0: 0,
        ref_frame1: 1,
        mv0: Mv::ZERO,
        mv1: Mv::ZERO,
    };

    assert!(compound_all_opfl_block_allowed(compound, 2, 2, equal));
    assert!(!compound_all_opfl_block_allowed(compound, 1, 2, equal));
    assert!(!compound_all_opfl_block_allowed(compound, 2, 2, unequal));
    assert!(!compound_all_opfl_block_allowed(compound, 2, 2, masked));
    compound.y_mode = CompoundYMode::GlobalGlobal;
    assert!(!compound_all_opfl_block_allowed(compound, 2, 2, equal));
}

#[test]
fn compound_refinemv_switchability_matches_mode_and_optflow() {
    let mut compound = crate::prediction::inter::compound::CompoundBlockSyntax {
        y_mode: CompoundYMode::NearNear,
        use_optflow: false,
        ref_frame0: 0,
        ref_frame1: 1,
        mv0: Mv::ZERO,
        mv1: Mv::ZERO,
    };
    assert!(!compound_refinemv_is_switchable(
        compound,
        REFINE_SWITCHABLE
    ));
    compound.y_mode = CompoundYMode::NearNew;
    assert!(compound_refinemv_is_switchable(compound, REFINE_SWITCHABLE));
    assert!(!compound_refinemv_mode_allowed_for_type(
        compound,
        REFINE_SWITCHABLE
    ));
    compound.use_optflow = true;
    assert!(compound_refinemv_mode_allowed_for_type(
        compound,
        REFINE_SWITCHABLE
    ));
    compound.y_mode = CompoundYMode::JointNew;
    assert!(!compound_refinemv_is_switchable(
        compound,
        REFINE_SWITCHABLE
    ));
    assert!(compound_refinemv_is_switchable(compound, REFINE_ALL));
    compound.y_mode = CompoundYMode::GlobalGlobal;
    compound.use_optflow = false;
    assert!(!compound_refinemv_mode_allowed_for_type(
        compound, REFINE_ALL
    ));
}

#[test]
fn compound_masked_blends_cancel_default_refinemv() {
    for (blend, expected) in [
        (mc::CompoundBlend::default(), true),
        (
            mc::CompoundBlend::Wedge {
                index: 0,
                sign: false,
            },
            false,
        ),
        (mc::CompoundBlend::DiffWeighted { inverse: false }, false),
    ] {
        assert_eq!(compound_refinemv_active_after_blend(true, blend), expected);
    }
    assert!(!compound_refinemv_active_after_blend(
        false,
        mc::CompoundBlend::default()
    ));
}

#[test]
fn wedge_temporal_storage_keeps_only_a_dominant_reference() {
    let blend = mc::CompoundBlend::Wedge {
        index: 0,
        sign: false,
    };
    assert_eq!(
        wedge_temporal_allowed_lists(blend, 64, 64, 0, 0).unwrap(),
        [false, true]
    );
    assert_eq!(
        wedge_temporal_allowed_lists(blend, 64, 64, 56, 0).unwrap(),
        [true, false]
    );
    assert_eq!(
        wedge_temporal_allowed_lists(blend, 64, 64, 32, 24).unwrap(),
        [true; 2]
    );
}

#[test]
fn compound_opfl_near_near_uses_one_paired_drl_idx() {
    let mut compound = crate::prediction::inter::compound::CompoundBlockSyntax {
        y_mode: CompoundYMode::NearNear,
        use_optflow: false,
        ref_frame0: 0,
        ref_frame1: 1,
        mv0: Mv::ZERO,
        mv1: Mv::ZERO,
    };

    assert!(compound_reads_second_drl(compound));
    compound.use_optflow = true;
    assert!(!compound_reads_second_drl(compound));
}

#[test]
fn compound_opfl_near_near_selects_the_indexed_paired_candidate() {
    let pairs = [
        [Mv { row: 8, col: 16 }, Mv { row: -8, col: 24 }],
        [Mv { row: -4, col: -149 }, Mv { row: 4, col: 187 }],
    ];
    let stack = crate::prediction::inter::find_mv_stack::CompoundMvStack::from_candidates(
        pairs
            .map(
                |mvs| crate::prediction::inter::find_mv_stack::CompoundMvCandidate {
                    mvs,
                    cwp_weight: mc::CWP_EQUAL,
                },
            )
            .to_vec(),
    );
    assert_eq!(stack.candidate(0).mvs, pairs[0]);
    assert_eq!(stack.candidate(1).mvs, pairs[1]);

    let compound = crate::prediction::inter::compound::CompoundBlockSyntax {
        y_mode: CompoundYMode::NearNear,
        use_optflow: true,
        ref_frame0: 0,
        ref_frame1: 1,
        mv0: Mv::ZERO,
        mv1: Mv::ZERO,
    };
    let independent_called = core::cell::Cell::new(false);
    let selected = select_near_near_candidates(
        compound_reads_second_drl(compound).then_some([0, 0]),
        1,
        |idx| stack.candidate(idx).mvs,
        |_| {
            independent_called.set(true);
            pairs[0]
        },
    );
    assert_eq!(selected, pairs[1]);
    assert!(!independent_called.get());
}

#[test]
fn compound_non_skip_near_mode_reads_second_drl_idx() {
    let compound = crate::prediction::inter::compound::CompoundBlockSyntax {
        y_mode: CompoundYMode::NearNear,
        use_optflow: false,
        ref_frame0: 0,
        ref_frame1: 1,
        mv0: Mv::ZERO,
        mv1: Mv::ZERO,
    };

    assert!(compound_reads_second_drl(compound));
}

#[test]
fn skip_mode_default_pair_treats_restricted_order_hint_as_zero_distance() {
    assert_eq!(
        skip_mode_default_pair(
            0,
            Some((CompoundOrderHint::Restricted, CompoundOrderHint::Value(1),)),
        ),
        (0, 1)
    );
    assert_eq!(skip_mode_default_pair(0, None), (0, 0));
}

#[test]
fn compound_local_warp_neighbour_raises_warp_contexts() {
    let (grid, block) = compound_motion_grid_and_block(Mv::ZERO, Mv::ZERO);
    let baseline = block_neighbour_ctx(&grid, &block);
    assert_eq!(baseline.use_local_warp_ctx(), 0);
    assert_eq!(baseline.use_extend_warp_ctx(), 0);

    let mut grid = NeighbourMvGrid::new(16, 16).unwrap();
    grid.record_compound_block(
        0,
        0,
        2,
        2,
        0,
        1,
        true,
        true,
        Mv::ZERO,
        Mv::ZERO,
        false,
        0,
        false,
        false,
        mc::CWP_EQUAL,
        false,
        BlockPrecisionRecord::default(),
        [
            Some([320, -640, 65_536 + 256, -128, 192, 65_536 - 320]),
            Some([-960, 480, 65_536 - 512, 96, -64, 65_536 + 448]),
        ],
    );
    let warp_ctx = block_neighbour_ctx(&grid, &block);
    assert!(warp_ctx.use_local_warp_ctx() > 0);
    assert!(warp_ctx.use_extend_warp_ctx() > 0);
}

fn encode_use_refinemv(ctx: usize, enabled: bool) -> Vec<u8> {
    let mut tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut encoder = SymbolEncoder::with_config(
        SymbolEncoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Enabled),
    );
    tile.with_row_mut(TileCdfSelector::UseRefinemv { ctx }, |row| {
        encoder.write_symbol_u16(row, Symbol::new(u8::from(enabled)))
    })
    .unwrap()
    .unwrap();
    encoder.finish().unwrap().into_bytes()
}

#[test]
fn switchable_refinemv_reads_both_values_at_the_optflow_context() {
    for enabled in [false, true] {
        let payload = encode_use_refinemv(8, enabled);
        let mut tile = FrameCdfSubset::from_defaults().tile_copy();
        let mut symbols = symbol_decoder(&payload);

        let use_refinemv = read_compound_use_refinemv_syntax(
            &mut tile,
            &mut symbols,
            CompoundYMode::NearNew,
            true,
            TILE_OFFSET,
        )
        .unwrap();

        assert_eq!(use_refinemv, enabled);
        symbols.exit_symbol().unwrap();
    }
}

#[test]
fn switchable_refinemv_context_drops_one_past_global_for_optflow() {
    let payload = encode_use_refinemv(10, true);
    let mut tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut symbols = symbol_decoder(&payload);

    let use_refinemv = read_compound_use_refinemv_syntax(
        &mut tile,
        &mut symbols,
        CompoundYMode::NewNew,
        true,
        TILE_OFFSET,
    )
    .unwrap();

    assert!(use_refinemv);
    symbols.exit_symbol().unwrap();

    let payload = encode_use_refinemv(6, false);
    let mut tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut symbols = symbol_decoder(&payload);

    let use_refinemv = read_compound_use_refinemv_syntax(
        &mut tile,
        &mut symbols,
        CompoundYMode::JointNew,
        false,
        TILE_OFFSET,
    )
    .unwrap();

    assert!(!use_refinemv);
    symbols.exit_symbol().unwrap();
}

#[test]
fn switchable_refinemv_on_forces_average_without_blend_symbols() {
    let payload = encode_wedge_compound_blend();
    let mut tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut symbols = symbol_decoder(&payload);
    let before = symbols.checkpoint();

    let blend = read_compound_blend_syntax(
        &mut tile,
        &mut symbols,
        CompoundBlendToolConfig {
            masked_enabled: true,
            implicit_mask: true,
        },
        CompoundBlendInput {
            skip_mode: false,
            use_optflow: false,
            joint_amvd: false,
            switchable_refinemv_on: true,
            n4w: 2,
            n4h: 2,
            block_size_index: 3,
            comp_group_idx_ctx: 0,
        },
        TILE_OFFSET,
    )
    .unwrap();

    assert_eq!(blend, mc::CompoundBlend::average_with_implicit_mask(true));
    assert_eq!(symbols.checkpoint(), before);
}

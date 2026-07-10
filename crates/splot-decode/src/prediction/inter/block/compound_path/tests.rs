// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use splot_core::symbol::{CdfUpdateMode, Symbol, SymbolDecoderConfig};
use splot_core::symbol_encoder::{SymbolEncoder, SymbolEncoderConfig};

use super::*;
use crate::bitstream::tile_payload::FrameCdfSubset;

const TILE_OFFSET: ByteOffset = ByteOffset::new(0);

fn encode_wedge_compound_blend() -> Vec<u8> {
    let mut tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut encoder = SymbolEncoder::with_config(
        SymbolEncoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Enabled),
    );
    tile.with_row_mut(TileCdfSelector::CompGroupIdx { ctx: 0 }, |row| {
        encoder.write_symbol(row, Symbol::new(1))
    })
    .unwrap()
    .unwrap();
    tile.with_row_mut(TileCdfSelector::CompoundType, |row| {
        encoder.write_symbol(row, Symbol::new(0))
    })
    .unwrap()
    .unwrap();
    tile.with_row_mut(TileCdfSelector::WedgeQuad, |row| {
        encoder.write_symbol(row, Symbol::new(0))
    })
    .unwrap()
    .unwrap();
    tile.with_row_mut(TileCdfSelector::WedgeAngle { quad: 0 }, |row| {
        encoder.write_symbol(row, Symbol::new(0))
    })
    .unwrap()
    .unwrap();
    tile.with_row_mut(TileCdfSelector::WedgeDist2, |row| {
        encoder.write_symbol(row, Symbol::new(0))
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

fn compound_motion_contexts() -> (ModeContext, BlockNeighbourContext) {
    let (grid, block) = compound_motion_grid_and_block(Mv::ZERO, Mv::ZERO);
    (
        find_mode_ctx(&grid, &block),
        block_neighbour_ctx(&grid, &block),
    )
}

fn encode_compound_local_warp(ctx: usize, enabled: bool) -> Vec<u8> {
    let mut tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut encoder = SymbolEncoder::with_config(
        SymbolEncoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Enabled),
    );
    tile.with_row_mut(TileCdfSelector::UseLocalWarp { ctx }, |row| {
        encoder.write_symbol(row, Symbol::new(u8::from(enabled)))
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
fn compound_local_warp_derives_a_model_per_reference_list() {
    let (grid, block) =
        compound_motion_grid_and_block(Mv { row: 24, col: -8 }, Mv { row: -16, col: 40 });
    let models = compound_local_warp_models(
        &grid,
        &block,
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
fn compound_local_warp_rejects_a_signalled_list_without_samples() {
    let grid = NeighbourMvGrid::new(16, 16).unwrap();
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
    let error =
        compound_local_warp_models(&grid, &block, Mv::ZERO, Mv::ZERO, 0, 2, 2, 2, TILE_OFFSET)
            .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("inter.compound.local_warp.empty_sample_list")
    );
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
        skip_mode_default_pair(0, Some((RESTRICTED_ORDER_HINT, 1))),
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

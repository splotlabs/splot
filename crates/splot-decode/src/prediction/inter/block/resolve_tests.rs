// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used)]

use splot_core::headers::frame::GmType;

use super::super::super::find_mv_stack::NeighbourMvGrid;
use super::super::super::read_mv::{MV_PRECISION_ONE_PEL, MV_PRECISION_QUARTER_PEL};
use super::super::super::tests::parse_inter_core_for_validation;
use super::*;

const GLOBAL_ROTZOOM_FIXTURE: &[u8] = include_bytes!(
    "../../../../../../tests/conformance/vectors/valid/syn-2frame-global-rotzoom-64x64-q80.ivf"
);
const MI_DIM: usize = 32;

/// A frame core whose references 0 and 1 code a sub-sample `ROTZOOM` model.
fn rotzoom_core() -> FrameHeaderCore {
    let (_, mut core, _) = parse_inter_core_for_validation(GLOBAL_ROTZOOM_FIXTURE).unwrap();
    let tail = core.inter_tail.as_mut().expect("fixture has an inter tail");
    for (model, translation) in tail
        .global_motion
        .references
        .iter_mut()
        .zip([(3, -5), (-7, 9)])
    {
        model.gm_type = GmType::RotZoom;
        model.gm_params = [
            translation.0 << 14,
            translation.1 << 13,
            1 << 16,
            0,
            0,
            1 << 16,
        ];
    }
    core
}

/// The vectors `y_mode` resolves to with no neighbours, at a frame precision
/// of a quarter sample and a block precision of one sample, with the global
/// MVs of the block at both precisions.
fn resolve_without_neighbours(y_mode: CompoundYMode) -> ([Mv; 2], [Mv; 2], [Mv; 2]) {
    let core = rotzoom_core();
    let block = MvBlockContext {
        mi_row: 8,
        mi_col: 8,
        bw4: 4,
        bh4: 4,
        sb_h4: 16,
        ref_frame0: 0,
        ref_frame1: Some(1),
        mi_rows: MI_DIM,
        mi_cols: MI_DIM,
    };
    let compound = CompoundMotionSyntax {
        y_mode,
        ref_frame1: 1,
        skip_mode: false,
        use_optflow: false,
        local_warp: false,
        global_warp: [None, None],
        ref_mv_idx: 0,
        independent_idx: y_mode.has_second_drl().then_some([0, 0]),
        mvd: [Mv::ZERO; 2],
        joint: None,
        jmvd_scale_mode: 0,
        temporal_allowed: true,
        temporal_first: [false; 2],
        use_refinemv: false,
        refinemv_switchable: false,
    };
    let syntax = InterBlockSyntax {
        block_ctx: block,
        motion: InterMotionSyntax::Compound(compound),
        interp: ReconInterpolationFilter::EightTap,
        precision: BlockPrecisionRecord::explicit(MV_PRECISION_ONE_PEL),
        skip: false,
        use_amvd: false,
        tip_size_16x16: false,
        blend: mc::CompoundBlend::average_with_implicit_mask(false),
        bawp: BawpSyntax::default(),
        interintra: None,
        optflow_distances: None,
        residual: None,
    };
    let mut grid = NeighbourMvGrid::new_for_tile(0..MI_DIM, 0..MI_DIM).unwrap();
    let order_hints = [Some(0), Some(0)];
    let state = MvResolutionState {
        grid: &mut grid,
        ref_mv_bank: &mut None,
        warp_param_bank: &mut WarpParamBank::new(),
        core: &core,
        temporal: None,
        order_hints: OrderHintMvContext::new(1, &order_hints),
        drl_reorder: DrlReorder::Disabled,
        max_drl_bits_minus_1: 0,
        frame_precision: MV_PRECISION_QUARTER_PEL,
        tile_offset: ByteOffset::new(0),
    };
    let mv = compound_motion(&syntax, &compound, &state).unwrap().mv;
    let global = |precision| [0, 1].map(|r| global_motion_mv(&core, r, &block, precision));
    (
        mv,
        global(MV_PRECISION_QUARTER_PEL),
        global(MV_PRECISION_ONE_PEL),
    )
}

/// § 7.12.2.1: `GlobalMvs` take `FrameMvPrecision`, so the NEAR list of a
/// `NEAR_NEWMV` block that codes a coarser `MvPrecision` keeps the
/// quarter-sample global MV its second-DRL stack offers.
#[test]
fn near_newmv_near_list_takes_the_frame_precision_global_mv() {
    let (mv, frame_global, block_global) = resolve_without_neighbours(CompoundYMode::NearNew);

    assert_ne!(frame_global[0], block_global[0]);
    assert_eq!(mv[0], frame_global[0]);
}

/// § 7.12.2.20: the paired stack's extra-search candidate is the same
/// frame-precision pair, so `NEW_NEARMV` keeps it on its NEAR list.
#[test]
fn new_nearmv_near_list_takes_the_frame_precision_global_mv() {
    let (mv, frame_global, block_global) = resolve_without_neighbours(CompoundYMode::NewNear);

    assert_ne!(frame_global[1], block_global[1]);
    assert_eq!(mv[1], frame_global[1]);
}

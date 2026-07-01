// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::headers::frame::InterpolationFilter as FrameInterpolationFilter;
use splot_core::headers::frame::{FrameHeaderCore, FrameType, MvPrecision, TxMode};
use splot_core::headers::sequence::SequenceHeader;
use splot_core::span::ByteOffset;
use splot_core::symbol::SymbolDecoder;
use splot_core::tables::conversion::{
    MAX_TX_SIZE_RECT, TX_HEIGHT, TX_HEIGHT_LOG2, TX_WIDTH, TX_WIDTH_LOG2,
};
use splot_recon::PlaneId as ReconPlaneId;
use splot_recon::math::round2_signed;
use splot_recon::{
    BitDepth, CurrentFrameWorkspace, IDENTITY_WARP_PARAMS,
    InterpolationFilter as ReconInterpolationFilter, ReconSample,
};

use super::super::wienerns_lr::intrabc_records::{
    IntrabcBlockGeometry, IntrabcBlockPrelude, IntrabcInfo, IntrabcUseSkip,
    TileIntrabcPreludeState, derive_intrabc_luma_prediction_geometry, read_intrabc_info,
    read_intrabc_use_and_skip,
};
use super::super::wienerns_lr::tx_records::{
    CdefState, DeltaQState, SelectableLumaTxRecord, ccso::CcsoState,
    derive_inter_luma_tx_records_for_block,
};
use super::super::{
    DecodeOptions, DecodePlannedObu, DecodeStreamPlan, Result, effective_allow_screen_content_tools,
};
use super::compound::{CompoundParseInput, read_compound_average_syntax};
use super::find_mv_stack::{
    MvBlockContext, NeighbourMvGrid, NeighbourYMode, block_neighbour_ctx, find_mode_ctx,
    find_mv_stack,
};
use super::read_mv::{
    MV_PRECISION_EIGHTH_PEL, MV_PRECISION_HALF_PEL, MV_PRECISION_ONE_PEL, MV_PRECISION_QUARTER_PEL,
    MvReadConfig, apply_inter_mvd_signs, mv_clamp_to_integer, read_newmv_amvd_block_mvd,
    read_newmv_block_mvd_magnitude,
};
use super::{
    BawpSyntax, InterBlock, InterReferenceState, InterResidual, InterResidualBlock, Mv,
    PlacedInterBlock, SINGLE_MODE_GLOBALMV, SINGLE_MODE_NEARMV, SINGLE_MODE_NEWMV, SPEC_MODE_INFO,
    effective_quantizer_deltas_are_zero, mc, unsupported_at, unsupported_compound_at,
};
use crate::tile_payload::{
    ActiveChromaResidualPolicy, ActiveIntraIstResidualPolicy, BlockSize, CoeffContextReset,
    DecodeBlockFrontier, DecodeTileWorkUnit, FrameCdfSubset, GeneralIntraLeafMode,
    GeneralIntraMultiblockError, GeneralIntraTreeWalkError, IsCflContext, LumaCoeffBlock,
    TileBlockDecodedState, TileCdfSelector, TileCdfSubset, TileCoeffContextState, TileFscModeState,
    TileIntraJointModeState, TilePartitionTraversalError, TileUsesMrlsState,
    TransformToolResidualPolicy, chroma_subsampling, decode_general_intra_multiblock_tree,
    decode_general_intra_plane_coeffs, frame_mi_dimensions, get_plane_residual_size,
};

const INTERP_FILTER_CTX_NO_NEIGHBOUR_BASE: usize = 3;
const INTERP_FILTER_CTX_SECOND_REF_INTER_OFFSET: usize = 4;
const SINGLE_REF_FRAME0: i8 = 0;
const MI_SIZE: usize = 4;
const CHUNK_64_N4: usize = 16;
const BLOCK_64X64: usize = 12;
const MAX_WARP_REF_CANDIDATES: usize = 4;
const WARP_DELTA_NUM_SYMBOLS_LOW: u8 = 8;
const WARP_DELTA_NUM_SYMBOLS_HIGH: u8 = 8;
const WARPEDMODEL_PREC_BITS: u32 = 16;
const WARP_PARAM_REDUCE_BITS: u32 = 6;
const WARP_TRANS_INTEGER_BITS: u32 = 12;
const WARP_DELTA_STEP_BITS: u32 = 10;
const WARPEDMODEL_TRANS_CLAMP: i64 = 1 << (WARPEDMODEL_PREC_BITS + WARP_TRANS_INTEGER_BITS - 1);
#[doc = "AV2 § 9.2 `Size_Group[BLOCK_SIZES]`."]
const SIZE_GROUP_LOOKUP: [usize; 29] = [
    0, 0, 0, 1, 1, 1, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 0, 0, 1, 1, 2, 2, 1, 1, 2, 2,
];
#[doc = "AV2 § 9.2 `Wedge_Bits[BLOCK_SIZES] != 0`."]
const WEDGE_USED_BY_BSIZE: [bool; 29] = [
    false, false, false, true, true, true, true, true, true, true, true, true, true, false, false,
    false, false, false, false, false, false, true, true, true, true, false, false, true, true,
];
const INTERINTRA_MODES: u8 = 4;
const WEDGE_QUADS: u8 = 4;
const QUAD_WEDGE_ANGLES: u8 = 5;
const H_WEDGE_ANGLES: u8 = 10;
const COEFF_CONTEXT_PLANES: [(usize, u32); 3] = [(0, 0), (1, 1), (2, 1)];
const WEDGE_0: u8 = 0;
const WEDGE_90: u8 = 5;
const NUM_WEDGE_DIST: u8 = 4;

fn trace_inter_block_mode(mi_row: usize, mi_col: usize) -> bool {
    if std::env::var_os("SPLOT_TRACE_INTER_BLOCK_MODE").is_none() {
        return false;
    }
    let Some(window) = std::env::var("SPLOT_TRACE_INTER_BLOCK_WINDOW").ok() else {
        return (mi_row == 0 && mi_col <= 256)
            || ((8..=12).contains(&mi_row) && (144..=156).contains(&mi_col))
            || (mi_row == 16 && mi_col == 128);
    };
    let Some((rows, cols)) = window.split_once(',') else {
        return false;
    };
    let Some((row_start, row_end)) = rows.split_once(':') else {
        return false;
    };
    let Some((col_start, col_end)) = cols.split_once(':') else {
        return false;
    };
    let Ok(row_start) = row_start.parse::<usize>() else {
        return false;
    };
    let Ok(row_end) = row_end.parse::<usize>() else {
        return false;
    };
    let Ok(col_start) = col_start.parse::<usize>() else {
        return false;
    };
    let Ok(col_end) = col_end.parse::<usize>() else {
        return false;
    };
    (row_start..=row_end).contains(&mi_row) && (col_start..=col_end).contains(&mi_col)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WarpInterMode {
    Warpmv,
    WarpNewmv,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn decode_inter_blocks<T: ReconSample>(
    plan: &DecodeStreamPlan,
    candidate: &DecodePlannedObu,
    bytes: &[u8],
    frame_envelope: splot_core::annexb::ObuEnvelope<'_>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    options: &DecodeOptions,
    frame_interpolation_filter: FrameInterpolationFilter,
    num_total_refs: usize,
    reference_select: bool,
    compound_is_joint_ctx: Option<usize>,
    num_same_ref_compound: u8,
    ref_frame_idx: &[u32],
    reference: &InterReferenceState<'_, T>,
    workspace: &mut CurrentFrameWorkspace<T>,
    _qindex: u32,
    luma_use_tcq: bool,
    residual_use_ddt: bool,
    bit_depth: BitDepth,
    initial_cdfs: FrameCdfSubset,
) -> Result<FrameCdfSubset> {
    let offset = frame_envelope.offset;
    let mut tile_plan = super::super::derive_inter_tile_plan(
        plan,
        candidate,
        bytes,
        frame_envelope,
        sequence,
        core,
        options,
        initial_cdfs,
    )?;
    let [tile] = tile_plan.work_units_mut() else {
        return Err(inter_cap!(
            "inter_unexpected_tile_work_units",
            offset,
            "inter.tile_count != 1",
            SPEC_MODE_INFO
        ));
    };
    let tile_offset = tile.tile_byte_span().start;

    let max_drl_bits_minus_1 = core
        .inter
        .as_ref()
        .and_then(|inter| inter.max_drl_bits_minus_1)
        .ok_or_else(|| {
            inter_missing!(
                "inter_missing_max_drl_bits",
                offset,
                "inter.max_drl_bits_minus_1",
                SPEC_MODE_INFO
            )
        })?;

    let (mi_rows, mi_cols) = frame_mi_dimensions(core).map_err(|_| {
        inter_missing!(
            "inter_mi_dimensions",
            offset,
            "inter.mi_dimensions",
            SPEC_MODE_INFO
        )
    })?;
    let mut coeff_ctx = TileCoeffContextState::new(mi_rows, mi_cols).map_err(|_| {
        inter_cap!(
            "inter_coeff_context_state",
            offset,
            "inter.residual_context_state",
            SPEC_MODE_INFO
        )
    })?;
    let mut cdef_state = CdefState::new(mi_rows, mi_cols, sequence, tile_offset)?;
    let mut ccso_state = CcsoState::new(mi_rows, mi_cols, sequence, core, tile_offset)?;
    let mut delta_q_state = DeltaQState::new(sequence, core, tile_offset)?;
    let mut intrabc_state = TileIntrabcPreludeState::new(mi_rows, mi_cols, sequence, tile_offset)?;

    let mut mv_grid = NeighbourMvGrid::new(mi_rows, mi_cols)
        .ok_or_else(|| inter_cap!("inter_mv_grid", offset, "inter.mv_grid", SPEC_MODE_INFO))?;
    let sb_h4 = superblock_h4(sequence, core).ok_or_else(|| {
        inter_missing!(
            "inter_sb_size",
            offset,
            "inter.superblock_size",
            SPEC_MODE_INFO
        )
    })?;

    let residual_tool_policy = transform_tool_residual_policy(sequence);
    let residual_quantizer_deltas_are_zero = core
        .quantization_params
        .as_ref()
        .is_some_and(|quant| effective_quantizer_deltas_are_zero(sequence, quant));
    let enable_adaptive_mvd = sequence
        .inter
        .as_ref()
        .is_some_and(|inter| inter.enable_adaptive_mvd);
    let allow_bawp = core.inter_tail.as_ref().is_some_and(|tail| tail.allow_bawp);
    let allow_warpmv_mode = core
        .inter_tail
        .as_ref()
        .is_some_and(|tail| tail.allow_warpmv_mode);
    let frame_is_switch = core.frame_type == Some(FrameType::Switch);

    let mut deblock_blocks: Vec<super::super::deblock::DeblockBlock> = Vec::new();
    let mut decoded_any = false;
    let limits = options.limits();
    trace_symbol_frame_marker(offset);
    let symbols = decode_general_intra_multiblock_tree(
        tile,
        sequence,
        core,
        limits,
        |work_unit,
         symbols,
         frontier,
         joint_modes,
         uses_mrls,
         fsc_modes,
         palette_state,
         is_cfl_ctx,
         block_decoded| {
            let leaf = decode_one_inter_or_intra_block(
                work_unit,
                symbols,
                frontier,
                sequence,
                core,
                &mut coeff_ctx,
                &mut cdef_state,
                &mut ccso_state,
                &mut delta_q_state,
                &mut intrabc_state,
                &mut mv_grid,
                sb_h4,
                mi_rows,
                mi_cols,
                max_drl_bits_minus_1,
                frame_interpolation_filter,
                residual_tool_policy,
                residual_quantizer_deltas_are_zero,
                num_total_refs,
                reference_select,
                compound_is_joint_ctx,
                num_same_ref_compound,
                joint_modes,
                uses_mrls,
                fsc_modes,
                palette_state,
                is_cfl_ctx,
                block_decoded,
                workspace,
                &mut deblock_blocks,
                luma_use_tcq,
                residual_use_ddt,
                ref_frame_idx,
                reference,
                bit_depth,
                enable_adaptive_mvd,
                allow_bawp,
                allow_warpmv_mode,
                frame_is_switch,
                core.order_hint_lsb.unwrap_or(0),
                tile_offset,
            )?;
            decoded_any = true;
            Ok(leaf)
        },
    )
    .map_err(|error| map_inter_multiblock_error(error, tile_offset))?;

    symbols.exit_symbol().map_err(|_| {
        if reference_select {
            compound_cap!(
                "compound_exit_symbol",
                tile_offset,
                "inter.compound.exit_symbol",
                SPEC_MODE_INFO
            )
        } else {
            inter_cap!(
                "inter_exit_symbol",
                tile_offset,
                "inter.exit_symbol",
                SPEC_MODE_INFO
            )
        }
    })?;

    if !decoded_any {
        return Err(inter_missing!(
            "inter_no_decoded_block",
            tile_offset,
            "inter.block",
            SPEC_MODE_INFO
        ));
    }
    tile.apply_frame_end_cdf_update();
    Ok(tile.frame_cdfs())
}

fn trace_symbol_frame_marker(offset: ByteOffset) {
    use std::io::Write as _;

    let Some(path) = std::env::var_os("SPLOT_SYMBOL_TRACE") else {
        return;
    };
    if let Ok(mut file) = std::fs::OpenOptions::new().append(true).open(path) {
        let _ = writeln!(file, "# inter frame offset={}", offset.get());
    }
}

fn superblock_h4(sequence: &SequenceHeader, core: &FrameHeaderCore) -> Option<usize> {
    let partition = sequence.partition?;
    core.frame_is_intra?;
    match partition.seq_sb_size() {
        splot_core::headers::sequence::SuperblockSize::Block64x64 => Some(16),
        splot_core::headers::sequence::SuperblockSize::Block128x128 => Some(32),
        splot_core::headers::sequence::SuperblockSize::Block256x256 => Some(64),
    }
}

#[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
fn decode_one_inter_or_intra_block<T: ReconSample>(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    frontier: &DecodeBlockFrontier,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    coeff_ctx: &mut TileCoeffContextState,
    cdef_state: &mut CdefState,
    ccso_state: &mut CcsoState,
    delta_q_state: &mut DeltaQState,
    intrabc_state: &mut TileIntrabcPreludeState,
    mv_grid: &mut NeighbourMvGrid,
    sb_h4: usize,
    mi_rows: usize,
    mi_cols: usize,
    max_drl_bits_minus_1: u32,
    frame_interpolation_filter: FrameInterpolationFilter,
    residual_tool_policy: TransformToolResidualPolicy,
    residual_quantizer_deltas_are_zero: bool,
    num_total_refs: usize,
    reference_select: bool,
    compound_is_joint_ctx: Option<usize>,
    num_same_ref_compound: u8,
    joint_modes: &TileIntraJointModeState,
    uses_mrls: &TileUsesMrlsState,
    fsc_modes: &TileFscModeState,
    palette_state: &crate::tile_payload::TileLumaPaletteState,
    is_cfl_ctx: IsCflContext,
    block_decoded: &TileBlockDecodedState,
    workspace: &mut CurrentFrameWorkspace<T>,
    deblock_blocks: &mut Vec<super::super::deblock::DeblockBlock>,
    luma_use_tcq: bool,
    residual_use_ddt: bool,
    ref_frame_idx: &[u32],
    reference: &InterReferenceState<'_, T>,
    bit_depth: BitDepth,
    enable_adaptive_mvd: bool,
    allow_bawp: bool,
    allow_warpmv_mode: bool,
    frame_is_switch: bool,
    current_order_hint: u32,
    tile_offset: ByteOffset,
) -> Result<GeneralIntraLeafMode> {
    let n4w = frontier.b_size.num_4x4_wide().map_err(|_| {
        inter_diag!(
            "inter_block_geometry",
            tile_offset,
            "minimal inter block geometry lookup failed",
            SPEC_MODE_INFO
        )
    })?;
    let n4h = frontier.b_size.num_4x4_high().map_err(|_| {
        inter_diag!(
            "inter_block_geometry",
            tile_offset,
            "minimal inter block geometry lookup failed",
            SPEC_MODE_INFO
        )
    })?;
    let mi_row = frontier.r;
    let mi_col = frontier.c;
    let entry_checkpoint = symbols.checkpoint();
    let trace_first_row = trace_inter_block_mode(mi_row, mi_col);
    if trace_first_row {
        eprintln!(
            "inter block entry r={mi_row} c={mi_col} b={} n4={}x{} tree_luma={} tree_chroma={} has_chroma={} checkpoint={entry_checkpoint:?}",
            frontier.b_size.index(),
            n4w,
            n4h,
            frontier.is_luma_part(),
            frontier.is_chroma_part(),
            frontier.has_chroma,
        );
    }
    let placed_block = |block| PlacedInterBlock {
        luma_x: mi_col * 4,
        luma_y: mi_row * 4,
        luma_w: n4w * 4,
        luma_h: n4h * 4,
        block,
    };

    let mut block_ctx = MvBlockContext {
        mi_row,
        mi_col,
        bw4: n4w,
        bh4: n4h,
        sb_h4,
        ref_frame0: SINGLE_REF_FRAME0,
        mi_rows,
        mi_cols,
    };

    let neighbour_ctx = block_neighbour_ctx(mv_grid, &block_ctx);
    let mode_ctx = find_mode_ctx(mv_grid, &block_ctx);

    let is_inter = if frontier.is_luma_part() || frontier.is_chroma_part() {
        0
    } else if frontier.shared_mixed_chroma_ref_forces_inter() {
        1
    } else {
        let cdfs = work_unit.cdf_mut().tile_cdfs_mut();
        cdfs.read_block_symbol_trace(
            TileCdfSelector::IsInter {
                ctx: neighbour_ctx.is_inter_ctx,
            },
            symbols,
        )
        .map_err(|_| symbol_read_error(tile_offset))?
        .get()
    };
    if trace_first_row {
        eprintln!(
            "inter block is_inter r={mi_row} c={mi_col} value={is_inter} ctx={} checkpoint={:?}",
            neighbour_ctx.is_inter_ctx,
            symbols.checkpoint()
        );
    }
    if is_inter == 0 {
        let mut prelude = IntrabcBlockPrelude::from_use_skip(
            IntrabcUseSkip {
                use_intrabc: false,
                skip_flag: false,
            },
            None,
        );
        if !frontier.is_chroma_part() {
            intrabc_state.prepare_for_block(frontier.r, frontier.c);
            let use_skip = read_intrabc_use_and_skip(
                work_unit.cdf_mut().tile_cdfs_mut(),
                symbols,
                intrabc_state,
                core,
                IntrabcBlockGeometry::from_frontier(frontier, n4w, n4h),
                tile_offset,
            )?;
            if trace_first_row {
                eprintln!(
                    "inter block intra-prelude r={mi_row} c={mi_col} use_intrabc={} skip={} checkpoint={:?}",
                    use_skip.use_intrabc,
                    use_skip.skip_flag,
                    symbols.checkpoint()
                );
            }
            cdef_state.read_for_block(
                work_unit,
                symbols,
                core,
                frontier,
                n4w,
                n4h,
                use_skip.skip_flag,
                tile_offset,
            )?;
            ccso_state.read_for_block(work_unit, symbols, frontier, tile_offset)?;
            delta_q_state.read_for_block(work_unit, symbols, frontier, tile_offset)?;
            let intrabc = if use_skip.use_intrabc {
                Some(read_intrabc_info(
                    work_unit.cdf_mut().tile_cdfs_mut(),
                    symbols,
                    intrabc_state,
                    sequence,
                    core,
                    IntrabcBlockGeometry::from_frontier(frontier, n4w, n4h),
                    use_skip.skip_flag,
                    None,
                    tile_offset,
                )?)
            } else {
                None
            };
            prelude = IntrabcBlockPrelude::from_use_skip(use_skip, intrabc);
        }
        if prelude.use_intrabc {
            let info = prelude.intrabc.ok_or_else(|| {
                inter_missing!(
                    "inter_intrabc_info",
                    tile_offset,
                    "inter.intrabc.info",
                    SPEC_MODE_INFO
                )
            })?;
            reconstruct_intrabc_luma_predictor(
                workspace,
                core,
                frontier,
                n4w,
                n4h,
                info,
                tile_offset,
            )?;
            let block_qindex = delta_q_state.qindex_u32();
            let residual = if prelude.skip_flag {
                reset_inter_skip_coeff_contexts(coeff_ctx, frontier, n4w, n4h, tile_offset)?;
                None
            } else {
                Some(read_inter_residual(
                    work_unit,
                    symbols,
                    coeff_ctx,
                    sequence,
                    core,
                    frontier,
                    n4w,
                    n4h,
                    mi_rows,
                    mi_cols,
                    residual_tool_policy,
                    tile_offset,
                )?)
            };
            if let Some(residual) = residual.as_ref() {
                super::add_inter_residual_to_workspace(
                    workspace,
                    residual,
                    block_qindex,
                    luma_use_tcq,
                    residual_use_ddt,
                    bit_depth,
                    tile_offset,
                )?;
            }
            mv_grid.record_block(
                mi_row,
                mi_col,
                n4w,
                n4h,
                true,
                -1,
                None,
                NeighbourYMode::Other,
                Mv::ZERO,
                prelude.skip_flag,
                interp_filter_no_neighbour_ctx(false) as u8,
                false,
            );
            intrabc_state.record_block(frontier.r, frontier.c, n4w, n4h, prelude, tile_offset)?;
            trace_leaf_exit("intrabc", frontier, entry_checkpoint, symbols.checkpoint());
            return Ok(non_intra_leaf_mode(frontier));
        }
        let block_qindex = delta_q_state.qindex_u32();
        let leaf = super::super::general_intra::decode_one_general_intra_block::<T>(
            work_unit,
            symbols,
            frontier,
            sequence,
            core,
            joint_modes,
            uses_mrls,
            fsc_modes,
            palette_state,
            is_cfl_ctx,
            block_decoded,
            workspace,
            coeff_ctx,
            deblock_blocks,
            block_qindex,
            luma_use_tcq,
            residual_tool_policy,
            mi_cols,
            mi_rows,
            bit_depth,
            tile_offset,
        )?;
        if !frontier.is_chroma_part() {
            mv_grid.record_block(
                mi_row,
                mi_col,
                n4w,
                n4h,
                false,
                -1,
                None,
                NeighbourYMode::Other,
                Mv::ZERO,
                false,
                interp_filter_no_neighbour_ctx(false) as u8,
                false,
            );
            intrabc_state.record_block(frontier.r, frontier.c, n4w, n4h, prelude, tile_offset)?;
        }
        trace_leaf_exit("intra", frontier, entry_checkpoint, symbols.checkpoint());
        return Ok(leaf);
    }
    if is_inter != 1 {
        return Err(inter_cap!(
            "inter_block_is_intra",
            tile_offset,
            "inter.block.is_inter out of range",
            SPEC_MODE_INFO
        ));
    }

    let skip = {
        let cdfs = work_unit.cdf_mut().tile_cdfs_mut();
        cdfs.read_block_symbol_trace(
            TileCdfSelector::Skip {
                ctx: neighbour_ctx.skip_ctx,
            },
            symbols,
        )
        .map_err(|_| symbol_read_error(tile_offset))?
    };
    let skip = skip.get();
    if trace_first_row {
        eprintln!(
            "inter block skip r={mi_row} c={mi_col} value={skip} ctx={} checkpoint={:?}",
            neighbour_ctx.skip_ctx,
            symbols.checkpoint()
        );
    }
    if skip != 0 && skip != 1 {
        return Err(inter_cap!(
            "inter_block_unexpected_skip",
            tile_offset,
            "inter.block.skip out of range",
            SPEC_MODE_INFO
        ));
    }

    cdef_state.read_for_block(
        work_unit,
        symbols,
        core,
        frontier,
        n4w,
        n4h,
        skip == 1,
        tile_offset,
    )?;
    ccso_state.read_for_block(work_unit, symbols, frontier, tile_offset)?;
    delta_q_state.read_for_block(work_unit, symbols, frontier, tile_offset)?;
    let block_qindex = delta_q_state.qindex_u32();
    if trace_first_row {
        eprintln!(
            "inter block after-side-info r={mi_row} c={mi_col} checkpoint={:?}",
            symbols.checkpoint()
        );
    }

    let cdfs = work_unit.cdf_mut().tile_cdfs_mut();
    let uses_compound = if reference_select {
        read_block_reference_mode(
            cdfs,
            symbols,
            &neighbour_ctx,
            ref_frame_idx,
            &reference.ref_order_hint,
            current_order_hint,
            tile_offset,
        )?
    } else {
        false
    };
    if trace_first_row {
        eprintln!(
            "inter block reference-mode r={mi_row} c={mi_col} uses_compound={uses_compound} reference_select={reference_select} checkpoint={:?}",
            symbols.checkpoint()
        );
    }

    if uses_compound {
        let is_joint_ctx = compound_is_joint_ctx.ok_or_else(|| {
            compound_missing!(
                "compound_missing_is_joint_context",
                tile_offset,
                "inter.compound.is_joint_context",
                SPEC_MODE_INFO
            )
        })?;
        let compound = read_compound_average_syntax(
            cdfs,
            symbols,
            CompoundParseInput {
                num_total_refs,
                num_same_ref_compound,
                has_neighbour: neighbour_ctx.has_neighbour,
                new_mv_context: mode_ctx.new_mv_context,
                is_joint_ctx,
                skip,
                n4w,
                n4h,
                mi_row,
                mi_col,
                mi_rows,
                mi_cols,
            },
            tile_offset,
        )?;
        let ref_mv_idx0 = read_drl_idx(
            cdfs,
            symbols,
            mode_ctx.new_mv_context,
            max_drl_bits_minus_1,
            tile_offset,
        )?;
        let ref_mv_idx1 = read_drl_idx(
            cdfs,
            symbols,
            mode_ctx.new_mv_context,
            max_drl_bits_minus_1,
            tile_offset,
        )?;
        if ref_mv_idx0 != 0 || ref_mv_idx1 != 0 {
            return Err(compound_cap!(
                "compound_block_drl_idx",
                tile_offset,
                "inter.compound.drl_idx != 0",
                SPEC_MODE_INFO
            ));
        }
        let interp_ctx = neighbour_ctx.interp_filter_ctx(compound.ref_frame0, true);
        trace_interp_filter_context(
            "compound",
            mi_row,
            mi_col,
            compound.ref_frame0,
            true,
            interp_ctx,
            &neighbour_ctx,
            symbols,
        );
        let interp = resolve_interp_filter(
            cdfs,
            symbols,
            frame_interpolation_filter,
            SINGLE_MODE_NEARMV,
            interp_ctx,
            tile_offset,
        )?;
        mv_grid.record_block(
            mi_row,
            mi_col,
            n4w,
            n4h,
            true,
            compound.ref_frame0,
            Some(compound.ref_frame1),
            NeighbourYMode::Other,
            compound.mv0,
            skip == 1,
            interp_filter_symbol(interp),
            false,
        );
        reset_inter_skip_coeff_contexts(coeff_ctx, frontier, n4w, n4h, tile_offset)?;
        let placed = placed_block(InterBlock {
            ref_frame0: compound.ref_frame0,
            ref_frame1: Some(compound.ref_frame1),
            mv: compound.mv0,
            mv1: compound.mv1,
            interp,
            warp_params: None,
            bawp: BawpSyntax::default(),
            residual: None,
        });
        reconstruct_placed_inter_block(
            workspace,
            &placed,
            ref_frame_idx,
            reference,
            block_qindex,
            luma_use_tcq,
            residual_use_ddt,
            bit_depth,
            tile_offset,
        )?;
        intrabc_state.record_block(
            frontier.r,
            frontier.c,
            n4w,
            n4h,
            IntrabcBlockPrelude::from_use_skip(
                IntrabcUseSkip {
                    use_intrabc: false,
                    skip_flag: skip == 1,
                },
                None,
            ),
            tile_offset,
        )?;
        trace_leaf_exit("compound", frontier, entry_checkpoint, symbols.checkpoint());
        return Ok(non_intra_leaf_mode(frontier));
    }

    let ref_frame0: i8 = if num_total_refs >= 2 {
        if neighbour_ctx.has_neighbour {
            return Err(inter_cap!(
                "inter_block_single_ref_with_neighbour",
                tile_offset,
                "inter.single_ref.neighbour_context",
                SPEC_MODE_INFO
            ));
        }
        let ctx = neighbour_ctx
            .single_ref_ctx(0, num_total_refs)
            .ok_or_else(|| {
                inter_missing!(
                    "inter_block_single_ref_ctx",
                    tile_offset,
                    "inter.single_ref.context",
                    SPEC_MODE_INFO
                )
            })?;
        let contexts = [ctx];
        let selected = super::single_ref::read_single_ref(cdfs, symbols, num_total_refs, &contexts)
            .map_err(|_| {
                inter_missing!(
                    "inter_block_single_ref_read",
                    tile_offset,
                    "inter.single_ref.symbol",
                    SPEC_MODE_INFO
                )
            })?;
        i8::try_from(selected).map_err(|_| {
            inter_cap!(
                "inter_block_single_ref_value",
                tile_offset,
                "inter.single_ref.selection out of range",
                SPEC_MODE_INFO
            )
        })?
    } else {
        SINGLE_REF_FRAME0
    };
    block_ctx.ref_frame0 = ref_frame0;
    if trace_first_row {
        eprintln!(
            "inter block ref r={mi_row} c={mi_col} ref_frame0={ref_frame0} checkpoint={:?}",
            symbols.checkpoint()
        );
    }

    let force_integer_mv = effective_force_integer_mv(core);
    let warp_mode = read_warp_inter_mode_syntax(
        cdfs,
        symbols,
        allow_warpmv_mode,
        force_integer_mv,
        n4w,
        n4h,
        mode_ctx.warp_mv_count,
        tile_offset,
    )?;
    if trace_first_row {
        eprintln!(
            "inter block warp-mode r={mi_row} c={mi_col} value={warp_mode:?} allow={} force_integer_mv={} ctx={} checkpoint={:?}",
            allow_warpmv_mode,
            force_integer_mv,
            mode_ctx.warp_mv_count,
            symbols.checkpoint()
        );
    }
    if let Some(warp_mode) = warp_mode {
        let stack = find_mv_stack(mv_grid, &block_ctx, Mv::ZERO);
        if std::env::var_os("SPLOT_TRACE_INTER_BLOCK_MODE").is_some() {
            eprintln!(
                "inter block warp-selected r={mi_row} c={mi_col} b={} n4={}x{} mode={warp_mode:?} stack0={:?} checkpoint={:?}",
                frontier.b_size.index(),
                n4w,
                n4h,
                stack.candidate(0),
                symbols.checkpoint()
            );
        }
        let mv_config = inter_mv_read_config(core, tile_offset)?;
        let warp = match warp_mode {
            WarpInterMode::WarpNewmv => read_warp_newmv_delta_syntax(
                cdfs,
                symbols,
                sequence,
                mv_config,
                frontier.b_size.index(),
                mi_row,
                mi_col,
                n4w,
                n4h,
                &stack,
                mode_ctx.new_mv_context,
                max_drl_bits_minus_1,
                tile_offset,
            )?,
            WarpInterMode::Warpmv => read_warpmv_delta_syntax(
                cdfs,
                symbols,
                mv_config,
                frontier.b_size.index(),
                mi_row,
                mi_col,
                n4w,
                n4h,
                &stack,
                tile_offset,
            )?,
        };
        let warp_inter_intra = read_warp_inter_intra_syntax(
            cdfs,
            symbols,
            frontier.b_size.index(),
            n4w,
            n4h,
            tile_offset,
        )?;
        if warp_inter_intra.enabled {
            return Err(inter_cap!(
                "inter_warp_interintra_unimplemented",
                tile_offset,
                "inter.warp_inter_intra prediction",
                "7.13.3"
            ));
        }
        let residual = if skip == 0 {
            if !residual_quantizer_deltas_are_zero {
                return Err(inter_cap!(
                    "inter_block_residual_quantizer_delta",
                    tile_offset,
                    "inter.residual.nonzero_quantizer_delta",
                    SPEC_MODE_INFO
                ));
            }
            if !inter_residual_geometry_supported(frontier) {
                return Err(inter_cap!(
                    "inter_block_chroma_partitioned_residual",
                    tile_offset,
                    "inter.residual.chroma_partition_geometry",
                    SPEC_MODE_INFO
                ));
            }
            Some(read_inter_residual(
                work_unit,
                symbols,
                coeff_ctx,
                sequence,
                core,
                frontier,
                n4w,
                n4h,
                mi_rows,
                mi_cols,
                residual_tool_policy,
                tile_offset,
            )?)
        } else {
            reset_inter_skip_coeff_contexts(coeff_ctx, frontier, n4w, n4h, tile_offset)?;
            None
        };
        if std::env::var_os("SPLOT_TRACE_INTER_BLOCK_MODE").is_some() {
            eprintln!(
                "inter block warp r={mi_row} c={mi_col} mode={warp_mode:?} ref={ref_frame0} ref_mv_idx={} ref_warp_idx={} precision={} warpmv_with_mvd={} mv=({}, {}) params={:?} warp_inter_intra={:?} residual_blocks={} checkpoint={:?}",
                warp.ref_mv_idx,
                warp.ref_warp_idx,
                warp.precision_idx,
                warp.warpmv_with_mvd,
                warp.mv.row,
                warp.mv.col,
                warp.warp_params,
                warp_inter_intra,
                residual
                    .as_ref()
                    .map_or(0, |residual| residual.blocks.len()),
                symbols.checkpoint()
            );
        }
        mv_grid.record_warp_block(
            mi_row,
            mi_col,
            n4w,
            n4h,
            ref_frame0,
            if warp_mode == WarpInterMode::WarpNewmv {
                NeighbourYMode::NewMv
            } else {
                NeighbourYMode::Other
            },
            warp.mv,
            skip == 1,
            interp_filter_symbol(ReconInterpolationFilter::EightTap),
            false,
        );
        intrabc_state.record_block(
            frontier.r,
            frontier.c,
            n4w,
            n4h,
            IntrabcBlockPrelude::from_use_skip(
                IntrabcUseSkip {
                    use_intrabc: false,
                    skip_flag: skip == 1,
                },
                None,
            ),
            tile_offset,
        )?;

        let placed = placed_block(InterBlock {
            ref_frame0,
            ref_frame1: None,
            mv: warp.mv,
            mv1: Mv::ZERO,
            interp: ReconInterpolationFilter::EightTap,
            warp_params: Some(warp.warp_params),
            bawp: BawpSyntax::default(),
            residual,
        });
        reconstruct_placed_inter_block(
            workspace,
            &placed,
            ref_frame_idx,
            reference,
            block_qindex,
            luma_use_tcq,
            residual_use_ddt,
            bit_depth,
            tile_offset,
        )?;
        trace_leaf_exit("warp", frontier, entry_checkpoint, symbols.checkpoint());
        return Ok(non_intra_leaf_mode(frontier));
    }

    let single_mode = cdfs
        .read_block_symbol_trace(
            TileCdfSelector::SingleMode {
                ctx: mode_ctx.new_mv_context,
            },
            symbols,
        )
        .map_err(|_| symbol_read_error(tile_offset))?;
    let single_mode = single_mode.get();
    if trace_first_row {
        eprintln!(
            "inter block single-mode r={mi_row} c={mi_col} value={single_mode} ctx={} checkpoint={:?}",
            mode_ctx.new_mv_context,
            symbols.checkpoint()
        );
    }
    if single_mode != SINGLE_MODE_NEARMV
        && single_mode != SINGLE_MODE_GLOBALMV
        && single_mode != SINGLE_MODE_NEWMV
    {
        return Err(inter_cap!(
            "inter_block_unsupported_single_mode",
            tile_offset,
            "inter.single_mode not in {NEARMV, GLOBALMV, NEWMV}",
            SPEC_MODE_INFO
        ));
    }
    let use_amvd = read_use_amvd_syntax(
        cdfs,
        symbols,
        enable_adaptive_mvd,
        single_mode,
        neighbour_ctx.amvd_ctx(ref_frame0),
        tile_offset,
    )?;
    if trace_first_row {
        eprintln!(
            "inter block use-amvd r={mi_row} c={mi_col} value={use_amvd} enable={} checkpoint={:?}",
            enable_adaptive_mvd,
            symbols.checkpoint()
        );
    }
    let bawp = read_bawp_syntax(
        cdfs,
        symbols,
        BawpParseInput {
            allow_bawp,
            frame_is_switch,
            single_mode,
            use_amvd,
            n4w,
            n4h,
            has_chroma: frontier.has_chroma,
        },
        tile_offset,
    )?;
    if bawp != BawpSyntax::default() {
        return Err(inter_cap!(
            "inter_bawp_prediction_unimplemented",
            tile_offset,
            "inter.bawp_prediction",
            "7.13.3.1"
        ));
    }
    if trace_first_row {
        eprintln!(
            "inter block bawp r={mi_row} c={mi_col} luma={} chroma={} allow={} frame_switch={} checkpoint={:?}",
            bawp.luma_flag,
            bawp.chroma_flag,
            allow_bawp,
            frame_is_switch,
            symbols.checkpoint()
        );
    }
    let stack = find_mv_stack(mv_grid, &block_ctx, Mv::ZERO);

    let ref_mv_idx = if single_mode == SINGLE_MODE_NEARMV || single_mode == SINGLE_MODE_NEWMV {
        read_drl_idx(
            cdfs,
            symbols,
            mode_ctx.new_mv_context,
            max_drl_bits_minus_1,
            tile_offset,
        )?
    } else {
        0
    };
    if trace_first_row {
        eprintln!(
            "inter block drl r={mi_row} c={mi_col} ref_mv_idx={ref_mv_idx} max_drl_bits_minus_1={max_drl_bits_minus_1} checkpoint={:?}",
            symbols.checkpoint()
        );
    }

    let pred_mv = stack.candidate(ref_mv_idx);
    let mv = match single_mode {
        SINGLE_MODE_GLOBALMV => Mv::ZERO,
        SINGLE_MODE_NEARMV => pred_mv,
        _ => {
            let config = inter_mv_read_config(core, tile_offset)?;
            let diff = if use_amvd {
                let magnitude = read_newmv_amvd_block_mvd(cdfs, symbols, tile_offset)?;
                apply_inter_mvd_signs(magnitude, symbols, tile_offset, config, false, 1)?
            } else {
                let magnitude = read_newmv_block_mvd_magnitude(cdfs, symbols, tile_offset, config)?;
                apply_inter_mvd_signs(
                    magnitude,
                    symbols,
                    tile_offset,
                    config,
                    inter_mvd_sign_derivation_allowed(
                        sequence,
                        core,
                        single_mode,
                        use_amvd,
                        config,
                    ),
                    1,
                )?
            };
            Mv {
                row: mv_clamp_to_integer(pred_mv.row + diff.row),
                col: mv_clamp_to_integer(pred_mv.col + diff.col),
            }
        }
    };

    let interp_ctx = neighbour_ctx.interp_filter_ctx(ref_frame0, false);
    trace_interp_filter_context(
        "single",
        mi_row,
        mi_col,
        ref_frame0,
        false,
        interp_ctx,
        &neighbour_ctx,
        symbols,
    );
    let interp = resolve_interp_filter(
        cdfs,
        symbols,
        frame_interpolation_filter,
        single_mode,
        interp_ctx,
        tile_offset,
    )?;
    if trace_first_row {
        eprintln!(
            "inter block interp r={mi_row} c={mi_col} frame_filter={frame_interpolation_filter:?} filter={interp:?} ctx={} checkpoint={:?}",
            interp_ctx,
            symbols.checkpoint()
        );
    }

    let residual = if skip == 0 {
        if !residual_quantizer_deltas_are_zero {
            return Err(inter_cap!(
                "inter_block_residual_quantizer_delta",
                tile_offset,
                "inter.residual.nonzero_quantizer_delta",
                SPEC_MODE_INFO
            ));
        }
        if !inter_residual_geometry_supported(frontier) {
            if std::env::var_os("SPLOT_TRACE_INTER_RESIDUAL_GUARD").is_some() {
                let chroma_ref = frontier.chroma_ref_geometry();
                eprintln!(
                    "inter residual geometry rejected r={} c={} b={} n4={}x{} has_chroma={} chroma_offset={} luma_part={} chroma_part={} chroma_ref=({}, {}, {})",
                    frontier.r,
                    frontier.c,
                    frontier.b_size.index(),
                    n4w,
                    n4h,
                    frontier.has_chroma,
                    frontier.chroma_offset,
                    frontier.is_luma_part(),
                    frontier.is_chroma_part(),
                    chroma_ref.row(),
                    chroma_ref.col(),
                    chroma_ref.size().index()
                );
            }
            return Err(inter_cap!(
                "inter_block_chroma_partitioned_residual",
                tile_offset,
                "inter.residual.chroma_partition_geometry",
                SPEC_MODE_INFO
            ));
        }
        Some(read_inter_residual(
            work_unit,
            symbols,
            coeff_ctx,
            sequence,
            core,
            frontier,
            n4w,
            n4h,
            mi_rows,
            mi_cols,
            residual_tool_policy,
            tile_offset,
        )?)
    } else {
        reset_inter_skip_coeff_contexts(coeff_ctx, frontier, n4w, n4h, tile_offset)?;
        None
    };
    if trace_first_row {
        eprintln!(
            "inter block r={mi_row} c={mi_col} b={} skip={skip} ref={ref_frame0} mode={single_mode} use_amvd={use_amvd} mv=({}, {}) residual_blocks={} checkpoint={:?}",
            frontier.b_size.index(),
            mv.row,
            mv.col,
            residual
                .as_ref()
                .map_or(0, |residual| residual.blocks.len()),
            symbols.checkpoint()
        );
    }

    let y_mode = if single_mode == SINGLE_MODE_NEWMV {
        NeighbourYMode::NewMv
    } else {
        NeighbourYMode::Other
    };
    mv_grid.record_block(
        mi_row,
        mi_col,
        n4w,
        n4h,
        true,
        ref_frame0,
        None,
        y_mode,
        mv,
        skip == 1,
        interp_filter_symbol(interp),
        use_amvd,
    );
    intrabc_state.record_block(
        frontier.r,
        frontier.c,
        n4w,
        n4h,
        IntrabcBlockPrelude::from_use_skip(
            IntrabcUseSkip {
                use_intrabc: false,
                skip_flag: skip == 1,
            },
            None,
        ),
        tile_offset,
    )?;

    let placed = placed_block(InterBlock {
        ref_frame0,
        ref_frame1: None,
        mv,
        mv1: Mv::ZERO,
        interp,
        warp_params: None,
        bawp,
        residual,
    });
    reconstruct_placed_inter_block(
        workspace,
        &placed,
        ref_frame_idx,
        reference,
        block_qindex,
        luma_use_tcq,
        residual_use_ddt,
        bit_depth,
        tile_offset,
    )?;
    trace_leaf_exit("inter", frontier, entry_checkpoint, symbols.checkpoint());
    Ok(non_intra_leaf_mode(frontier))
}

fn non_intra_leaf_mode(frontier: &DecodeBlockFrontier) -> GeneralIntraLeafMode {
    let leaf = GeneralIntraLeafMode::no_luma_mode();
    if frontier.has_chroma {
        return leaf.with_uv_cfl(false);
    }
    leaf
}

fn reconstruct_intrabc_luma_predictor<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    core: &FrameHeaderCore,
    frontier: &DecodeBlockFrontier,
    n4w: usize,
    n4h: usize,
    info: IntrabcInfo,
    tile_offset: ByteOffset,
) -> Result<()> {
    let prediction = derive_intrabc_luma_prediction_geometry(
        core,
        IntrabcBlockGeometry::from_frontier(frontier, n4w, n4h),
        info,
        tile_offset,
    )?;
    if prediction.source.size() != prediction.target.size() {
        return Err(inter_cap!(
            "inter_intrabc_fractional_predictor",
            tile_offset,
            "inter.intrabc.fractional_predictor",
            SPEC_MODE_INFO
        ));
    }
    workspace
        .copy_rect_within_plane(ReconPlaneId::Y, prediction.source, prediction.target)
        .map_err(|_| {
            inter_cap!(
                "inter_intrabc_copy",
                tile_offset,
                "inter.intrabc.copy",
                SPEC_MODE_INFO
            )
        })
}

fn read_block_reference_mode(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    neighbour_ctx: &super::find_mv_stack::BlockNeighbourContext,
    ref_frame_idx: &[u32],
    ref_order_hint: &[u32],
    current_order_hint: u32,
    tile_offset: ByteOffset,
) -> Result<bool> {
    let current_order_hint = i32::try_from(current_order_hint).unwrap_or(i32::MAX);
    let ctx = neighbour_ctx.comp_mode_ctx(ref_frame_idx, ref_order_hint, current_order_hint);
    let mode = cdfs
        .read_block_symbol_trace(TileCdfSelector::CompMode { ctx }, symbols)
        .map_err(|_| symbol_read_error(tile_offset))?
        .get();
    match mode {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(inter_cap!(
            "inter_block_reference_mode",
            tile_offset,
            "inter.reference_mode out of range",
            SPEC_MODE_INFO
        )),
    }
}

fn inter_residual_geometry_supported(frontier: &DecodeBlockFrontier) -> bool {
    inter_residual_geometry_supported_flags(frontier.is_luma_part(), frontier.is_chroma_part())
}

const fn inter_residual_geometry_supported_flags(is_luma_part: bool, is_chroma_part: bool) -> bool {
    !is_luma_part && !is_chroma_part
}

fn trace_leaf_exit(
    branch: &'static str,
    frontier: &DecodeBlockFrontier,
    before: splot_core::symbol::SymbolDecoderCheckpoint,
    after: splot_core::symbol::SymbolDecoderCheckpoint,
) {
    if std::env::var_os("SPLOT_TRACE_INTER_LEAF_EXIT").is_some()
        && before.symbol_max_bits >= -14
        && after.symbol_max_bits < -14
    {
        eprintln!(
            "inter leaf crossed exit floor branch={branch} r={} c={} b={} before={before:?} after={after:?}",
            frontier.r,
            frontier.c,
            frontier.b_size.index()
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn reconstruct_placed_inter_block<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    placed: &PlacedInterBlock,
    ref_frame_idx: &[u32],
    reference: &InterReferenceState<'_, T>,
    qindex: u32,
    luma_use_tcq: bool,
    residual_use_ddt: bool,
    bit_depth: BitDepth,
    tile_offset: ByteOffset,
) -> Result<()> {
    let rect = mc::McBlockRect {
        luma_x: placed.luma_x,
        luma_y: placed.luma_y,
        luma_w: placed.luma_w,
        luma_h: placed.luma_h,
    };
    let block_params =
        super::resolve_inter_block_params(ref_frame_idx, reference, placed, rect, tile_offset)?;
    mc::motion_compensate_inter_block_into(workspace, block_params, tile_offset)?;
    if let Some(residual) = placed.block.residual.as_ref() {
        super::add_inter_residual_to_workspace(
            workspace,
            residual,
            qindex,
            luma_use_tcq,
            residual_use_ddt,
            bit_depth,
            tile_offset,
        )?;
    }
    Ok(())
}

const INTER_UV_MODE_DC: usize = 0;

#[allow(clippy::too_many_arguments)]
fn read_inter_residual(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    coeff_ctx: &mut TileCoeffContextState,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    frontier: &DecodeBlockFrontier,
    n4w: usize,
    n4h: usize,
    mi_rows: usize,
    mi_cols: usize,
    residual_tool_policy: TransformToolResidualPolicy,
    tile_offset: ByteOffset,
) -> Result<InterResidual> {
    let (subsampling_x, subsampling_y) = chroma_subsampling(sequence.general.chroma_format_idc);
    let width_chunks = (n4w >> 4).max(1);
    let height_chunks = (n4h >> 4).max(1);
    let multi_chunk = width_chunks > 1 || height_chunks > 1;
    let mi_size_chunk = if multi_chunk {
        BlockSize::new(BLOCK_64X64).map_err(|_| residual_geometry_error(tile_offset))?
    } else {
        frontier.b_size
    };
    let luma_tx_size = max_tx_size(frontier.b_size.index(), tile_offset)?;
    let luma_tx_records = if inter_uses_selectable_tx_partitions(core) {
        Some(derive_inter_luma_tx_records_for_block(
            work_unit,
            symbols,
            frontier,
            (mi_rows, mi_cols),
            0,
            tile_offset,
        )?)
    } else {
        None
    };
    let mut blocks = Vec::new();

    for start_chunk_y in (0..height_chunks).step_by(2) {
        for start_chunk_x in (0..width_chunks).step_by(2) {
            for chunk_y in start_chunk_y..(start_chunk_y + 2).min(height_chunks) {
                for chunk_x in start_chunk_x..(start_chunk_x + 2).min(width_chunks) {
                    let at_start = (!subsampling_x || chunk_x % 2 == 0)
                        && (!subsampling_y || chunk_y % 2 == 0);
                    let luma_chunk_x = frontier.c + (chunk_x << 4);
                    let luma_chunk_y = frontier.r + (chunk_y << 4);
                    read_inter_residual_luma_chunk(
                        work_unit,
                        symbols,
                        coeff_ctx,
                        &mut blocks,
                        luma_tx_records.as_deref(),
                        luma_tx_size,
                        frontier,
                        luma_chunk_x,
                        luma_chunk_y,
                        n4w,
                        n4h,
                        residual_tool_policy,
                        tile_offset,
                    )?;
                    if frontier.has_chroma && at_start {
                        read_inter_residual_chroma_group(
                            work_unit,
                            symbols,
                            coeff_ctx,
                            &mut blocks,
                            frontier,
                            mi_size_chunk,
                            subsampling_x,
                            subsampling_y,
                            chunk_x,
                            chunk_y,
                            residual_tool_policy,
                            tile_offset,
                        )?;
                    }
                }
            }
        }
    }

    Ok(InterResidual { blocks })
}

fn inter_uses_selectable_tx_partitions(core: &FrameHeaderCore) -> bool {
    core.inter_tail
        .as_ref()
        .is_some_and(|tail| tail.tx_mode == TxMode::Select)
        && core
            .lossless_info
            .as_ref()
            .is_some_and(|lossless| !lossless.lossless_array[0])
}

#[allow(clippy::too_many_arguments)]
fn read_inter_residual_luma_chunk(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    coeff_ctx: &mut TileCoeffContextState,
    blocks: &mut Vec<InterResidualBlock>,
    luma_tx_records: Option<&[SelectableLumaTxRecord]>,
    tx_size: usize,
    frontier: &DecodeBlockFrontier,
    luma_chunk_x4: usize,
    luma_chunk_y4: usize,
    block_n4w: usize,
    block_n4h: usize,
    residual_tool_policy: TransformToolResidualPolicy,
    tile_offset: ByteOffset,
) -> Result<()> {
    let chunk_end_x4 = (frontier.c + block_n4w).min(luma_chunk_x4 + CHUNK_64_N4);
    let chunk_end_y4 = (frontier.r + block_n4h).min(luma_chunk_y4 + CHUNK_64_N4);
    if let Some(luma_tx_records) = luma_tx_records {
        return read_inter_residual_luma_records_for_chunk(
            work_unit,
            symbols,
            coeff_ctx,
            blocks,
            luma_tx_records,
            frontier,
            luma_chunk_x4,
            luma_chunk_y4,
            chunk_end_x4,
            chunk_end_y4,
            block_n4w,
            block_n4h,
            residual_tool_policy,
            tile_offset,
        );
    }
    let tx_w4 = tx_size_dimension("Tx_Width", &TX_WIDTH, tx_size, tile_offset)? / MI_SIZE;
    let tx_h4 = tx_size_dimension("Tx_Height", &TX_HEIGHT, tx_size, tile_offset)? / MI_SIZE;
    let mut y4 = luma_chunk_y4;
    while y4 < chunk_end_y4 {
        let mut x4 = luma_chunk_x4;
        while x4 < chunk_end_x4 {
            let tx_fills_block = tx_w4 == block_n4w && tx_h4 == block_n4h;
            if std::env::var_os("SPLOT_TRACE_INTER_RESIDUAL_BLOCKS").is_some() {
                let start_x = x4 * MI_SIZE;
                let start_y = y4 * MI_SIZE;
                if start_x >= 1760 && (160..=224).contains(&start_y) {
                    eprintln!(
                        "inter residual luma read r={} c={} b={} n4={}x{} chunk=({}, {}) start=({start_x},{start_y}) tx_size={tx_size} tx4={}x{} fills={tx_fills_block} has_chroma={} chroma_offset={} luma_part={} chroma_part={}",
                        frontier.r,
                        frontier.c,
                        frontier.b_size.index(),
                        block_n4w,
                        block_n4h,
                        luma_chunk_x4,
                        luma_chunk_y4,
                        tx_w4,
                        tx_h4,
                        frontier.has_chroma,
                        frontier.chroma_offset,
                        frontier.is_luma_part(),
                        frontier.is_chroma_part()
                    );
                }
            }
            let coeffs = read_inter_residual_plane(
                work_unit,
                symbols,
                coeff_ctx,
                0,
                tx_size,
                x4 * MI_SIZE,
                y4 * MI_SIZE,
                tx_fills_block,
                false,
                residual_tool_policy,
                tile_offset,
            )?;
            push_inter_residual_block(
                blocks,
                ReconPlaneId::Y,
                x4,
                y4,
                tx_size,
                coeffs,
                tile_offset,
            )?;
            x4 = x4
                .checked_add(tx_w4)
                .ok_or_else(|| residual_geometry_error(tile_offset))?;
        }
        y4 = y4
            .checked_add(tx_h4)
            .ok_or_else(|| residual_geometry_error(tile_offset))?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn read_inter_residual_luma_records_for_chunk(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    coeff_ctx: &mut TileCoeffContextState,
    blocks: &mut Vec<InterResidualBlock>,
    luma_tx_records: &[SelectableLumaTxRecord],
    frontier: &DecodeBlockFrontier,
    luma_chunk_x4: usize,
    luma_chunk_y4: usize,
    chunk_end_x4: usize,
    chunk_end_y4: usize,
    block_n4w: usize,
    block_n4h: usize,
    residual_tool_policy: TransformToolResidualPolicy,
    tile_offset: ByteOffset,
) -> Result<()> {
    let mut decoded_any = false;
    for record in luma_tx_records.iter().copied().filter(|record| {
        record.row >= luma_chunk_y4
            && record.col >= luma_chunk_x4
            && record.row < chunk_end_y4
            && record.col < chunk_end_x4
    }) {
        decoded_any = true;
        let tx_fills_block = record.cols == block_n4w && record.rows == block_n4h;
        let trace_residual = std::env::var_os("SPLOT_TRACE_INTER_RESIDUAL_BLOCKS").is_some();
        let start_x = record.col * MI_SIZE;
        let start_y = record.row * MI_SIZE;
        let trace_this = trace_residual
            && ((512..=640).contains(&start_x) && start_y <= 128
                || (start_x >= 1760 && (160..=224).contains(&start_y)));
        if trace_this {
            eprintln!(
                "inter residual luma read start r={} c={} b={} n4={}x{} chunk=({}, {}) start=({start_x},{start_y}) tx_size={} tx4={}x{} fills={tx_fills_block} has_chroma={} chroma_offset={} luma_part={} chroma_part={} checkpoint={:?}",
                frontier.r,
                frontier.c,
                frontier.b_size.index(),
                block_n4w,
                block_n4h,
                luma_chunk_x4,
                luma_chunk_y4,
                record.tx_size,
                record.cols,
                record.rows,
                frontier.has_chroma,
                frontier.chroma_offset,
                frontier.is_luma_part(),
                frontier.is_chroma_part(),
                symbols.checkpoint()
            );
        }
        let coeffs = read_inter_residual_plane(
            work_unit,
            symbols,
            coeff_ctx,
            0,
            record.tx_size,
            record.col * MI_SIZE,
            record.row * MI_SIZE,
            tx_fills_block,
            false,
            residual_tool_policy,
            tile_offset,
        )?;
        if trace_this {
            eprintln!(
                "inter residual luma read done start=({start_x},{start_y}) tx_size={} all_zero={} eob={} checkpoint={:?}",
                record.tx_size,
                coeffs.all_zero,
                coeffs.eob,
                symbols.checkpoint()
            );
        }
        push_inter_residual_block(
            blocks,
            ReconPlaneId::Y,
            record.col,
            record.row,
            record.tx_size,
            coeffs,
            tile_offset,
        )?;
    }
    if decoded_any {
        Ok(())
    } else {
        Err(residual_geometry_error(tile_offset))
    }
}

#[allow(clippy::too_many_arguments)]
fn read_inter_residual_chroma_group(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    coeff_ctx: &mut TileCoeffContextState,
    blocks: &mut Vec<InterResidualBlock>,
    frontier: &DecodeBlockFrontier,
    mi_size_chunk: BlockSize,
    subsampling_x: bool,
    subsampling_y: bool,
    chunk_x: usize,
    chunk_y: usize,
    residual_tool_policy: TransformToolResidualPolicy,
    tile_offset: ByteOffset,
) -> Result<()> {
    let chroma_ref = frontier.chroma_ref_geometry();
    let chroma_mi_size = chroma_ref.size();
    let chroma_ref_size = get_plane_residual_size(chroma_mi_size, 1, subsampling_x, subsampling_y)
        .map_err(|_| residual_geometry_error(tile_offset))?
        .valid()
        .ok_or_else(|| residual_geometry_error(tile_offset))?;
    let tx_size = max_tx_size(chroma_ref_size.index(), tile_offset)?;
    let plane_source_size = if chroma_mi_size != frontier.b_size {
        chroma_mi_size
    } else {
        mi_size_chunk
    };
    let plane_size = get_plane_residual_size(plane_source_size, 1, subsampling_x, subsampling_y)
        .map_err(|_| residual_geometry_error(tile_offset))?
        .valid()
        .ok_or_else(|| residual_geometry_error(tile_offset))?;
    let block_width_chunks = (frontier
        .b_size
        .num_4x4_wide()
        .map_err(|_| residual_geometry_error(tile_offset))?
        >> 4)
        .max(1);
    let block_height_chunks = (frontier
        .b_size
        .num_4x4_high()
        .map_err(|_| residual_geometry_error(tile_offset))?
        >> 4)
        .max(1);
    let mut num4x4_w = plane_size
        .num_4x4_wide()
        .map_err(|_| residual_geometry_error(tile_offset))?;
    let mut num4x4_h = plane_size
        .num_4x4_high()
        .map_err(|_| residual_geometry_error(tile_offset))?;
    if subsampling_x && chunk_x + 1 < block_width_chunks {
        num4x4_w <<= 1;
    }
    if subsampling_y && chunk_y + 1 < block_height_chunks {
        num4x4_h <<= 1;
    }
    let tx_w4 = tx_size_dimension("Tx_Width", &TX_WIDTH, tx_size, tile_offset)? / MI_SIZE;
    let tx_h4 = tx_size_dimension("Tx_Height", &TX_HEIGHT, tx_size, tile_offset)? / MI_SIZE;
    let x_offset4 = (chunk_x << 4) >> usize::from(subsampling_x);
    let y_offset4 = (chunk_y << 4) >> usize::from(subsampling_y);
    let base_x4 = chroma_ref.col() >> usize::from(subsampling_x);
    let base_y4 = chroma_ref.row() >> usize::from(subsampling_y);
    let mut y4 = y_offset4;
    while y4 < y_offset4 + num4x4_h {
        let mut x4 = x_offset4;
        while x4 < x_offset4 + num4x4_w {
            let start_x = (base_x4 + x4) * MI_SIZE;
            let start_y = (base_y4 + y4) * MI_SIZE;
            let trace_this = std::env::var_os("SPLOT_TRACE_INTER_RESIDUAL_BLOCKS").is_some()
                && ((256..=320).contains(&start_x) && start_y <= 64
                    || (start_x >= 880 && (80..=112).contains(&start_y)));
            if trace_this {
                eprintln!(
                    "inter residual chroma read start block=({},{} b={}) local=({}, {}) start=({start_x},{start_y}) tx_size={tx_size} tx4={}x{} plane_size={}x{} checkpoint={:?}",
                    frontier.r,
                    frontier.c,
                    frontier.b_size.index(),
                    x4,
                    y4,
                    tx_w4,
                    tx_h4,
                    num4x4_w,
                    num4x4_h,
                    symbols.checkpoint()
                );
            }
            let u = read_inter_residual_plane(
                work_unit,
                symbols,
                coeff_ctx,
                1,
                tx_size,
                start_x,
                start_y,
                tx_w4 == num4x4_w && tx_h4 == num4x4_h,
                false,
                residual_tool_policy,
                tile_offset,
            )?;
            let u_nonzero = !u.all_zero;
            if trace_this {
                eprintln!(
                    "inter residual chroma read u done start=({start_x},{start_y}) all_zero={} eob={} checkpoint={:?}",
                    u.all_zero,
                    u.eob,
                    symbols.checkpoint()
                );
            }
            push_inter_residual_block(
                blocks,
                ReconPlaneId::U,
                base_x4 + x4,
                base_y4 + y4,
                tx_size,
                u,
                tile_offset,
            )?;
            let v = read_inter_residual_plane(
                work_unit,
                symbols,
                coeff_ctx,
                2,
                tx_size,
                start_x,
                start_y,
                tx_w4 == num4x4_w && tx_h4 == num4x4_h,
                u_nonzero,
                residual_tool_policy,
                tile_offset,
            )?;
            if trace_this {
                eprintln!(
                    "inter residual chroma read v done start=({start_x},{start_y}) all_zero={} eob={} checkpoint={:?}",
                    v.all_zero,
                    v.eob,
                    symbols.checkpoint()
                );
            }
            push_inter_residual_block(
                blocks,
                ReconPlaneId::V,
                base_x4 + x4,
                base_y4 + y4,
                tx_size,
                v,
                tile_offset,
            )?;
            x4 = x4
                .checked_add(tx_w4)
                .ok_or_else(|| residual_geometry_error(tile_offset))?;
        }
        y4 = y4
            .checked_add(tx_h4)
            .ok_or_else(|| residual_geometry_error(tile_offset))?;
    }
    Ok(())
}

fn push_inter_residual_block(
    blocks: &mut Vec<InterResidualBlock>,
    plane: ReconPlaneId,
    x4: usize,
    y4: usize,
    tx_size: usize,
    coeffs: LumaCoeffBlock,
    tile_offset: ByteOffset,
) -> Result<()> {
    let log2_width = tx_size_dimension("Tx_Width_Log2", &TX_WIDTH_LOG2, tx_size, tile_offset)?;
    let log2_height = tx_size_dimension("Tx_Height_Log2", &TX_HEIGHT_LOG2, tx_size, tile_offset)?;
    let log2_width = u32::try_from(log2_width).map_err(|_| residual_geometry_error(tile_offset))?;
    let log2_height =
        u32::try_from(log2_height).map_err(|_| residual_geometry_error(tile_offset))?;
    blocks.try_reserve(1).map_err(|_| {
        inter_cap!(
            "inter_block_residual_allocation",
            tile_offset,
            "inter.residual.transform_block_list",
            SPEC_MODE_INFO
        )
    })?;
    blocks.push(InterResidualBlock {
        plane,
        x: x4 * MI_SIZE,
        y: y4 * MI_SIZE,
        log2_width,
        log2_height,
        coeffs,
    });
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn read_inter_residual_plane(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    coeff_ctx: &mut TileCoeffContextState,
    plane: usize,
    tx_size: usize,
    start_x: usize,
    start_y: usize,
    tx_fills_block: bool,
    chroma_eob_ctx: bool,
    residual_tool_policy: TransformToolResidualPolicy,
    tile_offset: ByteOffset,
) -> Result<LumaCoeffBlock> {
    decode_general_intra_plane_coeffs(
        work_unit,
        symbols,
        coeff_ctx,
        plane,
        tx_size,
        start_x,
        start_y,
        tx_fills_block,
        None,
        chroma_eob_ctx,
        INTER_UV_MODE_DC,
        0,
        true,
        false,
        false,
        residual_tool_policy,
    )
    .map_err(|error| {
        if std::env::var_os("SPLOT_TRACE_INTER_RESIDUAL_ERROR").is_some() {
            eprintln!(
                "inter residual error offset={} plane={plane} tx_size={tx_size} start=({start_x},{start_y}) fills={tx_fills_block}: {error:?}",
                tile_offset.get(),
            );
        }
        residual_read_error(tile_offset)
    })
}

fn transform_tool_residual_policy(sequence: &SequenceHeader) -> TransformToolResidualPolicy {
    sequence
        .transform_quant_entropy
        .as_ref()
        .map_or(TransformToolResidualPolicy::Allow, |tq| {
            if tq.enable_inter_ist
                || tq.enable_intra_ist
                || tq.enable_inter_ddt
                || tq.enable_cctx
                || tq.enable_fsc
                || tq.enable_idtx_intra
            {
                TransformToolResidualPolicy::AdmitTransformToolSubset {
                    luma: None,
                    active_intra_ist: ActiveIntraIstResidualPolicy::LrTxSkipRecordHandoff,
                    active_chroma: ActiveChromaResidualPolicy::LrTxSkipRecordHandoff,
                }
            } else {
                TransformToolResidualPolicy::Allow
            }
        })
}

fn max_tx_size(block_size: usize, tile_offset: ByteOffset) -> Result<usize> {
    table_value_usize(
        "Max_Tx_Size_Rect",
        &MAX_TX_SIZE_RECT,
        block_size,
        tile_offset,
    )
}

fn tx_size_dimension(
    table: &'static str,
    values: &[i32],
    tx_size: usize,
    tile_offset: ByteOffset,
) -> Result<usize> {
    table_value_usize(table, values, tx_size, tile_offset)
}

fn table_value_usize(
    _table: &'static str,
    values: &[i32],
    index: usize,
    tile_offset: ByteOffset,
) -> Result<usize> {
    let value = values
        .get(index)
        .copied()
        .ok_or_else(|| residual_geometry_error(tile_offset))?;
    usize::try_from(value).map_err(|_| residual_geometry_error(tile_offset))
}

fn residual_geometry_error(tile_offset: ByteOffset) -> super::super::DecodeError {
    inter_cap!(
        "inter_block_residual_geometry",
        tile_offset,
        "inter.residual.transform_geometry",
        SPEC_MODE_INFO
    )
}

fn reset_inter_skip_coeff_contexts(
    coeff_ctx: &mut TileCoeffContextState,
    frontier: &DecodeBlockFrontier,
    n4w: usize,
    n4h: usize,
    tile_offset: ByteOffset,
) -> Result<()> {
    let plane_count = 1 + usize::from(frontier.has_chroma) * (COEFF_CONTEXT_PLANES.len() - 1);
    for &(plane, sub) in COEFF_CONTEXT_PLANES.iter().take(plane_count) {
        coeff_ctx
            .reset_block_context_plane(CoeffContextReset {
                plane,
                c: frontier.c,
                r: frontier.r,
                w4: n4w,
                h4: n4h,
                sub_x: sub,
                sub_y: sub,
            })
            .map_err(|_| residual_geometry_error(tile_offset))?;
    }
    Ok(())
}

fn residual_read_error(tile_offset: ByteOffset) -> super::super::DecodeError {
    inter_missing!(
        "inter_block_residual_parse",
        tile_offset,
        "inter.residual.coefficients",
        SPEC_MODE_INFO
    )
}

fn resolve_interp_filter(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    frame_interpolation_filter: FrameInterpolationFilter,
    mode_for_needs_interp_filter: u8,
    ctx: usize,
    tile_offset: ByteOffset,
) -> Result<ReconInterpolationFilter> {
    match frame_interpolation_filter {
        FrameInterpolationFilter::Eighttap => Ok(ReconInterpolationFilter::EightTap),
        FrameInterpolationFilter::EighttapSmooth => Ok(ReconInterpolationFilter::EightTapSmooth),
        FrameInterpolationFilter::EighttapSharp => Ok(ReconInterpolationFilter::EightTapSharp),
        FrameInterpolationFilter::Bilinear => Ok(ReconInterpolationFilter::Bilinear),
        FrameInterpolationFilter::Switchable => {
            if mode_for_needs_interp_filter == SINGLE_MODE_GLOBALMV {
                return Ok(ReconInterpolationFilter::EightTap);
            }
            let symbol = cdfs
                .read_block_symbol_trace(TileCdfSelector::InterpFilter { ctx }, symbols)
                .map_err(|_| symbol_read_error(tile_offset))?;
            interp_filter_from_symbol(symbol.get(), tile_offset)
        }
        _ => Err(inter_cap!(
            "inter_unsupported_interpolation_filter",
            tile_offset,
            "inter.interpolation_filter",
            SPEC_MODE_INFO
        )),
    }
}

#[allow(clippy::too_many_arguments)]
fn trace_interp_filter_context(
    kind: &'static str,
    mi_row: usize,
    mi_col: usize,
    ref_frame0: i8,
    ref_frame1_is_inter: bool,
    ctx: usize,
    neighbour_ctx: &super::find_mv_stack::BlockNeighbourContext,
    symbols: &SymbolDecoder<'_>,
) {
    if std::env::var_os("SPLOT_TRACE_INTERP_FILTER_CTX").is_none() {
        return;
    }
    eprintln!(
        "interp_filter_ctx kind={kind} r={mi_row} c={mi_col} ref0={ref_frame0} ref1_is_inter={ref_frame1_is_inter} ctx={ctx} neighbour={neighbour_ctx:?} checkpoint={:?}",
        symbols.checkpoint(),
    );
}

pub(super) fn interp_filter_no_neighbour_ctx(ref_frame1_is_inter: bool) -> usize {
    INTERP_FILTER_CTX_NO_NEIGHBOUR_BASE
        + usize::from(ref_frame1_is_inter) * INTERP_FILTER_CTX_SECOND_REF_INTER_OFFSET
}

fn interp_filter_symbol(filter: ReconInterpolationFilter) -> u8 {
    match filter {
        ReconInterpolationFilter::EightTap => 0,
        ReconInterpolationFilter::EightTapSmooth => 1,
        ReconInterpolationFilter::EightTapSharp => 2,
        ReconInterpolationFilter::Bilinear => 3,
    }
}

fn interp_filter_from_symbol(
    symbol: u8,
    tile_offset: ByteOffset,
) -> Result<ReconInterpolationFilter> {
    match symbol {
        0 => Ok(ReconInterpolationFilter::EightTap),
        1 => Ok(ReconInterpolationFilter::EightTapSmooth),
        2 => Ok(ReconInterpolationFilter::EightTapSharp),
        3 => Ok(ReconInterpolationFilter::Bilinear),
        _ => Err(inter_cap!(
            "inter_invalid_interp_filter_symbol",
            tile_offset,
            "inter.interp_filter symbol out of range",
            SPEC_MODE_INFO
        )),
    }
}

fn read_drl_idx(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    new_mv_context: usize,
    max_drl_bits_minus_1: u32,
    tile_offset: ByteOffset,
) -> Result<usize> {
    let m = max_drl_bits_minus_1.saturating_add(1) as usize;
    for idx in 0..m {
        let bank = idx.min(2);
        let drl_mode = cdfs
            .read_block_symbol_trace(
                TileCdfSelector::DrlMode {
                    idx: bank,
                    ctx: new_mv_context,
                },
                symbols,
            )
            .map_err(|_| symbol_read_error(tile_offset))?;
        if drl_mode.get() == 0 {
            return Ok(idx);
        }
    }
    Ok(m)
}

fn read_use_amvd_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    enable_adaptive_mvd: bool,
    single_mode: u8,
    ctx: usize,
    tile_offset: ByteOffset,
) -> Result<bool> {
    if !enable_adaptive_mvd || single_mode != SINGLE_MODE_NEWMV {
        return Ok(false);
    }
    let use_amvd = cdfs
        .read_block_symbol_trace(TileCdfSelector::UseAmvd { index: 4, ctx }, symbols)
        .map_err(|_| symbol_read_error(tile_offset))?;
    Ok(use_amvd.get() != 0)
}

fn effective_force_integer_mv(core: &FrameHeaderCore) -> bool {
    core.force_integer_mv
        .or_else(|| core.inter.as_ref().and_then(|inter| inter.force_integer_mv))
        .unwrap_or(false)
}

#[allow(clippy::too_many_arguments)]
fn read_warp_inter_mode_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    allow_warpmv_mode: bool,
    force_integer_mv: bool,
    n4w: usize,
    n4h: usize,
    ctx: usize,
    tile_offset: ByteOffset,
) -> Result<Option<WarpInterMode>> {
    if !allow_warpmv_mode || n4w < 2 || n4h < 2 {
        return Ok(None);
    }
    let is_warp = cdfs
        .read_block_symbol_trace(TileCdfSelector::IsWarp { ctx }, symbols)
        .map_err(|_| symbol_read_error(tile_offset))?;
    if is_warp.get() == 0 {
        return Ok(None);
    }
    if force_integer_mv {
        return Ok(Some(WarpInterMode::Warpmv));
    }
    let warp_mv = cdfs
        .read_block_symbol_trace(TileCdfSelector::WarpMv, symbols)
        .map_err(|_| symbol_read_error(tile_offset))?;
    Ok(Some(if warp_mv.get() == 0 {
        WarpInterMode::WarpNewmv
    } else {
        WarpInterMode::Warpmv
    }))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ParsedWarpNewmv {
    mv: Mv,
    warp_params: [i64; 6],
    ref_mv_idx: usize,
    ref_warp_idx: usize,
    precision_idx: u8,
    warpmv_with_mvd: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct WarpInterIntraSyntax {
    enabled: bool,
    mode: Option<u8>,
    use_wedge: bool,
    wedge_index: Option<u8>,
}

fn inter_mv_read_config(core: &FrameHeaderCore, tile_offset: ByteOffset) -> Result<MvReadConfig> {
    let precision = core
        .inter
        .as_ref()
        .and_then(|inter| inter.mv_precision)
        .ok_or_else(|| {
            inter_missing!(
                "inter_mv_precision",
                tile_offset,
                "inter.mv_precision",
                SPEC_MODE_INFO
            )
        })?;
    let precision = mv_precision_code(precision).ok_or_else(|| {
        inter_cap!(
            "inter_mv_precision_unsupported",
            tile_offset,
            "inter.mv_precision unsupported",
            SPEC_MODE_INFO
        )
    })?;
    Ok(MvReadConfig::inter(precision))
}

const fn mv_precision_code(precision: MvPrecision) -> Option<u8> {
    Some(match precision {
        MvPrecision::OnePel => MV_PRECISION_ONE_PEL,
        MvPrecision::HalfPel => MV_PRECISION_HALF_PEL,
        MvPrecision::QuarterPel => MV_PRECISION_QUARTER_PEL,
        MvPrecision::EighthPel => MV_PRECISION_EIGHTH_PEL,
        _ => return None,
    })
}

fn inter_mvd_sign_derivation_allowed(
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    single_mode: u8,
    use_amvd: bool,
    config: MvReadConfig,
) -> bool {
    if single_mode != SINGLE_MODE_NEWMV || use_amvd {
        return false;
    }
    if !sequence
        .inter
        .as_ref()
        .is_some_and(|inter| inter.enable_mvd_sign_derive)
    {
        return false;
    }
    if effective_allow_screen_content_tools(core) {
        return false;
    }
    config.precision() < MV_PRECISION_QUARTER_PEL
}

#[allow(clippy::too_many_arguments)]
fn read_warp_newmv_delta_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    sequence: &SequenceHeader,
    mv_config: MvReadConfig,
    b_size: usize,
    mi_row: usize,
    mi_col: usize,
    n4w: usize,
    n4h: usize,
    stack: &super::find_mv_stack::MvStack,
    new_mv_context: usize,
    max_drl_bits_minus_1: u32,
    tile_offset: ByteOffset,
) -> Result<ParsedWarpNewmv> {
    let ref_warp_idx = read_warp_ref_idx(cdfs, symbols, MAX_WARP_REF_CANDIDATES, tile_offset)?;
    let ref_mv_idx = read_drl_idx(
        cdfs,
        symbols,
        new_mv_context,
        max_drl_bits_minus_1,
        tile_offset,
    )?;
    let pred_mv = stack.candidate(ref_mv_idx);
    let magnitude = read_newmv_block_mvd_magnitude(cdfs, symbols, tile_offset, mv_config)?;
    let diff = apply_inter_mvd_signs(magnitude, symbols, tile_offset, mv_config, false, 1)?;
    let mv = Mv {
        row: mv_clamp_to_integer(pred_mv.row + diff.row),
        col: mv_clamp_to_integer(pred_mv.col + diff.col),
    };
    let (warp_params, precision_idx) = read_warp_delta_syntax(
        cdfs,
        symbols,
        sequence,
        b_size,
        ref_warp_idx,
        mv,
        mi_row,
        mi_col,
        n4w,
        n4h,
        tile_offset,
    )?;
    Ok(ParsedWarpNewmv {
        mv,
        warp_params,
        ref_mv_idx,
        ref_warp_idx,
        precision_idx,
        warpmv_with_mvd: false,
    })
}

#[allow(clippy::too_many_arguments)]
fn read_warpmv_delta_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    mv_config: MvReadConfig,
    _b_size: usize,
    mi_row: usize,
    mi_col: usize,
    n4w: usize,
    n4h: usize,
    stack: &super::find_mv_stack::MvStack,
    tile_offset: ByteOffset,
) -> Result<ParsedWarpNewmv> {
    let ref_warp_idx = read_warp_ref_idx(cdfs, symbols, MAX_WARP_REF_CANDIDATES, tile_offset)?;
    let warpmv_with_mvd = if ref_warp_idx < 2 {
        read_warpmv_with_mvd_flag(cdfs, symbols, tile_offset)?
    } else {
        false
    };
    let base_mv = stack.candidate(0);
    let mv = if warpmv_with_mvd {
        let magnitude = read_newmv_block_mvd_magnitude(cdfs, symbols, tile_offset, mv_config)?;
        let diff = apply_inter_mvd_signs(magnitude, symbols, tile_offset, mv_config, false, 1)?;
        Mv {
            row: mv_clamp_to_integer(base_mv.row + diff.row),
            col: mv_clamp_to_integer(base_mv.col + diff.col),
        }
    } else {
        base_mv
    };
    let warp_params = derive_warp_params_from_mv(mv, mi_row, mi_col, n4w, n4h, tile_offset)?;
    Ok(ParsedWarpNewmv {
        mv,
        warp_params,
        ref_mv_idx: 0,
        ref_warp_idx,
        precision_idx: 0,
        warpmv_with_mvd,
    })
}

fn read_warpmv_with_mvd_flag(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    tile_offset: ByteOffset,
) -> Result<bool> {
    let flag = cdfs
        .read_block_symbol_trace(TileCdfSelector::WarpWithMvd, symbols)
        .map_err(|_| symbol_read_error(tile_offset))?
        .get();
    if flag > 1 {
        return Err(inter_cap!(
            "inter_warpmv_with_mvd_symbol",
            tile_offset,
            "inter.warpmv_with_mvd_flag symbol out of range",
            SPEC_MODE_INFO
        ));
    }
    Ok(flag != 0)
}

fn read_warp_ref_idx(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    max_num_warp_candidates: usize,
    tile_offset: ByteOffset,
) -> Result<usize> {
    if max_num_warp_candidates <= 1 {
        return Ok(0);
    }
    let mut ref_warp_idx = 0usize;
    for bit_idx in 0..max_num_warp_candidates.saturating_sub(1) {
        let warp_idx = cdfs
            .read_block_symbol_trace(TileCdfSelector::WarpIdx { ctx: bit_idx }, symbols)
            .map_err(|_| symbol_read_error(tile_offset))?
            .get();
        if warp_idx > 1 {
            return Err(inter_cap!(
                "inter_warp_ref_idx_symbol",
                tile_offset,
                "inter.warp_ref_idx symbol out of range",
                SPEC_MODE_INFO
            ));
        }
        ref_warp_idx = bit_idx + usize::from(warp_idx);
        if warp_idx == 0 {
            break;
        }
    }
    Ok(ref_warp_idx)
}

fn read_warp_inter_intra_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    b_size: usize,
    n4w: usize,
    n4h: usize,
    tile_offset: ByteOffset,
) -> Result<WarpInterIntraSyntax> {
    if n4w < 2 || n4h < 2 || n4w.max(n4h) > CHUNK_64_N4 {
        return Ok(WarpInterIntraSyntax::default());
    }
    let bsize_group = *SIZE_GROUP_LOOKUP.get(b_size).ok_or_else(|| {
        inter_cap!(
            "inter_warp_interintra_bsize_group",
            tile_offset,
            "inter.warp_inter_intra block size out of range",
            SPEC_MODE_INFO
        )
    })?;
    let enabled = cdfs
        .read_block_symbol_trace(TileCdfSelector::WarpInterIntra { bsize_group }, symbols)
        .map_err(|_| symbol_read_error(tile_offset))?
        .get();
    if enabled > 1 {
        return Err(inter_cap!(
            "inter_warp_interintra_symbol",
            tile_offset,
            "inter.warp_inter_intra symbol out of range",
            SPEC_MODE_INFO
        ));
    }
    if enabled == 0 {
        return Ok(WarpInterIntraSyntax::default());
    }

    let mode = cdfs
        .read_block_symbol_trace(TileCdfSelector::InterIntraMode { bsize_group }, symbols)
        .map_err(|_| symbol_read_error(tile_offset))?
        .get();
    if mode >= INTERINTRA_MODES {
        return Err(inter_cap!(
            "inter_warp_interintra_mode_symbol",
            tile_offset,
            "inter.interintra_mode symbol out of range",
            SPEC_MODE_INFO
        ));
    }

    let use_wedge = if WEDGE_USED_BY_BSIZE.get(b_size).copied().unwrap_or(false) {
        let symbol = cdfs
            .read_block_symbol_trace(TileCdfSelector::WedgeInterIntra, symbols)
            .map_err(|_| symbol_read_error(tile_offset))?
            .get();
        if symbol > 1 {
            return Err(inter_cap!(
                "inter_wedge_interintra_symbol",
                tile_offset,
                "inter.use_wedge_interintra symbol out of range",
                SPEC_MODE_INFO
            ));
        }
        symbol != 0
    } else {
        false
    };
    let wedge_index = if use_wedge {
        Some(read_wedge_mode_syntax(cdfs, symbols, tile_offset)?)
    } else {
        None
    };

    Ok(WarpInterIntraSyntax {
        enabled: true,
        mode: Some(mode),
        use_wedge,
        wedge_index,
    })
}

fn read_wedge_mode_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    tile_offset: ByteOffset,
) -> Result<u8> {
    let quad = cdfs
        .read_block_symbol_trace(TileCdfSelector::WedgeQuad, symbols)
        .map_err(|_| symbol_read_error(tile_offset))?
        .get();
    if quad >= WEDGE_QUADS {
        return Err(inter_cap!(
            "inter_wedge_quad_symbol",
            tile_offset,
            "inter.wedge_quad symbol out of range",
            SPEC_MODE_INFO
        ));
    }
    let angle_in_quad = cdfs
        .read_block_symbol_trace(
            TileCdfSelector::WedgeAngle {
                quad: usize::from(quad),
            },
            symbols,
        )
        .map_err(|_| symbol_read_error(tile_offset))?
        .get();
    if angle_in_quad >= QUAD_WEDGE_ANGLES {
        return Err(inter_cap!(
            "inter_wedge_angle_symbol",
            tile_offset,
            "inter.wedge_angle symbol out of range",
            SPEC_MODE_INFO
        ));
    }
    let angle = quad
        .checked_mul(QUAD_WEDGE_ANGLES)
        .and_then(|base| base.checked_add(angle_in_quad))
        .ok_or_else(|| warp_model_error(tile_offset))?;
    let use_dist2 = angle >= H_WEDGE_ANGLES || matches!(angle, WEDGE_90 | WEDGE_0);
    let dist = if use_dist2 {
        let symbol = cdfs
            .read_block_symbol_trace(TileCdfSelector::WedgeDist2, symbols)
            .map_err(|_| symbol_read_error(tile_offset))?
            .get();
        if symbol >= NUM_WEDGE_DIST - 1 {
            return Err(inter_cap!(
                "inter_wedge_dist2_symbol",
                tile_offset,
                "inter.wedge_dist_cdf2 symbol out of range",
                SPEC_MODE_INFO
            ));
        }
        symbol + 1
    } else {
        let symbol = cdfs
            .read_block_symbol_trace(TileCdfSelector::WedgeDist1, symbols)
            .map_err(|_| symbol_read_error(tile_offset))?
            .get();
        if symbol >= NUM_WEDGE_DIST {
            return Err(inter_cap!(
                "inter_wedge_dist1_symbol",
                tile_offset,
                "inter.wedge_dist_cdf symbol out of range",
                SPEC_MODE_INFO
            ));
        }
        symbol
    };
    Ok(angle.saturating_mul(NUM_WEDGE_DIST).saturating_add(dist))
}

#[allow(clippy::too_many_arguments)]
fn read_warp_delta_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    sequence: &SequenceHeader,
    b_size: usize,
    ref_warp_idx: usize,
    mv: Mv,
    mi_row: usize,
    mi_col: usize,
    n4w: usize,
    n4h: usize,
    tile_offset: ByteOffset,
) -> Result<([i64; 6], u8)> {
    if ref_warp_idx != 0 {
        return Err(inter_cap!(
            "inter_warp_ref_candidate_unimplemented",
            tile_offset,
            "inter.warp_param_stack reference",
            "5.20.7.7"
        ));
    }
    let mut params = IDENTITY_WARP_PARAMS;
    let use_six_param = sequence
        .inter
        .as_ref()
        .is_some_and(|inter| inter.enable_six_param_warp_delta)
        && ref_warp_idx == 1;
    let mut precision_idx = 0u8;

    if use_six_param || ref_warp_idx == 0 {
        precision_idx = cdfs
            .read_block_symbol_trace(
                TileCdfSelector::WarpPrecision { block_size: b_size },
                symbols,
            )
            .map_err(|_| symbol_read_error(tile_offset))?
            .get();
        if precision_idx > 1 {
            return Err(inter_cap!(
                "inter_warp_precision_symbol",
                tile_offset,
                "inter.warp_delta_precision symbol out of range",
                SPEC_MODE_INFO
            ));
        }
        let high_precision = precision_idx != 0;
        params[0] = 0;
        params[1] = 0;
        params[2] += read_warp_delta_param(cdfs, symbols, 2, high_precision, tile_offset)?;
        params[3] += read_warp_delta_param(cdfs, symbols, 3, high_precision, tile_offset)?;
        if use_six_param {
            params[4] += read_warp_delta_param(cdfs, symbols, 4, high_precision, tile_offset)?;
            params[5] += read_warp_delta_param(cdfs, symbols, 5, high_precision, tile_offset)?;
        } else {
            params[4] = -params[3];
            params[5] = params[2];
        }
    }

    reduce_warp_model(&mut params);
    set_warp_translation(&mut params, mv, mi_row, mi_col, n4w, n4h, tile_offset)?;
    Ok((params, precision_idx))
}

fn derive_warp_params_from_mv(
    mv: Mv,
    mi_row: usize,
    mi_col: usize,
    n4w: usize,
    n4h: usize,
    tile_offset: ByteOffset,
) -> Result<[i64; 6]> {
    let mut params = IDENTITY_WARP_PARAMS;
    reduce_warp_model(&mut params);
    set_warp_translation(&mut params, mv, mi_row, mi_col, n4w, n4h, tile_offset)?;
    Ok(params)
}

fn read_warp_delta_param(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    index: usize,
    high_precision: bool,
    tile_offset: ByteOffset,
) -> Result<i64> {
    let index_type = match index {
        2 | 5 => 0,
        3 | 4 => 1,
        _ => {
            return Err(inter_cap!(
                "inter_warp_delta_param_index",
                tile_offset,
                "inter.warp_delta_param index out of range",
                SPEC_MODE_INFO
            ));
        }
    };
    let mut value = cdfs
        .read_block_symbol_trace(TileCdfSelector::WarpDeltaParamLow { index_type }, symbols)
        .map_err(|_| symbol_read_error(tile_offset))?
        .get();
    if high_precision && value == WARP_DELTA_NUM_SYMBOLS_LOW - 1 {
        let high = cdfs
            .read_block_symbol_trace(TileCdfSelector::WarpDeltaParamHigh { index_type }, symbols)
            .map_err(|_| symbol_read_error(tile_offset))?
            .get();
        if high >= WARP_DELTA_NUM_SYMBOLS_HIGH {
            return Err(inter_cap!(
                "inter_warp_delta_param_high_symbol",
                tile_offset,
                "inter.warp_delta_param_high symbol out of range",
                SPEC_MODE_INFO
            ));
        }
        value = value
            .checked_add(high)
            .ok_or_else(|| warp_model_error(tile_offset))?;
    }
    let mut signed = i64::from(value);
    if signed != 0 {
        let sign = cdfs
            .read_block_symbol_trace(TileCdfSelector::WarpDeltaParamSign, symbols)
            .map_err(|_| symbol_read_error(tile_offset))?
            .get();
        if sign > 1 {
            return Err(inter_cap!(
                "inter_warp_delta_param_sign_symbol",
                tile_offset,
                "inter.warp_delta_param_sign symbol out of range",
                SPEC_MODE_INFO
            ));
        }
        if sign != 0 {
            signed = -signed;
        }
    }
    let step_bits = WARP_DELTA_STEP_BITS + 1 - u32::from(high_precision);
    signed
        .checked_shl(step_bits)
        .ok_or_else(|| warp_model_error(tile_offset))
}

fn reduce_warp_model(params: &mut [i64; 6]) {
    let max_value = (1i64 << (WARPEDMODEL_PREC_BITS - 1)) - (1i64 << WARP_PARAM_REDUCE_BITS);
    let min_value = -max_value;
    for (index, param) in params.iter_mut().enumerate().skip(2) {
        let offset = if index == 2 || index == 5 {
            1i64 << WARPEDMODEL_PREC_BITS
        } else {
            0
        };
        let original = *param - offset;
        let clamped = original.clamp(min_value, max_value);
        *param =
            (round2_signed(clamped, WARP_PARAM_REDUCE_BITS) << WARP_PARAM_REDUCE_BITS) + offset;
    }
}

fn set_warp_translation(
    params: &mut [i64; 6],
    mv: Mv,
    mi_row: usize,
    mi_col: usize,
    n4w: usize,
    n4h: usize,
    tile_offset: ByteOffset,
) -> Result<()> {
    let center_x = mi_col
        .checked_mul(MI_SIZE)
        .and_then(|value| value.checked_add(n4w.checked_mul(2)?))
        .and_then(|value| value.checked_sub(1))
        .ok_or_else(|| warp_model_error(tile_offset))?;
    let center_y = mi_row
        .checked_mul(MI_SIZE)
        .and_then(|value| value.checked_add(n4h.checked_mul(2)?))
        .and_then(|value| value.checked_sub(1))
        .ok_or_else(|| warp_model_error(tile_offset))?;
    let one = 1i128 << WARPEDMODEL_PREC_BITS;
    let mv_scale = 1i128 << (WARPEDMODEL_PREC_BITS - 3);
    let wmmat0 = i128::from(mv.col) * mv_scale
        - (center_x as i128 * (i128::from(params[2]) - one)
            + center_y as i128 * i128::from(params[3]));
    let wmmat1 = i128::from(mv.row) * mv_scale
        - (center_x as i128 * i128::from(params[4])
            + center_y as i128 * (i128::from(params[5]) - one));
    let high = WARPEDMODEL_TRANS_CLAMP - (1 << WARP_PARAM_REDUCE_BITS);
    params[0] = clamp_i128_to_i64(wmmat0, -WARPEDMODEL_TRANS_CLAMP, high);
    params[1] = clamp_i128_to_i64(wmmat1, -WARPEDMODEL_TRANS_CLAMP, high);
    Ok(())
}

fn clamp_i128_to_i64(value: i128, low: i64, high: i64) -> i64 {
    value.clamp(i128::from(low), i128::from(high)) as i64
}

fn warp_model_error(tile_offset: ByteOffset) -> crate::error::DecodeError {
    inter_cap!(
        "inter_warp_model_overflow",
        tile_offset,
        "inter.warp_model arithmetic overflow",
        SPEC_MODE_INFO
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct BawpParseInput {
    allow_bawp: bool,
    frame_is_switch: bool,
    single_mode: u8,
    use_amvd: bool,
    n4w: usize,
    n4h: usize,
    has_chroma: bool,
}

fn read_bawp_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: BawpParseInput,
    tile_offset: ByteOffset,
) -> Result<BawpSyntax> {
    if !input.allow_bawp
        || input.frame_is_switch
        || input.single_mode == SINGLE_MODE_GLOBALMV
        || input.n4w < 2
        || input.n4h < 2
    {
        return Ok(BawpSyntax::default());
    }

    let use_bawp = cdfs
        .read_block_symbol_trace(TileCdfSelector::UseBawp, symbols)
        .map_err(|_| symbol_read_error(tile_offset))?;
    if use_bawp.get() == 0 {
        return Ok(BawpSyntax::default());
    }

    let mut luma_flag = 1u8;
    let explicit_bawp = cdfs
        .read_block_symbol_trace(
            TileCdfSelector::ExplicitBawp {
                ctx: explicit_bawp_context(input.single_mode, input.use_amvd),
            },
            symbols,
        )
        .map_err(|_| symbol_read_error(tile_offset))?;
    if explicit_bawp.get() != 0 {
        luma_flag = luma_flag.saturating_add(1);
        let explicit_bawp_scale = cdfs
            .read_block_symbol_trace(TileCdfSelector::ExplicitBawpScale, symbols)
            .map_err(|_| symbol_read_error(tile_offset))?;
        luma_flag = luma_flag.saturating_add(explicit_bawp_scale.get());
    }
    let chroma_flag = if input.has_chroma {
        cdfs.read_block_symbol_trace(TileCdfSelector::UseBawpChroma, symbols)
            .map_err(|_| symbol_read_error(tile_offset))?
            .get()
            != 0
    } else {
        false
    };

    Ok(BawpSyntax {
        luma_flag,
        chroma_flag,
    })
}

fn explicit_bawp_context(single_mode: u8, use_amvd: bool) -> usize {
    if single_mode == SINGLE_MODE_NEARMV {
        0
    } else if single_mode == SINGLE_MODE_NEWMV && use_amvd {
        1
    } else {
        2
    }
}

fn symbol_read_error(tile_offset: ByteOffset) -> super::super::DecodeError {
    inter_missing!(
        "inter_block_mode_parse",
        tile_offset,
        "inter.block.mode_info_symbols",
        SPEC_MODE_INFO
    )
}

fn map_inter_multiblock_error(
    error: GeneralIntraMultiblockError<super::super::DecodeError>,
    tile_offset: ByteOffset,
) -> super::super::DecodeError {
    match error {
        GeneralIntraMultiblockError::Walk(GeneralIntraTreeWalkError::Leaf(error)) => error,
        GeneralIntraMultiblockError::Walk(GeneralIntraTreeWalkError::Traversal(
            TilePartitionTraversalError::Limit(source),
        )) => super::super::DecodeError::Limit { source },
        _ => inter_cap!(
            "inter_partition_walk",
            tile_offset,
            "inter.partition_walk",
            SPEC_MODE_INFO
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::inter_residual_geometry_supported_flags;

    #[test]
    fn inter_residual_geometry_allows_shared_leaves() {
        assert!(inter_residual_geometry_supported_flags(false, false));
    }

    #[test]
    fn inter_residual_geometry_rejects_chroma_partitioned_leaves() {
        assert!(!inter_residual_geometry_supported_flags(true, false));
        assert!(!inter_residual_geometry_supported_flags(false, true));
    }
}

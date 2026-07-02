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
    BitDepth, CurrentFrameWorkspace, IDENTITY_WARP_PARAMS, InterIntraMode,
    InterpolationFilter as ReconInterpolationFilter, IntraCardinalDirection,
    IntraDirectionalAngleEdges, IntraRectBlockSize, ReconSample, apply_intra_ibp_dc_rect,
    predict_intra_cardinal_directional_rect_into, predict_intra_dc_rect_value,
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
use super::compound::{
    CompoundParseInput, read_compound_mode_syntax, read_compound_reference_pair,
};
use super::find_mv_stack::{
    BlockNeighbourContext, BlockPrecisionRecord, ModeContext, MotionMode, MvBlockContext,
    NeighbourMvGrid, NeighbourYMode, block_neighbour_ctx, find_mode_ctx, find_mv_stack,
};
use super::read_mv::{
    MV_PRECISION_EIGHTH_PEL, MV_PRECISION_HALF_PEL, MV_PRECISION_ONE_PEL, MV_PRECISION_QUARTER_PEL,
    MV_PRECISION_TWO_PEL, MvReadConfig, apply_inter_mvd_signs, lower_mv_precision,
    mv_clamp_to_integer, read_newmv_amvd_block_mvd, read_newmv_block_mvd_magnitude,
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
    TransformToolResidualPolicy, chroma_subsampling,
    decode_general_intra_multiblock_tree_with_lr_source_blocks, decode_general_intra_plane_coeffs,
    frame_mi_dimensions, get_plane_residual_size,
};

const INTERP_FILTER_CTX_NO_NEIGHBOUR_BASE: usize = 3;
const INTERP_FILTER_CTX_SECOND_REF_INTER_OFFSET: usize = 4;
const SINGLE_REF_FRAME0: i8 = 0;
const MI_SIZE: usize = 4;
const CHUNK_64_N4: usize = 16;
const BLOCK_8X8: usize = 3;
const BLOCK_64X64: usize = 12;
const MAX_WARP_REF_CANDIDATES: usize = 4;
const WARP_DELTA_NUM_SYMBOLS_LOW: u8 = 8;
const WARP_DELTA_NUM_SYMBOLS_HIGH: u8 = 8;
const WARPEDMODEL_PREC_BITS: u32 = 16;
const MI_SIZE_LOG2: u32 = 2;
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

/// Per-frame filter state the inter block walk hands to the shared § 7.2
/// final filter chain.
pub(super) struct InterFilterInputs {
    pub(super) deblock_blocks: Vec<super::super::deblock::DeblockBlock>,
    pub(super) chroma_deblock_blocks: [Vec<super::super::deblock::DeblockBlock>; 2],
    pub(super) cdef_grid: super::super::cdef::CdefUnitGrid,
    pub(super) ccso_grid: Option<super::super::ccso::CcsoUnitGrid>,
    pub(super) lr_source_blocks: Vec<crate::tile_payload::WienerNsLrSourceBlock>,
    pub(super) lr_unit_filters: Vec<crate::tile_payload::WienerNsLrUnitFilter>,
}

/// Records § 7.17 deblock geometry for one inter block: per decoded transform
/// when residual was read, or the § 5.20.6.2 `Max_Tx_Size_Rect` tiling for a
/// skipped block (which reads no transform symbols).
#[allow(clippy::too_many_arguments)]
fn record_inter_deblock_geometry(
    deblock_blocks: &mut Vec<super::super::deblock::DeblockBlock>,
    chroma_deblock_blocks: &mut [Vec<super::super::deblock::DeblockBlock>; 2],
    frontier: &DecodeBlockFrontier,
    n4w: usize,
    n4h: usize,
    residual: Option<&InterResidual>,
    qindex: u32,
    tile_offset: ByteOffset,
) -> Result<()> {
    let Some(residual) = residual else {
        let tx_size = self::residual::max_tx_size(frontier.b_size.index(), tile_offset)?;
        let tx_w4 = self::residual::tx_size_dimension("Tx_Width", &TX_WIDTH, tx_size, tile_offset)?
            / MI_SIZE;
        let tx_h4 =
            self::residual::tx_size_dimension("Tx_Height", &TX_HEIGHT, tx_size, tile_offset)?
                / MI_SIZE;
        for row4 in (0..n4h).step_by(tx_h4.max(1)) {
            for col4 in (0..n4w).step_by(tx_w4.max(1)) {
                deblock_blocks.push(super::super::deblock::DeblockBlock {
                    r: frontier.r + row4,
                    c: frontier.c + col4,
                    n4w: tx_w4,
                    n4h: tx_h4,
                    luma_tx: tx_size,
                    chroma_tx:
                        super::super::wienerns_lr::fixed_largest_420_chroma_tx_size_from_luma_4x4(
                            tx_w4, tx_h4,
                        ),
                    qindex,
                    skip: true,
                });
            }
        }
        return Ok(());
    };
    for block in &residual.blocks {
        match block.plane {
            ReconPlaneId::Y => {
                let tx_w4 = (1usize << block.log2_width) / MI_SIZE;
                let tx_h4 = (1usize << block.log2_height) / MI_SIZE;
                deblock_blocks.push(super::super::deblock::DeblockBlock {
                    r: block.y / MI_SIZE,
                    c: block.x / MI_SIZE,
                    n4w: tx_w4,
                    n4h: tx_h4,
                    luma_tx: block.tx_size,
                    chroma_tx:
                        super::super::wienerns_lr::fixed_largest_420_chroma_tx_size_from_luma_4x4(
                            tx_w4, tx_h4,
                        ),
                    qindex,
                    skip: false,
                });
            }
            ReconPlaneId::U | ReconPlaneId::V => {
                if let Some((plane_index, record)) =
                    super::super::wienerns_lr::chroma_transform_deblock_block(
                        block.plane,
                        block.x,
                        block.y,
                        block.tx_size,
                        qindex,
                    )
                {
                    chroma_deblock_blocks[plane_index].push(record);
                }
            }
        }
    }
    Ok(())
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
) -> Result<(FrameCdfSubset, InterFilterInputs)> {
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
    let mut chroma_deblock_blocks: [Vec<super::super::deblock::DeblockBlock>; 2] =
        [Vec::new(), Vec::new()];
    let mut decoded_any = false;
    let mut ref_mv_bank = sequence
        .inter
        .as_ref()
        .is_some_and(|inter| inter.enable_refmvbank)
        .then(super::find_mv_stack::RefMvBank::new);
    let limits = options.limits();
    trace_symbol_frame_marker(offset);
    let walk = decode_general_intra_multiblock_tree_with_lr_source_blocks(
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
                &mut ref_mv_bank,
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
                &mut chroma_deblock_blocks,
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

    let crate::tile_payload::GeneralIntraMultiblockOutput {
        symbols,
        active_source_blocks,
        unit_filters,
    } = walk;
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
    let filter_inputs = InterFilterInputs {
        deblock_blocks,
        chroma_deblock_blocks,
        cdef_grid: cdef_state.into_grid(tile_offset)?,
        ccso_grid: ccso_state.into_grid(tile_offset)?,
        lr_source_blocks: active_source_blocks,
        lr_unit_filters: unit_filters,
    };
    Ok((tile.frame_cdfs(), filter_inputs))
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
    ref_mv_bank: &mut Option<super::find_mv_stack::RefMvBank>,
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
    block_decoded: &mut TileBlockDecodedState,
    workspace: &mut CurrentFrameWorkspace<T>,
    deblock_blocks: &mut Vec<super::super::deblock::DeblockBlock>,
    chroma_deblock_blocks: &mut [Vec<super::super::deblock::DeblockBlock>; 2],
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
        ref_frame1: None,
        mi_rows,
        mi_cols,
    };

    if let Some(bank) = ref_mv_bank.as_mut() {
        bank.reset_for_leaf(mv_grid, mi_row, mi_col, sb_h4);
    }
    let neighbour_ctx = block_neighbour_ctx(mv_grid, &block_ctx);

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
            record_inter_deblock_geometry(
                deblock_blocks,
                chroma_deblock_blocks,
                frontier,
                n4w,
                n4h,
                residual.as_ref(),
                block_qindex,
                tile_offset,
            )?;
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
                BlockPrecisionRecord::explicit(frame_mv_precision(core, tile_offset)?),
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
                BlockPrecisionRecord::explicit(frame_mv_precision(core, tile_offset)?),
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
    let uses_compound = if reference_select && is_comp_ref_allowed(n4w, n4h) {
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
        let pair = read_compound_reference_pair(
            cdfs,
            symbols,
            CompoundParseInput {
                num_total_refs,
                num_same_ref_compound,
                has_neighbour: neighbour_ctx.has_neighbour,
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
        block_ctx.ref_frame0 = pair.0;
        block_ctx.ref_frame1 = Some(pair.1);
        let mode_ctx = find_mode_ctx(mv_grid, &block_ctx);
        let compound = read_compound_mode_syntax(
            cdfs,
            symbols,
            pair,
            mode_ctx.new_mv_context,
            is_joint_ctx,
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
            BlockPrecisionRecord::most_probable(frame_mv_precision(core, tile_offset)?),
        );
        if let Some(bank) = ref_mv_bank.as_mut() {
            bank.update_for_block(
                compound.ref_frame0,
                Some(compound.ref_frame1),
                compound.mv0,
                Some(compound.mv1),
                mi_row,
                mi_col,
                n4w,
                n4h,
                sb_h4,
            );
        }
        reset_inter_skip_coeff_contexts(coeff_ctx, frontier, n4w, n4h, tile_offset)?;
        record_inter_deblock_geometry(
            deblock_blocks,
            chroma_deblock_blocks,
            frontier,
            n4w,
            n4h,
            None,
            block_qindex,
            tile_offset,
        )?;
        let placed = placed_block(InterBlock {
            ref_frame0: compound.ref_frame0,
            ref_frame1: Some(compound.ref_frame1),
            mv: compound.mv0,
            mv1: compound.mv1,
            interp,
            warp_params: None,
            bawp: BawpSyntax::default(),
            interintra: None,
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
            sequence_enables_ibp(sequence),
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
    let mode_ctx = find_mode_ctx(mv_grid, &block_ctx);
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
        let stack = find_mv_stack(
            mv_grid,
            &block_ctx,
            Mv::ZERO,
            ref_mv_bank
                .as_ref()
                .map(|bank| (bank, max_drl_bits_minus_1 as usize + 2)),
        );
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
        let motion_mode = if warp_mode == WarpInterMode::WarpNewmv {
            read_warp_newmv_motion_mode_syntax(
                cdfs,
                symbols,
                core,
                &neighbour_ctx,
                mode_ctx.warp_sample_found,
                tile_offset,
            )?
        } else {
            MotionMode::DeltaWarp
        };
        let warp = match (warp_mode, motion_mode) {
            (WarpInterMode::WarpNewmv, MotionMode::ExtendWarp | MotionMode::LocalWarp) => {
                read_warp_extend_syntax(
                    cdfs,
                    symbols,
                    sequence,
                    core,
                    &neighbour_ctx,
                    mv_config,
                    mv_grid,
                    &block_ctx,
                    &mode_ctx,
                    motion_mode,
                    mi_row,
                    mi_col,
                    n4w,
                    n4h,
                    &stack,
                    mode_ctx.new_mv_context,
                    max_drl_bits_minus_1,
                    tile_offset,
                )?
            }
            (WarpInterMode::WarpNewmv, _) => read_warp_newmv_delta_syntax(
                cdfs,
                symbols,
                sequence,
                core,
                &neighbour_ctx,
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
            (WarpInterMode::Warpmv, _) => read_warpmv_delta_syntax(
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
        let warp_inter_intra = if warp_mode == WarpInterMode::Warpmv {
            read_warp_inter_intra_syntax(
                cdfs,
                symbols,
                frontier.b_size.index(),
                n4w,
                n4h,
                tile_offset,
            )?
        } else {
            WarpInterIntraSyntax::default()
        };
        let warp_interintra_mode = interintra_prediction_mode(warp_inter_intra, tile_offset)?;
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
        record_inter_deblock_geometry(
            deblock_blocks,
            chroma_deblock_blocks,
            frontier,
            n4w,
            n4h,
            residual.as_ref(),
            delta_q_state.qindex_u32(),
            tile_offset,
        )?;
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
            NeighbourYMode::Other,
            warp.mv,
            skip == 1,
            interp_filter_symbol(ReconInterpolationFilter::EightTap),
            false,
            motion_mode,
            warp.warp_params,
            warp.block_precision,
        );
        if let Some(bank) = ref_mv_bank.as_mut() {
            bank.update_for_block(
                ref_frame0, None, warp.mv, None, mi_row, mi_col, n4w, n4h, sb_h4,
            );
        }
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
            interintra: warp_interintra_mode,
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
            sequence_enables_ibp(sequence),
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
    let bawp = if bawp.enabled {
        let slot = usize::try_from(ref_frame0)
            .ok()
            .and_then(|list_ref| ref_frame_idx.get(list_ref).copied())
            .unwrap_or(0);
        let ref_hint = reference
            .ref_order_hint
            .get(slot as usize)
            .copied()
            .map_or(0, |hint| i32::try_from(hint).unwrap_or(i32::MAX));
        BawpSyntax {
            ref_dist_gt4: super::get_relative_dist(ref_hint, current_order_hint as i32).abs() > 4,
            ..bawp
        }
    } else {
        bawp
    };
    if trace_first_row {
        eprintln!(
            "inter block bawp r={mi_row} c={mi_col} luma={} chroma={} allow={} frame_switch={} checkpoint={:?}",
            bawp.enabled,
            bawp.chroma,
            allow_bawp,
            frame_is_switch,
            symbols.checkpoint()
        );
    }
    read_inter_intra_flag_syntax(
        cdfs,
        symbols,
        core,
        frontier.b_size.index(),
        n4w,
        n4h,
        tile_offset,
    )?;
    let stack = find_mv_stack(
        mv_grid,
        &block_ctx,
        Mv::ZERO,
        ref_mv_bank
            .as_ref()
            .map(|bank| (bank, max_drl_bits_minus_1 as usize + 2)),
    );

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

    let frame_mv_config = inter_mv_read_config(core, tile_offset)?;
    let precision = read_block_mv_precision_syntax(
        cdfs,
        symbols,
        sequence,
        core,
        &neighbour_ctx,
        frame_mv_config.precision(),
        single_mode == SINGLE_MODE_NEWMV,
        use_amvd,
        tile_offset,
    )?;

    let pred_mv = stack.candidate(ref_mv_idx);
    let mv = match single_mode {
        SINGLE_MODE_GLOBALMV => Mv::ZERO,
        SINGLE_MODE_NEARMV => pred_mv,
        _ => {
            let config = MvReadConfig::inter(precision.mv_precision);
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
                        frame_mv_config,
                        config,
                    ),
                    1,
                )?
            };
            let pred_mv = if use_amvd {
                pred_mv
            } else {
                lowered_pred_mv(precision, pred_mv)
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
    record_inter_deblock_geometry(
        deblock_blocks,
        chroma_deblock_blocks,
        frontier,
        n4w,
        n4h,
        residual.as_ref(),
        delta_q_state.qindex_u32(),
        tile_offset,
    )?;
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
        precision,
    );
    if let Some(bank) = ref_mv_bank.as_mut() {
        bank.update_for_block(ref_frame0, None, mv, None, mi_row, mi_col, n4w, n4h, sb_h4);
    }
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
        interintra: None,
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
        sequence_enables_ibp(sequence),
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

/// AV2 § 5.20.7.10 `is_comp_ref_allowed()`: `Min(w, h) >= 8 ||
/// is_thin_4xn_nx4_block()`, in units of 4-sample mode-info columns/rows.
pub(super) fn is_comp_ref_allowed(n4w: usize, n4h: usize) -> bool {
    n4w.min(n4h) >= 2 || (n4w == 1 && n4h >= 4) || (n4h == 1 && n4w >= 4)
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

/// The § 5.3 `enable_ibp` sequence flag driving the § 7.13.2.12 IBP DC arm.
fn sequence_enables_ibp(sequence: &SequenceHeader) -> bool {
    sequence
        .intra
        .as_ref()
        .is_some_and(|intra| intra.enable_ibp)
}

/// One plane's § 5.20.7.22 `IntraPred` snapshot for the interintra blend.
struct InterIntraPlanePrediction<T> {
    plane: ReconPlaneId,
    x: usize,
    y: usize,
    size: IntraRectBlockSize,
    samples: Vec<T>,
}

/// Predicts the § 5.20.7.22 interintra intra predictor for every plane of the
/// block into caller-owned snapshots (edges are read from already-reconstructed
/// neighbours, so this runs before motion compensation overwrites the block).
/// Blocks whose chroma belongs to another leaf (§ 5.20.7.22 `sub8x8Inter`,
/// chroma-offset shared partitions) cannot reach this blend today: their
/// sub-8x8 chroma plane fail-closes in the warp small-block defer first. When
/// that defer lifts, U/V must be skipped unless the block carries its chroma.
fn predict_interintra_planes<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    placed: &PlacedInterBlock,
    mode: InterIntraMode,
    enable_ibp: bool,
    bit_depth: BitDepth,
    tile_offset: ByteOffset,
) -> Result<Vec<InterIntraPlanePrediction<T>>> {
    let geometry_error = || {
        inter_diag!(
            "inter_interintra_geometry",
            tile_offset,
            "invalid interintra plane geometry",
            "5.20.7.22"
        )
    };
    let mut planes = Vec::with_capacity(mc::YUV420_MC_PLANES.len());
    for (plane, sub_x, sub_y) in mc::YUV420_MC_PLANES {
        let x = placed.luma_x >> sub_x;
        let y = placed.luma_y >> sub_y;
        let w = placed.luma_w >> sub_x;
        let h = placed.luma_h >> sub_y;
        if !w.is_power_of_two() || !h.is_power_of_two() {
            return Err(geometry_error());
        }
        let log2_w = u8::try_from(w.trailing_zeros()).map_err(|_| geometry_error())?;
        let log2_h = u8::try_from(h.trailing_zeros()).map_err(|_| geometry_error())?;
        let size = IntraRectBlockSize::new(log2_w, log2_h).map_err(|_| geometry_error())?;
        let edges = workspace
            .intra_dc_edges_for_rect(plane, x, y, size)
            .map_err(|_| geometry_error())?;
        let mut samples = vec![T::default(); w * h];
        match mode {
            InterIntraMode::Dc => {
                let dc = predict_intra_dc_rect_value(bit_depth, size, edges.as_dc_edges())
                    .map_err(|_| geometry_error())?;
                samples.fill(dc);
                if enable_ibp && !(w == 4 && h == 4) {
                    apply_intra_ibp_dc_rect(bit_depth, size, edges.as_dc_edges(), &mut samples, w)
                        .map_err(|_| geometry_error())?;
                }
            }
            InterIntraMode::Vertical | InterIntraMode::Horizontal => {
                let (direction, edge) = if mode == InterIntraMode::Vertical {
                    (IntraCardinalDirection::Vertical, edges.above_samples())
                } else {
                    (IntraCardinalDirection::Horizontal, edges.left_samples())
                };
                let Some(edge) = edge else {
                    return Err(inter_cap!(
                        "inter_interintra_edge_unavailable",
                        tile_offset,
                        "inter.interintra.boundary_edge_synthesis",
                        "7.13.2.1"
                    ));
                };
                let prepared = if mode == InterIntraMode::Vertical {
                    IntraDirectionalAngleEdges::above(edge)
                } else {
                    IntraDirectionalAngleEdges::left(edge)
                };
                predict_intra_cardinal_directional_rect_into(
                    bit_depth,
                    size,
                    direction,
                    prepared,
                    &mut samples,
                    w,
                )
                .map_err(|_| geometry_error())?;
            }
            InterIntraMode::Smooth => {
                return Err(inter_cap!(
                    "inter_interintra_smooth_unimplemented",
                    tile_offset,
                    "inter.interintra.ii_smooth",
                    "7.13.3.29"
                ));
            }
        }
        planes.push(InterIntraPlanePrediction {
            plane,
            x,
            y,
            size,
            samples,
        });
    }
    Ok(planes)
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
    enable_ibp: bool,
    tile_offset: ByteOffset,
) -> Result<()> {
    let rect = mc::McBlockRect {
        luma_x: placed.luma_x,
        luma_y: placed.luma_y,
        luma_w: placed.luma_w,
        luma_h: placed.luma_h,
    };
    let intra_predictions = placed
        .block
        .interintra
        .map(|mode| {
            predict_interintra_planes(workspace, placed, mode, enable_ibp, bit_depth, tile_offset)
        })
        .transpose()?;
    let block_params =
        super::resolve_inter_block_params(ref_frame_idx, reference, placed, rect, tile_offset)?;
    mc::motion_compensate_inter_block_into(workspace, block_params, tile_offset)?;
    if placed.block.bawp.enabled {
        let slot = usize::try_from(placed.block.ref_frame0)
            .ok()
            .and_then(|list_ref| ref_frame_idx.get(list_ref).copied())
            .ok_or_else(|| {
                inter_missing!(
                    "inter_missing_reference_frame",
                    tile_offset,
                    "inter.bawp.reference_frame",
                    super::SPEC_REFERENCE
                )
            })?;
        let ref_frame = reference.frame_for_slot(slot).ok_or_else(|| {
            inter_missing!(
                "inter_missing_reference_frame",
                tile_offset,
                "inter.bawp.reference_frame",
                super::SPEC_REFERENCE
            )
        })?;
        super::bawp::apply_bawp(
            workspace,
            ref_frame,
            placed,
            placed.block.bawp,
            placed.block.mv,
            tile_offset,
        )?;
    }
    if let (Some(predictions), Some(mode)) = (intra_predictions, placed.block.interintra) {
        for prediction in predictions {
            workspace
                .blend_smooth_interintra_rect(
                    prediction.plane,
                    prediction.x,
                    prediction.y,
                    prediction.size,
                    mode,
                    &prediction.samples,
                )
                .map_err(|_| {
                    inter_diag!(
                        "inter_interintra_blend",
                        tile_offset,
                        "interintra blend failed",
                        "7.13.3.30"
                    )
                })?;
        }
    }
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

mod residual;

use self::residual::{
    read_inter_residual, reset_inter_skip_coeff_contexts, transform_tool_residual_policy,
};

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

/// § 5.18.2 `FrameMvPrecision` as a Table 6.19 code.
fn frame_mv_precision(core: &FrameHeaderCore, tile_offset: ByteOffset) -> Result<u8> {
    Ok(inter_mv_read_config(core, tile_offset)?.precision())
}

/// § 5.18.2 `UsePerBlockMvPrecision`: `enable_flex_mvres` outside the
/// `force_integer_mv` path (which pins `MV_PRECISION_ONE_PEL`).
fn use_per_block_mv_precision(sequence: &SequenceHeader, core: &FrameHeaderCore) -> bool {
    sequence
        .inter
        .as_ref()
        .is_some_and(|inter| inter.enable_flex_mvres)
        && !effective_force_integer_mv(core)
}

/// § 5.20.7.13 per-block MV precision: the `use_most_probable_precision` and
/// `pb_mv_precision` reads plus the `adjustedPrecision` derivation.
#[allow(clippy::too_many_arguments)]
fn read_block_mv_precision_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    neighbour_ctx: &BlockNeighbourContext,
    frame_precision: u8,
    block_has_newmv: bool,
    is_adaptive_mvd: bool,
    tile_offset: ByteOffset,
) -> Result<BlockPrecisionRecord> {
    if is_adaptive_mvd
        || !block_has_newmv
        || !use_per_block_mv_precision(sequence, core)
        || frame_precision < MV_PRECISION_HALF_PEL
    {
        return Ok(BlockPrecisionRecord::most_probable(frame_precision));
    }
    let use_most_probable = cdfs
        .read_block_symbol_trace(
            TileCdfSelector::UseMostProbablePrecision {
                ctx: neighbour_ctx.most_probable_precision_ctx(),
            },
            symbols,
        )
        .map_err(|_| symbol_read_error(tile_offset))?;
    if use_most_probable.get() != 0 {
        return Ok(BlockPrecisionRecord::most_probable(frame_precision));
    }
    let pb_mv_precision = cdfs
        .read_block_symbol_trace(
            TileCdfSelector::PbMvPrecision {
                ctx: neighbour_ctx.pb_mv_precision_ctx(frame_precision),
                frame_ctx: usize::from(frame_precision - MV_PRECISION_HALF_PEL),
            },
            symbols,
        )
        .map_err(|_| symbol_read_error(tile_offset))?;
    let adjusted = MV_PRECISION_ONE_PEL
        .max(frame_precision - 2)
        .checked_sub(pb_mv_precision.get())
        .filter(|&adjusted| adjusted > 0)
        .ok_or_else(|| symbol_read_error(tile_offset))?;
    let mv_precision = if adjusted <= MV_PRECISION_TWO_PEL {
        adjusted - 1
    } else {
        adjusted
    };
    Ok(BlockPrecisionRecord::explicit(mv_precision))
}

/// § 5.20.7.13 `assign_mv` predictor rounding: `lower_mv_precision` applies to
/// NEWMV-family predictors below `MV_PRECISION_HALF_PEL`.
fn lowered_pred_mv(precision: BlockPrecisionRecord, pred_mv: Mv) -> Mv {
    if precision.mv_precision < MV_PRECISION_HALF_PEL {
        lower_mv_precision(precision.mv_precision, pred_mv)
    } else {
        pred_mv
    }
}

/// § 5.20.7.14 `read_motion_mode` SIMPLE-path prefix: the § 5.20.7.15
/// `read_interintra_mode(0)` `inter_intra` flag, read for single-reference
/// non-warp blocks of 8x8..=64x64 when the frame enables the INTERINTRA
/// motion mode. Interintra prediction is beyond the current frontier, so a
/// set flag defers; reading it keeps entropy sync for flag == 0. The other
/// `motion_mode_allowed` bail-outs (skip_mode, BAWP, TIP/INTRA references,
/// segmentation features) are already frontier-rejected before this point.
fn read_inter_intra_flag_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    core: &FrameHeaderCore,
    b_size: usize,
    n4w: usize,
    n4h: usize,
    tile_offset: ByteOffset,
) -> Result<()> {
    let frame_enables_interintra = core
        .inter
        .as_ref()
        .and_then(|inter| inter.frame_enabled_motion_modes)
        .is_some_and(|modes| modes[splot_core::headers::frame::INTERINTRA]);
    if !frame_enables_interintra || b_size < BLOCK_8X8 || n4w.max(n4h) > CHUNK_64_N4 {
        return Ok(());
    }
    let bsize_group = *SIZE_GROUP_LOOKUP.get(b_size).ok_or_else(|| {
        inter_cap!(
            "inter_interintra_bsize_group",
            tile_offset,
            "inter.inter_intra block size out of range",
            SPEC_MODE_INFO
        )
    })?;
    let inter_intra = cdfs
        .read_block_symbol_trace(TileCdfSelector::InterIntra { bsize_group }, symbols)
        .map_err(|_| symbol_read_error(tile_offset))?;
    if inter_intra.get() != 0 {
        return Err(inter_cap!(
            "inter_interintra_unimplemented",
            tile_offset,
            "inter.inter_intra prediction",
            "5.20.7.15"
        ));
    }
    Ok(())
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

/// § 5.20.7.14 `read_motion_mode` WARP_NEWMV branch: the `use_extend_warp`
/// and `use_local_warp` reads, gated on § 7.11.4 `WarpSampleFound[ 0 ]` and
/// the frame-enabled motion modes. LOCALWARP prediction is beyond the
/// frontier, so that flag defers; otherwise the selected mode is returned.
fn read_warp_newmv_motion_mode_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    core: &FrameHeaderCore,
    neighbour_ctx: &BlockNeighbourContext,
    warp_sample_found: bool,
    tile_offset: ByteOffset,
) -> Result<MotionMode> {
    let frame_modes = core
        .inter
        .as_ref()
        .and_then(|inter| inter.frame_enabled_motion_modes)
        .unwrap_or([false; splot_core::headers::frame::MOTION_MODES]);
    let mut read_flag = |selector: TileCdfSelector| -> Result<bool> {
        let flag = cdfs
            .read_block_symbol_trace(selector, symbols)
            .map_err(|_| symbol_read_error(tile_offset))?;
        Ok(flag.get() != 0)
    };
    if warp_sample_found
        && frame_modes[splot_core::headers::frame::EXTENDWARP]
        && read_flag(TileCdfSelector::UseExtendWarp {
            ctx: neighbour_ctx.use_extend_warp_ctx(),
        })?
    {
        return Ok(MotionMode::ExtendWarp);
    }
    if warp_sample_found
        && frame_modes[splot_core::headers::frame::LOCALWARP]
        && read_flag(TileCdfSelector::UseLocalWarp {
            ctx: neighbour_ctx.use_local_warp_ctx(),
        })?
    {
        return Ok(MotionMode::LocalWarp);
    }
    Ok(MotionMode::DeltaWarp)
}

/// § 4.8 `Round2` over a signed § 7.13.3.24 projection difference: the
/// spec's integer form `(x + (1 << (n - 1))) >> n` with an arithmetic shift
/// (AVM `ROUND_POWER_OF_TWO_64`), NOT the sign-magnitude `Round2Signed`.
const fn warp_round2(value: i64, n: u32) -> i64 {
    if n == 0 {
        return value;
    }
    (value + (1i64 << (n - 1))) >> n
}

/// AV2 § 3 `LS_MV_MAX`.
const LS_MV_MAX: i64 = 256;

/// § 7.13.3.23 `ls_product`.
const fn ls_product(a: i64, b: i64) -> i64 {
    ((a * b) >> 2) + (a + b)
}

/// § 7.13.3.23 warp estimation: integer least-squares fit over the § 7.12.3
/// warp samples, falling back to a pure translation when the determinant is
/// zero.
#[allow(clippy::too_many_arguments)]
fn local_warp_estimation(
    samples: &[[i64; 4]],
    mv: Mv,
    mi_row: usize,
    mi_col: usize,
    n4w: usize,
    n4h: usize,
    tile_offset: ByteOffset,
) -> Result<[i64; 6]> {
    let geometry_error = || warp_model_error(tile_offset);
    let mid_y = i64::try_from(mi_row * 4 + n4h * 2).map_err(|_| geometry_error())? - 1;
    let mid_x = i64::try_from(mi_col * 4 + n4w * 2).map_err(|_| geometry_error())? - 1;
    let suy = mid_y * 8;
    let sux = mid_x * 8;
    let duy = suy + i64::from(mv.row);
    let dux = sux + i64::from(mv.col);
    let mut a = [[0i64; 2]; 2];
    let mut bx = [0i64; 2];
    let mut by = [0i64; 2];
    for sample in samples {
        let sy = sample[0] - suy;
        let sx = sample[1] - sux;
        let dy = sample[2] - duy;
        let dx = sample[3] - dux;
        if (sx - dx).abs() < LS_MV_MAX && (sy - dy).abs() < LS_MV_MAX {
            a[0][0] += ls_product(sx, sx) + 8;
            a[0][1] += ls_product(sx, sy) + 4;
            a[1][1] += ls_product(sy, sy) + 8;
            bx[0] += ls_product(sx, dx) + 8;
            bx[1] += ls_product(sy, dx) + 4;
            by[0] += ls_product(sx, dy) + 4;
            by[1] += ls_product(sy, dy) + 8;
        }
    }
    let det = a[0][0] * a[1][1] - a[0][1] * a[0][1];
    let mut params = IDENTITY_WARP_PARAMS;
    if det == 0 {
        set_warp_translation(&mut params, mv, mi_row, mi_col, n4w, n4h, tile_offset)?;
        return Ok(params);
    }
    let (raw_shift, factor) =
        splot_recon::resolve_divisor(det.unsigned_abs()).map_err(|_| geometry_error())?;
    let div_factor = if det < 0 {
        -i64::from(factor)
    } else {
        i64::from(factor)
    };
    let mut div_shift = i64::from(raw_shift) - i64::from(WARPEDMODEL_PREC_BITS);
    let mut div_factor = div_factor;
    if div_shift < 0 {
        div_factor <<= -div_shift;
        div_shift = 0;
    }
    let shift = u32::try_from(div_shift).map_err(|_| geometry_error())?;
    let diag = |v: i64| -> i64 {
        let product = i128::from(v) * i128::from(div_factor);
        let magnitude = product.unsigned_abs();
        let rounded = if shift == 0 {
            magnitude
        } else {
            (magnitude + (1u128 << (shift - 1))) >> shift
        };
        let signed = if product < 0 {
            -i128::try_from(rounded).unwrap_or(i128::MAX)
        } else {
            i128::try_from(rounded).unwrap_or(i128::MAX)
        };
        i64::try_from(signed.clamp(i128::from(i32::MIN), i128::from(i32::MAX))).unwrap_or_default()
    };
    params[2] = diag(a[1][1] * bx[0] - a[0][1] * bx[1]);
    params[3] = diag(-a[0][1] * bx[0] + a[0][0] * bx[1]);
    params[4] = diag(a[1][1] * by[0] - a[0][1] * by[1]);
    params[5] = diag(-a[0][1] * by[0] + a[0][0] * by[1]);
    reduce_warp_model(&mut params);
    set_warp_translation(&mut params, mv, mi_row, mi_col, n4w, n4h, tile_offset)?;
    Ok(params)
}

/// § 7.13.3.24 extend-warp estimation: extends the base neighbour's warp
/// model through the block's signalled MV. The global-motion `params` arm is
/// statically unreachable (global-motion frames defer at the frame gate).
#[allow(clippy::too_many_arguments)]
fn extend_warp_estimation(
    mv_grid: &NeighbourMvGrid,
    block_ctx: &MvBlockContext,
    mode_ctx: &ModeContext,
    stack: &super::find_mv_stack::MvStack,
    ref_mv_idx: usize,
    mv: Mv,
    mi_row: usize,
    mi_col: usize,
    n4w: usize,
    n4h: usize,
    tile_offset: ByteOffset,
) -> Result<[i64; 6]> {
    let (mut delta_row, mut delta_col) = stack.candidate_offsets(ref_mv_idx);
    if delta_row != -1 && delta_col != -1 {
        let Some((fallback_row, fallback_col)) = mode_ctx.extend_delta else {
            return Err(inter_cap!(
                "inter_warp_extend_base_missing",
                tile_offset,
                "inter.warp_extend.base_position",
                "7.13.3.24"
            ));
        };
        delta_row = fallback_row;
        delta_col = fallback_col;
    }
    let params = match super::find_mv_stack::extend_warp_neighbour_params(
        mv_grid, block_ctx, delta_row, delta_col,
    ) {
        super::find_mv_stack::ExtendWarpNeighbour::Params(params) => params,
        super::find_mv_stack::ExtendWarpNeighbour::List1MvUnretained => {
            return Err(inter_cap!(
                "inter_warp_extend_list1_mv_unretained",
                tile_offset,
                "inter.warp_extend.second_list_neighbour_mv",
                "7.13.3.24"
            ));
        }
        super::find_mv_stack::ExtendWarpNeighbour::Missing => {
            return Err(inter_cap!(
                "inter_warp_extend_base_missing",
                tile_offset,
                "inter.warp_extend.base_position",
                "7.13.3.24"
            ));
        }
    };
    let geometry_error = || warp_model_error(tile_offset);
    let mid_y = i64::try_from(mi_row * 4 + n4h * 2).map_err(|_| geometry_error())? - 1;
    let mid_x = i64::try_from(mi_col * 4 + n4w * 2).map_err(|_| geometry_error())? - 1;
    let proj_mid_x =
        (mid_x << WARPEDMODEL_PREC_BITS) + (i64::from(mv.col) << (WARPEDMODEL_PREC_BITS - 3));
    let proj_mid_y =
        (mid_y << WARPEDMODEL_PREC_BITS) + (i64::from(mv.row) << (WARPEDMODEL_PREC_BITS - 3));
    let mut extended = IDENTITY_WARP_PARAMS;
    extended[0] = 0;
    extended[1] = 0;
    let neighbour_is_above = delta_row == -1 && delta_col >= 0;
    if neighbour_is_above {
        extended[2] = params[2];
        extended[4] = params[4];
        let above_x = mid_x;
        let above_y = i64::try_from(mi_row * 4).map_err(|_| geometry_error())? - 1;
        let proj_above_x = params[2] * above_x + params[3] * above_y + params[0];
        let proj_above_y = params[4] * above_x + params[5] * above_y + params[1];
        let shift = n4h.trailing_zeros() + MI_SIZE_LOG2 - 1;
        extended[3] = warp_round2(proj_mid_x - proj_above_x, shift);
        extended[5] = warp_round2(proj_mid_y - proj_above_y, shift);
    } else {
        extended[3] = params[3];
        extended[5] = params[5];
        let left_x = i64::try_from(mi_col * 4).map_err(|_| geometry_error())? - 1;
        let left_y = mid_y;
        let proj_left_x = params[2] * left_x + params[3] * left_y + params[0];
        let proj_left_y = params[4] * left_x + params[5] * left_y + params[1];
        let shift = n4w.trailing_zeros() + MI_SIZE_LOG2 - 1;
        extended[2] = warp_round2(proj_mid_x - proj_left_x, shift);
        extended[4] = warp_round2(proj_mid_y - proj_left_y, shift);
    }
    reduce_warp_model(&mut extended);
    set_warp_translation(&mut extended, mv, mi_row, mi_col, n4w, n4h, tile_offset)?;
    Ok(extended)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ParsedWarpNewmv {
    mv: Mv,
    warp_params: [i64; 6],
    ref_mv_idx: usize,
    ref_warp_idx: usize,
    precision_idx: u8,
    warpmv_with_mvd: bool,
    block_precision: BlockPrecisionRecord,
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
    frame_config: MvReadConfig,
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
    frame_config.precision() <= MV_PRECISION_QUARTER_PEL
        && config.precision() < MV_PRECISION_QUARTER_PEL
}

/// § 5.20.7.13 EXTENDWARP / LOCALWARP tail: DRL, block precision, and the
/// NEWMV MVD — no `warp_idx` loop and no `read_warp_delta` — then the
/// mode's model derivation (§ 7.13.3.24 extension or § 7.13.3.23 least
/// squares over the § 7.12.3 warp samples).
#[allow(clippy::too_many_arguments)]
fn read_warp_extend_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    neighbour_ctx: &BlockNeighbourContext,
    mv_config: MvReadConfig,
    mv_grid: &NeighbourMvGrid,
    block_ctx: &MvBlockContext,
    mode_ctx: &ModeContext,
    motion_mode: MotionMode,
    mi_row: usize,
    mi_col: usize,
    n4w: usize,
    n4h: usize,
    stack: &super::find_mv_stack::MvStack,
    new_mv_context: usize,
    max_drl_bits_minus_1: u32,
    tile_offset: ByteOffset,
) -> Result<ParsedWarpNewmv> {
    let ref_mv_idx = read_drl_idx(
        cdfs,
        symbols,
        new_mv_context,
        max_drl_bits_minus_1,
        tile_offset,
    )?;
    let block_precision = read_block_mv_precision_syntax(
        cdfs,
        symbols,
        sequence,
        core,
        neighbour_ctx,
        mv_config.precision(),
        true,
        false,
        tile_offset,
    )?;
    let block_config = MvReadConfig::inter(block_precision.mv_precision);
    let pred_mv = lowered_pred_mv(block_precision, stack.candidate(ref_mv_idx));
    let magnitude = read_newmv_block_mvd_magnitude(cdfs, symbols, tile_offset, block_config)?;
    let diff = apply_inter_mvd_signs(magnitude, symbols, tile_offset, block_config, false, 1)?;
    let mv = Mv {
        row: mv_clamp_to_integer(pred_mv.row + diff.row),
        col: mv_clamp_to_integer(pred_mv.col + diff.col),
    };
    let warp_params = if motion_mode == MotionMode::LocalWarp {
        match super::find_mv_stack::find_warp_samples(mv_grid, block_ctx) {
            super::find_mv_stack::WarpSampleCollection::Samples(samples) => {
                local_warp_estimation(&samples, mv, mi_row, mi_col, n4w, n4h, tile_offset)?
            }
            super::find_mv_stack::WarpSampleCollection::List1MvUnretained => {
                return Err(inter_cap!(
                    "inter_warp_sample_list1_mv_unretained",
                    tile_offset,
                    "inter.local_warp.second_list_neighbour_mv",
                    "7.12.3.2"
                ));
            }
        }
    } else {
        extend_warp_estimation(
            mv_grid,
            block_ctx,
            mode_ctx,
            stack,
            ref_mv_idx,
            mv,
            mi_row,
            mi_col,
            n4w,
            n4h,
            tile_offset,
        )?
    };
    Ok(ParsedWarpNewmv {
        mv,
        warp_params,
        ref_mv_idx,
        ref_warp_idx: 0,
        precision_idx: 0,
        warpmv_with_mvd: false,
        block_precision,
    })
}

#[allow(clippy::too_many_arguments)]
fn read_warp_newmv_delta_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    neighbour_ctx: &BlockNeighbourContext,
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
    let block_precision = read_block_mv_precision_syntax(
        cdfs,
        symbols,
        sequence,
        core,
        neighbour_ctx,
        mv_config.precision(),
        true,
        false,
        tile_offset,
    )?;
    let block_config = MvReadConfig::inter(block_precision.mv_precision);
    let pred_mv = lowered_pred_mv(block_precision, stack.candidate(ref_mv_idx));
    let magnitude = read_newmv_block_mvd_magnitude(cdfs, symbols, tile_offset, block_config)?;
    let diff = apply_inter_mvd_signs(magnitude, symbols, tile_offset, block_config, false, 1)?;
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
        block_precision,
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
        block_precision: BlockPrecisionRecord::most_probable(mv_config.precision()),
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

/// Maps a parsed § 5.20.7.15 interintra read onto the supported prediction
/// subset: smooth-mask II_DC/II_V/II_H. Wedge interintra (§ 7.13.3.27 tables)
/// and II_SMOOTH stay fail-closed defers after the bit-exact parse.
fn interintra_prediction_mode(
    syntax: WarpInterIntraSyntax,
    tile_offset: ByteOffset,
) -> Result<Option<InterIntraMode>> {
    if !syntax.enabled {
        return Ok(None);
    }
    if syntax.use_wedge {
        return Err(inter_cap!(
            "inter_wedge_interintra_prediction_unimplemented",
            tile_offset,
            "inter.interintra.wedge_mask",
            "7.13.3.27"
        ));
    }
    match syntax.mode {
        Some(0) => Ok(Some(InterIntraMode::Dc)),
        Some(1) => Ok(Some(InterIntraMode::Vertical)),
        Some(2) => Ok(Some(InterIntraMode::Horizontal)),
        Some(3) => Err(inter_cap!(
            "inter_interintra_smooth_unimplemented",
            tile_offset,
            "inter.interintra.ii_smooth",
            "7.13.3.29"
        )),
        _ => Err(inter_cap!(
            "inter_interintra_mode_missing",
            tile_offset,
            "inter.interintra.mode",
            "5.20.7.15"
        )),
    }
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

    let list_index = explicit_bawp_context(input.single_mode, input.use_amvd);
    let explicit_bawp = cdfs
        .read_block_symbol_trace(TileCdfSelector::ExplicitBawp { ctx: list_index }, symbols)
        .map_err(|_| symbol_read_error(tile_offset))?;
    let explicit = explicit_bawp.get() != 0;
    let explicit_scale_positive = if explicit {
        cdfs.read_block_symbol_trace(TileCdfSelector::ExplicitBawpScale, symbols)
            .map_err(|_| symbol_read_error(tile_offset))?
            .get()
            != 0
    } else {
        false
    };
    let chroma = if input.has_chroma {
        cdfs.read_block_symbol_trace(TileCdfSelector::UseBawpChroma, symbols)
            .map_err(|_| symbol_read_error(tile_offset))?
            .get()
            != 0
    } else {
        false
    };

    Ok(BawpSyntax {
        enabled: true,
        explicit,
        explicit_scale_positive,
        list_index: list_index as u8,
        ref_dist_gt4: false,
        chroma,
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

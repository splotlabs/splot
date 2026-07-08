// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_recon::{
    BitDepth, CurrentFrameWorkspace, DpcmDirection, IntraCardinalDirection, PlaneId, ReconSample,
};

use super::*;
use crate::bitstream::tile_payload::{
    ActiveChromaResidualPolicy, ActiveIntraIstResidualPolicy, CflIndex, GeneralIntraBlockModes,
    GeneralIntraChromaBlockMode, GeneralIntraChromaModeContext, GeneralIntraChromaToolConfig,
    GeneralIntraLeafMode, IntraYMode, IsCflContext, LumaTransformPartitionContext,
    LumaTransformTypeContext, SupportedChromaMode, SupportedDirectionalLumaMode,
    SupportedNonDcLumaMode, TransformToolResidualPolicy,
};
use crate::prediction::intra::{IntraLumaUnsupported, plan_luma_prediction};
use crate::residual::pipeline::{
    GeneralIntraResidualPlan, RectChromaPlan, RectLumaPlan, ResidualPipelineUnsupported,
};
use crate::support::capability::missing_capability_message;
use crate::tile::block_context::{BlockCtx, BlockRect, ChromaSampling, TxShape};

pub(super) const FULL_SB_N4_LUMA: usize = 16;
const MI_SIZE: usize = 4;
const ANGLE_STEP: i32 = 3;
const MRL_INDEX_TO_DELTA: [i32; 4] = [0, 1, -1, 0];
const WAIP_WH_RATIO_THRESHOLDS: [(usize, i32); 4] = [(2, 61), (4, 73), (8, 82), (16, 86)];

macro_rules! general_intra_at {
    ($reason:expr, $offset:expr, $message:expr, $spec_section:expr $(,)?) => {
        general_intra_unsupported($reason, Some($offset), $message, $spec_section)
    };
}

fn general_intra_chroma_tools(
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
) -> GeneralIntraChromaToolConfig {
    sequence
        .intra
        .as_ref()
        .map_or(GeneralIntraChromaToolConfig::disabled(), |intra| {
            GeneralIntraChromaToolConfig::new(intra.enable_cfl_intra, intra.enable_mhccp)
                .with_enable_mrls(intra.enable_mrls)
        })
        .with_enable_idtx_intra(
            sequence
                .transform_quant_entropy
                .is_some_and(|tq| tq.enable_idtx_intra),
        )
        .with_allow_screen_content_tools(effective_allow_screen_content_tools(core))
}

pub(crate) fn general_intra_transform_tool_residual_policy(
    sequence: &SequenceHeader,
) -> TransformToolResidualPolicy {
    TransformToolResidualPolicy::from_sequence_tools(
        sequence,
        ActiveIntraIstResidualPolicy::LrTxSkipRecordHandoff,
        ActiveChromaResidualPolicy::LrTxSkipRecordHandoff,
    )
}

fn sequence_cfl_ds_filter_index(sequence: &SequenceHeader) -> u8 {
    sequence
        .intra
        .as_ref()
        .map_or(0, |intra| intra.cfl_ds_filter_index)
}

fn sequence_sb_mib(sequence: &SequenceHeader) -> usize {
    let sb_size = sequence.partition.as_ref().map_or(
        splot_core::headers::sequence::SuperblockSize::Block64x64,
        splot_core::headers::sequence::SequencePartitionConfig::seq_sb_size,
    );
    splot_core::tile::num_4x4_blocks_wide(sb_size) as usize
}

fn chroma_edge_smoothness(
    grid: Option<&crate::prediction::intra_edge::TileChromaSmoothGrid>,
    block_ctx: BlockCtx,
) -> (bool, bool) {
    let chroma = block_ctx.plane_block(PlaneId::U);
    grid.map_or((false, false), |grid| {
        grid.block_smoothness(chroma.x() / MI_SIZE, chroma.y() / MI_SIZE)
    })
}

fn record_chroma_smooth(
    grid: Option<&mut crate::prediction::intra_edge::TileChromaSmoothGrid>,
    block_ctx: BlockCtx,
    mode: Option<SupportedChromaMode>,
) {
    let Some(grid) = grid else {
        return;
    };
    let chroma = block_ctx.plane_block(PlaneId::U);
    grid.record(
        chroma.y() / MI_SIZE,
        chroma.x() / MI_SIZE,
        chroma.width4(),
        chroma.height4(),
        mode.is_some_and(SupportedChromaMode::is_smooth),
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn decode_one_general_intra_block<T: ReconSample>(
    work_unit: &mut crate::bitstream::tile_payload::DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    frontier: &crate::bitstream::tile_payload::DecodeBlockFrontier,
    sequence: &SequenceHeader,
    y_smooth: Option<&crate::prediction::intra_edge::TileYSmoothGrid>,
    mut chroma_smooth: Option<&mut crate::prediction::intra_edge::TileChromaSmoothGrid>,
    core: &FrameHeaderCore,
    joint_modes: &crate::bitstream::tile_payload::TileIntraJointModeState,
    uses_mrls: &crate::bitstream::tile_payload::TileUsesMrlsState,
    fsc_modes: &crate::bitstream::tile_payload::TileFscModeState,
    palette_state: &crate::bitstream::tile_payload::TileLumaPaletteState,
    is_cfl_ctx: IsCflContext,
    segment_id: u8,
    block_decoded: &mut crate::bitstream::tile_payload::TileBlockDecodedState,
    workspace: &mut CurrentFrameWorkspace<T>,
    coeff_ctx: &mut crate::bitstream::tile_payload::TileCoeffContextState,
    deblock_blocks: &mut Vec<crate::filters::deblock::DeblockBlock>,
    chroma_deblock_blocks: &mut [Vec<crate::filters::deblock::DeblockBlock>; 2],
    tx_skip_records: &mut Vec<crate::filters::wienerns_lr::WienerNsLrTxSkipTransformRecord>,
    qindex: u32,
    luma_use_tcq: bool,
    transform_tool_residual_policy: TransformToolResidualPolicy,
    mi_cols: usize,
    mi_rows: usize,
    bit_depth: BitDepth,
    tile_offset: ByteOffset,
) -> Result<GeneralIntraLeafMode> {
    let _qm_segment_scope =
        crate::bitstream::tile_payload::FrameQmSegmentScope::install(usize::from(segment_id));
    let geometry_error = || {
        general_intra_at!(
            "general_intra_block_geometry",
            tile_offset,
            "general intra block geometry lookup failed",
            GENERAL_INTRA_PARTITION_SPEC_SECTION,
        )
    };
    let n4w = frontier
        .b_size
        .num_4x4_wide()
        .map_err(|_| geometry_error())?;
    let n4h = frontier
        .b_size
        .num_4x4_high()
        .map_err(|_| geometry_error())?;
    let Some(block_tx_shape) = TxShape::from_luma_4x4(n4w, n4h) else {
        return Err(geometry_error());
    };
    let chroma_sampling =
        ChromaSampling::from_chroma_format_idc(sequence.general.chroma_format_idc);
    let mi_row_range = work_unit.mi_row_range();
    let mi_col_range = work_unit.mi_col_range();
    let mut block_ctx = BlockCtx::new(
        BlockRect::new(frontier.r, frontier.c, n4w, n4h),
        block_tx_shape,
        mi_cols,
        mi_rows,
        bit_depth,
        chroma_sampling,
    )
    .with_tile_bounds(
        mi_row_range.start as usize,
        mi_row_range.end as usize,
        mi_col_range.start as usize,
        mi_col_range.end as usize,
    );
    let chroma_mode_geometry = if frontier.has_chroma {
        let chroma_ref = frontier.chroma_ref_geometry();
        let chroma_n4w = chroma_ref
            .size()
            .num_4x4_wide()
            .map_err(|_| geometry_error())?;
        let chroma_n4h = chroma_ref
            .size()
            .num_4x4_high()
            .map_err(|_| geometry_error())?;
        let Some(chroma_tx_shape) = TxShape::from_luma_4x4(chroma_n4w, chroma_n4h) else {
            return Err(geometry_error());
        };
        block_ctx = block_ctx.with_chroma_ref(
            BlockRect::new(chroma_ref.row(), chroma_ref.col(), chroma_n4w, chroma_n4h),
            chroma_tx_shape,
        );
        Some((chroma_ref.size().index(), chroma_n4w, chroma_n4h))
    } else {
        None
    };
    let (above_smooth, left_smooth) = y_smooth.map_or((false, false), |grid| {
        grid.block_smoothness(frontier.c, frontier.r)
    });
    let (chroma_above_smooth, chroma_left_smooth) =
        chroma_edge_smoothness(chroma_smooth.as_deref(), block_ctx);
    let intra_edge = crate::prediction::intra_edge::IntraEdgeCtx {
        enable_ibp: sequence
            .intra
            .as_ref()
            .is_some_and(|intra| intra.enable_ibp),
        enable_intra_edge_filter: sequence
            .intra
            .as_ref()
            .is_some_and(|intra| intra.enable_intra_edge_filter),
        above_smooth,
        left_smooth,
        chroma_above_smooth,
        chroma_left_smooth,
    };
    let lossless = work_unit
        .coeff_frame_facts()
        .lossless_for_segment(usize::from(segment_id))
        .unwrap_or(false);
    let chroma_tools = general_intra_chroma_tools(sequence, core).with_lossless(lossless);
    let cfl_ds_filter_index = sequence_cfl_ds_filter_index(sequence);
    let sb_mib = sequence_sb_mib(sequence);

    if frontier.is_chroma_part() {
        return decode_one_general_intra_chroma_part_block::<T>(
            intra_edge,
            work_unit,
            symbols,
            frontier,
            chroma_tools,
            is_cfl_ctx,
            cfl_ds_filter_index,
            sb_mib,
            lossless,
            block_decoded,
            workspace,
            coeff_ctx,
            deblock_blocks,
            chroma_deblock_blocks,
            tx_skip_records,
            chroma_smooth.as_deref_mut(),
            qindex,
            transform_tool_residual_policy,
            block_ctx,
            tile_offset,
        );
    }

    let trace_bits = crate::trace_flags::trace_flag!("SPLOT_TRACE_GENERAL_INTRA_BITS");
    let mode_start_bits = symbols.consumed_bits().get();
    let luma_only = frontier.is_luma_part() || !frontier.has_chroma;
    let use_neighbor_fsc_context = core.frame_is_intra == Some(true) || !frontier.is_mixed_region();
    let modes = if luma_only {
        let luma = crate::bitstream::tile_payload::decode_general_intra_luma_block_mode_with_fsc_context(
            work_unit,
            symbols,
            chroma_tools,
            joint_modes,
            uses_mrls,
            fsc_modes,
            use_neighbor_fsc_context,
            frontier.b_size.index(),
            frontier.r,
            frontier.c,
            n4w,
            n4h,
        )
        .map_err(|error| {
            if trace_bits || crate::trace_flags::trace_flag!("SPLOT_TRACE_GENERAL_INTRA_CHROMA_MODE") {
                eprintln!(
                    "general intra block mode error block=({},{} {}x{}) bits={}..{} error={error:?}",
                    frontier.r,
                    frontier.c,
                    n4w,
                    n4h,
                    mode_start_bits,
                    symbols.consumed_bits().get()
                );
            }
            general_intra_block_mode_error(error, tile_offset)
        })?;
        let palette_y = crate::bitstream::tile_payload::read_general_intra_palette_y_mode(
            work_unit,
            symbols,
            chroma_tools,
            palette_state,
            luma.y_mode,
            frontier.b_size.index(),
            frontier.r,
            frontier.c,
            n4w,
            n4h,
            u32::from(bit_depth.bits()),
        )
        .map_err(|error| {
            if trace_bits || crate::trace_flags::trace_flag!("SPLOT_TRACE_GENERAL_INTRA_CHROMA_MODE") {
                eprintln!(
                    "general intra palette mode error block=({},{} {}x{}) bits={}..{} error={error:?}",
                    frontier.r,
                    frontier.c,
                    n4w,
                    n4h,
                    mode_start_bits,
                    symbols.consumed_bits().get()
                );
            }
            general_intra_block_mode_error(error, tile_offset)
        })?;
        GeneralIntraBlockModes::luma_only(luma).with_palette_y(palette_y)
    } else {
        let (chroma_block_size_index, chroma_n4w, chroma_n4h) =
            chroma_mode_geometry.unwrap_or((frontier.b_size.index(), n4w, n4h));
        crate::bitstream::tile_payload::decode_general_intra_block_modes_with_fsc_context(
            work_unit,
            symbols,
            chroma_tools,
            joint_modes,
            uses_mrls,
            fsc_modes,
            use_neighbor_fsc_context,
            palette_state,
            is_cfl_ctx.get(),
            frontier.b_size.index(),
            frontier.r,
            frontier.c,
            n4w,
            n4h,
            chroma_block_size_index,
            chroma_n4w,
            chroma_n4h,
            u32::from(bit_depth.bits()),
        )
        .map_err(|error| {
            if trace_bits || crate::trace_flags::trace_flag!("SPLOT_TRACE_GENERAL_INTRA_CHROMA_MODE") {
                eprintln!(
                    "general intra block mode error block=({},{} {}x{}) bits={}..{} error={error:?}",
                    frontier.r,
                    frontier.c,
                    n4w,
                    n4h,
                    mode_start_bits,
                    symbols.consumed_bits().get()
                );
            }
            general_intra_block_mode_error(error, tile_offset)
        })?
    };
    if trace_bits {
        eprintln!(
            "general intra block mode block=({},{} {}x{}) bits={}..{} modes={modes:?}",
            frontier.r,
            frontier.c,
            n4w,
            n4h,
            mode_start_bits,
            symbols.consumed_bits().get()
        );
    }
    let rect_mrl_admitted = rect_luma_plan(&modes, block_ctx, luma_use_tcq, sb_mib).is_ok();
    if modes.uses_active_mrl() && !rect_mrl_admitted {
        return Err(general_intra_at!(
            "general_intra_unsupported_mrl_mode",
            tile_offset,
            missing_capability_message!("intra.luma.mrl", mode = "active"),
            "7.13.2",
        ));
    }
    if lossless && modes.uses_active_fsc() {
        return Err(general_intra_at!(
            "general_intra_lossless_fsc_unverified",
            tile_offset,
            missing_capability_message!("intra.lossless.fsc", mode = "active"),
            "5.20.6.1",
        ));
    }
    ensure_lossless_verified_prediction_subset(
        lossless,
        frontier.has_chroma,
        &modes,
        block_ctx,
        sb_mib,
        tile_offset,
    )?;
    if luma_only {
        ensure_10bit_general_intra_luma_capability(
            &modes,
            block_ctx,
            sb_mib,
            lossless,
            tile_offset,
        )?;
    } else {
        ensure_10bit_general_intra_capability(
            &modes,
            block_ctx,
            (mi_cols, mi_rows),
            sb_mib,
            lossless,
            tile_offset,
        )?;
    }

    if n4w != n4h
        || modes.uses_active_mrl()
        || square_luma_needs_rect_residual_path(&modes, block_ctx, luma_use_tcq, sb_mib, lossless)
    {
        let leaf = decode_one_general_intra_rect_block::<T>(
            intra_edge,
            work_unit,
            symbols,
            frontier.has_chroma,
            &modes,
            workspace,
            coeff_ctx,
            deblock_blocks,
            chroma_deblock_blocks,
            tx_skip_records,
            qindex,
            luma_use_tcq,
            lossless,
            transform_tool_residual_policy,
            luma_tx_partition_context(frame_tx_mode(core), frontier.b_size.index(), lossless),
            block_ctx,
            block_decoded,
            cfl_ds_filter_index,
            sb_mib,
            tile_offset,
        )?;
        if frontier.has_chroma {
            record_chroma_smooth(chroma_smooth, block_ctx, modes.supported_chroma_mode());
        }
        return Ok(leaf);
    }

    let chroma_plan = if frontier.has_chroma {
        Some(
            chroma_plan_for_modes(&modes, block_ctx, cfl_ds_filter_index, sb_mib, lossless)
                .map_err(|error| general_intra_chroma_capability_error(error, tile_offset))?,
        )
    } else {
        None
    };
    let luma_plan = plan_luma_prediction_for_segment(&modes, block_ctx, lossless, sb_mib)
        .map_err(|error| general_intra_luma_plan_error(error, tile_offset))?;

    let residual_plan = GeneralIntraResidualPlan::square(
        block_ctx,
        luma_plan,
        chroma_plan,
        luma_use_tcq,
        modes.uses_active_fsc(),
        lossless,
    )
    .map_err(|error| general_intra_residual_plan_error(error, tile_offset))?;
    execute_general_intra_residual_plan(
        residual_plan,
        work_unit,
        symbols,
        coeff_ctx,
        workspace,
        block_ctx,
        block_decoded,
        deblock_blocks,
        chroma_deblock_blocks,
        tx_skip_records,
        modes.coeff_uv_mode(),
        luma_transform_type_context(&modes),
        luma_tx_partition_context(frame_tx_mode(core), frontier.b_size.index(), lossless),
        transform_tool_residual_policy,
        qindex,
        intra_edge,
        tile_offset,
    )?;
    if frontier.has_chroma {
        record_chroma_smooth(chroma_smooth, block_ctx, modes.supported_chroma_mode());
    }
    Ok(leaf_mode_for_block(&modes, frontier.has_chroma))
}

#[allow(clippy::too_many_arguments)]
fn decode_one_general_intra_chroma_part_block<T: ReconSample>(
    intra_edge: crate::prediction::intra_edge::IntraEdgeCtx,
    work_unit: &mut crate::bitstream::tile_payload::DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    frontier: &crate::bitstream::tile_payload::DecodeBlockFrontier,
    chroma_tools: GeneralIntraChromaToolConfig,
    is_cfl_ctx: IsCflContext,
    cfl_ds_filter_index: u8,
    sb_mib: usize,
    lossless: bool,
    block_decoded: &mut crate::bitstream::tile_payload::TileBlockDecodedState,
    workspace: &mut CurrentFrameWorkspace<T>,
    coeff_ctx: &mut crate::bitstream::tile_payload::TileCoeffContextState,
    deblock_blocks: &mut Vec<crate::filters::deblock::DeblockBlock>,
    chroma_deblock_blocks: &mut [Vec<crate::filters::deblock::DeblockBlock>; 2],
    tx_skip_records: &mut Vec<crate::filters::wienerns_lr::WienerNsLrTxSkipTransformRecord>,
    chroma_smooth: Option<&mut crate::prediction::intra_edge::TileChromaSmoothGrid>,
    qindex: u32,
    transform_tool_residual_policy: TransformToolResidualPolicy,
    block_ctx: BlockCtx,
    tile_offset: ByteOffset,
) -> Result<GeneralIntraLeafMode> {
    let (y_mode, angle_delta_y) = frontier
        .stored_luma_y_mode()
        .zip(frontier.stored_luma_angle_delta_y())
        .ok_or_else(|| {
            general_intra_at!(
                "general_intra_missing_sdp_luma_mode",
                tile_offset,
                "SDP chroma decode requires the collocated luma-part mode facts (an intraBC luma part records DC_PRED)",
                GENERAL_INTRA_MODE_SPEC_SECTION,
            )
        })?;
    let trace_bits = crate::trace_flags::trace_flag!("SPLOT_TRACE_GENERAL_INTRA_BITS");
    let mode_start_bits = symbols.consumed_bits().get();
    let chroma = crate::bitstream::tile_payload::decode_general_intra_chroma_block_mode(
        work_unit,
        symbols,
        chroma_tools,
        GeneralIntraChromaModeContext::sdp_chroma_part(
            frontier.cfl_allowed_in_sdp(),
            is_cfl_ctx.get(),
        ),
        y_mode,
        frontier.b_size.index(),
        block_ctx.block().width4(),
        block_ctx.block().height4(),
    )
    .map_err(|error| {
        if trace_bits || crate::trace_flags::trace_flag!("SPLOT_TRACE_GENERAL_INTRA_CHROMA_MODE") {
            eprintln!(
                "general intra chroma part mode error block=({},{} {}x{}) bits={}..{} error={error:?}",
                frontier.r,
                frontier.c,
                block_ctx.block().width4(),
                block_ctx.block().height4(),
                mode_start_bits,
                symbols.consumed_bits().get()
            );
        }
        general_intra_block_mode_error(error, tile_offset)
    })?;
    if lossless
        && !lossless_chroma_part_prediction_verified(
            chroma.supported_chroma_mode(y_mode),
            chroma.uses_dpcm_uv(),
            y_mode,
            block_ctx,
            sb_mib,
        )
    {
        return Err(general_intra_at!(
            "general_intra_lossless_nondc_chroma_part_unverified",
            tile_offset,
            missing_capability_message!("intra.lossless.chroma_prediction", mode = "non_dc"),
            "7.13.2",
        ));
    }
    if trace_bits {
        eprintln!(
            "general intra chroma part mode block=({},{} {}x{}) bits={}..{} mode={chroma:?}",
            frontier.r,
            frontier.c,
            block_ctx.block().width4(),
            block_ctx.block().height4(),
            mode_start_bits,
            symbols.consumed_bits().get()
        );
    }
    let chroma_plan = chroma_plan_for_parts(
        chroma,
        y_mode,
        angle_delta_y,
        block_ctx,
        cfl_ds_filter_index,
        sb_mib,
        lossless,
    )
    .map_err(|error| general_intra_chroma_capability_error(error, tile_offset))?;
    let residual_plan = GeneralIntraResidualPlan::chroma(block_ctx, chroma_plan)
        .map_err(|error| general_intra_residual_plan_error(error, tile_offset))?;
    execute_general_intra_residual_plan(
        residual_plan,
        work_unit,
        symbols,
        coeff_ctx,
        workspace,
        block_ctx,
        block_decoded,
        deblock_blocks,
        chroma_deblock_blocks,
        tx_skip_records,
        chroma.coeff_uv_mode(),
        LumaTransformTypeContext::new(y_mode, angle_delta_y),
        None,
        transform_tool_residual_policy,
        qindex,
        intra_edge,
        tile_offset,
    )?;
    record_chroma_smooth(
        chroma_smooth,
        block_ctx,
        chroma.supported_chroma_mode(y_mode),
    );
    Ok(GeneralIntraLeafMode::chroma(chroma.is_cfl()))
}

#[allow(clippy::too_many_arguments)]
fn decode_one_general_intra_rect_block<T: ReconSample>(
    intra_edge: crate::prediction::intra_edge::IntraEdgeCtx,
    work_unit: &mut crate::bitstream::tile_payload::DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    has_chroma: bool,
    modes: &GeneralIntraBlockModes,
    workspace: &mut CurrentFrameWorkspace<T>,
    coeff_ctx: &mut crate::bitstream::tile_payload::TileCoeffContextState,
    deblock_blocks: &mut Vec<crate::filters::deblock::DeblockBlock>,
    chroma_deblock_blocks: &mut [Vec<crate::filters::deblock::DeblockBlock>; 2],
    tx_skip_records: &mut Vec<crate::filters::wienerns_lr::WienerNsLrTxSkipTransformRecord>,
    qindex: u32,
    luma_use_tcq: bool,
    lossless: bool,
    transform_tool_residual_policy: TransformToolResidualPolicy,
    luma_tx_partition_context: Option<LumaTransformPartitionContext>,
    block_ctx: BlockCtx,
    block_decoded: &mut crate::bitstream::tile_payload::TileBlockDecodedState,
    cfl_ds_filter_index: u8,
    sb_mib: usize,
    tile_offset: ByteOffset,
) -> Result<GeneralIntraLeafMode> {
    let luma_plan = rect_luma_plan(modes, block_ctx, luma_use_tcq, sb_mib)
        .map_err(|error| general_intra_luma_plan_error(error, tile_offset))?;
    let chroma_plan = if has_chroma {
        Some(
            rect_chroma_plan(modes, block_ctx, cfl_ds_filter_index, sb_mib)
                .map_err(|error| general_intra_chroma_capability_error(error, tile_offset))?,
        )
    } else {
        None
    };

    let residual_plan = GeneralIntraResidualPlan::rect(
        block_ctx,
        luma_plan,
        chroma_plan,
        modes.uses_active_fsc(),
        lossless,
    )
    .map_err(|error| general_intra_residual_plan_error(error, tile_offset))?;
    execute_general_intra_residual_plan(
        residual_plan,
        work_unit,
        symbols,
        coeff_ctx,
        workspace,
        block_ctx,
        block_decoded,
        deblock_blocks,
        chroma_deblock_blocks,
        tx_skip_records,
        modes.coeff_uv_mode(),
        luma_transform_type_context(modes),
        luma_tx_partition_context,
        transform_tool_residual_policy,
        qindex,
        intra_edge,
        tile_offset,
    )?;
    Ok(leaf_mode_for_block(modes, has_chroma))
}

fn leaf_mode(modes: &GeneralIntraBlockModes) -> GeneralIntraLeafMode {
    GeneralIntraLeafMode::luma(
        modes.intra_joint_mode,
        modes.y_mode,
        modes.angle_delta_y,
        modes.fsc_mode,
        modes.uses_mrls,
    )
    .with_palette_y(modes.palette_y())
}

fn leaf_mode_for_block(modes: &GeneralIntraBlockModes, has_chroma: bool) -> GeneralIntraLeafMode {
    let leaf = leaf_mode(modes);
    if has_chroma {
        leaf.with_uv_cfl(modes.is_cfl())
    } else {
        leaf
    }
}

pub(super) fn ensure_lossless_verified_prediction_subset(
    lossless: bool,
    has_chroma: bool,
    modes: &GeneralIntraBlockModes,
    block_ctx: BlockCtx,
    sb_mib: usize,
    tile_offset: ByteOffset,
) -> Result<()> {
    if !lossless {
        return Ok(());
    }
    if !lossless_luma_prediction_verified(modes, block_ctx, sb_mib) {
        return Err(general_intra_at!(
            "general_intra_lossless_other_nondc_luma_unverified",
            tile_offset,
            missing_capability_message!("intra.lossless.luma_prediction", mode = "non_dc"),
            "7.13.2",
        ));
    }
    if has_chroma
        && !lossless_chroma_block_prediction_verified(
            modes.supported_chroma_mode(),
            modes.uses_dpcm_uv(),
            block_ctx,
            sb_mib,
        )
    {
        return Err(general_intra_at!(
            "general_intra_lossless_other_nondc_chroma_block_unverified",
            tile_offset,
            missing_capability_message!("intra.lossless.chroma_prediction", mode = "non_dc"),
            "7.13.2",
        ));
    }
    Ok(())
}

fn lossless_luma_prediction_verified(
    modes: &GeneralIntraBlockModes,
    block_ctx: BlockCtx,
    sb_mib: usize,
) -> bool {
    use SupportedDirectionalLumaMode as L;

    if modes.luma_is_dc() || modes.uses_dpcm_y() {
        return true;
    }
    let block = block_ctx.block();
    let full_64_sb_8bit = block_ctx.bit_depth() == BitDepth::Eight
        && sb_mib == FULL_SB_N4_LUMA
        && block.width4() == FULL_SB_N4_LUMA
        && block.height4() == FULL_SB_N4_LUMA;
    let directional = modes.y_mode.supported_directional();
    let top_left_directional = block_ctx.is_top_left()
        && matches!(
            directional,
            Some(L::Vertical | L::Horizontal | L::D45 | L::D135)
        );
    let top_left_paeth = block_ctx.is_top_left() && modes.y_mode.is_paeth();
    let y_neighbours = block_ctx.neighbours(PlaneId::Y);
    let left_edge_d45_or_d113 = !y_neighbours.has_above()
        && y_neighbours.has_left()
        && (directional == Some(L::D45)
            || (directional == Some(L::D113)
                && modes.supported_chroma_mode() == Some(SupportedChromaMode::D113Follow))
            || (directional == Some(L::D135)
                && modes.supported_chroma_mode() == Some(SupportedChromaMode::D135Follow))
            || (directional == Some(L::D157)
                && modes.supported_chroma_mode() == Some(SupportedChromaMode::D157Follow)));
    full_64_sb_8bit
        && modes.angle_delta_y == 0
        && (top_left_directional || top_left_paeth || left_edge_d45_or_d113)
}
fn top_left_no_neighbour_directional_prediction_verified(
    modes: &GeneralIntraBlockModes,
    block_ctx: BlockCtx,
    sb_mib: usize,
) -> bool {
    let block = block_ctx.block();
    let top_left_full_sb = block_ctx.bit_depth() == BitDepth::Eight
        && sb_mib == FULL_SB_N4_LUMA
        && block_ctx.is_top_left()
        && block.width4() == FULL_SB_N4_LUMA
        && block.height4() == FULL_SB_N4_LUMA;
    top_left_full_sb
        && matches!(
            (modes.y_mode.supported_directional(), modes.angle_delta_y),
            (Some(SupportedDirectionalLumaMode::Vertical), 2)
                | (Some(SupportedDirectionalLumaMode::Horizontal), 0)
        )
}
fn plan_luma_prediction_for_segment(
    modes: &GeneralIntraBlockModes,
    block_ctx: BlockCtx,
    lossless: bool,
    sb_mib: usize,
) -> core::result::Result<crate::prediction::intra::IntraLumaPlan, IntraLumaUnsupported> {
    if (lossless && lossless_luma_prediction_verified(modes, block_ctx, sb_mib))
        || (!lossless
            && top_left_no_neighbour_directional_prediction_verified(modes, block_ctx, sb_mib))
    {
        plan_luma_prediction(modes, block_ctx, true)
    } else {
        plan_luma_prediction(modes, block_ctx, false)
    }
}

pub(super) fn lossless_chroma_prediction_verified(
    mode: Option<SupportedChromaMode>,
    uses_dpcm_uv: bool,
) -> bool {
    matches!(mode, Some(SupportedChromaMode::Dc))
        || (uses_dpcm_uv
            && matches!(
                mode,
                Some(SupportedChromaMode::Vertical | SupportedChromaMode::Horizontal)
            ))
}

pub(super) fn lossless_chroma_part_prediction_verified(
    mode: Option<SupportedChromaMode>,
    uses_dpcm_uv: bool,
    y_mode: IntraYMode,
    block_ctx: BlockCtx,
    sb_mib: usize,
) -> bool {
    if lossless_chroma_prediction_verified(mode, uses_dpcm_uv) {
        return true;
    }
    !uses_dpcm_uv
        && lossless_chroma_full_64_block(block_ctx)
        && sb_mib == FULL_SB_N4_LUMA
        && block_ctx.is_top_left()
        && y_mode == IntraYMode::DC_PRED
        && matches!(
            mode,
            Some(
                SupportedChromaMode::Vertical
                    | SupportedChromaMode::Horizontal
                    | SupportedChromaMode::D45
                    | SupportedChromaMode::D135
                    | SupportedChromaMode::Paeth
            )
        )
}

pub(super) fn lossless_chroma_block_prediction_verified(
    mode: Option<SupportedChromaMode>,
    uses_dpcm_uv: bool,
    block_ctx: BlockCtx,
    sb_mib: usize,
) -> bool {
    use SupportedChromaMode as M;

    if lossless_chroma_prediction_verified(mode, uses_dpcm_uv) {
        return true;
    }
    if uses_dpcm_uv || !lossless_chroma_full_64_block(block_ctx) {
        return false;
    }
    let Some(mode) = mode else {
        return false;
    };
    let neighbours = block_ctx.neighbours(PlaneId::U);
    ((sb_mib == FULL_SB_N4_LUMA && block_ctx.is_top_left())
        && matches!(
            mode,
            M::Horizontal | M::Vertical | M::D45 | M::D135 | M::Paeth
        ))
        || (!neighbours.has_above()
            && neighbours.has_left()
            && (matches!(
                mode,
                M::D45 | M::D45Follow | M::D113 | M::D113Follow | M::D135Follow
            ) || matches!(mode, M::D157 | M::D157Follow | M::D203 | M::D203Follow)))
}

fn lossless_chroma_full_64_block(block_ctx: BlockCtx) -> bool {
    let block = block_ctx.block();
    block_ctx.bit_depth() == BitDepth::Eight
        && block_ctx.chroma() == ChromaSampling::Yuv420
        && (block.width4(), block.height4()) == (FULL_SB_N4_LUMA, FULL_SB_N4_LUMA)
}

fn luma_transform_type_context(modes: &GeneralIntraBlockModes) -> LumaTransformTypeContext {
    LumaTransformTypeContext::with_mrl_indices(
        modes.y_mode,
        modes.angle_delta_y,
        modes.mrl_index,
        modes.mrl_sec_index,
        modes.luma_dpcm_direction(),
    )
}

fn luma_tx_partition_context(
    tx_mode: Option<TxMode>,
    block_size_index: usize,
    lossless: bool,
) -> Option<LumaTransformPartitionContext> {
    if tx_mode != Some(TxMode::Select) || lossless {
        return None;
    }
    Some(LumaTransformPartitionContext::new(block_size_index))
}

fn frame_tx_mode(core: &FrameHeaderCore) -> Option<TxMode> {
    core.intra_tail
        .as_ref()
        .map(|tail| tail.tx_mode)
        .or_else(|| core.inter_tail.as_ref().map(|tail| tail.tx_mode))
}

fn square_luma_needs_rect_residual_path(
    modes: &GeneralIntraBlockModes,
    block_ctx: BlockCtx,
    use_tcq: bool,
    sb_mib: usize,
    lossless: bool,
) -> bool {
    let block = block_ctx.block();
    block.width4() == block.height4()
        && plan_luma_prediction_for_segment(modes, block_ctx, lossless, sb_mib).is_err()
        && rect_luma_plan(modes, block_ctx, use_tcq, sb_mib).is_ok()
}

fn rect_luma_plan(
    modes: &GeneralIntraBlockModes,
    block_ctx: BlockCtx,
    use_tcq: bool,
    sb_mib: usize,
) -> core::result::Result<RectLumaPlan, IntraLumaUnsupported> {
    if let Some(palette) = modes.palette_y() {
        return Ok(RectLumaPlan::Palette { palette, use_tcq });
    }
    if modes.uses_active_mrl() {
        return rect_luma_mrl_plan(modes, block_ctx, use_tcq, sb_mib);
    }
    let directional_p_angle = rect_luma_directional_p_angle(modes, block_ctx);
    rect_luma_plan_for_parts_ext(
        modes.y_mode.is_paeth(),
        modes.supported_nondc_luma(),
        directional_p_angle,
        modes.luma_is_dc(),
        block_ctx,
        use_tcq,
    )
    .or_else(|error| {
        rect_luma_middle_left_only_plan(modes.y_mode, directional_p_angle, block_ctx, use_tcq)
            .ok_or(error)
    })
}

fn rect_luma_mrl_plan(
    modes: &GeneralIntraBlockModes,
    block_ctx: BlockCtx,
    use_tcq: bool,
    sb_mib: usize,
) -> core::result::Result<RectLumaPlan, IntraLumaUnsupported> {
    rect_luma_mrl_plan_for_parts(
        modes.y_mode,
        modes.angle_delta_y,
        modes.mrl_index,
        modes.mrl_sec_index,
        block_ctx,
        use_tcq,
        sb_mib,
    )
}

fn rect_luma_mrl_plan_for_parts(
    y_mode: IntraYMode,
    angle_delta_y: i8,
    mrl_index: u8,
    mrl_sec_index: Option<u8>,
    block_ctx: BlockCtx,
    use_tcq: bool,
    sb_mib: usize,
) -> core::result::Result<RectLumaPlan, IntraLumaUnsupported> {
    let nominal = y_mode.mode_to_angle().ok_or_else(unsupported_rect_luma)?;
    let mrl_index = usize::from(mrl_index);
    let mrl_delta = *MRL_INDEX_TO_DELTA
        .get(mrl_index)
        .ok_or_else(unsupported_rect_luma)?;
    let block = block_ctx.block();
    let width = block.width4().saturating_mul(MI_SIZE);
    let height = block.height4().saturating_mul(MI_SIZE);
    let nominal_angle = i32::from(nominal) + i32::from(angle_delta_y) * ANGLE_STEP + mrl_delta;
    let p_angle = wide_angle_mapped_p_angle(width, height, nominal_angle);
    let neighbours = block_ctx.neighbours(PlaneId::Y);
    let is_sb_boundary = sb_mib != 0 && block.row4().is_multiple_of(sb_mib);
    let above_mrl_index = if is_sb_boundary { 0 } else { mrl_index };
    let secondary_mrl = mrl_sec_index == Some(1) && !(width == MI_SIZE && height == MI_SIZE);
    if p_angle == 90 && neighbours.has_above() {
        return Ok(RectLumaPlan::CardinalMrl {
            direction: IntraCardinalDirection::Vertical,
            mrl_index,
            above_mrl_index,
            secondary_mrl,
            use_tcq,
        });
    }
    if p_angle == 180 && neighbours.has_left() {
        return Ok(RectLumaPlan::CardinalMrl {
            direction: IntraCardinalDirection::Horizontal,
            mrl_index,
            above_mrl_index,
            secondary_mrl,
            use_tcq,
        });
    }
    if p_angle > 0 && p_angle < 90 && neighbours.has_above() {
        let p_angle = u16::try_from(p_angle).map_err(|_| unsupported_rect_luma())?;
        return Ok(RectLumaPlan::OneSidedAboveMrl {
            p_angle,
            mrl_index,
            above_mrl_index,
            secondary_mrl,
            use_tcq,
        });
    }
    if p_angle > 180 && p_angle < 270 && neighbours.has_left() {
        let p_angle = u16::try_from(p_angle).map_err(|_| unsupported_rect_luma())?;
        return Ok(RectLumaPlan::OneSidedLeftMrl {
            p_angle,
            mrl_index,
            secondary_mrl,
            use_tcq,
        });
    }
    let top_row_left = !neighbours.has_above() && neighbours.has_left();
    if !(90 < p_angle
        && p_angle < 180
        && (neighbours.has_above() && neighbours.has_left() || top_row_left))
    {
        return Err(unsupported_rect_luma());
    }
    let p_angle = u16::try_from(p_angle).map_err(|_| unsupported_rect_luma())?;
    Ok(RectLumaPlan::MiddleMrl {
        p_angle,
        mrl_index,
        above_mrl_index,
        is_sb_boundary,
        secondary_mrl,
        use_tcq,
    })
}

#[allow(dead_code)]
fn rect_luma_plan_for_parts(
    nondc: Option<SupportedNonDcLumaMode>,
    directional_p_angle: Option<u16>,
    luma_is_dc: bool,
    block_ctx: BlockCtx,
    use_tcq: bool,
) -> core::result::Result<RectLumaPlan, IntraLumaUnsupported> {
    rect_luma_plan_for_parts_ext(
        false,
        nondc,
        directional_p_angle,
        luma_is_dc,
        block_ctx,
        use_tcq,
    )
}

/// Plans a rectangular (or square-as-part) non-DC luma leaf. Smooth modes are
/// admitted for any neighbour configuration because § 7.13.2.1 always derives
/// `LeftCol`/`AboveRow` (real reconstructed neighbours or the `haveLeft` /
/// `haveAbove` fallbacks), so smooth prediction is defined even with no edge;
/// directional and one-sided modes still require the edge they read.
fn rect_luma_plan_for_parts_ext(
    luma_is_paeth: bool,
    nondc: Option<SupportedNonDcLumaMode>,
    directional_p_angle: Option<u16>,
    luma_is_dc: bool,
    block_ctx: BlockCtx,
    use_tcq: bool,
) -> core::result::Result<RectLumaPlan, IntraLumaUnsupported> {
    if luma_is_dc {
        return Ok(RectLumaPlan::Dc { use_tcq });
    }
    let block = block_ctx.block();
    let supported_rect = block.width4() >= 8 && block.height4() >= 8;
    let supported_cardinal_rect = block.width4() >= 1 && block.height4() >= 1;
    let supported_middle_rect = block.width4() >= 1 && block.height4() >= 1;
    let supported_one_sided_above_rect = block.width4() >= 1 && block.height4() >= 1;
    let supported_one_sided_left_rect = block.width4() >= 1 && block.height4() >= 1;
    let neighbours = block_ctx.neighbours(PlaneId::Y);
    if luma_is_paeth {
        return Ok(RectLumaPlan::Paeth { use_tcq });
    }
    if let Some(mode) = nondc {
        return Ok(RectLumaPlan::Smooth { mode, use_tcq });
    }
    let has_edge = neighbours.has_above() || neighbours.has_left();
    match directional_p_angle {
        Some(90) if supported_cardinal_rect && has_edge => {
            return Ok(RectLumaPlan::Cardinal {
                direction: IntraCardinalDirection::Vertical,
                use_tcq,
            });
        }
        Some(180) if supported_cardinal_rect && has_edge => {
            return Ok(RectLumaPlan::Cardinal {
                direction: IntraCardinalDirection::Horizontal,
                use_tcq,
            });
        }
        Some(p_angle @ 91..=179)
            if supported_middle_rect && neighbours.has_above() && neighbours.has_left() =>
        {
            return Ok(RectLumaPlan::Middle { p_angle, use_tcq });
        }
        _ => {}
    }
    if let Some(p_angle @ 1..=89) = directional_p_angle
        && ((neighbours.has_above() && supported_one_sided_above_rect)
            || (!neighbours.has_above() && supported_one_sided_above_rect && neighbours.has_left()))
    {
        return Ok(RectLumaPlan::OneSidedAbove { p_angle, use_tcq });
    }
    match directional_p_angle {
        Some(p_angle @ 181..=269)
            if neighbours.has_left() && (supported_rect || supported_one_sided_left_rect) =>
        {
            return Ok(RectLumaPlan::OneSidedLeft { p_angle, use_tcq });
        }
        _ => {}
    }
    Err(unsupported_rect_luma())
}

fn rect_luma_middle_left_only_plan(
    y_mode: IntraYMode,
    directional_p_angle: Option<u16>,
    block_ctx: BlockCtx,
    use_tcq: bool,
) -> Option<RectLumaPlan> {
    let p_angle = directional_p_angle?;
    if !(91..=179).contains(&p_angle) {
        return None;
    }
    let mode = y_mode.supported_directional()?;
    let neighbours = block_ctx.neighbours(PlaneId::Y);
    let top_row_left = !neighbours.has_above() && neighbours.has_left();
    if !top_row_left {
        return None;
    }
    let base_p_angle = directional_p_angle_for_luma(y_mode, 0, block_ctx)?;
    let needs_delta_or_d113 =
        p_angle != base_p_angle || matches!(mode, SupportedDirectionalLumaMode::D113);
    needs_delta_or_d113.then_some(RectLumaPlan::Middle { p_angle, use_tcq })
}

fn rect_luma_directional_p_angle(
    modes: &GeneralIntraBlockModes,
    block_ctx: BlockCtx,
) -> Option<u16> {
    directional_p_angle_for_luma(modes.y_mode, modes.angle_delta_y, block_ctx)
}

fn directional_p_angle_for_luma(
    y_mode: IntraYMode,
    angle_delta_y: i8,
    block_ctx: BlockCtx,
) -> Option<u16> {
    let base = i32::from(y_mode.mode_to_angle()?);
    let angle = base.checked_add(i32::from(angle_delta_y) * ANGLE_STEP)?;
    let block = block_ctx.block();
    let width = block.width4().checked_mul(4)?;
    let height = block.height4().checked_mul(4)?;
    u16::try_from(wide_angle_mapped_p_angle(width, height, angle)).ok()
}

fn rect_chroma_plan(
    modes: &GeneralIntraBlockModes,
    block_ctx: BlockCtx,
    cfl_ds_filter_index: u8,
    sb_mib: usize,
) -> core::result::Result<RectChromaPlan, ChromaCapabilityUnsupported> {
    if modes.is_cfl() {
        return cfl_chroma_plan(
            modes.cfl_params(),
            "general_intra_rect_cfl_missing_params",
            cfl_ds_filter_index,
            sb_mib,
        );
    }
    let mode = modes.supported_chroma_mode().ok_or(unsupported_chroma(
        "general_intra_rect_non_dc_chroma",
        missing_capability_message!("intra.rect.chroma_mode", mode = "unsupported_non_dc"),
    ))?;
    if let Err(error) = ensure_supported_rect_chroma_capability(mode, block_ctx) {
        if crate::trace_flags::trace_flag!("SPLOT_TRACE_GENERAL_INTRA_MODE") {
            let neighbours = block_ctx.neighbours(PlaneId::U);
            eprintln!(
                "rect chroma rejected block={:?} mode={mode:?} modes={modes:?} uv_neigh=(above:{} left:{} ar:{} bl:{})",
                block_ctx.block(),
                neighbours.has_above(),
                neighbours.has_left(),
                neighbours.num_above_right(),
                neighbours.num_below_left()
            );
        }
        return Err(error);
    }
    Ok(rect_chroma_plan_for_mode(
        mode,
        modes.angle_delta_y,
        modes.chroma_dpcm_direction(),
        block_ctx,
    ))
}

fn chroma_plan_for_modes(
    modes: &GeneralIntraBlockModes,
    block_ctx: BlockCtx,
    cfl_ds_filter_index: u8,
    sb_mib: usize,
    lossless: bool,
) -> core::result::Result<RectChromaPlan, ChromaCapabilityUnsupported> {
    if modes.is_cfl() {
        return cfl_chroma_plan(
            modes.cfl_params(),
            "general_intra_cfl_missing_params",
            cfl_ds_filter_index,
            sb_mib,
        );
    }
    let mode = modes.supported_chroma_mode().ok_or(unsupported_chroma(
        "general_intra_non_dc_chroma",
        missing_capability_message!("intra.chroma.mode", mode = "unsupported_non_dc"),
    ))?;
    if let Err(error) =
        ensure_supported_chroma_capability(mode, modes.chroma_dpcm_direction(), block_ctx)
        && !(lossless
            && lossless_chroma_block_prediction_verified(
                Some(mode),
                modes.uses_dpcm_uv(),
                block_ctx,
                sb_mib,
            ))
    {
        return Err(error);
    }
    Ok(rect_chroma_plan_for_mode(
        mode,
        modes.angle_delta_y,
        modes.chroma_dpcm_direction(),
        block_ctx,
    ))
}

fn rect_chroma_plan_for_mode(
    mode: SupportedChromaMode,
    angle_delta_y: i8,
    dpcm: Option<DpcmDirection>,
    block_ctx: BlockCtx,
) -> RectChromaPlan {
    let Some((base, inherits_luma_delta)) = mode.directional_base_angle() else {
        return RectChromaPlan::Mode(mode, dpcm);
    };
    let angle_delta = if inherits_luma_delta {
        angle_delta_y
    } else {
        0
    };
    let Some(angle) = base.checked_add(i32::from(angle_delta) * ANGLE_STEP) else {
        return RectChromaPlan::Mode(mode, dpcm);
    };
    let block = block_ctx.plane_block(PlaneId::U);
    let Some(width) = block.width4().checked_mul(4) else {
        return RectChromaPlan::Mode(mode, dpcm);
    };
    let Some(height) = block.height4().checked_mul(4) else {
        return RectChromaPlan::Mode(mode, dpcm);
    };
    let Ok(p_angle) = u16::try_from(wide_angle_mapped_p_angle(width, height, angle)) else {
        return RectChromaPlan::Mode(mode, dpcm);
    };
    match p_angle {
        1..=89 | 181..=270 => RectChromaPlan::OneSided { p_angle },
        91..=179 => RectChromaPlan::Middle { p_angle },
        _ => RectChromaPlan::Mode(mode, dpcm),
    }
}

fn chroma_plan_for_parts(
    chroma: GeneralIntraChromaBlockMode,
    y_mode: IntraYMode,
    angle_delta_y: i8,
    block_ctx: BlockCtx,
    cfl_ds_filter_index: u8,
    sb_mib: usize,
    lossless: bool,
) -> core::result::Result<RectChromaPlan, ChromaCapabilityUnsupported> {
    if chroma.is_cfl() {
        return cfl_chroma_plan(
            chroma.cfl_params(),
            "general_intra_chroma_part_cfl_missing_params",
            cfl_ds_filter_index,
            sb_mib,
        );
    }
    let mode = chroma
        .supported_chroma_mode(y_mode)
        .ok_or(unsupported_chroma(
            "general_intra_chroma_part_non_dc_chroma",
            missing_capability_message!("intra.chroma.mode", mode = "unsupported_non_dc"),
        ))?;
    if let Err(error) = ensure_supported_rect_chroma_capability(mode, block_ctx)
        && !(lossless
            && lossless_chroma_part_prediction_verified(
                Some(mode),
                chroma.uses_dpcm_uv(),
                y_mode,
                block_ctx,
                sb_mib,
            ))
    {
        return Err(error);
    }
    Ok(rect_chroma_plan_for_mode(
        mode,
        angle_delta_y,
        chroma.chroma_dpcm_direction(),
        block_ctx,
    ))
}

fn cfl_chroma_plan(
    params: Option<crate::bitstream::tile_payload::CflParams>,
    missing_reason: &'static str,
    cfl_ds_filter_index: u8,
    sb_mib: usize,
) -> core::result::Result<RectChromaPlan, ChromaCapabilityUnsupported> {
    let params = params.ok_or(unsupported_chroma(
        missing_reason,
        missing_capability_message!("intra.chroma.cfl", mode = "missing_params"),
    ))?;
    let supported = match params.index {
        CflIndex::Explicit | CflIndex::DerivedAlpha => true,
        CflIndex::Multi => params.mh_dir.is_some_and(|dir| dir <= 2),
    };
    if !supported {
        return Err(unsupported_chroma(
            "general_intra_cfl_non_multi",
            missing_capability_message!("intra.chroma.cfl", mode = "unsupported_params"),
        ));
    }
    Ok(RectChromaPlan::Cfl {
        params,
        cfl_ds_filter_index,
        sb_mib,
    })
}

pub(crate) fn wide_angle_mapped_p_angle(width: usize, height: usize, p_angle: i32) -> i32 {
    if WAIP_WH_RATIO_THRESHOLDS
        .iter()
        .any(|&(scale, threshold)| height == width.saturating_mul(scale) && p_angle < threshold)
    {
        180 + p_angle
    } else if WAIP_WH_RATIO_THRESHOLDS.iter().any(|&(scale, threshold)| {
        width == height.saturating_mul(scale) && p_angle > 270 - threshold
    }) {
        p_angle - 180
    } else {
        p_angle
    }
}

const fn unsupported_rect_luma() -> IntraLumaUnsupported {
    IntraLumaUnsupported::new(
        "general_intra_rect_non_dc_luma",
        missing_capability_message!("intra.rect.luma_mode", mode = "non_dc"),
    )
}

fn ensure_10bit_general_intra_luma_capability(
    modes: &GeneralIntraBlockModes,
    block_ctx: BlockCtx,
    sb_mib: usize,
    lossless: bool,
    tile_offset: ByteOffset,
) -> Result<()> {
    if block_ctx.bit_depth() == BitDepth::Eight {
        return Ok(());
    }
    let luma_admitted = modes.luma_is_dc()
        || plan_luma_prediction_for_segment(modes, block_ctx, lossless, sb_mib).is_ok()
        || rect_luma_plan(modes, block_ctx, false, sb_mib).is_ok();
    if luma_admitted {
        return Ok(());
    }
    Err(general_intra_at!(
        "unsupported_10bit_non_dc_intra",
        tile_offset,
        missing_capability_message!("intra.10bit.non_dc", luma = "non_dc",),
        GENERAL_INTRA_MODE_SPEC_SECTION,
    ))
}

fn ensure_10bit_general_intra_capability(
    modes: &GeneralIntraBlockModes,
    block_ctx: BlockCtx,
    frame_n4: (usize, usize),
    sb_mib: usize,
    lossless: bool,
    tile_offset: ByteOffset,
) -> Result<()> {
    if block_ctx.bit_depth() == BitDepth::Eight {
        return Ok(());
    }
    let chroma_admitted = if modes.is_cfl() {
        modes.cfl_params().is_some_and(|params| match params.index {
            CflIndex::Explicit | CflIndex::DerivedAlpha => true,
            CflIndex::Multi => params.mh_dir.is_some_and(|dir| dir <= 2),
        })
    } else {
        ten_bit_general_intra_chroma_admitted(modes.supported_chroma_mode(), block_ctx)
    };
    let luma_admitted = modes.luma_is_dc()
        || plan_luma_prediction_for_segment(modes, block_ctx, lossless, sb_mib).is_ok()
        || rect_luma_plan(modes, block_ctx, false, sb_mib).is_ok();
    if !luma_admitted || !chroma_admitted {
        if crate::trace_flags::trace_flag!("SPLOT_TRACE_GENERAL_INTRA_MODE") {
            let luma_plan = plan_luma_prediction_for_segment(modes, block_ctx, lossless, sb_mib);
            let chroma_mode = modes.supported_chroma_mode();
            let y_neighbours = block_ctx.neighbours(PlaneId::Y);
            let uv_neighbours = block_ctx.neighbours(PlaneId::U);
            eprintln!(
                "general intra rejected block={:?} bit_depth={:?} modes={modes:?} luma_plan={luma_plan:?} chroma_mode={chroma_mode:?} chroma_admitted={chroma_admitted} frame_n4={frame_n4:?} y_neigh=(above:{} left:{} ar:{} bl:{}) uv_neigh=(above:{} left:{} ar:{} bl:{})",
                block_ctx.block(),
                block_ctx.bit_depth(),
                y_neighbours.has_above(),
                y_neighbours.has_left(),
                y_neighbours.num_above_right(),
                y_neighbours.num_below_left(),
                uv_neighbours.has_above(),
                uv_neighbours.has_left(),
                uv_neighbours.num_above_right(),
                uv_neighbours.num_below_left()
            );
        }
        return Err(general_intra_at!(
            "unsupported_10bit_non_dc_intra_chroma",
            tile_offset,
            missing_capability_message!("intra.10bit.non_dc", luma = "non_dc_or_chroma_neighbour",),
            GENERAL_INTRA_MODE_SPEC_SECTION,
        ));
    }
    Ok(())
}

fn ten_bit_general_intra_chroma_admitted(
    mode: Option<SupportedChromaMode>,
    block_ctx: BlockCtx,
) -> bool {
    let neighbours = block_ctx.neighbours(PlaneId::U);
    let has_edge = neighbours.has_above() || neighbours.has_left();
    let full_sb = block_ctx.block().width4() == FULL_SB_N4_LUMA;
    let chroma_block = block_ctx.plane_block(PlaneId::U);
    let chroma_smooth_shape = chroma_block.width4() >= 1 && chroma_block.height4() >= 1;
    let chroma_cardinal_shape = chroma_block.width4() >= 1 && chroma_block.height4() >= 1;
    let chroma_middle_shape = chroma_block.width4() >= 1 && chroma_block.height4() >= 1;
    let chroma_one_sided_above_shape = chroma_block.width4() >= 1 && chroma_block.height4() >= 1;
    let one_sided_above_available = (neighbours.has_above() && neighbours.num_above_right() > 0)
        || (!neighbours.has_above() && neighbours.has_left());
    let no_neighbour_horizontal_first = mode == Some(SupportedChromaMode::Horizontal)
        && block_ctx.block().width4() == FULL_SB_N4_LUMA
        && block_ctx.block().height4() == FULL_SB_N4_LUMA
        && block_ctx.is_top_left();
    match mode {
        Some(
            SupportedChromaMode::Dc | SupportedChromaMode::D203Follow | SupportedChromaMode::D203,
        ) => true,
        Some(
            SupportedChromaMode::Smooth
            | SupportedChromaMode::SmoothVertical
            | SupportedChromaMode::SmoothHorizontal,
        ) => full_sb || (chroma_smooth_shape && has_edge),
        Some(SupportedChromaMode::Vertical | SupportedChromaMode::VerticalFollow) => {
            chroma_cardinal_shape && has_edge
        }
        Some(SupportedChromaMode::Horizontal | SupportedChromaMode::HorizontalFollow) => {
            chroma_cardinal_shape && (has_edge || no_neighbour_horizontal_first)
        }
        Some(
            SupportedChromaMode::D113Follow
            | SupportedChromaMode::D113
            | SupportedChromaMode::D135Follow
            | SupportedChromaMode::D135
            | SupportedChromaMode::D157Follow
            | SupportedChromaMode::D157,
        ) => chroma_middle_shape && has_edge,
        Some(
            SupportedChromaMode::D45Follow
            | SupportedChromaMode::D45
            | SupportedChromaMode::D67Follow
            | SupportedChromaMode::D67,
        ) => chroma_one_sided_above_shape && one_sided_above_available,
        Some(SupportedChromaMode::Paeth) => {
            chroma_smooth_shape && (neighbours.has_above() || neighbours.has_left())
        }
        _ => false,
    }
}

#[allow(clippy::needless_pass_by_value, clippy::too_many_arguments)]
fn execute_general_intra_residual_plan<T: ReconSample>(
    residual_plan: GeneralIntraResidualPlan,
    work_unit: &mut crate::bitstream::tile_payload::DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    coeff_ctx: &mut crate::bitstream::tile_payload::TileCoeffContextState,
    workspace: &mut CurrentFrameWorkspace<T>,
    block_ctx: BlockCtx,
    block_decoded: &mut crate::bitstream::tile_payload::TileBlockDecodedState,
    deblock_blocks: &mut Vec<crate::filters::deblock::DeblockBlock>,
    chroma_deblock_blocks: &mut [Vec<crate::filters::deblock::DeblockBlock>; 2],
    tx_skip_records: &mut Vec<crate::filters::wienerns_lr::WienerNsLrTxSkipTransformRecord>,
    uv_mode: usize,
    luma_transform_type_context: LumaTransformTypeContext,
    luma_tx_partition_context: Option<LumaTransformPartitionContext>,
    transform_tool_residual_policy: TransformToolResidualPolicy,
    qindex: u32,
    intra_edge: crate::prediction::intra_edge::IntraEdgeCtx,
    tile_offset: ByteOffset,
) -> Result<()> {
    let block = block_ctx.block();
    let transforms = residual_plan.transforms();
    let mut deblock = crate::residual::pipeline::DeblockRecorder {
        blocks: deblock_blocks,
        chroma_blocks: chroma_deblock_blocks,
        tx_skip_records,
        block_r: block.row4(),
        block_c: block.col4(),
        block_w4: block.width4(),
        block_h4: block.height4(),
        luma_tx: transforms.luma_tx(),
        chroma_tx: transforms.chroma_tx(),
        qindex,
    };
    residual_plan
        .execute(
            work_unit,
            symbols,
            coeff_ctx,
            workspace,
            block_decoded,
            uv_mode,
            luma_transform_type_context,
            luma_tx_partition_context,
            transform_tool_residual_policy,
            qindex,
            intra_edge,
            &mut deblock,
        )
        .map_err(|error| general_intra_residual_error(error, tile_offset))?;
    Ok(())
}

#[derive(Clone, Copy)]
struct ChromaCapabilityUnsupported {
    reason_id: &'static str,
    message: &'static str,
}

fn ensure_supported_chroma_capability(
    mode: SupportedChromaMode,
    dpcm: Option<DpcmDirection>,
    block_ctx: BlockCtx,
) -> core::result::Result<(), ChromaCapabilityUnsupported> {
    let n4w = block_ctx.block().width4();
    let neighbours = block_ctx.neighbours(PlaneId::U);
    let chroma_block = block_ctx.plane_block(PlaneId::U);
    let full_sb = n4w == FULL_SB_N4_LUMA;
    let has_edge = neighbours.has_above() || neighbours.has_left();
    let above_left = full_sb && neighbours.has_above() && neighbours.has_left();
    let left_only = full_sb && !neighbours.has_above() && neighbours.has_left();
    let smooth_subblock = chroma_block.width4() >= 1 && chroma_block.height4() >= 1;
    let cardinal_subblock = chroma_block.width4() >= 1 && chroma_block.height4() >= 1;
    let paeth_subblock = chroma_block.width4() >= 1 && chroma_block.height4() >= 1;
    let middle_subblock = chroma_block.width4() >= 1 && chroma_block.height4() >= 1;
    let one_sided_above_subblock = chroma_block.width4() >= 1
        && chroma_block.height4() >= 1
        && (neighbours.has_above() || neighbours.has_left()); // § 7.13.2.1 clamps the above-right read to the frame edge (`aboveLimit`), so a right-edge subblock with no above-right is admissible like the cardinal path
    match mode {
        SupportedChromaMode::Dc | SupportedChromaMode::D203Follow | SupportedChromaMode::D203 => {
            Ok(())
        }
        SupportedChromaMode::Smooth
        | SupportedChromaMode::SmoothVertical
        | SupportedChromaMode::SmoothHorizontal
            if full_sb || smooth_subblock =>
        {
            Ok(())
        }
        SupportedChromaMode::Smooth
        | SupportedChromaMode::SmoothVertical
        | SupportedChromaMode::SmoothHorizontal => Err(unsupported_chroma(
            "general_intra_smooth_chroma_subblock",
            missing_capability_message!(
                "intra.chroma.smooth",
                neighbour = "above_right_below_left",
                block = "subpartition",
            ),
        )),
        SupportedChromaMode::D135Follow
        | SupportedChromaMode::D135
        | SupportedChromaMode::Horizontal
            if full_sb && neighbours.is_top_left() =>
        {
            Ok(())
        }
        SupportedChromaMode::D135Follow
        | SupportedChromaMode::D135
        | SupportedChromaMode::D157Follow
        | SupportedChromaMode::D157
            if left_only || above_left || (middle_subblock && has_edge) =>
        {
            Ok(())
        }
        SupportedChromaMode::D135Follow | SupportedChromaMode::D135 => Err(unsupported_chroma(
            "general_intra_directional_chroma_neighbour",
            missing_capability_message!(
                "intra.chroma.directional.d135",
                neighbour = "unsupported",
                block = "non_full_sb_or_first_col",
            ),
        )),
        SupportedChromaMode::D113Follow | SupportedChromaMode::D113
            if above_left || (middle_subblock && has_edge) =>
        {
            Ok(())
        }
        SupportedChromaMode::D113Follow | SupportedChromaMode::D113 => Err(unsupported_chroma(
            "general_intra_directional_d113_chroma_neighbour",
            missing_capability_message!(
                "intra.chroma.directional.d113",
                neighbour = "above_left",
                block = "non_full_sb_or_edge",
            ),
        )),
        SupportedChromaMode::D157Follow | SupportedChromaMode::D157 => Err(unsupported_chroma(
            "general_intra_directional_d157_chroma_neighbour",
            missing_capability_message!(
                "intra.chroma.directional.d157",
                neighbour = "left_or_above_left",
                block = "non_full_sb_or_not_first_row",
            ),
        )),
        SupportedChromaMode::D45 if full_sb && neighbours.is_top_left() => Ok(()),
        SupportedChromaMode::D45Follow
        | SupportedChromaMode::D45
        | SupportedChromaMode::D67Follow
        | SupportedChromaMode::D67
            if one_sided_above_subblock =>
        {
            Ok(())
        }
        SupportedChromaMode::D45Follow
        | SupportedChromaMode::D45
        | SupportedChromaMode::D67Follow
        | SupportedChromaMode::D67 => Err(unsupported_chroma(
            "general_intra_directional_d45_chroma_neighbour",
            missing_capability_message!(
                "intra.chroma.directional.above_right",
                neighbour = "above_right",
                block = "non_full_sb_or_edge",
            ),
        )),
        SupportedChromaMode::VerticalFollow
        | SupportedChromaMode::Vertical
        | SupportedChromaMode::HorizontalFollow
        | SupportedChromaMode::Horizontal
            if (cardinal_subblock || full_sb) && has_edge =>
        {
            Ok(())
        }
        SupportedChromaMode::Vertical if dpcm.is_some() && full_sb && neighbours.is_top_left() => {
            Ok(())
        }
        SupportedChromaMode::Horizontal
            if cardinal_subblock && chroma_block.x() == 0 && chroma_block.y() == 0 =>
        {
            Ok(())
        }
        SupportedChromaMode::VerticalFollow | SupportedChromaMode::Vertical => {
            Err(unsupported_chroma(
                "general_intra_cardinal_vertical_chroma",
                missing_capability_message!(
                    "intra.chroma.cardinal.vertical",
                    neighbour = "above",
                    block = "non_full_sb_or_first_row",
                ),
            ))
        }
        SupportedChromaMode::HorizontalFollow => Err(unsupported_chroma(
            "general_intra_cardinal_horizontal_chroma",
            missing_capability_message!(
                "intra.chroma.cardinal.horizontal",
                neighbour = "left",
                block = "non_full_sb_or_first_col",
            ),
        )),
        SupportedChromaMode::Horizontal => Err(unsupported_chroma(
            "general_intra_horizontal_chroma_position",
            missing_capability_message!(
                "intra.chroma.horizontal",
                neighbour = "top_left_only",
                block = "non_full_sb_or_neighbour",
            ),
        )),
        SupportedChromaMode::Paeth if paeth_subblock => Ok(()),
        SupportedChromaMode::Paeth => Err(unsupported_chroma(
            "general_intra_paeth_chroma",
            missing_capability_message!(
                "intra.chroma.paeth",
                neighbour = "above_left_synthesized",
                block = "empty",
            ),
        )),
    }
}

fn ensure_supported_rect_chroma_capability(
    mode: SupportedChromaMode,
    block_ctx: BlockCtx,
) -> core::result::Result<(), ChromaCapabilityUnsupported> {
    let neighbours = block_ctx.neighbours(PlaneId::U);
    let chroma_block = block_ctx.plane_block(PlaneId::U);
    let supported_middle_shape = chroma_block.width4() >= 1 && chroma_block.height4() >= 1;
    let supported_smooth_shape = chroma_block.width4() >= 1 && chroma_block.height4() >= 1;
    match mode {
        SupportedChromaMode::Dc => Ok(()),
        SupportedChromaMode::D203Follow | SupportedChromaMode::D203
            if supported_smooth_shape && neighbours.has_left() =>
        {
            Ok(())
        }
        SupportedChromaMode::Vertical
        | SupportedChromaMode::VerticalFollow
        | SupportedChromaMode::Horizontal
        | SupportedChromaMode::HorizontalFollow
        | SupportedChromaMode::Smooth
        | SupportedChromaMode::SmoothVertical
        | SupportedChromaMode::SmoothHorizontal
            if supported_smooth_shape =>
        {
            Ok(())
        }
        SupportedChromaMode::Horizontal
            if supported_smooth_shape && chroma_block.x() == 0 && chroma_block.y() == 0 =>
        {
            Ok(())
        }
        mode if rect_chroma_is_middle_directional(mode)
            && supported_middle_shape
            && (neighbours.has_above() || neighbours.has_left()) =>
        {
            Ok(())
        }
        mode if rect_chroma_is_one_sided_above_directional(mode)
            && supported_smooth_shape
            && (neighbours.has_above() || neighbours.has_left()) =>
        {
            Ok(())
        }
        SupportedChromaMode::Paeth
            if supported_smooth_shape && (neighbours.has_above() || neighbours.has_left()) =>
        {
            Ok(())
        }
        _ => Err(unsupported_chroma(
            "general_intra_rect_non_dc_chroma",
            missing_capability_message!("intra.rect.chroma_mode", mode = "unsupported_non_dc"),
        )),
    }
}

const fn rect_chroma_is_middle_directional(mode: SupportedChromaMode) -> bool {
    matches!(
        mode,
        SupportedChromaMode::D135Follow
            | SupportedChromaMode::D135
            | SupportedChromaMode::D113Follow
            | SupportedChromaMode::D113
            | SupportedChromaMode::D157Follow
            | SupportedChromaMode::D157
    )
}

const fn rect_chroma_is_one_sided_above_directional(mode: SupportedChromaMode) -> bool {
    matches!(
        mode,
        SupportedChromaMode::D45Follow
            | SupportedChromaMode::D45
            | SupportedChromaMode::D67Follow
            | SupportedChromaMode::D67
    )
}

const fn unsupported_chroma(
    reason_id: &'static str,
    message: &'static str,
) -> ChromaCapabilityUnsupported {
    ChromaCapabilityUnsupported { reason_id, message }
}

fn general_intra_chroma_capability_error(
    error: ChromaCapabilityUnsupported,
    offset: ByteOffset,
) -> DecodeError {
    general_intra_at!(
        error.reason_id,
        offset,
        error.message,
        GENERAL_INTRA_MODE_SPEC_SECTION,
    )
}

fn general_intra_luma_plan_error(error: IntraLumaUnsupported, offset: ByteOffset) -> DecodeError {
    general_intra_at!(
        error.reason_id(),
        offset,
        error.message(),
        GENERAL_INTRA_MODE_SPEC_SECTION,
    )
}

fn general_intra_residual_plan_error(
    error: ResidualPipelineUnsupported,
    offset: ByteOffset,
) -> DecodeError {
    general_intra_at!(
        error.reason_id(),
        offset,
        error.message(),
        error.spec_section(),
    )
}

#[allow(clippy::needless_pass_by_value)]
fn general_intra_residual_error(
    error: GeneralIntraResidualError,
    offset: ByteOffset,
) -> DecodeError {
    match error {
        GeneralIntraResidualError::AllZeroRead { .. }
        | GeneralIntraResidualError::NonZeroPass { .. }
        | GeneralIntraResidualError::NonZeroStart { .. }
        | GeneralIntraResidualError::StagedNonZeroPass { .. }
        | GeneralIntraResidualError::StagedFscPass { .. }
        | GeneralIntraResidualError::TransformPartitionRead { .. }
        | GeneralIntraResidualError::TransformPartitionGeometry { .. }
        | GeneralIntraResidualError::TransformTypeRead { .. } => general_intra_at!(
            "general_intra_luma_coeff_parse",
            offset,
            "general intra luma transform-block coefficient syntax could not be parsed from the tile payload",
            GENERAL_INTRA_RESIDUAL_SPEC_SECTION,
        ),
        GeneralIntraResidualError::CoeffContextState { .. } => general_intra_at!(
            "general_intra_luma_coeff_state",
            offset,
            "general intra luma coefficient context state could not be derived from the tile work unit",
            GENERAL_INTRA_RESIDUAL_SPEC_SECTION,
        ),
        GeneralIntraResidualError::PaletteSymbolRead { .. }
        | GeneralIntraResidualError::PaletteLiteral { .. }
        | GeneralIntraResidualError::PaletteInvalidIdentityRow
        | GeneralIntraResidualError::PaletteColorIndex { .. } => general_intra_at!(
            "general_intra_luma_palette_parse",
            offset,
            "general intra luma palette color-map syntax could not be parsed from the tile payload",
            "5.20.8.1",
        ),
        GeneralIntraResidualError::UnsupportedTransformToolResidual { .. } => {
            general_intra_at!(
                "general_intra_transform_tool_residual",
                offset,
                missing_capability_message!("intra.residual.transform_tools", residual = "nonzero"),
                GENERAL_INTRA_RESIDUAL_SPEC_SECTION,
            )
        }
        GeneralIntraResidualError::UnsupportedTransformPartition { reason } => {
            general_intra_at!(
                reason,
                offset,
                missing_capability_message!("intra.residual.transform_partition"),
                GENERAL_INTRA_RESIDUAL_SPEC_SECTION,
            )
        }
        GeneralIntraResidualError::UnexpectedBranch => general_intra_at!(
            "general_intra_luma_coeff_unexpected_branch",
            offset,
            "general intra luma coefficient decode produced an unexpected branch result",
            GENERAL_INTRA_RESIDUAL_SPEC_SECTION,
        ),
        GeneralIntraResidualError::QuantLength { .. }
        | GeneralIntraResidualError::PredictionLength { .. }
        | GeneralIntraResidualError::Reconstruct { .. } => general_intra_at!(
            "general_intra_luma_reconstruct",
            offset,
            "general intra luma transform-block reconstruction could not be composed from the decoded coefficients",
            GENERAL_INTRA_RESIDUAL_SPEC_SECTION,
        ),
        GeneralIntraResidualError::UnsupportedDirectionalAboveEdge => general_intra_at!(
            "general_intra_directional_above_edge",
            offset,
            missing_capability_message!("intra.luma.directional.above_edge", neighbour = "corner",),
            GENERAL_INTRA_RESIDUAL_SPEC_SECTION,
        ),
        GeneralIntraResidualError::CardinalModeInMiddleAnglePath => general_intra_at!(
            "general_intra_cardinal_in_middle_angle_path",
            offset,
            missing_capability_message!("intra.luma.dispatch", path = "cardinal_in_middle_angle"),
            GENERAL_INTRA_RESIDUAL_SPEC_SECTION,
        ),
    }
}

#[allow(clippy::needless_pass_by_value)]
fn general_intra_block_mode_error(
    error: GeneralIntraBlockModeError,
    offset: ByteOffset,
) -> DecodeError {
    match error {
        GeneralIntraBlockModeError::SymbolRead { .. }
        | GeneralIntraBlockModeError::Literal { .. } => general_intra_at!(
            "general_intra_block_mode_parse",
            offset,
            "general intra block mode-info syntax could not be parsed from the tile payload",
            GENERAL_INTRA_MODE_SPEC_SECTION,
        ),
        GeneralIntraBlockModeError::UnsupportedYMode { .. } => general_intra_at!(
            "general_intra_unsupported_y_mode",
            offset,
            missing_capability_message!("intra.luma.mode", mode = "unsupported"),
            GENERAL_INTRA_MODE_SPEC_SECTION,
        ),
        GeneralIntraBlockModeError::InvalidUvMode { .. } => general_intra_at!(
            "general_intra_invalid_uv_mode",
            offset,
            "general intra decode rejected an out-of-range chroma uv_mode index",
            GENERAL_INTRA_MODE_SPEC_SECTION,
        ),
        GeneralIntraBlockModeError::InvalidFscBlockSizeIndex { .. } => general_intra_at!(
            "general_intra_invalid_fsc_block_size_index",
            offset,
            "general intra decode could not map MiSize through Fsc_Bsize_Groups",
            "8.3.2",
        ),
        GeneralIntraBlockModeError::InvalidCflMhDirBlockSizeIndex { .. } => {
            general_intra_at!(
                "general_intra_invalid_cfl_mh_dir_size_group",
                offset,
                "general intra decode could not map MiSize through Size_Group for cfl_mh_dir",
                "8.3.2",
            )
        }
        GeneralIntraBlockModeError::InvalidPaletteYSize { .. } => general_intra_at!(
            "general_intra_invalid_palette_y_size",
            offset,
            "general intra decode rejected an out-of-range luma palette size",
            "5.20.8.1",
        ),
        GeneralIntraBlockModeError::UnsupportedDirectionalNeighbourReorder { .. } => {
            general_intra_at!(
                "general_intra_directional_neighbour_reorder",
                offset,
                missing_capability_message!(
                    "intra.luma.directional_neighbour_reorder",
                    neighbour = "directional",
                ),
                GENERAL_INTRA_MODE_SPEC_SECTION,
            )
        }
    }
}

pub(crate) fn general_intra_unsupported(
    reason: &'static str,
    byte_offset: Option<ByteOffset>,
    message: &'static str,
    spec_section: &'static str,
) -> DecodeError {
    crate::pipeline::unsupported_with_spec(reason, byte_offset, message, spec_section)
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

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

    fn assert_chroma_admitted(mode: SupportedChromaMode, block: BlockCtx) {
        assert!(ensure_supported_chroma_capability(mode, None, block).is_ok());
        assert!(ten_bit_general_intra_chroma_admitted(Some(mode), block));
    }

    fn assert_rect_chroma_admitted(mode: SupportedChromaMode, block: BlockCtx) {
        assert!(ensure_supported_rect_chroma_capability(mode, block).is_ok());
        assert!(ten_bit_general_intra_chroma_admitted(Some(mode), block));
    }

    fn luma_modes(y_mode: IntraYMode) -> GeneralIntraBlockModes {
        luma_modes_with_angle(y_mode, 0)
    }

    fn luma_modes_with_angle(y_mode: IntraYMode, angle_delta_y: i8) -> GeneralIntraBlockModes {
        luma_modes_with_parts(y_mode, angle_delta_y, 0, 0)
    }

    fn luma_modes_with_dpcm(
        y_mode: IntraYMode,
        use_dpcm_y: u8,
        dpcm_mode_y: u8,
    ) -> GeneralIntraBlockModes {
        luma_modes_with_parts(y_mode, 0, use_dpcm_y, dpcm_mode_y)
    }

    fn luma_modes_with_parts(
        y_mode: IntraYMode,
        angle_delta_y: i8,
        use_dpcm_y: u8,
        dpcm_mode_y: u8,
    ) -> GeneralIntraBlockModes {
        GeneralIntraBlockModes::luma_only(
            crate::bitstream::tile_payload::GeneralIntraLumaBlockMode {
                y_mode,
                angle_delta_y,
                intra_joint_mode: 0,
                mrl_index: 0,
                mrl_sec_index: None,
                fsc_mode: 0,
                uses_mrls: 0,
                use_dpcm_y,
                dpcm_mode_y,
            },
        )
    }

    #[test]
    fn luma_tx_partition_context_uses_current_block_lossless_flag() {
        let block_size_index = 6;

        assert_eq!(
            luma_tx_partition_context(Some(TxMode::Select), block_size_index, false),
            Some(LumaTransformPartitionContext::new(block_size_index))
        );
        assert_eq!(
            luma_tx_partition_context(Some(TxMode::Select), block_size_index, true),
            None
        );
        assert_eq!(
            luma_tx_partition_context(Some(TxMode::Largest), block_size_index, false),
            None
        );
    }

    #[test]
    fn lossless_prediction_guard_admits_dc_luma_and_dpcm() {
        let modes = luma_modes(IntraYMode::DC_PRED);
        let top_left = ctx(0, 0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA);

        assert!(
            ensure_lossless_verified_prediction_subset(
                true,
                false,
                &modes,
                top_left,
                FULL_SB_N4_LUMA,
                splot_core::span::ByteOffset::new(0),
            )
            .is_ok()
        );

        let modes = luma_modes_with_dpcm(IntraYMode::V_PRED_FOR_TEST, 1, 0);
        assert!(
            ensure_lossless_verified_prediction_subset(
                true,
                false,
                &modes,
                top_left,
                FULL_SB_N4_LUMA,
                splot_core::span::ByteOffset::new(9),
            )
            .is_ok()
        );
    }

    fn assert_lossless_directional_luma_admitted(mode: IntraYMode) {
        let top_left = ctx_with_bit_depth(0, 0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA, BitDepth::Eight);
        let modes = luma_modes(mode);

        assert!(
            ensure_lossless_verified_prediction_subset(
                true,
                false,
                &modes,
                top_left,
                FULL_SB_N4_LUMA,
                splot_core::span::ByteOffset::new(9),
            )
            .is_ok()
        );
    }

    #[test]
    fn lossless_prediction_guard_admits_top_left_cardinal_luma() {
        for mode in [IntraYMode::V_PRED_FOR_TEST, IntraYMode::H_PRED_FOR_TEST] {
            assert_lossless_directional_luma_admitted(mode);
        }
    }

    #[test]
    fn lossless_prediction_guard_admits_top_left_d135_luma() {
        assert_lossless_directional_luma_admitted(IntraYMode::D135_PRED_FOR_TEST);
    }

    #[test]
    fn cardinal_top_left_guard_admits_only_verified_shapes() {
        let top_left_8 =
            ctx_with_bit_depth(0, 0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA, BitDepth::Eight);
        let vertical = luma_modes_with_angle(IntraYMode::V_PRED_FOR_TEST, 2);
        let horizontal = luma_modes(IntraYMode::H_PRED_FOR_TEST);

        assert_eq!(
            plan_luma_prediction_for_segment(&vertical, top_left_8, false, FULL_SB_N4_LUMA)
                .expect("verified top-left V_PRED angle delta should plan"),
            crate::prediction::intra::IntraLumaPlan::DirectionalMiddle { p_angle: 96 }
        );
        assert_eq!(
            plan_luma_prediction_for_segment(&horizontal, top_left_8, false, FULL_SB_N4_LUMA)
                .expect("verified top-left H_PRED cardinal should plan"),
            crate::prediction::intra::IntraLumaPlan::CardinalNeighbour {
                direction: IntraCardinalDirection::Horizontal,
            }
        );

        for (modes, block_ctx, sb_mib) in [
            (
                luma_modes(IntraYMode::V_PRED_FOR_TEST),
                top_left_8,
                FULL_SB_N4_LUMA,
            ),
            (
                luma_modes_with_angle(IntraYMode::V_PRED_FOR_TEST, 1),
                top_left_8,
                FULL_SB_N4_LUMA,
            ),
            (
                vertical,
                ctx(0, 0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA),
                FULL_SB_N4_LUMA,
            ),
            (vertical, top_left_8, 32),
            (
                horizontal,
                ctx_with_bit_depth(0, 0, 4, 4, BitDepth::Eight),
                FULL_SB_N4_LUMA,
            ),
            (
                vertical,
                ctx_with_bit_depth(0, 0, 4, 4, BitDepth::Eight),
                FULL_SB_N4_LUMA,
            ),
        ] {
            assert!(
                plan_luma_prediction_for_segment(&modes, block_ctx, false, sb_mib).is_err(),
                "unverified non-lossless cardinal luma shape must fail closed"
            );
        }
    }

    #[test]
    fn lossless_prediction_guard_rejects_unverified_directional_variants() {
        let top_left_8 =
            ctx_with_bit_depth(0, 0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA, BitDepth::Eight);
        let top_left_10 = ctx(0, 0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA);

        for mode in [
            IntraYMode::V_PRED_FOR_TEST,
            IntraYMode::H_PRED_FOR_TEST,
            IntraYMode::D45_PRED_FOR_TEST,
            IntraYMode::D135_PRED_FOR_TEST,
        ] {
            for (modes, block_ctx, sb_mib) in [
                (luma_modes_with_angle(mode, 1), top_left_8, FULL_SB_N4_LUMA),
                (luma_modes(mode), top_left_10, FULL_SB_N4_LUMA),
                (luma_modes(mode), top_left_8, 32),
            ] {
                let error = ensure_lossless_verified_prediction_subset(
                    true,
                    false,
                    &modes,
                    block_ctx,
                    sb_mib,
                    splot_core::span::ByteOffset::new(9),
                )
                .expect_err("unverified lossless directional luma variant must fail closed");

                let reason = match error {
                    DecodeError::UnsupportedFeature { unsupported } => unsupported.reason(),
                    _ => "",
                };
                assert_eq!(reason, "general_intra_lossless_other_nondc_luma_unverified");
            }
        }
    }

    #[test]
    fn admits_10bit_vertical_chroma_with_left_only_edge() {
        let first_row_second_sb = ctx(0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA);

        assert!(
            ensure_supported_chroma_capability(
                SupportedChromaMode::Vertical,
                None,
                first_row_second_sb,
            )
            .is_ok()
        );
        assert!(ten_bit_general_intra_chroma_admitted(
            Some(SupportedChromaMode::Vertical),
            first_row_second_sb
        ));
    }

    #[test]
    fn admits_10bit_horizontal_chroma_with_above_only_edge() {
        let first_col_second_row = ctx(FULL_SB_N4_LUMA, 0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA);

        assert!(
            ensure_supported_chroma_capability(
                SupportedChromaMode::HorizontalFollow,
                None,
                first_col_second_row,
            )
            .is_ok()
        );
        assert!(ten_bit_general_intra_chroma_admitted(
            Some(SupportedChromaMode::HorizontalFollow),
            first_col_second_row
        ));
    }

    #[test]
    fn admits_top_left_horizontal_chroma_subblock() {
        let top_left_subblock = ctx(0, 0, 8, 8);

        assert!(
            ensure_supported_chroma_capability(
                SupportedChromaMode::Horizontal,
                None,
                top_left_subblock,
            )
            .is_ok()
        );
        assert!(!ten_bit_general_intra_chroma_admitted(
            Some(SupportedChromaMode::Horizontal),
            top_left_subblock
        ));
    }

    #[test]
    fn admits_top_left_vertical_chroma_only_for_dpcm() {
        let top_left = ctx(0, 0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA);

        assert!(
            ensure_supported_chroma_capability(SupportedChromaMode::Vertical, None, top_left)
                .is_err()
        );
        assert!(
            ensure_supported_chroma_capability(
                SupportedChromaMode::Vertical,
                Some(DpcmDirection::Vertical),
                top_left,
            )
            .is_ok()
        );
        assert!(!ten_bit_general_intra_chroma_admitted(
            Some(SupportedChromaMode::Vertical),
            top_left,
        ));
    }

    #[test]
    fn admits_top_left_rect_horizontal_chroma_subblock() {
        let top_left_rect = ctx(0, 0, 8, 4);

        assert!(
            ensure_supported_rect_chroma_capability(SupportedChromaMode::Horizontal, top_left_rect,)
                .is_ok()
        );
        assert!(!ten_bit_general_intra_chroma_admitted(
            Some(SupportedChromaMode::Horizontal),
            top_left_rect
        ));
    }

    #[test]
    fn admits_top_left_smooth_chroma_subblock() {
        let top_left = ctx(0, 0, 2, 2);

        assert!(
            ensure_supported_chroma_capability(
                SupportedChromaMode::SmoothHorizontal,
                None,
                top_left
            )
            .is_ok()
        );
        assert!(
            ensure_supported_rect_chroma_capability(
                SupportedChromaMode::SmoothHorizontal,
                top_left,
            )
            .is_ok()
        );
        assert!(!ten_bit_general_intra_chroma_admitted(
            Some(SupportedChromaMode::SmoothHorizontal),
            top_left
        ));
    }

    #[test]
    fn admits_10bit_rect_horizontal_chroma_with_left_only_edge() {
        let first_row_wide_block = ctx(0, 224, 32, FULL_SB_N4_LUMA);

        assert!(
            ensure_supported_rect_chroma_capability(
                SupportedChromaMode::Horizontal,
                first_row_wide_block
            )
            .is_ok()
        );
        assert!(ten_bit_general_intra_chroma_admitted(
            Some(SupportedChromaMode::Horizontal),
            first_row_wide_block
        ));
    }

    #[test]
    fn admits_10bit_rect_smooth_chroma_with_left_only_edge() {
        let first_row_rect_block = ctx(0, 288, FULL_SB_N4_LUMA, 8);

        assert!(
            ensure_supported_rect_chroma_capability(
                SupportedChromaMode::Smooth,
                first_row_rect_block
            )
            .is_ok()
        );
        assert!(ten_bit_general_intra_chroma_admitted(
            Some(SupportedChromaMode::Smooth),
            first_row_rect_block
        ));
    }

    #[test]
    fn admits_10bit_small_rect_smooth_chroma_with_above_left_edges() {
        let rect_block = ctx(24, 200, 2, 4);

        assert_rect_chroma_admitted(SupportedChromaMode::Smooth, rect_block);
    }

    #[test]
    fn admits_square_smooth_chroma_subblock_with_above_left_edges() {
        let square_block = ctx(24, 200, 8, 8);

        assert_chroma_admitted(SupportedChromaMode::Smooth, square_block);
    }

    #[test]
    fn admits_small_vertical_follow_chroma_with_above_edge() {
        let small_block = ctx(20, 218, 2, 2);

        assert_chroma_admitted(SupportedChromaMode::VerticalFollow, small_block);
    }

    #[test]
    fn admits_rect_vertical_follow_chroma_with_left_only_edge() {
        let first_row_rect_block = ctx(0, 416, 8, FULL_SB_N4_LUMA);

        assert!(
            ensure_supported_chroma_capability(
                SupportedChromaMode::VerticalFollow,
                None,
                first_row_rect_block,
            )
            .is_ok()
        );
        assert!(
            ensure_supported_rect_chroma_capability(
                SupportedChromaMode::VerticalFollow,
                first_row_rect_block,
            )
            .is_ok()
        );
        assert!(ten_bit_general_intra_chroma_admitted(
            Some(SupportedChromaMode::VerticalFollow),
            first_row_rect_block
        ));
    }

    #[test]
    fn admits_small_d135_follow_chroma_with_above_left_edges() {
        let small_block = ctx(24, 200, 8, 8);

        assert_chroma_admitted(SupportedChromaMode::D135Follow, small_block);
    }

    #[test]
    fn admits_square_middle_chroma_with_one_sided_edge() {
        let above_only = ctx(FULL_SB_N4_LUMA, 0, 8, 8);
        let left_only = ctx(0, FULL_SB_N4_LUMA, 8, 8);
        let modes = [
            SupportedChromaMode::D113Follow,
            SupportedChromaMode::D113,
            SupportedChromaMode::D135Follow,
            SupportedChromaMode::D135,
            SupportedChromaMode::D157Follow,
            SupportedChromaMode::D157,
        ];

        for mode in modes {
            assert_chroma_admitted(mode, above_only);
            assert_chroma_admitted(mode, left_only);
        }
    }

    #[test]
    fn admits_small_d157_follow_chroma_with_above_left_edges() {
        let small_block = ctx(16, 416, 8, 8);

        assert_chroma_admitted(SupportedChromaMode::D157Follow, small_block);
    }

    #[test]
    fn admits_small_d67_follow_chroma_with_above_right_edge() {
        let small_block = ctx(28, 216, 2, 4);

        assert!(
            ensure_supported_chroma_capability(SupportedChromaMode::D67Follow, None, small_block)
                .is_ok()
        );
        assert!(
            ensure_supported_rect_chroma_capability(SupportedChromaMode::D67Follow, small_block)
                .is_ok()
        );
        assert!(ten_bit_general_intra_chroma_admitted(
            Some(SupportedChromaMode::D67Follow),
            small_block
        ));
    }

    #[test]
    fn admits_d67_follow_chroma_with_above_only_edge() {
        let first_col_block = ctx(128, 0, 32, 32);

        assert!(
            ensure_supported_chroma_capability(
                SupportedChromaMode::D67Follow,
                None,
                first_col_block,
            )
            .is_ok()
        );
        assert!(
            ensure_supported_rect_chroma_capability(
                SupportedChromaMode::D67Follow,
                first_col_block
            )
            .is_ok()
        );
        assert!(ten_bit_general_intra_chroma_admitted(
            Some(SupportedChromaMode::D67Follow),
            first_col_block
        ));
    }

    #[test]
    fn admits_top_left_full_sb_d45_chroma() {
        let top_left = ctx(0, 0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA);

        assert!(
            ensure_supported_chroma_capability(SupportedChromaMode::D45, None, top_left).is_ok()
        );
    }

    #[test]
    fn admits_rect_d113_chroma_at_tile_start_with_above_edge() {
        let tile_start_block = ctx(8, 16, 16, 8).with_tile_bounds(0, 16, 16, 32);
        let neighbours = tile_start_block.neighbours(PlaneId::U);

        assert!(neighbours.has_above());
        assert!(!neighbours.has_left());
        assert!(
            ensure_supported_rect_chroma_capability(SupportedChromaMode::D113, tile_start_block)
                .is_ok()
        );
    }

    #[test]
    fn admits_chroma_ref_rect_d157_follow_chroma_with_above_left_edges() {
        let chroma_ref = BlockRect::new(24, 220, 2, 4);
        let chroma_tx = TxShape::from_luma_4x4(2, 4).expect("valid chroma reference transform");
        let rect_block = ctx(24, 221, 1, 4).with_chroma_ref(chroma_ref, chroma_tx);

        assert!(
            ensure_supported_rect_chroma_capability(SupportedChromaMode::D157Follow, rect_block)
                .is_ok()
        );
        assert!(ten_bit_general_intra_chroma_admitted(
            Some(SupportedChromaMode::D157Follow),
            rect_block
        ));
    }

    #[test]
    fn admits_10bit_rect_paeth_chroma_with_above_left_edges() {
        let rect_block = ctx(FULL_SB_N4_LUMA, 192, FULL_SB_N4_LUMA, 8);

        assert!(
            ensure_supported_rect_chroma_capability(SupportedChromaMode::Paeth, rect_block).is_ok()
        );
        assert!(ten_bit_general_intra_chroma_admitted(
            Some(SupportedChromaMode::Paeth),
            rect_block
        ));
    }

    #[test]
    fn admits_10bit_square_paeth_chroma_with_left_only_edge() {
        let first_row_second_sb = ctx(0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA);

        assert!(
            ensure_supported_chroma_capability(
                SupportedChromaMode::Paeth,
                None,
                first_row_second_sb,
            )
            .is_ok()
        );
        assert!(ten_bit_general_intra_chroma_admitted(
            Some(SupportedChromaMode::Paeth),
            first_row_second_sb
        ));
    }

    #[test]
    fn admits_10bit_square_paeth_chroma_subblock_with_above_left_edges() {
        let square_subblock = ctx(40, 160, 8, 8);

        assert_chroma_admitted(SupportedChromaMode::Paeth, square_subblock);
    }

    #[test]
    fn admits_10bit_small_d203_follow_chroma_subblock() {
        let d203_subblock = ctx(32, 300, 2, 8);

        assert_chroma_admitted(SupportedChromaMode::D203Follow, d203_subblock);
    }

    #[test]
    fn admits_rect_d203_follow_chroma_subblock_with_left_edge() {
        let d203_subblock = ctx(32, 300, 2, 8);

        assert!(
            ensure_supported_rect_chroma_capability(
                SupportedChromaMode::D203Follow,
                d203_subblock,
            )
            .is_ok()
        );
    }

    #[test]
    fn admits_large_rect_smooth_luma_with_left_only_edge() {
        let first_row_wide_block = ctx(0, 256, 32, FULL_SB_N4_LUMA);
        let mode = RectLumaPlan::Smooth {
            mode: crate::bitstream::tile_payload::SupportedNonDcLumaMode::SmoothVertical,
            use_tcq: false,
        };

        assert_eq!(
            rect_luma_plan_for_parts(
                Some(crate::bitstream::tile_payload::SupportedNonDcLumaMode::SmoothVertical),
                None,
                false,
                first_row_wide_block,
                false,
            ),
            Ok(mode)
        );
    }

    #[test]
    fn admits_small_rect_smooth_luma_with_above_left_edges() {
        let rect_block = ctx(24, 200, 2, 4);
        let mode = RectLumaPlan::Smooth {
            mode: crate::bitstream::tile_payload::SupportedNonDcLumaMode::Smooth,
            use_tcq: false,
        };

        assert_eq!(
            rect_luma_plan_for_parts(
                Some(crate::bitstream::tile_payload::SupportedNonDcLumaMode::Smooth),
                None,
                false,
                rect_block,
                false,
            ),
            Ok(mode)
        );
    }

    #[test]
    fn admits_thin_rect_smooth_luma_with_above_left_edges() {
        let rect_block = ctx(48, 150, 1, 4);
        let mode = RectLumaPlan::Smooth {
            mode: crate::bitstream::tile_payload::SupportedNonDcLumaMode::Smooth,
            use_tcq: false,
        };

        assert_eq!(
            rect_luma_plan_for_parts(
                Some(crate::bitstream::tile_payload::SupportedNonDcLumaMode::Smooth),
                None,
                false,
                rect_block,
                false,
            ),
            Ok(mode)
        );
    }

    #[test]
    fn admits_small_rect_smooth_horizontal_luma_with_left_edge() {
        let rect_block = ctx(17, 220, 4, 1);
        let mode = RectLumaPlan::Smooth {
            mode: crate::bitstream::tile_payload::SupportedNonDcLumaMode::SmoothHorizontal,
            use_tcq: false,
        };

        assert_eq!(
            rect_luma_plan_for_parts(
                Some(crate::bitstream::tile_payload::SupportedNonDcLumaMode::SmoothHorizontal),
                None,
                false,
                rect_block,
                false,
            ),
            Ok(mode)
        );
    }

    #[test]
    fn admits_small_rect_paeth_luma_regardless_of_neighbour_edges() {
        let want = Ok(RectLumaPlan::Paeth { use_tcq: false });
        for block in [ctx(18, 220, 4, 2), ctx(0, 0, 4, 2)] {
            assert_eq!(
                rect_luma_plan_for_parts_ext(true, None, None, false, block, false),
                want
            );
        }
    }

    #[test]
    fn admits_small_rect_d135_mrl_middle_luma_with_above_left_edges() {
        let rect_block = ctx(20, 216, 1, 4);

        assert_eq!(
            rect_luma_mrl_plan_for_parts(
                IntraYMode::D135_PRED_FOR_TEST,
                0,
                3,
                Some(0),
                rect_block,
                false,
                32,
            ),
            Ok(RectLumaPlan::MiddleMrl {
                p_angle: 135,
                mrl_index: 3,
                above_mrl_index: 3,
                is_sb_boundary: false,
                secondary_mrl: false,
                use_tcq: false,
            })
        );
    }

    #[test]
    fn admits_small_square_v_pred_mrl3_secondary_as_cardinal_mrl() {
        let square_block = ctx(16, 264, 4, 4);

        assert_eq!(
            rect_luma_mrl_plan_for_parts(
                IntraYMode::V_PRED_FOR_TEST,
                0,
                3,
                Some(1),
                square_block,
                false,
                32,
            ),
            Ok(RectLumaPlan::CardinalMrl {
                direction: IntraCardinalDirection::Vertical,
                mrl_index: 3,
                above_mrl_index: 3,
                secondary_mrl: true,
                use_tcq: false,
            })
        );
    }

    #[test]
    fn admits_d45_mrl_one_sided_above_luma_on_sb_boundary() {
        let rect_block = ctx(32, 216, 4, 4);

        assert_eq!(
            rect_luma_mrl_plan_for_parts(
                IntraYMode::D45_PRED_FOR_TEST,
                -2,
                1,
                Some(0),
                rect_block,
                false,
                32,
            ),
            Ok(RectLumaPlan::OneSidedAboveMrl {
                p_angle: 40,
                mrl_index: 1,
                above_mrl_index: 0,
                secondary_mrl: false,
                use_tcq: false,
            })
        );
    }

    #[test]
    fn admits_small_square_d157_mrl_middle_luma_with_above_left_edges() {
        let square_block = ctx(26, 222, 2, 2);

        assert_eq!(
            rect_luma_mrl_plan_for_parts(
                IntraYMode::D157_PRED_FOR_TEST,
                3,
                1,
                Some(0),
                square_block,
                false,
                32,
            ),
            Ok(RectLumaPlan::MiddleMrl {
                p_angle: 167,
                mrl_index: 1,
                above_mrl_index: 1,
                is_sb_boundary: false,
                secondary_mrl: false,
                use_tcq: false,
            })
        );
    }

    #[test]
    fn admits_top_row_rect_d113_mrl_middle_luma_with_left_only_edge() {
        let rect_block = ctx(0, 316, 4, 8);

        assert_eq!(
            rect_luma_mrl_plan_for_parts(
                IntraYMode::D113_PRED_FOR_TEST,
                -1,
                2,
                Some(1),
                rect_block,
                false,
                32,
            ),
            Ok(RectLumaPlan::MiddleMrl {
                p_angle: 109,
                mrl_index: 2,
                above_mrl_index: 0,
                is_sb_boundary: true,
                secondary_mrl: true,
                use_tcq: false,
            })
        );
    }

    #[test]
    fn admits_rect_d67_mrl_as_one_sided_left_after_wide_angle_mapping() {
        let rect_block = ctx(22, 313, 2, 8);

        assert_eq!(
            rect_luma_mrl_plan_for_parts(
                IntraYMode::D67_PRED_FOR_TEST,
                0,
                2,
                Some(0),
                rect_block,
                false,
                32,
            ),
            Ok(RectLumaPlan::OneSidedLeftMrl {
                p_angle: 246,
                mrl_index: 2,
                secondary_mrl: false,
                use_tcq: false,
            })
        );
    }

    #[test]
    fn admits_small_rect_d67_angle_delta_as_one_sided_above_luma() {
        let rect_block = ctx(28, 216, 2, 4);

        assert_eq!(
            rect_luma_plan_for_parts(None, Some(76), false, rect_block, false),
            Ok(RectLumaPlan::OneSidedAbove {
                p_angle: 76,
                use_tcq: false,
            })
        );
    }

    #[test]
    fn admits_right_edge_rect_d45_as_one_sided_above_luma_without_above_right() {
        let right_edge_block = ctx(8, 479, 1, 2);

        let neighbours = right_edge_block.neighbours(PlaneId::Y);
        assert!(neighbours.has_above());
        assert_eq!(neighbours.num_above_right(), 0);

        let plan = rect_luma_plan_for_parts(None, Some(45), false, right_edge_block, false);
        assert!(matches!(
            plan,
            Ok(RectLumaPlan::OneSidedAbove {
                p_angle: 45,
                use_tcq: false,
            })
        ));
    }

    #[test]
    fn admits_large_rect_vertical_luma_with_above_edge() {
        let second_row_wide_block = ctx(FULL_SB_N4_LUMA, 256, 32, FULL_SB_N4_LUMA);
        let mode = RectLumaPlan::Cardinal {
            direction: IntraCardinalDirection::Vertical,
            use_tcq: false,
        };

        assert_eq!(
            rect_luma_plan_for_parts(None, Some(90), false, second_row_wide_block, false,),
            Ok(mode)
        );
    }

    #[test]
    fn admits_rect_vertical_luma_with_left_only_edge() {
        let first_row_block = ctx(0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA);
        let mode = RectLumaPlan::Cardinal {
            direction: IntraCardinalDirection::Vertical,
            use_tcq: false,
        };

        assert_eq!(
            rect_luma_plan_for_parts(None, Some(90), false, first_row_block, false),
            Ok(mode)
        );
    }

    #[test]
    fn admits_rect_horizontal_luma_with_above_only_edge() {
        let first_col_block = ctx(80, 0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA);
        let mode = RectLumaPlan::Cardinal {
            direction: IntraCardinalDirection::Horizontal,
            use_tcq: false,
        };

        assert_eq!(
            rect_luma_plan_for_parts(None, Some(180), false, first_col_block, false),
            Ok(mode)
        );
    }

    #[test]
    fn admits_small_rect_vertical_luma_with_above_edge() {
        let small_block = ctx(24, 204, 1, 2);
        let mode = RectLumaPlan::Cardinal {
            direction: IntraCardinalDirection::Vertical,
            use_tcq: false,
        };

        assert_eq!(
            rect_luma_plan_for_parts(None, Some(90), false, small_block, false,),
            Ok(mode)
        );
    }

    #[test]
    fn admits_small_first_row_hpred_angle_delta_as_one_sided_left_luma() {
        let first_row_block = ctx(0, 264, 8, 4);

        assert_eq!(
            rect_luma_plan_for_parts(None, Some(186), false, first_row_block, false),
            Ok(RectLumaPlan::OneSidedLeft {
                p_angle: 186,
                use_tcq: false,
            })
        );
    }

    #[test]
    fn admits_small_rect_middle_luma_with_above_left_edges() {
        let small_block = ctx(26, 204, 1, 2);
        let mode = RectLumaPlan::Middle {
            p_angle: 151,
            use_tcq: false,
        };

        assert_eq!(
            rect_luma_plan_for_parts(None, Some(151), false, small_block, false,),
            Ok(mode)
        );
    }

    #[test]
    fn admits_top_row_rect_d113_angle_delta_as_middle_left_only() {
        let rect_block = ctx(0, 312, 4, 8);
        let mode = RectLumaPlan::Middle {
            p_angle: 104,
            use_tcq: false,
        };

        assert_eq!(
            rect_luma_middle_left_only_plan(
                IntraYMode::D113_PRED_FOR_TEST,
                Some(104),
                rect_block,
                false,
            ),
            Some(mode)
        );
    }

    #[test]
    fn admits_rect_d67_angle_delta_as_one_sided_above_luma() {
        let rect_block = ctx(8, 336, FULL_SB_N4_LUMA, 8);
        let mode = RectLumaPlan::OneSidedAbove {
            p_angle: 61,
            use_tcq: false,
        };

        assert_eq!(
            rect_luma_plan_for_parts(None, Some(61), false, rect_block, false),
            Ok(mode)
        );
    }

    #[test]
    fn square_d67_angle_delta_uses_rect_residual_path_when_square_plan_rejects() {
        let first_col_block = ctx(128, 0, 32, 32);
        let modes = GeneralIntraBlockModes::luma_only(
            crate::bitstream::tile_payload::GeneralIntraLumaBlockMode {
                y_mode: IntraYMode::D67_PRED_FOR_TEST,
                angle_delta_y: -1,
                intra_joint_mode: 0,
                mrl_index: 0,
                mrl_sec_index: None,
                fsc_mode: 0,
                uses_mrls: 0,
                use_dpcm_y: 0,
                dpcm_mode_y: 0,
            },
        );

        assert!(plan_luma_prediction(&modes, first_col_block, false).is_err());
        assert_eq!(
            rect_luma_plan(&modes, first_col_block, false, 32),
            Ok(RectLumaPlan::OneSidedAbove {
                p_angle: 64,
                use_tcq: false,
            })
        );
        assert!(square_luma_needs_rect_residual_path(
            &modes,
            first_col_block,
            false,
            32,
            false
        ));
    }

    #[test]
    fn admits_rect_d135_angle_delta_as_middle_luma() {
        let rect_block = ctx(FULL_SB_N4_LUMA, 320, 8, FULL_SB_N4_LUMA);
        let mode = RectLumaPlan::Middle {
            p_angle: 126,
            use_tcq: false,
        };

        assert_eq!(
            rect_luma_plan_for_parts(None, Some(126), false, rect_block, false),
            Ok(mode)
        );
    }

    #[test]
    fn admits_rect_middle_chroma_with_above_left_edges() {
        let rect_block = ctx(FULL_SB_N4_LUMA, 320, 8, FULL_SB_N4_LUMA);

        assert!(
            ensure_supported_rect_chroma_capability(SupportedChromaMode::D135Follow, rect_block)
                .is_ok()
        );
        assert!(ten_bit_general_intra_chroma_admitted(
            Some(SupportedChromaMode::D135Follow),
            rect_block
        ));
        assert_eq!(
            rect_chroma_plan_for_mode(SupportedChromaMode::D135Follow, -3, None, rect_block),
            RectChromaPlan::Middle { p_angle: 126 }
        );
    }

    #[test]
    fn admits_top_row_rect_d113_follow_chroma_with_left_only_edge() {
        let first_row_rect_block = ctx(0, 316, 4, 8);

        assert!(
            ensure_supported_rect_chroma_capability(
                SupportedChromaMode::D113Follow,
                first_row_rect_block,
            )
            .is_ok()
        );
        assert!(ten_bit_general_intra_chroma_admitted(
            Some(SupportedChromaMode::D113Follow),
            first_row_rect_block
        ));
        assert_eq!(
            rect_chroma_plan_for_mode(
                SupportedChromaMode::D113Follow,
                -1,
                None,
                first_row_rect_block,
            ),
            RectChromaPlan::Middle { p_angle: 110 }
        );
    }

    #[test]
    fn admits_top_row_rect_d157_follow_chroma_with_left_only_edge() {
        let first_row_rect_block = ctx(0, 352, 16, 8);

        assert!(
            ensure_supported_rect_chroma_capability(
                SupportedChromaMode::D157Follow,
                first_row_rect_block,
            )
            .is_ok()
        );
        assert!(ten_bit_general_intra_chroma_admitted(
            Some(SupportedChromaMode::D157Follow),
            first_row_rect_block
        ));
        assert_eq!(
            rect_chroma_plan_for_mode(
                SupportedChromaMode::D157Follow,
                -1,
                None,
                first_row_rect_block,
            ),
            RectChromaPlan::Middle { p_angle: 154 }
        );
    }

    #[test]
    fn admits_top_row_rect_d135_chroma_with_left_only_edge() {
        let first_row_rect_block = ctx(0, 320, 32, 16);

        assert!(
            ensure_supported_rect_chroma_capability(
                SupportedChromaMode::D135,
                first_row_rect_block,
            )
            .is_ok()
        );
        assert!(ten_bit_general_intra_chroma_admitted(
            Some(SupportedChromaMode::D135),
            first_row_rect_block
        ));
        assert_eq!(
            rect_chroma_plan_for_mode(SupportedChromaMode::D135, -3, None, first_row_rect_block),
            RectChromaPlan::Middle { p_angle: 135 }
        );
    }

    #[test]
    fn follows_luma_angle_delta_for_directional_chroma() {
        let tall_chroma_block = ctx(48, 220, 4, 16);

        assert_eq!(
            rect_chroma_plan_for_mode(
                SupportedChromaMode::VerticalFollow,
                -1,
                None,
                tall_chroma_block,
            ),
            RectChromaPlan::OneSided { p_angle: 87 }
        );
        assert_eq!(
            rect_chroma_plan_for_mode(
                SupportedChromaMode::VerticalFollow,
                1,
                None,
                tall_chroma_block,
            ),
            RectChromaPlan::Middle { p_angle: 93 }
        );
        assert_eq!(
            rect_chroma_plan_for_mode(SupportedChromaMode::Vertical, 1, None, tall_chroma_block),
            RectChromaPlan::Mode(SupportedChromaMode::Vertical, None)
        );
        assert_eq!(
            rect_chroma_plan_for_mode(
                SupportedChromaMode::Vertical,
                1,
                Some(DpcmDirection::Vertical),
                tall_chroma_block,
            ),
            RectChromaPlan::Mode(SupportedChromaMode::Vertical, Some(DpcmDirection::Vertical))
        );
    }

    #[test]
    fn admits_top_row_rect_d45_follow_chroma_with_left_only_edge() {
        let first_row_rect_block = ctx(0, 352, 32, 16);
        let right_edge_rect_block = ctx(28, 478, 2, 2);

        assert!(
            ensure_supported_rect_chroma_capability(
                SupportedChromaMode::D45Follow,
                first_row_rect_block,
            )
            .is_ok()
        );
        assert!(ten_bit_general_intra_chroma_admitted(
            Some(SupportedChromaMode::D45Follow),
            first_row_rect_block
        ));
        assert!(
            right_edge_rect_block
                .neighbours(PlaneId::U)
                .num_above_right()
                == 0
                && ensure_supported_rect_chroma_capability(
                    SupportedChromaMode::D45,
                    right_edge_rect_block,
                )
                .is_ok()
        );
        assert_eq!(
            rect_chroma_plan_for_mode(SupportedChromaMode::D45, 0, None, right_edge_rect_block),
            RectChromaPlan::OneSided { p_angle: 45 }
        );
    }

    #[test]
    fn admits_rect_middle_chroma_with_one_sided_edge() {
        let first_row_rect_block = ctx(0, 320, 8, FULL_SB_N4_LUMA);
        let first_col_rect_block = ctx(320, 0, FULL_SB_N4_LUMA, 8);
        let modes = [
            SupportedChromaMode::D113Follow,
            SupportedChromaMode::D113,
            SupportedChromaMode::D135Follow,
            SupportedChromaMode::D135,
            SupportedChromaMode::D157Follow,
            SupportedChromaMode::D157,
        ];

        for mode in modes {
            assert_rect_chroma_admitted(mode, first_row_rect_block);
            assert_rect_chroma_admitted(mode, first_col_rect_block);
        }
    }
}

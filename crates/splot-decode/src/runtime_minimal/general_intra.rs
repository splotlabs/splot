// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::headers::sequence::BitDepthIdc;
use splot_recon::{BitDepth, CurrentFrameWorkspace, IntraCardinalDirection, PlaneId, ReconSample};

use super::block_context::{BlockCtx, BlockRect, ChromaSampling, TxShape};
use super::capability::missing_capability_message;
use super::intra_prediction::{IntraLumaUnsupported, plan_luma_prediction};
use super::residual_pipeline::{
    GeneralIntraResidualPlan, RectChromaPlan, RectLumaPlan, ResidualPipelineUnsupported,
};
use super::*;
use crate::tile_payload::{
    ActiveChromaResidualPolicy, ActiveIntraIstResidualPolicy, CflIndex, GeneralIntraBlockModes,
    GeneralIntraChromaBlockMode, GeneralIntraChromaModeContext, GeneralIntraChromaToolConfig,
    GeneralIntraLeafMode, IsCflContext, LumaTransformPartitionContext, LumaTransformTypeContext,
    SupportedChromaMode, SupportedDirectionalLumaMode, SupportedNonDcLumaMode,
    TransformToolResidualPolicy,
};

const FULL_SB_N4_LUMA: usize = 16;
const MI_SIZE: usize = 4;
const ANGLE_STEP: i32 = 3;
const MRL_INDEX_TO_DELTA: [i32; 4] = [0, 1, -1, 0];
const WAIP_WH_RATIO_THRESHOLDS: [(usize, i32); 4] = [(2, 61), (4, 73), (8, 82), (16, 86)];

macro_rules! general_intra_at {
    ($reason:expr, $offset:expr, $message:expr, $spec_section:expr $(,)?) => {
        general_intra_unsupported($reason, Some($offset), $message, $spec_section)
    };
}

pub(super) fn route_general_minimal_intra(
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
) -> bool {
    core.quantization_params
        .is_some_and(|quant| quant.base_q_idx != FROZEN_MINIMAL_BASE_Q_IDX)
        && core.quantization_params.is_some_and(|quant| {
            quant.delta_q_y_dc == 0
                && quant.delta_q_u_dc == 0
                && quant.delta_q_u_ac == 0
                && quant.delta_q_v_dc == 0
                && quant.delta_q_v_ac == 0
        })
        && sequence.intra.as_ref().is_some_and(|intra| {
            !intra.enable_dip
                && !intra.enable_ibp
                && !intra.enable_mrls
                && !intra.enable_intra_edge_filter
        })
        && sequence
            .partition
            .is_some_and(|partition| !partition.enable_sdp)
        && sequence.transform_quant_entropy.is_some_and(|tq| {
            tq.equal_ac_dc_q
                && !tq.enable_fsc
                && !tq.enable_cctx
                && !tq.enable_idtx_intra
                && !tq.enable_intra_ist
                && i32::from(tq.base_uv_dc_delta_q) + GENERAL_INTRA_DELTA_DCQUANT_MIN == 0
                && i32::from(tq.base_uv_ac_delta_q) + GENERAL_INTRA_DELTA_DCQUANT_MIN == 0
        })
        && core
            .intra_tail
            .is_some_and(|tail| tail.tx_mode == TxMode::Largest)
        && core.deblocking_filter_params.is_some_and(|filter| {
            filter.apply_deblocking_filter == [false; 4]
                || matches!(sequence.general.bit_depth_idc, BitDepthIdc::Eight)
        })
        && core.cdef_params.as_ref().is_some_and(|cdef| {
            !cdef.cdef_frame_enable || matches!(sequence.general.bit_depth_idc, BitDepthIdc::Eight)
        })
        && is_general_minimal_intra(core)
}
fn is_general_minimal_intra(core: &FrameHeaderCore) -> bool {
    core.status == FrameHeaderParseStatus::IntraHeaderComplete
        && core.cur_mfh_id.is_zero()
        && core.show_existing_frame == Some(false)
        && core.frame_is_intra == Some(true)
        && core.is_key_frame
        && core.immediate_output_frame == Some(true)
        && core.implicit_output_frame == Some(false)
        && core.frame_size.is_some_and(|size| {
            size.width != 0
                && size.height != 0
                && size.width % MINIMAL_WIDTH == 0
                && size.height % MINIMAL_HEIGHT == 0
        })
        && core
            .tile_info
            .as_ref()
            .is_some_and(|tile_info| tile_info.tile_cols == 1 && tile_info.tile_rows == 1)
        && core.quantization_params.is_some()
        && core
            .segmentation_params
            .as_ref()
            .is_some_and(|seg| !seg.segmentation_enabled)
        && core.setup_qm_params.is_some_and(|qm| !qm.using_qmatrix)
        && core
            .delta_q_params
            .is_some_and(|delta| !delta.delta_q_present)
        && core
            .lossless_info
            .as_ref()
            .is_some_and(|lossless| !lossless.coded_lossless)
        && core
            .deblocking_filter_params
            .is_some_and(|filter| filter.df_delta_q == [0; 4])
        && core.gdf_params.is_some_and(|gdf| !gdf.gdf_frame_enable)
        && core.cdef_params.as_ref().is_some_and(|cdef| {
            !cdef.cdef_frame_enable
                || (cdef.cdef_strengths == Some(1)
                    && cdef.cdef_on_skip_txfm_frame_enable == Some(true)
                    && cdef.cdef_damping.is_some()
                    && !cdef.strengths.is_empty())
        })
        && core.lr_params.as_ref().is_some_and(|lr| !lr.uses_lr)
        && core
            .ccso_params
            .as_ref()
            .is_some_and(|ccso| ccso.ccso_frame_flag.is_none() && ccso.planes.is_empty())
        && core
            .intra_tail
            .is_some_and(|tail| !tail.film_grain.apply_grain)
        && core.allow_screen_content_tools != Some(true)
}
#[allow(clippy::too_many_arguments)]
pub(super) fn decode_general_minimal_intra_frame(
    plan: &DecodeStreamPlan,
    candidate: &DecodePlannedObu,
    bytes: &[u8],
    frame_envelope: ObuEnvelope<'_>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    options: &DecodeOptions,
    header: IvfHeader,
) -> Result<MinimalRuntimeFrame> {
    let mut tile_plan = derive_tile_plan(
        plan,
        candidate,
        bytes,
        frame_envelope,
        sequence,
        core,
        options,
    )?;
    let tile = match tile_plan.work_units_mut() {
        [tile] => tile,
        [] => {
            return Err(general_intra_unsupported(
                "general_intra_missing_tile_work_unit",
                None,
                "general intra decode requires one tile work unit",
                GENERAL_INTRA_TILE_SPEC_SECTION,
            ));
        }
        work_units => {
            return Err(general_intra_unsupported(
                "general_intra_unexpected_tile_work_units",
                work_units.first().map(|tile| tile.tile_byte_span().start),
                missing_capability_message!("intra.tile.count", count = "not_one"),
                GENERAL_INTRA_TILE_SPEC_SECTION,
            ));
        }
    };
    let tile_offset = tile.tile_byte_span().start;

    let qindex = core
        .quantization_params
        .map(|quant| quant.base_q_idx)
        .ok_or_else(|| {
            general_intra_at!(
                "general_intra_missing_base_q",
                tile_offset,
                "general intra decode requires a parsed base_q_idx",
                GENERAL_INTRA_RESIDUAL_SPEC_SECTION,
            )
        })?;
    let luma_use_tcq = tile.coeff_frame_facts().allow_tcq();
    let (mi_rows, mi_cols) = crate::tile_payload::frame_mi_dimensions(core)
        .map_err(|error| general_intra_partition_frontier_error(error, tile_offset))?;

    let frame_size = core.frame_size.ok_or_else(|| {
        general_intra_at!(
            "general_intra_missing_frame_size",
            tile_offset,
            "general intra decode requires a parsed frame size",
            GENERAL_INTRA_RESIDUAL_SPEC_SECTION,
        )
    })?;
    let frame_width = frame_size.width;
    let frame_height = frame_size.height;

    let tile_size = tile.tile_size();
    let limits = options.limits();

    let bit_depth = match sequence.general.bit_depth_idc {
        BitDepthIdc::Eight => BitDepth::Eight,
        BitDepthIdc::Ten => BitDepth::Ten,
    };

    ensure_runtime_limits(limits, frame_width, frame_height, tile_size, bit_depth)?;

    let frame = match bit_depth {
        BitDepth::Eight => {
            MinimalRuntimeDecodedFrame::Eight(decode_general_intra_frame_into::<u8>(
                tile,
                sequence,
                core,
                limits,
                frame_width as usize,
                frame_height as usize,
                mi_rows,
                mi_cols,
                qindex,
                luma_use_tcq,
                bit_depth,
                tile_offset,
            )?)
        }
        BitDepth::Ten => MinimalRuntimeDecodedFrame::Ten(decode_general_intra_frame_into::<u16>(
            tile,
            sequence,
            core,
            limits,
            frame_width as usize,
            frame_height as usize,
            mi_rows,
            mi_cols,
            qindex,
            luma_use_tcq,
            bit_depth,
            tile_offset,
        )?),
    };
    let frame_cdfs = tile.frame_cdfs();
    Ok(MinimalRuntimeFrame {
        frame,
        frame_cdfs,
        frame_rate_numerator: header.timebase_denominator,
        frame_rate_denominator: header.timebase_numerator,
    })
}
#[allow(clippy::too_many_arguments)]
fn decode_general_intra_frame_into<T: ReconSample>(
    tile: &mut crate::tile_payload::DecodeTileWorkUnit<'_>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    limits: crate::DecodeLimits,
    frame_width: usize,
    frame_height: usize,
    mi_rows: usize,
    mi_cols: usize,
    qindex: u32,
    luma_use_tcq: bool,
    bit_depth: BitDepth,
    tile_offset: ByteOffset,
) -> Result<DecodedFrame<T>> {
    let mut workspace = crate::runtime_minimal_recon::new_general_intra_workspace::<T>(
        frame_width,
        frame_height,
        bit_depth,
    )?;
    let mut coeff_ctx =
        crate::tile_payload::TileCoeffContextState::new(mi_rows, mi_cols).map_err(|source| {
            general_intra_residual_error(
                GeneralIntraResidualError::CoeffContextState { source },
                tile_offset,
            )
        })?;

    let mut deblock_blocks: Vec<super::deblock::DeblockBlock> = Vec::new();

    let symbols = crate::tile_payload::decode_general_intra_multiblock_tree(
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
            decode_one_general_intra_block::<T>(
                work_unit,
                symbols,
                frontier,
                sequence,
                None,
                core,
                joint_modes,
                uses_mrls,
                fsc_modes,
                palette_state,
                is_cfl_ctx,
                block_decoded,
                &mut workspace,
                &mut coeff_ctx,
                &mut deblock_blocks,
                qindex,
                luma_use_tcq,
                general_intra_transform_tool_residual_policy(sequence),
                mi_cols,
                mi_rows,
                bit_depth,
                tile_offset,
            )
        },
    )
    .map_err(|error| map_general_intra_multiblock_error(error, tile_offset))?;

    symbols.exit_symbol().map_err(|_| {
        general_intra_at!(
            "general_intra_exit_symbol",
            tile_offset,
            "general intra tile payload did not satisfy §8.2.4 exit_symbol() after the decoded blocks",
            GENERAL_INTRA_RESIDUAL_SPEC_SECTION,
        )
    })?;
    tile.apply_frame_end_cdf_update();

    if let Some(filter) = core.deblocking_filter_params {
        super::deblock::deblock_general_intra_frame(
            &mut workspace,
            &deblock_blocks,
            [&[], &[]],
            mi_rows,
            mi_cols,
            filter,
            super::deblock_quant_deltas(sequence, core),
            bit_depth,
        )
        .map_err(|error| general_intra_deblock_error(error, tile_offset))?;
    }

    if let Some(params) = cdef_frame_params(core) {
        super::cdef::cdef_general_intra_frame(&mut workspace, params, mi_rows, mi_cols, bit_depth)
            .map_err(|error| general_intra_cdef_error(error, tile_offset))?;
    }

    Ok(workspace.freeze()?)
}
fn cdef_frame_params(core: &FrameHeaderCore) -> Option<super::cdef::CdefFrameParams> {
    super::cdef::cdef_frame_strengths(core)?.into_iter().next()
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

fn general_intra_transform_tool_residual_policy(
    sequence: &SequenceHeader,
) -> TransformToolResidualPolicy {
    TransformToolResidualPolicy::from_sequence_tools(
        sequence,
        ActiveIntraIstResidualPolicy::Reject,
        ActiveChromaResidualPolicy::Reject,
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
fn general_intra_cdef_error(_error: super::cdef::CdefError, offset: ByteOffset) -> DecodeError {
    general_intra_at!(
        "general_intra_cdef",
        offset,
        missing_capability_message!("intra.cdef.block_config"),
        "7.18",
    )
}
fn general_intra_deblock_error(
    _error: super::deblock::DeblockError,
    offset: ByteOffset,
) -> DecodeError {
    general_intra_at!(
        "general_intra_deblock",
        offset,
        missing_capability_message!("intra.deblock.edge_config"),
        "7.17",
    )
}
#[allow(clippy::too_many_arguments)]
pub(super) fn decode_one_general_intra_block<T: ReconSample>(
    work_unit: &mut crate::tile_payload::DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    frontier: &crate::tile_payload::DecodeBlockFrontier,
    sequence: &SequenceHeader,
    y_smooth: Option<&super::intra_edge::TileYSmoothGrid>,
    core: &FrameHeaderCore,
    joint_modes: &crate::tile_payload::TileIntraJointModeState,
    uses_mrls: &crate::tile_payload::TileUsesMrlsState,
    fsc_modes: &crate::tile_payload::TileFscModeState,
    palette_state: &crate::tile_payload::TileLumaPaletteState,
    is_cfl_ctx: IsCflContext,
    block_decoded: &mut crate::tile_payload::TileBlockDecodedState,
    workspace: &mut CurrentFrameWorkspace<T>,
    coeff_ctx: &mut crate::tile_payload::TileCoeffContextState,
    deblock_blocks: &mut Vec<super::deblock::DeblockBlock>,
    qindex: u32,
    luma_use_tcq: bool,
    transform_tool_residual_policy: TransformToolResidualPolicy,
    mi_cols: usize,
    mi_rows: usize,
    bit_depth: BitDepth,
    tile_offset: ByteOffset,
) -> Result<GeneralIntraLeafMode> {
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
    let mut block_ctx = BlockCtx::new(
        BlockRect::new(frontier.r, frontier.c, n4w, n4h),
        block_tx_shape,
        mi_cols,
        mi_rows,
        bit_depth,
        ChromaSampling::Yuv420,
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
    let intra_edge = super::intra_edge::IntraEdgeCtx {
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
    };
    let chroma_tools = general_intra_chroma_tools(sequence, core);
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
            block_decoded,
            workspace,
            coeff_ctx,
            deblock_blocks,
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
        let luma = crate::tile_payload::decode_general_intra_luma_block_mode_with_fsc_context(
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
        let palette_y = crate::tile_payload::read_general_intra_palette_y_mode(
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
        crate::tile_payload::decode_general_intra_block_modes_with_fsc_context(
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
    if luma_only {
        ensure_10bit_general_intra_luma_capability(&modes, block_ctx, sb_mib, tile_offset)?;
    } else {
        ensure_10bit_general_intra_capability(
            &modes,
            block_ctx,
            (mi_cols, mi_rows),
            sb_mib,
            tile_offset,
        )?;
    }

    if n4w != n4h
        || modes.uses_active_mrl()
        || square_luma_needs_rect_residual_path(&modes, block_ctx, luma_use_tcq, sb_mib)
    {
        return decode_one_general_intra_rect_block::<T>(
            intra_edge,
            work_unit,
            symbols,
            frontier.has_chroma,
            &modes,
            workspace,
            coeff_ctx,
            deblock_blocks,
            qindex,
            luma_use_tcq,
            transform_tool_residual_policy,
            luma_tx_partition_context(core, frontier),
            block_ctx,
            block_decoded,
            cfl_ds_filter_index,
            sb_mib,
            tile_offset,
        );
    }

    let chroma_plan = if frontier.has_chroma {
        Some(
            chroma_plan_for_modes(&modes, block_ctx, cfl_ds_filter_index, sb_mib)
                .map_err(|error| general_intra_chroma_capability_error(error, tile_offset))?,
        )
    } else {
        None
    };
    if let Some(RectChromaPlan::Mode(supported_chroma)) = chroma_plan {
        ensure_supported_chroma_capability(supported_chroma, block_ctx)
            .map_err(|error| general_intra_chroma_capability_error(error, tile_offset))?;
    }
    let luma_plan = plan_luma_prediction(&modes, block_ctx)
        .map_err(|error| general_intra_luma_plan_error(error, tile_offset))?;

    let residual_plan = GeneralIntraResidualPlan::square(
        block_ctx,
        luma_plan,
        chroma_plan,
        luma_use_tcq,
        modes.uses_active_fsc(),
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
        modes.coeff_uv_mode(),
        luma_transform_type_context(&modes),
        luma_tx_partition_context(core, frontier),
        transform_tool_residual_policy,
        qindex,
        intra_edge,
        tile_offset,
    )?;

    Ok(leaf_mode_for_block(&modes, frontier.has_chroma))
}

#[allow(clippy::too_many_arguments)]
fn decode_one_general_intra_chroma_part_block<T: ReconSample>(
    intra_edge: super::intra_edge::IntraEdgeCtx,
    work_unit: &mut crate::tile_payload::DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    frontier: &crate::tile_payload::DecodeBlockFrontier,
    chroma_tools: GeneralIntraChromaToolConfig,
    is_cfl_ctx: IsCflContext,
    cfl_ds_filter_index: u8,
    sb_mib: usize,
    block_decoded: &mut crate::tile_payload::TileBlockDecodedState,
    workspace: &mut CurrentFrameWorkspace<T>,
    coeff_ctx: &mut crate::tile_payload::TileCoeffContextState,
    deblock_blocks: &mut Vec<super::deblock::DeblockBlock>,
    qindex: u32,
    transform_tool_residual_policy: TransformToolResidualPolicy,
    block_ctx: BlockCtx,
    tile_offset: ByteOffset,
) -> Result<GeneralIntraLeafMode> {
    let y_mode = frontier.stored_luma_y_mode().ok_or_else(|| {
        general_intra_at!(
            "general_intra_missing_sdp_luma_mode",
            tile_offset,
            "SDP chroma decode requires stored collocated luma mode facts",
            GENERAL_INTRA_MODE_SPEC_SECTION,
        )
    })?;
    let angle_delta_y = frontier.stored_luma_angle_delta_y().ok_or_else(|| {
        general_intra_at!(
            "general_intra_missing_sdp_luma_angle",
            tile_offset,
            "SDP chroma decode requires stored collocated luma angle facts",
            GENERAL_INTRA_MODE_SPEC_SECTION,
        )
    })?;
    let trace_bits = crate::trace_flags::trace_flag!("SPLOT_TRACE_GENERAL_INTRA_BITS");
    let mode_start_bits = symbols.consumed_bits().get();
    let chroma = crate::tile_payload::decode_general_intra_chroma_block_mode(
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
        chroma.coeff_uv_mode(),
        LumaTransformTypeContext::new(y_mode, angle_delta_y),
        None,
        transform_tool_residual_policy,
        qindex,
        intra_edge,
        tile_offset,
    )?;
    Ok(GeneralIntraLeafMode::chroma(chroma.is_cfl()))
}

#[allow(clippy::too_many_arguments)]
fn decode_one_general_intra_rect_block<T: ReconSample>(
    intra_edge: super::intra_edge::IntraEdgeCtx,
    work_unit: &mut crate::tile_payload::DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    has_chroma: bool,
    modes: &GeneralIntraBlockModes,
    workspace: &mut CurrentFrameWorkspace<T>,
    coeff_ctx: &mut crate::tile_payload::TileCoeffContextState,
    deblock_blocks: &mut Vec<super::deblock::DeblockBlock>,
    qindex: u32,
    luma_use_tcq: bool,
    transform_tool_residual_policy: TransformToolResidualPolicy,
    luma_tx_partition_context: Option<LumaTransformPartitionContext>,
    block_ctx: BlockCtx,
    block_decoded: &mut crate::tile_payload::TileBlockDecodedState,
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

    let residual_plan =
        GeneralIntraResidualPlan::rect(block_ctx, luma_plan, chroma_plan, modes.uses_active_fsc())
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

fn luma_transform_type_context(modes: &GeneralIntraBlockModes) -> LumaTransformTypeContext {
    LumaTransformTypeContext::with_mrl_indices(
        modes.y_mode,
        modes.angle_delta_y,
        modes.mrl_index,
        modes.mrl_sec_index,
    )
}

fn luma_tx_partition_context(
    core: &FrameHeaderCore,
    frontier: &crate::tile_payload::DecodeBlockFrontier,
) -> Option<LumaTransformPartitionContext> {
    if frame_tx_mode(core) != Some(TxMode::Select) {
        return None;
    }
    if core
        .lossless_info
        .as_ref()
        .is_none_or(|lossless| lossless.lossless_array[0])
    {
        return None;
    }
    Some(LumaTransformPartitionContext::new(frontier.b_size.index()))
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
) -> bool {
    let block = block_ctx.block();
    block.width4() == block.height4()
        && plan_luma_prediction(modes, block_ctx).is_err()
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
    y_mode: crate::tile_payload::IntraYMode,
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
    if p_angle > 0 && p_angle < 90 && neighbours.has_above() && neighbours.has_left() {
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
    let large_rect = block.width4() > FULL_SB_N4_LUMA || block.height4() > FULL_SB_N4_LUMA;
    let supported_rect = block.width4() >= 8 && block.height4() >= 8;
    let supported_smooth_axis_rect = block.width4() >= 1 && block.height4() >= 1;
    let supported_cardinal_rect = block.width4() >= 1 && block.height4() >= 1;
    let supported_middle_rect = block.width4() >= 1 && block.height4() >= 1;
    let supported_one_sided_above_rect = block.width4() >= 1 && block.height4() >= 1;
    let supported_one_sided_left_rect = block.width4() >= 1 && block.height4() >= 1;
    let supported_smooth_rect = block.width4() >= 1 && block.height4() >= 1;
    let neighbours = block_ctx.neighbours(PlaneId::Y);
    if luma_is_paeth && (neighbours.has_above() || neighbours.has_left()) {
        return Ok(RectLumaPlan::Paeth { use_tcq });
    }
    if let Some(mode) = nondc {
        let has_edge = neighbours.has_above() || neighbours.has_left();
        let supported = match mode {
            SupportedNonDcLumaMode::Smooth => (large_rect || supported_smooth_rect) && has_edge,
            SupportedNonDcLumaMode::SmoothVertical => {
                (large_rect && has_edge) || (supported_smooth_axis_rect && neighbours.has_above())
            }
            SupportedNonDcLumaMode::SmoothHorizontal => {
                (large_rect && has_edge) || (supported_smooth_axis_rect && neighbours.has_left())
            }
        };
        if supported {
            return Ok(RectLumaPlan::Smooth { mode, use_tcq });
        }
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
        && ((neighbours.has_above()
            && neighbours.num_above_right() > 0
            && (supported_rect || (supported_one_sided_above_rect && neighbours.has_left())))
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
    y_mode: crate::tile_payload::IntraYMode,
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
    y_mode: crate::tile_payload::IntraYMode,
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
    Ok(
        if let Some(p_angle) = rect_chroma_middle_p_angle_for_parts(
            mode,
            rect_luma_directional_p_angle(modes, block_ctx),
            block_ctx,
        ) {
            RectChromaPlan::Middle { p_angle }
        } else {
            RectChromaPlan::Mode(mode)
        },
    )
}

fn chroma_plan_for_modes(
    modes: &GeneralIntraBlockModes,
    block_ctx: BlockCtx,
    cfl_ds_filter_index: u8,
    sb_mib: usize,
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
    ensure_supported_chroma_capability(mode, block_ctx)?;
    Ok(RectChromaPlan::Mode(mode))
}

fn rect_chroma_middle_p_angle_for_parts(
    mode: SupportedChromaMode,
    follow_directional_p_angle: Option<u16>,
    block_ctx: BlockCtx,
) -> Option<u16> {
    let base = match mode {
        SupportedChromaMode::D135Follow
        | SupportedChromaMode::D113Follow
        | SupportedChromaMode::D157Follow => i32::from(follow_directional_p_angle?),
        SupportedChromaMode::D135 => 135,
        SupportedChromaMode::D113 => 113,
        SupportedChromaMode::D157 => 157,
        _ => return None,
    };
    let block = block_ctx.plane_block(PlaneId::U);
    let width = block.width4().checked_mul(4)?;
    let height = block.height4().checked_mul(4)?;
    let p_angle = u16::try_from(wide_angle_mapped_p_angle(width, height, base)).ok()?;
    (91..=179).contains(&p_angle).then_some(p_angle)
}

fn chroma_plan_for_parts(
    chroma: GeneralIntraChromaBlockMode,
    y_mode: crate::tile_payload::IntraYMode,
    angle_delta_y: i8,
    block_ctx: BlockCtx,
    cfl_ds_filter_index: u8,
    sb_mib: usize,
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
    if block_ctx.block().width4() == block_ctx.block().height4() {
        ensure_supported_chroma_capability(mode, block_ctx)?;
        return Ok(RectChromaPlan::Mode(mode));
    }
    ensure_supported_rect_chroma_capability(mode, block_ctx)?;
    Ok(
        if let Some(p_angle) = rect_chroma_middle_p_angle_for_parts(
            mode,
            directional_p_angle_for_luma(y_mode, angle_delta_y, block_ctx),
            block_ctx,
        ) {
            RectChromaPlan::Middle { p_angle }
        } else {
            RectChromaPlan::Mode(mode)
        },
    )
}

fn cfl_chroma_plan(
    params: Option<crate::tile_payload::CflParams>,
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

pub(super) fn wide_angle_mapped_p_angle(width: usize, height: usize, p_angle: i32) -> i32 {
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
    tile_offset: ByteOffset,
) -> Result<()> {
    if block_ctx.bit_depth() == BitDepth::Eight {
        return Ok(());
    }
    let luma_admitted = modes.luma_is_dc()
        || plan_luma_prediction(modes, block_ctx).is_ok()
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
        ten_bit_general_intra_chroma_admitted(modes.supported_chroma_mode(), block_ctx, frame_n4)
    };
    let luma_admitted = modes.luma_is_dc()
        || plan_luma_prediction(modes, block_ctx).is_ok()
        || rect_luma_plan(modes, block_ctx, false, sb_mib).is_ok();
    if !luma_admitted || !chroma_admitted {
        if crate::trace_flags::trace_flag!("SPLOT_TRACE_GENERAL_INTRA_MODE") {
            let luma_plan = plan_luma_prediction(modes, block_ctx);
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
            "unsupported_10bit_non_dc_intra",
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
    frame_n4: (usize, usize),
) -> bool {
    let neighbours = block_ctx.neighbours(PlaneId::U);
    let has_edge = neighbours.has_above() || neighbours.has_left();
    let chroma_block = block_ctx.plane_block(PlaneId::U);
    let chroma_smooth_shape = chroma_block.width4() >= 1 && chroma_block.height4() >= 1;
    let chroma_cardinal_shape = chroma_block.width4() >= 1 && chroma_block.height4() >= 1;
    let chroma_middle_shape = chroma_block.width4() >= 1 && chroma_block.height4() >= 1;
    let chroma_one_sided_above_shape = chroma_block.width4() >= 1 && chroma_block.height4() >= 1;
    let d135_left_only =
        mode == Some(SupportedChromaMode::D135) && !neighbours.has_above() && neighbours.has_left();
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
        ) => {
            (frame_n4 == (FULL_SB_N4_LUMA, FULL_SB_N4_LUMA) && block_ctx.is_top_left())
                || (chroma_smooth_shape && has_edge)
        }
        Some(SupportedChromaMode::Vertical | SupportedChromaMode::VerticalFollow) => {
            chroma_cardinal_shape && has_edge
        }
        Some(SupportedChromaMode::Horizontal | SupportedChromaMode::HorizontalFollow) => {
            chroma_cardinal_shape && (has_edge || no_neighbour_horizontal_first)
        }
        Some(
            SupportedChromaMode::D113Follow
            | SupportedChromaMode::D113
            | SupportedChromaMode::D157Follow
            | SupportedChromaMode::D157,
        ) => chroma_middle_shape && neighbours.has_left(),
        Some(SupportedChromaMode::D135Follow | SupportedChromaMode::D135) => {
            chroma_middle_shape
                && neighbours.has_left()
                && (neighbours.has_above() || d135_left_only)
        }
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
    work_unit: &mut crate::tile_payload::DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    coeff_ctx: &mut crate::tile_payload::TileCoeffContextState,
    workspace: &mut CurrentFrameWorkspace<T>,
    block_ctx: BlockCtx,
    block_decoded: &mut crate::tile_payload::TileBlockDecodedState,
    deblock_blocks: &mut Vec<super::deblock::DeblockBlock>,
    uv_mode: usize,
    luma_transform_type_context: LumaTransformTypeContext,
    luma_tx_partition_context: Option<LumaTransformPartitionContext>,
    transform_tool_residual_policy: TransformToolResidualPolicy,
    qindex: u32,
    intra_edge: super::intra_edge::IntraEdgeCtx,
    tile_offset: ByteOffset,
) -> Result<()> {
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
        )
        .map_err(|error| general_intra_residual_error(error, tile_offset))?;
    let block = block_ctx.block();
    let transforms = residual_plan.transforms();
    deblock_blocks.push(super::deblock::DeblockBlock {
        r: block.row4(),
        c: block.col4(),
        n4w: block.width4(),
        n4h: block.height4(),
        luma_tx: transforms.luma_tx(),
        chroma_tx: transforms.chroma_tx(),
        qindex,
        skip: false,
    });
    Ok(())
}

fn map_general_intra_multiblock_error(
    error: crate::tile_payload::GeneralIntraMultiblockError<DecodeError>,
    tile_offset: ByteOffset,
) -> DecodeError {
    use crate::tile_payload::{GeneralIntraMultiblockError, GeneralIntraTreeWalkError};
    match error {
        GeneralIntraMultiblockError::Setup(error) => {
            general_intra_partition_frontier_error(error, tile_offset)
        }
        GeneralIntraMultiblockError::Walk(GeneralIntraTreeWalkError::Leaf(error)) => error,
        GeneralIntraMultiblockError::Walk(GeneralIntraTreeWalkError::Traversal(
            TilePartitionTraversalError::Limit(source),
        )) => DecodeError::Limit { source },
        GeneralIntraMultiblockError::Walk(_) => general_intra_at!(
            "general_intra_partition_walk",
            tile_offset,
            missing_capability_message!("intra.partition.walk", path = "unsupported"),
            GENERAL_INTRA_PARTITION_SPEC_SECTION,
        ),
    }
}

#[derive(Clone, Copy)]
struct ChromaCapabilityUnsupported {
    reason_id: &'static str,
    message: &'static str,
}

fn ensure_supported_chroma_capability(
    mode: SupportedChromaMode,
    block_ctx: BlockCtx,
) -> core::result::Result<(), ChromaCapabilityUnsupported> {
    let n4w = block_ctx.block().width4();
    let neighbours = block_ctx.neighbours(PlaneId::U);
    let chroma_block = block_ctx.plane_block(PlaneId::U);
    let full_sb = n4w == FULL_SB_N4_LUMA;
    let has_edge = neighbours.has_above() || neighbours.has_left();
    let above_left = full_sb && neighbours.has_above() && neighbours.has_left();
    let left_only = full_sb && !neighbours.has_above() && neighbours.has_left();
    let smooth_subblock =
        chroma_block.width4() >= 1 && chroma_block.height4() >= 1 && !neighbours.is_top_left();
    let cardinal_subblock = chroma_block.width4() >= 1 && chroma_block.height4() >= 1;
    let paeth_subblock = chroma_block.width4() >= 1 && chroma_block.height4() >= 1;
    let middle_subblock = chroma_block.width4() >= 1
        && chroma_block.height4() >= 1
        && neighbours.has_above()
        && neighbours.has_left();
    let one_sided_above_subblock = chroma_block.width4() >= 1
        && chroma_block.height4() >= 1
        && ((neighbours.has_above() && neighbours.num_above_right() > 0)
            || (!neighbours.has_above() && neighbours.has_left()));
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
            if left_only || above_left || middle_subblock =>
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
            if above_left || middle_subblock =>
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
            if supported_smooth_shape && (neighbours.has_above() || neighbours.has_left()) =>
        {
            Ok(())
        }
        mode if rect_chroma_is_middle_directional(mode)
            && supported_middle_shape
            && (neighbours.has_above()
                || matches!(
                    mode,
                    SupportedChromaMode::D113Follow
                        | SupportedChromaMode::D113
                        | SupportedChromaMode::D157Follow
                        | SupportedChromaMode::D157
                        | SupportedChromaMode::D135
                ))
            && neighbours.has_left() =>
        {
            Ok(())
        }
        mode if rect_chroma_is_one_sided_above_directional(mode)
            && supported_smooth_shape
            && ((neighbours.has_above() && neighbours.num_above_right() > 0)
                || (!neighbours.has_above() && neighbours.has_left())) =>
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
        GeneralIntraResidualError::MissingCardinalEdge => general_intra_at!(
            "general_intra_cardinal_missing_edge",
            offset,
            missing_capability_message!("intra.luma.cardinal.edge", neighbour = "missing"),
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
        GeneralIntraBlockModeError::UnsupportedMhccpMode => general_intra_at!(
            "general_intra_unsupported_mhccp_mode",
            offset,
            missing_capability_message!("intra.chroma.mhccp", mode = "active"),
            "5.20.5.6",
        ),
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

#[allow(clippy::needless_pass_by_value)]
fn general_intra_partition_frontier_error(
    error: MinimalRuntimePartitionFrontierError,
    offset: ByteOffset,
) -> DecodeError {
    match error {
        MinimalRuntimePartitionFrontierError::Limit(source)
        | MinimalRuntimePartitionFrontierError::Traversal(TilePartitionTraversalError::Limit(
            source,
        )) => DecodeError::Limit { source },
        MinimalRuntimePartitionFrontierError::MissingFact { .. }
        | MinimalRuntimePartitionFrontierError::MiSizeState(_)
        | MinimalRuntimePartitionFrontierError::IntraJointModeState(_)
        | MinimalRuntimePartitionFrontierError::UsesMrlsState(_)
        | MinimalRuntimePartitionFrontierError::FscModeState(_)
        | MinimalRuntimePartitionFrontierError::UvCflState(_)
        | MinimalRuntimePartitionFrontierError::LumaPaletteState(_)
        | MinimalRuntimePartitionFrontierError::Traversal(_)
        | MinimalRuntimePartitionFrontierError::UnexpectedFrontier { .. } => {
            general_intra_at!(
                "general_intra_partition_frontier",
                offset,
                missing_capability_message!(
                    "intra.partition.frontier",
                    shape = "non_single_block_root",
                ),
                GENERAL_INTRA_PARTITION_SPEC_SECTION,
            )
        }
    }
}

fn general_intra_unsupported(
    reason: &'static str,
    byte_offset: Option<ByteOffset>,
    message: &'static str,
    spec_section: &'static str,
) -> DecodeError {
    DecodeError::UnsupportedFeature {
        unsupported: Box::new(DecodeUnsupportedFeature::new(
            reason,
            GENERAL_INTRA_TIER_ID,
            GENERAL_INTRA_MATRIX_ROW,
            GENERAL_INTRA_FEATURE_ID,
            spec_section,
            message,
            GENERAL_INTRA_REMEDIATION,
            byte_offset,
        )),
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    fn ctx(row4: usize, col4: usize, width4: usize, height4: usize) -> BlockCtx {
        BlockCtx::new(
            BlockRect::new(row4, col4, width4, height4),
            TxShape::from_luma_4x4(width4, height4).expect("valid transform shape"),
            480,
            270,
            BitDepth::Ten,
            ChromaSampling::Yuv420,
        )
    }

    #[test]
    fn admits_10bit_vertical_chroma_with_left_only_edge() {
        let first_row_second_sb = ctx(0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA);

        assert!(
            ensure_supported_chroma_capability(SupportedChromaMode::Vertical, first_row_second_sb)
                .is_ok()
        );
        assert!(ten_bit_general_intra_chroma_admitted(
            Some(SupportedChromaMode::Vertical),
            first_row_second_sb,
            (480, 270)
        ));
    }

    #[test]
    fn admits_10bit_horizontal_chroma_with_above_only_edge() {
        let first_col_second_row = ctx(FULL_SB_N4_LUMA, 0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA);

        assert!(
            ensure_supported_chroma_capability(
                SupportedChromaMode::HorizontalFollow,
                first_col_second_row
            )
            .is_ok()
        );
        assert!(ten_bit_general_intra_chroma_admitted(
            Some(SupportedChromaMode::HorizontalFollow),
            first_col_second_row,
            (480, 270)
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
            first_row_wide_block,
            (480, 270)
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
            first_row_rect_block,
            (480, 270)
        ));
    }

    #[test]
    fn admits_10bit_small_rect_smooth_chroma_with_above_left_edges() {
        let rect_block = ctx(24, 200, 2, 4);

        assert!(
            ensure_supported_rect_chroma_capability(SupportedChromaMode::Smooth, rect_block)
                .is_ok()
        );
        assert!(ten_bit_general_intra_chroma_admitted(
            Some(SupportedChromaMode::Smooth),
            rect_block,
            (480, 270)
        ));
    }

    #[test]
    fn admits_square_smooth_chroma_subblock_with_above_left_edges() {
        let square_block = ctx(24, 200, 8, 8);

        assert!(
            ensure_supported_chroma_capability(SupportedChromaMode::Smooth, square_block).is_ok()
        );
        assert!(ten_bit_general_intra_chroma_admitted(
            Some(SupportedChromaMode::Smooth),
            square_block,
            (480, 270)
        ));
    }

    #[test]
    fn admits_small_vertical_follow_chroma_with_above_edge() {
        let small_block = ctx(20, 218, 2, 2);

        assert!(
            ensure_supported_chroma_capability(SupportedChromaMode::VerticalFollow, small_block)
                .is_ok()
        );
        assert!(ten_bit_general_intra_chroma_admitted(
            Some(SupportedChromaMode::VerticalFollow),
            small_block,
            (480, 270)
        ));
    }

    #[test]
    fn admits_rect_vertical_follow_chroma_with_left_only_edge() {
        let first_row_rect_block = ctx(0, 416, 8, FULL_SB_N4_LUMA);

        assert!(
            ensure_supported_chroma_capability(
                SupportedChromaMode::VerticalFollow,
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
            first_row_rect_block,
            (480, 270)
        ));
    }

    #[test]
    fn admits_small_d135_follow_chroma_with_above_left_edges() {
        let small_block = ctx(24, 200, 8, 8);

        assert!(
            ensure_supported_chroma_capability(SupportedChromaMode::D135Follow, small_block)
                .is_ok()
        );
        assert!(ten_bit_general_intra_chroma_admitted(
            Some(SupportedChromaMode::D135Follow),
            small_block,
            (480, 270)
        ));
    }

    #[test]
    fn admits_small_d157_follow_chroma_with_above_left_edges() {
        let small_block = ctx(16, 416, 8, 8);

        assert!(
            ensure_supported_chroma_capability(SupportedChromaMode::D157Follow, small_block)
                .is_ok()
        );
        assert!(ten_bit_general_intra_chroma_admitted(
            Some(SupportedChromaMode::D157Follow),
            small_block,
            (480, 270)
        ));
    }

    #[test]
    fn admits_small_d67_follow_chroma_with_above_right_edge() {
        let small_block = ctx(28, 216, 2, 4);

        assert!(
            ensure_supported_chroma_capability(SupportedChromaMode::D67Follow, small_block).is_ok()
        );
        assert!(
            ensure_supported_rect_chroma_capability(SupportedChromaMode::D67Follow, small_block)
                .is_ok()
        );
        assert!(ten_bit_general_intra_chroma_admitted(
            Some(SupportedChromaMode::D67Follow),
            small_block,
            (480, 270)
        ));
    }

    #[test]
    fn admits_d67_follow_chroma_with_above_only_edge() {
        let first_col_block = ctx(128, 0, 32, 32);

        assert!(
            ensure_supported_chroma_capability(SupportedChromaMode::D67Follow, first_col_block)
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
            first_col_block,
            (480, 270)
        ));
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
            rect_block,
            (480, 270)
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
            rect_block,
            (480, 270)
        ));
    }

    #[test]
    fn admits_10bit_square_paeth_chroma_with_left_only_edge() {
        let first_row_second_sb = ctx(0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA);

        assert!(
            ensure_supported_chroma_capability(SupportedChromaMode::Paeth, first_row_second_sb)
                .is_ok()
        );
        assert!(ten_bit_general_intra_chroma_admitted(
            Some(SupportedChromaMode::Paeth),
            first_row_second_sb,
            (480, 270)
        ));
    }

    #[test]
    fn admits_10bit_square_paeth_chroma_subblock_with_above_left_edges() {
        let square_subblock = ctx(40, 160, 8, 8);

        assert!(
            ensure_supported_chroma_capability(SupportedChromaMode::Paeth, square_subblock).is_ok()
        );
        assert!(ten_bit_general_intra_chroma_admitted(
            Some(SupportedChromaMode::Paeth),
            square_subblock,
            (480, 270)
        ));
    }

    #[test]
    fn admits_10bit_small_d203_follow_chroma_subblock() {
        let d203_subblock = ctx(32, 300, 2, 8);

        assert!(
            ensure_supported_chroma_capability(SupportedChromaMode::D203Follow, d203_subblock)
                .is_ok()
        );
        assert!(ten_bit_general_intra_chroma_admitted(
            Some(SupportedChromaMode::D203Follow),
            d203_subblock,
            (480, 270)
        ));
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
            mode: crate::tile_payload::SupportedNonDcLumaMode::SmoothVertical,
            use_tcq: false,
        };

        assert_eq!(
            rect_luma_plan_for_parts(
                Some(crate::tile_payload::SupportedNonDcLumaMode::SmoothVertical),
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
            mode: crate::tile_payload::SupportedNonDcLumaMode::Smooth,
            use_tcq: false,
        };

        assert_eq!(
            rect_luma_plan_for_parts(
                Some(crate::tile_payload::SupportedNonDcLumaMode::Smooth),
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
            mode: crate::tile_payload::SupportedNonDcLumaMode::Smooth,
            use_tcq: false,
        };

        assert_eq!(
            rect_luma_plan_for_parts(
                Some(crate::tile_payload::SupportedNonDcLumaMode::Smooth),
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
            mode: crate::tile_payload::SupportedNonDcLumaMode::SmoothHorizontal,
            use_tcq: false,
        };

        assert_eq!(
            rect_luma_plan_for_parts(
                Some(crate::tile_payload::SupportedNonDcLumaMode::SmoothHorizontal),
                None,
                false,
                rect_block,
                false,
            ),
            Ok(mode)
        );
    }

    #[test]
    fn admits_small_rect_paeth_luma_with_above_left_edges() {
        let rect_block = ctx(18, 220, 4, 2);

        assert_eq!(
            rect_luma_plan_for_parts_ext(true, None, None, false, rect_block, false),
            Ok(RectLumaPlan::Paeth { use_tcq: false })
        );
    }

    #[test]
    fn admits_small_rect_d135_mrl_middle_luma_with_above_left_edges() {
        let rect_block = ctx(20, 216, 1, 4);

        assert_eq!(
            rect_luma_mrl_plan_for_parts(
                crate::tile_payload::IntraYMode::D135_PRED_FOR_TEST,
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
                crate::tile_payload::IntraYMode::V_PRED_FOR_TEST,
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
                crate::tile_payload::IntraYMode::D45_PRED_FOR_TEST,
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
                crate::tile_payload::IntraYMode::D157_PRED_FOR_TEST,
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
                crate::tile_payload::IntraYMode::D113_PRED_FOR_TEST,
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
                crate::tile_payload::IntraYMode::D67_PRED_FOR_TEST,
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
                crate::tile_payload::IntraYMode::D113_PRED_FOR_TEST,
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
        let modes =
            GeneralIntraBlockModes::luma_only(crate::tile_payload::GeneralIntraLumaBlockMode {
                y_mode: crate::tile_payload::IntraYMode::D67_PRED_FOR_TEST,
                angle_delta_y: -1,
                intra_joint_mode: 0,
                mrl_index: 0,
                mrl_sec_index: None,
                fsc_mode: 0,
                uses_mrls: 0,
            });

        assert!(plan_luma_prediction(&modes, first_col_block).is_err());
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
            32
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
            rect_block,
            (480, 270)
        ));
        assert_eq!(
            rect_chroma_middle_p_angle_for_parts(
                SupportedChromaMode::D135Follow,
                Some(126),
                rect_block,
            ),
            Some(126)
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
            first_row_rect_block,
            (480, 270)
        ));
        assert_eq!(
            rect_chroma_middle_p_angle_for_parts(
                SupportedChromaMode::D113Follow,
                Some(110),
                first_row_rect_block,
            ),
            Some(110)
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
            first_row_rect_block,
            (480, 270)
        ));
        assert_eq!(
            rect_chroma_middle_p_angle_for_parts(
                SupportedChromaMode::D157Follow,
                Some(154),
                first_row_rect_block,
            ),
            Some(154)
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
            first_row_rect_block,
            (480, 270)
        ));
        assert_eq!(
            rect_chroma_middle_p_angle_for_parts(
                SupportedChromaMode::D135,
                None,
                first_row_rect_block,
            ),
            Some(135)
        );
    }

    #[test]
    fn admits_top_row_rect_d45_follow_chroma_with_left_only_edge() {
        let first_row_rect_block = ctx(0, 352, 32, 16);

        assert!(
            ensure_supported_rect_chroma_capability(
                SupportedChromaMode::D45Follow,
                first_row_rect_block,
            )
            .is_ok()
        );
        assert!(ten_bit_general_intra_chroma_admitted(
            Some(SupportedChromaMode::D45Follow),
            first_row_rect_block,
            (480, 270)
        ));
    }

    #[test]
    fn keeps_rect_middle_chroma_left_only_gated() {
        let first_row_rect_block = ctx(0, 320, 8, FULL_SB_N4_LUMA);

        assert!(
            ensure_supported_rect_chroma_capability(
                SupportedChromaMode::D135Follow,
                first_row_rect_block,
            )
            .is_err()
        );
        assert!(!ten_bit_general_intra_chroma_admitted(
            Some(SupportedChromaMode::D135Follow),
            first_row_rect_block,
            (480, 270)
        ));
    }

    #[test]
    fn keeps_10bit_vertical_chroma_top_left_gated() {
        let top_left = ctx(0, 0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA);

        assert!(!ten_bit_general_intra_chroma_admitted(
            Some(SupportedChromaMode::Vertical),
            top_left,
            (480, 270)
        ));
    }
}

// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::headers::sequence::BitDepthIdc;
use splot_recon::{BitDepth, CurrentFrameWorkspace, PlaneId, ReconSample};

use super::block_context::{BlockCtx, BlockRect, ChromaSampling, TxShape};
use super::capability::missing_capability_message;
use super::intra_prediction::{IntraLumaUnsupported, plan_luma_prediction};
use super::residual_pipeline::{GeneralIntraResidualPlan, ResidualPipelineUnsupported};
use super::*;
use crate::tile_payload::{GeneralIntraBlockModes, GeneralIntraLeafMode, SupportedChromaMode};

const FULL_SB_N4_LUMA: usize = 16;

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
    options: DecodeOptions,
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
    Ok(MinimalRuntimeFrame {
        frame,
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
         _is_cfl_ctx,
         block_decoded| {
            decode_one_general_intra_block::<T>(
                work_unit,
                symbols,
                frontier,
                joint_modes,
                uses_mrls,
                fsc_modes,
                block_decoded,
                &mut workspace,
                &mut coeff_ctx,
                &mut deblock_blocks,
                qindex,
                luma_use_tcq,
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
fn decode_one_general_intra_block<T: ReconSample>(
    work_unit: &mut crate::tile_payload::DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    frontier: &crate::tile_payload::DecodeBlockFrontier,
    joint_modes: &crate::tile_payload::TileIntraJointModeState,
    uses_mrls: &crate::tile_payload::TileUsesMrlsState,
    fsc_modes: &crate::tile_payload::TileFscModeState,
    block_decoded: &crate::tile_payload::TileBlockDecodedState,
    workspace: &mut CurrentFrameWorkspace<T>,
    coeff_ctx: &mut crate::tile_payload::TileCoeffContextState,
    deblock_blocks: &mut Vec<super::deblock::DeblockBlock>,
    qindex: u32,
    luma_use_tcq: bool,
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
    if n4w < 2 || n4h < 2 {
        return Err(general_intra_at!(
            "general_intra_sub_8x8_block",
            tile_offset,
            missing_capability_message!("intra.block.size", block = "sub_8x8"),
            GENERAL_INTRA_PARTITION_SPEC_SECTION,
        ));
    }
    if !frontier.has_chroma {
        return Err(general_intra_at!(
            "general_intra_luma_only_block",
            tile_offset,
            missing_capability_message!("intra.chroma.presence", chroma = "absent"),
            GENERAL_INTRA_PARTITION_SPEC_SECTION,
        ));
    }
    let Some(block_tx_shape) = TxShape::from_luma_4x4(n4w, n4h) else {
        return Err(geometry_error());
    };
    let block_ctx = BlockCtx::new(
        BlockRect::new(frontier.r, frontier.c, n4w, n4h),
        block_tx_shape,
        mi_cols,
        mi_rows,
        bit_depth,
        ChromaSampling::Yuv420,
    );

    let modes = crate::tile_payload::decode_general_intra_block_modes(
        work_unit,
        symbols,
        crate::tile_payload::GeneralIntraChromaToolConfig::disabled(),
        joint_modes,
        uses_mrls,
        fsc_modes,
        0,
        frontier.b_size.index(),
        frontier.r,
        frontier.c,
        n4w,
        n4h,
    )
    .map_err(|error| general_intra_block_mode_error(error, tile_offset))?;
    if modes.uses_active_mrl() {
        return Err(general_intra_at!(
            "general_intra_unsupported_mrl_mode",
            tile_offset,
            missing_capability_message!("intra.luma.mrl", mode = "active"),
            "7.13.2",
        ));
    }
    if modes.uses_active_fsc() {
        return Err(general_intra_at!(
            "general_intra_unsupported_fsc_mode",
            tile_offset,
            missing_capability_message!("intra.transform.fsc", mode = "active"),
            "5.20.7.27",
        ));
    }
    ensure_10bit_general_intra_capability(&modes, block_ctx, (mi_cols, mi_rows), tile_offset)?;

    if n4w != n4h {
        return decode_one_general_intra_rect_block::<T>(
            work_unit,
            symbols,
            frontier.has_chroma,
            &modes,
            workspace,
            coeff_ctx,
            deblock_blocks,
            qindex,
            luma_use_tcq,
            block_ctx,
            block_decoded,
            tile_offset,
        );
    }

    let Some(supported_chroma) = modes.supported_chroma_mode() else {
        return Err(general_intra_at!(
            "general_intra_non_dc_chroma_mode",
            tile_offset,
            missing_capability_message!("intra.chroma.mode", mode = "unsupported_non_dc"),
            GENERAL_INTRA_MODE_SPEC_SECTION,
        ));
    };
    ensure_supported_chroma_capability(supported_chroma, block_ctx)
        .map_err(|error| general_intra_chroma_capability_error(error, tile_offset))?;
    let luma_plan = plan_luma_prediction(&modes, block_ctx)
        .map_err(|error| general_intra_luma_plan_error(error, tile_offset))?;

    let residual_plan = GeneralIntraResidualPlan::square(
        block_ctx,
        luma_plan,
        frontier.has_chroma.then_some(supported_chroma),
        luma_use_tcq,
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
        qindex,
        tile_offset,
    )?;

    Ok(leaf_mode(&modes))
}
#[allow(clippy::too_many_arguments)]
fn decode_one_general_intra_rect_block<T: ReconSample>(
    work_unit: &mut crate::tile_payload::DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    has_chroma: bool,
    modes: &GeneralIntraBlockModes,
    workspace: &mut CurrentFrameWorkspace<T>,
    coeff_ctx: &mut crate::tile_payload::TileCoeffContextState,
    deblock_blocks: &mut Vec<super::deblock::DeblockBlock>,
    qindex: u32,
    luma_use_tcq: bool,
    block_ctx: BlockCtx,
    block_decoded: &crate::tile_payload::TileBlockDecodedState,
    tile_offset: ByteOffset,
) -> Result<GeneralIntraLeafMode> {
    let block = block_ctx.block();
    if (block.width4(), block.height4()) != (FULL_SB_N4_LUMA, FULL_SB_N4_LUMA / 2) {
        return Err(general_intra_at!(
            "general_intra_rect_unverified_geometry",
            tile_offset,
            missing_capability_message!(
                "intra.rect.geometry",
                block = "not_64x32",
                partition = "non_horz",
            ),
            GENERAL_INTRA_PARTITION_SPEC_SECTION,
        ));
    }
    if !modes.luma_is_dc() {
        return Err(general_intra_at!(
            "general_intra_rect_non_dc_luma",
            tile_offset,
            missing_capability_message!("intra.rect.luma_mode", mode = "non_dc"),
            GENERAL_INTRA_MODE_SPEC_SECTION,
        ));
    }
    if modes.supported_chroma_mode() != Some(SupportedChromaMode::Dc) {
        return Err(general_intra_at!(
            "general_intra_rect_non_dc_chroma",
            tile_offset,
            missing_capability_message!("intra.rect.chroma_mode", mode = "non_dc"),
            GENERAL_INTRA_MODE_SPEC_SECTION,
        ));
    }

    let residual_plan = GeneralIntraResidualPlan::rect(block_ctx, has_chroma, luma_use_tcq)
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
        qindex,
        tile_offset,
    )?;
    Ok(leaf_mode(modes))
}

fn leaf_mode(modes: &GeneralIntraBlockModes) -> GeneralIntraLeafMode {
    GeneralIntraLeafMode::luma(
        modes.intra_joint_mode,
        modes.y_mode,
        modes.angle_delta_y,
        modes.fsc_mode,
        modes.uses_mrls,
    )
}

fn ensure_10bit_general_intra_capability(
    modes: &GeneralIntraBlockModes,
    block_ctx: BlockCtx,
    frame_n4: (usize, usize),
    tile_offset: ByteOffset,
) -> Result<()> {
    if block_ctx.bit_depth() == BitDepth::Eight {
        return Ok(());
    }
    let chroma_admitted = match modes.supported_chroma_mode() {
        Some(SupportedChromaMode::Dc) => true,
        Some(SupportedChromaMode::Smooth) => {
            frame_n4 == (FULL_SB_N4_LUMA, FULL_SB_N4_LUMA) && block_ctx.is_top_left()
        }
        _ => false,
    };
    if !modes.luma_is_dc() || !chroma_admitted {
        return Err(general_intra_at!(
            "unsupported_10bit_non_dc_intra",
            tile_offset,
            missing_capability_message!("intra.10bit.non_dc", luma = "non_dc_or_chroma_neighbour",),
            GENERAL_INTRA_MODE_SPEC_SECTION,
        ));
    }
    let block = block_ctx.block();
    if block.width4() != FULL_SB_N4_LUMA || block.height4() != FULL_SB_N4_LUMA {
        return Err(general_intra_at!(
            "unsupported_10bit_non_64x64_leaf",
            tile_offset,
            missing_capability_message!("intra.10bit.leaf_shape", block = "non_64x64"),
            GENERAL_INTRA_MODE_SPEC_SECTION,
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn execute_general_intra_residual_plan<T: ReconSample>(
    residual_plan: GeneralIntraResidualPlan,
    work_unit: &mut crate::tile_payload::DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    coeff_ctx: &mut crate::tile_payload::TileCoeffContextState,
    workspace: &mut CurrentFrameWorkspace<T>,
    block_ctx: BlockCtx,
    block_decoded: &crate::tile_payload::TileBlockDecodedState,
    deblock_blocks: &mut Vec<super::deblock::DeblockBlock>,
    uv_mode: usize,
    qindex: u32,
    tile_offset: ByteOffset,
) -> Result<()> {
    residual_plan
        .execute(
            work_unit,
            symbols,
            coeff_ctx,
            workspace,
            block_ctx,
            block_decoded,
            uv_mode,
            qindex,
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
    let full_sb = n4w == FULL_SB_N4_LUMA;
    let above_left = full_sb && neighbours.has_above() && neighbours.has_left();
    let left_only = full_sb && !neighbours.has_above() && neighbours.has_left();
    match mode {
        SupportedChromaMode::Dc | SupportedChromaMode::D203Follow | SupportedChromaMode::D203 => {
            Ok(())
        }
        SupportedChromaMode::Smooth
        | SupportedChromaMode::SmoothVertical
        | SupportedChromaMode::SmoothHorizontal
            if full_sb =>
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
        SupportedChromaMode::D135Follow | SupportedChromaMode::D135 if left_only || above_left => {
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
        SupportedChromaMode::D113Follow | SupportedChromaMode::D113 if above_left => Ok(()),
        SupportedChromaMode::D113Follow | SupportedChromaMode::D113 => Err(unsupported_chroma(
            "general_intra_directional_d113_chroma_neighbour",
            missing_capability_message!(
                "intra.chroma.directional.d113",
                neighbour = "above_left",
                block = "non_full_sb_or_edge",
            ),
        )),
        SupportedChromaMode::D157Follow if left_only => Ok(()),
        SupportedChromaMode::D157Follow => Err(unsupported_chroma(
            "general_intra_directional_d157_chroma_neighbour",
            missing_capability_message!(
                "intra.chroma.directional.d157",
                neighbour = "left_only",
                block = "non_full_sb_or_not_first_row",
            ),
        )),
        SupportedChromaMode::D157 => Err(unsupported_chroma(
            "general_intra_directional_d157_chroma_explicit",
            missing_capability_message!(
                "intra.chroma.directional.d157",
                neighbour = "above_left",
                block = "full_recon_only",
            ),
        )),
        SupportedChromaMode::D45Follow
        | SupportedChromaMode::D45
        | SupportedChromaMode::D67Follow
        | SupportedChromaMode::D67
            if above_left && neighbours.num_above_right() > 0 =>
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
        SupportedChromaMode::VerticalFollow | SupportedChromaMode::Vertical
            if full_sb && neighbours.has_above() =>
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
        SupportedChromaMode::HorizontalFollow if full_sb && neighbours.has_left() => Ok(()),
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
        SupportedChromaMode::Paeth => Err(unsupported_chroma(
            "general_intra_paeth_chroma",
            missing_capability_message!(
                "intra.chroma.paeth",
                neighbour = "above_left",
                block = "full_recon_only",
            ),
        )),
    }
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
        GeneralIntraResidualError::UnsupportedTransformToolResidual { .. } => {
            general_intra_at!(
                "general_intra_transform_tool_residual",
                offset,
                missing_capability_message!("intra.residual.transform_tools", residual = "nonzero"),
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

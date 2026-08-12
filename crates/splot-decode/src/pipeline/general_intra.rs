// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_recon::{
    BitDepth, CurrentFrameWorkspace, DpcmDirection, IntraCardinalDirection, IntraPredictionScratch,
    PlaneId, ReconSample,
};

use super::*;
use crate::bitstream::tile_payload::{
    BlockSize, BlockSymbolTraceReadError, CflParams, CoeffParseErrorClass, GeneralIntraBlockModes,
    GeneralIntraChromaBlockMode, GeneralIntraChromaModeContext, GeneralIntraChromaToolConfig,
    GeneralIntraLeafMode, IntraYMode, IntraYModeClass, IsCflContext, LumaTransformPartitionContext,
    LumaTransformTypeContext, MrlSelection, SupportedChromaMode, TileCoeffStateError,
    TransformToolResidualPolicy, classify_coeff_parse_error, read_lossless_luma_tx_size,
};
use crate::residual::pipeline::{
    GeneralIntraResidualPlan, ParsedGeneralIntraResidual, RectChromaPlan, RectLumaPlan,
    ResidualPlanError,
};
use crate::support::capability::missing_capability_message;
use crate::tile::block_context::{BlockCtx, BlockRect, ChromaSampling, TxShape};

const MI_SIZE: usize = 4;
const ANGLE_STEP: i32 = 3;
pub(crate) const MRL_INDEX_TO_DELTA: [i32; 4] = [0, 1, -1, 0];
const WAIP_WH_RATIO_THRESHOLDS: [(usize, i32); 4] = [(2, 61), (4, 73), (8, 82), (16, 86)];
const HOT_INTRA_VECTOR_COUNT: usize = 4;
const MAX_INTRA_BLOCK_SAMPLES: usize = 128 * 128;
const MAX_INTRA_EDGE_SAMPLES: usize = 128 + 32;

struct HotIntraVectors {
    slots: [Option<Box<dyn std::any::Any + Send>>; HOT_INTRA_VECTOR_COUNT],
}

impl Default for HotIntraVectors {
    fn default() -> Self {
        Self {
            slots: core::array::from_fn(|_| None),
        }
    }
}

impl HotIntraVectors {
    fn with_sample_capacity<T: Send + 'static>(capacity: usize) -> Self {
        Self {
            slots: core::array::from_fn(|_| {
                Some(Box::new(Vec::<T>::with_capacity(capacity)) as Box<dyn std::any::Any + Send>)
            }),
        }
    }
}

std::thread_local! {
    static HOT_INTRA_VECTORS: std::cell::RefCell<
        [Option<Box<dyn std::any::Any + Send>>; HOT_INTRA_VECTOR_COUNT]
    > = std::cell::RefCell::new(core::array::from_fn(|_| None));
}

pub(crate) struct RecycledIntraSamples<T: Send + 'static>(Vec<T>);

impl<T: Clone + Send + 'static> RecycledIntraSamples<T> {
    pub(crate) fn filled(len: usize, value: T) -> Self {
        let mut samples =
            HOT_INTRA_VECTORS.with(crate::support::reusable_scratch::take_reusable_vec);
        samples.clear();
        samples.resize(len, value);
        Self(samples)
    }

    pub(crate) fn with_capacity(capacity: usize) -> Self {
        let mut samples =
            HOT_INTRA_VECTORS.with(crate::support::reusable_scratch::take_reusable_vec);
        samples.clear();
        if samples.capacity() < capacity {
            samples.reserve(capacity);
        }
        Self(samples)
    }
}

impl<T: Send + 'static> std::ops::Deref for RecycledIntraSamples<T> {
    type Target = Vec<T>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: Send + 'static> std::ops::DerefMut for RecycledIntraSamples<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T: Send + 'static> Drop for RecycledIntraSamples<T> {
    fn drop(&mut self) {
        HOT_INTRA_VECTORS.with(|cell| {
            crate::support::reusable_scratch::recycle_reusable_vec(cell, &mut self.0);
        });
    }
}

pub(crate) struct GeneralIntraReconScratch<T: ReconSample> {
    intra_prediction: IntraPredictionScratch<T>,
    hot_vectors: HotIntraVectors,
    pub(crate) cfl_luma_ac: Vec<i32>,
    pub(crate) cfl_prediction: Vec<T>,
    pub(crate) mhccp_refs: [Vec<u16>; 2],
    pub(crate) paeth_edges: [Vec<T>; 2],
    pub(crate) cctx_u_prediction: Vec<T>,
    pub(crate) cctx_v_prediction: Vec<T>,
    pub(crate) cctx_u_output: Vec<T>,
    pub(crate) cctx_v_output: Vec<T>,
}

impl<T: ReconSample> Default for GeneralIntraReconScratch<T> {
    fn default() -> Self {
        Self {
            intra_prediction: IntraPredictionScratch::with_capacity(MAX_INTRA_BLOCK_SAMPLES),
            hot_vectors: HotIntraVectors::with_sample_capacity::<T>(MAX_INTRA_EDGE_SAMPLES),
            cfl_luma_ac: Vec::with_capacity(MAX_INTRA_BLOCK_SAMPLES),
            cfl_prediction: Vec::with_capacity(MAX_INTRA_BLOCK_SAMPLES),
            mhccp_refs: core::array::from_fn(|_| Vec::with_capacity(MAX_INTRA_EDGE_SAMPLES)),
            paeth_edges: core::array::from_fn(|_| Vec::with_capacity(MAX_INTRA_EDGE_SAMPLES)),
            cctx_u_prediction: Vec::with_capacity(MAX_INTRA_BLOCK_SAMPLES),
            cctx_v_prediction: Vec::with_capacity(MAX_INTRA_BLOCK_SAMPLES),
            cctx_u_output: Vec::with_capacity(MAX_INTRA_BLOCK_SAMPLES),
            cctx_v_output: Vec::with_capacity(MAX_INTRA_BLOCK_SAMPLES),
        }
    }
}

pub(crate) struct GeneralIntraReconCommand {
    residual: ParsedGeneralIntraResidual,
    qindex: u32,
    intra_edge: crate::prediction::intra_edge::IntraEdgeCtx,
    luma_transform_type_context: LumaTransformTypeContext,
    segment_id: u8,
    tile_offset: ByteOffset,
}

impl GeneralIntraReconCommand {
    pub(crate) fn reconstruct<T: ReconSample>(
        self,
        scratch: &mut GeneralIntraReconScratch<T>,
        workspace: &mut CurrentFrameWorkspace<T>,
        block_decoded: &mut crate::bitstream::tile_payload::TileBlockDecodedState,
    ) -> Result<()> {
        let _qm_segment_scope = crate::bitstream::tile_payload::FrameQmSegmentScope::install(
            usize::from(self.segment_id),
        );
        workspace.swap_intra_prediction_scratch(&mut scratch.intra_prediction);
        HOT_INTRA_VECTORS.with(|vectors| {
            std::mem::swap(&mut *vectors.borrow_mut(), &mut scratch.hot_vectors.slots);
        });
        let result = self.reconstruct_with_installed_quantizer(scratch, workspace, block_decoded);
        HOT_INTRA_VECTORS.with(|vectors| {
            std::mem::swap(&mut *vectors.borrow_mut(), &mut scratch.hot_vectors.slots);
        });
        workspace.swap_intra_prediction_scratch(&mut scratch.intra_prediction);
        result
    }

    fn reconstruct_with_installed_quantizer<T: ReconSample>(
        self,
        scratch: &mut GeneralIntraReconScratch<T>,
        workspace: &mut CurrentFrameWorkspace<T>,
        block_decoded: &mut crate::bitstream::tile_payload::TileBlockDecodedState,
    ) -> Result<()> {
        self.residual
            .reconstruct(
                scratch,
                workspace,
                block_decoded,
                self.qindex,
                self.intra_edge,
                self.luma_transform_type_context,
            )
            .map_err(|error| general_intra_residual_error(error, self.tile_offset))
    }
}

macro_rules! general_intra_at {
    ($reason:literal, $offset:expr, $message:expr, $spec_section:expr $(,)?) => {
        general_intra_unsupported($reason, Some($offset), $message, $spec_section)
    };
}

macro_rules! general_intra_internal {
    ($reason:literal, $offset:expr $(,)?) => {
        DecodeError::InternalState {
            reason: $reason,
            byte_offset: $offset,
        }
    };
}

fn general_intra_chroma_tools(
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
) -> GeneralIntraChromaToolConfig {
    let chroma_sampling =
        ChromaSampling::from_chroma_format_idc(sequence.general.chroma_format_idc);
    let (subsampling_x, subsampling_y) = chroma_sampling.subsampling(PlaneId::U);

    sequence
        .intra
        .as_ref()
        .map_or(GeneralIntraChromaToolConfig::disabled(), |intra| {
            GeneralIntraChromaToolConfig::new(intra.enable_cfl_intra, intra.enable_mhccp)
                .with_enable_mrls(intra.enable_mrls)
                .with_enable_dip(intra.enable_dip)
        })
        .with_chroma_subsampling(subsampling_x, subsampling_y)
        .with_enable_idtx_intra(
            sequence
                .transform_quant_entropy
                .is_some_and(|tq| tq.enable_idtx_intra),
        )
        .with_allow_screen_content_tools(effective_allow_screen_content_tools(core))
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
pub(crate) fn decode_one_general_intra_block(
    work_unit: &mut crate::bitstream::tile_payload::DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    frontier: &crate::bitstream::tile_payload::DecodeBlockFrontier,
    sequence: &SequenceHeader,
    y_smooth: Option<&crate::prediction::intra_edge::TileYSmoothGrid>,
    mut chroma_smooth: Option<&mut crate::prediction::intra_edge::TileChromaSmoothGrid>,
    core: &FrameHeaderCore,
    joint_modes: &crate::bitstream::tile_payload::TileIntraJointModeState,
    uses_mrls: &crate::bitstream::tile_payload::TileUsesMrlsState,
    use_dip: &crate::bitstream::tile_payload::TileUseDipState,
    fsc_modes: &crate::bitstream::tile_payload::TileFscModeState,
    palette_state: &crate::bitstream::tile_payload::TileLumaPaletteState,
    is_cfl_ctx: IsCflContext,
    segment_id: u8,
    coeff_ctx: &mut crate::bitstream::tile_payload::TileCoeffContextState,
    deblock_blocks: &mut Vec<crate::filters::deblock::DeblockBlock>,
    chroma_deblock_blocks: &mut crate::filters::deblock::ChromaDeblockRecords,
    tx_skip_records: &mut Vec<crate::filters::wienerns_lr::WienerNsLrTxSkipTransformRecord>,
    qindex: u32,
    luma_use_tcq: bool,
    transform_tool_residual_policy: TransformToolResidualPolicy,
    mi_cols: usize,
    mi_rows: usize,
    bit_depth: BitDepth,
    tile_offset: ByteOffset,
) -> Result<(GeneralIntraLeafMode, GeneralIntraReconCommand)> {
    let _qm_segment_scope =
        crate::bitstream::tile_payload::FrameQmSegmentScope::install(usize::from(segment_id));
    let block_tx_shape = TxShape::from_av2_block_size(frontier.b_size);
    let n4w = block_tx_shape.width4();
    let n4h = block_tx_shape.height4();
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
        mi_col_range.start as usize,
        mi_col_range.end as usize,
    );
    let chroma_mode_geometry = if frontier.has_chroma {
        let chroma_ref = frontier.chroma_ref_geometry();
        let chroma_tx_shape = TxShape::from_av2_block_size(chroma_ref.size());
        let chroma_n4w = chroma_tx_shape.width4();
        let chroma_n4h = chroma_tx_shape.height4();
        block_ctx = block_ctx.with_chroma_ref(
            BlockRect::new(chroma_ref.row(), chroma_ref.col(), chroma_n4w, chroma_n4h),
            chroma_tx_shape,
        );
        Some((chroma_n4w, chroma_n4h))
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

    if let crate::bitstream::tile_payload::DecodeBlockPart::ChromaPart {
        y_mode,
        angle_delta_y,
    } = frontier.part()
    {
        let lossless_luma_fsc = lossless
            && fsc_modes
                .fsc_mode_at(frontier.r, frontier.c)
                .is_some_and(|mode| mode != 0);
        return parse_one_general_intra_chroma_part_block(
            intra_edge,
            work_unit,
            symbols,
            frontier,
            y_mode,
            angle_delta_y,
            lossless_luma_fsc,
            chroma_tools,
            is_cfl_ctx,
            cfl_ds_filter_index,
            sb_mib,
            lossless,
            coeff_ctx,
            deblock_blocks,
            chroma_deblock_blocks,
            tx_skip_records,
            chroma_smooth.as_deref_mut(),
            qindex,
            transform_tool_residual_policy,
            block_ctx,
            segment_id,
            tile_offset,
        );
    }

    let luma_only = frontier.is_luma_part() || !frontier.has_chroma;
    let use_neighbor_fsc_context = core.frame_is_intra == Some(true) || !frontier.is_mixed_region();
    let modes = if luma_only {
        let luma =
            crate::bitstream::tile_payload::decode_general_intra_luma_block_mode_with_fsc_context(
                work_unit,
                symbols,
                chroma_tools,
                joint_modes,
                uses_mrls,
                fsc_modes,
                use_neighbor_fsc_context,
                frontier.b_size,
                frontier.r,
                frontier.c,
                n4w,
                n4h,
            )
            .map_err(|error| general_intra_block_mode_error(error, tile_offset))?;
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
        .map_err(|error| general_intra_block_mode_error(error, tile_offset))?;
        let (use_dip_value, dip_transpose, dip_mode) =
            crate::bitstream::tile_payload::read_general_intra_dip_mode_info(
                work_unit,
                symbols,
                chroma_tools,
                use_dip,
                luma.y_mode,
                palette_y,
                frontier.r,
                frontier.c,
                n4w,
                n4h,
            )
            .map_err(|error| general_intra_block_mode_error(error, tile_offset))?;
        GeneralIntraBlockModes::luma_only(luma.with_dip(use_dip_value, dip_transpose, dip_mode))
            .with_palette_y(palette_y)
    } else {
        let (chroma_n4w, chroma_n4h) = chroma_mode_geometry.unwrap_or((n4w, n4h));
        crate::bitstream::tile_payload::decode_general_intra_block_modes_with_fsc_context(
            work_unit,
            symbols,
            chroma_tools,
            joint_modes,
            uses_mrls,
            use_dip,
            fsc_modes,
            use_neighbor_fsc_context,
            palette_state,
            is_cfl_ctx.get(),
            frontier.b_size,
            frontier.r,
            frontier.c,
            n4w,
            n4h,
            chroma_n4w,
            chroma_n4h,
            u32::from(bit_depth.bits()),
        )
        .map_err(|error| general_intra_block_mode_error(error, tile_offset))?
    };
    let luma_lossless_tx_size = if lossless && modes.uses_active_fsc() {
        Some(
            read_lossless_luma_tx_size(work_unit, symbols, frontier.b_size, true, true)
                .map_err(|error| general_intra_block_mode_error(error, tile_offset))?,
        )
    } else {
        None
    };

    let (leaf, command) = parse_one_general_intra_rect_block(
        intra_edge,
        work_unit,
        symbols,
        &modes,
        coeff_ctx,
        deblock_blocks,
        chroma_deblock_blocks,
        tx_skip_records,
        qindex,
        luma_use_tcq,
        lossless,
        luma_lossless_tx_size,
        transform_tool_residual_policy,
        luma_tx_partition_context(frame_tx_mode(core), frontier.b_size, lossless),
        block_ctx,
        cfl_ds_filter_index,
        sb_mib,
        segment_id,
        tile_offset,
    )?;
    if frontier.has_chroma {
        record_chroma_smooth(chroma_smooth, block_ctx, modes.supported_chroma_mode());
    }
    Ok((leaf, command))
}

#[allow(clippy::too_many_arguments)]
fn parse_one_general_intra_chroma_part_block(
    intra_edge: crate::prediction::intra_edge::IntraEdgeCtx,
    work_unit: &mut crate::bitstream::tile_payload::DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    frontier: &crate::bitstream::tile_payload::DecodeBlockFrontier,
    y_mode: crate::bitstream::tile_payload::IntraYMode,
    angle_delta_y: i8,
    lossless_luma_fsc: bool,
    chroma_tools: GeneralIntraChromaToolConfig,
    is_cfl_ctx: IsCflContext,
    cfl_ds_filter_index: u8,
    sb_mib: usize,
    lossless: bool,
    coeff_ctx: &mut crate::bitstream::tile_payload::TileCoeffContextState,
    deblock_blocks: &mut Vec<crate::filters::deblock::DeblockBlock>,
    chroma_deblock_blocks: &mut crate::filters::deblock::ChromaDeblockRecords,
    tx_skip_records: &mut Vec<crate::filters::wienerns_lr::WienerNsLrTxSkipTransformRecord>,
    chroma_smooth: Option<&mut crate::prediction::intra_edge::TileChromaSmoothGrid>,
    qindex: u32,
    transform_tool_residual_policy: TransformToolResidualPolicy,
    block_ctx: BlockCtx,
    segment_id: u8,
    tile_offset: ByteOffset,
) -> Result<(GeneralIntraLeafMode, GeneralIntraReconCommand)> {
    let chroma = crate::bitstream::tile_payload::decode_general_intra_chroma_block_mode(
        work_unit,
        symbols,
        chroma_tools,
        GeneralIntraChromaModeContext::sdp_chroma_part(
            frontier.cfl_allowed_in_sdp(),
            is_cfl_ctx.get(),
        ),
        y_mode,
        frontier.b_size,
        block_ctx.block().width4(),
        block_ctx.block().height4(),
    )
    .map_err(|error| general_intra_block_mode_error(error, tile_offset))?;
    let chroma_plan =
        chroma_plan_for_parts(chroma, y_mode, angle_delta_y, cfl_ds_filter_index, sb_mib);
    let residual_plan = GeneralIntraResidualPlan::chroma(block_ctx, chroma_plan, lossless_luma_fsc)
        .map_err(|error| general_intra_residual_plan_error(error, tile_offset))?;
    let command = parse_general_intra_residual_plan(
        residual_plan,
        work_unit,
        symbols,
        coeff_ctx,
        block_ctx,
        deblock_blocks,
        chroma_deblock_blocks,
        tx_skip_records,
        chroma.coeff_uv_mode(),
        LumaTransformTypeContext::new(y_mode, angle_delta_y),
        None,
        transform_tool_residual_policy,
        qindex,
        lossless,
        intra_edge,
        segment_id,
        tile_offset,
    )?;
    record_chroma_smooth(chroma_smooth, block_ctx, chroma.supported_chroma_mode());
    Ok((GeneralIntraLeafMode::chroma(chroma.is_cfl()), command))
}

#[allow(clippy::too_many_arguments)]
fn parse_one_general_intra_rect_block(
    intra_edge: crate::prediction::intra_edge::IntraEdgeCtx,
    work_unit: &mut crate::bitstream::tile_payload::DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    modes: &GeneralIntraBlockModes,
    coeff_ctx: &mut crate::bitstream::tile_payload::TileCoeffContextState,
    deblock_blocks: &mut Vec<crate::filters::deblock::DeblockBlock>,
    chroma_deblock_blocks: &mut crate::filters::deblock::ChromaDeblockRecords,
    tx_skip_records: &mut Vec<crate::filters::wienerns_lr::WienerNsLrTxSkipTransformRecord>,
    qindex: u32,
    luma_use_tcq: bool,
    lossless: bool,
    luma_lossless_tx_size: Option<usize>,
    transform_tool_residual_policy: TransformToolResidualPolicy,
    luma_tx_partition_context: Option<LumaTransformPartitionContext>,
    block_ctx: BlockCtx,
    cfl_ds_filter_index: u8,
    sb_mib: usize,
    segment_id: u8,
    tile_offset: ByteOffset,
) -> Result<(GeneralIntraLeafMode, GeneralIntraReconCommand)> {
    let luma_plan = rect_luma_plan(modes, block_ctx, luma_use_tcq, sb_mib)?;
    let chroma_plan = rect_chroma_plan(modes, cfl_ds_filter_index, sb_mib);

    let residual_plan = GeneralIntraResidualPlan::rect(
        block_ctx,
        luma_plan,
        chroma_plan,
        modes.uses_active_fsc(),
        luma_lossless_tx_size,
        lossless,
    )
    .map_err(|error| general_intra_residual_plan_error(error, tile_offset))?;
    let command = parse_general_intra_residual_plan(
        residual_plan,
        work_unit,
        symbols,
        coeff_ctx,
        block_ctx,
        deblock_blocks,
        chroma_deblock_blocks,
        tx_skip_records,
        modes.coeff_uv_mode(),
        luma_transform_type_context(modes),
        luma_tx_partition_context,
        transform_tool_residual_policy,
        qindex,
        lossless,
        intra_edge,
        segment_id,
        tile_offset,
    )?;
    Ok((leaf_mode_for_block(modes), command))
}

fn leaf_mode(modes: &GeneralIntraBlockModes) -> GeneralIntraLeafMode {
    GeneralIntraLeafMode::luma(
        modes.intra_joint_mode,
        modes.y_mode,
        modes.angle_delta_y,
        modes.fsc_mode,
        modes.mrl,
    )
    .with_palette_y(modes.palette_y())
    .with_use_dip(modes.use_dip)
}

fn leaf_mode_for_block(modes: &GeneralIntraBlockModes) -> GeneralIntraLeafMode {
    let leaf = leaf_mode(modes);
    if modes.chroma().is_some() {
        leaf.with_uv_cfl(modes.is_cfl())
    } else {
        leaf
    }
}

fn luma_transform_type_context(modes: &GeneralIntraBlockModes) -> LumaTransformTypeContext {
    LumaTransformTypeContext::with_mrl_indices(
        modes.y_mode,
        modes.angle_delta_y,
        modes.mrl.index() as u8,
        modes.luma_dpcm_direction(),
    )
}

fn luma_tx_partition_context(
    tx_mode: Option<TxMode>,
    block_size: BlockSize,
    lossless: bool,
) -> Option<LumaTransformPartitionContext> {
    if tx_mode != Some(TxMode::Select) || lossless {
        return None;
    }
    Some(LumaTransformPartitionContext::new(block_size))
}

fn frame_tx_mode(core: &FrameHeaderCore) -> Option<TxMode> {
    core.intra_tail
        .as_ref()
        .map(|tail| tail.tx_mode)
        .or_else(|| core.inter_tail.as_ref().map(|tail| tail.tx_mode))
}

fn rect_luma_plan(
    modes: &GeneralIntraBlockModes,
    block_ctx: BlockCtx,
    use_tcq: bool,
    sb_mib: usize,
) -> core::result::Result<RectLumaPlan, crate::DecodeHeaderStateError> {
    if let Some(palette) = modes.palette_y() {
        return Ok(RectLumaPlan::Palette { palette, use_tcq });
    }
    if modes.uses_active_dip() {
        return Ok(RectLumaPlan::Dip {
            mode: modes.dip_mode,
            transpose: modes.dip_transpose != 0,
            use_tcq,
        });
    }
    if modes.uses_active_mrl() {
        return rect_luma_mrl_plan(modes, block_ctx, use_tcq, sb_mib);
    }
    rect_luma_plan_for_mode(
        modes.y_mode,
        rect_luma_directional_p_angle(modes, block_ctx)?,
        use_tcq,
    )
}

fn rect_luma_mrl_plan(
    modes: &GeneralIntraBlockModes,
    block_ctx: BlockCtx,
    use_tcq: bool,
    sb_mib: usize,
) -> core::result::Result<RectLumaPlan, crate::DecodeHeaderStateError> {
    rect_luma_mrl_plan_for_parts(
        modes.y_mode,
        modes.angle_delta_y,
        modes.mrl,
        block_ctx,
        use_tcq,
        sb_mib,
    )
}

fn rect_luma_mrl_plan_for_parts(
    y_mode: IntraYMode,
    angle_delta_y: i8,
    mrl: MrlSelection,
    block_ctx: BlockCtx,
    use_tcq: bool,
    sb_mib: usize,
) -> core::result::Result<RectLumaPlan, crate::DecodeHeaderStateError> {
    let IntraYModeClass::Directional {
        base_angle: nominal,
    } = y_mode.class()
    else {
        return Err(crate::DecodeHeaderStateError::InvalidGeneralIntraModeState);
    };
    if !mrl.is_active() {
        return Err(crate::DecodeHeaderStateError::InvalidGeneralIntraModeState);
    }
    let mrl_index = mrl.index();
    let mrl_delta = MRL_INDEX_TO_DELTA[mrl_index];
    let block = block_ctx.block();
    let width = block.width4().saturating_mul(MI_SIZE);
    let height = block.height4().saturating_mul(MI_SIZE);
    let nominal_angle = i32::from(nominal) + i32::from(angle_delta_y) * ANGLE_STEP + mrl_delta;
    let p_angle = wide_angle_mapped_p_angle(width, height, nominal_angle);
    let is_sb_boundary = sb_mib != 0 && block.row4().is_multiple_of(sb_mib);
    let above_mrl_index = if is_sb_boundary { 0 } else { mrl_index };
    let secondary_mrl = mrl.is_secondary() && !(width == MI_SIZE && height == MI_SIZE);
    let p_angle = u16::try_from(p_angle)
        .map_err(|_| crate::DecodeHeaderStateError::InvalidGeneralIntraModeState)?;
    match p_angle {
        90 => Ok(RectLumaPlan::CardinalMrl {
            direction: IntraCardinalDirection::Vertical,
            mrl_index,
            above_mrl_index,
            secondary_mrl,
            use_tcq,
        }),
        180 => Ok(RectLumaPlan::CardinalMrl {
            direction: IntraCardinalDirection::Horizontal,
            mrl_index,
            above_mrl_index,
            secondary_mrl,
            use_tcq,
        }),
        1..=89 => Ok(RectLumaPlan::OneSidedAboveMrl {
            p_angle,
            mrl_index,
            above_mrl_index,
            secondary_mrl,
            use_tcq,
        }),
        91..=179 => Ok(RectLumaPlan::MiddleMrl {
            p_angle,
            mrl_index,
            above_mrl_index,
            is_sb_boundary,
            secondary_mrl,
            use_tcq,
        }),
        181..=269 => Ok(RectLumaPlan::OneSidedLeftMrl {
            p_angle,
            mrl_index,
            above_mrl_index,
            is_sb_boundary,
            secondary_mrl,
            use_tcq,
        }),
        _ => Err(crate::DecodeHeaderStateError::InvalidGeneralIntraModeState),
    }
}

fn rect_luma_plan_for_mode(
    y_mode: IntraYMode,
    directional_p_angle: Option<u16>,
    use_tcq: bool,
) -> core::result::Result<RectLumaPlan, crate::DecodeHeaderStateError> {
    match y_mode.class() {
        IntraYModeClass::Dc => return Ok(RectLumaPlan::Dc { use_tcq }),
        IntraYModeClass::Paeth => return Ok(RectLumaPlan::Paeth { use_tcq }),
        IntraYModeClass::Smooth(mode) => return Ok(RectLumaPlan::Smooth { mode, use_tcq }),
        IntraYModeClass::Directional { .. } => {}
    }
    match directional_p_angle {
        Some(90) => {
            return Ok(RectLumaPlan::Cardinal {
                direction: IntraCardinalDirection::Vertical,
                use_tcq,
            });
        }
        Some(180) => {
            return Ok(RectLumaPlan::Cardinal {
                direction: IntraCardinalDirection::Horizontal,
                use_tcq,
            });
        }
        Some(p_angle @ 91..=179) => {
            return Ok(RectLumaPlan::Middle { p_angle, use_tcq });
        }
        _ => {}
    }
    if let Some(p_angle @ 1..=89) = directional_p_angle {
        return Ok(RectLumaPlan::OneSidedAbove { p_angle, use_tcq });
    }
    if let Some(p_angle @ 181..=269) = directional_p_angle {
        return Ok(RectLumaPlan::OneSidedLeft { p_angle, use_tcq });
    }
    Err(crate::DecodeHeaderStateError::InvalidGeneralIntraModeState)
}

fn rect_luma_directional_p_angle(
    modes: &GeneralIntraBlockModes,
    block_ctx: BlockCtx,
) -> core::result::Result<Option<u16>, crate::DecodeHeaderStateError> {
    directional_p_angle_for_luma(modes.y_mode, modes.angle_delta_y, block_ctx)
}

fn directional_p_angle_for_luma(
    y_mode: IntraYMode,
    angle_delta_y: i8,
    block_ctx: BlockCtx,
) -> core::result::Result<Option<u16>, crate::DecodeHeaderStateError> {
    let Some(base) = y_mode.mode_to_angle() else {
        return Ok(None);
    };
    let angle = i32::from(base)
        .checked_add(i32::from(angle_delta_y) * ANGLE_STEP)
        .ok_or(crate::DecodeHeaderStateError::InvalidGeneralIntraModeState)?;
    let block = block_ctx.block();
    let width = block
        .width4()
        .checked_mul(4)
        .ok_or(crate::DecodeHeaderStateError::InvalidGeneralIntraModeState)?;
    let height = block
        .height4()
        .checked_mul(4)
        .ok_or(crate::DecodeHeaderStateError::InvalidGeneralIntraModeState)?;
    u16::try_from(wide_angle_mapped_p_angle(width, height, angle))
        .map(Some)
        .map_err(|_| crate::DecodeHeaderStateError::InvalidGeneralIntraModeState)
}

fn rect_chroma_plan(
    modes: &GeneralIntraBlockModes,
    cfl_ds_filter_index: u8,
    sb_mib: usize,
) -> Option<RectChromaPlan> {
    modes.chroma().map(|chroma| {
        chroma_plan_for_parts(
            chroma,
            modes.y_mode,
            modes.angle_delta_y,
            cfl_ds_filter_index,
            sb_mib,
        )
    })
}

fn rect_chroma_plan_for_mode(
    mode: SupportedChromaMode,
    angle_delta_uv: i8,
    dpcm: Option<DpcmDirection>,
) -> RectChromaPlan {
    if mode.directional_base_angle().is_none() {
        return RectChromaPlan::Mode(mode, dpcm);
    }
    RectChromaPlan::Directional {
        mode,
        angle_delta_uv,
        dpcm,
    }
}

pub(crate) const fn inherited_chroma_angle_delta(
    uv_mode: usize,
    y_mode: IntraYMode,
    angle_delta_y: i8,
) -> i8 {
    if uv_mode == y_mode.value() {
        angle_delta_y
    } else {
        0
    }
}

fn chroma_plan_for_parts(
    chroma: GeneralIntraChromaBlockMode,
    y_mode: IntraYMode,
    angle_delta_y: i8,
    cfl_ds_filter_index: u8,
    sb_mib: usize,
) -> RectChromaPlan {
    match chroma {
        GeneralIntraChromaBlockMode::Prediction {
            mode,
            coeff_uv_mode,
            dpcm,
        } => rect_chroma_plan_for_mode(
            mode,
            inherited_chroma_angle_delta(usize::from(coeff_uv_mode), y_mode, angle_delta_y),
            dpcm,
        ),
        GeneralIntraChromaBlockMode::Cfl(params) => {
            cfl_chroma_plan(params, cfl_ds_filter_index, sb_mib)
        }
    }
}

fn cfl_chroma_plan(params: CflParams, cfl_ds_filter_index: u8, sb_mib: usize) -> RectChromaPlan {
    RectChromaPlan::Cfl {
        params,
        cfl_ds_filter_index,
        sb_mib,
    }
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

#[allow(clippy::needless_pass_by_value, clippy::too_many_arguments)]
fn parse_general_intra_residual_plan(
    residual_plan: GeneralIntraResidualPlan,
    work_unit: &mut crate::bitstream::tile_payload::DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    coeff_ctx: &mut crate::bitstream::tile_payload::TileCoeffContextState,
    block_ctx: BlockCtx,
    deblock_blocks: &mut Vec<crate::filters::deblock::DeblockBlock>,
    chroma_deblock_blocks: &mut crate::filters::deblock::ChromaDeblockRecords,
    tx_skip_records: &mut Vec<crate::filters::wienerns_lr::WienerNsLrTxSkipTransformRecord>,
    uv_mode: usize,
    luma_transform_type_context: LumaTransformTypeContext,
    luma_tx_partition_context: Option<LumaTransformPartitionContext>,
    transform_tool_residual_policy: TransformToolResidualPolicy,
    qindex: u32,
    lossless: bool,
    intra_edge: crate::prediction::intra_edge::IntraEdgeCtx,
    segment_id: u8,
    tile_offset: ByteOffset,
) -> Result<GeneralIntraReconCommand> {
    let block = block_ctx.block();
    let mut deblock = crate::residual::pipeline::DeblockRecorder {
        blocks: deblock_blocks,
        chroma_blocks: chroma_deblock_blocks,
        tx_skip_records,
        block_r: block.row4(),
        block_c: block.col4(),
        chroma_tx: residual_plan.chroma_tx(),
        chroma_subsampling: block_ctx.chroma().subsampling(PlaneId::U),
        qindex,
        lossless,
    };
    let residual = residual_plan
        .parse(
            work_unit,
            symbols,
            coeff_ctx,
            uv_mode,
            luma_transform_type_context,
            luma_tx_partition_context,
            transform_tool_residual_policy,
            &mut deblock,
        )
        .map_err(|error| general_intra_residual_error(error, tile_offset))?;
    Ok(GeneralIntraReconCommand {
        residual,
        qindex,
        intra_edge,
        luma_transform_type_context,
        segment_id,
        tile_offset,
    })
}

fn general_intra_residual_plan_error(error: ResidualPlanError, _offset: ByteOffset) -> DecodeError {
    match error {
        ResidualPlanError::InvalidGeometry => {
            crate::DecodeHeaderStateError::InvalidBlockGeometry.into()
        }
        ResidualPlanError::Allocation { plane } => {
            splot_recon::ReconError::WorkspaceAllocationFailed {
                plane,
                context: "general-intra residual plan",
            }
            .into()
        }
    }
}

#[allow(clippy::needless_pass_by_value)]
fn general_intra_residual_error(
    error: GeneralIntraResidualError,
    offset: ByteOffset,
) -> DecodeError {
    match error {
        error @ (GeneralIntraResidualError::AllZeroRead { .. }
        | GeneralIntraResidualError::NonZeroStart { .. }
        | GeneralIntraResidualError::StagedNonZeroPass { .. }
        | GeneralIntraResidualError::StagedFscPass { .. }
        | GeneralIntraResidualError::CctxTypeRead { .. }) => {
            general_intra_coeff_error(&error, offset, GENERAL_INTRA_RESIDUAL_SPEC_SECTION)
        }
        error @ (GeneralIntraResidualError::TransformPartitionRead { .. }
        | GeneralIntraResidualError::TransformPartitionGeometry { .. }) => {
            general_intra_coeff_error(&error, offset, "5.20.6.3")
        }
        error @ GeneralIntraResidualError::TransformTypeRead { .. } => {
            general_intra_coeff_error(&error, offset, "5.20.8.2")
        }
        GeneralIntraResidualError::InvalidTransformPartitionDimensions { .. } => {
            malformed_tile_payload(offset, "6.19.6.3", error)
        }
        GeneralIntraResidualError::CoeffContextState { source } => {
            general_intra_coeff_context_error(&source)
        }
        GeneralIntraResidualError::PaletteSymbolRead {
            source:
                BlockSymbolTraceReadError::Cdf(_)
                | BlockSymbolTraceReadError::Symbol(
                    splot_core::Error::InvalidSymbolCdf { .. }
                    | splot_core::Error::InvalidSymbolDecoderState { .. },
                ),
        }
        | GeneralIntraResidualError::PaletteLiteral {
            source: splot_core::Error::InvalidSymbolDecoderState { .. },
            ..
        } => crate::DecodeHeaderStateError::InvalidGeneralIntraPaletteEntropyState.into(),
        GeneralIntraResidualError::PaletteSymbolRead {
            source: BlockSymbolTraceReadError::Symbol(_),
        }
        | GeneralIntraResidualError::PaletteLiteral {
            source: splot_core::Error::UnexpectedEof { .. },
            ..
        } => malformed_tile_payload(offset, "5.20.8.4", error),
        GeneralIntraResidualError::PaletteLiteral { .. } => {
            crate::DecodeHeaderStateError::InvalidGeneralIntraPaletteEntropyState.into()
        }
        GeneralIntraResidualError::PaletteInvalidIdentityRow => {
            malformed_tile_payload(offset, "6.19.8.3", error)
        }
        GeneralIntraResidualError::PaletteColorIndex { .. } => {
            crate::DecodeHeaderStateError::InvalidGeneralIntraPaletteColorState.into()
        }
        GeneralIntraResidualError::UnsupportedTransformToolResidual { .. } => {
            general_intra_at!(
                "general_intra_transform_tool_residual",
                offset,
                missing_capability_message!("intra.residual.transform_tools", residual = "nonzero"),
                GENERAL_INTRA_RESIDUAL_SPEC_SECTION,
            )
        }
        GeneralIntraResidualError::UnexpectedBranch => {
            general_intra_internal!("general_intra_luma_coeff_unexpected_branch", offset,)
        }
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
    }
}

fn general_intra_coeff_context_error(error: &TileCoeffStateError) -> DecodeError {
    match error {
        TileCoeffStateError::Allocation(_) => splot_recon::ReconError::WorkspaceAllocationFailed {
            plane: PlaneId::Y,
            context: "general-intra coefficient context state",
        }
        .into(),
        TileCoeffStateError::EmptyTileDimensions { .. }
        | TileCoeffStateError::InvalidAdjustedTransformExtent { .. }
        | TileCoeffStateError::ArithmeticOverflow { .. }
        | TileCoeffStateError::InvalidPlane { .. }
        | TileCoeffStateError::InvalidDcCategory { .. }
        | TileCoeffStateError::EmptyContextRange { .. }
        | TileCoeffStateError::CoordinateOverflow { .. }
        | TileCoeffStateError::ContextRangeOutOfBounds { .. }
        | TileCoeffStateError::TransformCoordinateOutOfBounds { .. }
        | TileCoeffStateError::QuantPositionOutOfBounds { .. }
        | TileCoeffStateError::InvalidSubsampling { .. } => {
            crate::DecodeHeaderStateError::InvalidGeneralIntraCoefficientContextState.into()
        }
    }
}

fn general_intra_coeff_error(
    error: &(dyn std::error::Error + 'static),
    offset: ByteOffset,
    syntax_spec_section: &'static str,
) -> DecodeError {
    match classify_coeff_parse_error(error, syntax_spec_section) {
        CoeffParseErrorClass::Allocation => splot_recon::ReconError::WorkspaceAllocationFailed {
            plane: PlaneId::Y,
            context: "general-intra coefficient parse state",
        }
        .into(),
        CoeffParseErrorClass::Malformed { spec_section } => {
            malformed_tile_payload(offset, spec_section, error)
        }
        CoeffParseErrorClass::EntropyState => {
            crate::DecodeHeaderStateError::InvalidGeneralIntraCoefficientEntropyState.into()
        }
        CoeffParseErrorClass::CoefficientState => {
            crate::DecodeHeaderStateError::InvalidGeneralIntraCoefficientState.into()
        }
    }
}

#[allow(clippy::needless_pass_by_value)]
fn general_intra_block_mode_error(
    error: GeneralIntraBlockModeError,
    offset: ByteOffset,
) -> DecodeError {
    match error {
        GeneralIntraBlockModeError::SymbolRead {
            source:
                BlockSymbolTraceReadError::Cdf(_)
                | BlockSymbolTraceReadError::Symbol(
                    splot_core::Error::InvalidSymbolCdf { .. }
                    | splot_core::Error::InvalidSymbolDecoderState { .. },
                ),
            ..
        }
        | GeneralIntraBlockModeError::Literal {
            source: splot_core::Error::InvalidSymbolDecoderState { .. },
            ..
        } => crate::DecodeHeaderStateError::InvalidGeneralIntraModeState.into(),
        GeneralIntraBlockModeError::SymbolRead {
            source: BlockSymbolTraceReadError::Symbol(_),
            ..
        }
        | GeneralIntraBlockModeError::Literal {
            source: splot_core::Error::UnexpectedEof { .. },
            ..
        }
        | GeneralIntraBlockModeError::InvalidUvMode { .. } => {
            malformed_tile_payload(offset, GENERAL_INTRA_MODE_SPEC_SECTION, error)
        }
        GeneralIntraBlockModeError::Literal { .. }
        | GeneralIntraBlockModeError::InvalidModeState(_) => {
            crate::DecodeHeaderStateError::InvalidGeneralIntraModeState.into()
        }
        GeneralIntraBlockModeError::InvalidCflMhDirection { .. } => {
            crate::DecodeHeaderStateError::InvalidGeneralIntraMhccpDirection.into()
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
#[path = "general_intra_unit_tests.rs"]
mod tests;

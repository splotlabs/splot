// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_recon::{
    BitDepth, CurrentFrameWorkspace, DpcmDirection, IntraCardinalDirection, IntraPredictionScratch,
    PlaneId, ReconSample,
};

use super::*;
use crate::bitstream::tile_payload::{
    BlockSize, BlockSymbolTraceReadError, CoeffParseErrorClass, GeneralIntraBlockModes,
    GeneralIntraChromaBlockMode, GeneralIntraChromaModeContext, GeneralIntraChromaToolConfig,
    GeneralIntraLeafMode, IntraYMode, IntraYModeClass, LumaTransformPartitionContext,
    LumaTransformTypeContext, MrlSelection, SupportedChromaMode, TileCoeffStateError,
    TransformToolResidualPolicy, classify_coeff_parse_error, read_lossless_luma_tx_size,
};
use crate::residual::pipeline::{
    ParsedGeneralIntraResidual, RectChromaPlan, RectLumaPlan, ResidualPlanError,
};
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

impl<T: Send + 'static> RecycledIntraSamples<T> {
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
    pub(crate) cfl_luma_ac: Vec<i16>,
    pub(crate) cfl_prediction: Vec<T>,
    pub(crate) mhccp_refs: [Vec<u16>; 3],
    pub(crate) paeth_edges: [Vec<T>; 2],
    cctx: Option<Box<CctxReconScratch<T>>>,
}

pub(crate) struct CctxReconScratch<T: ReconSample> {
    pub(crate) u_prediction: Vec<T>,
    pub(crate) v_prediction: Vec<T>,
    pub(crate) u_output: Vec<T>,
    pub(crate) v_output: Vec<T>,
}

impl<T: ReconSample> Default for CctxReconScratch<T> {
    fn default() -> Self {
        let vector = || Vec::with_capacity(MAX_INTRA_BLOCK_SAMPLES);
        Self {
            u_prediction: vector(),
            v_prediction: vector(),
            u_output: vector(),
            v_output: vector(),
        }
    }
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
            cctx: None,
        }
    }
}

impl<T: ReconSample> GeneralIntraReconScratch<T> {
    pub(crate) fn cctx_mut(&mut self) -> &mut CctxReconScratch<T> {
        self.cctx
            .get_or_insert_with(|| Box::new(CctxReconScratch::default()))
    }
}

#[cfg(test)]
mod scratch_tests {
    use super::{GeneralIntraReconScratch, MAX_INTRA_BLOCK_SAMPLES};
    use splot_recon::ReconSample;

    fn assert_cctx_is_lazy_and_reused<T: ReconSample>() {
        let mut scratch = GeneralIntraReconScratch::<T>::default();
        assert!(scratch.cctx.is_none());

        let first = scratch.cctx_mut();
        let capacities = [
            first.u_prediction.capacity(),
            first.v_prediction.capacity(),
            first.u_output.capacity(),
            first.v_output.capacity(),
        ];
        assert!(
            capacities
                .iter()
                .all(|&capacity| capacity >= MAX_INTRA_BLOCK_SAMPLES)
        );
        first.u_prediction.resize(16, T::default());
        first.v_prediction.resize(16, T::default());
        first.u_output.resize(16, T::default());
        first.v_output.resize(16, T::default());
        let pointers = [
            first.u_prediction.as_ptr(),
            first.v_prediction.as_ptr(),
            first.u_output.as_ptr(),
            first.v_output.as_ptr(),
        ];

        let reused = scratch.cctx_mut();
        assert_eq!(
            [
                reused.u_prediction.capacity(),
                reused.v_prediction.capacity(),
                reused.u_output.capacity(),
                reused.v_output.capacity(),
            ],
            capacities
        );
        assert_eq!(
            [
                reused.u_prediction.as_ptr(),
                reused.v_prediction.as_ptr(),
                reused.u_output.as_ptr(),
                reused.v_output.as_ptr(),
            ],
            pointers
        );
    }

    #[test]
    fn cctx_scratch_is_lazy_and_reused_for_each_sample_type() {
        assert_cctx_is_lazy_and_reused::<u8>();
        assert_cctx_is_lazy_and_reused::<u16>();
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

pub(crate) fn swap_general_intra_scratch<T: ReconSample>(
    scratch: &mut GeneralIntraReconScratch<T>,
    workspace: &mut CurrentFrameWorkspace<T>,
) {
    workspace.swap_intra_prediction_scratch(&mut scratch.intra_prediction);
    HOT_INTRA_VECTORS.with(|vectors| {
        std::mem::swap(&mut *vectors.borrow_mut(), &mut scratch.hot_vectors.slots);
    });
}

impl GeneralIntraReconCommand {
    pub(crate) fn reconstruct<T: ReconSample>(
        &self,
        arena: &mut crate::residual::pipeline::ResidualPlaneArena,
        coeffs_arena: &[i32],
        scratch: &mut GeneralIntraReconScratch<T>,
        workspace: &mut CurrentFrameWorkspace<T>,
        block_decoded: &mut crate::bitstream::tile_payload::TileBlockDecodedState,
    ) -> Result<()> {
        let _qm_segment_scope = crate::bitstream::tile_payload::FrameQmSegmentScope::install(
            usize::from(self.segment_id),
        );
        self.reconstruct_with_installed_quantizer(
            arena,
            coeffs_arena,
            scratch,
            workspace,
            block_decoded,
        )
    }

    fn reconstruct_with_installed_quantizer<T: ReconSample>(
        &self,
        arena: &mut crate::residual::pipeline::ResidualPlaneArena,
        coeffs_arena: &[i32],
        scratch: &mut GeneralIntraReconScratch<T>,
        workspace: &mut CurrentFrameWorkspace<T>,
        block_decoded: &mut crate::bitstream::tile_payload::TileBlockDecodedState,
    ) -> Result<()> {
        self.residual
            .reconstruct(
                arena,
                coeffs_arena,
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

pub(crate) fn general_intra_chroma_tools(
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

pub(crate) fn sequence_cfl_ds_filter_index(sequence: &SequenceHeader) -> u8 {
    sequence
        .intra
        .as_ref()
        .map_or(0, |intra| intra.cfl_ds_filter_index)
}

/// The frame facts every general-intra leaf of a frame reads, derived once.
#[derive(Clone, Copy)]
pub(crate) struct GeneralIntraFrameTools {
    chroma_tools: GeneralIntraChromaToolConfig,
    chroma_format: splot_core::headers::sequence::ChromaFormatIdc,
    frame_is_intra: bool,
    enable_ibp: bool,
    enable_intra_edge_filter: bool,
    cfl_ds_filter_index: u8,
    sb_mib: usize,
    tx_mode: Option<TxMode>,
}

impl GeneralIntraFrameTools {
    pub(crate) fn new(sequence: &SequenceHeader, core: &FrameHeaderCore) -> Self {
        let frame_is_intra = core.frame_is_intra == Some(true);
        let intra = sequence.intra.as_ref();
        Self {
            chroma_tools: general_intra_chroma_tools(sequence, core),
            chroma_format: sequence.general.chroma_format_idc,
            frame_is_intra,
            enable_ibp: intra.is_some_and(|intra| intra.enable_ibp),
            enable_intra_edge_filter: intra.is_some_and(|intra| intra.enable_intra_edge_filter),
            cfl_ds_filter_index: sequence_cfl_ds_filter_index(sequence),
            sb_mib: sequence.partition.as_ref().map_or(16, |partition| {
                crate::tile::frame_superblock_h4(partition.seq_sb_size(), frame_is_intra)
            }),
            tx_mode: frame_tx_mode(core),
        }
    }
}

/// The tile state a general-intra leaf parses into and its per-leaf facts,
/// lent down the mode and residual syntax by one pointer.
pub(crate) struct GeneralIntraLeafParse<'a, 'p> {
    pub(crate) work_unit: &'a mut crate::bitstream::tile_payload::DecodeTileWorkUnit,
    pub(crate) symbols: &'a mut SymbolDecoder<'p>,
    pub(crate) coeff_ctx: &'a mut crate::bitstream::tile_payload::TileCoeffContextState,
    pub(crate) deblock_blocks: &'a mut Vec<crate::filters::deblock::DeblockBlock>,
    pub(crate) chroma_deblock_blocks: &'a mut crate::filters::deblock::ChromaDeblockRecords,
    pub(crate) tx_skip_records:
        &'a mut Vec<crate::filters::wienerns_lr::WienerNsLrTxSkipTransformRecord>,
    pub(crate) arena: &'a mut crate::residual::pipeline::ResidualPlaneArena,
    pub(crate) coeffs_arena: &'a mut Vec<i32>,
    pub(crate) qindex: u32,
    pub(crate) segment_id: u8,
    pub(crate) luma_use_tcq: bool,
    pub(crate) policy: TransformToolResidualPolicy,
    pub(crate) mi_cols: usize,
    pub(crate) mi_rows: usize,
    pub(crate) bit_depth: BitDepth,
    pub(crate) tile_offset: ByteOffset,
}

/// The residual-plan facts of one leaf beside its [`GeneralIntraLeafParse`].
struct LeafResidualFacts {
    block_ctx: BlockCtx,
    intra_edge: crate::prediction::intra_edge::IntraEdgeCtx,
    lossless: bool,
    uv_mode: usize,
    luma_transform_type_context: LumaTransformTypeContext,
    luma_tx_partition_context: Option<LumaTransformPartitionContext>,
}

pub(crate) fn decode_one_general_intra_block(
    leaf: &mut GeneralIntraLeafParse<'_, '_>,
    frontier: &crate::bitstream::tile_payload::DecodeBlockFrontier,
    tools: &GeneralIntraFrameTools,
    neighbours: crate::bitstream::tile_payload::LeafNeighbours<'_>,
) -> Result<(GeneralIntraLeafMode, GeneralIntraReconCommand)> {
    let edges = neighbours.edges;
    let _qm_segment_scope =
        crate::bitstream::tile_payload::FrameQmSegmentScope::install(usize::from(leaf.segment_id));
    let mi_row_range = leaf.work_unit.mi_row_range();
    let mi_col_range = leaf.work_unit.mi_col_range();
    let (block_ctx, chroma_mode_geometry) = general_intra_block_ctx(
        frontier,
        tools.chroma_format,
        (
            mi_row_range.start as usize,
            mi_col_range.start as usize..mi_col_range.end as usize,
        ),
        (leaf.mi_rows, leaf.mi_cols),
        leaf.bit_depth,
    );
    let n4w = block_ctx.block().width4();
    let n4h = block_ctx.block().height4();
    let intra_neighbours = edges.intra_neighbours(frontier.r, frontier.c, n4w, n4h);
    let (above_smooth, left_smooth) = intra_neighbours.smoothness();
    let chroma_ref = frontier.chroma_ref_geometry();
    let uv_edges = edges.uv_edges(chroma_ref.row(), chroma_ref.col());
    let (chroma_above_smooth, chroma_left_smooth) = uv_edges.smoothness();
    let intra_edge = crate::prediction::intra_edge::IntraEdgeCtx {
        enable_ibp: tools.enable_ibp,
        enable_intra_edge_filter: tools.enable_intra_edge_filter,
        above_smooth,
        left_smooth,
        chroma_above_smooth,
        chroma_left_smooth,
    };
    let lossless = leaf
        .work_unit
        .coeff_frame_facts()
        .lossless_for_segment(usize::from(leaf.segment_id))
        .unwrap_or(false);
    let chroma_tools = tools.chroma_tools.with_lossless(lossless);
    let tile_offset = leaf.tile_offset;

    if let crate::bitstream::tile_payload::DecodeBlockPart::ChromaPart {
        y_mode,
        angle_delta_y,
    } = frontier.part()
    {
        let chroma = crate::bitstream::tile_payload::decode_general_intra_chroma_block_mode(
            leaf.work_unit,
            leaf.symbols,
            chroma_tools,
            GeneralIntraChromaModeContext::sdp_chroma_part(
                frontier.cfl_allowed_in_sdp(),
                uv_edges.cfl_ctx(),
            ),
            y_mode,
            frontier.b_size,
            n4w,
            n4h,
        )
        .map_err(|error| general_intra_block_mode_error(error, tile_offset))?;
        let chroma_plan = chroma_plan_for_parts(
            chroma,
            y_mode,
            angle_delta_y,
            tools.cfl_ds_filter_index,
            tools.sb_mib,
        );
        let lossless_luma_fsc =
            lossless && neighbours.collocated.uses_fsc_at(frontier.r, frontier.c);
        let residual_plan = leaf
            .arena
            .plan_chroma(block_ctx, chroma_plan, lossless_luma_fsc)
            .map_err(general_intra_residual_plan_error)?;
        let command = parse_general_intra_residual_plan(
            leaf,
            residual_plan,
            &LeafResidualFacts {
                block_ctx,
                intra_edge,
                lossless,
                uv_mode: chroma.coeff_uv_mode(),
                luma_transform_type_context: LumaTransformTypeContext::new(y_mode, angle_delta_y),
                luma_tx_partition_context: None,
            },
        )?;
        return Ok((GeneralIntraLeafMode::chroma(chroma.uv_bits()), command));
    }

    let luma_only = frontier.is_luma_part() || !frontier.has_chroma;
    let use_neighbor_fsc_context = tools.frame_is_intra || !frontier.is_mixed_region();
    let bit_depth_bits = u32::from(leaf.bit_depth.bits());
    let modes = if luma_only {
        let luma =
            crate::bitstream::tile_payload::decode_general_intra_luma_block_mode_with_fsc_context(
                leaf.work_unit,
                leaf.symbols,
                chroma_tools,
                intra_neighbours,
                use_neighbor_fsc_context,
                frontier.b_size,
                n4w,
                n4h,
            )
            .map_err(|error| general_intra_block_mode_error(error, tile_offset))?;
        let palette_y = crate::bitstream::tile_payload::read_general_intra_palette_y_mode(
            leaf.work_unit,
            leaf.symbols,
            chroma_tools,
            edges,
            luma.y_mode,
            frontier.b_size.index(),
            frontier.r,
            frontier.c,
            n4w,
            n4h,
            bit_depth_bits,
        )
        .map_err(|error| general_intra_block_mode_error(error, tile_offset))?;
        let (use_dip_value, dip_transpose, dip_mode) =
            crate::bitstream::tile_payload::read_general_intra_dip_mode_info(
                leaf.work_unit,
                leaf.symbols,
                chroma_tools,
                intra_neighbours,
                luma.y_mode,
                palette_y,
                n4w,
                n4h,
            )
            .map_err(|error| general_intra_block_mode_error(error, tile_offset))?;
        GeneralIntraBlockModes::luma_only(luma.with_dip(use_dip_value, dip_transpose, dip_mode))
            .with_palette_y(palette_y)
    } else {
        let (chroma_n4w, chroma_n4h) = chroma_mode_geometry.unwrap_or((n4w, n4h));
        crate::bitstream::tile_payload::decode_general_intra_block_modes_with_fsc_context(
            leaf.work_unit,
            leaf.symbols,
            chroma_tools,
            edges,
            intra_neighbours,
            use_neighbor_fsc_context,
            uv_edges.cfl_ctx(),
            frontier.b_size,
            frontier.r,
            frontier.c,
            n4w,
            n4h,
            chroma_n4w,
            chroma_n4h,
            bit_depth_bits,
        )
        .map_err(|error| general_intra_block_mode_error(error, tile_offset))?
    };
    let luma_lossless_tx_size = if lossless && modes.uses_active_fsc() {
        Some(
            read_lossless_luma_tx_size(leaf.work_unit, leaf.symbols, frontier.b_size, true, true)
                .map_err(|error| general_intra_block_mode_error(error, tile_offset))?,
        )
    } else {
        None
    };

    let luma_plan = rect_luma_plan(&modes, block_ctx, leaf.luma_use_tcq, tools.sb_mib)?;
    let chroma_plan = rect_chroma_plan(&modes, tools.cfl_ds_filter_index, tools.sb_mib);
    let residual_plan = leaf
        .arena
        .plan_rect(
            block_ctx,
            luma_plan,
            chroma_plan,
            modes.uses_active_fsc(),
            luma_lossless_tx_size,
            lossless,
        )
        .map_err(general_intra_residual_plan_error)?;
    let command = parse_general_intra_residual_plan(
        leaf,
        residual_plan,
        &LeafResidualFacts {
            block_ctx,
            intra_edge,
            lossless,
            uv_mode: modes.coeff_uv_mode(),
            luma_transform_type_context: luma_transform_type_context(&modes),
            luma_tx_partition_context: luma_tx_partition_context(
                tools.tx_mode,
                frontier.b_size,
                lossless,
            ),
        },
    )?;
    let leaf = GeneralIntraLeafMode::luma(
        modes.intra_joint_mode,
        modes.y_mode,
        modes.angle_delta_y,
        modes.fsc_mode,
        modes.mrl,
    )
    .with_palette_y(modes.palette_y())
    .with_use_dip(modes.use_dip)
    .with_uv(modes.uv_bits());
    Ok((leaf, command))
}

/// The block context of `frontier` and, with chroma, its chroma 4x4 extent.
#[inline]
pub(crate) fn general_intra_block_ctx(
    frontier: &crate::bitstream::tile_payload::DecodeBlockFrontier,
    chroma_format: splot_core::headers::sequence::ChromaFormatIdc,
    (tile_row_start, tile_cols): (usize, core::ops::Range<usize>),
    (mi_rows, mi_cols): (usize, usize),
    bit_depth: BitDepth,
) -> (BlockCtx, Option<(usize, usize)>) {
    let block_tx_shape = TxShape::from_av2_block_size(frontier.b_size);
    let mut block_ctx = BlockCtx::new(
        BlockRect::new(
            frontier.r,
            frontier.c,
            block_tx_shape.width4(),
            block_tx_shape.height4(),
        ),
        block_tx_shape,
        mi_cols,
        mi_rows,
        bit_depth,
        ChromaSampling::from_chroma_format_idc(chroma_format),
    )
    .with_tile_bounds(tile_row_start, tile_cols.start, tile_cols.end);
    if !frontier.has_chroma {
        return (block_ctx, None);
    }
    let chroma_ref = frontier.chroma_ref_geometry();
    let chroma_tx_shape = TxShape::from_av2_block_size(chroma_ref.size());
    let chroma_n4w = chroma_tx_shape.width4();
    let chroma_n4h = chroma_tx_shape.height4();
    block_ctx = block_ctx.with_chroma_ref(
        BlockRect::new(chroma_ref.row(), chroma_ref.col(), chroma_n4w, chroma_n4h),
        chroma_tx_shape,
    );
    (block_ctx, Some((chroma_n4w, chroma_n4h)))
}

pub(crate) fn luma_transform_type_context(
    modes: &GeneralIntraBlockModes,
) -> LumaTransformTypeContext {
    LumaTransformTypeContext::with_mrl_indices(
        modes.y_mode,
        modes.angle_delta_y,
        modes.mrl.index() as u8,
        modes.luma_dpcm_direction(),
    )
}

pub(crate) fn luma_tx_partition_context(
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

#[inline]
pub(crate) fn rect_luma_plan(
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
        return rect_luma_mrl_plan_for_parts(
            modes.y_mode,
            modes.angle_delta_y,
            modes.mrl,
            block_ctx,
            use_tcq,
            sb_mib,
        );
    }
    rect_luma_plan_for_mode(
        modes.y_mode,
        directional_p_angle_for_luma(modes.y_mode, modes.angle_delta_y, block_ctx)?,
        use_tcq,
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

pub(crate) fn rect_chroma_plan(
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
        GeneralIntraChromaBlockMode::Cfl(params) => RectChromaPlan::Cfl {
            params,
            cfl_ds_filter_index,
            sb_mib,
        },
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

fn parse_general_intra_residual_plan(
    leaf: &mut GeneralIntraLeafParse<'_, '_>,
    residual_plan: crate::residual::pipeline::PlannedGeneralIntraResidual,
    facts: &LeafResidualFacts,
) -> Result<GeneralIntraReconCommand> {
    let block = facts.block_ctx.block();
    let mut deblock = crate::residual::pipeline::DeblockRecorder {
        blocks: leaf.deblock_blocks,
        chroma_blocks: leaf.chroma_deblock_blocks,
        tx_skip_records: leaf.tx_skip_records,
        block_r: block.row4(),
        block_c: block.col4(),
        chroma_tx: leaf.arena.chroma_tx(&residual_plan),
        chroma_subsampling: facts.block_ctx.chroma().subsampling(PlaneId::U),
        qindex: leaf.qindex,
        lossless: facts.lossless,
    };
    let residual = leaf
        .arena
        .parse(
            residual_plan,
            leaf.work_unit,
            leaf.symbols,
            leaf.coeff_ctx,
            facts.uv_mode,
            facts.luma_transform_type_context,
            facts.luma_tx_partition_context,
            leaf.policy,
            &mut deblock,
            leaf.coeffs_arena,
        )
        .map_err(|error| general_intra_residual_error(error, leaf.tile_offset))?;
    Ok(GeneralIntraReconCommand {
        residual,
        qindex: leaf.qindex,
        intra_edge: facts.intra_edge,
        luma_transform_type_context: facts.luma_transform_type_context,
        segment_id: leaf.segment_id,
        tile_offset: leaf.tile_offset,
    })
}

fn general_intra_residual_plan_error(error: ResidualPlanError) -> DecodeError {
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
        GeneralIntraResidualError::CoeffSpanOutOfRange => {
            DecodeError::from(crate::DecodeHeaderStateError::InvalidInterResidualReconstruction)
        }
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
        GeneralIntraResidualError::InvalidReconstructionState { .. }
        | GeneralIntraResidualError::QuantLength { .. }
        | GeneralIntraResidualError::PredictionLength { .. } => {
            crate::DecodeHeaderStateError::InvalidGeneralIntraReconstructionState.into()
        }
        GeneralIntraResidualError::Reconstruct { source } => {
            general_intra_reconstruction_error(source)
        }
        GeneralIntraResidualError::InvalidDirectionalEdgeState => {
            crate::DecodeHeaderStateError::InvalidGeneralIntraDirectionalEdgeState.into()
        }
    }
}

fn general_intra_reconstruction_error(error: splot_recon::ReconError) -> DecodeError {
    match error {
        error @ (splot_recon::ReconError::WorkspaceAllocationFailed { .. }
        | splot_recon::ReconError::IntraPredictionAllocationFailed { .. }) => error.into(),
        _ => crate::DecodeHeaderStateError::InvalidGeneralIntraReconstructionState.into(),
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

#[cfg(test)]
#[path = "general_intra_unit_tests.rs"]
mod tests;

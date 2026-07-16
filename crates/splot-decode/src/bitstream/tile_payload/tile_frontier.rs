// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use core::mem::size_of;

use splot_core::headers::frame::{FrameHeaderCore, FrameRestorationType, LrParams};
use splot_core::headers::sequence::{ChromaFormatIdc, SequenceHeader, SuperblockSize};
use splot_core::symbol::SymbolDecoder;

use super::DecodeTileWorkUnit;
use super::intra_joint_modes::{
    IsCflContext, TileFscModeState, TileFscModeStateError, TileIntraJointModeState,
    TileIntraJointModeStateError, TileLumaPaletteState, TileLumaPaletteStateError, TileUseDipState,
    TileUseDipStateError, TileUsesMrlsState, TileUsesMrlsStateError, TileUvCflState,
    TileUvCflStateError,
};
use super::mi_size_state::{TileMiSizeState, TileMiSizeStateError};
use super::partition::PartitionType;
use super::partition_allowed::PartitionFeatureFlags;
use super::partition_size::BlockSize;
pub(crate) use super::partition_traversal::GeneralIntraPartitionTreeOutput as GeneralIntraMultiblockOutput;
use super::partition_traversal::{
    DecodeBlockFrontier, DecodedLeafPublication, GeneralIntraLeafMode,
    GeneralIntraPartitionTreeCursor, GeneralIntraTreeWalkError, TileLoopRestorationRootFrontier,
    TilePartitionBruState, TilePartitionFrameFacts, TilePartitionLoopRestorationFrameState,
    TilePartitionLoopRestorationPlaneTool, TilePartitionLoopRestorationState,
    TilePartitionTraversalError, TilePartitionTraversalInput, TilePartitionTraversalPlan,
    consume_tile_loop_restoration_root_frontier, plan_tile_partition_traversal_cursor,
};
use crate::{DecodeLimitError, DecodeLimitName, DecodeLimits};

const BLOCK_64X64_INDEX: usize = 12;
const BLOCK_128X128_INDEX: usize = 15;
const BLOCK_256X256_INDEX: usize = 18;

#[derive(Debug, thiserror::Error)]
pub(crate) enum TilePartitionFrontierError {
    #[error("partition frontier missing fact: {fact}")]
    MissingFact { fact: &'static str },
    #[error("partition frontier limit failed: {0}")]
    Limit(#[from] DecodeLimitError),
    #[error("partition traversal failed: {0}")]
    Traversal(#[from] TilePartitionTraversalError),
    #[error("MI-size state failed: {0}")]
    MiSizeState(#[from] TileMiSizeStateError),
    #[error("intra joint-mode state failed: {0}")]
    IntraJointModeState(#[from] TileIntraJointModeStateError),
    #[error("intra UsesMrls state failed: {0}")]
    UsesMrlsState(#[from] TileUsesMrlsStateError),
    #[error("intra UseDip state failed: {0}")]
    UseDipState(#[from] TileUseDipStateError),
    #[error("intra FscModes state failed: {0}")]
    FscModeState(#[from] TileFscModeStateError),
    #[error("luma palette state failed: {0}")]
    LumaPaletteState(#[from] TileLumaPaletteStateError),
    #[error("intra UVCfls state failed: {0}")]
    UvCflState(#[from] TileUvCflStateError),
    #[error("partition frontier mismatch: {reason}")]
    UnexpectedFrontier { reason: &'static str },
}

pub(crate) fn consume_tile_lr_unit_frontier(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    limits: DecodeLimits,
) -> Result<TileLoopRestorationRootFrontier, TilePartitionFrontierError> {
    let frame = minimal_partition_frame_facts(sequence, core)?;
    let (mi_rows, mi_cols) = frame_mi_dimensions(core)?;
    ensure_mi_size_allocation_within_limits(mi_rows, mi_cols, frame.sb_size(), limits)?;
    let mi_size_state = TileMiSizeState::new(mi_rows, mi_cols, frame.sb_size())?;
    let root = consume_tile_loop_restoration_root_frontier(TilePartitionTraversalInput::new(
        work_unit,
        frame,
        mi_size_state.context_state(),
        limits,
    ))?;
    Ok(root)
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum GeneralIntraMultiblockError<E> {
    #[error("general intra multi-block setup failed: {0}")]
    Setup(#[from] TilePartitionFrontierError),
    #[error("general intra multi-block tree walk failed: {0}")]
    Walk(GeneralIntraTreeWalkError<E>),
}

pub(crate) struct GeneralIntraMultiblockCursor<'payload> {
    tree: GeneralIntraPartitionTreeCursor<'payload>,
    mi_size_state: TileMiSizeState,
    joint_modes: TileIntraJointModeState,
    uses_mrls: TileUsesMrlsState,
    use_dip: TileUseDipState,
    fsc_modes: TileFscModeState,
    palette_y: TileLumaPaletteState,
    uv_cfls: TileUvCflState,
}

impl<'payload> GeneralIntraMultiblockCursor<'payload> {
    pub(crate) fn new(
        work_unit: &DecodeTileWorkUnit<'payload>,
        sequence: &SequenceHeader,
        core: &FrameHeaderCore,
        limits: DecodeLimits,
    ) -> Result<Self, TilePartitionFrontierError> {
        let frame = minimal_partition_frame_facts(sequence, core)?;
        let (mi_rows, mi_cols) = frame_mi_dimensions(core)?;
        let mi_size_state = TileMiSizeState::new(mi_rows, mi_cols, frame.sb_size())?;
        let joint_modes = TileIntraJointModeState::new(mi_rows, mi_cols)?;
        let sb_size4 = frame
            .sb_size()
            .num_4x4_wide()
            .map_err(TilePartitionTraversalError::from)?
            .max(1);
        let uses_mrls = TileUsesMrlsState::new(mi_rows, mi_cols, sb_size4)?;
        let use_dip = TileUseDipState::new(mi_rows, mi_cols, sb_size4)?;
        let fsc_modes = TileFscModeState::new(mi_rows, mi_cols, sb_size4)?;
        let palette_y = TileLumaPaletteState::new(mi_rows, mi_cols, sb_size4)?;
        let uv_cfls = TileUvCflState::new(mi_rows, mi_cols)?;
        let tree = GeneralIntraPartitionTreeCursor::new(work_unit, frame, limits)?;
        Ok(Self {
            tree,
            mi_size_state,
            joint_modes,
            uses_mrls,
            use_dip,
            fsc_modes,
            palette_y,
            uv_cfls,
        })
    }

    pub(crate) fn decode_next_superblock<E, C, F, P>(
        &mut self,
        work_unit: &mut DecodeTileWorkUnit<'payload>,
        on_leaf: &mut F,
        on_published: &mut P,
    ) -> Result<Option<[usize; 2]>, GeneralIntraMultiblockError<E>>
    where
        F: FnMut(
            &mut DecodeTileWorkUnit<'payload>,
            &mut SymbolDecoder<'payload>,
            &DecodeBlockFrontier,
            &TileIntraJointModeState,
            &TileUsesMrlsState,
            &TileUseDipState,
            &TileFscModeState,
            &TileLumaPaletteState,
            IsCflContext,
            DecodedLeafPublication,
        ) -> Result<(GeneralIntraLeafMode, C), E>,
        P: FnMut(DecodedLeafPublication, C),
    {
        self.tree
            .decode_next_superblock_with_publication(
                work_unit,
                &mut self.mi_size_state,
                &mut self.joint_modes,
                &mut self.uses_mrls,
                &mut self.use_dip,
                &mut self.fsc_modes,
                &mut self.palette_y,
                &mut self.uv_cfls,
                on_leaf,
                on_published,
            )
            .map_err(GeneralIntraMultiblockError::Walk)
    }

    pub(crate) fn into_output(self) -> GeneralIntraMultiblockOutput<'payload> {
        self.tree.into_output()
    }
}

fn plan_tile_partition_frontier<'payload>(
    work_unit: &mut DecodeTileWorkUnit<'payload>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    limits: DecodeLimits,
) -> Result<
    (
        SymbolDecoder<'payload>,
        TileMiSizeState,
        DecodeBlockFrontier,
    ),
    TilePartitionFrontierError,
> {
    let frame = minimal_partition_frame_facts(sequence, core)?;
    let (mi_rows, mi_cols) = frame_mi_dimensions(core)?;
    ensure_mi_size_allocation_within_limits(mi_rows, mi_cols, frame.sb_size(), limits)?;

    let mi_size_state = TileMiSizeState::new(mi_rows, mi_cols, frame.sb_size())?;
    let cursor = plan_tile_partition_traversal_cursor(TilePartitionTraversalInput::new(
        work_unit,
        frame,
        mi_size_state.context_state(),
        limits,
    ))?;
    let (plan, symbols) = cursor.into_parts();
    ensure_minimal_root_frontier(&plan, &symbols)?;
    let frontier = plan.frontier();

    Ok((symbols, mi_size_state, frontier))
}

fn ensure_mi_size_allocation_within_limits(
    mi_rows: usize,
    mi_cols: usize,
    sb_size: BlockSize,
    limits: DecodeLimits,
) -> Result<(), TilePartitionFrontierError> {
    let allocation = TileMiSizeState::allocation(mi_rows, mi_cols, sb_size)?;
    limits.ensure_allocation_len(
        DecodeLimitName::MaxLumaSamplesPerFrame,
        allocation.padded_grid_cells() as u64,
    )?;
    let allocation_bytes = limits
        .ensure_mul(
            DecodeLimitName::MaxDecodedFrameBytes,
            allocation.entry_count() as u64,
            size_of::<usize>() as u64,
        )?
        .actual();
    limits.ensure_allocation_len(DecodeLimitName::MaxDecodedFrameBytes, allocation_bytes)?;
    Ok(())
}

pub(crate) fn minimal_partition_frame_facts(
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
) -> Result<TilePartitionFrameFacts, TilePartitionFrontierError> {
    let partition = sequence
        .partition
        .ok_or(TilePartitionFrontierError::MissingFact {
            fact: "sequence.partition",
        })?;
    let frame_is_intra = core
        .frame_is_intra
        .ok_or(TilePartitionFrontierError::MissingFact {
            fact: "frame_is_intra",
        })?;
    let (mi_rows, mi_cols) = frame_mi_dimensions(core)?;
    let chroma = sequence.general.chroma_format_idc;
    let num_planes = if chroma.is_monochrome() { 1 } else { 3 };
    let (subsampling_x, subsampling_y) = chroma_subsampling(chroma);
    let filter = sequence
        .filter
        .ok_or(TilePartitionFrontierError::MissingFact {
            fact: "sequence.filter",
        })?;
    let lr_params = core
        .lr_params
        .as_ref()
        .ok_or(TilePartitionFrontierError::MissingFact { fact: "lr_params" })?;
    let loop_restoration = loop_restoration_state(lr_params, num_planes);

    let sb_size_index = frame_sb_size_index(partition.seq_sb_size(), frame_is_intra);

    Ok(TilePartitionFrameFacts::new(
        mi_rows,
        mi_cols,
        sb_size_index,
        num_planes,
        subsampling_x,
        subsampling_y,
        frame_is_intra,
        partition.enable_sdp,
        partition.enable_extended_sdp,
        filter.disable_loopfilters_across_tiles,
        loop_restoration,
        PartitionFeatureFlags::new(
            partition.enable_ext_partitions,
            partition.enable_uneven_4way_partitions,
        ),
        partition.max_pb_aspect_ratio as usize,
        num_planes > 1,
        TilePartitionBruState::Active,
    )?)
}

fn loop_restoration_state(lr: &LrParams, num_planes: usize) -> TilePartitionLoopRestorationState {
    if !lr.uses_lr {
        return TilePartitionLoopRestorationState::NoSyntax;
    }
    let mut plane_tool = [TilePartitionLoopRestorationPlaneTool::None; 3];
    let mut frame_filters_on = [false; 3];
    let mut unit_size = [0usize; 3];
    for plane in 0..num_planes.min(3) {
        let Some(params) = lr.planes.get(plane) else {
            return TilePartitionLoopRestorationState::UnsupportedReadLrSyntax;
        };
        match params.restoration_type {
            FrameRestorationType::None => {}
            FrameRestorationType::WienerNonsep => {
                plane_tool[plane] = TilePartitionLoopRestorationPlaneTool::WienerNs;
                frame_filters_on[plane] = params.frame_filters_on;
                unit_size[plane] = lr.loop_restoration_size[plane] as usize;
            }
            FrameRestorationType::PcWiener if plane == 0 => {
                plane_tool[plane] = TilePartitionLoopRestorationPlaneTool::PcWiener;
                unit_size[plane] = lr.loop_restoration_size[plane] as usize;
            }
            FrameRestorationType::Switchable if plane == 0 && params.frame_filters_on => {
                plane_tool[plane] = TilePartitionLoopRestorationPlaneTool::Switchable;
                frame_filters_on[plane] = true;
                unit_size[plane] = lr.loop_restoration_size[plane] as usize;
            }
            FrameRestorationType::PcWiener | FrameRestorationType::Switchable => {
                return TilePartitionLoopRestorationState::UnsupportedReadLrSyntax;
            }
        }
    }
    if plane_tool
        .iter()
        .any(|tool| *tool != TilePartitionLoopRestorationPlaneTool::None)
    {
        TilePartitionLoopRestorationState::Frame(TilePartitionLoopRestorationFrameState::new(
            plane_tool,
            frame_filters_on,
            unit_size,
        ))
    } else {
        TilePartitionLoopRestorationState::UnsupportedReadLrSyntax
    }
}

pub(crate) fn frame_mi_dimensions(
    core: &FrameHeaderCore,
) -> Result<(usize, usize), TilePartitionFrontierError> {
    let tile_info = core
        .tile_info
        .as_ref()
        .ok_or(TilePartitionFrontierError::MissingFact { fact: "tile_info" })?;
    let mi_rows =
        *tile_info
            .mi_row_starts
            .last()
            .ok_or(TilePartitionFrontierError::MissingFact {
                fact: "mi_row_starts",
            })? as usize;
    let mi_cols =
        *tile_info
            .mi_col_starts
            .last()
            .ok_or(TilePartitionFrontierError::MissingFact {
                fact: "mi_col_starts",
            })? as usize;
    if mi_rows == 0 || mi_cols == 0 {
        return Err(TilePartitionFrontierError::UnexpectedFrontier {
            reason: "empty_mi_dimensions",
        });
    }
    Ok((mi_rows, mi_cols))
}

fn ensure_minimal_root_frontier(
    plan: &TilePartitionTraversalPlan,
    symbols: &SymbolDecoder<'_>,
) -> Result<(), TilePartitionFrontierError> {
    let steps = plan.steps();
    let [step] = steps else {
        return unexpected("unexpected_partition_step_count");
    };
    if step.call.r != 0 || step.call.c != 0 {
        return unexpected("non_root_partition_call");
    }
    if step.decision.partition != PartitionType::None
        || step.decision.trace.do_split != Some(false)
        || step.symbol_count_before != 0
        || step.symbol_count_after != 1
    {
        return unexpected("unexpected_root_partition_decision");
    }
    let frontier = plan.frontier();
    if frontier.r != 0
        || frontier.c != 0
        || frontier.b_size.index() != BLOCK_64X64_INDEX
        || frontier.symbol_count_before_block != 1
        || !plan.pending_children().is_empty()
        || plan.symbol_count_after() != 1
    {
        return unexpected("unexpected_decode_block_frontier");
    }
    let checkpoint = frontier.symbol_checkpoint_before_block;
    if checkpoint.symbol_count != symbols.symbol_count()
        || checkpoint.consumed_bits != symbols.consumed_bits()
        || checkpoint != symbols.checkpoint()
    {
        return unexpected("frontier_symbol_checkpoint_mismatch");
    }
    Ok(())
}

fn unexpected<T>(reason: &'static str) -> Result<T, TilePartitionFrontierError> {
    Err(TilePartitionFrontierError::UnexpectedFrontier { reason })
}

fn frame_sb_size_index(seq_sb_size: SuperblockSize, frame_is_intra: bool) -> usize {
    match (seq_sb_size, frame_is_intra) {
        (SuperblockSize::Block256x256, true) | (SuperblockSize::Block128x128, _) => {
            BLOCK_128X128_INDEX
        }
        (SuperblockSize::Block256x256, false) => BLOCK_256X256_INDEX,
        (SuperblockSize::Block64x64, _) => BLOCK_64X64_INDEX,
    }
}

pub(crate) fn chroma_subsampling(chroma: ChromaFormatIdc) -> (bool, bool) {
    match chroma {
        ChromaFormatIdc::Yuv420 | ChromaFormatIdc::Monochrome => (true, true),
        ChromaFormatIdc::Yuv422 => (true, false),
        ChromaFormatIdc::Yuv444 => (false, false),
    }
}

#[cfg(test)]
#[path = "tile_frontier_tests.rs"]
mod tests;

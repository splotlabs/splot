// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use core::mem::size_of;

use splot_core::headers::frame::{FrameHeaderCore, FrameRestorationType, LrParams};
use splot_core::headers::sequence::{ChromaFormatIdc, SequenceHeader, SuperblockSize};
use splot_core::symbol::SymbolDecoder;

use super::DecodeTileWorkUnit;
use super::block_decoded_state::TileBlockDecodedState;
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
use super::partition_traversal::{
    DecodeBlockFrontier, GeneralIntraLeafMode, GeneralIntraPartitionTreeOutput,
    GeneralIntraTreeWalkError, TileLoopRestorationRootFrontier, TilePartitionBruState,
    TilePartitionFrameFacts, TilePartitionLoopRestorationFrameState,
    TilePartitionLoopRestorationPlaneTool, TilePartitionLoopRestorationState,
    TilePartitionTraversalError, TilePartitionTraversalInput, TilePartitionTraversalPlan,
    WienerNsLrSourceBlock, WienerNsLrUnitFilter, consume_tile_loop_restoration_root_frontier,
    decode_general_intra_partition_tree, plan_tile_partition_traversal_cursor,
};
use crate::{DecodeLimitError, DecodeLimitName, DecodeLimits};

const BLOCK_64X64_INDEX: usize = 12;
const BLOCK_128X128_INDEX: usize = 15;
const BLOCK_256X256_INDEX: usize = 18;

#[derive(Debug, thiserror::Error)]
pub(crate) enum TilePartitionFrontierError {
    #[error("minimal-tier partition frontier missing fact: {fact}")]
    MissingFact { fact: &'static str },
    #[error("minimal-tier partition frontier limit failed: {0}")]
    Limit(#[from] DecodeLimitError),
    #[error("minimal-tier partition traversal failed: {0}")]
    Traversal(#[from] TilePartitionTraversalError),
    #[error("minimal-tier MI-size state failed: {0}")]
    MiSizeState(#[from] TileMiSizeStateError),
    #[error("minimal-tier intra joint-mode state failed: {0}")]
    IntraJointModeState(#[from] TileIntraJointModeStateError),
    #[error("minimal-tier intra UsesMrls state failed: {0}")]
    UsesMrlsState(#[from] TileUsesMrlsStateError),
    #[error("minimal-tier intra UseDip state failed: {0}")]
    UseDipState(#[from] TileUseDipStateError),
    #[error("minimal-tier intra FscModes state failed: {0}")]
    FscModeState(#[from] TileFscModeStateError),
    #[error("minimal-tier luma palette state failed: {0}")]
    LumaPaletteState(#[from] TileLumaPaletteStateError),
    #[error("minimal-tier intra UVCfls state failed: {0}")]
    UvCflState(#[from] TileUvCflStateError),
    #[error("minimal-tier partition frontier mismatch: {reason}")]
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
    let root = mi_size_state.with_context_state(|context| {
        consume_tile_loop_restoration_root_frontier(TilePartitionTraversalInput::new(
            work_unit, frame, context, limits,
        ))
    })??;
    Ok(root)
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum GeneralIntraMultiblockError<E> {
    #[error("general intra multi-block setup failed: {0}")]
    Setup(#[from] TilePartitionFrontierError),
    #[error("general intra multi-block tree walk failed: {0}")]
    Walk(GeneralIntraTreeWalkError<E>),
}

pub(crate) struct GeneralIntraMultiblockOutput<'payload> {
    pub(crate) symbols: SymbolDecoder<'payload>,
    pub(crate) active_source_blocks: Vec<WienerNsLrSourceBlock>,
    pub(crate) unit_filters: Vec<WienerNsLrUnitFilter>,
}

#[allow(dead_code)]
pub(crate) fn decode_general_intra_multiblock_tree<'payload, E, F>(
    work_unit: &mut DecodeTileWorkUnit<'payload>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    limits: DecodeLimits,
    on_leaf: F,
) -> Result<SymbolDecoder<'payload>, GeneralIntraMultiblockError<E>>
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
        &mut TileBlockDecodedState,
    ) -> Result<GeneralIntraLeafMode, E>,
{
    Ok(decode_general_intra_multiblock_tree_impl(
        work_unit, sequence, core, limits, false, on_leaf,
    )?
    .symbols)
}

pub(crate) fn decode_general_intra_multiblock_tree_with_lr_source_blocks<'payload, E, F>(
    work_unit: &mut DecodeTileWorkUnit<'payload>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    limits: DecodeLimits,
    on_leaf: F,
) -> Result<GeneralIntraMultiblockOutput<'payload>, GeneralIntraMultiblockError<E>>
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
        &mut TileBlockDecodedState,
    ) -> Result<GeneralIntraLeafMode, E>,
{
    decode_general_intra_multiblock_tree_impl(work_unit, sequence, core, limits, true, on_leaf)
}

fn decode_general_intra_multiblock_tree_impl<'payload, E, F>(
    work_unit: &mut DecodeTileWorkUnit<'payload>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    limits: DecodeLimits,
    retain_lr_source_blocks: bool,
    on_leaf: F,
) -> Result<GeneralIntraMultiblockOutput<'payload>, GeneralIntraMultiblockError<E>>
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
        &mut TileBlockDecodedState,
    ) -> Result<GeneralIntraLeafMode, E>,
{
    let frame = minimal_partition_frame_facts(sequence, core)?;
    let (mi_rows, mi_cols) = frame_mi_dimensions(core)?;
    let mut mi_size_state = TileMiSizeState::new(mi_rows, mi_cols, frame.sb_size())
        .map_err(TilePartitionFrontierError::from)?;
    let mut joint_modes =
        TileIntraJointModeState::new(mi_rows, mi_cols).map_err(TilePartitionFrontierError::from)?;
    let sb_size4 = frame
        .sb_size()
        .num_4x4_wide()
        .map_err(TilePartitionTraversalError::from)
        .map_err(TilePartitionFrontierError::from)?
        .max(1);
    let mut uses_mrls = TileUsesMrlsState::new(mi_rows, mi_cols, sb_size4)
        .map_err(TilePartitionFrontierError::from)?;
    let mut use_dip = TileUseDipState::new(mi_rows, mi_cols, sb_size4)
        .map_err(TilePartitionFrontierError::from)?;
    let mut fsc_modes = TileFscModeState::new(mi_rows, mi_cols, sb_size4)
        .map_err(TilePartitionFrontierError::from)?;
    let mut palette_y = TileLumaPaletteState::new(mi_rows, mi_cols, sb_size4)
        .map_err(TilePartitionFrontierError::from)?;
    let mut uv_cfls =
        TileUvCflState::new(mi_rows, mi_cols).map_err(TilePartitionFrontierError::from)?;
    let output: GeneralIntraPartitionTreeOutput<'payload> = decode_general_intra_partition_tree(
        work_unit,
        frame,
        &mut mi_size_state,
        &mut joint_modes,
        &mut uses_mrls,
        &mut use_dip,
        &mut fsc_modes,
        &mut palette_y,
        &mut uv_cfls,
        limits,
        retain_lr_source_blocks,
        on_leaf,
    )
    .map_err(GeneralIntraMultiblockError::Walk)?;
    Ok(GeneralIntraMultiblockOutput {
        symbols: output.symbols,
        active_source_blocks: output.active_source_blocks,
        unit_filters: output.unit_filters,
    })
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
    let cursor = mi_size_state.with_context_state(|context| {
        plan_tile_partition_traversal_cursor(TilePartitionTraversalInput::new(
            work_unit, frame, context, limits,
        ))
    })??;
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

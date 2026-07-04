// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use core::mem::size_of;

use splot_core::headers::frame::{FrameHeaderCore, FrameRestorationType, LrParams};
use splot_core::headers::sequence::{ChromaFormatIdc, SequenceHeader, SuperblockSize};
use splot_core::symbol::{SymbolDecoder, SymbolDecoderSummary};

use super::DecodeTileWorkUnit;
use super::block_decoded_state::TileBlockDecodedState;
use super::block_symbol::{MinimalBlockSymbolTraceError, consume_minimal_block_symbol_trace};
use super::intra_joint_modes::{
    IsCflContext, TileFscModeState, TileFscModeStateError, TileIntraJointModeState,
    TileIntraJointModeStateError, TileLumaPaletteState, TileLumaPaletteStateError,
    TileUsesMrlsState, TileUsesMrlsStateError, TileUvCflState, TileUvCflStateError,
};
use super::mi_size_state::{TileMiSizeState, TileMiSizeStateError};
use super::partition::PartitionType;
use super::partition_allowed::PartitionFeatureFlags;
use super::partition_size::BlockSize;
use super::partition_traversal::{
    DecodeBlockFrontier, GeneralIntraLeafMode, GeneralIntraPartitionTreeOutput,
    GeneralIntraTreeWalkError, TileLoopRestorationRootFrontier, TilePartitionBruState,
    TilePartitionFrameFacts, TilePartitionLoopRestorationState, TilePartitionTraversalError,
    TilePartitionTraversalInput, TilePartitionTraversalPlan,
    TilePartitionWienerNsLoopRestorationState, WienerNsLrSourceBlock, WienerNsLrUnitFilter,
    consume_tile_loop_restoration_root_frontier, decode_general_intra_partition_tree,
    plan_tile_partition_traversal_cursor,
};
use crate::{DecodeLimitError, DecodeLimitName, DecodeLimits};

const BLOCK_64X64_INDEX: usize = 12;
const BLOCK_128X128_INDEX: usize = 15;
const BLOCK_256X256_INDEX: usize = 18;

#[derive(Debug)]
pub(crate) struct TileBlockSymbolFrontier {
    summary: SymbolDecoderSummary,
    reconstruction_trace: TileReconstructionTrace,
}

impl TileBlockSymbolFrontier {
    #[must_use]
    pub(crate) const fn summary(&self) -> SymbolDecoderSummary {
        self.summary
    }

    #[must_use]
    pub(crate) const fn reconstruction_trace(&self) -> TileReconstructionTrace {
        self.reconstruction_trace
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TileReconstructionTrace {
    LumaDcNoResidual8Bit420_64x64,
}

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
    #[error("minimal-tier intra FscModes state failed: {0}")]
    FscModeState(#[from] TileFscModeStateError),
    #[error("minimal-tier luma palette state failed: {0}")]
    LumaPaletteState(#[from] TileLumaPaletteStateError),
    #[error("minimal-tier intra UVCfls state failed: {0}")]
    UvCflState(#[from] TileUvCflStateError),
    #[error("minimal-tier partition frontier mismatch: {reason}")]
    UnexpectedFrontier { reason: &'static str },
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum TileBlockSymbolFrontierError {
    #[error("minimal-tier partition frontier failed: {0}")]
    Partition(#[from] TilePartitionFrontierError),
    #[error("minimal-tier block-symbol frontier failed: {0}")]
    Block(#[from] MinimalBlockSymbolTraceError),
}

pub(crate) fn plan_tile_block_symbol_frontier(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    limits: DecodeLimits,
) -> Result<TileBlockSymbolFrontier, TileBlockSymbolFrontierError> {
    let (symbols, mut mi_size_state, block_frontier) =
        plan_tile_partition_frontier(work_unit, sequence, core, limits)?;
    let tile_num = work_unit.tile_num();
    let trace = consume_minimal_block_symbol_trace(work_unit, symbols)?;
    mi_size_state
        .update_luma_block(block_frontier.r, block_frontier.c, block_frontier.b_size)
        .map_err(TilePartitionFrontierError::from)?;
    work_unit.cdf_mut().apply_completed_tile_to_saved(tile_num);
    work_unit.cdf_mut().frame_end_update_cdf_subset();
    Ok(TileBlockSymbolFrontier {
        summary: trace.summary(),
        reconstruction_trace: TileReconstructionTrace::LumaDcNoResidual8Bit420_64x64,
    })
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
    let mut plane_enabled = [false; 3];
    let mut frame_filters_on = [false; 3];
    let mut unit_size = [0usize; 3];
    for plane in 0..num_planes.min(3) {
        let Some(params) = lr.planes.get(plane) else {
            return TilePartitionLoopRestorationState::UnsupportedReadLrSyntax;
        };
        match params.restoration_type {
            FrameRestorationType::None => {}
            FrameRestorationType::WienerNonsep => {
                if params.frame_filters_on && params.frame_filter_bank.is_none() {
                    return TilePartitionLoopRestorationState::UnsupportedReadLrSyntax;
                }
                plane_enabled[plane] = true;
                frame_filters_on[plane] = params.frame_filters_on;
                unit_size[plane] = lr.loop_restoration_size[plane] as usize;
            }
            FrameRestorationType::PcWiener | FrameRestorationType::Switchable => {
                return TilePartitionLoopRestorationState::UnsupportedReadLrSyntax;
            }
        }
    }
    if plane_enabled.iter().any(|enabled| *enabled) {
        TilePartitionLoopRestorationState::FrameWienerNs(
            TilePartitionWienerNsLoopRestorationState::new(
                plane_enabled,
                frame_filters_on,
                unit_size,
            ),
        )
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
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use splot_core::bitio::BitReader;
    use splot_core::headers::frame::{
        FrameHeaderCore, FrameHeaderParseInput, FrameHeaderParseMode, FrameReferenceStateView,
        parse_frame_header_core,
    };
    use splot_core::headers::sequence::{SequenceHeader, parse_sequence_header};
    use splot_core::stream::{ParsedBitstream, parse_bitstream_partial};
    use splot_parallel::ThreadCount;

    use super::super::{
        DecodeTileWorkUnit, FrameCandidateCdfFacts, FrameCandidateCoeffFacts,
        FrameCandidateTileBoundaryInput, FrameCandidateTileFacts, TileGroupPositionFacts,
        plan_derived_tile_payload_boundary,
    };
    use super::*;
    use crate::{
        DecodeContext, DecodeLimitName, DecodeLimitThreshold, DecodeOptions, DecodeRuntimeConfig,
    };

    const LEGACY_INVERTED_SKIP_TRACE: &[u8] = &[
        0x44, 0x4b, 0x49, 0x46, 0x00, 0x00, 0x20, 0x00, 0x41, 0x56, 0x30, 0x32, 0x40, 0x00, 0x40,
        0x00, 0x1e, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x16, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
        0x08, 0x0c, 0x04, 0x82, 0x0a, 0x55, 0xff, 0xf1, 0xc2, 0x0d, 0xd7, 0x0b, 0x70, 0x11, 0x06,
        0x10, 0xe3, 0xfe, 0x21, 0x2f, 0xba,
    ];

    #[test]
    fn legacy_inverted_skip_trace_fails_closed_before_output() {
        with_minimal_work_unit(LEGACY_INVERTED_SKIP_TRACE, |work_unit, sequence, core| {
            let Err(err) =
                plan_tile_partition_frontier(work_unit, sequence, core, DecodeLimits::DEFAULT)
            else {
                panic!("retired payload unexpectedly reached the runtime frontier")
            };

            assert!(
                matches!(
                    err,
                    TilePartitionFrontierError::UnexpectedFrontier {
                        reason: "unexpected_decode_block_frontier"
                    }
                ),
                "expected the retired payload to fail closed at the runtime frontier, got {err:?}"
            );
        });
    }

    #[test]
    fn partition_frontier_checks_padded_mi_state_cells_before_allocation() {
        with_minimal_work_unit(LEGACY_INVERTED_SKIP_TRACE, |work_unit, sequence, core| {
            let mut padded_core = core.clone();
            let tile_info = padded_core.tile_info.as_mut().unwrap();
            *tile_info.mi_row_starts.last_mut().unwrap() = 17;
            *tile_info.mi_col_starts.last_mut().unwrap() = 16;
            let limits = DecodeLimits::unlimited()
                .with_max_luma_samples_per_frame(DecodeLimitThreshold::Max(300));

            let Err(err) = plan_tile_partition_frontier(work_unit, sequence, &padded_core, limits)
            else {
                panic!("expected padded MI-state limit");
            };

            let TilePartitionFrontierError::Limit(limit) = err else {
                panic!("expected padded MI-state limit, got {err:?}");
            };
            assert_eq!(limit.name(), DecodeLimitName::MaxLumaSamplesPerFrame);
            assert_eq!(limit.actual(), Some(512));
        });
    }

    #[test]
    fn partition_frontier_checks_mi_state_byte_budget_before_allocation() {
        with_minimal_work_unit(LEGACY_INVERTED_SKIP_TRACE, |work_unit, sequence, core| {
            let limits = DecodeLimits::unlimited()
                .with_max_decoded_frame_bytes(DecodeLimitThreshold::Max(1024));

            let Err(err) = plan_tile_partition_frontier(work_unit, sequence, core, limits) else {
                panic!("expected MI-state byte limit");
            };

            let TilePartitionFrontierError::Limit(limit) = err else {
                panic!("expected MI-state byte limit, got {err:?}");
            };
            assert_eq!(limit.name(), DecodeLimitName::MaxDecodedFrameBytes);
            assert_eq!(
                limit.actual(),
                Some((2 * (256 + 16 + 16) * size_of::<usize>()) as u64)
            );
        });
    }

    #[test]
    fn lr_unit_frontier_checks_padded_mi_state_cells_before_allocation() {
        with_minimal_work_unit(LEGACY_INVERTED_SKIP_TRACE, |work_unit, sequence, core| {
            let mut padded_core = core.clone();
            let tile_info = padded_core.tile_info.as_mut().unwrap();
            *tile_info.mi_row_starts.last_mut().unwrap() = 17;
            *tile_info.mi_col_starts.last_mut().unwrap() = 16;
            let limits = DecodeLimits::unlimited()
                .with_max_luma_samples_per_frame(DecodeLimitThreshold::Max(300));

            let Err(err) = consume_tile_lr_unit_frontier(work_unit, sequence, &padded_core, limits)
            else {
                panic!("expected padded MI-state limit");
            };

            let TilePartitionFrontierError::Limit(limit) = err else {
                panic!("expected padded MI-state limit, got {err:?}");
            };
            assert_eq!(limit.name(), DecodeLimitName::MaxLumaSamplesPerFrame);
            assert_eq!(limit.actual(), Some(512));
        });
    }

    #[test]
    fn lr_unit_frontier_checks_mi_state_byte_budget_before_allocation() {
        with_minimal_work_unit(LEGACY_INVERTED_SKIP_TRACE, |work_unit, sequence, core| {
            let limits = DecodeLimits::unlimited()
                .with_max_decoded_frame_bytes(DecodeLimitThreshold::Max(1024));

            let Err(err) = consume_tile_lr_unit_frontier(work_unit, sequence, core, limits) else {
                panic!("expected MI-state byte limit");
            };

            let TilePartitionFrontierError::Limit(limit) = err else {
                panic!("expected MI-state byte limit, got {err:?}");
            };
            assert_eq!(limit.name(), DecodeLimitName::MaxDecodedFrameBytes);
            assert_eq!(
                limit.actual(),
                Some((2 * (256 + 16 + 16) * size_of::<usize>()) as u64)
            );
        });
    }

    fn with_minimal_work_unit<R>(
        bytes: &[u8],
        f: impl FnOnce(&mut DecodeTileWorkUnit<'_>, &SequenceHeader, &FrameHeaderCore) -> R + Send,
    ) -> R
    where
        R: Send,
    {
        let context =
            DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize))).unwrap();
        context.pool().install(move || {
            let context =
                DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize))).unwrap();
            let plan = context.plan_bytes(bytes, DecodeOptions::default()).unwrap();
            let candidate = plan.frame_candidates().next().unwrap();

            let ParsedBitstream::Ivf(ivf) = parse_bitstream_partial(bytes) else {
                panic!("minimal fixture must parse as IVF");
            };
            let frame = ivf.frames.first().unwrap();
            let [_, sequence_envelope, frame_envelope] = frame.obus.as_slice() else {
                panic!("minimal fixture must contain temporal delimiter, sequence, and frame OBUs");
            };

            let mut sequence_reader = BitReader::new(
                sequence_envelope.payload,
                sequence_envelope.payload_offset(),
            );
            let sequence = parse_sequence_header(&mut sequence_reader).unwrap();

            let mut frame_reader =
                BitReader::new(frame_envelope.payload, frame_envelope.payload_offset());
            assert_ne!(frame_reader.read_bit().unwrap(), 0);
            let frame_input = FrameHeaderParseInput {
                obu_type: frame_envelope.header.obu_type,
                first_picture_in_tu: true,
                active_sequence: Some(&sequence),
                mfh_record: None,
                reference_state: FrameReferenceStateView::unknown(),
                mode: FrameHeaderParseMode::Core,
            };
            let core = parse_frame_header_core(&mut frame_reader, &frame_input).unwrap();

            let tq = sequence.transform_quant_entropy.as_ref().unwrap();
            let coeff = FrameCandidateCoeffFacts::from_tq(tq);
            let facts = FrameCandidateTileFacts::from_frame_core(&core, coeff).unwrap();
            let cdf = FrameCandidateCdfFacts::new(tq.enable_avg_cdf, tq.avg_cdf_type != 0);
            let input = FrameCandidateTileBoundaryInput::new(
                &plan,
                candidate,
                bytes,
                *frame_envelope,
                TileGroupPositionFacts::new(true, true),
                facts,
                cdf,
                DecodeLimits::DEFAULT,
            );
            let mut tile_plan = plan_derived_tile_payload_boundary(&input).unwrap();
            let [work_unit] = tile_plan.work_units_mut() else {
                panic!("minimal fixture must derive one tile work unit");
            };

            f(work_unit, &sequence, &core)
        })
    }
}

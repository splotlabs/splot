// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Minimal runtime bridge into the partition traversal frontier.
//!
//! Feature tracking: `DECODE-MINIMAL-TIER-RUNTIME-SUCCESS`,
//! `DECODE-TILE-PARTITION-TRAVERSAL-BOUNDARY`,
//! `DECODE-MINIMAL-BLOCK-SYNTAX-FRONTIER`.

use core::mem::size_of;

use splot_core::headers::frame::{FrameHeaderCore, FrameRestorationType, LrParams};
use splot_core::headers::sequence::{ChromaFormatIdc, SequenceHeader, SuperblockSize};
use splot_core::symbol::{SymbolDecoder, SymbolDecoderSummary};

use super::DecodeTileWorkUnit;
use super::block_decoded_state::TileBlockDecodedState;
use super::block_symbol::{MinimalBlockSymbolTraceError, consume_minimal_block_symbol_trace};
use super::intra_joint_modes::{TileIntraJointModeState, TileIntraJointModeStateError};
use super::mi_size_state::{TileMiSizeState, TileMiSizeStateError};
use super::partition::PartitionType;
use super::partition_allowed::PartitionFeatureFlags;
use super::partition_size::BlockSize;
use super::partition_traversal::{
    DecodeBlockFrontier, GeneralIntraTreeWalkError, TilePartitionBruState, TilePartitionFrameFacts,
    TilePartitionLoopRestorationState, TilePartitionTraversalError, TilePartitionTraversalInput,
    TilePartitionTraversalPlan, TilePartitionWienerNsLoopRestorationState,
    consume_tile_loop_restoration_root_frontier, decode_general_intra_partition_tree,
    plan_tile_partition_traversal_cursor,
};
use crate::{DecodeLimitError, DecodeLimitName, DecodeLimitOp, DecodeLimits};

const BLOCK_64X64_INDEX: usize = 12;
const BLOCK_128X128_INDEX: usize = 15;
const BLOCK_256X256_INDEX: usize = 18;

/// Live symbol cursor positioned at the minimal runtime block frontier.
pub(crate) struct MinimalRuntimePartitionFrontier<'payload> {
    symbols: SymbolDecoder<'payload>,
    mi_size_state: TileMiSizeState,
    frontier: DecodeBlockFrontier,
}

impl<'payload> MinimalRuntimePartitionFrontier<'payload> {
    /// Consumes the frontier and returns the live symbol decoder.
    #[must_use]
    pub(crate) fn into_symbol_decoder(self) -> SymbolDecoder<'payload> {
        self.symbols
    }

    /// Splits the frontier into the live symbol decoder and MI-size state.
    #[must_use]
    fn into_parts(
        self,
    ) -> (
        SymbolDecoder<'payload>,
        TileMiSizeState,
        DecodeBlockFrontier,
    ) {
        (self.symbols, self.mi_size_state, self.frontier)
    }
}

/// Result of the minimal runtime block-symbol frontier.
#[derive(Debug)]
pub(crate) struct MinimalRuntimeBlockSymbolFrontier {
    summary: SymbolDecoderSummary,
    reconstruction_trace: MinimalRuntimeReconstructionTrace,
}

impl MinimalRuntimeBlockSymbolFrontier {
    /// Successful AV2 § 8.2.4 `exit_symbol()` summary after the traced block symbols.
    #[must_use]
    pub(crate) const fn summary(&self) -> SymbolDecoderSummary {
        self.summary
    }

    /// Narrow reconstruction trace facts returned after block-symbol validation.
    #[must_use]
    pub(crate) const fn reconstruction_trace(&self) -> MinimalRuntimeReconstructionTrace {
        self.reconstruction_trace
    }
}

/// AV2 trace facts supported by the current minimal runtime frontier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MinimalRuntimeReconstructionTrace {
    /// 64x64 8-bit 4:2:0 luma DC prediction with no residual path.
    ///
    /// The traced chroma symbol is not a neutral-chroma proof; current minimal
    /// output materializes chroma separately as an output-contract fallback.
    LumaDcNoResidual8Bit420_64x64,
}

/// Error returned while deriving the minimal runtime partition frontier.
#[derive(Debug, thiserror::Error)]
pub(crate) enum MinimalRuntimePartitionFrontierError {
    /// A parser fact required by the runtime bridge is absent.
    #[error("minimal runtime partition frontier missing fact: {fact}")]
    MissingFact {
        /// Missing fact name.
        fact: &'static str,
    },
    /// A bounded runtime allocation or arithmetic check failed.
    #[error("minimal runtime partition frontier limit failed: {0}")]
    Limit(#[from] DecodeLimitError),
    /// The underlying traversal frontier failed.
    #[error("minimal runtime partition traversal failed: {0}")]
    Traversal(#[from] TilePartitionTraversalError),
    /// The MI-size state boundary failed.
    #[error("minimal runtime MI-size state failed: {0}")]
    MiSizeState(#[from] TileMiSizeStateError),
    /// The `IntraJointModes` neighbour-mode state boundary failed.
    #[error("minimal runtime intra joint-mode state failed: {0}")]
    IntraJointModeState(#[from] TileIntraJointModeStateError),
    /// Traversal reached a shape outside the minimal tier.
    #[error("minimal runtime partition frontier mismatch: {reason}")]
    UnexpectedFrontier {
        /// Stable mismatch reason.
        reason: &'static str,
    },
}

/// Error returned while deriving the minimal runtime block-symbol frontier.
#[derive(Debug, thiserror::Error)]
pub(crate) enum MinimalRuntimeBlockSymbolFrontierError {
    /// The prerequisite partition frontier failed.
    #[error("minimal runtime partition frontier failed: {0}")]
    Partition(#[from] MinimalRuntimePartitionFrontierError),
    /// The traced block-symbol frontier failed.
    #[error("minimal runtime block-symbol frontier failed: {0}")]
    Block(#[from] MinimalBlockSymbolTraceError),
}

/// Plans the minimal runtime partition and traced block-symbol frontier.
pub(crate) fn plan_minimal_runtime_block_symbol_frontier<'payload>(
    work_unit: &mut DecodeTileWorkUnit<'payload>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    limits: DecodeLimits,
) -> Result<MinimalRuntimeBlockSymbolFrontier, MinimalRuntimeBlockSymbolFrontierError> {
    let (symbols, mut mi_size_state, block_frontier) =
        plan_minimal_runtime_partition_frontier(work_unit, sequence, core, limits)?.into_parts();
    let tile_num = work_unit.tile_num();
    let trace = consume_minimal_block_symbol_trace(work_unit, symbols)?;
    mi_size_state
        .update_luma_block(block_frontier.r, block_frontier.c, block_frontier.b_size)
        .map_err(MinimalRuntimePartitionFrontierError::from)?;
    work_unit.cdf_mut().apply_completed_tile_to_saved(tile_num);
    work_unit.cdf_mut().frame_end_update_cdf_subset();
    Ok(MinimalRuntimeBlockSymbolFrontier {
        summary: trace.summary(),
        reconstruction_trace: MinimalRuntimeReconstructionTrace::LumaDcNoResidual8Bit420_64x64,
    })
}

/// Consumes the supported superblock-root LR unit syntax and stops before
/// partition or block syntax.
pub(crate) fn consume_minimal_runtime_lr_unit_frontier(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    limits: DecodeLimits,
) -> Result<(), MinimalRuntimePartitionFrontierError> {
    let frame = minimal_partition_frame_facts(sequence, core)?;
    let (mi_rows, mi_cols) = frame_mi_dimensions(core)?;
    ensure_mi_size_allocation_within_limits(mi_rows, mi_cols, frame.sb_size(), limits)?;
    let mi_size_state = TileMiSizeState::new(mi_rows, mi_cols, frame.sb_size())?;
    mi_size_state.with_context_state(|context| {
        consume_tile_loop_restoration_root_frontier(TilePartitionTraversalInput::new(
            work_unit, frame, context, limits,
        ))
    })??;
    Ok(())
}

/// Error from the general intra multi-block tree decode, separating the frame
/// setup (frame facts / MI dimensions / MI-size allocation) from the partition
/// tree walk (whose leaf error `E` is the caller's per-block decode error).
#[derive(Debug, thiserror::Error)]
pub(crate) enum GeneralIntraMultiblockError<E> {
    /// Frame-fact / MI-dimension / MI-size-state setup failed.
    #[error("general intra multi-block setup failed: {0}")]
    Setup(#[from] MinimalRuntimePartitionFrontierError),
    /// The partition tree walk failed.
    #[error("general intra multi-block tree walk failed: {0}")]
    Walk(GeneralIntraTreeWalkError<E>),
}

/// Decodes the complete general intra partition tree for the tile, invoking
/// `on_leaf` at each leaf block in decode order, and returns the live symbol
/// decoder for the caller's § 8.2.4 `exit_symbol()` check. The MI-size partition
/// context and the AV2 § 5.20.5.3 `IntraJointModes` neighbour-mode grid are
/// maintained across blocks internally.
///
/// `on_leaf` receives the shared per-MI `IntraJointModes` grid (read-only, for
/// the § 8.3.2 `y_mode_index` neighbour context) and the superblock-relative
/// § 5.20.2.3 `BlockDecoded` state (read-only, for the § 7.13.2.1 above-right /
/// below-left sentinel availability via § 5.20.7.25 `count_top_right_avail` /
/// `count_bottom_left_avail`), and returns the block's reconstructed
/// `IntraJointMode` (`= modeDelta`), which the walk then records into the grid
/// for that block's MI region so later blocks see it as a neighbour. The walk
/// clears `BlockDecoded` per superblock (§ 5.20.2.3 `clear_block_decoded_flags`)
/// and marks each decoded transform block's 4x4 units after `on_leaf` returns.
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
        &TileBlockDecodedState,
    ) -> Result<u8, E>,
{
    let frame = minimal_partition_frame_facts(sequence, core)?;
    let (mi_rows, mi_cols) = frame_mi_dimensions(core)?;
    let mut mi_size_state = TileMiSizeState::new(mi_rows, mi_cols, frame.sb_size())
        .map_err(MinimalRuntimePartitionFrontierError::from)?;
    let mut joint_modes = TileIntraJointModeState::new(mi_rows, mi_cols)
        .map_err(MinimalRuntimePartitionFrontierError::from)?;
    decode_general_intra_partition_tree(
        work_unit,
        frame,
        &mut mi_size_state,
        &mut joint_modes,
        limits,
        on_leaf,
    )
    .map_err(GeneralIntraMultiblockError::Walk)
}

/// Plans the minimal runtime root partition frontier and returns its live cursor.
pub(crate) fn plan_minimal_runtime_partition_frontier<'payload>(
    work_unit: &mut DecodeTileWorkUnit<'payload>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    limits: DecodeLimits,
) -> Result<MinimalRuntimePartitionFrontier<'payload>, MinimalRuntimePartitionFrontierError> {
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

    Ok(MinimalRuntimePartitionFrontier {
        symbols,
        mi_size_state,
        frontier,
    })
}

fn ensure_mi_size_allocation_within_limits(
    mi_rows: usize,
    mi_cols: usize,
    sb_size: BlockSize,
    limits: DecodeLimits,
) -> Result<(), MinimalRuntimePartitionFrontierError> {
    let allocation = TileMiSizeState::allocation(mi_rows, mi_cols, sb_size)?;
    limits.ensure_allocation_len(
        DecodeLimitName::MaxLumaSamplesPerFrame,
        allocation.padded_grid_cells() as u64,
    )?;
    let allocation_bytes = checked_mul_u64(
        DecodeLimitName::MaxDecodedFrameBytes,
        allocation.entry_count() as u64,
        size_of::<usize>() as u64,
    )?;
    limits.ensure_allocation_len(DecodeLimitName::MaxDecodedFrameBytes, allocation_bytes)?;
    Ok(())
}

pub(crate) fn minimal_partition_frame_facts(
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
) -> Result<TilePartitionFrameFacts, MinimalRuntimePartitionFrontierError> {
    let partition =
        sequence
            .partition
            .ok_or(MinimalRuntimePartitionFrontierError::MissingFact {
                fact: "sequence.partition",
            })?;
    let frame_is_intra =
        core.frame_is_intra
            .ok_or(MinimalRuntimePartitionFrontierError::MissingFact {
                fact: "frame_is_intra",
            })?;
    let (mi_rows, mi_cols) = frame_mi_dimensions(core)?;
    let chroma = sequence.general.chroma_format_idc;
    let num_planes = if chroma.is_monochrome() { 1 } else { 3 };
    let (subsampling_x, subsampling_y) = chroma_subsampling(chroma);
    let loop_restoration = match core.lr_params.as_ref() {
        Some(lr) => loop_restoration_state(lr, num_planes),
        None => {
            return Err(MinimalRuntimePartitionFrontierError::MissingFact { fact: "lr_params" });
        }
    };

    Ok(TilePartitionFrameFacts::new(
        mi_rows,
        mi_cols,
        frame_sb_size_index(partition.seq_sb_size(), frame_is_intra),
        num_planes,
        subsampling_x,
        subsampling_y,
        frame_is_intra,
        partition.enable_sdp,
        partition.enable_extended_sdp,
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
    let mut unit_size = [0usize; 3];
    for plane in 0..num_planes.min(3) {
        let Some(params) = lr.planes.get(plane) else {
            return TilePartitionLoopRestorationState::UnsupportedReadLrSyntax;
        };
        match params.restoration_type {
            FrameRestorationType::None => {}
            FrameRestorationType::WienerNonsep
                if params.frame_filters_on && params.frame_filter_bank.is_some() =>
            {
                plane_enabled[plane] = true;
                unit_size[plane] = lr.loop_restoration_size[plane] as usize;
            }
            FrameRestorationType::PcWiener
            | FrameRestorationType::WienerNonsep
            | FrameRestorationType::Switchable => {
                return TilePartitionLoopRestorationState::UnsupportedReadLrSyntax;
            }
        }
    }
    if plane_enabled.iter().any(|enabled| *enabled) {
        TilePartitionLoopRestorationState::FrameWienerNs(
            TilePartitionWienerNsLoopRestorationState::new(plane_enabled, unit_size),
        )
    } else {
        TilePartitionLoopRestorationState::UnsupportedReadLrSyntax
    }
}

pub(crate) fn frame_mi_dimensions(
    core: &FrameHeaderCore,
) -> Result<(usize, usize), MinimalRuntimePartitionFrontierError> {
    let tile_info = core
        .tile_info
        .as_ref()
        .ok_or(MinimalRuntimePartitionFrontierError::MissingFact { fact: "tile_info" })?;
    let mi_rows = *tile_info.mi_row_starts.last().ok_or(
        MinimalRuntimePartitionFrontierError::MissingFact {
            fact: "mi_row_starts",
        },
    )? as usize;
    let mi_cols = *tile_info.mi_col_starts.last().ok_or(
        MinimalRuntimePartitionFrontierError::MissingFact {
            fact: "mi_col_starts",
        },
    )? as usize;
    if mi_rows == 0 || mi_cols == 0 {
        return Err(MinimalRuntimePartitionFrontierError::UnexpectedFrontier {
            reason: "empty_mi_dimensions",
        });
    }
    Ok((mi_rows, mi_cols))
}

fn ensure_minimal_root_frontier(
    plan: &TilePartitionTraversalPlan,
    symbols: &SymbolDecoder<'_>,
) -> Result<(), MinimalRuntimePartitionFrontierError> {
    let steps = plan.steps();
    let step = match steps {
        [step] => step,
        _ => return unexpected("unexpected_partition_step_count"),
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
        || frontier.b_size.index() != frame_size_block_index()
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

fn unexpected<T>(reason: &'static str) -> Result<T, MinimalRuntimePartitionFrontierError> {
    Err(MinimalRuntimePartitionFrontierError::UnexpectedFrontier { reason })
}

fn frame_sb_size_index(seq_sb_size: SuperblockSize, frame_is_intra: bool) -> usize {
    // AV2 §5.18.2 caps intra frames signaled with BLOCK_256X256 superblocks to
    // BLOCK_128X128 before tile partition traversal.
    match (seq_sb_size, frame_is_intra) {
        (SuperblockSize::Block256x256, true) | (SuperblockSize::Block128x128, _) => {
            BLOCK_128X128_INDEX
        }
        (SuperblockSize::Block256x256, false) => BLOCK_256X256_INDEX,
        (SuperblockSize::Block64x64, _) => frame_size_block_index(),
    }
}

fn frame_size_block_index() -> usize {
    BLOCK_64X64_INDEX
}

fn chroma_subsampling(chroma: ChromaFormatIdc) -> (bool, bool) {
    match chroma {
        ChromaFormatIdc::Yuv420 | ChromaFormatIdc::Monochrome => (true, true),
        ChromaFormatIdc::Yuv422 => (true, false),
        ChromaFormatIdc::Yuv444 => (false, false),
    }
}

fn checked_mul_u64(name: DecodeLimitName, left: u64, right: u64) -> Result<u64, DecodeLimitError> {
    left.checked_mul(right)
        .ok_or(DecodeLimitError::ArithmeticOverflow {
            name,
            op: DecodeLimitOp::Mul,
            left,
            right,
        })
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

    // The retired pre-AVM "minimal" frozen-tier payload. It coded the luma/V
    // all_zero (skip) symbol as 0, which is inverted vs AV2 § 5.20.7.27 / AVM
    // (`decodetxb.c`), where a skipped transform block carries all_zero == 1.
    // This payload is no longer a committed conformance vector (avmdec rejects
    // it); it is kept here only to exercise the frozen partition frontier's
    // pre-allocation limit guards and to prove the AVM-honest block-symbol trace
    // now rejects the inverted-polarity skip. The committed conformance fixture
    // is the avmdec/dav2d-agreed luma-skip stream decoded by the general intra
    // path (see runtime_minimal::general_intra_tests).
    const LEGACY_INVERTED_SKIP_TRACE: &[u8] = &[
        0x44, 0x4b, 0x49, 0x46, 0x00, 0x00, 0x20, 0x00, 0x41, 0x56, 0x30, 0x32, 0x40, 0x00, 0x40,
        0x00, 0x1e, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x16, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
        0x08, 0x0c, 0x04, 0x82, 0x0a, 0x55, 0xff, 0xf1, 0xc2, 0x0d, 0xd7, 0x0b, 0x70, 0x11, 0x06,
        0x10, 0xe3, 0xfe, 0x21, 0x2f, 0xba,
    ];

    #[test]
    fn block_symbol_trace_rejects_legacy_inverted_skip() {
        // Honesty regression: the frozen block-symbol trace now asserts the AVM
        // skip polarity (luma all_zero == 1 for a skipped transform block). The
        // retired payload coded that symbol as 0, so the trace fails closed with
        // a typed mismatch on the luma txb_skip read (expected 1, decoded 0) and
        // rolls back the tile CDFs it touched. The frozen partition frontier
        // still traces this payload's partition tree, so the failure is at the
        // block-symbol stage, not the partition stage.
        with_minimal_work_unit(LEGACY_INVERTED_SKIP_TRACE, |work_unit, sequence, core| {
            let symbols = plan_minimal_runtime_partition_frontier(
                work_unit,
                sequence,
                core,
                DecodeLimits::DEFAULT,
            )
            .unwrap()
            .into_symbol_decoder();
            let before = work_unit.cdf().tile_cdfs().clone();
            let saved_before = work_unit.cdf().saved_cdfs().clone();
            let frame_before = work_unit.cdf().frame_cdfs().clone();

            let err = consume_minimal_block_symbol_trace(work_unit, symbols).unwrap_err();

            assert!(
                matches!(
                    err,
                    MinimalBlockSymbolTraceError::UnexpectedSymbol {
                        expected: 1,
                        actual: 0,
                        ..
                    }
                ),
                "expected a luma txb_skip mismatch (expected 1, decoded 0), got {err:?}"
            );
            assert_eq!(work_unit.cdf().tile_cdfs(), &before);
            assert_eq!(work_unit.cdf().saved_cdfs(), &saved_before);
            assert_eq!(work_unit.cdf().frame_cdfs(), &frame_before);
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

            let Err(err) =
                plan_minimal_runtime_partition_frontier(work_unit, sequence, &padded_core, limits)
            else {
                panic!("expected padded MI-state limit");
            };

            let MinimalRuntimePartitionFrontierError::Limit(limit) = err else {
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

            let Err(err) =
                plan_minimal_runtime_partition_frontier(work_unit, sequence, core, limits)
            else {
                panic!("expected MI-state byte limit");
            };

            let MinimalRuntimePartitionFrontierError::Limit(limit) = err else {
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

            let Err(err) =
                consume_minimal_runtime_lr_unit_frontier(work_unit, sequence, &padded_core, limits)
            else {
                panic!("expected padded MI-state limit");
            };

            let MinimalRuntimePartitionFrontierError::Limit(limit) = err else {
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

            let Err(err) =
                consume_minimal_runtime_lr_unit_frontier(work_unit, sequence, core, limits)
            else {
                panic!("expected MI-state byte limit");
            };

            let MinimalRuntimePartitionFrontierError::Limit(limit) = err else {
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
        f: impl FnOnce(&mut DecodeTileWorkUnit<'_>, &SequenceHeader, &FrameHeaderCore) -> R,
    ) -> R {
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
        let coeff = FrameCandidateCoeffFacts::new(tq.enable_fsc, tq.enable_chroma_dctonly);
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
        let mut tile_plan = plan_derived_tile_payload_boundary(input).unwrap();
        let [work_unit] = tile_plan.work_units_mut() else {
            panic!("minimal fixture must derive one tile work unit");
        };

        f(work_unit, &sequence, &core)
    }
}

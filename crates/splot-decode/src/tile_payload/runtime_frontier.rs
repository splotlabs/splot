// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Minimal runtime bridge into the partition traversal frontier.
//!
//! Feature tracking: `DECODE-MINIMAL-TIER-RUNTIME-SUCCESS`,
//! `DECODE-TILE-PARTITION-TRAVERSAL-BOUNDARY`.

use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::sequence::{ChromaFormatIdc, SequenceHeader, SuperblockSize};
use splot_core::symbol::SymbolDecoder;

use super::DecodeTileWorkUnit;
use super::partition::PartitionType;
use super::partition_allowed::PartitionFeatureFlags;
use super::partition_traversal::{
    TilePartitionBruState, TilePartitionContextState, TilePartitionFrameFacts,
    TilePartitionLoopRestorationState, TilePartitionTraversalError, TilePartitionTraversalInput,
    TilePartitionTraversalPlan, plan_tile_partition_traversal_cursor,
};
use crate::{DecodeLimitError, DecodeLimitName, DecodeLimitOp, DecodeLimits};

const BLOCK_64X64_INDEX: usize = 12;
const BLOCK_128X128_INDEX: usize = 15;
// AV2 §6 clear_left_context()/clear_above_context() seed partition neighbor
// context with BLOCK_256X256; §9.2 defines this generated table index.
const BLOCK_256X256_INDEX: usize = 18;

/// Live symbol cursor positioned at the minimal runtime block frontier.
pub(crate) struct MinimalRuntimePartitionFrontier<'payload> {
    symbols: SymbolDecoder<'payload>,
}

impl<'payload> MinimalRuntimePartitionFrontier<'payload> {
    /// Consumes the frontier and returns the live symbol decoder.
    #[must_use]
    pub(crate) fn into_symbol_decoder(self) -> SymbolDecoder<'payload> {
        self.symbols
    }
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
    /// Traversal reached a shape outside the minimal tier.
    #[error("minimal runtime partition frontier mismatch: {reason}")]
    UnexpectedFrontier {
        /// Stable mismatch reason.
        reason: &'static str,
    },
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
    let cell_count = checked_mul_u64(
        DecodeLimitName::MaxLumaSamplesPerFrame,
        mi_rows as u64,
        mi_cols as u64,
    )?;
    limits.ensure_allocation_len(DecodeLimitName::MaxLumaSamplesPerFrame, cell_count)?;

    let mi0 = initial_context_grid(mi_rows, mi_cols);
    let mi1 = initial_context_grid(mi_rows, mi_cols);
    let left0 = initial_context_line(mi_rows);
    let left1 = initial_context_line(mi_rows);
    let above0 = initial_context_line(mi_cols);
    let above1 = initial_context_line(mi_cols);
    let mi0_rows: Vec<&[usize]> = mi0.iter().map(Vec::as_slice).collect();
    let mi1_rows: Vec<&[usize]> = mi1.iter().map(Vec::as_slice).collect();
    let context = TilePartitionContextState::new(
        [&mi0_rows, &mi1_rows],
        [&left0, &left1],
        [&above0, &above1],
    );

    let (plan, symbols) = plan_tile_partition_traversal_cursor(TilePartitionTraversalInput::new(
        work_unit, frame, context, limits,
    ))?
    .into_parts();
    ensure_minimal_root_frontier(&plan, &symbols)?;

    Ok(MinimalRuntimePartitionFrontier { symbols })
}

fn minimal_partition_frame_facts(
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
        Some(lr) if !lr.uses_lr => TilePartitionLoopRestorationState::NoSyntax,
        Some(_) => TilePartitionLoopRestorationState::UnsupportedReadLrSyntax,
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

fn frame_mi_dimensions(
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

fn initial_context_grid(rows: usize, cols: usize) -> Vec<Vec<usize>> {
    vec![initial_context_line(cols); rows]
}

fn initial_context_line(len: usize) -> Vec<usize> {
    vec![BLOCK_256X256_INDEX; len]
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
    use super::*;

    #[test]
    fn initial_context_lines_use_block_256x256() {
        assert_eq!(initial_context_line(4), vec![BLOCK_256X256_INDEX; 4]);
        assert_eq!(
            initial_context_grid(2, 3),
            vec![vec![BLOCK_256X256_INDEX; 3]; 2]
        );
    }
}

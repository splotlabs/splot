// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Minimal runtime bridge into the partition traversal frontier.
//!
//! Feature tracking: `DECODE-MINIMAL-TIER-RUNTIME-SUCCESS`,
//! `DECODE-TILE-PARTITION-TRAVERSAL-BOUNDARY`,
//! `DECODE-MINIMAL-BLOCK-SYNTAX-FRONTIER`.

use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::sequence::{ChromaFormatIdc, SequenceHeader, SuperblockSize};
use splot_core::symbol::{SymbolDecoder, SymbolDecoderSummary};

use super::DecodeTileWorkUnit;
use super::block_symbol::{MinimalBlockSymbolTraceError, consume_minimal_block_symbol_trace};
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
    let symbols = plan_minimal_runtime_partition_frontier(work_unit, sequence, core, limits)?
        .into_symbol_decoder();
    let trace = consume_minimal_block_symbol_trace(work_unit, symbols)?;
    Ok(MinimalRuntimeBlockSymbolFrontier {
        summary: trace.summary(),
        reconstruction_trace: MinimalRuntimeReconstructionTrace::LumaDcNoResidual8Bit420_64x64,
    })
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
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use splot_core::bitio::BitReader;
    use splot_core::headers::frame::{
        FrameHeaderCore, FrameHeaderParseInput, FrameHeaderParseMode, FrameReferenceStateView,
        parse_frame_header_core,
    };
    use splot_core::headers::sequence::{SequenceHeader, parse_sequence_header};
    use splot_core::stream::{ParsedBitstream, parse_bitstream_partial};
    use splot_parallel::ThreadCount;

    use super::super::cdf::TileCdfSelector;
    use super::super::{
        DecodeTileWorkUnit, FrameCandidateCdfFacts, FrameCandidateTileBoundaryInput,
        FrameCandidateTileFacts, TileGroupPositionFacts, plan_derived_tile_payload_boundary,
    };
    use super::*;
    use crate::{DecodeContext, DecodeOptions, DecodeRuntimeConfig};

    const MINIMAL_FIXTURE: &[u8] = include_bytes!(
        "../../../../tests/conformance/vectors/valid/syn-flat-intra-64x64-minimal.ivf"
    );
    const MINIMAL_TRACE_SYMBOLS: u64 = 6;
    const MINIMAL_TRACE_TRAILING_BIT_POSITION: u64 = 14;
    const MINIMAL_TRACE_PADDING_END_POSITION: u64 = 16;

    #[test]
    fn initial_context_lines_use_block_256x256() {
        assert_eq!(initial_context_line(4), vec![BLOCK_256X256_INDEX; 4]);
        assert_eq!(
            initial_context_grid(2, 3),
            vec![vec![BLOCK_256X256_INDEX; 3]; 2]
        );
    }

    #[test]
    fn block_symbol_frontier_accepts_minimal_fixture_trace() {
        with_minimal_work_unit(MINIMAL_FIXTURE, |work_unit, sequence, core| {
            let selected = block_trace_selectors()
                .map(|selector| work_unit.cdf().tile_cdfs().row(selector).unwrap().to_vec());
            let untouched = TileCdfSelector::DoExtPartition {
                plane_start: 0,
                ctx: 4,
            };
            let untouched_before = work_unit.cdf().tile_cdfs().row(untouched).unwrap().to_vec();

            let frontier = plan_minimal_runtime_block_symbol_frontier(
                work_unit,
                sequence,
                core,
                DecodeLimits::DEFAULT,
            )
            .unwrap();
            let summary = frontier.summary();

            assert_eq!(summary.symbol_count, MINIMAL_TRACE_SYMBOLS);
            assert_eq!(
                frontier.reconstruction_trace(),
                MinimalRuntimeReconstructionTrace::LumaDcNoResidual8Bit420_64x64
            );
            assert_eq!(
                summary.trailing_bit_position.get(),
                MINIMAL_TRACE_TRAILING_BIT_POSITION
            );
            assert_eq!(
                summary.padding_end_position.get(),
                MINIMAL_TRACE_PADDING_END_POSITION
            );
            for (selector, before) in block_trace_selectors().into_iter().zip(selected) {
                assert_ne!(
                    work_unit.cdf().tile_cdfs().row(selector).unwrap(),
                    before.as_slice()
                );
            }
            assert_eq!(
                work_unit.cdf().tile_cdfs().row(untouched).unwrap(),
                untouched_before.as_slice()
            );
        });
    }

    #[test]
    fn block_symbol_frontier_rejects_exit_symbol_padding_failure() {
        let mut bytes = MINIMAL_FIXTURE.to_vec();
        *bytes.last_mut().unwrap() ^= 0x01;

        with_minimal_work_unit(&bytes, |work_unit, sequence, core| {
            let symbols = plan_minimal_runtime_partition_frontier(
                work_unit,
                sequence,
                core,
                DecodeLimits::DEFAULT,
            )
            .unwrap()
            .into_symbol_decoder();
            let before = work_unit.cdf().tile_cdfs().clone();
            let err = consume_minimal_block_symbol_trace(work_unit, symbols).unwrap_err();

            assert!(matches!(
                err,
                MinimalBlockSymbolTraceError::ExitSymbol { .. }
            ));
            assert_eq!(work_unit.cdf().tile_cdfs(), &before);
        });
    }

    fn block_trace_selectors() -> [TileCdfSelector; 5] {
        [
            TileCdfSelector::YModeSet,
            TileCdfSelector::YModeIndex { ctx: 0 },
            TileCdfSelector::TxbSkip {
                coeff_cdf_q_ctx: 2,
                plane_type: 0,
                tx_size: 0,
                ctx: 0,
            },
            TileCdfSelector::UvModeCflNotAllowed { ctx: 0 },
            TileCdfSelector::VTxbSkip {
                coeff_cdf_q_ctx: 1,
                ctx: 3,
            },
        ]
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

        let facts = FrameCandidateTileFacts::from_frame_core(&core).unwrap();
        let tq = sequence.transform_quant_entropy.as_ref().unwrap();
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

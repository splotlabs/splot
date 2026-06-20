// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 5.20.3.1 partition traversal frontier.
//!
//! Feature tracking: `DECODE-TILE-PARTITION-TRAVERSAL-BOUNDARY`.

use splot_core::symbol::{SymbolDecoder, SymbolDecoderCheckpoint, SymbolDecoderConfig};

use super::DecodeTileWorkUnit;
use super::cdf::TileCdfError;
use super::cdf::context::{PartitionContextInput, SquareSplitContextInput};
use super::mi_size_state::{TileMiSizeState, TileMiSizeStateError};
use super::partition::{PartitionDecisionError, PartitionType, ReadPartitionDecision};
use super::partition_allowed::{
    PartitionAllowedError, PartitionAllowedInput, PartitionFeatureFlags, PartitionTreeType,
    partition_decision_facts,
};
use super::partition_size::{
    BlockSize, PartitionSizeError, PartitionSubsize, h_partition_midsize, partition_subsize,
};
use crate::{DecodeLimitError, DecodeLimitName, DecodeLimits};

const BLOCK_8X32: usize = 21;
const BLOCK_32X8: usize = 22;
const BLOCK_64X64: usize = 12;

/// Decoder support matrix row for this traversal frontier.
pub(crate) const TILE_PARTITION_TRAVERSAL_MATRIX_ROW: &str = "tile-partition-traversal-boundary";
/// Implementation-matrix feature id for this traversal frontier.
pub(crate) const TILE_PARTITION_TRAVERSAL_FEATURE_ID: &str =
    "DECODE-TILE-PARTITION-TRAVERSAL-BOUNDARY";

/// AV2 partition-context state read by the traversal frontier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TilePartitionContextState<'a> {
    mi_sizes: [&'a [&'a [usize]]; 2],
    left_mi_sizes: [&'a [usize]; 2],
    above_mi_sizes: [&'a [usize]; 2],
}

impl<'a> TilePartitionContextState<'a> {
    /// Creates a read-only context-state view for § 8.3.2 partition selectors.
    #[must_use]
    pub(crate) const fn new(
        mi_sizes: [&'a [&'a [usize]]; 2],
        left_mi_sizes: [&'a [usize]; 2],
        above_mi_sizes: [&'a [usize]; 2],
    ) -> Self {
        Self {
            mi_sizes,
            left_mi_sizes,
            above_mi_sizes,
        }
    }
}

/// BRU state supported by the traversal frontier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TilePartitionBruState {
    /// Normal non-BRU path: `bru_mode == BRU_ACTIVE`.
    Active,
    /// BRU, bridge, or inactive paths remain outside this frontier.
    Unsupported,
}

/// Loop-restoration syntax state supported by the traversal frontier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TilePartitionLoopRestorationState {
    /// No §5.20.10.4 `read_lr()` syntax is needed before partition reads.
    NoSyntax,
    /// Root `read_lr()` syntax remains outside this frontier.
    UnsupportedReadLrSyntax,
}

/// Frame and sequence facts required by the traversal frontier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TilePartitionFrameFacts {
    mi_rows: usize,
    mi_cols: usize,
    sb_size: BlockSize,
    num_planes: usize,
    subsampling_x: bool,
    subsampling_y: bool,
    frame_is_intra: bool,
    enable_sdp: bool,
    enable_extended_sdp: bool,
    loop_restoration: TilePartitionLoopRestorationState,
    features: PartitionFeatureFlags,
    max_pb_aspect_ratio: usize,
    has_chroma: bool,
    bru_state: TilePartitionBruState,
}

impl TilePartitionFrameFacts {
    /// Creates checked frame facts for the traversal frontier.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        mi_rows: usize,
        mi_cols: usize,
        sb_size: usize,
        num_planes: usize,
        subsampling_x: bool,
        subsampling_y: bool,
        frame_is_intra: bool,
        enable_sdp: bool,
        enable_extended_sdp: bool,
        loop_restoration: TilePartitionLoopRestorationState,
        features: PartitionFeatureFlags,
        max_pb_aspect_ratio: usize,
        has_chroma: bool,
        bru_state: TilePartitionBruState,
    ) -> Result<Self, TilePartitionTraversalError> {
        Ok(Self {
            mi_rows,
            mi_cols,
            sb_size: BlockSize::new(sb_size)?,
            num_planes,
            subsampling_x,
            subsampling_y,
            frame_is_intra,
            enable_sdp,
            enable_extended_sdp,
            loop_restoration,
            features,
            max_pb_aspect_ratio,
            has_chroma,
            bru_state,
        })
    }

    /// Superblock size used by this frame's tile partition traversal.
    #[must_use]
    pub(crate) const fn sb_size(&self) -> BlockSize {
        self.sb_size
    }
}

/// Input to the crate-private traversal frontier.
#[derive(Debug)]
pub(crate) struct TilePartitionTraversalInput<'work, 'payload, 'ctx> {
    work_unit: &'work mut DecodeTileWorkUnit<'payload>,
    frame: TilePartitionFrameFacts,
    context: TilePartitionContextState<'ctx>,
    limits: DecodeLimits,
}

impl<'work, 'payload, 'ctx> TilePartitionTraversalInput<'work, 'payload, 'ctx> {
    /// Creates traversal-frontier input from explicit caller facts.
    #[must_use]
    pub(crate) const fn new(
        work_unit: &'work mut DecodeTileWorkUnit<'payload>,
        frame: TilePartitionFrameFacts,
        context: TilePartitionContextState<'ctx>,
        limits: DecodeLimits,
    ) -> Self {
        Self {
            work_unit,
            frame,
            context,
            limits,
        }
    }
}

/// One `decode_partition()` call on the frontier path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TilePartitionCall {
    pub(crate) r: usize,
    pub(crate) c: usize,
    pub(crate) b_size: BlockSize,
    pub(crate) parent_size: Option<BlockSize>,
    pub(crate) chroma_offset: bool,
    pub(crate) has_chroma: bool,
}

impl TilePartitionCall {
    const fn root(r: usize, c: usize, b_size: BlockSize, has_chroma: bool) -> Self {
        Self {
            r,
            c,
            b_size,
            parent_size: None,
            chroma_offset: false,
            has_chroma,
        }
    }
}

/// AV2 tile-local MI bounds used by § 5.20.9.1 `is_inside`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TilePartitionBounds {
    mi_row_start: usize,
    mi_row_end: usize,
    mi_col_start: usize,
    mi_col_end: usize,
}

impl TilePartitionBounds {
    fn from_work_unit(work_unit: &DecodeTileWorkUnit<'_>) -> Self {
        let row_range = work_unit.mi_row_range();
        let col_range = work_unit.mi_col_range();
        Self {
            mi_row_start: row_range.start as usize,
            mi_row_end: row_range.end as usize,
            mi_col_start: col_range.start as usize,
            mi_col_end: col_range.end as usize,
        }
    }

    const fn is_inside(self, r: usize, c: usize) -> bool {
        self.mi_col_start <= c
            && c < self.mi_col_end
            && self.mi_row_start <= r
            && r < self.mi_row_end
    }

    fn avail_u(self, call: TilePartitionCall) -> bool {
        match call.r.checked_sub(1) {
            Some(candidate_r) => self.is_inside(candidate_r, call.c),
            None => false,
        }
    }

    fn avail_l(self, call: TilePartitionCall) -> bool {
        match call.c.checked_sub(1) {
            Some(candidate_c) => self.is_inside(call.r, candidate_c),
            None => false,
        }
    }
}

/// One consumed partition decision on the frontier path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TilePartitionFrontierStep {
    pub(crate) call: TilePartitionCall,
    pub(crate) decision: ReadPartitionDecision,
    pub(crate) symbol_count_before: u64,
    pub(crate) symbol_count_after: u64,
}

/// The first § 5.20.4.1 `decode_block()` boundary reached by traversal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DecodeBlockFrontier {
    pub(crate) r: usize,
    pub(crate) c: usize,
    pub(crate) b_size: BlockSize,
    pub(crate) has_chroma: bool,
    pub(crate) chroma_offset: bool,
    pub(crate) symbol_count_before_block: u64,
    pub(crate) symbol_checkpoint_before_block: SymbolDecoderCheckpoint,
}

/// Successful partition frontier plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TilePartitionTraversalPlan {
    pub(crate) tile_num: u32,
    steps: Vec<TilePartitionFrontierStep>,
    skipped_out_of_frame: Vec<TilePartitionCall>,
    pending_children: Vec<TilePartitionCall>,
    frontier: DecodeBlockFrontier,
    pub(crate) consumed_bits_before: u64,
    pub(crate) consumed_bits_after: u64,
    symbol_count_after: u64,
}

impl TilePartitionTraversalPlan {
    /// Ordered partition decisions consumed before the block frontier.
    #[must_use]
    pub(crate) fn steps(&self) -> &[TilePartitionFrontierStep] {
        &self.steps
    }

    /// Pending sibling child calls that cannot be processed before block syntax.
    #[must_use]
    pub(crate) fn pending_children(&self) -> &[TilePartitionCall] {
        &self.pending_children
    }

    /// First block frontier.
    #[must_use]
    pub(crate) const fn frontier(&self) -> DecodeBlockFrontier {
        self.frontier
    }

    /// Symbol count after planning the frontier.
    #[must_use]
    pub(crate) const fn symbol_count_after(&self) -> u64 {
        self.symbol_count_after
    }
}

/// Planned frontier plus the live symbol cursor positioned before block syntax.
pub(crate) struct TilePartitionTraversalCursor<'payload> {
    plan: TilePartitionTraversalPlan,
    symbols: SymbolDecoder<'payload>,
}

impl<'payload> TilePartitionTraversalCursor<'payload> {
    /// Splits the cursor into the deterministic plan and live symbol decoder.
    #[must_use]
    pub(crate) fn into_parts(self) -> (TilePartitionTraversalPlan, SymbolDecoder<'payload>) {
        (self.plan, self.symbols)
    }
}

/// Error returned by the traversal frontier.
#[derive(Debug, thiserror::Error)]
pub(crate) enum TilePartitionTraversalError {
    /// Resource limit rejected traversal.
    #[error("partition traversal rejected by resource limit: {0}")]
    Limit(#[from] DecodeLimitError),
    /// A partition-size lookup failed.
    #[error("partition traversal size lookup failed: {0}")]
    Size(#[from] PartitionSizeError),
    /// Allowed-partition derivation failed.
    #[error("partition traversal allowed-set derivation failed: {0}")]
    Allowed(#[from] PartitionAllowedError),
    /// Partition decision failed.
    #[error("partition traversal decision failed: {0}")]
    Decision(#[from] PartitionDecisionError),
    /// Symbol decoder initialization failed.
    #[error("partition traversal symbol initialization failed: {0}")]
    Symbol(#[from] splot_core::Error),
    /// CDF context access failed.
    #[error("partition traversal CDF context failed: {0}")]
    Cdf(#[from] TileCdfError),
    /// Unsupported traversal path.
    #[error("partition traversal unsupported path: {0:?}")]
    Unsupported(TilePartitionTraversalUnsupported),
    /// A coordinate addition overflowed.
    #[error("{coordinate} coordinate overflow: {base} + {offset}")]
    CoordinateOverflow {
        /// Coordinate name.
        coordinate: &'static str,
        /// Base coordinate.
        base: usize,
        /// Derived offset.
        offset: usize,
    },
    /// A coordinate multiplication overflowed.
    #[error("{coordinate} coordinate offset overflow: {left} * {right}")]
    CoordinateOffsetOverflow {
        /// Coordinate name.
        coordinate: &'static str,
        /// Left operand.
        left: usize,
        /// Right operand.
        right: usize,
    },
    /// A selected partition had no valid child size.
    #[error("partition traversal selected invalid child size for {partition:?} at bSize {b_size}")]
    InvalidPartitionSubsize {
        /// Selected partition.
        partition: PartitionType,
        /// Source block size.
        b_size: usize,
    },
    /// Internal child-call arity invariant failed.
    #[error("partition traversal produced more than four child calls")]
    TooManyChildCalls,
    /// Traversal found no in-frame block frontier.
    #[error("partition traversal reached no in-frame decode_block frontier")]
    NoBlockFrontier,
}

/// Unsupported traversal-frontier path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TilePartitionTraversalUnsupported {
    /// Non-intra paths remain outside this frontier.
    NonIntra,
    /// SDP luma/chroma partition side effects remain outside this frontier.
    Sdp,
    /// Extended SDP region signaling remains outside this frontier.
    ExtendedSdp,
    /// Root `read_lr()` syntax remains outside this frontier.
    ReadLoopRestoration,
    /// BRU/bridge/inactive behavior remains outside this frontier.
    BruOrBridge,
}

/// Plans the AV2 § 5.20.3.1 partition frontier to the first block boundary.
pub(crate) fn plan_tile_partition_traversal_frontier(
    input: TilePartitionTraversalInput<'_, '_, '_>,
) -> Result<TilePartitionTraversalPlan, TilePartitionTraversalError> {
    Ok(plan_tile_partition_traversal_cursor(input)?.plan)
}

/// Plans the partition frontier and returns the live symbol cursor at it.
pub(crate) fn plan_tile_partition_traversal_cursor<'payload>(
    input: TilePartitionTraversalInput<'_, 'payload, '_>,
) -> Result<TilePartitionTraversalCursor<'payload>, TilePartitionTraversalError> {
    let TilePartitionTraversalInput {
        work_unit,
        frame,
        context,
        limits,
    } = input;
    let extended_sdp_allowed = frame.enable_extended_sdp && !frame.frame_is_intra;
    if extended_sdp_allowed {
        return Err(TilePartitionTraversalError::Unsupported(
            TilePartitionTraversalUnsupported::ExtendedSdp,
        ));
    }
    if !frame.frame_is_intra {
        return Err(TilePartitionTraversalError::Unsupported(
            TilePartitionTraversalUnsupported::NonIntra,
        ));
    }
    // AV2 §5.20.3.1 invokes §5.20.10.4 `read_lr()` at the root before
    // `read_partition`; keep that syntax explicit until this frontier models it.
    if frame.loop_restoration == TilePartitionLoopRestorationState::UnsupportedReadLrSyntax {
        return Err(TilePartitionTraversalError::Unsupported(
            TilePartitionTraversalUnsupported::ReadLoopRestoration,
        ));
    }
    if frame.bru_state != TilePartitionBruState::Active {
        return Err(TilePartitionTraversalError::Unsupported(
            TilePartitionTraversalUnsupported::BruOrBridge,
        ));
    }

    let mut cdfs = work_unit.cdf().tile_cdfs().clone();
    let config = SymbolDecoderConfig::new().with_cdf_update_mode(work_unit.cdf().update_mode());
    let mut symbols = SymbolDecoder::with_base_and_config(
        work_unit.tile_bytes(),
        work_unit.tile_byte_span().start,
        config,
    )?;
    let consumed_bits_before = symbols.consumed_bits().get();
    let tile_bounds = TilePartitionBounds::from_work_unit(work_unit);
    let root = TilePartitionCall::root(
        work_unit.mi_row_range().start as usize,
        work_unit.mi_col_range().start as usize,
        frame.sb_size,
        frame.has_chroma,
    );
    let mut stack = vec![root];
    let mut steps = Vec::new();
    let mut skipped_out_of_frame = Vec::new();

    while let Some(call) = stack.pop() {
        limits.ensure(
            DecodeLimitName::MaxTilePartitionSteps,
            (steps.len() + 1) as u64,
        )?;
        if call.r >= frame.mi_rows || call.c >= frame.mi_cols {
            skipped_out_of_frame.push(call);
            continue;
        }
        ensure_supported_call(frame, call)?;

        let symbol_count_before = symbols.symbol_count();
        let decision = read_frontier_partition_decision(
            call,
            frame,
            tile_bounds,
            context,
            &mut cdfs,
            &mut symbols,
        )?;
        let symbol_count_after = symbols.symbol_count();
        let partition = decision.partition;
        if is_minimal_sdp_root(frame, call) && partition != PartitionType::None {
            return Err(TilePartitionTraversalError::Unsupported(
                TilePartitionTraversalUnsupported::Sdp,
            ));
        }
        steps.push(TilePartitionFrontierStep {
            call,
            decision,
            symbol_count_before,
            symbol_count_after,
        });

        let sub_size = valid_subsize(partition, call.b_size)?;
        let chroma_offset = updated_chroma_offset(call, partition, sub_size, frame)?;
        if partition == PartitionType::None {
            stack.reverse();
            let tree_type = partition_tree_type(frame, call);
            let plan = TilePartitionTraversalPlan {
                tile_num: work_unit.tile_num(),
                steps,
                skipped_out_of_frame,
                pending_children: stack,
                frontier: DecodeBlockFrontier {
                    r: call.r,
                    c: call.c,
                    b_size: sub_size,
                    // AV2 §5.20.3.1 decode_block() excludes LUMA_PART from HasChroma.
                    has_chroma: call.has_chroma
                        && frame.num_planes > 1
                        && tree_type != PartitionTreeType::LumaPart,
                    chroma_offset,
                    symbol_count_before_block: symbols.symbol_count(),
                    symbol_checkpoint_before_block: symbols.checkpoint(),
                },
                consumed_bits_before,
                consumed_bits_after: symbols.consumed_bits().get(),
                symbol_count_after: symbols.symbol_count(),
            };
            *work_unit.cdf_mut().tile_cdfs_mut() = cdfs;
            return Ok(TilePartitionTraversalCursor { plan, symbols });
        }

        let children = child_calls(call, partition, sub_size, frame, chroma_offset)?;
        for child in children.as_slice().iter().rev().copied() {
            stack.push(child);
        }
    }

    Err(TilePartitionTraversalError::NoBlockFrontier)
}

/// Error from the general intra full partition-tree walk, distinguishing
/// traversal/MI-state failures from a caller leaf-decode failure `E`.
#[derive(Debug, thiserror::Error)]
pub(crate) enum GeneralIntraTreeWalkError<E> {
    /// A partition traversal step failed.
    #[error("partition tree walk traversal failed: {0}")]
    Traversal(#[from] TilePartitionTraversalError),
    /// An MI-size state update failed.
    #[error("partition tree walk MI-size update failed: {0}")]
    MiSize(TileMiSizeStateError),
    /// The caller's per-leaf-block decode failed.
    #[error("partition tree walk leaf-block decode failed")]
    Leaf(E),
}

/// Drives the complete AV2 § 5.20.3.1 partition tree for a general intra tile,
/// invoking `on_leaf` at each `PARTITION_NONE` leaf block in decode (DFS) order.
///
/// Unlike [`plan_tile_partition_traversal_cursor`], which stops at the first
/// leaf, this walks the whole tree: partition-split symbols and per-block syntax
/// are read interleaved on one live symbol decoder and the tile CDFs, exactly as
/// the spec orders them. The MI-size partition context is maintained across
/// blocks via `mi_size_state` (read for each partition decision, updated after
/// each leaf). The live symbol decoder is returned for the caller's § 8.2.4
/// `exit_symbol()` check.
pub(crate) fn decode_general_intra_partition_tree<'payload, E, F>(
    work_unit: &mut DecodeTileWorkUnit<'payload>,
    frame: TilePartitionFrameFacts,
    mi_size_state: &mut TileMiSizeState,
    limits: DecodeLimits,
    mut on_leaf: F,
) -> Result<SymbolDecoder<'payload>, GeneralIntraTreeWalkError<E>>
where
    F: FnMut(
        &mut DecodeTileWorkUnit<'payload>,
        &mut SymbolDecoder<'payload>,
        &DecodeBlockFrontier,
    ) -> Result<(), E>,
{
    if !frame.frame_is_intra {
        return Err(TilePartitionTraversalError::Unsupported(
            TilePartitionTraversalUnsupported::NonIntra,
        )
        .into());
    }
    if frame.loop_restoration == TilePartitionLoopRestorationState::UnsupportedReadLrSyntax {
        return Err(TilePartitionTraversalError::Unsupported(
            TilePartitionTraversalUnsupported::ReadLoopRestoration,
        )
        .into());
    }
    if frame.bru_state != TilePartitionBruState::Active {
        return Err(TilePartitionTraversalError::Unsupported(
            TilePartitionTraversalUnsupported::BruOrBridge,
        )
        .into());
    }

    let config = SymbolDecoderConfig::new().with_cdf_update_mode(work_unit.cdf().update_mode());
    let mut symbols = SymbolDecoder::with_base_and_config(
        work_unit.tile_bytes(),
        work_unit.tile_byte_span().start,
        config,
    )
    .map_err(TilePartitionTraversalError::from)?;
    let tile_bounds = TilePartitionBounds::from_work_unit(work_unit);
    // AV2 § 5.20.2.1 decode_tile(): iterate the tile's MI range as a raster grid
    // of superblocks, `sbSize4 = Num_4x4_Blocks_Wide[SbSize]` MI units apart.
    // Each superblock is one `decode_partition(r, c, SbSize, ...)` root; the
    // shared symbol decoder, tile CDFs, and MI-size context carry across them so
    // later superblocks read the already-decoded left/above neighbours.
    // `frame.sb_size` is a validated BlockSize, so `num_4x4_wide()` is >= 1; the
    // `max(1)` is a belt-and-braces guard that keeps the loop progressing.
    let sb_size4 = frame
        .sb_size
        .num_4x4_wide()
        .map_err(TilePartitionTraversalError::from)?
        .max(1);
    let mi_row_start = work_unit.mi_row_range().start as usize;
    let mi_row_end = (work_unit.mi_row_range().end as usize).min(frame.mi_rows);
    let mi_col_start = work_unit.mi_col_range().start as usize;
    let mi_col_end = (work_unit.mi_col_range().end as usize).min(frame.mi_cols);
    let mut step_count: u64 = 0;

    let mut sb_row = mi_row_start;
    while sb_row < mi_row_end {
        // § 5.20.2.1 clear_left_context() runs at the start of every superblock
        // row; the above context persists across rows.
        mi_size_state.clear_left_context();
        let mut sb_col = mi_col_start;
        while sb_col < mi_col_end {
            // One superblock-rooted § 5.20.3.1 partition DFS.
            let root = TilePartitionCall::root(sb_row, sb_col, frame.sb_size, frame.has_chroma);
            let mut stack = vec![root];
            while let Some(call) = stack.pop() {
                step_count += 1;
                limits
                    .ensure(DecodeLimitName::MaxTilePartitionSteps, step_count)
                    .map_err(TilePartitionTraversalError::from)?;
                if call.r >= frame.mi_rows || call.c >= frame.mi_cols {
                    continue;
                }
                ensure_supported_call(frame, call)?;

                let decision = mi_size_state
                    .with_context_state(|context| {
                        read_frontier_partition_decision(
                            call,
                            frame,
                            tile_bounds,
                            context,
                            work_unit.cdf_mut().tile_cdfs_mut(),
                            &mut symbols,
                        )
                    })
                    .map_err(GeneralIntraTreeWalkError::MiSize)??;
                let partition = decision.partition;
                if is_minimal_sdp_root(frame, call) && partition != PartitionType::None {
                    return Err(TilePartitionTraversalError::Unsupported(
                        TilePartitionTraversalUnsupported::Sdp,
                    )
                    .into());
                }

                let sub_size = valid_subsize(partition, call.b_size)?;
                let chroma_offset = updated_chroma_offset(call, partition, sub_size, frame)?;
                if partition == PartitionType::None {
                    let tree_type = partition_tree_type(frame, call);
                    let frontier = DecodeBlockFrontier {
                        r: call.r,
                        c: call.c,
                        b_size: sub_size,
                        has_chroma: call.has_chroma
                            && frame.num_planes > 1
                            && tree_type != PartitionTreeType::LumaPart,
                        chroma_offset,
                        symbol_count_before_block: symbols.symbol_count(),
                        symbol_checkpoint_before_block: symbols.checkpoint(),
                    };
                    on_leaf(work_unit, &mut symbols, &frontier)
                        .map_err(GeneralIntraTreeWalkError::Leaf)?;
                    mi_size_state
                        .update_luma_block(call.r, call.c, sub_size)
                        .map_err(GeneralIntraTreeWalkError::MiSize)?;
                } else {
                    let children = child_calls(call, partition, sub_size, frame, chroma_offset)?;
                    for child in children.as_slice().iter().rev().copied() {
                        stack.push(child);
                    }
                }
            }
            sb_col += sb_size4;
        }
        sb_row += sb_size4;
    }

    Ok(symbols)
}

fn ensure_supported_call(
    frame: TilePartitionFrameFacts,
    call: TilePartitionCall,
) -> Result<(), TilePartitionTraversalError> {
    if frame.enable_sdp && call.b_size.index() == BLOCK_64X64 && !is_minimal_sdp_root(frame, call) {
        return Err(TilePartitionTraversalError::Unsupported(
            TilePartitionTraversalUnsupported::Sdp,
        ));
    }
    Ok(())
}

fn read_frontier_partition_decision(
    call: TilePartitionCall,
    frame: TilePartitionFrameFacts,
    tile_bounds: TilePartitionBounds,
    context: TilePartitionContextState<'_>,
    cdfs: &mut super::cdf::TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
) -> Result<ReadPartitionDecision, TilePartitionTraversalError> {
    let allowed = PartitionAllowedInput::new(
        call.r,
        call.c,
        frame.mi_rows,
        frame.mi_cols,
        call.b_size.index(),
        partition_tree_type(frame, call),
        frame.subsampling_x,
        frame.subsampling_y,
        frame.features,
        frame.frame_is_intra,
        false,
        frame.max_pb_aspect_ratio,
        call.has_chroma,
        call.chroma_offset,
        frame.num_planes,
        None,
    )?;
    let facts = partition_decision_facts(allowed)?;
    let partition_context = PartitionContextInput::new(
        call.b_size.index(),
        0,
        call.r,
        call.c,
        context.left_mi_sizes,
        context.above_mi_sizes,
    )?;
    let avail_u = tile_bounds.avail_u(call);
    let avail_l = tile_bounds.avail_l(call);
    let square_context = SquareSplitContextInput::new(
        call.b_size.index(),
        0,
        call.r,
        call.c,
        avail_u,
        avail_l,
        context.mi_sizes,
    )?;
    let decision_input =
        facts.read_partition_decision_input(true, partition_context, square_context);
    Ok(super::partition::read_partition_decision(
        decision_input,
        cdfs,
        symbols,
    )?)
}

fn partition_tree_type(
    frame: TilePartitionFrameFacts,
    call: TilePartitionCall,
) -> PartitionTreeType {
    if is_minimal_sdp_root(frame, call) {
        PartitionTreeType::LumaPart
    } else {
        PartitionTreeType::Shared
    }
}

fn is_minimal_sdp_root(frame: TilePartitionFrameFacts, call: TilePartitionCall) -> bool {
    frame.enable_sdp
        && frame.frame_is_intra
        && call.parent_size.is_none()
        && call.b_size.index() == BLOCK_64X64
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TilePartitionChildCalls {
    calls: [TilePartitionCall; 4],
    len: usize,
}

impl TilePartitionChildCalls {
    fn new(fill: TilePartitionCall) -> Self {
        Self {
            calls: [fill; 4],
            len: 0,
        }
    }

    fn push(&mut self, call: TilePartitionCall) -> Result<(), TilePartitionTraversalError> {
        let slot = self
            .calls
            .get_mut(self.len)
            .ok_or(TilePartitionTraversalError::TooManyChildCalls)?;
        *slot = call;
        self.len += 1;
        Ok(())
    }

    fn as_slice(&self) -> &[TilePartitionCall] {
        &self.calls[..self.len]
    }
}

fn child_calls(
    call: TilePartitionCall,
    partition: PartitionType,
    sub_size: BlockSize,
    frame: TilePartitionFrameFacts,
    chroma_offset: bool,
) -> Result<TilePartitionChildCalls, TilePartitionTraversalError> {
    let num4x4wide = call.b_size.num_4x4_wide()?;
    let num4x4high = call.b_size.num_4x4_high()?;
    let half_w = num4x4wide >> 1;
    let half_h = num4x4high >> 1;
    let parent = Some(call.b_size);
    let mut children = TilePartitionChildCalls::new(call);
    match partition {
        PartitionType::None => {}
        PartitionType::Horz => {
            children.push(child(
                call.r,
                call.c,
                sub_size,
                parent,
                chroma_offset,
                call.has_chroma && !chroma_offset,
            ))?;
            children.push(child(
                checked_add("r", call.r, half_h)?,
                call.c,
                sub_size,
                parent,
                chroma_offset,
                call.has_chroma,
            ))?;
        }
        PartitionType::Vert => {
            children.push(child(
                call.r,
                call.c,
                sub_size,
                parent,
                chroma_offset,
                call.has_chroma && !chroma_offset,
            ))?;
            children.push(child(
                call.r,
                checked_add("c", call.c, half_w)?,
                sub_size,
                parent,
                chroma_offset,
                call.has_chroma,
            ))?;
        }
        PartitionType::Split => {
            children.push(child(
                call.r,
                call.c,
                sub_size,
                parent,
                false,
                call.has_chroma,
            ))?;
            children.push(child(
                call.r,
                checked_add("c", call.c, half_w)?,
                sub_size,
                parent,
                false,
                call.has_chroma,
            ))?;
            children.push(child(
                checked_add("r", call.r, half_h)?,
                call.c,
                sub_size,
                parent,
                false,
                call.has_chroma,
            ))?;
            children.push(child(
                checked_add("r", call.r, half_h)?,
                checked_add("c", call.c, half_w)?,
                sub_size,
                parent,
                false,
                call.has_chroma,
            ))?;
        }
        PartitionType::Horz3 => {
            let middle = h_partition_midsize(call.b_size)?.valid().ok_or(
                TilePartitionTraversalError::InvalidPartitionSubsize {
                    partition,
                    b_size: call.b_size.index(),
                },
            )?;
            let middle_chroma =
                call.b_size.index() == BLOCK_8X32 && call.has_chroma && frame.subsampling_x;
            children.push(child(
                call.r,
                call.c,
                sub_size,
                parent,
                chroma_offset,
                call.has_chroma && !chroma_offset,
            ))?;
            children.push(child(
                checked_add("r", call.r, half_h >> 1)?,
                call.c,
                middle,
                parent,
                chroma_offset || middle_chroma,
                call.has_chroma && !chroma_offset && !middle_chroma,
            ))?;
            children.push(child(
                checked_add("r", call.r, half_h >> 1)?,
                checked_add("c", call.c, half_w)?,
                middle,
                parent,
                chroma_offset || middle_chroma,
                call.has_chroma && !chroma_offset,
            ))?;
            children.push(child(
                checked_scaled_add("r", call.r, 3, half_h >> 1)?,
                call.c,
                sub_size,
                parent,
                chroma_offset,
                call.has_chroma,
            ))?;
        }
        PartitionType::Vert3 => {
            let middle = h_partition_midsize(call.b_size)?.valid().ok_or(
                TilePartitionTraversalError::InvalidPartitionSubsize {
                    partition,
                    b_size: call.b_size.index(),
                },
            )?;
            let middle_chroma =
                call.b_size.index() == BLOCK_32X8 && call.has_chroma && frame.subsampling_y;
            children.push(child(
                call.r,
                call.c,
                sub_size,
                parent,
                chroma_offset,
                call.has_chroma && !chroma_offset,
            ))?;
            children.push(child(
                call.r,
                checked_add("c", call.c, half_w >> 1)?,
                middle,
                parent,
                chroma_offset || middle_chroma,
                call.has_chroma && !chroma_offset && !middle_chroma,
            ))?;
            children.push(child(
                checked_add("r", call.r, half_h)?,
                checked_add("c", call.c, half_w >> 1)?,
                middle,
                parent,
                chroma_offset || middle_chroma,
                call.has_chroma && !chroma_offset,
            ))?;
            children.push(child(
                call.r,
                checked_scaled_add("c", call.c, 3, half_w >> 1)?,
                sub_size,
                parent,
                chroma_offset,
                call.has_chroma,
            ))?;
        }
        PartitionType::Horz4A | PartitionType::Horz4B => {
            let b_size_big = valid_subsize(PartitionType::Horz, call.b_size)?;
            let b_size_med = valid_subsize(PartitionType::Horz, b_size_big)?;
            let third = if partition == PartitionType::Horz4A {
                b_size_big
            } else {
                b_size_med
            };
            let second = if partition == PartitionType::Horz4A {
                b_size_med
            } else {
                b_size_big
            };
            children.push(child(
                call.r,
                call.c,
                sub_size,
                parent,
                chroma_offset,
                call.has_chroma && !chroma_offset,
            ))?;
            children.push(child(
                checked_add("r", call.r, num4x4high >> 3)?,
                call.c,
                second,
                parent,
                chroma_offset,
                call.has_chroma && !chroma_offset,
            ))?;
            children.push(child(
                checked_scaled_add(
                    "r",
                    call.r,
                    if partition == PartitionType::Horz4A {
                        3
                    } else {
                        5
                    },
                    num4x4high >> 3,
                )?,
                call.c,
                third,
                parent,
                chroma_offset,
                call.has_chroma && !chroma_offset,
            ))?;
            children.push(child(
                checked_scaled_add("r", call.r, 7, num4x4high >> 3)?,
                call.c,
                sub_size,
                parent,
                chroma_offset,
                call.has_chroma,
            ))?;
        }
        PartitionType::Vert4A | PartitionType::Vert4B => {
            let b_size_big = valid_subsize(PartitionType::Vert, call.b_size)?;
            let b_size_med = valid_subsize(PartitionType::Vert, b_size_big)?;
            let third = if partition == PartitionType::Vert4A {
                b_size_big
            } else {
                b_size_med
            };
            let second = if partition == PartitionType::Vert4A {
                b_size_med
            } else {
                b_size_big
            };
            children.push(child(
                call.r,
                call.c,
                sub_size,
                parent,
                chroma_offset,
                call.has_chroma && !chroma_offset,
            ))?;
            children.push(child(
                call.r,
                checked_add("c", call.c, num4x4wide >> 3)?,
                second,
                parent,
                chroma_offset,
                call.has_chroma && !chroma_offset,
            ))?;
            children.push(child(
                call.r,
                checked_scaled_add(
                    "c",
                    call.c,
                    if partition == PartitionType::Vert4A {
                        3
                    } else {
                        5
                    },
                    num4x4wide >> 3,
                )?,
                third,
                parent,
                chroma_offset,
                call.has_chroma && !chroma_offset,
            ))?;
            children.push(child(
                call.r,
                checked_scaled_add("c", call.c, 7, num4x4wide >> 3)?,
                sub_size,
                parent,
                chroma_offset,
                call.has_chroma,
            ))?;
        }
    };
    Ok(children)
}

fn child(
    r: usize,
    c: usize,
    b_size: BlockSize,
    parent_size: Option<BlockSize>,
    chroma_offset: bool,
    has_chroma: bool,
) -> TilePartitionCall {
    TilePartitionCall {
        r,
        c,
        b_size,
        parent_size,
        chroma_offset,
        has_chroma,
    }
}

fn updated_chroma_offset(
    call: TilePartitionCall,
    partition: PartitionType,
    sub_size: BlockSize,
    frame: TilePartitionFrameFacts,
) -> Result<bool, TilePartitionTraversalError> {
    if call.chroma_offset || !call.has_chroma {
        return Ok(call.chroma_offset);
    }
    if is_chroma_offset_for_subsize(sub_size, frame)? {
        return Ok(true);
    }
    if partition == PartitionType::Horz3 {
        let middle_chroma = call.b_size.index() == BLOCK_8X32 && frame.subsampling_x;
        if !middle_chroma
            && let Some(midsize) = h_partition_midsize(call.b_size)?.valid()
            && is_chroma_offset_for_subsize(midsize, frame)?
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn is_chroma_offset_for_subsize(
    sub_size: BlockSize,
    frame: TilePartitionFrameFacts,
) -> Result<bool, TilePartitionTraversalError> {
    Ok((frame.subsampling_y && sub_size.mi_height_log2()? == 0)
        || (frame.subsampling_x && sub_size.mi_width_log2()? == 0))
}

fn valid_subsize(
    partition: PartitionType,
    b_size: BlockSize,
) -> Result<BlockSize, TilePartitionTraversalError> {
    match partition_subsize(partition, b_size)? {
        PartitionSubsize::Valid(sub_size) => Ok(sub_size),
        PartitionSubsize::Invalid => Err(TilePartitionTraversalError::InvalidPartitionSubsize {
            partition,
            b_size: b_size.index(),
        }),
    }
}

fn checked_add(
    coordinate: &'static str,
    base: usize,
    offset: usize,
) -> Result<usize, TilePartitionTraversalError> {
    base.checked_add(offset)
        .ok_or(TilePartitionTraversalError::CoordinateOverflow {
            coordinate,
            base,
            offset,
        })
}

fn checked_scaled_add(
    coordinate: &'static str,
    base: usize,
    scale: usize,
    value: usize,
) -> Result<usize, TilePartitionTraversalError> {
    checked_add(
        coordinate,
        base,
        scale
            .checked_mul(value)
            .ok_or(TilePartitionTraversalError::CoordinateOffsetOverflow {
                coordinate,
                left: scale,
                right: value,
            })?,
    )
}

#[cfg(test)]
#[path = "partition_traversal_tests.rs"]
mod tests;

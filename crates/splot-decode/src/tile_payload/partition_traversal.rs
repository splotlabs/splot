// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 5.20.3.1 partition traversal frontier.
//!
//! Feature tracking: `DECODE-TILE-PARTITION-TRAVERSAL-BOUNDARY`.

use splot_core::symbol::{SymbolDecoder, SymbolDecoderConfig};

use super::DecodeTileWorkUnit;
use super::cdf::TileCdfError;
use super::cdf::context::{PartitionContextInput, SquareSplitContextInput};
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
            features,
            max_pb_aspect_ratio,
            has_chroma,
            bru_state,
        })
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
    r: usize,
    c: usize,
    b_size: BlockSize,
    parent_size: Option<BlockSize>,
    chroma_offset: bool,
    has_chroma: bool,
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

/// One consumed partition decision on the frontier path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TilePartitionFrontierStep {
    call: TilePartitionCall,
    decision: ReadPartitionDecision,
    symbol_count_before: u64,
    symbol_count_after: u64,
}

/// The first § 5.20.4.1 `decode_block()` boundary reached by traversal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DecodeBlockFrontier {
    r: usize,
    c: usize,
    b_size: BlockSize,
    has_chroma: bool,
    chroma_offset: bool,
    symbol_count_before_block: u64,
}

/// Successful partition frontier plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TilePartitionTraversalPlan {
    tile_num: u32,
    steps: Vec<TilePartitionFrontierStep>,
    skipped_out_of_frame: Vec<TilePartitionCall>,
    pending_children: Vec<TilePartitionCall>,
    frontier: DecodeBlockFrontier,
    consumed_bits_before: u64,
    consumed_bits_after: u64,
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
    /// BRU/bridge/inactive behavior remains outside this frontier.
    BruOrBridge,
}

/// Plans the AV2 § 5.20.3.1 partition frontier to the first block boundary.
pub(crate) fn plan_tile_partition_traversal_frontier(
    input: TilePartitionTraversalInput<'_, '_, '_>,
) -> Result<TilePartitionTraversalPlan, TilePartitionTraversalError> {
    let TilePartitionTraversalInput {
        work_unit,
        frame,
        context,
        limits,
    } = input;
    if !frame.frame_is_intra {
        return Err(TilePartitionTraversalError::Unsupported(
            TilePartitionTraversalUnsupported::NonIntra,
        ));
    }
    if frame.enable_extended_sdp {
        return Err(TilePartitionTraversalError::Unsupported(
            TilePartitionTraversalUnsupported::ExtendedSdp,
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
        limits.ensure(DecodeLimitName::MaxTileCount, (steps.len() + 1) as u64)?;
        if call.r >= frame.mi_rows || call.c >= frame.mi_cols {
            skipped_out_of_frame.push(call);
            continue;
        }
        ensure_supported_call(frame, call)?;

        let symbol_count_before = symbols.symbol_count();
        let decision =
            read_frontier_partition_decision(call, frame, context, &mut cdfs, &mut symbols)?;
        let symbol_count_after = symbols.symbol_count();
        let partition = decision.partition;
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
            let plan = TilePartitionTraversalPlan {
                tile_num: work_unit.tile_num(),
                steps,
                skipped_out_of_frame,
                pending_children: stack,
                frontier: DecodeBlockFrontier {
                    r: call.r,
                    c: call.c,
                    b_size: sub_size,
                    has_chroma: call.has_chroma && frame.num_planes > 1,
                    chroma_offset,
                    symbol_count_before_block: symbols.symbol_count(),
                },
                consumed_bits_before,
                consumed_bits_after: symbols.consumed_bits().get(),
                symbol_count_after: symbols.symbol_count(),
            };
            *work_unit.cdf_mut().tile_cdfs_mut() = cdfs;
            return Ok(plan);
        }

        let children = child_calls(call, partition, sub_size, frame, chroma_offset)?;
        for child in children.as_slice().iter().rev().copied() {
            stack.push(child);
        }
    }

    Err(TilePartitionTraversalError::NoBlockFrontier)
}

fn ensure_supported_call(
    frame: TilePartitionFrameFacts,
    call: TilePartitionCall,
) -> Result<(), TilePartitionTraversalError> {
    if frame.enable_sdp && call.b_size.index() == BLOCK_64X64 {
        return Err(TilePartitionTraversalError::Unsupported(
            TilePartitionTraversalUnsupported::Sdp,
        ));
    }
    Ok(())
}

fn read_frontier_partition_decision(
    call: TilePartitionCall,
    frame: TilePartitionFrameFacts,
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
        PartitionTreeType::Shared,
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
    let avail_u = call.r > 0 && call.r - 1 < frame.mi_rows && call.c < frame.mi_cols;
    let avail_l = call.c > 0 && call.r < frame.mi_rows && call.c - 1 < frame.mi_cols;
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

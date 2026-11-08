// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 5.20.3.1 partition-tree syntax walk.

use super::lr_records::{WienerNsLrSourceBlock, WienerNsLrUnitActivity, WienerNsLrUnitFilter};
use super::lr_syntax::read_loop_restoration_for_call;
use super::state_publication::{DecodedLeafPublication, publish_intra_leaf_state};
use super::{
    BLOCK_8X32, BlockSize, CALL_CFL_ALLOWED_IN_SDP, ChildCallSink, ChromaRefGeometry,
    DecodeBlockFrontier, DecodeBlockPart, DecodeLimitName, DecodeLimits, DecodeTileWorkUnit,
    GeneralIntraLeafMode, IsCflContext, MAX_CHILD_CALLS, PartitionAllowedInput,
    PartitionContextInput, PartitionDecisionMemo, PartitionTreeType, PartitionType,
    ROOT_HAS_CHROMA, SquareSplitContextInput, SymbolDecoder, TileIntraFactsState, TileMiSizeState,
    TileMiSizeStateError, TilePartitionBounds, TilePartitionCall, TilePartitionFrameFacts,
    TilePartitionTraversalError, TileUvCflState, call_in_frame, checked_mul, child_calls_into,
    decision_memo_key, ensure_supported_traversal_frame, h_partition_midsize, partition_subsize,
    symbol_decoder_for_work_unit,
};

const BLOCK_64X64: usize = 12;
const INTER_SDP_MAX_BLOCK_SIZE: usize = 64;
const INTRA_REGION: u8 = 0;
pub(super) const MIXED_REGION: u8 = 1;

pub(crate) fn is_cfl_context_for_chroma_ref(
    uv_cfls: &TileUvCflState,
    tile_bounds: TilePartitionBounds,
    chroma_ref: ChromaRefGeometry,
) -> IsCflContext {
    let row = chroma_ref.row();
    let col = chroma_ref.col();
    IsCflContext::new(uv_cfls.is_cfl_ctx(
        row,
        col,
        tile_bounds.avail_u_at(row, col),
        tile_bounds.avail_l_at(row, col),
    ))
}

/// The `is_cfl` context of a leaf's chroma reference block, derived only when
/// the leaf's chroma mode syntax reads it.
#[derive(Clone, Copy)]
pub(crate) struct LeafCflContext<'a> {
    uv_cfls: &'a TileUvCflState,
    tile_bounds: TilePartitionBounds,
    chroma_ref: ChromaRefGeometry,
}

impl LeafCflContext<'_> {
    pub(crate) fn resolve(self) -> IsCflContext {
        is_cfl_context_for_chroma_ref(self.uv_cfls, self.tile_bounds, self.chroma_ref)
    }
}

pub(crate) fn decode_block_frontier(
    call: TilePartitionCall,
    frame: &TilePartitionFrameFacts,
    sub_size: BlockSize,
    chroma_offset: bool,
    part: DecodeBlockPart,
) -> DecodeBlockFrontier {
    DecodeBlockFrontier {
        r: call.r(),
        c: call.c(),
        b_size: sub_size,
        has_chroma: call.has_chroma()
            && frame.num_planes > 1
            && !matches!(part, DecodeBlockPart::LumaPart),
        chroma_offset,
        chroma_ref: call.chroma_ref_geometry(),
        part,
        intra_region: call.intra_region(),
        cfl_allowed_in_sdp: call.cfl_allowed_in_sdp(),
    }
}

pub(crate) fn decode_block_part(
    call: TilePartitionCall,
    intra_facts: &TileIntraFactsState,
) -> Result<DecodeBlockPart, TilePartitionTraversalError> {
    match call.tree_type {
        PartitionTreeType::Shared => Ok(DecodeBlockPart::Shared),
        PartitionTreeType::LumaPart => Ok(DecodeBlockPart::LumaPart),
        PartitionTreeType::ChromaPart => {
            let Some(facts) = intra_facts.y_mode_facts_at(call.r(), call.c()) else {
                return Err(TilePartitionTraversalError::MissingSdpLumaModeState {
                    r: call.r(),
                    c: call.c(),
                });
            };
            Ok(DecodeBlockPart::ChromaPart {
                y_mode: facts.y_mode,
                angle_delta_y: facts.angle_delta_y,
            })
        }
    }
}

pub(crate) struct GeneralIntraPartitionTreeOutput<'payload> {
    pub(crate) symbols: SymbolDecoder<'payload>,
    pub(crate) storage: TileTraversalStorage,
}

/// The calls one superblock walk has yet to visit. A partition replaces its
/// call with at most `MAX_CHILD_CALLS` children of at most half its area plus
/// an extended-SDP chroma block, and the area halves at most 12 times from
/// 256x256, so with the SDP split the stack holds fewer than 64 calls.
const WALK_STACK_CAPACITY: usize = 64;

struct WalkStack {
    calls: [TilePartitionCall; WALK_STACK_CAPACITY],
    len: usize,
}

impl WalkStack {
    fn new(fill: TilePartitionCall) -> Self {
        Self {
            calls: [fill; WALK_STACK_CAPACITY],
            len: 0,
        }
    }

    #[inline]
    fn pop(&mut self) -> Option<TilePartitionCall> {
        self.len = self.len.checked_sub(1)?;
        self.calls.get(self.len).copied()
    }

    #[inline]
    fn push(&mut self, call: TilePartitionCall) -> Result<(), TilePartitionTraversalError> {
        let Some(slot) = self.calls.get_mut(self.len) else {
            return Err(TilePartitionTraversalError::TooManyChildCalls);
        };
        *slot = call;
        self.len += 1;
        Ok(())
    }

    /// Pushes the `count` calls `fill` hands its sink in syntax order, last
    /// call deepest, so they pop in syntax order.
    #[inline]
    fn push_reversed(
        &mut self,
        count: usize,
        fill: impl FnOnce(&mut ReversedChildCalls<'_>) -> Result<(), TilePartitionTraversalError>,
    ) -> Result<(), TilePartitionTraversalError> {
        let Some(slots) = self.calls.get_mut(self.len..self.len + count) else {
            return Err(TilePartitionTraversalError::TooManyChildCalls);
        };
        let mut sink = ReversedChildCalls { slots, top: count };
        fill(&mut sink)?;
        if sink.top != 0 {
            return Err(TilePartitionTraversalError::TooManyChildCalls);
        }
        self.len += count;
        Ok(())
    }
}

struct ReversedChildCalls<'s> {
    slots: &'s mut [TilePartitionCall],
    top: usize,
}

impl ChildCallSink for ReversedChildCalls<'_> {
    #[inline]
    fn push_call(&mut self, call: TilePartitionCall) -> Result<(), TilePartitionTraversalError> {
        let Some(top) = self.top.checked_sub(1) else {
            return Err(TilePartitionTraversalError::TooManyChildCalls);
        };
        self.top = top;
        if let Some(slot) = self.slots.get_mut(top) {
            *slot = call;
        }
        Ok(())
    }
}

/// The number of § 5.20.3.1 child calls `partition` makes.
const fn child_call_count(partition: PartitionType) -> usize {
    match partition {
        PartitionType::None => 0,
        PartitionType::Horz | PartitionType::Vert => 2,
        _ => MAX_CHILD_CALLS,
    }
}

/// Parser-owned superblock-row boundary for `INFRA-DECODE-PARALLEL-STAGES`.
pub(crate) struct GeneralIntraPartitionTreeCursor<'payload> {
    frame: TilePartitionFrameFacts,
    symbols: SymbolDecoder<'payload>,
    lr_activity: WienerNsLrUnitActivity,
    tile_bounds: TilePartitionBounds,
    intra_facts: TileIntraFactsState,
    sb_size4: usize,
    sb_mask: usize,
    next_sb_row: usize,
    next_sb_col: usize,
    limits: DecodeLimits,
    step_count: u64,
    sdp_state: SdpPartitionState,
    partition_memo: PartitionDecisionMemo,
    stack: WalkStack,
}

/// Tile-slot storage lent to partition traversal and returned after record merging.
#[derive(Default)]
pub(crate) struct TileTraversalStorage {
    pub(crate) active_source_blocks: Vec<WienerNsLrSourceBlock>,
    pub(crate) unit_filters: Vec<WienerNsLrUnitFilter>,
}

impl<'payload> GeneralIntraPartitionTreeCursor<'payload> {
    /// Whether the intra facts grid touched a row its window had reused.
    pub(crate) fn window_violated(&self) -> bool {
        self.intra_facts.window_violated()
    }

    /// Whether the next superblock starts a new superblock row.
    pub(crate) const fn at_row_start(&self) -> bool {
        self.next_sb_col == self.tile_bounds.mi_col_start
    }

    /// The superblock mask the walk publishes its leaves under.
    pub(crate) const fn sb_mask(&self) -> usize {
        self.sb_mask
    }

    #[cfg(test)]
    pub(crate) fn new(
        work_unit: &DecodeTileWorkUnit,
        frame: TilePartitionFrameFacts,
        limits: DecodeLimits,
        storage: TileTraversalStorage,
    ) -> Result<Self, TilePartitionTraversalError> {
        Self::new_with_bytes(work_unit, work_unit.tile_bytes(), frame, limits, storage)
    }

    pub(crate) fn new_with_bytes(
        work_unit: &DecodeTileWorkUnit,
        tile_bytes: &'payload [u8],
        frame: TilePartitionFrameFacts,
        limits: DecodeLimits,
        storage: TileTraversalStorage,
    ) -> Result<Self, TilePartitionTraversalError> {
        ensure_supported_traversal_frame(frame)?;
        let symbols = symbol_decoder_for_work_unit(work_unit, tile_bytes)?;
        let TileTraversalStorage {
            mut active_source_blocks,
            mut unit_filters,
        } = storage;
        active_source_blocks.clear();
        unit_filters.clear();
        let lr_activity = WienerNsLrUnitActivity {
            active_source_blocks,
            unit_filters,
            ..Default::default()
        };
        let tile_bounds = TilePartitionBounds::from_work_unit(work_unit);
        let tile_rows = work_unit.mi_row_range().start as usize
            ..(work_unit.mi_row_range().end as usize).min(frame.mi_rows);
        let tile_cols = work_unit.mi_col_range().start as usize
            ..(work_unit.mi_col_range().end as usize).min(frame.mi_cols);
        let sb_size4 = frame.sb_size.num_4x4_wide()?.max(1);
        let intra_facts = TileIntraFactsState::new_for_tile(tile_rows, tile_cols, sb_size4)?;
        let next_sb_row = work_unit.mi_row_range().start as usize;
        Ok(Self {
            frame,
            symbols,
            lr_activity,
            tile_bounds,
            intra_facts,
            sb_size4,
            sb_mask: sb_size4.saturating_sub(1),
            next_sb_row,
            next_sb_col: tile_bounds.mi_col_start,
            limits,
            step_count: 0,
            sdp_state: SdpPartitionState::default(),
            partition_memo: PartitionDecisionMemo::default(),
            stack: WalkStack::new(TilePartitionCall::root(
                0,
                0,
                frame.sb_size,
                ROOT_HAS_CHROMA,
            )),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn decode_next_superblock_with_publication<E, C, F, P>(
        &mut self,
        work_unit: &mut DecodeTileWorkUnit,
        mi_size_state: &mut TileMiSizeState,
        uv_cfls: &mut TileUvCflState,
        on_leaf: &mut F,
        on_published: &mut P,
    ) -> Result<Option<[usize; 2]>, GeneralIntraTreeWalkError<E>>
    where
        F: FnMut(
            &mut DecodeTileWorkUnit,
            &mut SymbolDecoder<'payload>,
            &DecodeBlockFrontier,
            &TileIntraFactsState,
            LeafCflContext<'_>,
        ) -> Result<(GeneralIntraLeafMode, C), E>,
        P: FnMut(DecodedLeafPublication, C),
    {
        if self.next_sb_row >= self.tile_bounds.mi_row_end.min(self.frame.mi_rows) {
            return Ok(None);
        }
        let sb_row = self.next_sb_row;
        let sb_col = self.next_sb_col;
        if sb_col == self.tile_bounds.mi_col_start {
            mi_size_state.enter_sb_row(sb_row);
        }
        self.stack.len = 0;
        self.stack.push(TilePartitionCall::root(
            sb_row,
            sb_col,
            self.frame.sb_size,
            ROOT_HAS_CHROMA,
        ))?;
        let step_limit = self
            .limits
            .threshold(DecodeLimitName::MaxTilePartitionSteps);
        while let Some(call) = self.stack.pop() {
            self.step_count += 1;
            if !step_limit.allows(self.step_count) {
                self.limits
                    .ensure(DecodeLimitName::MaxTilePartitionSteps, self.step_count)
                    .map_err(TilePartitionTraversalError::from)?;
            }
            if !call_in_frame(&self.frame, call) {
                continue;
            }
            let (call, sub_size, chroma_offset) = if call.is_extended_sdp_chroma_block() {
                (call, call.b_size, false)
            } else {
                if is_intra_sdp_shared_root(&self.frame, call) {
                    self.stack
                        .push(call.with_tree_type(PartitionTreeType::ChromaPart))?;
                    self.stack
                        .push(call.with_tree_type(PartitionTreeType::LumaPart))?;
                    continue;
                }
                if call.b_size == self.frame.sb_size {
                    read_loop_restoration_for_call(
                        self.frame,
                        call,
                        self.tile_bounds,
                        work_unit.cdf_mut().tile_cdfs_mut(),
                        &mut self.symbols,
                        &mut self.lr_activity,
                        self.limits,
                    )?;
                }
                let mut call = call;
                let (partition, using_extended_sdp) = self.read_partition_step(
                    &mut call,
                    mi_size_state,
                    work_unit.cdf_mut().tile_cdfs_mut(),
                )?;
                let sub_size = valid_subsize(partition, call.b_size)?;
                let chroma_offset = updated_chroma_offset(call, partition, sub_size, &self.frame)?;
                if partition != PartitionType::None {
                    schedule_partition_children(
                        &mut self.stack,
                        call,
                        partition,
                        sub_size,
                        &self.frame,
                        chroma_offset,
                        using_extended_sdp,
                    )?;
                    continue;
                }
                (call, sub_size, chroma_offset)
            };
            let part = decode_block_part(call, &self.intra_facts)?;
            let frontier = decode_block_frontier(call, &self.frame, sub_size, chroma_offset, part);
            let is_cfl_ctx = LeafCflContext {
                uv_cfls,
                tile_bounds: self.tile_bounds,
                chroma_ref: frontier.chroma_ref_geometry(),
            };
            let decoded_leaf = DecodedLeafPublication::new(call, sub_size, self.sb_mask);
            let (leaf_mode, command) = on_leaf(
                work_unit,
                &mut self.symbols,
                &frontier,
                &self.intra_facts,
                is_cfl_ctx,
            )
            .map_err(GeneralIntraTreeWalkError::Leaf)?;
            publish_intra_leaf_state(
                &self.frame,
                &call,
                &frontier,
                leaf_mode,
                sub_size,
                &mut self.intra_facts,
                uv_cfls,
                mi_size_state,
            )?;
            on_published(decoded_leaf, command);
        }
        self.next_sb_col = self.next_sb_col.saturating_add(self.sb_size4);
        if self.next_sb_col >= self.tile_bounds.mi_col_end.min(self.frame.mi_cols) {
            self.next_sb_col = self.tile_bounds.mi_col_start;
            self.next_sb_row = self.next_sb_row.saturating_add(self.sb_size4);
        }
        Ok(Some([sb_row, sb_col]))
    }

    /// Reads the § 5.20.3.2 partition of `call`, then the `CflAllowedInSdp`
    /// and extended-SDP region state it sets.
    #[allow(
        clippy::inline_always,
        reason = "measured: keeps the call in registers"
    )]
    #[inline(always)]
    fn read_partition_step(
        &mut self,
        call: &mut TilePartitionCall,
        mi_size_state: &TileMiSizeState,
        cdfs: &mut super::cdf::TileCdfSubset,
    ) -> Result<(PartitionType, bool), TilePartitionTraversalError> {
        let node = *call;
        let frame = &self.frame;
        let forced_chroma_partition = self.sdp_state.forced_chroma_partition(frame, node);
        let key = if forced_chroma_partition.is_none() {
            decision_memo_key(
                node.b_size,
                node.tree_type,
                [
                    node.has_chroma(),
                    node.chroma_offset(),
                    mixed_region(frame, node),
                ],
                frame.mi_rows - node.r(),
                frame.mi_cols - node.c(),
            )
        } else {
            None
        };
        let facts = self.partition_memo.facts_keyed(key, || {
            partition_allowed_input(node, frame, forced_chroma_partition)
        })?;
        let partition = if let Some(partition) = facts.unread_partition() {
            partition
        } else {
            let (partition_context, square_context) =
                partition_context_inputs(node, self.tile_bounds, mi_size_state)?;
            let decision_input =
                facts.read_partition_decision_input(true, partition_context, square_context);
            super::partition::read_partition_decision(decision_input, cdfs, &mut self.symbols)?
        };
        let cfl_allowed_in_sdp = self.sdp_state.record_partition(frame, node, partition);
        call.flags = call.flags & !CALL_CFL_ALLOWED_IN_SDP
            | super::call_flag(cfl_allowed_in_sdp, CALL_CFL_ALLOWED_IN_SDP);
        let using_extended_sdp =
            read_extended_sdp_region_type(frame, call, partition, cdfs, &mut self.symbols)?;
        Ok((partition, using_extended_sdp))
    }

    pub(crate) fn into_output(self) -> GeneralIntraPartitionTreeOutput<'payload> {
        let Self {
            symbols,
            lr_activity,
            intra_facts,
            ..
        } = self;
        intra_facts.recycle();
        GeneralIntraPartitionTreeOutput {
            symbols,
            storage: TileTraversalStorage {
                active_source_blocks: lr_activity.active_source_blocks,
                unit_filters: lr_activity.unit_filters,
            },
        }
    }
}

/// Pushes the children of `call` so they pop in syntax order, with the
/// extended-SDP chroma block after them.
#[inline]
fn schedule_partition_children(
    stack: &mut WalkStack,
    call: TilePartitionCall,
    partition: PartitionType,
    sub_size: BlockSize,
    frame: &TilePartitionFrameFacts,
    chroma_offset: bool,
    using_extended_sdp: bool,
) -> Result<(), TilePartitionTraversalError> {
    if using_extended_sdp {
        stack.push(extended_sdp_chroma_call(call))?;
    }
    stack.push_reversed(child_call_count(partition), |sink| {
        child_calls_into(sink, call, partition, sub_size, frame, chroma_offset)
    })
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum GeneralIntraTreeWalkError<E> {
    #[error("partition tree walk traversal failed: {0}")]
    Traversal(#[from] TilePartitionTraversalError),
    #[error("partition tree walk MI-size update failed: {0}")]
    MiSize(TileMiSizeStateError),
    #[error("partition tree walk leaf-block decode failed")]
    Leaf(E),
}

#[cfg(test)]
pub(super) fn read_frontier_partition_decision(
    call: TilePartitionCall,
    frame: TilePartitionFrameFacts,
    tile_bounds: TilePartitionBounds,
    context: &TileMiSizeState,
    forced_chroma_partition: Option<PartitionType>,
    cdfs: &mut super::cdf::TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
) -> Result<PartitionType, TilePartitionTraversalError> {
    let facts = PartitionDecisionMemo::default().facts(partition_allowed_input(
        call,
        &frame,
        forced_chroma_partition,
    )?)?;
    let (partition_context, square_context) = partition_context_inputs(call, tile_bounds, context)?;
    let decision_input =
        facts.read_partition_decision_input(true, partition_context, square_context);
    Ok(super::partition::read_partition_decision(
        decision_input,
        cdfs,
        symbols,
    )?)
}

/// Whether § 5.20.3.2 treats `call` as a mixed intra/inter region.
const fn mixed_region(frame: &TilePartitionFrameFacts, call: TilePartitionCall) -> bool {
    !frame.frame_is_intra && call.parent_size.is_some() && !call.intra_region()
}

/// The § 5.20.3.2 `is_partition_allowed` inputs of `call`.
#[inline]
pub(crate) fn partition_allowed_input(
    call: TilePartitionCall,
    frame: &TilePartitionFrameFacts,
    forced_chroma_partition: Option<PartitionType>,
) -> Result<PartitionAllowedInput, TilePartitionTraversalError> {
    Ok(PartitionAllowedInput::new(
        call.r(),
        call.c(),
        frame.mi_rows,
        frame.mi_cols,
        call.b_size.index(),
        call.tree_type,
        frame.subsampling_x,
        frame.subsampling_y,
        frame.features,
        frame.frame_is_intra,
        mixed_region(frame, call),
        frame.max_pb_aspect_ratio,
        call.has_chroma(),
        call.chroma_offset(),
        frame.num_planes,
        forced_chroma_partition,
    )?)
}

/// The § 8.3.2 partition CDF context inputs of `call`.
#[inline]
pub(crate) fn partition_context_inputs(
    call: TilePartitionCall,
    tile_bounds: TilePartitionBounds,
    mi_sizes: &TileMiSizeState,
) -> Result<(PartitionContextInput, SquareSplitContextInput), TilePartitionTraversalError> {
    let plane = partition_cdf_plane(call.tree_type);
    let Some(neighbours) = mi_sizes.partition_neighbours(
        plane,
        call.r(),
        call.c(),
        call.b_size.num_4x4_high()? >> 1,
        call.b_size.num_4x4_wide()? >> 1,
    ) else {
        return Err(TilePartitionTraversalError::CoordinateOverflow {
            coordinate: "partition context line",
            base: call.r(),
            offset: call.c(),
        });
    };
    Ok((
        PartitionContextInput::from_neighbours(
            call.b_size,
            plane,
            neighbours.left,
            neighbours.above,
        ),
        SquareSplitContextInput::from_neighbours(
            call.b_size,
            tile_bounds.avail_u(call).then_some(neighbours.luma_above),
            tile_bounds.avail_l(call).then_some(neighbours.luma_left),
        ),
    ))
}

pub(super) const fn partition_cdf_plane(tree_type: PartitionTreeType) -> usize {
    matches!(tree_type, PartitionTreeType::ChromaPart) as usize
}

pub(crate) fn is_intra_sdp_shared_root(
    frame: &TilePartitionFrameFacts,
    call: TilePartitionCall,
) -> bool {
    frame.enable_sdp
        && frame.frame_is_intra
        && call.tree_type == PartitionTreeType::Shared
        && call.b_size.index() == BLOCK_64X64
}

#[cfg(test)]
mod row_cursor_tests {
    #![allow(clippy::unwrap_used)]

    const WHOLE_TILE: usize = crate::tile::SbRowWindow::WHOLE_TILE_SB_H4;

    use splot_core::symbol::CdfUpdateMode;

    use super::*;
    use crate::bitstream::tile_payload::make_test_work_unit;

    type ParserStates = (TileMiSizeState, TileUvCflState);

    fn frame() -> TilePartitionFrameFacts {
        TilePartitionFrameFacts::new(
            64,
            64,
            9,
            3,
            true,
            true,
            true,
            false,
            false,
            false,
            super::super::TilePartitionLoopRestorationState::NoSyntax,
            super::super::PartitionFeatureFlags::new(true, true),
            4,
        )
        .unwrap()
    }

    fn sdp_frame() -> TilePartitionFrameFacts {
        TilePartitionFrameFacts::new(
            64,
            64,
            BLOCK_64X64,
            3,
            true,
            true,
            true,
            true,
            false,
            false,
            super::super::TilePartitionLoopRestorationState::NoSyntax,
            super::super::PartitionFeatureFlags::new(true, true),
            4,
        )
        .unwrap()
    }

    fn parser_states(frame: TilePartitionFrameFacts) -> ParserStates {
        (
            TileMiSizeState::new_for_tile(0..frame.mi_rows, 0..frame.mi_cols, frame.sb_size)
                .unwrap(),
            TileUvCflState::new(frame.mi_rows, frame.mi_cols, WHOLE_TILE).unwrap(),
        )
    }

    fn leaf_mode() -> GeneralIntraLeafMode {
        GeneralIntraLeafMode::luma(
            super::super::IntraJointMode::DC,
            super::super::IntraYMode::Dc,
            0,
            0,
            super::super::MrlSelection::Disabled,
        )
        .with_uv_cfl(false)
    }

    #[test]
    fn cursor_yields_each_superblock_in_raster_order() {
        let payload = [0_u8; 4096];
        let frame = frame();
        let mut storage = TileTraversalStorage::default();
        for _ in 0..32 {
            let mut work = make_test_work_unit(&payload, CdfUpdateMode::Enabled);
            let mut states = parser_states(frame);
            let mut superblocks = Vec::new();
            let mut cursor =
                GeneralIntraPartitionTreeCursor::new(&work, frame, DecodeLimits::DEFAULT, storage)
                    .unwrap();
            loop {
                let superblock = cursor
                    .decode_next_superblock_with_publication(
                        &mut work,
                        &mut states.0,
                        &mut states.1,
                        &mut |_, _, _, _, _| Ok::<_, ()>((leaf_mode(), ())),
                        &mut |_, ()| {},
                    )
                    .unwrap();
                let Some(superblock) = superblock else {
                    break;
                };
                superblocks.push(superblock);
            }
            assert_eq!(superblocks.len(), 64);
            assert_eq!(superblocks[0], [0, 0]);
            assert_eq!(superblocks[7], [0, 56]);
            assert_eq!(superblocks[8], [8, 0]);
            assert_eq!(superblocks[63], [56, 56]);
            storage = cursor.into_output().storage;
        }
    }

    #[test]
    fn publication_hook_keeps_prefix_before_later_leaf_error() {
        let payload = [0_u8; 4096];
        let frame = frame();
        let mut work = make_test_work_unit(&payload, CdfUpdateMode::Enabled);
        let mut states = parser_states(frame);
        let mut cursor = GeneralIntraPartitionTreeCursor::new(
            &work,
            frame,
            DecodeLimits::DEFAULT,
            TileTraversalStorage::default(),
        )
        .unwrap();
        let mut calls = 0;
        let mut published = Vec::new();

        let result = loop {
            let result = cursor.decode_next_superblock_with_publication(
                &mut work,
                &mut states.0,
                &mut states.1,
                &mut |_, _, _, _, _| {
                    calls += 1;
                    if calls == 2 {
                        Err("malformed second leaf")
                    } else {
                        Ok((leaf_mode(), calls))
                    }
                },
                &mut |_, command| published.push(command),
            );
            if !matches!(result, Ok(Some(_))) {
                break result;
            }
        };

        assert!(result.is_err());
        assert_eq!(calls, 2);
        assert_eq!(published, [1]);
    }

    #[test]
    fn ordinary_sdp_publishes_luma_mode_before_chroma_frontier() {
        let payload = [0_u8; 32];
        let frame = sdp_frame();
        let mut work = make_test_work_unit(&payload, CdfUpdateMode::Enabled);
        let mut states = parser_states(frame);
        let mut cursor = GeneralIntraPartitionTreeCursor::new(
            &work,
            frame,
            DecodeLimits::DEFAULT,
            TileTraversalStorage::default(),
        )
        .unwrap();
        let mut parts = Vec::new();
        let mut published = 0;

        cursor
            .decode_next_superblock_with_publication(
                &mut work,
                &mut states.0,
                &mut states.1,
                &mut |_, _, frontier, _, _| {
                    parts.push(frontier.part());
                    let leaf = if frontier.is_chroma_part() {
                        GeneralIntraLeafMode::chroma(false)
                    } else {
                        GeneralIntraLeafMode::luma(
                            super::super::IntraJointMode::DC,
                            super::super::IntraYMode::D67,
                            2,
                            0,
                            super::super::MrlSelection::Disabled,
                        )
                    };
                    Ok::<_, ()>((leaf, ()))
                },
                &mut |_, ()| published += 1,
            )
            .unwrap();

        assert_eq!(
            parts,
            [
                DecodeBlockPart::LumaPart,
                DecodeBlockPart::ChromaPart {
                    y_mode: super::super::IntraYMode::D67,
                    angle_delta_y: 2,
                },
            ]
        );
        assert_eq!(published, 2);
    }

    #[test]
    fn sdp_intrabc_publishes_dc_mode_before_chroma_frontier() {
        let payload = [0_u8; 32];
        let frame = sdp_frame();
        let mut work = make_test_work_unit(&payload, CdfUpdateMode::Enabled);
        let mut states = parser_states(frame);
        let mut cursor = GeneralIntraPartitionTreeCursor::new(
            &work,
            frame,
            DecodeLimits::DEFAULT,
            TileTraversalStorage::default(),
        )
        .unwrap();
        let mut parts = Vec::new();

        cursor
            .decode_next_superblock_with_publication(
                &mut work,
                &mut states.0,
                &mut states.1,
                &mut |_, _, frontier, _, _| {
                    parts.push(frontier.part());
                    let leaf = if frontier.is_chroma_part() {
                        GeneralIntraLeafMode::chroma(false)
                    } else {
                        GeneralIntraLeafMode::no_luma_mode().mark_intrabc()
                    };
                    Ok::<_, ()>((leaf, ()))
                },
                &mut |_, ()| {},
            )
            .unwrap();

        assert_eq!(
            parts,
            [
                DecodeBlockPart::LumaPart,
                DecodeBlockPart::ChromaPart {
                    y_mode: super::super::IntraYMode::Dc,
                    angle_delta_y: 0,
                },
            ]
        );
    }

    #[test]
    fn failed_sdp_luma_leaf_neither_publishes_mode_nor_reaches_chroma() {
        let payload = [0_u8; 32];
        let frame = sdp_frame();
        let mut work = make_test_work_unit(&payload, CdfUpdateMode::Enabled);
        let mut states = parser_states(frame);
        let mut cursor = GeneralIntraPartitionTreeCursor::new(
            &work,
            frame,
            DecodeLimits::DEFAULT,
            TileTraversalStorage::default(),
        )
        .unwrap();
        let mut leaf_calls = 0;
        let mut published = 0;

        let result = cursor.decode_next_superblock_with_publication(
            &mut work,
            &mut states.0,
            &mut states.1,
            &mut |_, _, frontier, _, _| {
                leaf_calls += 1;
                assert!(frontier.is_luma_part());
                Err::<(GeneralIntraLeafMode, ()), _>("luma decode failed")
            },
            &mut |_, ()| published += 1,
        );

        assert!(matches!(result, Err(GeneralIntraTreeWalkError::Leaf(_))));
        assert_eq!(leaf_calls, 1);
        assert_eq!(published, 0);
        assert_eq!(cursor.intra_facts.y_mode_facts_at(0, 0), None);
    }

    #[test]
    fn missing_chroma_collocation_is_a_typed_construction_error() {
        let intra_facts = TileIntraFactsState::new_for_tile(0..16, 0..16, WHOLE_TILE).unwrap();
        let call =
            TilePartitionCall::root(7, 11, BlockSize::new(BLOCK_64X64).unwrap(), ROOT_HAS_CHROMA)
                .with_tree_type(PartitionTreeType::ChromaPart);

        assert!(matches!(
            decode_block_part(call, &intra_facts),
            Err(TilePartitionTraversalError::MissingSdpLumaModeState { r: 7, c: 11 })
        ));
    }

    #[test]
    fn extended_sdp_schedules_luma_children_before_collocated_chroma() {
        let frame = sdp_frame();
        let luma_call =
            TilePartitionCall::root(0, 0, BlockSize::new(BLOCK_64X64).unwrap(), ROOT_HAS_CHROMA)
                .with_tree_type(PartitionTreeType::LumaPart)
                .with_intra_region(true);
        let sub_size = valid_subsize(PartitionType::Horz, luma_call.b_size).unwrap();
        let children =
            super::super::child_calls(luma_call, PartitionType::Horz, sub_size, &frame, false)
                .unwrap();
        let chroma_call = extended_sdp_chroma_call(luma_call);
        let mut stack = WalkStack::new(luma_call);

        schedule_partition_children(
            &mut stack,
            luma_call,
            PartitionType::Horz,
            sub_size,
            &frame,
            false,
            true,
        )
        .unwrap();

        assert_eq!(stack.pop(), Some(children.as_slice()[0]));
        assert_eq!(stack.pop(), Some(children.as_slice()[1]));
        assert_eq!(stack.pop(), Some(chroma_call));
        assert!(chroma_call.is_extended_sdp_chroma_block());
        assert_eq!(stack.pop(), None);
    }

    #[test]
    fn walk_stack_rejects_overflow_and_short_child_lists() {
        let call =
            TilePartitionCall::root(0, 0, BlockSize::new(BLOCK_64X64).unwrap(), ROOT_HAS_CHROMA);
        let mut stack = WalkStack::new(call);
        for _ in 0..WALK_STACK_CAPACITY {
            stack.push(call).unwrap();
        }
        assert!(matches!(
            stack.push(call),
            Err(TilePartitionTraversalError::TooManyChildCalls)
        ));
        stack.len = 0;
        assert!(stack.push_reversed(2, |sink| sink.push_call(call)).is_err());
        assert_eq!(stack.len, 0);
    }
}

pub(crate) fn extended_sdp_allowed_for_child(
    frame: &TilePartitionFrameFacts,
    call: TilePartitionCall,
    partition: PartitionType,
    sub_size: BlockSize,
) -> bool {
    if !frame.enable_extended_sdp || !call.extended_sdp_allowed() {
        return false;
    }
    let Ok(width) = sub_size.width_samples() else {
        return false;
    };
    let Ok(height) = sub_size.height_samples() else {
        return false;
    };
    if width <= 4 || height <= 4 {
        return false;
    }
    if !matches!(partition, PartitionType::Horz3 | PartitionType::Vert3) {
        return true;
    }
    let Ok(middle_size) = h_partition_midsize(call.b_size) else {
        return false;
    };
    let Some(middle_size) = middle_size else {
        return false;
    };
    let Ok(middle_width) = middle_size.width_samples() else {
        return false;
    };
    let Ok(middle_height) = middle_size.height_samples() else {
        return false;
    };
    middle_width > 4 && middle_height > 4
}

/// Reads the § 5.20.3.1 `region_type` of `call` when it codes one and
/// applies it; returns whether the call starts an extended-SDP luma tree.
pub(super) fn read_extended_sdp_region_type(
    frame: &TilePartitionFrameFacts,
    call: &mut TilePartitionCall,
    partition: PartitionType,
    cdfs: &mut super::cdf::TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
) -> Result<bool, TilePartitionTraversalError> {
    if !should_read_extended_sdp_region_type(frame, *call, partition)? {
        return Ok(false);
    }
    let ctx = intra_region_context(call.b_size)?;
    let selector = super::cdf::TileCdfSelector::RegionType { ctx };
    let region_type = cdfs
        .with_row_mut(selector, |row| symbols.read_symbol_u16(row))??
        .get();
    match region_type {
        INTRA_REGION => {
            *call = call
                .with_tree_type(PartitionTreeType::LumaPart)
                .with_intra_region(true);
            Ok(true)
        }
        MIXED_REGION => {
            *call = call.with_intra_region(false);
            Ok(false)
        }
        value => Err(TilePartitionTraversalError::InvalidRegionType { value }),
    }
}

pub(super) fn should_read_extended_sdp_region_type(
    frame: &TilePartitionFrameFacts,
    call: TilePartitionCall,
    partition: PartitionType,
) -> Result<bool, TilePartitionTraversalError> {
    if frame.frame_is_intra
        || !frame.enable_extended_sdp
        || call.tree_type != PartitionTreeType::Shared
        || call.intra_region()
        || !call.extended_sdp_allowed()
        || call.b_size == frame.sb_size
        || partition == PartitionType::None
    {
        return Ok(false);
    }
    is_bsize_allowed_for_extended_sdp(call.b_size, partition)
}
fn is_bsize_allowed_for_extended_sdp(
    b_size: BlockSize,
    partition: PartitionType,
) -> Result<bool, TilePartitionTraversalError> {
    let width = b_size.width_samples()?;
    let height = b_size.height_samples()?;
    Ok(width <= INTER_SDP_MAX_BLOCK_SIZE
        && height <= INTER_SDP_MAX_BLOCK_SIZE
        && width >= 8
        && height >= 8
        && matches!(
            partition,
            PartitionType::Horz | PartitionType::Vert | PartitionType::Horz3 | PartitionType::Vert3
        ))
}

fn intra_region_context(b_size: BlockSize) -> Result<usize, TilePartitionTraversalError> {
    let samples = checked_mul(
        "region_type_area",
        b_size.width_samples()?,
        b_size.height_samples()?,
    )?;
    Ok(match samples {
        0..=128 => 0,
        129..=512 => 1,
        513..=1024 => 2,
        _ => 3,
    })
}

fn extended_sdp_chroma_call(call: TilePartitionCall) -> TilePartitionCall {
    TilePartitionCall::child(
        call.r(),
        call.c(),
        call.b_size,
        call.parent_size,
        false,
        ROOT_HAS_CHROMA,
        PartitionTreeType::ChromaPart,
        Some(ChromaRefGeometry::new(call.r(), call.c(), call.b_size)),
        false,
        true,
    )
    .with_cfl_allowed_in_sdp(true)
    .as_extended_sdp_chroma_block()
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct SdpPartitionState {
    top_luma_horz: bool,
    top_luma_vert: bool,
    top_luma_uneven_horz: bool,
    top_luma_uneven_vert: bool,
    chroma_follows_luma: bool,
    luma_64x64_partition: Option<PartitionType>,
}

impl SdpPartitionState {
    pub(crate) fn record_partition(
        &mut self,
        frame: &TilePartitionFrameFacts,
        call: TilePartitionCall,
        partition: PartitionType,
    ) -> bool {
        if !frame.frame_is_intra {
            return call.cfl_allowed_in_sdp();
        }

        if call.tree_type == PartitionTreeType::LumaPart && call.b_size.index() == BLOCK_64X64 {
            self.top_luma_horz = matches!(partition, PartitionType::Horz | PartitionType::Horz3);
            self.top_luma_vert = matches!(partition, PartitionType::Vert | PartitionType::Vert3);
            self.top_luma_uneven_horz =
                matches!(partition, PartitionType::Horz4A | PartitionType::Horz4B);
            self.top_luma_uneven_vert =
                matches!(partition, PartitionType::Vert4A | PartitionType::Vert4B);
            self.chroma_follows_luma =
                partition == PartitionType::None || self.top_luma_horz || self.top_luma_vert;
            self.luma_64x64_partition = Some(partition);
        }

        let this_horz = matches!(
            partition,
            PartitionType::Horz
                | PartitionType::Horz3
                | PartitionType::Horz4A
                | PartitionType::Horz4B
        );
        let this_vert = matches!(
            partition,
            PartitionType::Vert
                | PartitionType::Vert3
                | PartitionType::Vert4A
                | PartitionType::Vert4B
        );

        let mut cfl_allowed_in_sdp = call.cfl_allowed_in_sdp();
        if call.tree_type == PartitionTreeType::ChromaPart && call.b_size.index() == BLOCK_64X64 {
            cfl_allowed_in_sdp = self.chroma_follows_luma
                || partition == PartitionType::None
                || ((self.top_luma_horz || self.top_luma_uneven_horz) && this_horz)
                || ((self.top_luma_vert || self.top_luma_uneven_vert) && this_vert);
        }

        if call.tree_type == PartitionTreeType::LumaPart
            && call
                .parent_size
                .is_some_and(|parent| parent.index() == BLOCK_64X64)
            && (partition == PartitionType::None
                || (self.top_luma_horz && this_horz)
                || (self.top_luma_vert && this_vert))
        {
            self.chroma_follows_luma = false;
        }

        cfl_allowed_in_sdp
    }

    pub(crate) fn forced_chroma_partition(
        &self,
        frame: &TilePartitionFrameFacts,
        call: TilePartitionCall,
    ) -> Option<PartitionType> {
        if !frame.frame_is_intra
            || call.tree_type != PartitionTreeType::ChromaPart
            || call.b_size.index() != BLOCK_64X64
            || !self.chroma_follows_luma
        {
            return None;
        }
        self.luma_64x64_partition
    }
}

pub(crate) fn updated_chroma_offset(
    call: TilePartitionCall,
    partition: PartitionType,
    sub_size: BlockSize,
    frame: &TilePartitionFrameFacts,
) -> Result<bool, TilePartitionTraversalError> {
    if call.chroma_offset() || !call.has_chroma() {
        return Ok(call.chroma_offset());
    }
    if is_chroma_offset_for_subsize(sub_size, frame)? {
        return Ok(true);
    }
    if partition == PartitionType::Horz3 {
        let middle_chroma = call.b_size.index() == BLOCK_8X32 && frame.subsampling_x;
        if !middle_chroma
            && let Some(midsize) = h_partition_midsize(call.b_size)?
            && is_chroma_offset_for_subsize(midsize, frame)?
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn is_chroma_offset_for_subsize(
    sub_size: BlockSize,
    frame: &TilePartitionFrameFacts,
) -> Result<bool, TilePartitionTraversalError> {
    Ok((frame.subsampling_y && sub_size.mi_height_log2()? == 0)
        || (frame.subsampling_x && sub_size.mi_width_log2()? == 0))
}

pub(crate) fn valid_subsize(
    partition: PartitionType,
    b_size: BlockSize,
) -> Result<BlockSize, TilePartitionTraversalError> {
    match partition_subsize(partition, b_size)? {
        Some(sub_size) => Ok(sub_size),
        None => Err(TilePartitionTraversalError::InvalidPartitionSubsize {
            partition,
            b_size: b_size.index(),
        }),
    }
}

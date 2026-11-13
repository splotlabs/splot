// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 5.20.3.1 partition-tree syntax walk.

use super::lr_records::{WienerNsLrSourceBlock, WienerNsLrUnitActivity, WienerNsLrUnitFilter};
use super::lr_syntax::read_loop_restoration_for_call;
use super::state_publication::{DecodedLeafPublication, publish_intra_leaf_state};
use super::{
    BLOCK_8X32, BLOCK_64X64, BlockSize, CALL_CFL_ALLOWED_IN_SDP, CALL_HAS_PARENT,
    CALL_PARENT_64X64, ChildCallSink, ChromaRefGeometry, DecodeBlockFrontier, DecodeBlockPart,
    DecodeLimitName, DecodeLimits, DecodeTileWorkUnit, GeneralIntraLeafMode, PartitionAllowedInput,
    PartitionContextInput, PartitionDecisionMemo, PartitionTreeType, PartitionType,
    ROOT_HAS_CHROMA, SquareSplitContextInput, SymbolDecoder, TileIntraFactsState, TileMiSizeState,
    TileMiSizeStateError, TilePartitionBounds, TilePartitionCall, TilePartitionFrameFacts,
    TilePartitionTraversalError, call_in_frame, checked_mul, child_calls_into, derive_memo_facts,
    ensure_supported_traversal_frame, h_partition_midsize, partition_subsize,
    symbol_decoder_for_work_unit,
};

const INTER_SDP_MAX_BLOCK_SIZE: usize = 64;
pub(super) const INTRA_REGION: u8 = 0;
pub(super) const MIXED_REGION: u8 = 1;
/// § 5.20.2.1 `bru_mode = use_bru ? BRU_INACTIVE : BRU_ACTIVE` without
/// `BruTileActive`: the tile facts admit only `use_bru == 0` frames, so every
/// superblock the walk reaches is `BRU_ACTIVE`.
const NON_BRU_FRAME_SUPERBLOCK_BRU_ACTIVE: bool = true;

/// The neighbour state a leaf's mode syntax reads.
#[derive(Clone, Copy)]
pub(crate) struct LeafNeighbours<'a> {
    /// The tile's edge lines.
    pub(crate) edges: &'a TileMiSizeState,
    /// The luma facts an SDP chroma part reads at its own position.
    pub(crate) collocated: &'a TileIntraFactsState,
}

pub(crate) fn decode_block_frontier(
    call: TilePartitionCall,
    chroma_ref: ChromaRefGeometry,
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
        chroma_ref,
        part,
        intra_region: call.intra_region(),
        cfl_allowed_in_sdp: call.cfl_allowed_in_sdp(),
    }
}

#[allow(clippy::inline_always, reason = "measured: once per leaf")]
#[inline(always)]
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

/// Parser-owned superblock-row boundary for `INFRA-DECODE-PARALLEL-STAGES`.
pub(crate) struct GeneralIntraPartitionTreeCursor<'payload> {
    frame: TilePartitionFrameFacts,
    walker: TreeWalker<'payload>,
    sb_size4: usize,
    next_sb_row: usize,
    next_sb_col: usize,
}

/// The state a superblock's recursive walk updates, apart from the frame
/// facts it reads beside it.
struct TreeWalker<'payload> {
    symbols: SymbolDecoder<'payload>,
    lr_activity: WienerNsLrUnitActivity,
    tile_bounds: TilePartitionBounds,
    intra_facts: TileIntraFactsState,
    sb_mask: usize,
    limits: DecodeLimits,
    step_count: u64,
    step_max: u64,
    sdp_state: SdpPartitionState,
    partition_memo: PartitionDecisionMemo,
    /// The chroma reference block the next call inherits.
    chroma_ref: ChromaRefGeometry,
    /// Whether the current superblock's § 5.20.2.1 `bru_mode` is `BRU_ACTIVE`.
    bru_active: bool,
}

/// The leaf callbacks and parser states one superblock walk lends its leaves.
struct LeafHooks<'h, F, P> {
    work_unit: &'h mut DecodeTileWorkUnit,
    mi_size_state: &'h mut TileMiSizeState,
    on_leaf: &'h mut F,
    on_published: &'h mut P,
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
        self.walker.intra_facts.window_violated()
    }

    /// Whether the next superblock starts a new superblock row.
    pub(crate) const fn at_row_start(&self) -> bool {
        self.next_sb_col == self.walker.tile_bounds.mi_col_start
    }

    /// The superblock mask the walk publishes its leaves under.
    pub(crate) const fn sb_mask(&self) -> usize {
        self.walker.sb_mask
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
        let step_max = limits
            .threshold(DecodeLimitName::MaxTilePartitionSteps)
            .max_value()
            .unwrap_or(u64::MAX);
        Ok(Self {
            frame,
            walker: TreeWalker {
                symbols,
                lr_activity,
                tile_bounds,
                intra_facts,
                sb_mask: sb_size4.saturating_sub(1),
                limits,
                step_count: 0,
                step_max,
                sdp_state: SdpPartitionState::default(),
                partition_memo: PartitionDecisionMemo::default(),
                chroma_ref: ChromaRefGeometry::new(0, 0, frame.sb_size),
                bru_active: NON_BRU_FRAME_SUPERBLOCK_BRU_ACTIVE,
            },
            sb_size4,
            next_sb_row,
            next_sb_col: tile_bounds.mi_col_start,
        })
    }

    pub(crate) fn decode_next_superblock_with_publication<E, C, F, P>(
        &mut self,
        work_unit: &mut DecodeTileWorkUnit,
        mi_size_state: &mut TileMiSizeState,
        on_leaf: &mut F,
        on_published: &mut P,
    ) -> Result<Option<[usize; 2]>, GeneralIntraTreeWalkError<E>>
    where
        F: FnMut(
            &mut DecodeTileWorkUnit,
            &mut SymbolDecoder<'payload>,
            &DecodeBlockFrontier,
            LeafNeighbours<'_>,
        ) -> Result<(GeneralIntraLeafMode, C), E>,
        P: FnMut(DecodedLeafPublication, C),
    {
        let tile_bounds = self.walker.tile_bounds;
        if self.next_sb_row >= tile_bounds.mi_row_end.min(self.frame.mi_rows) {
            return Ok(None);
        }
        let sb_row = self.next_sb_row;
        let sb_col = self.next_sb_col;
        if sb_col == tile_bounds.mi_col_start {
            mi_size_state.enter_sb_row(sb_row);
        }
        mi_size_state
            .check_extent(sb_row, sb_col, self.sb_size4, self.sb_size4)
            .map_err(GeneralIntraTreeWalkError::MiSize)?;
        let mut hooks = LeafHooks {
            work_unit,
            mi_size_state,
            on_leaf,
            on_published,
        };
        let root = TilePartitionCall::root(sb_row, sb_col, self.frame.sb_size, ROOT_HAS_CHROMA);
        walk_call(&self.frame, &mut self.walker, &mut hooks, root).map_err(|error| *error)?;
        self.next_sb_col = self.next_sb_col.saturating_add(self.sb_size4);
        if self.next_sb_col >= tile_bounds.mi_col_end.min(self.frame.mi_cols) {
            self.next_sb_col = tile_bounds.mi_col_start;
            self.next_sb_row = self.next_sb_row.saturating_add(self.sb_size4);
        }
        Ok(Some([sb_row, sb_col]))
    }

    pub(crate) fn into_output(self) -> GeneralIntraPartitionTreeOutput<'payload> {
        let TreeWalker {
            symbols,
            lr_activity,
            intra_facts,
            ..
        } = self.walker;
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

/// The error of a walk step, boxed so each recursion returns one register.
type WalkError<E> = Box<GeneralIntraTreeWalkError<E>>;

impl<E> From<TilePartitionTraversalError> for WalkError<E> {
    #[cold]
    #[inline(never)]
    fn from(error: TilePartitionTraversalError) -> Self {
        Box::new(GeneralIntraTreeWalkError::Traversal(error))
    }
}

/// The § 5.20.3.1 `decode_partition` call `call`: its partition, then its
/// children in syntax order or its leaf block.
fn walk_call<'payload, E, C, F, P>(
    frame: &TilePartitionFrameFacts,
    walker: &mut TreeWalker<'payload>,
    hooks: &mut LeafHooks<'_, F, P>,
    call: TilePartitionCall,
) -> Result<(), WalkError<E>>
where
    F: FnMut(
        &mut DecodeTileWorkUnit,
        &mut SymbolDecoder<'payload>,
        &DecodeBlockFrontier,
        LeafNeighbours<'_>,
    ) -> Result<(GeneralIntraLeafMode, C), E>,
    P: FnMut(DecodedLeafPublication, C),
{
    walker.step_count += 1;
    if walker.step_count > walker.step_max {
        walker
            .limits
            .ensure(DecodeLimitName::MaxTilePartitionSteps, walker.step_count)
            .map_err(TilePartitionTraversalError::from)?;
    }
    if !call_in_frame(frame, call) {
        return Ok(());
    }
    let chroma_ref = call.chroma_ref(walker.chroma_ref);
    let (call, sub_size, chroma_offset) = if call.is_extended_sdp_chroma_block() {
        (call, call.b_size, false)
    } else {
        if is_intra_sdp_shared_root(frame, call) {
            walker.chroma_ref = chroma_ref;
            walk_call(
                frame,
                walker,
                hooks,
                call.with_tree_type(PartitionTreeType::LumaPart),
            )?;
            walker.chroma_ref = chroma_ref;
            return walk_call(
                frame,
                walker,
                hooks,
                call.with_tree_type(PartitionTreeType::ChromaPart),
            );
        }
        if call.b_size == frame.sb_size {
            read_loop_restoration_for_call(
                *frame,
                call,
                walker.tile_bounds,
                hooks.work_unit.cdf_mut().tile_cdfs_mut(),
                &mut walker.symbols,
                &mut walker.lr_activity,
                walker.limits,
            )?;
        }
        let mut call = call;
        let partition = walker.read_partition_step(
            frame,
            &mut call,
            hooks.mi_size_state,
            hooks.work_unit.cdf_mut().tile_cdfs_mut(),
        )?;
        let sub_size = valid_subsize(partition, call.b_size)?;
        let chroma_offset = updated_chroma_offset(call, partition, sub_size, frame)?;
        if partition != PartitionType::None {
            let using_extended_sdp = read_extended_sdp_region_type(
                frame,
                &mut call,
                partition,
                walker.bru_active,
                hooks.work_unit.cdf_mut().tile_cdfs_mut(),
                &mut walker.symbols,
            )?;
            return visit_partition_children(
                &mut |child, inherited| {
                    walker.chroma_ref = inherited;
                    walk_call(frame, walker, hooks, child)
                },
                call,
                chroma_ref,
                partition,
                sub_size,
                frame,
                chroma_offset,
                using_extended_sdp,
            );
        }
        (call, sub_size, chroma_offset)
    };
    let part = decode_block_part(call, &walker.intra_facts)?;
    let frontier = decode_block_frontier(call, chroma_ref, frame, sub_size, chroma_offset, part);
    let decoded_leaf = DecodedLeafPublication::new(call, sub_size, walker.sb_mask);
    let (leaf_mode, command) = (hooks.on_leaf)(
        hooks.work_unit,
        &mut walker.symbols,
        &frontier,
        LeafNeighbours {
            edges: hooks.mi_size_state,
            collocated: &walker.intra_facts,
        },
    )
    .map_err(|error| Box::new(GeneralIntraTreeWalkError::Leaf(error)))?;
    publish_intra_leaf_state(
        frame,
        &call,
        &frontier,
        leaf_mode,
        sub_size,
        &mut walker.intra_facts,
        hooks.mi_size_state,
    )
    .map_err(Box::new)?;
    (hooks.on_published)(decoded_leaf, command);
    Ok(())
}

/// Hands `sink` the children of `call` in syntax order, then the
/// extended-SDP chroma block decoded after its luma tree.
#[allow(
    clippy::inline_always,
    clippy::too_many_arguments,
    reason = "measured: the decoder walk recurses through each push"
)]
#[inline(always)]
fn visit_partition_children<S: ChildCallSink>(
    sink: &mut S,
    call: TilePartitionCall,
    chroma_ref: ChromaRefGeometry,
    partition: PartitionType,
    sub_size: BlockSize,
    frame: &TilePartitionFrameFacts,
    chroma_offset: bool,
    using_extended_sdp: bool,
) -> Result<(), S::Error> {
    child_calls_into(
        sink,
        call,
        chroma_ref,
        partition,
        sub_size,
        frame,
        chroma_offset,
    )?;
    if using_extended_sdp {
        let chroma_call = extended_sdp_chroma_call(call);
        sink.push_call(chroma_call, chroma_call.own_chroma_ref())?;
    }
    Ok(())
}

impl TreeWalker<'_> {
    /// Reads the § 5.20.3.2 partition of `call`, then the `CflAllowedInSdp`
    /// state it sets.
    #[allow(
        clippy::inline_always,
        reason = "measured: keeps the call in registers"
    )]
    #[inline(always)]
    fn read_partition_step(
        &mut self,
        frame: &TilePartitionFrameFacts,
        call: &mut TilePartitionCall,
        mi_size_state: &TileMiSizeState,
        cdfs: &mut super::cdf::TileCdfSubset,
    ) -> Result<PartitionType, TilePartitionTraversalError> {
        let node = *call;
        let facts = match self.sdp_state.forced_chroma_partition(frame, node) {
            None => self.partition_memo.facts_at(
                node.b_size,
                node.tree_type,
                usize::from(node.flags),
                [frame.mi_rows - node.r(), frame.mi_cols - node.c()],
                || partition_allowed_input(node, frame, None),
            )?,
            forced => {
                derive_memo_facts(None, move || partition_allowed_input(node, frame, forced))?
            }
        };
        let partition = if let Some(partition) = facts.unread_partition() {
            partition
        } else {
            let (partition_context, square_context) =
                partition_context_inputs(node, self.tile_bounds, mi_size_state)?;
            let decision_input = facts.read_partition_decision_input(
                self.bru_active,
                partition_context,
                square_context,
            );
            super::partition::read_partition_decision(decision_input, cdfs, &mut self.symbols)?
        };
        let cfl_allowed_in_sdp = self.sdp_state.record_partition(frame, node, partition);
        call.flags = call.flags & !CALL_CFL_ALLOWED_IN_SDP
            | super::call_flag(cfl_allowed_in_sdp, CALL_CFL_ALLOWED_IN_SDP);
        Ok(partition)
    }
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
    let decision_input = facts.read_partition_decision_input(
        NON_BRU_FRAME_SUPERBLOCK_BRU_ACTIVE,
        partition_context,
        square_context,
    );
    Ok(super::partition::read_partition_decision(
        decision_input,
        cdfs,
        symbols,
    )?)
}

/// Whether § 5.20.3.2 treats `call` as a mixed intra/inter region.
const fn mixed_region(frame: &TilePartitionFrameFacts, call: TilePartitionCall) -> bool {
    !frame.frame_is_intra && call.flag(CALL_HAS_PARENT) && !call.intra_region()
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

    fn parser_states(frame: TilePartitionFrameFacts) -> TileMiSizeState {
        TileMiSizeState::new_for_tile(0..frame.mi_rows, 0..frame.mi_cols, frame.sb_size).unwrap()
    }

    fn leaf_mode() -> GeneralIntraLeafMode {
        GeneralIntraLeafMode::luma(
            super::super::IntraJointMode::DC,
            super::super::IntraYMode::Dc,
            0,
            0,
            super::super::MrlSelection::Disabled,
        )
        .with_uv(0)
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
                        &mut states,
                        &mut |_, _, _, _| Ok::<_, ()>((leaf_mode(), ())),
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
                &mut states,
                &mut |_, _, _, _| {
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
                &mut states,
                &mut |_, _, frontier, _| {
                    parts.push(frontier.part());
                    let leaf = if frontier.is_chroma_part() {
                        GeneralIntraLeafMode::chroma(0)
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
                &mut states,
                &mut |_, _, frontier, _| {
                    parts.push(frontier.part());
                    let leaf = if frontier.is_chroma_part() {
                        GeneralIntraLeafMode::chroma(0)
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
            &mut states,
            &mut |_, _, frontier, _| {
                leaf_calls += 1;
                assert!(frontier.is_luma_part());
                Err::<(GeneralIntraLeafMode, ()), _>("luma decode failed")
            },
            &mut |_, ()| published += 1,
        );

        assert!(matches!(result, Err(GeneralIntraTreeWalkError::Leaf(_))));
        assert_eq!(leaf_calls, 1);
        assert_eq!(published, 0);
        assert_eq!(cursor.walker.intra_facts.y_mode_facts_at(0, 0), None);
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
    fn extended_sdp_visits_luma_children_before_collocated_chroma() {
        let frame = sdp_frame();
        let luma_call =
            TilePartitionCall::root(0, 0, BlockSize::new(BLOCK_64X64).unwrap(), ROOT_HAS_CHROMA)
                .with_tree_type(PartitionTreeType::LumaPart)
                .with_intra_region(true);
        let sub_size = valid_subsize(PartitionType::Horz, luma_call.b_size).unwrap();
        let children = super::super::child_calls(
            luma_call,
            luma_call.own_chroma_ref(),
            PartitionType::Horz,
            sub_size,
            &frame,
            false,
        )
        .unwrap();
        let chroma_call = extended_sdp_chroma_call(luma_call);
        let mut visited = Vec::new();

        visit_partition_children(
            &mut |call, _| {
                visited.push(call);
                Ok::<_, TilePartitionTraversalError>(())
            },
            luma_call,
            luma_call.own_chroma_ref(),
            PartitionType::Horz,
            sub_size,
            &frame,
            false,
            true,
        )
        .unwrap();

        assert!(chroma_call.is_extended_sdp_chroma_block());
        assert_eq!(
            visited,
            [children.as_slice()[0], children.as_slice()[1], chroma_call]
        );
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
#[allow(clippy::inline_always, reason = "measured: once per partition node")]
#[inline(always)]
pub(super) fn read_extended_sdp_region_type(
    frame: &TilePartitionFrameFacts,
    call: &mut TilePartitionCall,
    partition: PartitionType,
    bru_active: bool,
    cdfs: &mut super::cdf::TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
) -> Result<bool, TilePartitionTraversalError> {
    if !should_read_extended_sdp_region_type(frame, *call, partition, bru_active)? {
        return Ok(false);
    }
    read_region_type(call, cdfs, symbols)
}

#[inline(never)]
fn read_region_type(
    call: &mut TilePartitionCall,
    cdfs: &mut super::cdf::TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
) -> Result<bool, TilePartitionTraversalError> {
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

#[allow(clippy::inline_always, reason = "measured: once per partition node")]
#[inline(always)]
pub(super) fn should_read_extended_sdp_region_type(
    frame: &TilePartitionFrameFacts,
    call: TilePartitionCall,
    partition: PartitionType,
    bru_active: bool,
) -> Result<bool, TilePartitionTraversalError> {
    if partition == PartitionType::None
        || !bru_active
        || frame.frame_is_intra
        || !frame.enable_extended_sdp
        || call.tree_type != PartitionTreeType::Shared
        || call.intra_region()
        || !call.extended_sdp_allowed()
        || call.b_size == frame.sb_size
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

/// The collocated chroma block of an extended-SDP luma tree: its own chroma
/// reference, intra region, `CflAllowedInSdp`, and the parent of `call`.
const fn extended_sdp_chroma_call(call: TilePartitionCall) -> TilePartitionCall {
    TilePartitionCall {
        tree_type: PartitionTreeType::ChromaPart,
        flags: call.flags & (CALL_HAS_PARENT | CALL_PARENT_64X64)
            | super::call_flag(ROOT_HAS_CHROMA, super::CALL_HAS_CHROMA)
            | CALL_CFL_ALLOWED_IN_SDP
            | super::CALL_INTRA_REGION
            | super::CALL_EXTENDED_SDP_CHROMA_BLOCK,
        ..call
    }
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
    #[allow(clippy::inline_always, reason = "measured: once per partition node")]
    #[inline(always)]
    pub(crate) fn record_partition(
        &mut self,
        frame: &TilePartitionFrameFacts,
        call: TilePartitionCall,
        partition: PartitionType,
    ) -> bool {
        if !frame.frame_is_intra {
            return call.cfl_allowed_in_sdp();
        }
        self.record_intra_partition(call, partition)
    }

    fn record_intra_partition(
        &mut self,
        call: TilePartitionCall,
        partition: PartitionType,
    ) -> bool {
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
            && call.flag(CALL_PARENT_64X64)
            && (partition == PartitionType::None
                || (self.top_luma_horz && this_horz)
                || (self.top_luma_vert && this_vert))
        {
            self.chroma_follows_luma = false;
        }

        cfl_allowed_in_sdp
    }

    pub(crate) fn forced_chroma_partition(
        self,
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

#[allow(
    clippy::inline_always,
    reason = "measured: the decoder walk recurses through each push"
)]
#[inline(always)]
pub(crate) fn updated_chroma_offset(
    call: TilePartitionCall,
    partition: PartitionType,
    sub_size: BlockSize,
    frame: &TilePartitionFrameFacts,
) -> Result<bool, TilePartitionTraversalError> {
    if call.chroma_offset() || !call.has_chroma() {
        return Ok(call.chroma_offset());
    }
    if is_chroma_offset_for_subsize(sub_size, frame) {
        return Ok(true);
    }
    if partition == PartitionType::Horz3 {
        let middle_chroma = call.b_size.index() == BLOCK_8X32 && frame.subsampling_x;
        if !middle_chroma
            && let Some(midsize) = h_partition_midsize(call.b_size)?
            && is_chroma_offset_for_subsize(midsize, frame)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

#[inline]
fn is_chroma_offset_for_subsize(sub_size: BlockSize, frame: &TilePartitionFrameFacts) -> bool {
    let [high, wide] = sub_size.num_4x4();
    (frame.subsampling_y && high == 1) || (frame.subsampling_x && wide == 1)
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

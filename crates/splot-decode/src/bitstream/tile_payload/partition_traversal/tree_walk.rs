// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 5.20.3.1 partition-tree syntax walk.

use super::lr_records::{WienerNsLrSourceBlock, WienerNsLrUnitActivity, WienerNsLrUnitFilter};
use super::lr_syntax::read_loop_restoration_for_call;
use super::state_publication::{DecodedLeafPublication, publish_intra_leaf_state};
use super::{
    BLOCK_8X32, BlockSize, ChromaRefGeometry, DecodeBlockFrontier, DecodeLimitName, DecodeLimits,
    DecodeTileWorkUnit, GeneralIntraLeafMode, IsCflContext, PartitionAllowedInput,
    PartitionContextInput, PartitionSubsize, PartitionTreeType, PartitionType,
    ReadPartitionDecision, SquareSplitContextInput, SymbolDecoder, TileFscModeState,
    TileIntraJointModeState, TileIntraYModeFacts, TileIntraYModeState, TileLumaPaletteState,
    TileMiSizeState, TileMiSizeStateError, TilePartitionBounds, TilePartitionBruState,
    TilePartitionCall, TilePartitionContextState, TilePartitionFrameFacts,
    TilePartitionFrontierStep, TilePartitionTraversalCursor, TilePartitionTraversalError,
    TilePartitionTraversalInput, TilePartitionTraversalPlan, TilePartitionTraversalUnsupported,
    TileUseDipState, TileUsesMrlsState, TileUvCflState, call_in_frame, checked_mul, child_calls,
    ensure_supported_traversal_frame, h_partition_midsize, partition_decision_facts,
    partition_subsize, root_partition_call, symbol_decoder_for_work_unit,
};

const BLOCK_64X64: usize = 12;
const INTER_SDP_MAX_BLOCK_SIZE: usize = 64;
const INTRA_REGION: u8 = 0;
pub(super) const MIXED_REGION: u8 = 1;

pub(super) fn is_cfl_context_for_chroma_ref(
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

pub(crate) fn plan_tile_partition_traversal_frontier(
    input: TilePartitionTraversalInput<'_, '_, '_>,
) -> Result<TilePartitionTraversalPlan, TilePartitionTraversalError> {
    Ok(plan_tile_partition_traversal_cursor(input)?.plan)
}

pub(super) fn decode_block_frontier(
    call: TilePartitionCall,
    frame: TilePartitionFrameFacts,
    sub_size: BlockSize,
    chroma_offset: bool,
    stored_luma_y_mode: Option<TileIntraYModeFacts>,
    symbols: &SymbolDecoder<'_>,
) -> DecodeBlockFrontier {
    let tree_type = call.tree_type;
    DecodeBlockFrontier {
        r: call.r,
        c: call.c,
        b_size: sub_size,
        has_chroma: call.has_chroma
            && frame.num_planes > 1
            && tree_type != PartitionTreeType::LumaPart,
        chroma_offset,
        chroma_ref: call.chroma_ref_geometry(),
        tree_type,
        intra_region: call.intra_region,
        stored_luma_y_mode,
        cfl_allowed_in_sdp: call.cfl_allowed_in_sdp,
        symbol_count_before_block: symbols.symbol_count(),
        symbol_checkpoint_before_block: symbols.checkpoint(),
    }
}

pub(crate) fn plan_tile_partition_traversal_cursor<'payload>(
    input: TilePartitionTraversalInput<'_, 'payload, '_>,
) -> Result<TilePartitionTraversalCursor<'payload>, TilePartitionTraversalError> {
    let TilePartitionTraversalInput {
        work_unit,
        frame,
        context,
        limits,
    } = input;
    ensure_supported_traversal_frame(frame, false)?;

    let mut cdfs = work_unit.cdf().tile_cdfs().clone();
    let mut symbols = symbol_decoder_for_work_unit(work_unit)?;
    let mut lr_activity = WienerNsLrUnitActivity::default();
    let mut sdp_state = SdpPartitionState::default();
    let consumed_bits_before = symbols.consumed_bits().get();
    let tile_bounds = TilePartitionBounds::from_work_unit(work_unit);
    let root = root_partition_call(work_unit, frame);
    let mut stack = vec![root];
    let mut steps = Vec::new();
    let mut skipped_out_of_frame = Vec::new();

    while let Some(call) = stack.pop() {
        limits.ensure(
            DecodeLimitName::MaxTilePartitionSteps,
            (steps.len() + 1) as u64,
        )?;
        if !call_in_frame(frame, call) {
            skipped_out_of_frame.push(call);
            continue;
        }
        if is_intra_sdp_shared_root(frame, call) {
            stack.push(call.with_tree_type(PartitionTreeType::ChromaPart));
            stack.push(call.with_tree_type(PartitionTreeType::LumaPart));
            continue;
        }
        read_loop_restoration_for_call(
            frame,
            call,
            tile_bounds,
            &mut cdfs,
            &mut symbols,
            &mut lr_activity,
            limits,
        )?;

        let step = read_frontier_partition_step(
            call,
            frame,
            tile_bounds,
            context,
            &mut sdp_state,
            &mut cdfs,
            &mut symbols,
        )?;
        if step.using_extended_sdp() {
            return Err(TilePartitionTraversalError::Unsupported(
                TilePartitionTraversalUnsupported::ExtendedSdp,
            ));
        }

        let call = step.call;
        let partition = step.partition();
        steps.push(step);
        let sub_size = valid_subsize(partition, call.b_size)?;
        let chroma_offset = updated_chroma_offset(call, partition, sub_size, frame)?;
        if partition == PartitionType::None {
            stack.reverse();
            let plan = TilePartitionTraversalPlan {
                tile_num: work_unit.tile_num(),
                steps,
                skipped_out_of_frame,
                pending_children: stack,
                frontier: decode_block_frontier(
                    call,
                    frame,
                    sub_size,
                    chroma_offset,
                    None,
                    &symbols,
                ),
                consumed_bits_before,
                consumed_bits_after: symbols.consumed_bits().get(),
                symbol_count_after: symbols.symbol_count(),
            };
            *work_unit.cdf_mut().tile_cdfs_mut() = cdfs;
            return Ok(TilePartitionTraversalCursor { plan, symbols });
        }

        let children = child_calls(call, partition, sub_size, frame, chroma_offset)?;
        stack.extend(children.as_slice().iter().rev().copied());
    }

    Err(TilePartitionTraversalError::NoBlockFrontier)
}

pub(crate) struct GeneralIntraPartitionTreeOutput<'payload> {
    pub(crate) symbols: SymbolDecoder<'payload>,
    pub(crate) active_source_blocks: Vec<WienerNsLrSourceBlock>,
    pub(crate) unit_filters: Vec<WienerNsLrUnitFilter>,
}

/// Parser-owned superblock-row boundary for `INFRA-DECODE-PARALLEL-STAGES`.
pub(crate) struct GeneralIntraPartitionTreeCursor<'payload> {
    frame: TilePartitionFrameFacts,
    symbols: SymbolDecoder<'payload>,
    lr_activity: WienerNsLrUnitActivity,
    tile_bounds: TilePartitionBounds,
    y_modes: TileIntraYModeState,
    sb_size4: usize,
    sb_mask: usize,
    next_sb_row: usize,
    next_sb_col: usize,
    mi_row_end: usize,
    mi_col_start: usize,
    mi_col_end: usize,
    limits: DecodeLimits,
    step_count: u64,
    sdp_state: SdpPartitionState,
    stack: Vec<TilePartitionStackEntry>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TilePartitionStackEntry {
    Partition(TilePartitionCall),
    ExtendedSdpChromaBlock(TilePartitionCall),
}

impl<'payload> GeneralIntraPartitionTreeCursor<'payload> {
    pub(crate) fn new(
        work_unit: &DecodeTileWorkUnit<'payload>,
        frame: TilePartitionFrameFacts,
        limits: DecodeLimits,
    ) -> Result<Self, TilePartitionTraversalError> {
        ensure_supported_traversal_frame(frame, false)?;
        let symbols = symbol_decoder_for_work_unit(work_unit)?;
        let lr_activity = WienerNsLrUnitActivity::retaining_source_blocks();
        let tile_bounds = TilePartitionBounds::from_work_unit(work_unit);
        let tile_rows = work_unit.mi_row_range().start as usize
            ..(work_unit.mi_row_range().end as usize).min(frame.mi_rows);
        let tile_cols = work_unit.mi_col_range().start as usize
            ..(work_unit.mi_col_range().end as usize).min(frame.mi_cols);
        let y_modes = TileIntraYModeState::new(tile_rows.len(), tile_cols.len())?
            .with_origin(tile_rows.start, tile_cols.start);
        let sb_size4 = frame.sb_size.num_4x4_wide()?.max(1);
        let next_sb_row = work_unit.mi_row_range().start as usize;
        let mi_row_end = (work_unit.mi_row_range().end as usize).min(frame.mi_rows);
        let mi_col_start = work_unit.mi_col_range().start as usize;
        let mi_col_end = (work_unit.mi_col_range().end as usize).min(frame.mi_cols);
        Ok(Self {
            frame,
            symbols,
            lr_activity,
            tile_bounds,
            y_modes,
            sb_size4,
            sb_mask: sb_size4.saturating_sub(1),
            next_sb_row,
            next_sb_col: mi_col_start,
            mi_row_end,
            mi_col_start,
            mi_col_end,
            limits,
            step_count: 0,
            sdp_state: SdpPartitionState::default(),
            stack: Vec::new(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn decode_next_superblock_with_publication<E, C, F, P>(
        &mut self,
        work_unit: &mut DecodeTileWorkUnit<'payload>,
        mi_size_state: &mut TileMiSizeState,
        joint_modes: &mut TileIntraJointModeState,
        uses_mrls: &mut TileUsesMrlsState,
        use_dip: &mut TileUseDipState,
        fsc_modes: &mut TileFscModeState,
        palette_y: &mut TileLumaPaletteState,
        uv_cfls: &mut TileUvCflState,
        on_leaf: &mut F,
        on_published: &mut P,
    ) -> Result<Option<[usize; 2]>, GeneralIntraTreeWalkError<E>>
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
        if self.next_sb_row >= self.mi_row_end {
            return Ok(None);
        }
        let sb_row = self.next_sb_row;
        let sb_col = self.next_sb_col;
        if sb_col == self.mi_col_start {
            mi_size_state.clear_left_context();
        }
        let root =
            TilePartitionCall::root(sb_row, sb_col, self.frame.sb_size, self.frame.has_chroma);
        self.stack.clear();
        self.stack.push(TilePartitionStackEntry::Partition(root));
        while let Some(entry) = self.stack.pop() {
            let (call, forced_extended_sdp_chroma_block) = match entry {
                TilePartitionStackEntry::Partition(call) => (call, false),
                TilePartitionStackEntry::ExtendedSdpChromaBlock(call) => (call, true),
            };
            self.step_count += 1;
            self.limits
                .ensure(DecodeLimitName::MaxTilePartitionSteps, self.step_count)
                .map_err(TilePartitionTraversalError::from)?;
            if !call_in_frame(self.frame, call) {
                continue;
            }
            if !forced_extended_sdp_chroma_block && is_intra_sdp_shared_root(self.frame, call) {
                self.stack.push(TilePartitionStackEntry::Partition(
                    call.with_tree_type(PartitionTreeType::ChromaPart),
                ));
                self.stack.push(TilePartitionStackEntry::Partition(
                    call.with_tree_type(PartitionTreeType::LumaPart),
                ));
                continue;
            }
            let (call, sub_size, chroma_offset, partition_is_none) =
                if forced_extended_sdp_chroma_block {
                    (call, call.b_size, false, true)
                } else {
                    read_loop_restoration_for_call(
                        self.frame,
                        call,
                        self.tile_bounds,
                        work_unit.cdf_mut().tile_cdfs_mut(),
                        &mut self.symbols,
                        &mut self.lr_activity,
                        self.limits,
                    )?;

                    let step = read_frontier_partition_step(
                        call,
                        self.frame,
                        self.tile_bounds,
                        mi_size_state.context_state(),
                        &mut self.sdp_state,
                        work_unit.cdf_mut().tile_cdfs_mut(),
                        &mut self.symbols,
                    )?;
                    let call = step.call;
                    let partition = step.partition();

                    let sub_size = valid_subsize(partition, call.b_size)?;
                    let chroma_offset =
                        updated_chroma_offset(call, partition, sub_size, self.frame)?;
                    if partition != PartitionType::None {
                        let children =
                            child_calls(call, partition, sub_size, self.frame, chroma_offset)?;
                        if step.using_extended_sdp() {
                            self.stack
                                .push(TilePartitionStackEntry::ExtendedSdpChromaBlock(
                                    extended_sdp_chroma_call(self.frame, call),
                                ));
                        }
                        self.stack.extend(
                            children
                                .as_slice()
                                .iter()
                                .rev()
                                .copied()
                                .map(TilePartitionStackEntry::Partition),
                        );
                    }
                    (
                        call,
                        sub_size,
                        chroma_offset,
                        partition == PartitionType::None,
                    )
                };
            if partition_is_none {
                let stored_luma_y_mode = if call.tree_type == PartitionTreeType::ChromaPart {
                    self.y_modes.y_mode_facts_at(call.r, call.c)
                } else {
                    None
                };
                let frontier = decode_block_frontier(
                    call,
                    self.frame,
                    sub_size,
                    chroma_offset,
                    stored_luma_y_mode,
                    &self.symbols,
                );
                let is_cfl_ctx = is_cfl_context_for_chroma_ref(
                    uv_cfls,
                    self.tile_bounds,
                    frontier.chroma_ref_geometry(),
                );
                let decoded_leaf = DecodedLeafPublication::new(call, sub_size, self.sb_mask);
                let (leaf_mode, command) = on_leaf(
                    work_unit,
                    &mut self.symbols,
                    &frontier,
                    joint_modes,
                    uses_mrls,
                    use_dip,
                    fsc_modes,
                    palette_y,
                    is_cfl_ctx,
                    decoded_leaf,
                )
                .map_err(GeneralIntraTreeWalkError::Leaf)?;
                publish_intra_leaf_state(
                    self.frame,
                    call,
                    &frontier,
                    leaf_mode,
                    sub_size,
                    joint_modes,
                    fsc_modes,
                    use_dip,
                    uses_mrls,
                    palette_y,
                    uv_cfls,
                    &mut self.y_modes,
                    mi_size_state,
                )?;
                on_published(decoded_leaf, command);
            }
        }
        self.next_sb_col = self.next_sb_col.saturating_add(self.sb_size4);
        if self.next_sb_col >= self.mi_col_end {
            self.next_sb_col = self.mi_col_start;
            self.next_sb_row = self.next_sb_row.saturating_add(self.sb_size4);
        }
        Ok(Some([sb_row, sb_col]))
    }

    pub(crate) fn into_output(self) -> GeneralIntraPartitionTreeOutput<'payload> {
        let Self {
            symbols,
            lr_activity,
            y_modes,
            ..
        } = self;
        y_modes.recycle();
        GeneralIntraPartitionTreeOutput {
            symbols,
            active_source_blocks: lr_activity.active_source_blocks,
            unit_filters: lr_activity.unit_filters,
        }
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

fn read_frontier_partition_step(
    call: TilePartitionCall,
    frame: TilePartitionFrameFacts,
    tile_bounds: TilePartitionBounds,
    context: TilePartitionContextState<'_>,
    sdp_state: &mut SdpPartitionState,
    cdfs: &mut super::cdf::TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
) -> Result<TilePartitionFrontierStep, TilePartitionTraversalError> {
    let symbol_count_before = symbols.symbol_count();
    let forced_chroma_partition = sdp_state.forced_chroma_partition(frame, call);
    let decision = read_frontier_partition_decision(
        call,
        frame,
        tile_bounds,
        context,
        forced_chroma_partition,
        cdfs,
        symbols,
    )?;
    let symbol_count_after = symbols.symbol_count();
    let partition = decision.partition;
    let call = call.with_cfl_allowed_in_sdp(sdp_state.record_partition(frame, call, partition));
    let (call, using_extended_sdp) =
        read_extended_sdp_region_type(frame, call, partition, cdfs, symbols)?;
    Ok(TilePartitionFrontierStep {
        call,
        decision,
        symbol_count_before,
        symbol_count_after,
        using_extended_sdp,
    })
}

pub(super) fn read_frontier_partition_decision(
    call: TilePartitionCall,
    frame: TilePartitionFrameFacts,
    tile_bounds: TilePartitionBounds,
    context: TilePartitionContextState<'_>,
    forced_chroma_partition: Option<PartitionType>,
    cdfs: &mut super::cdf::TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
) -> Result<ReadPartitionDecision, TilePartitionTraversalError> {
    let mixed_region = !frame.frame_is_intra && call.parent_size.is_some() && !call.intra_region;
    let allowed = PartitionAllowedInput::new(
        call.r,
        call.c,
        frame.mi_rows,
        frame.mi_cols,
        call.b_size.index(),
        call.tree_type,
        frame.subsampling_x,
        frame.subsampling_y,
        frame.features,
        frame.frame_is_intra,
        mixed_region,
        frame.max_pb_aspect_ratio,
        call.has_chroma,
        call.chroma_offset,
        frame.num_planes,
        forced_chroma_partition,
    )?;
    let facts = partition_decision_facts(allowed)?;
    let partition_plane = partition_cdf_plane(call.tree_type);
    let local_r = call.r.checked_sub(context.origin_row).ok_or(
        TilePartitionTraversalError::CoordinateUnderflow {
            coordinate: "tile-local partition row",
            base: call.r,
            offset: context.origin_row,
        },
    )?;
    let local_c = call.c.checked_sub(context.origin_col).ok_or(
        TilePartitionTraversalError::CoordinateUnderflow {
            coordinate: "tile-local partition column",
            base: call.c,
            offset: context.origin_col,
        },
    )?;
    let partition_context = PartitionContextInput::new(
        call.b_size.index(),
        partition_plane,
        local_r,
        local_c,
        context.left_mi_sizes,
        context.above_mi_sizes,
    )?;
    let avail_u = tile_bounds.avail_u(call);
    let avail_l = tile_bounds.avail_l(call);
    let square_context = SquareSplitContextInput::new(
        call.b_size.index(),
        0,
        local_r,
        local_c,
        avail_u,
        avail_l,
        context.mi_sizes,
        context.mi_size_stride,
    )?;
    let decision_input =
        facts.read_partition_decision_input(true, partition_context, square_context);
    let decision = super::partition::read_partition_decision(decision_input, cdfs, symbols)?;
    Ok(decision)
}

pub(super) const fn partition_cdf_plane(tree_type: PartitionTreeType) -> usize {
    matches!(tree_type, PartitionTreeType::ChromaPart) as usize
}

pub(super) fn is_intra_sdp_shared_root(
    frame: TilePartitionFrameFacts,
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

    use splot_core::symbol::CdfUpdateMode;

    use super::*;
    use crate::bitstream::tile_payload::make_test_work_unit;

    type ParserStates = (
        TileMiSizeState,
        TileIntraJointModeState,
        TileUsesMrlsState,
        TileUseDipState,
        TileFscModeState,
        TileLumaPaletteState,
        TileUvCflState,
    );

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
            true,
            super::super::TilePartitionBruState::Active,
        )
        .unwrap()
    }

    fn parser_states(frame: TilePartitionFrameFacts) -> ParserStates {
        let sb_size4 = frame.sb_size.num_4x4_wide().unwrap();
        (
            TileMiSizeState::new(frame.mi_rows, frame.mi_cols, frame.sb_size).unwrap(),
            TileIntraJointModeState::new(frame.mi_rows, frame.mi_cols).unwrap(),
            TileUsesMrlsState::new(frame.mi_rows, frame.mi_cols, sb_size4).unwrap(),
            TileUseDipState::new(frame.mi_rows, frame.mi_cols, sb_size4).unwrap(),
            TileFscModeState::new(frame.mi_rows, frame.mi_cols, sb_size4).unwrap(),
            TileLumaPaletteState::new(frame.mi_rows, frame.mi_cols, sb_size4).unwrap(),
            TileUvCflState::new(frame.mi_rows, frame.mi_cols).unwrap(),
        )
    }

    fn leaf_mode() -> GeneralIntraLeafMode {
        GeneralIntraLeafMode::luma(0, super::super::IntraYMode::DC_PRED, 0, 0, 0).with_uv_cfl(false)
    }

    #[test]
    fn cursor_yields_each_superblock_in_raster_order() {
        let payload = [0_u8; 4096];
        let frame = frame();
        let mut work = make_test_work_unit(&payload, CdfUpdateMode::Enabled);
        let mut states = parser_states(frame);
        let mut superblocks = Vec::new();
        let mut cursor =
            GeneralIntraPartitionTreeCursor::new(&work, frame, DecodeLimits::DEFAULT).unwrap();
        loop {
            let superblock = cursor
                .decode_next_superblock_with_publication(
                    &mut work,
                    &mut states.0,
                    &mut states.1,
                    &mut states.2,
                    &mut states.3,
                    &mut states.4,
                    &mut states.5,
                    &mut states.6,
                    &mut |_, _, _, _, _, _, _, _, _, _| Ok::<_, ()>((leaf_mode(), ())),
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
    }

    #[test]
    fn publication_hook_keeps_prefix_before_later_leaf_error() {
        let payload = [0_u8; 4096];
        let frame = frame();
        let mut work = make_test_work_unit(&payload, CdfUpdateMode::Enabled);
        let mut states = parser_states(frame);
        let mut cursor =
            GeneralIntraPartitionTreeCursor::new(&work, frame, DecodeLimits::DEFAULT).unwrap();
        let mut calls = 0;
        let mut published = Vec::new();

        let result = loop {
            let result = cursor.decode_next_superblock_with_publication(
                &mut work,
                &mut states.0,
                &mut states.1,
                &mut states.2,
                &mut states.3,
                &mut states.4,
                &mut states.5,
                &mut states.6,
                &mut |_, _, _, _, _, _, _, _, _, _| {
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
}

pub(crate) fn extended_sdp_allowed_for_child(
    frame: TilePartitionFrameFacts,
    call: TilePartitionCall,
    partition: PartitionType,
    sub_size: BlockSize,
) -> bool {
    if !frame.enable_extended_sdp || !call.extended_sdp_allowed {
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
    let Some(middle_size) = middle_size.valid() else {
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

pub(super) fn read_extended_sdp_region_type(
    frame: TilePartitionFrameFacts,
    call: TilePartitionCall,
    partition: PartitionType,
    cdfs: &mut super::cdf::TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
) -> Result<(TilePartitionCall, bool), TilePartitionTraversalError> {
    if !should_read_extended_sdp_region_type(frame, call, partition)? {
        return Ok((call, false));
    }
    let ctx = intra_region_context(call.b_size)?;
    let selector = super::cdf::TileCdfSelector::RegionType { ctx };
    let region_type = cdfs
        .with_row_mut(selector, |row| symbols.read_symbol_u16(row))??
        .get();
    match region_type {
        INTRA_REGION => Ok((
            call.with_tree_type(PartitionTreeType::LumaPart)
                .with_intra_region(true),
            true,
        )),
        MIXED_REGION => Ok((call.with_intra_region(false), false)),
        value => Err(TilePartitionTraversalError::InvalidRegionType { value }),
    }
}

pub(super) fn should_read_extended_sdp_region_type(
    frame: TilePartitionFrameFacts,
    call: TilePartitionCall,
    partition: PartitionType,
) -> Result<bool, TilePartitionTraversalError> {
    if frame.frame_is_intra
        || !frame.enable_extended_sdp
        || call.tree_type != PartitionTreeType::Shared
        || call.intra_region
        || !call.extended_sdp_allowed
        || call.b_size == frame.sb_size
        || frame.bru_state != TilePartitionBruState::Active
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

fn extended_sdp_chroma_call(
    frame: TilePartitionFrameFacts,
    call: TilePartitionCall,
) -> TilePartitionCall {
    TilePartitionCall::child(
        call.r,
        call.c,
        call.b_size,
        call.parent_size,
        false,
        frame.has_chroma,
        PartitionTreeType::ChromaPart,
        Some(ChromaRefGeometry::new(call.r, call.c, call.b_size)),
        false,
        true,
    )
    .with_cfl_allowed_in_sdp(true)
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct SdpPartitionState {
    top_luma_horz: bool,
    top_luma_vert: bool,
    top_luma_uneven_horz: bool,
    top_luma_uneven_vert: bool,
    chroma_follows_luma: bool,
    luma_64x64_partition: Option<PartitionType>,
}

impl SdpPartitionState {
    pub(super) fn record_partition(
        &mut self,
        frame: TilePartitionFrameFacts,
        call: TilePartitionCall,
        partition: PartitionType,
    ) -> bool {
        if !frame.frame_is_intra {
            return call.cfl_allowed_in_sdp;
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

        let mut cfl_allowed_in_sdp = call.cfl_allowed_in_sdp;
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

    pub(super) fn forced_chroma_partition(
        &self,
        frame: TilePartitionFrameFacts,
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

pub(crate) fn valid_subsize(
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

// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Tile-local block decode and reconstruction.
//!
//! Feature tracking: `INFRA-DECODE-PARALLEL-STAGES`.

use std::num::NonZeroUsize;
use std::ops::Range;

use splot_recon::{PlaneId, ReconError};

use super::*;

mod admission;
mod mvres;

pub(crate) use admission::ScheduledFrameProgress;
use admission::TileCommit;
pub(super) use admission::prepare_scheduled_tile;
pub(crate) use admission::{ScheduledTileRecon, ScheduledTileWorkspace};

enum ParserStep<Row> {
    More(Row),
    Last(Row),
}

use super::super::MotionFieldHandle;
use super::temporal::MotionFieldUnits;
use crate::prediction::TileGridConstructionError;
use parking_lot::Mutex;

pub(super) struct TileDecodeOutput {
    pub(super) cdef_state: CdefState,
    pub(super) gdf_state: GdfState,
    pub(super) ccso_state: CcsoState,
    pub(super) motion_field: TemporalMotionField,
}

/// Folds one tile's walk-parsed filter grids into the frame-level state.
#[allow(clippy::too_many_arguments)]
fn merge_tile_filter_state(
    cdef_state: &mut CdefState,
    gdf_state: &mut GdfState,
    ccso_state: &mut CcsoState,
    segment_ids: Option<&mut FrameSegmentIdMap>,
    tile: &TileParserOutput,
    mi_rows: Range<usize>,
    mi_cols: Range<usize>,
) -> Result<()> {
    cdef_state.merge_tile(&tile.cdef_state, mi_rows.clone(), mi_cols.clone())?;
    gdf_state.merge_tile(&tile.gdf_state, mi_rows.clone(), mi_cols.clone())?;
    ccso_state.merge_tile(&tile.ccso_state, mi_rows, mi_cols)?;
    if let Some(segment_ids) = segment_ids {
        segment_ids.merge_tile(&tile.segment_id_state);
    }
    Ok(())
}

fn append_lr_records(
    blocks: &mut Vec<crate::bitstream::tile_payload::WienerNsLrSourceBlock>,
    filters: &mut Vec<crate::bitstream::tile_payload::WienerNsLrUnitFilter>,
    tile_blocks: &mut Vec<crate::bitstream::tile_payload::WienerNsLrSourceBlock>,
    tile_filters: &mut Vec<crate::bitstream::tile_payload::WienerNsLrUnitFilter>,
) -> Result<()> {
    let filter_base = filters.len();
    for block in tile_blocks.iter() {
        if let Some(index) = block.unit_filter_index
            && index >= tile_filters.len()
        {
            return Err(crate::DecodeHeaderStateError::InvalidLoopRestorationFilterState.into());
        }
    }

    blocks
        .try_reserve_exact(tile_blocks.len())
        .map_err(|_| inter_allocation!("inter LR source-block records"))?;
    filters
        .try_reserve_exact(tile_filters.len())
        .map_err(|_| inter_allocation!("inter LR unit-filter records"))?;

    for block in tile_blocks.iter_mut() {
        if let Some(index) = block.unit_filter_index {
            block.unit_filter_index = Some(
                filter_base
                    .checked_add(index)
                    .ok_or(crate::DecodeHeaderStateError::InvalidLoopRestorationFilterState)?,
            );
        }
    }
    blocks.append(tile_blocks);
    filters.append(tile_filters);
    Ok(())
}

#[derive(Default)]
pub(super) struct TileFilterRecords {
    pub(super) deblock_blocks: Vec<crate::filters::deblock::DeblockBlock>,
    pub(super) chroma_deblock_blocks: crate::filters::deblock::ChromaDeblockRecords,
    pub(super) tx_skip_records: Vec<crate::filters::wienerns_lr::WienerNsLrTxSkipTransformRecord>,
}

/// The frame-level facts every tile phase reads, all owned so a resolve pass
/// that runs after the driver moved on can rebuild its tile context.
#[derive(Clone, Copy)]
pub(crate) struct TileWalkParams {
    pub(super) limits: crate::DecodeLimits,
    pub(super) mi_rows: usize,
    pub(super) mi_cols: usize,
    pub(super) sb_h4: usize,
    pub(super) max_drl_bits_minus_1: u32,
    pub(super) frame_interpolation_filter: FrameInterpolationFilter,
    pub(super) residual_tool_policy: TransformToolResidualPolicy,
    pub(super) num_total_refs: usize,
    pub(super) reference_select: bool,
    pub(super) num_same_ref_compound: u8,
    pub(super) luma_use_tcq: bool,
    pub(super) residual_use_ddt: bool,
    pub(super) bit_depth: BitDepth,
    pub(super) enable_adaptive_mvd: bool,
    pub(super) allow_bawp: bool,
    pub(super) allow_warpmv_mode: bool,
    pub(super) frame_is_switch: bool,
    pub(super) current_order_hint: u32,
    /// AV2 § 7.12.2 TIP reference pair, derived from the reference order hints
    /// alone so the entropy pass never reads the projected temporal field.
    pub(super) tip_ref_pair: Option<(i8, i8)>,
}

impl TileWalkParams {
    pub(super) fn context<'a, T: ReconSample>(
        &self,
        sequence: &'a SequenceHeader,
        core: &'a FrameHeaderCore,
        reference: &'a InterReferenceState<T>,
        ref_frame_idx: &'a [u32],
    ) -> TileDecodeContext<'a, T> {
        TileDecodeContext {
            sequence,
            core,
            reference,
            ref_frame_idx,
            params: *self,
        }
    }
}

pub(super) struct TileDecodeContext<'a, T: ReconSample> {
    sequence: &'a SequenceHeader,
    core: &'a FrameHeaderCore,
    reference: &'a InterReferenceState<T>,
    ref_frame_idx: &'a [u32],
    params: TileWalkParams,
}

pub(super) struct TileParser<'payload> {
    walk: TileParserWalk<GeneralIntraMultiblockCursor<'payload>>,
    coeff_ctx: TileCoeffContextState,
    residual_scratch: InterResidualParseScratch,
    delta_q_state: DeltaQState,
    intrabc_state: TileIntrabcPreludeState,
    mv_grid: NeighbourMvGrid,
    y_smooth: crate::prediction::intra_edge::TileYSmoothGrid,
    chroma_smooth: crate::prediction::intra_edge::TileChromaSmoothGrid,
    filter_records: TileFilterRecords,
    /// The planes this row's general-intra blocks have parsed.
    residual_planes: crate::residual::pipeline::ResidualPlaneArena,
    output: TileParserOutput,
    parser_ordinal: usize,
}

enum TileParserWalk<T> {
    Active(T),
    Finished,
}

impl<T> TileParserWalk<T> {
    fn active_mut(&mut self) -> Result<&mut T> {
        match self {
            Self::Active(walk) => Ok(walk),
            Self::Finished => {
                Err(crate::DecodeHeaderStateError::InvalidInterTileTraversalState.into())
            }
        }
    }

    fn finish(&mut self) -> Result<T> {
        match core::mem::replace(self, Self::Finished) {
            Self::Active(walk) => Ok(walk),
            Self::Finished => {
                Err(crate::DecodeHeaderStateError::InvalidInterTileTraversalState.into())
            }
        }
    }
}

struct TileParserOutput {
    cdef_state: CdefState,
    gdf_state: GdfState,
    ccso_state: CcsoState,
    segment_id_state: TileSegmentIdState,
    traversal: crate::bitstream::tile_payload::TileTraversalStorage,
}

fn inter_tile_coeff_context_error(error: &TileCoeffStateError) -> crate::DecodeError {
    match error {
        TileCoeffStateError::Allocation(_) => {
            inter_allocation!("inter coefficient context state")
        }
        TileCoeffStateError::EmptyTileDimensions { .. }
        | TileCoeffStateError::InvalidAdjustedTransformExtent { .. }
        | TileCoeffStateError::ArithmeticOverflow { .. }
        | TileCoeffStateError::InvalidPlane { .. }
        | TileCoeffStateError::InvalidDcCategory { .. }
        | TileCoeffStateError::EmptyContextRange { .. }
        | TileCoeffStateError::CoordinateOverflow { .. }
        | TileCoeffStateError::ContextRangeOutOfBounds { .. }
        | TileCoeffStateError::TransformCoordinateOutOfBounds { .. }
        | TileCoeffStateError::QuantPositionOutOfBounds { .. }
        | TileCoeffStateError::InvalidSubsampling { .. } => {
            crate::DecodeHeaderStateError::InvalidInterTileConstructionState.into()
        }
    }
}

fn inter_tile_segment_id_error(error: &TileSegmentIdStateError) -> crate::DecodeError {
    match error {
        TileSegmentIdStateError::Allocation { .. } => {
            inter_allocation!("inter segment id state")
        }
        TileSegmentIdStateError::EmptyDimensions { .. }
        | TileSegmentIdStateError::ArithmeticOverflow { .. } => {
            crate::DecodeHeaderStateError::InvalidInterTileConstructionState.into()
        }
    }
}

fn inter_tile_block_decoded_error(error: &TileBlockDecodedStateError) -> crate::DecodeError {
    match error {
        TileBlockDecodedStateError::Allocation { .. } => {
            inter_allocation!("inter block decoded state")
        }
        TileBlockDecodedStateError::InvalidPlanes { .. }
        | TileBlockDecodedStateError::EmptySuperblock
        | TileBlockDecodedStateError::InvalidSubsampling { .. }
        | TileBlockDecodedStateError::Overflow => {
            crate::DecodeHeaderStateError::InvalidInterTileConstructionState.into()
        }
    }
}

fn inter_tile_grid_error(
    error: &TileGridConstructionError,
    allocation_context: &'static str,
) -> crate::DecodeError {
    match error {
        TileGridConstructionError::Allocation => {
            splot_recon::ReconError::WorkspaceAllocationFailed {
                plane: PlaneId::Y,
                context: allocation_context,
            }
            .into()
        }
        TileGridConstructionError::EmptyDimensions
        | TileGridConstructionError::ReversedDimensions
        | TileGridConstructionError::AreaOverflow => {
            crate::DecodeHeaderStateError::InvalidInterTileConstructionState.into()
        }
    }
}

impl<'payload> TileParser<'payload> {
    fn new<T: ReconSample>(
        tile: &mut DecodeTileWorkUnit,
        tile_bytes: &'payload [u8],
        context: &TileDecodeContext<'_, T>,
        cdef_state: CdefState,
        gdf_state: GdfState,
        ccso_state: CcsoState,
        mut parse: TileParseState,
    ) -> Result<Self> {
        let tile_offset = tile.tile_byte_span().start;
        let chroma = context.sequence.general.chroma_format_idc;
        let tile_rows = tile.mi_row_range().start as usize
            ..(tile.mi_row_range().end as usize).min(context.params.mi_rows);
        let tile_cols = tile.mi_col_range().start as usize
            ..(tile.mi_col_range().end as usize).min(context.params.mi_cols);
        parse
            .coeff_ctx
            .reset_for_tile_chroma(tile_rows.clone(), tile_cols.clone(), chroma)
            .map_err(|error| inter_tile_coeff_context_error(&error))?;
        let delta_q_state = DeltaQState::new(context.sequence, context.core)?;
        parse.intrabc_state.reset_for_tile(
            (context.params.mi_rows, context.params.mi_cols),
            tile_rows.clone(),
            tile_cols.clone(),
            context.sequence,
            context.core.frame_is_intra == Some(true),
            crate::filters::wienerns_lr::intrabc_records::frame_allows_intrabc(context.core),
        )?;
        let segment_id_state =
            TileSegmentIdState::new_for_tile(tile_rows.clone(), tile_cols.clone())
                .map_err(|error| inter_tile_segment_id_error(&error))?;
        parse
            .mv_grid
            .reset_for_tile(tile_rows.clone(), tile_cols.clone())
            .map_err(|error| inter_tile_grid_error(&error, "inter parser MV grid"))?;
        parse
            .y_smooth
            .reset_for_tile(tile_rows.clone(), tile_cols.clone())
            .map_err(|error| inter_tile_grid_error(&error, "inter luma smooth grid"))?;
        let (chroma_rows, chroma_cols) =
            super::chroma_smooth_tile_ranges(tile_rows, tile_cols, chroma);
        parse
            .chroma_smooth
            .reset_for_tile(chroma_rows, chroma_cols)
            .map_err(|error| inter_tile_grid_error(&error, "inter chroma smooth grid"))?;
        let walk = GeneralIntraMultiblockCursor::new(
            tile,
            tile_bytes,
            context.sequence,
            context.core,
            context.params.limits,
            core::mem::take(&mut parse.traversal),
        )
        .map_err(|error| {
            map_inter_multiblock_error(
                GeneralIntraMultiblockError::<crate::DecodeError>::Setup(error),
                tile_offset,
            )
        })?;
        Ok(Self {
            walk: TileParserWalk::Active(walk),
            coeff_ctx: parse.coeff_ctx,
            residual_scratch: parse.residual_scratch,
            delta_q_state,
            intrabc_state: parse.intrabc_state,
            mv_grid: parse.mv_grid,
            y_smooth: parse.y_smooth,
            chroma_smooth: parse.chroma_smooth,
            filter_records: parse.filter_records,
            residual_planes: crate::residual::pipeline::ResidualPlaneArena::new(),
            output: TileParserOutput {
                cdef_state,
                gdf_state,
                ccso_state,
                segment_id_state,
                traversal: crate::bitstream::tile_payload::TileTraversalStorage::default(),
            },
            parser_ordinal: 0,
        })
    }

    fn next_unit<T: ReconSample>(
        &mut self,
        tile: &mut DecodeTileWorkUnit,
        context: &TileDecodeContext<'_, T>,
        buffers: Option<ReconRowBuffers>,
    ) -> ParserStep<ReconRow> {
        let tile_offset = tile.tile_byte_span().start;
        let mut buffers = buffers.unwrap_or_default();
        let reservation = superblock_coefficient_capacity(
            context.params.sb_h4,
            context.sequence.general.chroma_format_idc,
        )
        .and_then(|capacity| buffers.reserve_coefficients(capacity));
        let ReconRowBuffers {
            superblocks,
            residual_coeffs,
            entries,
            residual_blocks,
            temporal,
            motion_grids,
            motion_storage,
            mut flag_log,
            filter_records,
            residual_planes,
        } = buffers;
        self.mv_grid.take_flag_log(&mut flag_log);
        self.filter_records = filter_records;
        self.residual_planes = residual_planes;
        let mut recon_row = ReconRow {
            ordinal: self.parser_ordinal,
            residual_source: None,
            superblocks,
            residual_coeffs,
            entries,
            residual_blocks,
            temporal,
            motion_grids,
            motion_storage,
            flag_log,
            filter_records: TileFilterRecords::default(),
            residual_planes: crate::residual::pipeline::ResidualPlaneArena::new(),
            motion_folded: false,
            motion_derived: false,
            failure: ReconRowFailure::None,
        };
        if let Err(error) = reservation {
            recon_row.record_terminal_error(error);
            return ParserStep::Last(recon_row);
        }
        self.parser_ordinal = self.parser_ordinal.saturating_add(1);
        let walk = match self.walk.active_mut() {
            Ok(walk) => walk,
            Err(error) => {
                recon_row.record_terminal_error(error);
                return ParserStep::Last(recon_row);
            }
        };
        let decoded_row = {
            let mut decode_leaf =
                |work_unit: &mut DecodeTileWorkUnit,
                 symbols: &mut SymbolDecoder<'_>,
                 frontier: &DecodeBlockFrontier,
                 joint_modes: &TileIntraJointModeState,
                 uses_mrls: &TileUsesMrlsState,
                 use_dip: &crate::bitstream::tile_payload::TileUseDipState,
                 fsc_modes: &TileFscModeState,
                 palette_state: &crate::bitstream::tile_payload::TileLumaPaletteState,
                 is_cfl_ctx: IsCflContext| {
                    decode_block(
                        work_unit,
                        symbols,
                        frontier,
                        context.sequence,
                        context.core,
                        &mut self.coeff_ctx,
                        &mut self.residual_scratch,
                        &mut recon_row.residual_blocks,
                        &mut recon_row.residual_coeffs,
                        &mut self.output.gdf_state,
                        &mut self.output.cdef_state,
                        &mut self.output.ccso_state,
                        &mut self.delta_q_state,
                        &mut self.intrabc_state,
                        &mut self.output.segment_id_state,
                        &mut self.mv_grid,
                        context.params.tip_ref_pair,
                        &mut self.y_smooth,
                        &mut self.chroma_smooth,
                        context.params.sb_h4,
                        context.params.mi_rows,
                        context.params.mi_cols,
                        context.params.max_drl_bits_minus_1,
                        context.params.frame_interpolation_filter,
                        context.params.residual_tool_policy,
                        context.params.num_total_refs,
                        context.params.reference_select,
                        context.params.num_same_ref_compound,
                        joint_modes,
                        uses_mrls,
                        use_dip,
                        fsc_modes,
                        palette_state,
                        is_cfl_ctx,
                        &mut self.filter_records.deblock_blocks,
                        &mut self.filter_records.chroma_deblock_blocks,
                        &mut self.filter_records.tx_skip_records,
                        &mut self.residual_planes,
                        context.params.luma_use_tcq,
                        context.params.residual_use_ddt,
                        context.ref_frame_idx,
                        context.reference,
                        context.params.bit_depth,
                        context.params.enable_adaptive_mvd,
                        context.params.allow_bawp,
                        context.params.allow_warpmv_mode,
                        context.params.frame_is_switch,
                        context.params.current_order_hint,
                        tile_offset,
                    )
                };
            let mut on_published = |publication: DecodedLeafPublication,
                                    resolve: LeafResolveRecord| {
                let origin = publication.superblock_origin();
                push_recon_entry(
                    &mut recon_row.superblocks,
                    &mut recon_row.entries,
                    origin,
                    ReconRowEntry {
                        publication,
                        state: Some(ReconEntryState::Resolve(resolve)),
                        motion: None,
                        temporal: 0..0,
                    },
                );
            };
            walk.decode_next_superblock(tile, &mut decode_leaf, &mut on_published)
                .map(|superblock| superblock.is_some())
        };
        recon_row.filter_records = core::mem::take(&mut self.filter_records);
        recon_row.residual_planes = core::mem::take(&mut self.residual_planes);
        self.mv_grid.take_flag_log(&mut recon_row.flag_log);
        match decoded_row {
            Ok(true) => ParserStep::More(recon_row),
            Err(error) => {
                recon_row.record_terminal_error(map_inter_multiblock_error(error, tile_offset));
                ParserStep::Last(recon_row)
            }
            Ok(false) => {
                let walk = match self.walk.finish() {
                    Ok(walk) => walk,
                    Err(error) => {
                        recon_row.record_terminal_error(error);
                        return ParserStep::Last(recon_row);
                    }
                };
                let crate::bitstream::tile_payload::GeneralIntraMultiblockOutput {
                    symbols,
                    storage,
                } = walk.into_output();
                if let Err(error) = finish_tile_symbols(symbols, tile_offset) {
                    recon_row.record_terminal_error(error);
                }
                self.output.traversal = storage;
                ParserStep::Last(recon_row)
            }
        }
    }

    fn into_output(self) -> (TileParserOutput, TileParseState) {
        (
            self.output,
            TileParseState {
                filter_records: self.filter_records,
                mv_grid: self.mv_grid,
                coeff_ctx: self.coeff_ctx,
                residual_scratch: self.residual_scratch,
                intrabc_state: self.intrabc_state,
                y_smooth: self.y_smooth,
                chroma_smooth: self.chroma_smooth,
                row_buffers: ReconRowBufferPool::default(),
                traversal: crate::bitstream::tile_payload::TileTraversalStorage::default(),
                block_decoded: TileBlockDecodedState::default(),
                commit_block_decoded: TileBlockDecodedState::default(),
            },
        )
    }
}

fn finish_tile_symbols(symbols: SymbolDecoder<'_>, tile_offset: ByteOffset) -> Result<()> {
    symbols
        .exit_symbol()
        .map(|_| ())
        .map_err(|error| crate::pipeline::malformed_tile_payload(tile_offset, "8.2.4", error))
}

/// The § 7.12 banks one tile's resolve pass owns, alongside whichever
/// neighbour grid the pass publishes its motion plane into.
struct TileResolveState {
    ref_mv_bank: Option<super::super::find_mv_stack::RefMvBank>,
    warp_param_bank: super::super::find_mv_stack::WarpParamBank,
}

impl TileResolveState {
    fn new(sequence: &SequenceHeader) -> Self {
        Self {
            ref_mv_bank: sequence
                .inter
                .as_ref()
                .is_some_and(|inter| inter.enable_refmvbank)
                .then(super::super::find_mv_stack::RefMvBank::new),
            warp_param_bank: super::super::find_mv_stack::WarpParamBank::new(),
        }
    }

    /// Replays one parsed unit's § 7.12 work, in the leaf order the fused walk
    /// used, and completes the inter leaves' reconstruction commands.
    ///
    /// Callers skip this pass for a unit carrying a terminal parser error.
    fn resolve_unit<T: ReconSample>(
        &mut self,
        grid: &mut NeighbourMvGrid,
        context: &TileDecodeContext<'_, T>,
        temporal_context: &TemporalMvContext,
        row: &mut ReconRow,
        tile_offset: ByteOffset,
    ) -> Result<()> {
        resolve_parsed_leaves(
            &mut row.entries,
            &mut MvResolutionState {
                grid,
                ref_mv_bank: &mut self.ref_mv_bank,
                warp_param_bank: &mut self.warp_param_bank,
                core: context.core,
                temporal: frame_uses_temporal_mvs(context.core).then_some(temporal_context),
                order_hints: temporal_context.order_hint_mv_context(),
                drl_reorder: sequence_drl_reorder(context.sequence),
                max_drl_bits_minus_1: context.params.max_drl_bits_minus_1,
                frame_precision: 0,
                tile_offset,
            },
            context.params.sb_h4,
        )
    }
}

/// Runs one parse unit's resolve pass unless parsing has already failed.
fn resolve_parser_step(
    step: ParserStep<ReconRow>,
    resolve: impl FnOnce(&mut ReconRow) -> Result<()>,
) -> ParserStep<ReconRow> {
    let (mut row, last) = match step {
        ParserStep::More(row) => (row, false),
        ParserStep::Last(row) => (row, true),
    };
    if !row.has_terminal_error()
        && let Err(error) = resolve(&mut row)
    {
        row.record_terminal_error(error);
        return ParserStep::Last(row);
    }
    if last {
        ParserStep::Last(row)
    } else {
        ParserStep::More(row)
    }
}

pub(super) struct ReconRowEntry {
    pub(super) publication: DecodedLeafPublication,
    state: Option<ReconEntryState>,
    /// The refinement grid the motion pass derived, which is the only grid the
    /// entry's prediction may sample through.
    motion: Option<NonZeroUsize>,
    pub(super) temporal: Range<usize>,
}

enum ReconEntryState {
    Resolve(LeafResolveRecord),
    Command(ReconCommand),
}

impl ReconRowEntry {
    pub(super) fn command(&self) -> Option<&ReconCommand> {
        match self.state.as_ref()? {
            ReconEntryState::Command(command)
            | ReconEntryState::Resolve(
                LeafResolveRecord::Reseed(command) | LeafResolveRecord::NonInter { command, .. },
            ) => Some(command),
            ReconEntryState::Resolve(
                LeafResolveRecord::Inter(_) | LeafResolveRecord::Intrabc(_),
            ) => None,
        }
    }

    pub(super) fn take_resolve(&mut self) -> Option<LeafResolveRecord> {
        match self.state.take()? {
            ReconEntryState::Resolve(resolve) => Some(resolve),
            state @ ReconEntryState::Command(_) => {
                self.state = Some(state);
                None
            }
        }
    }

    pub(super) fn store_command(&mut self, command: ReconCommand) {
        self.state = Some(ReconEntryState::Command(command));
    }

    pub(super) fn take_command(&mut self) -> Option<ReconCommand> {
        match self.state.take()? {
            ReconEntryState::Command(command) => Some(command),
            state @ ReconEntryState::Resolve(_) => {
                self.state = Some(state);
                None
            }
        }
    }

    /// The § 7.22 record a non-inter luma-tree leaf stores: every covered 8x8
    /// cell is reset to "no reference", clearing earlier inter writes there.
    pub(super) fn temporal_clear_record(
        &self,
        mi_rows: usize,
        mi_cols: usize,
        current_order_hint: u32,
    ) -> Option<TemporalMotionBlock> {
        if !matches!(
            self.command(),
            Some(ReconCommand::GeneralIntra(_) | ReconCommand::Intrabc(_))
        ) {
            return None;
        }
        let (mi_row, mi_col, n4w, n4h) = self.publication.luma_tree_block()?;
        Some(TemporalMotionBlock::new(
            mi_row,
            mi_col,
            n4w,
            n4h,
            mi_rows,
            mi_cols,
            current_order_hint,
            [None, None],
            [Mv::ZERO; 2],
            [None, None],
        ))
    }

    fn store_motion(
        &mut self,
        grid: Option<super::super::mc::StoredMotionGrid>,
        grids: &mut Vec<Option<super::super::mc::StoredMotionGrid>>,
    ) {
        self.motion = grid.and_then(|grid| {
            grids.push(Some(grid));
            NonZeroUsize::new(grids.len())
        });
    }

    pub(super) fn take_motion(
        &mut self,
        grids: &mut [Option<super::super::mc::StoredMotionGrid>],
        storage: Option<&std::sync::Arc<super::super::mc::MotionRowStorage>>,
    ) -> Result<Option<super::super::mc::CompoundMotionGrid>> {
        let Some(index) = self
            .motion
            .take()
            .and_then(|index| index.get().checked_sub(1))
        else {
            return Ok(None);
        };
        let grid = grids
            .get_mut(index)
            .and_then(Option::take)
            .ok_or_else(invalid_inter_tile_scheduling_state)?;
        let storage = storage.ok_or_else(invalid_inter_tile_scheduling_state)?;
        Ok(Some(grid.view(storage)))
    }
}

pub(super) struct ReconSuperblock {
    pub(super) origin: [usize; 2],
    pub(super) entries: Range<usize>,
}

fn push_recon_entry<Entry>(
    superblocks: &mut Vec<ReconSuperblock>,
    entries: &mut Vec<Entry>,
    origin: [usize; 2],
    entry: Entry,
) {
    let entry_index = entries.len();
    entries.push(entry);
    if let Some(superblock) = superblocks.last_mut().filter(|sb| sb.origin == origin) {
        superblock.entries.end = entries.len();
    } else {
        superblocks.push(ReconSuperblock {
            origin,
            entries: entry_index..entries.len(),
        });
    }
}

#[derive(Default)]
struct FrameResiduals {
    coefficients: Vec<i32>,
    planes: crate::residual::pipeline::ResidualPlaneArena,
}

pub(super) struct RowResiduals {
    frame: Arc<Mutex<FrameResiduals>>,
    planes: crate::residual::pipeline::ResidualPlaneSpan,
    range: Range<usize>,
    capacity: usize,
}

impl RowResiduals {
    pub(super) fn take_planes(
        &self,
        target: &mut crate::residual::pipeline::ResidualPlaneArena,
    ) -> Result<()> {
        self.frame
            .lock()
            .planes
            .take_row(&self.planes, target, self.capacity / 16)
    }

    pub(super) fn copy_into(&self, target: &mut Vec<i32>) -> Result<()> {
        target.clear();
        target
            .try_reserve_exact(self.capacity)
            .map_err(|_| inter_allocation!("coefficient row snapshot"))?;
        let frame = self.frame.lock();
        let samples = frame
            .coefficients
            .get(self.range.clone())
            .ok_or_else(invalid_inter_tile_scheduling_state)?;
        if samples.len() > self.capacity {
            return Err(invalid_inter_tile_scheduling_state());
        }
        target.extend_from_slice(samples);
        Ok(())
    }
}

pub(super) struct ReconRow {
    pub(super) residual_source: Option<RowResiduals>,
    pub(super) ordinal: usize,
    pub(super) superblocks: Vec<ReconSuperblock>,
    /// The coefficients this row's transform blocks index into.
    pub(super) residual_coeffs: Vec<i32>,
    pub(super) entries: Vec<ReconRowEntry>,
    pub(super) residual_blocks: Vec<InterResidualBlock>,
    pub(super) temporal: Vec<TemporalMotionBlock>,
    pub(super) motion_grids: Vec<Option<super::super::mc::StoredMotionGrid>>,
    pub(super) motion_storage: Option<std::sync::Arc<super::super::mc::MotionRowStorage>>,
    /// The unit's flag-plane publications, replayed by a resolve pass that runs
    /// on a grid of its own. Empty unless the parser was logging.
    pub(super) flag_log: Vec<NeighbourFlagRecord>,
    pub(super) filter_records: TileFilterRecords,
    pub(super) residual_planes: crate::residual::pipeline::ResidualPlaneArena,
    /// Whether the prepass already folded this unit's records into the frame's
    /// motion field, which it does for a unit it reconstructed in full.
    pub(super) motion_folded: bool,
    /// Whether the motion pass already derived every entry's grid and records,
    /// so no later pass may derive either again.
    pub(super) motion_derived: bool,
    failure: ReconRowFailure,
}

impl ReconRow {
    fn has_terminal_error(&self) -> bool {
        matches!(self.failure, ReconRowFailure::Terminal(_))
    }

    fn record_terminal_error(&mut self, error: crate::DecodeError) {
        self.failure.record_terminal(error);
    }

    fn record_precompute_error(&mut self, index: usize, error: crate::DecodeError) {
        self.failure.record_precompute(index, error);
    }

    pub(super) fn return_terminal_error(&mut self) -> Result<()> {
        if let Some(error) = self.failure.take_terminal() {
            return Err(error);
        }
        Ok(())
    }

    pub(super) fn take_precompute_error(&mut self) -> Option<(usize, crate::DecodeError)> {
        self.failure.take_precompute()
    }
}

#[derive(Default)]
enum ReconRowFailure {
    #[default]
    None,
    Terminal(crate::DecodeError),
    Precompute {
        index: usize,
        error: crate::DecodeError,
    },
}

impl ReconRowFailure {
    fn record_terminal(&mut self, error: crate::DecodeError) {
        if !matches!(self, Self::Terminal(_)) {
            *self = Self::Terminal(error);
        }
    }

    fn record_precompute(&mut self, index: usize, error: crate::DecodeError) {
        if matches!(self, Self::None) {
            *self = Self::Precompute { index, error };
        }
    }

    fn take_terminal(&mut self) -> Option<crate::DecodeError> {
        match core::mem::take(self) {
            Self::Terminal(error) => Some(error),
            failure => {
                *self = failure;
                None
            }
        }
    }

    fn take_precompute(&mut self) -> Option<(usize, crate::DecodeError)> {
        match core::mem::take(self) {
            Self::Precompute { index, error } => Some((index, error)),
            failure => {
                *self = failure;
                None
            }
        }
    }
}

#[derive(Default)]
pub(crate) struct ReconRowBuffers {
    pub(super) superblocks: Vec<ReconSuperblock>,
    /// The coefficients this row's transform blocks index into.
    pub(super) residual_coeffs: Vec<i32>,
    pub(super) entries: Vec<ReconRowEntry>,
    pub(super) residual_blocks: Vec<InterResidualBlock>,
    pub(super) temporal: Vec<TemporalMotionBlock>,
    pub(super) motion_grids: Vec<Option<super::super::mc::StoredMotionGrid>>,
    pub(super) motion_storage: Option<std::sync::Arc<super::super::mc::MotionRowStorage>>,
    pub(super) flag_log: Vec<NeighbourFlagRecord>,
    pub(super) filter_records: TileFilterRecords,
    pub(super) residual_planes: crate::residual::pipeline::ResidualPlaneArena,
}

impl ReconRowBuffers {
    fn reserve_coefficients(&mut self, capacity: usize) -> Result<()> {
        self.residual_coeffs
            .try_reserve_exact(capacity.saturating_sub(self.residual_coeffs.len()))
            .map_err(|_| inter_allocation!("superblock coefficients"))
    }
}

fn superblock_coefficient_capacity(sb_h4: usize, chroma: ChromaFormatIdc) -> Result<usize> {
    let side = sb_h4
        .checked_mul(4)
        .ok_or(crate::DecodeHeaderStateError::InvalidBlockGeometry)?;
    let luma = side
        .checked_mul(side)
        .ok_or(crate::DecodeHeaderStateError::InvalidBlockGeometry)?;
    let (sx, sy) = chroma_subsampling(chroma);
    let chroma = if chroma == ChromaFormatIdc::Monochrome {
        0
    } else {
        (luma >> (usize::from(sx) + usize::from(sy)))
            .checked_mul(2)
            .ok_or(crate::DecodeHeaderStateError::InvalidBlockGeometry)?
    };
    luma.checked_add(chroma)
        .ok_or_else(|| crate::DecodeHeaderStateError::InvalidBlockGeometry.into())
}

#[derive(Default)]
struct ReconRowBufferPool {
    available: Mutex<Vec<ReconRowBuffers>>,
}

impl ReconRowBufferPool {
    /// Tops this tile's set up to `slots`, keeping what the last tile left.
    ///
    /// The decoder holds one of these for the life of the stream, so the sets
    /// stay here between tiles instead of going back to the retained list.
    fn reset(&mut self, slots: usize) {
        let available = self.available.get_mut();
        if available.len() < slots {
            available.resize_with(slots, ReconRowBuffers::default);
        }
    }

    fn take(&self) -> ReconRowBuffers {
        self.available.lock().pop().unwrap_or_default()
    }

    fn recycle(&self, buffers: ReconRowBuffers) {
        self.available.lock().push(buffers);
    }
}

struct ReadyReconRow<T: ReconSample> {
    row: ReconRow,
    surface: Option<splot_recon::OwnedFrameRect<T>>,
    bounds: row_gate::RowReferenceBounds,
}

pub(crate) struct InterReconScratchPool<T: ReconSample> {
    available: Mutex<(usize, Vec<deferred_recon::InterReconScratch<T>>)>,
}

impl<T: ReconSample> InterReconScratchPool<T> {
    fn ensure_workers(&self, workers: usize) {
        let mut pool = self.available.lock();
        let (allocated, available) = &mut *pool;
        while *allocated < workers {
            available.push(deferred_recon::InterReconScratch::default());
            *allocated += 1;
        }
    }

    fn with_scratch<R>(
        &self,
        f: impl FnOnce(&mut deferred_recon::InterReconScratch<T>) -> R,
    ) -> Result<R> {
        let mut scratch = self
            .available
            .lock()
            .1
            .pop()
            .ok_or_else(invalid_inter_tile_scheduling_state)?;
        let result = f(&mut scratch);
        self.available.lock().1.push(scratch);
        Ok(result)
    }
}

impl<T: ReconSample> Default for InterReconScratchPool<T> {
    fn default() -> Self {
        Self {
            available: Mutex::new((0, Vec::new())),
        }
    }
}

/// The parse state one tile at a time is laid out into.
///
/// dav2d keeps a frame context's arrays for the whole stream and resets them
/// per frame. These are splot's tile-scoped equivalents: the decoder holds one
/// set, and each tile is laid out into it rather than building its own.
#[derive(Default)]
pub(in crate::prediction::inter) struct TileParseState {
    filter_records: TileFilterRecords,
    intrabc_state: TileIntrabcPreludeState,
    residual_scratch: InterResidualParseScratch,
    mv_grid: NeighbourMvGrid,
    coeff_ctx: TileCoeffContextState,
    /// The partition stack and loop-restoration record lists this tile fills.
    traversal: crate::bitstream::tile_payload::TileTraversalStorage,
    /// The row buffer sets this tile's units are parsed and replayed through.
    row_buffers: ReconRowBufferPool,
    /// The smooth-mode grids this tile's intra edges are recorded in.
    y_smooth: crate::prediction::intra_edge::TileYSmoothGrid,
    chroma_smooth: crate::prediction::intra_edge::TileChromaSmoothGrid,
    /// The tile's block-decoded grid, and the copy the commit spine reads.
    block_decoded: TileBlockDecodedState,
    commit_block_decoded: TileBlockDecodedState,
}

#[derive(Default)]
pub(in crate::prediction::inter) struct TileDecodeScratch<T: ReconSample> {
    parse: TileParseState,
    /// The surface source the tile's units draw their reconstruction
    /// rectangles from, kept whole so its lock and handle outlive the tile.
    surface_source: Option<std::sync::Arc<Mutex<admission::SurfaceSource<T>>>>,
    ordered: deferred_recon::InterReconScratch<T>,
    workers: InterReconScratchPool<T>,
    surfaces: Vec<splot_recon::OwnedFrameRect<T>>,
    batches: admission::BatchRowSlots<T>,
    scheduled_rows: admission::ScheduledRowSlots<T>,
    /// The decode's reusable storage, for the sealed copy and the row sets.
    pub(in crate::prediction::inter) buffers:
        Option<std::sync::Arc<crate::support::decode_buffers::DecodeBuffers>>,
}

impl<T: ReconSample> TileDecodeScratch<T> {
    fn from_scheduled(
        ordered: deferred_recon::InterReconScratch<T>,
        surfaces: Vec<splot_recon::OwnedFrameRect<T>>,
    ) -> Self {
        Self {
            parse: TileParseState::default(),
            surface_source: None,
            ordered,
            workers: InterReconScratchPool::default(),
            surfaces,
            batches: admission::BatchRowSlots::default(),
            scheduled_rows: admission::ScheduledRowSlots::default(),
            buffers: None,
        }
    }
}

/// Stamps a reused reconstruction surface with a sentinel legal at every bit
/// depth, so any sample the prepass and commit replay leave unwritten is an
/// obviously wrong output rather than the previous frame's plausible one.
///
/// Guarded exactly as `debug_assert!` is, and so is not compiled into a release
/// build at all.
#[cfg(debug_assertions)]
fn poison_reused_surface<T: ReconSample>(surface: &mut splot_recon::OwnedFrameRect<T>) {
    surface.fill(T::try_from_u16(u8::MAX.into()).unwrap_or_default());
}

#[cfg(not(debug_assertions))]
#[expect(
    clippy::inline_always,
    reason = "empty in release; inlining removes the call the poison check costs"
)]
#[inline(always)]
fn poison_reused_surface<T: ReconSample>(_surface: &mut splot_recon::OwnedFrameRect<T>) {}

#[allow(clippy::too_many_arguments)]
fn precompute_recon_row<T: ReconSample>(
    mut ready: ReadyReconRow<T>,
    scratch: &mut deferred_recon::InterReconScratch<T>,
    block_decoded: &TileBlockDecodedState,
    motion: &MotionFieldUnits,
    quantizer: &FrameQuantizerSnapshot,
    temporal_context: &TemporalMvContext,
    reference: &InterReferenceState<T>,
    ref_frame_idx: &[u32],
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    sb_h4: usize,
    mi_rows: usize,
    mi_cols: usize,
    current_order_hint: u32,
    luma_use_tcq: bool,
    residual_use_ddt: bool,
    bit_depth: BitDepth,
) -> ReadyReconRow<T> {
    let Some(surface) = ready.surface.as_mut() else {
        return ready;
    };
    let mut surface = mc::WorkspaceSink::OwnedRect(surface);
    ready.row = precompute_recon_row_on_surface(
        ready.row,
        &mut surface,
        scratch,
        block_decoded,
        motion,
        quantizer,
        temporal_context,
        reference,
        ref_frame_idx,
        sequence,
        core,
        sb_h4,
        mi_rows,
        mi_cols,
        current_order_hint,
        luma_use_tcq,
        residual_use_ddt,
        bit_depth,
    );
    ready
}

/// Precomputes the row's leading run of independent entries into `surface`,
/// stopping at the first entry that must replay in walk order.
///
/// Stopping — rather than skipping past — is what keeps the prepass
/// walk-order-exact: the whole surface publishes before any replay, so an
/// entry precomputed past a skipped one would land writes that overlap it
/// (mixed-region chroma residual) before it instead of after, reading
/// prepublish samples as its residual base.
#[allow(clippy::too_many_arguments)]
fn precompute_recon_row_on_surface<T: ReconSample>(
    mut row: ReconRow,
    surface: &mut super::super::mc::WorkspaceSink<'_, '_, T>,
    scratch: &mut deferred_recon::InterReconScratch<T>,
    block_decoded: &TileBlockDecodedState,
    motion: &MotionFieldUnits,
    quantizer: &FrameQuantizerSnapshot,
    temporal_context: &TemporalMvContext,
    reference: &InterReferenceState<T>,
    ref_frame_idx: &[u32],
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    sb_h4: usize,
    mi_rows: usize,
    mi_cols: usize,
    current_order_hint: u32,
    luma_use_tcq: bool,
    residual_use_ddt: bool,
    bit_depth: BitDepth,
) -> ReconRow {
    let mut coefficient_scratch = std::mem::take(&mut scratch.coefficients);
    let row = (|| {
        if row.has_terminal_error() {
            return row;
        }
        let shared_coefficients = row.residual_source.is_some();
        let mut coefficients_loaded = false;
        let _quantizer_scopes = quantizer.install_frame();
        let info = surface.info();
        if !row.motion_derived {
            let temporal_capacity = row.entries.iter().fold(0usize, |capacity, entry| {
                capacity.saturating_add(
                    entry
                        .command()
                        .map_or(0, ReconCommand::temporal_record_capacity),
                )
            });
            let _ = row.temporal.try_reserve(temporal_capacity);
        }
        'superblocks: for superblock in &row.superblocks {
            let entry_start = superblock.entries.start;
            let Some(entries) = row.entries.get_mut(superblock.entries.clone()) else {
                break;
            };
            for (offset, entry) in entries.iter_mut().enumerate() {
                let safe = matches!(
                    entry.command(),
                    Some(ReconCommand::Inter(command))
                        if !command.reads_current_frame()
                            && command.prepass_write_is_contained(
                                superblock.origin,
                                sb_h4,
                                info,
                                &row.residual_blocks,
                            )
                );
                if !safe {
                    break 'superblocks;
                }
                let command = match entry.take_command() {
                    Some(ReconCommand::Inter(command)) => command,
                    command => {
                        if let Some(command) = command {
                            entry.store_command(command);
                        }
                        break 'superblocks;
                    }
                };
                if !coefficients_loaded && let Some(source) = &row.residual_source {
                    if let Err(error) = source.copy_into(&mut coefficient_scratch) {
                        row.record_precompute_error(entry_start + offset, error);
                        break 'superblocks;
                    }
                    coefficients_loaded = true;
                }
                let start = row.temporal.len();
                let result = if row.motion_derived {
                    entry
                        .take_motion(&mut row.motion_grids, row.motion_storage.as_ref())
                        .and_then(|grid| {
                            scratch.reconstruct_from_motion(
                                &command,
                                surface,
                                block_decoded,
                                grid,
                                &row.residual_blocks,
                                if shared_coefficients {
                                    &coefficient_scratch
                                } else {
                                    &row.residual_coeffs
                                },
                                &deferred_recon::ReconShared {
                                    reference,
                                    ref_frame_idx,
                                    temporal_context,
                                    sequence,
                                    core,
                                    luma_use_tcq,
                                    residual_use_ddt,
                                    bit_depth,
                                    mi_rows,
                                    mi_cols,
                                    current_order_hint,
                                },
                            )
                        })
                        .map(drop)
                } else {
                    scratch.reconstruct_logged(
                        &command,
                        surface,
                        block_decoded,
                        &mut row.temporal,
                        &row.residual_blocks,
                        if shared_coefficients {
                            &coefficient_scratch
                        } else {
                            &row.residual_coeffs
                        },
                        temporal_context,
                        reference,
                        ref_frame_idx,
                        sequence,
                        core,
                        mi_rows,
                        mi_cols,
                        current_order_hint,
                        luma_use_tcq,
                        residual_use_ddt,
                        bit_depth,
                    )
                };
                match result {
                    Ok(()) => {
                        if !row.motion_derived {
                            entry.temporal = start..row.temporal.len();
                        }
                    }
                    Err(error) => {
                        row.temporal.truncate(start);
                        row.record_precompute_error(entry_start + offset, error);
                        break 'superblocks;
                    }
                }
            }
        }
        if row.motion_derived {
            return row;
        }
        row.motion_folded = row
            .entries
            .iter()
            .all(|entry| !matches!(entry.command(), Some(ReconCommand::Inter(_))));
        if row.motion_folded && !row.superblocks.is_empty() {
            for entry in &mut row.entries {
                if let Some(clear) =
                    entry.temporal_clear_record(mi_rows, mi_cols, current_order_hint)
                {
                    let start = row.temporal.len();
                    row.temporal.push(clear);
                    entry.temporal = start..row.temporal.len();
                }
            }
            if let Err(error) = motion.fold_unit(row.ordinal, &row.temporal) {
                row.record_terminal_error(error);
            } else {
                motion.unit_landed_for(row.ordinal);
            }
        }
        row
    })();
    scratch.coefficients = coefficient_scratch;
    row
}

/// One tile's units after the entropy pass, owned so the § 7.12 resolve pass
/// and the reconstruction pass can run once the driver has moved on.
///
/// The parse pass reads no reference sample and no projected motion field, so
/// everything here is settled by the bitstream alone; what is still owed is the
/// resolve pass (which needs the frame's temporal prelude) and reconstruction
/// (which needs reference pixels).
pub(super) struct ParsedTile {
    unit_count: usize,
    output: TileParserOutput,
}

impl ParsedTile {
    /// Finishes the scheduled one-tile path without copying its frame-sized
    /// filter grids through a second set of tile buffers.
    pub(super) fn finish_single_tile_filter_state(
        mut self,
        parse_progress: &ParseProgress,
        frame_filter_records: &mut crate::filters::wienerns_lr::FrameFilterRecords,
        segment_ids: Option<&mut FrameSegmentIdMap>,
    ) -> Result<(usize, CdefState, GdfState, CcsoState)> {
        let output = &mut self.output;
        if let Some(segment_ids) = segment_ids {
            segment_ids.merge_tile(&output.segment_id_state);
        }
        append_lr_records(
            &mut frame_filter_records.lr_source_blocks,
            &mut frame_filter_records.lr_unit_filters,
            &mut output.traversal.active_source_blocks,
            &mut output.traversal.unit_filters,
        )?;
        parse_progress.parser.lock().traversal = core::mem::take(&mut output.traversal);
        let TileParserOutput {
            cdef_state,
            gdf_state,
            ccso_state,
            ..
        } = self.output;
        Ok((self.unit_count, cdef_state, gdf_state, ccso_state))
    }
}

/// How many parse units one tile yields, plus the terminating empty unit.
fn tile_unit_capacity(
    mi_rows: &Range<usize>,
    mi_cols: &Range<usize>,
    frame_mi_rows: usize,
    frame_mi_cols: usize,
    sb_h4: usize,
) -> usize {
    let sb_rows = mi_rows
        .end
        .min(frame_mi_rows)
        .saturating_sub(mi_rows.start)
        .div_ceil(sb_h4);
    let sb_cols = mi_cols
        .end
        .min(frame_mi_cols)
        .saturating_sub(mi_cols.start)
        .div_ceil(sb_h4);
    sb_rows * sb_cols + 1
}

/// The tile geometry the § 8.2 parser settles before it reads its first
/// unit, which is everything the admission scheduler needs to lay out
/// batches and surfaces.
pub(crate) struct TileGeometry {
    pub(super) tile_offset: ByteOffset,
    pub(super) mi_rows: Range<usize>,
    pub(super) mi_cols: Range<usize>,
    pub(super) unit_count: usize,
}

#[derive(Default)]
pub(crate) struct ParseProgress {
    row_buffers: Mutex<Vec<Option<ReconRowBuffers>>>,
    residuals: Arc<Mutex<FrameResiduals>>,
    coefficient_scratch: Mutex<Vec<i32>>,
    plane_scratch: Mutex<crate::residual::pipeline::ResidualPlaneArena>,
    finished: splot_parallel::WatermarkCell,
    rows: Mutex<Vec<Option<ReconRow>>>,
    geometry: Mutex<GeometryState>,
    records: Mutex<crate::filters::wienerns_lr::FrameFilterRecords>,
    parser: Mutex<TileParseState>,
}

#[derive(Default)]
enum GeometryState {
    #[default]
    Unpublished,
    Spare(Arc<TileGeometry>),
    Published(Arc<TileGeometry>),
}

impl ParseProgress {
    /// Resets a retired frame's parse state while keeping its backing storage.
    pub(crate) fn reset(
        &mut self,
        buffers: &crate::support::decode_buffers::DecodeBuffers,
    ) -> bool {
        let geometry = self.geometry.get_mut();
        let geometry_reusable = match geometry {
            GeometryState::Unpublished => true,
            GeometryState::Spare(geometry) | GeometryState::Published(geometry) => {
                Arc::get_mut(geometry).is_some()
            }
        };
        if !geometry_reusable {
            return false;
        }
        let Some(residuals) = Arc::get_mut(&mut self.residuals) else {
            return false;
        };
        *geometry = match core::mem::take(geometry) {
            GeometryState::Published(geometry) | GeometryState::Spare(geometry) => {
                GeometryState::Spare(geometry)
            }
            GeometryState::Unpublished => GeometryState::Unpublished,
        };
        self.rows.get_mut().clear();
        let residuals = residuals.get_mut();
        residuals.coefficients.clear();
        residuals.planes.clear();
        self.finished.reset();
        let records = self.records.get_mut();
        records.clear();
        records.reserve_from(buffers.tile_record_capacities());
        true
    }

    /// Hands one finished unit to the scheduler and publishes the new count.
    ///
    /// The frame's § 7.17 and loop-restoration records leave the unit here, in
    /// parse order, because the scheduler claims units on its own schedule and
    /// the frame-level detach must not depend on when it does.
    pub(super) fn publish_row(&self, mut row: ReconRow) -> TileFilterRecords {
        pixel_commit::detach_row_filter_records(&mut row, &mut self.records.lock());
        let records = std::mem::take(&mut row.filter_records);
        let finished = {
            let mut rows = self.rows.lock();
            rows.push(Some(row));
            rows.len()
        };
        self.finished.publish(finished);
        records
    }

    /// Takes the unit at `index`, which a caller may claim exactly once.
    pub(super) fn take_row(&self, index: usize) -> Option<ReconRow> {
        self.rows.lock().get_mut(index).and_then(Option::take)
    }

    /// Moves the parsed records into the frame, keeping this slot's lists.
    pub(super) fn append_records(
        &self,
        target: &mut crate::filters::wienerns_lr::FrameFilterRecords,
    ) {
        let mut records = self.records.lock();
        if let Some(buffers) = target.buffers.as_ref() {
            buffers.note_tile_record_capacities(records.capacities());
        }
        target.append(&mut records);
    }

    /// Reserves room for one tile's units up front, so the parser never
    /// reallocates the shared buffer while a reader holds an index.
    pub(super) fn reserve(&self, capacity: usize) -> Result<()> {
        self.rows
            .lock()
            .try_reserve_exact(capacity)
            .map_err(|_| inter_allocation!("inter parsed rows"))
    }

    /// Publishes the tile geometry, which the parser settles before its
    /// first unit.
    pub(super) fn publish_geometry(
        &self,
        geometry: TileGeometry,
        coefficients: usize,
    ) -> Result<()> {
        let slots = geometry.unit_count;
        let mut buffers = self.row_buffers.lock();
        if buffers.len() < slots {
            buffers.resize_with(slots, || Some(ReconRowBuffers::default()));
        }
        let capacity = coefficients
            .checked_mul(slots.saturating_sub(1))
            .ok_or(crate::DecodeHeaderStateError::InvalidBlockGeometry)?;
        let mut residuals = self.residuals.lock();
        let additional = capacity.saturating_sub(residuals.coefficients.len());
        residuals
            .coefficients
            .try_reserve_exact(additional)
            .map_err(|_| inter_allocation!("frame coefficients"))?;
        residuals
            .planes
            .reserve_records(capacity / 16)
            .map_err(|_| inter_allocation!("frame residual records"))?;
        drop(residuals);
        drop(buffers);
        let mut published = self.geometry.lock();
        let geometry = match core::mem::take(&mut *published) {
            GeometryState::Unpublished => Arc::new(geometry),
            GeometryState::Spare(mut output) => {
                *Arc::get_mut(&mut output).ok_or_else(invalid_inter_tile_scheduling_state)? =
                    geometry;
                output
            }
            GeometryState::Published(output) => {
                *published = GeometryState::Published(output);
                return Err(invalid_inter_tile_scheduling_state());
            }
        };
        *published = GeometryState::Published(geometry);
        Ok(())
    }

    /// The published tile geometry, if the parser has reached its first unit.
    pub(super) fn geometry(&self) -> Option<Arc<TileGeometry>> {
        match &*self.geometry.lock() {
            GeometryState::Published(geometry) => Some(Arc::clone(geometry)),
            GeometryState::Unpublished | GeometryState::Spare(_) => None,
        }
    }

    /// Releases every waiter after a failed pass.
    ///
    /// Batches and resolve steps wait on unit thresholds the pass will now
    /// never reach, so the watermark is driven past all of them.
    pub(crate) fn fail(&self) {
        self.finished.publish(splot_parallel::WatermarkCell::FAILED);
    }

    fn take_row_buffers(&self, index: usize) -> Result<ReconRowBuffers> {
        self.row_buffers
            .lock()
            .get_mut(index)
            .and_then(Option::take)
            .ok_or_else(invalid_inter_tile_scheduling_state)
    }

    pub(super) fn return_row_buffers(&self, index: usize, buffers: ReconRowBuffers) -> Result<()> {
        let mut rows = self.row_buffers.lock();
        let slot = rows
            .get_mut(index)
            .ok_or_else(invalid_inter_tile_scheduling_state)?;
        if slot.is_some() {
            return Err(invalid_inter_tile_scheduling_state());
        }
        *slot = Some(buffers);
        Ok(())
    }

    /// The cell a batch waits on for its own units.
    pub(crate) fn cell(&self) -> &splot_parallel::WatermarkCell {
        &self.finished
    }
}

/// Settles the tile geometry and publishes it, before anything reads a unit.
///
/// The admission scheduler is built from this alone, and it is promoted while
/// the § 8.2 pass still runs, so this must be called before the walk is
/// promoted -- not as the pass's first act, which would race it.
pub(crate) fn publish_tile_geometry(
    tile: &DecodeTileWorkUnit,
    params: &TileWalkParams,
    chroma: ChromaFormatIdc,
    parse_progress: &ParseProgress,
) -> Result<()> {
    let mi_rows = tile.mi_row_range().start as usize..tile.mi_row_range().end as usize;
    let mi_cols = tile.mi_col_range().start as usize..tile.mi_col_range().end as usize;
    let capacity = tile_unit_capacity(
        &mi_rows,
        &mi_cols,
        params.mi_rows,
        params.mi_cols,
        params.sb_h4,
    );
    parse_progress.reserve(capacity)?;
    parse_progress.publish_geometry(
        TileGeometry {
            tile_offset: tile.tile_byte_span().start,
            mi_rows,
            mi_cols,
            unit_count: capacity,
        },
        superblock_coefficient_capacity(params.sb_h4, chroma)?,
    )?;
    Ok(())
}

impl<'payload> TileParser<'payload> {
    pub(super) fn scheduled<T: ReconSample>(
        tile: &mut DecodeTileWorkUnit,
        tile_bytes: &'payload [u8],
        context: &TileDecodeContext<'_, T>,
        cdef_state: CdefState,
        gdf_state: GdfState,
        ccso_state: CcsoState,
        parse_progress: &ParseProgress,
    ) -> Result<Self> {
        parse_progress
            .geometry()
            .ok_or_else(invalid_inter_tile_scheduling_state)?;
        let mut parser = Self::new(
            tile,
            tile_bytes,
            context,
            cdef_state,
            gdf_state,
            ccso_state,
            core::mem::take(&mut *parse_progress.parser.lock()),
        )?;
        parser.mv_grid.log_flags();
        Ok(parser)
    }

    pub(super) fn parse_scheduled_unit<T: ReconSample>(
        &mut self,
        tile: &mut DecodeTileWorkUnit,
        context: &TileDecodeContext<'_, T>,
        parse_progress: &ParseProgress,
    ) -> Result<bool> {
        let mut row_set = parse_progress.take_row_buffers(self.parser_ordinal)?;
        std::mem::swap(&mut row_set.filter_records, &mut self.filter_records);
        std::mem::swap(
            &mut row_set.residual_coeffs,
            &mut *parse_progress.coefficient_scratch.lock(),
        );
        std::mem::swap(
            &mut row_set.residual_planes,
            &mut *parse_progress.plane_scratch.lock(),
        );
        let capacity = superblock_coefficient_capacity(
            context.params.sb_h4,
            context.sequence.general.chroma_format_idc,
        )?;
        row_set
            .residual_planes
            .reserve_records(capacity / 16)
            .map_err(|_| inter_allocation!("producer residual records"))?;
        let (mut row, last) = match self.next_unit(tile, context, Some(row_set)) {
            ParserStep::More(row) => (row, false),
            ParserStep::Last(row) => (row, true),
        };
        let publication = (|| {
            row.return_terminal_error()?;
            let mut residuals = parse_progress.residuals.lock();
            let start = residuals.coefficients.len();
            residuals
                .coefficients
                .try_reserve_exact(row.residual_coeffs.len())
                .map_err(|_| inter_allocation!("frame coefficient publication"))?;
            residuals
                .coefficients
                .extend_from_slice(&row.residual_coeffs);
            let planes = residuals
                .planes
                .append_row(&mut row.residual_planes)
                .map_err(|_| inter_allocation!("frame residual publication"))?;
            row.residual_source = Some(RowResiduals {
                frame: Arc::clone(&parse_progress.residuals),
                range: start..residuals.coefficients.len(),
                planes,
                capacity,
            });
            Ok::<(), crate::DecodeError>(())
        })();
        row.residual_coeffs.clear();
        std::mem::swap(
            &mut row.residual_coeffs,
            &mut *parse_progress.coefficient_scratch.lock(),
        );
        row.residual_planes.clear();
        std::mem::swap(
            &mut row.residual_planes,
            &mut *parse_progress.plane_scratch.lock(),
        );
        publication?;
        self.filter_records = parse_progress.publish_row(row);
        Ok(last)
    }

    pub(super) fn finish_scheduled(self, parse_progress: &ParseProgress) -> Result<ParsedTile> {
        if !matches!(self.walk, TileParserWalk::Finished) {
            return Err(invalid_inter_tile_scheduling_state());
        }
        parse_progress
            .geometry()
            .ok_or_else(invalid_inter_tile_scheduling_state)?;
        let unit_count = self.parser_ordinal;
        let (output, state) = self.into_output();
        *parse_progress.parser.lock() = state;
        Ok(ParsedTile { unit_count, output })
    }
}

fn reset_tile_block_decoded(
    state: &mut TileBlockDecodedState,
    chroma: ChromaFormatIdc,
    params: &TileWalkParams,
    mi_col_end: usize,
    mi_row_end: usize,
) -> Result<()> {
    let (subsampling_x, subsampling_y) = chroma_subsampling(chroma);
    state
        .reset(
            if chroma == ChromaFormatIdc::Monochrome {
                1
            } else {
                3
            },
            usize::from(subsampling_x),
            usize::from(subsampling_y),
            params.sb_h4,
            mi_col_end.min(params.mi_cols),
            mi_row_end.min(params.mi_rows),
        )
        .map_err(|error| inter_tile_block_decoded_error(&error))
}

fn luma_rect<T: ReconSample>(
    mi_rows: &Range<usize>,
    mi_cols: &Range<usize>,
    workspace: &CurrentFrameWorkspace<T>,
) -> Result<splot_recon::PlaneRect> {
    let storage = workspace.plane(PlaneId::Y)?.storage_size();
    let x = (mi_cols.start)
        .checked_mul(4)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "tile rectangle luma x",
        })?;
    let y = (mi_rows.start)
        .checked_mul(4)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "tile rectangle luma y",
        })?;
    let right = (mi_cols.end)
        .checked_mul(4)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "tile rectangle luma right edge",
        })?
        .min(storage.width());
    let bottom = (mi_rows.end)
        .checked_mul(4)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "tile rectangle luma bottom edge",
        })?
        .min(storage.height());
    Ok(splot_recon::PlaneRect::new(
        x,
        y,
        right.saturating_sub(x),
        bottom.saturating_sub(y),
    )?)
}

fn superblock_luma_rects<T: ReconSample>(
    mi_rows: &Range<usize>,
    mi_cols: &Range<usize>,
    workspace: &CurrentFrameWorkspace<T>,
    sb_h4: usize,
) -> Result<Vec<splot_recon::PlaneRect>> {
    let mut rects = Vec::new();
    superblock_luma_rects_into(mi_rows, mi_cols, workspace, sb_h4, &mut rects)?;
    Ok(rects)
}

fn superblock_luma_rects_into<T: ReconSample>(
    mi_rows: &Range<usize>,
    mi_cols: &Range<usize>,
    workspace: &CurrentFrameWorkspace<T>,
    sb_h4: usize,
    rects: &mut Vec<splot_recon::PlaneRect>,
) -> Result<()> {
    let bounds = luma_rect(mi_rows, mi_cols, workspace)?;
    let side = sb_h4 * 4;
    let rows = bounds.height().div_ceil(side);
    let cols = bounds.width().div_ceil(side);
    let count = rows * cols;
    rects.clear();
    rects
        .try_reserve_exact(count)
        .map_err(|_| inter_allocation!("inter superblock surfaces"))?;
    for row in 0..rows {
        let y = bounds.y() + row * side;
        for column in 0..cols {
            let x = bounds.x() + column * side;
            rects.push(splot_recon::PlaneRect::new(
                x,
                y,
                side.min(bounds.x() + bounds.width() - x),
                side.min(bounds.y() + bounds.height() - y),
            )?);
        }
    }
    Ok(())
}

fn no_decoded_block_error() -> crate::DecodeError {
    crate::DecodeHeaderStateError::InvalidInterTileTraversalState.into()
}

pub(super) fn invalid_inter_tile_scheduling_state() -> crate::DecodeError {
    crate::DecodeHeaderStateError::InvalidInterTileSchedulingState.into()
}

#[allow(clippy::too_many_arguments)]
pub(super) fn decode_tiles<T: ReconSample>(
    scratch: TileDecodeScratch<T>,
    frame_filter_records: &mut crate::filters::wienerns_lr::FrameFilterRecords,
    payload: &[u8],
    extra_payloads: &[&[u8]],
    work_units: &mut [DecodeTileWorkUnit],
    params: &TileWalkParams,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    temporal_context: &TemporalMvContext,
    reference: &InterReferenceState<T>,
    ref_frame_idx: &[u32],
    mut workspace: CurrentFrameWorkspace<T>,
    mut cdef_state: CdefState,
    mut gdf_state: GdfState,
    mut ccso_state: CcsoState,
    motion_field: TemporalMotionField,
    mut segment_ids: Option<&mut FrameSegmentIdMap>,
) -> Result<(
    TileDecodeScratch<T>,
    CurrentFrameWorkspace<T>,
    TileDecodeOutput,
)> {
    let TileDecodeScratch {
        parse: mut parse_state,
        surface_source: mut spent_surface_source,
        mut ordered,
        workers,
        surfaces: mut recycled_surfaces,
        mut batches,
        scheduled_rows,
        buffers,
    } = scratch;
    let context = params.context(sequence, core, reference, ref_frame_idx);
    let &TileWalkParams {
        mi_rows,
        mi_cols,
        sb_h4,
        ..
    } = params;
    frame_filter_records.clear();
    let motion = MotionFieldUnits::new(motion_field);
    let mut decoded_any = false;
    let row_gate = row_gate::RowReferenceGate::new(
        reference,
        core,
        ref_frame_idx,
        workspace.info(),
        temporal_context,
    );
    let global_intrabc = super::intrabc::global_intrabc_enabled(core.intrabc);
    for tile in work_units.iter_mut() {
        let source = if tile.payload_index() == 0 {
            Some(payload)
        } else {
            extra_payloads.get(tile.payload_index() - 1).copied()
        };
        let tile_bytes = source
            .and_then(|payload| payload.get(tile.payload_range()))
            .ok_or_else(invalid_inter_tile_scheduling_state)?;
        let tile_offset = tile.tile_byte_span().start;
        reset_tile_block_decoded(
            &mut parse_state.block_decoded,
            context.sequence.general.chroma_format_idc,
            &context.params,
            tile.mi_col_range().end as usize,
            tile.mi_row_range().end as usize,
        )?;
        parse_state
            .commit_block_decoded
            .clone_from(&parse_state.block_decoded);
        let commit_block_decoded = core::mem::take(&mut parse_state.commit_block_decoded);
        workers.ensure_workers(splot_parallel::current_pool_width().max(1));
        let reusable_surface_source = spent_surface_source.take();
        let block_decoded = core::mem::take(&mut parse_state.block_decoded);
        let quantizer = FrameQuantizerSnapshot::capture();
        let rows = tile.mi_row_range().start as usize..tile.mi_row_range().end as usize;
        let cols = tile.mi_col_range().start as usize..tile.mi_col_range().end as usize;
        let unit_count = tile_unit_capacity(&rows, &cols, mi_rows, mi_cols, sb_h4);
        let units_per_row = cols
            .end
            .min(mi_cols)
            .saturating_sub(cols.start)
            .div_ceil(sb_h4);
        let tile_mi_rows = rows;
        let tile_mi_cols = cols;
        parse_state.row_buffers.reset(
            splot_parallel::current_pool_width()
                .saturating_mul(3)
                .max(1),
        );
        let row_buffers = core::mem::take(&mut parse_state.row_buffers);
        let mut parser = TileParser::new(
            tile,
            tile_bytes,
            &context,
            cdef_state.try_for_tile(tile_mi_rows.clone(), tile_mi_cols.clone())?,
            gdf_state.for_tile(tile_mi_rows.clone(), tile_mi_cols.clone())?,
            ccso_state.try_for_tile(tile_mi_rows.clone(), tile_mi_cols.clone())?,
            parse_state,
        )?;
        let mut resolve_state = TileResolveState::new(sequence);
        let info = workspace.info();
        let rects = if global_intrabc {
            Vec::new()
        } else {
            superblock_luma_rects(&tile_mi_rows, &tile_mi_cols, &workspace, sb_h4)?
        };
        let surface_source = match reusable_surface_source
            .filter(|source| std::sync::Arc::strong_count(source) == 1)
        {
            Some(mut source) => {
                if let Some(inner) = std::sync::Arc::get_mut(&mut source) {
                    inner.get_mut().reset(info, rects, recycled_surfaces);
                    source
                } else {
                    std::sync::Arc::new(Mutex::new(admission::SurfaceSource::new(
                        info,
                        rects,
                        recycled_surfaces,
                    )))
                }
            }
            None => std::sync::Arc::new(Mutex::new(admission::SurfaceSource::new(
                info,
                rects,
                recycled_surfaces,
            ))),
        };
        let commit = TileCommit::direct(
            ordered,
            workspace,
            commit_block_decoded,
            decoded_any,
            std::sync::Arc::clone(&surface_source),
            core::mem::take(frame_filter_records),
        );
        let commit = admission::run_ordinary_tile(
            &mut parser,
            tile,
            &mut resolve_state,
            tile_offset,
            &surface_source,
            unit_count,
            units_per_row,
            &context,
            temporal_context,
            &quantizer,
            &row_gate,
            &row_buffers,
            &workers,
            &block_decoded,
            &motion,
            &mut batches,
            commit,
        )?;
        let (
            next_ordered,
            next_workspace,
            next_decoded,
            next_surfaces,
            next_records,
            spent_block_decoded,
        ) = commit.finish_direct();
        ordered = next_ordered;
        workspace = next_workspace;
        decoded_any = next_decoded;
        recycled_surfaces = next_surfaces;
        *frame_filter_records = next_records;
        let (output, next_parse_state) = parser.into_output();
        parse_state = next_parse_state;
        parse_state.block_decoded = block_decoded;
        parse_state.commit_block_decoded = spent_block_decoded;
        parse_state.row_buffers = row_buffers;
        spent_surface_source = Some(surface_source);
        merge_tile_filter_state(
            &mut cdef_state,
            &mut gdf_state,
            &mut ccso_state,
            segment_ids.as_deref_mut(),
            &output,
            tile_mi_rows,
            tile_mi_cols,
        )?;
        let mut output = output;
        append_lr_records(
            &mut frame_filter_records.lr_source_blocks,
            &mut frame_filter_records.lr_unit_filters,
            &mut output.traversal.active_source_blocks,
            &mut output.traversal.unit_filters,
        )?;
        parse_state.traversal = core::mem::take(&mut output.traversal);
    }
    if !decoded_any {
        return Err(no_decoded_block_error());
    }

    Ok((
        TileDecodeScratch {
            buffers,
            parse: parse_state,
            surface_source: spent_surface_source,
            ordered,
            workers,
            surfaces: recycled_surfaces,
            batches,
            scheduled_rows,
        },
        workspace,
        TileDecodeOutput {
            cdef_state,
            gdf_state,
            ccso_state,
            motion_field: motion.into_field(),
        },
    ))
}

#[cfg(test)]
#[path = "tile_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tile_state_tests.rs"]
mod state_tests;

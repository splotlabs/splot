// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Tile-local block decode and reconstruction.
//!
//! Feature tracking: `INFRA-DECODE-PARALLEL-STAGES`.

use std::ops::Range;

use splot_recon::{PlaneId, ReconError};

use super::*;

mod admission;
pub(super) use admission::project_temporal_band;

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
    pub(super) inter_leaves: Vec<super::filter_records::InterDeblockLeaf>,
    /// The list lengths the last publication moved out.
    pub(super) published: [usize; 4],
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
    pub(super) allow_intrabc: bool,
    pub(super) frame_is_switch: bool,
    pub(super) current_order_hint: u32,
    pub(super) temporal_first_frame: bool,
    /// AV2 § 7.12.2 TIP reference pair, derived from the reference order hints
    /// alone so the entropy pass never reads the projected temporal field.
    pub(super) tip_ref_pair: Option<(i8, i8)>,
    pub(super) intra_tools: crate::pipeline::general_intra::GeneralIntraFrameTools,
    /// Bit `s`: segment `s` enables `SEG_LVL_SKIP`, then `SEG_LVL_GLOBALMV`.
    pub(super) segment_skip: u16,
    pub(super) segment_globalmv: u16,
    pub(super) seg_pre_skip: bool,
    pub(super) skip_mode_present: bool,
    pub(super) tip_frame_mode: TipFrameMode,
    pub(super) force_integer_mv: bool,
    /// `None` when the header has no MV precision; each block that reads it
    /// then returns the header error.
    pub(super) mv_config: Option<super::super::read_mv::MvReadConfig>,
    pub(super) ref_facts: super::compound_path::FrameRefFacts,
}

impl TileWalkParams {
    /// The frame MV read configuration, or the error of a header without one.
    pub(super) fn mv_config(
        &self,
        core: &FrameHeaderCore,
    ) -> Result<super::super::read_mv::MvReadConfig> {
        self.mv_config
            .map_or_else(|| super::inter_mv_read_config(core), Ok)
    }

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

/// How far one [`TileParser::walk_leaves`] call parses.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum WalkSpan {
    Superblock,
    Row,
}

pub(super) struct TileParser<'payload> {
    walk: TileParserWalk<GeneralIntraMultiblockCursor<'payload>>,
    coeff_ctx: TileCoeffContextState,
    residual_scratch: InterResidualParseScratch,
    delta_q_state: DeltaQState,
    intrabc_state: TileIntrabcPreludeState,
    mv_grid: NeighbourMvGrid,
    filter_records: TileFilterRecords,
    /// The planes this row's general-intra blocks have parsed.
    residual_planes: crate::residual::pipeline::ResidualPlaneArena,
    output: TileParserOutput,
    parser_ordinal: usize,
    /// The frame raster numbering of a scheduled tile's units.
    numbering: Option<UnitNumbering>,
    tile: usize,
}

/// Maps one tile's parse units to frame raster superblock indices, so the
/// scheduled frame commits every tile's units in one superblock-row order.
#[derive(Clone, Copy)]
struct UnitNumbering {
    sb_row0: usize,
    sb_col0: usize,
    tile_sb_cols: usize,
    frame_sb_cols: usize,
    tile_sbs: usize,
    /// The frame's trailing empty unit, which only the last tile publishes.
    trailing: Option<usize>,
}

impl UnitNumbering {
    fn ordinal(self, unit: usize) -> usize {
        if unit >= self.tile_sbs {
            return self.trailing.unwrap_or(usize::MAX);
        }
        (self.sb_row0 + unit / self.tile_sb_cols) * self.frame_sb_cols
            + self.sb_col0
            + unit % self.tile_sb_cols
    }
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
        let frame_is_intra = context.core.frame_is_intra == Some(true);
        parse.intrabc_state.reset_for_tile(
            (context.params.mi_rows, context.params.mi_cols),
            tile_rows.clone(),
            tile_cols.clone(),
            context.sequence,
            frame_is_intra,
            frame_is_intra
                && crate::filters::wienerns_lr::intrabc_records::frame_allows_intrabc(context.core),
        )?;
        let segment_id_state = if super::segmentation_enabled(context.core) {
            TileSegmentIdState::new_for_tile(tile_rows.clone(), tile_cols.clone())
                .map_err(|error| inter_tile_segment_id_error(&error))?
        } else {
            TileSegmentIdState::disabled()
        };
        parse
            .mv_grid
            .reset_for_tile(tile_rows.clone(), tile_cols.clone(), context.params.sb_h4)
            .and_then(|()| parse.mv_grid.track_edges())
            .map_err(|error| inter_tile_grid_error(&error, "inter parser MV grid"))?;
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
            numbering: None,
            tile: 0,
        })
    }

    /// Parses the next unit into `buffers`.
    ///
    /// Row units, the single-thread walk's, reserve the superblock bound up
    /// front; scheduled units grow their slot's recycled lists instead, so a
    /// frame in flight holds only what it parsed.
    fn next_unit<T: ReconSample>(
        &mut self,
        tile: &mut DecodeTileWorkUnit,
        context: &TileDecodeContext<'_, T>,
        buffers: Option<ReconRowBuffers>,
        row_units: bool,
    ) -> ParserStep<ReconRow> {
        let tile_offset = tile.tile_byte_span().start;
        let mut buffers = buffers.unwrap_or_default();
        let reservation = if row_units {
            superblock_coefficient_capacity(
                context.params.sb_h4,
                context.sequence.general.chroma_format_idc,
            )
            .and_then(|capacity| buffers.reserve_unit(capacity))
        } else {
            Ok(())
        };
        let ReconRowBuffers {
            superblocks,
            residual_coeffs,
            entries,
            slots,
            residual_blocks,
            temporal,
            residual_planes,
            filter_records,
            ..
        } = buffers;
        self.residual_planes = residual_planes;
        let mut recon_row = ReconRow {
            ordinal: self.numbering.map_or(self.parser_ordinal, |numbering| {
                numbering.ordinal(self.parser_ordinal)
            }),
            tile: self.tile,
            superblocks,
            residual_coeffs,
            entries,
            slots,
            residual_blocks,
            temporal,
            residual_planes: crate::residual::pipeline::ResidualPlaneArena::new(),
            filter_records,
            motion_folded: false,
            failure: ReconRowFailure::None,
        };
        if let Err(error) = reservation {
            recon_row.record_terminal_error(error);
            return ParserStep::Last(recon_row);
        }
        self.parser_ordinal = self.parser_ordinal.saturating_add(1);
        let decoded_row = {
            let ReconRow {
                superblocks,
                entries,
                slots,
                residual_blocks,
                residual_coeffs,
                filter_records,
                ..
            } = &mut recon_row;
            self.walk_leaves(
                tile,
                context,
                residual_blocks,
                residual_coeffs,
                filter_records,
                slots,
                if row_units {
                    WalkSpan::Row
                } else {
                    WalkSpan::Superblock
                },
                |_, publication, resolve| {
                    push_leaf_entry(superblocks, entries, publication, resolve);
                    Ok(())
                },
            )
        };
        recon_row.residual_planes = core::mem::take(&mut self.residual_planes);
        match decoded_row.and_then(|more| self.close_walk(more, tile_offset).map(|()| more)) {
            Ok(true) => ParserStep::More(recon_row),
            Ok(false) => ParserStep::Last(recon_row),
            Err(error) => {
                recon_row.record_terminal_error(error);
                ParserStep::Last(recon_row)
            }
        }
    }

    /// Parses one superblock or one superblock row and hands every leaf's
    /// parse record to `consume` in walk order.
    ///
    /// This is the split point between the passes: the consumer decides
    /// whether a record is stored for the resolve and reconstruction stages
    /// or acted on at once. Returns whether superblocks remain.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn walk_leaves<T: ReconSample>(
        &mut self,
        tile: &mut DecodeTileWorkUnit,
        context: &TileDecodeContext<'_, T>,
        residual_blocks: &mut Vec<InterResidualBlock>,
        residual_arena: &mut Vec<i32>,
        filter_records: &mut TileFilterRecords,
        slots: &mut Vec<LeafSlot>,
        span: WalkSpan,
        mut consume: impl FnMut(
            &mut super::BlockDecodeContext<'_, T>,
            DecodedLeafPublication,
            LeafResolveRecord,
        ) -> Result<()>,
    ) -> Result<bool> {
        let tile_offset = tile.tile_byte_span().start;
        let walk = self.walk.active_mut()?;
        let sb_mask = walk.sb_mask();
        let mut block_context = super::BlockDecodeContext {
            sequence: context.sequence,
            core: context.core,
            params: &context.params,
            ref_frame_idx: context.ref_frame_idx,
            reference: context.reference,
            tile_offset,
            coeff_ctx: &mut self.coeff_ctx,
            residual_scratch: &mut self.residual_scratch,
            residual_blocks,
            residual_arena,
            gdf_state: &mut self.output.gdf_state,
            cdef_state: &mut self.output.cdef_state,
            ccso_state: &mut self.output.ccso_state,
            delta_q_state: &mut self.delta_q_state,
            intrabc_state: &mut self.intrabc_state,
            segment_id_state: &mut self.output.segment_id_state,
            mv_grid: &mut self.mv_grid,
            filter_records,
            slots,
            arena: &mut self.residual_planes,
        };
        let mut decode_leaf =
            |work_unit: &mut DecodeTileWorkUnit,
             symbols: &mut SymbolDecoder<'_>,
             frontier: &DecodeBlockFrontier,
             neighbours: crate::bitstream::tile_payload::LeafNeighbours<'_>| {
                let (leaf, resolve) =
                    decode_block(work_unit, symbols, frontier, &mut block_context, neighbours)?;
                consume(
                    &mut block_context,
                    DecodedLeafPublication::from_frontier(frontier, sb_mask),
                    resolve,
                )?;
                Ok((leaf, ()))
            };
        let mut on_published = |_: DecodedLeafPublication, ()| {};
        loop {
            match walk.decode_next_superblock(tile, &mut decode_leaf, &mut on_published) {
                Ok(Some(_)) if span == WalkSpan::Row && !walk.at_row_start() => {}
                Ok(next) => return Ok(next.is_some()),
                Err(error) => return Err(map_inter_multiblock_error(error, tile_offset)),
            }
        }
    }

    /// Checks the neighbour windows after a walk and, once the tile has no
    /// superblock left, retires the walk and its symbol decoder.
    fn close_walk(&mut self, more: bool, tile_offset: ByteOffset) -> Result<()> {
        let walk = self.walk.active_mut()?;
        if walk.window_violated()
            || self.mv_grid.window_violated()
            || self.intrabc_state.window_violated()
        {
            return Err(invalid_inter_tile_scheduling_state());
        }
        if more {
            return Ok(());
        }
        let crate::bitstream::tile_payload::GeneralIntraMultiblockOutput { symbols, storage } =
            self.walk.finish()?.into_output();
        self.output.traversal = storage;
        finish_tile_symbols(symbols, tile_offset)
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
                row_buffers: ReconRowBufferPool::default(),
                traversal: crate::bitstream::tile_payload::TileTraversalStorage::default(),
                commit_block_decoded: TileBlockDecodedState::default(),
                spent_filters: None,
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
            &mut row.slots,
            &mut MvResolutionState {
                grid,
                ref_mv_bank: &mut self.ref_mv_bank,
                warp_param_bank: &mut self.warp_param_bank,
                core: context.core,
                temporal: frame_uses_temporal_mvs(context.core).then_some(temporal_context),
                order_hints: temporal_context.order_hint_mv_context(),
                drl_reorder: sequence_drl_reorder(context.sequence),
                max_drl_bits_minus_1: context.params.max_drl_bits_minus_1,
                frame_precision: context
                    .params
                    .mv_config
                    .map_or(0, super::super::read_mv::MvReadConfig::precision),
                tile_offset,
                sb_h4: context.params.sb_h4,
                mi_rows: context.params.mi_rows,
                mi_cols: context.params.mi_cols,
            },
        )?;
        if grid.window_violated() {
            return Err(invalid_inter_tile_scheduling_state());
        }
        Ok(())
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
    state: EntryState,
    /// Whether the resolve pass still owes this leaf its [`LeafReset`].
    reset_pending: bool,
    temporal: [u32; 2],
}

/// A general-intra leaf, most of a typical row, keeps its small command
/// inline; every other leaf keeps its record, then its command, in the row's
/// [`LeafSlot`] list, so the inline entry stays small.
enum EntryState {
    /// The leaf's luma size in 4x4 units when it codes the luma tree.
    Intra {
        command: crate::pipeline::general_intra::GeneralIntraReconCommand,
        size4: Option<[u8; 2]>,
    },
    Slot(u32),
    Spent,
}

/// A non-intra leaf's record: an inter leaf's command from its parse on, an
/// IntraBC leaf's parse output until the resolve pass turns it into its command.
#[allow(
    clippy::large_enum_variant,
    reason = "nearly every slot is an inter command; boxing it costs an allocation per leaf"
)]
pub(super) enum LeafSlot {
    /// Boxed: only screen-content intra frames carry these, and inline they
    /// would set the size of every slot.
    Intrabc(Box<PendingIntrabcBlock>),
    Command(ReconCommand),
    Spent,
}

impl ReconRowEntry {
    fn slot<'a>(&self, slots: &'a [LeafSlot]) -> Option<&'a LeafSlot> {
        match self.state {
            EntryState::Slot(index) => slots.get(index as usize),
            EntryState::Intra { .. } | EntryState::Spent => None,
        }
    }

    pub(super) fn slot_mut<'a>(&self, slots: &'a mut [LeafSlot]) -> Option<&'a mut LeafSlot> {
        match self.state {
            EntryState::Slot(index) => slots.get_mut(index as usize),
            EntryState::Intra { .. } | EntryState::Spent => None,
        }
    }

    pub(super) fn command<'a>(&'a self, slots: &'a [LeafSlot]) -> Option<ReconCommandRef<'a>> {
        if let EntryState::Intra { .. } = &self.state {
            return Some(ReconCommandRef::GeneralIntra);
        }
        match self.slot(slots)? {
            LeafSlot::Command(command) => Some(command.as_view()),
            LeafSlot::Intrabc(_) | LeafSlot::Spent => None,
        }
    }

    pub(super) fn take_reset(&mut self) -> LeafReset {
        if !core::mem::take(&mut self.reset_pending) {
            return LeafReset::None;
        }
        let size4 = match self.state {
            EntryState::Intra { size4, .. } => size4,
            EntryState::Slot(_) => return LeafReset::Slot,
            EntryState::Spent => return LeafReset::None,
        };
        size4.map_or(LeafReset::Reseed, |[n4w, n4h]| LeafReset::NonInter {
            n4w: usize::from(n4w),
            n4h: usize::from(n4h),
        })
    }

    /// The entry's § 7.22 records in its row's temporal list.
    pub(super) fn temporal(&self) -> Range<usize> {
        self.temporal[0] as usize..self.temporal[1] as usize
    }

    /// Row temporal lists hold a few records per leaf, far below `u32::MAX`.
    pub(super) fn set_temporal(&mut self, records: Range<usize>) {
        self.temporal = [records.start as u32, records.end as u32];
    }

    /// The final command, for a pass that consumes it in place.
    pub(super) fn command_mut<'a>(
        &'a mut self,
        slots: &'a mut [LeafSlot],
    ) -> Option<ReconCommandMut<'a>> {
        let slot = self.slot_mut(slots);
        if let EntryState::Intra { command, .. } = &mut self.state {
            return Some(ReconCommandMut::GeneralIntra(command));
        }
        match slot? {
            LeafSlot::Command(command) => Some(command.as_view_mut()),
            LeafSlot::Intrabc(_) | LeafSlot::Spent => None,
        }
    }

    /// Marks the command consumed, as [`Self::take_command`] would.
    pub(super) fn drop_command(&mut self, slots: &mut [LeafSlot]) {
        if let Some(slot) = self.slot_mut(slots) {
            *slot = LeafSlot::Spent;
        }
        self.state = EntryState::Spent;
    }

    /// The § 7.22 record a non-inter luma-tree leaf stores: every covered 8x8
    /// cell is reset to "no reference", clearing earlier inter writes there.
    pub(super) fn temporal_clear_record(
        &self,
        slots: &[LeafSlot],
        mi_rows: usize,
        mi_cols: usize,
        current_order_hint: u32,
    ) -> Option<TemporalMotionBlock> {
        if !matches!(
            self.command(slots),
            Some(ReconCommandRef::GeneralIntra | ReconCommandRef::Intrabc)
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
}

pub(super) struct ReconSuperblock {
    pub(super) origin: [usize; 2],
    pub(super) entries: Range<usize>,
}

fn push_leaf_entry(
    superblocks: &mut Vec<ReconSuperblock>,
    entries: &mut Vec<ReconRowEntry>,
    publication: DecodedLeafPublication,
    resolve: LeafResolveRecord,
) {
    let state = match resolve {
        LeafResolveRecord::NonInter { command, n4w, n4h } => EntryState::Intra {
            command,
            size4: Some([n4w, n4h]),
        },
        LeafResolveRecord::Reseed(command) => EntryState::Intra {
            command,
            size4: None,
        },
        LeafResolveRecord::Slot(index) => EntryState::Slot(index),
    };
    push_recon_entry(
        superblocks,
        entries,
        publication.superblock_origin(),
        ReconRowEntry {
            publication,
            state,
            reset_pending: true,
            temporal: [0, 0],
        },
    );
}

#[inline]
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

pub(super) struct ReconRow {
    pub(super) ordinal: usize,
    /// The frame tile whose parser produced this unit.
    pub(super) tile: usize,
    pub(super) superblocks: Vec<ReconSuperblock>,
    /// The coefficients this row's transform blocks index into.
    pub(super) residual_coeffs: Vec<i32>,
    pub(super) entries: Vec<ReconRowEntry>,
    /// The non-intra leaves' records and commands, which entries index.
    pub(super) slots: Vec<LeafSlot>,
    pub(super) residual_blocks: Vec<InterResidualBlock>,
    pub(super) temporal: Vec<TemporalMotionBlock>,
    pub(super) residual_planes: crate::residual::pipeline::ResidualPlaneArena,
    /// The unit's deblock and tx-skip records, published by its resolve.
    pub(super) filter_records: TileFilterRecords,
    /// Whether the prepass derived this unit's motion records into `temporal`,
    /// which it does for a unit it reconstructed in full; the in-order commit
    /// folds them, so it is the band's one writer.
    pub(super) motion_folded: bool,
    failure: ReconRowFailure,
}

impl ReconRow {
    /// Publishes the flags of the unit's luma-tree leaves onto `grid`, in leaf
    /// order, as the parser published them on its own grid.
    /// Each inter command takes its leaf index in `grid`.
    pub(super) fn replay_flags(&mut self, grid: &mut NeighbourMvGrid, frame_precision: u8) {
        for entry in &self.entries {
            let Some((r, c, n4w, n4h)) = entry.publication.luma_tree_block() else {
                continue;
            };
            let slot = entry.slot_mut(&mut self.slots);
            let syntax = match (&slot, &entry.state) {
                (
                    Some(slot @ (LeafSlot::Command(ReconCommand::Inter(_)) | LeafSlot::Intrabc(_))),
                    _,
                ) => slot.flag_syntax(frame_precision),
                (None, EntryState::Intra { .. }) => non_inter_flag_syntax(None, frame_precision),
                _ => continue,
            };
            let leaf = grid.record_flags(r, c, n4w, n4h, syntax);
            if let Some(LeafSlot::Command(ReconCommand::Inter(command))) = slot {
                command.leaf = leaf;
            }
        }
    }

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
    /// The non-intra leaves' records and commands, which entries index.
    pub(super) slots: Vec<LeafSlot>,
    pub(super) residual_blocks: Vec<InterResidualBlock>,
    pub(super) temporal: Vec<TemporalMotionBlock>,
    pub(super) residual_planes: crate::residual::pipeline::ResidualPlaneArena,
    pub(super) filter_records: TileFilterRecords,
    /// The large lists' lengths in the unit these buffers last held.
    pub(super) lengths: crate::support::decode_buffers::UnitLengths,
}

impl ReconRowBuffers {
    /// Sizes the emptied large lists to the decode's typical unit; see
    /// [`fit_list`](crate::support::decode_buffers::fit_list).
    fn fit(&mut self, typical: crate::support::decode_buffers::UnitLengths) {
        use crate::support::decode_buffers::fit_list;
        let [
            coeffs,
            entries,
            slots,
            blocks,
            temporal,
            planes,
            units,
            luma,
            chroma,
            tx_skip,
            leaves,
        ] = typical;
        fit_list(&mut self.residual_coeffs, coeffs);
        fit_list(&mut self.entries, entries);
        fit_list(&mut self.slots, slots);
        fit_list(&mut self.residual_blocks, blocks);
        fit_list(&mut self.temporal, temporal);
        self.residual_planes.fit([planes, units]);
        self.filter_records.fit([luma, chroma, tx_skip, leaves]);
    }

    /// Sizes the unit's coefficient and block lists to a superblock's
    /// geometric bound, as dav2d sizes its frame-thread block arrays: every
    /// leaf and transform block covers at least 16 samples. The bound is far
    /// above typical content, but the allocator commits pages only as they
    /// are written, so content never has to grow these lists. The block lists
    /// are only a hint: where the address space is refused they grow instead.
    fn reserve_unit(&mut self, coefficients: usize) -> Result<()> {
        reserve_list(&mut self.residual_coeffs, coefficients)?;
        let blocks = coefficients / 16;
        let _ = self
            .entries
            .try_reserve_exact(blocks.saturating_sub(self.entries.len()));
        let _ = self
            .slots
            .try_reserve_exact(blocks.saturating_sub(self.slots.len()));
        let _ = self
            .residual_blocks
            .try_reserve_exact(blocks.saturating_sub(self.residual_blocks.len()));
        Ok(())
    }
}

fn reserve_list<E>(list: &mut Vec<E>, capacity: usize) -> Result<()> {
    list.try_reserve_exact(capacity.saturating_sub(list.len()))
        .map_err(|_| inter_allocation!("superblock row lists"))
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

/// One reconstruction scratch per scheduler worker, each used only by its
/// worker, so a precompute never waits for another worker's scratch.
pub(crate) struct InterReconScratchPool<T: ReconSample> {
    workers: Vec<Mutex<deferred_recon::InterReconScratch<T>>>,
    surfaces: std::sync::OnceLock<std::sync::Arc<admission::SpareSurfaces<T>>>,
}

impl<T: ReconSample> InterReconScratchPool<T> {
    /// A pool for `workers` scheduler workers.
    pub(crate) fn new(workers: usize) -> Self {
        Self {
            workers: (0..workers.max(1)).map(|_| Mutex::default()).collect(),
            surfaces: std::sync::OnceLock::new(),
        }
    }

    /// The worker count the pool was sized for.
    pub(crate) fn workers(&self) -> usize {
        self.workers.len().max(1)
    }

    /// Lends worker `worker` its own scratch. The lock is only ever taken by
    /// that worker, unless the pool has fewer slots than the scheduler.
    fn with_scratch<R>(
        &self,
        worker: usize,
        f: impl FnOnce(&mut deferred_recon::InterReconScratch<T>) -> R,
    ) -> Result<R> {
        let slot = worker
            .checked_rem(self.workers.len())
            .and_then(|slot| self.workers.get(slot))
            .ok_or_else(invalid_inter_tile_scheduling_state)?;
        Ok(f(&mut slot.lock()))
    }

    fn spare_surfaces(&self) -> std::sync::Arc<admission::SpareSurfaces<T>> {
        std::sync::Arc::clone(
            self.surfaces.get_or_init(|| {
                std::sync::Arc::new(admission::SpareSurfaces::new(self.workers.len()))
            }),
        )
    }
}

impl<T: ReconSample> Default for InterReconScratchPool<T> {
    fn default() -> Self {
        Self {
            workers: Vec::new(),
            surfaces: std::sync::OnceLock::new(),
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
    /// The block-decoded grid the commit spine reads.
    commit_block_decoded: TileBlockDecodedState,
    /// The CDEF, GDF and CCSO tile states the last tile merged, whose grids
    /// the next tile's states are laid out on.
    spent_filters: Option<(CdefState, GdfState, CcsoState)>,
}

#[derive(Default)]
pub(in crate::prediction::inter) struct TileDecodeScratch<T: ReconSample> {
    parse: TileParseState,
    /// The surface source the tile's units draw their reconstruction
    /// rectangles from, kept whole so its handle outlives the tile.
    surface_source: Option<std::sync::Arc<admission::SurfaceSource<T>>>,
    ordered: deferred_recon::InterReconScratch<T>,
    workers: InterReconScratchPool<T>,
    /// The ordinary walk's parsed rows waiting for their replay.
    pending_rows: Vec<ReconRow>,
    scheduled_rows: admission::ScheduledRowSlots<T>,
    /// The decode's reusable storage, for the row sets.
    pub(in crate::prediction::inter) buffers:
        Option<std::sync::Arc<crate::support::decode_buffers::DecodeBuffers>>,
}

impl<T: ReconSample> TileDecodeScratch<T> {
    fn from_scheduled(ordered: deferred_recon::InterReconScratch<T>) -> Self {
        Self {
            parse: TileParseState::default(),
            surface_source: None,
            ordered,
            workers: InterReconScratchPool::default(),
            pending_rows: Vec::new(),
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
    if row.has_terminal_error() {
        return row;
    }
    let _quantizer_scopes = quantizer.install_frame();
    let info = surface.info();
    let shared = deferred_recon::ReconShared {
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
        tip: core::cell::OnceCell::new(),
    };
    'superblocks: for superblock in &row.superblocks {
        let entry_start = superblock.entries.start;
        let Some(entries) = row.entries.get_mut(superblock.entries.clone()) else {
            break;
        };
        let mut skipped_any = false;
        for (offset, entry) in entries.iter_mut().enumerate() {
            let safe = matches!(
                entry.command(&row.slots),
                Some(ReconCommandRef::Inter(command))
                    if !command.reads_current_frame()
                        && (!skipped_any
                            || (command.placed().luma_w >= 8
                                && command.placed().luma_h >= 8
                                && [
                                    command.placed().chroma_luma_x,
                                    command.placed().chroma_luma_y,
                                    command.placed().chroma_luma_w,
                                    command.placed().chroma_luma_h,
                                ] == [
                                    command.placed().luma_x,
                                    command.placed().luma_y,
                                    command.placed().luma_w,
                                    command.placed().luma_h,
                                ]))
                        && command.prepass_write_is_contained(
                            superblock.origin,
                            sb_h4,
                            info,
                            &row.residual_blocks,
                        )
            );
            if !safe {
                match entry.command_mut(&mut row.slots) {
                    Some(ReconCommandMut::Inter(command)) => {
                        let start = row.temporal.len();
                        let result = scratch.derive_motion_logged(
                            command,
                            surface,
                            &mut row.temporal,
                            &shared,
                        );
                        match result {
                            Ok(()) => {
                                entry.set_temporal(start..row.temporal.len());
                            }
                            Err(error) => {
                                row.temporal.truncate(start);
                                row.record_precompute_error(entry_start + offset, error);
                                break 'superblocks;
                            }
                        }
                    }
                    _ => {
                        if let Some(clear) = entry.temporal_clear_record(
                            &row.slots,
                            mi_rows,
                            mi_cols,
                            current_order_hint,
                        ) {
                            let start = row.temporal.len();
                            row.temporal.push(clear);
                            entry.set_temporal(start..row.temporal.len());
                        }
                    }
                }
                skipped_any = true;
                continue;
            }
            let Some(ReconCommandMut::Inter(command)) = entry.command_mut(&mut row.slots) else {
                skipped_any = true;
                continue;
            };
            let start = row.temporal.len();
            let result = {
                scratch.reconstruct_logged(
                    command,
                    surface,
                    block_decoded,
                    &mut row.temporal,
                    &row.residual_blocks,
                    &row.residual_coeffs,
                    &shared,
                )
            };
            entry.drop_command(&mut row.slots);
            match result {
                Ok(()) => {
                    entry.set_temporal(start..row.temporal.len());
                }
                Err(error) => {
                    row.temporal.truncate(start);
                    row.record_precompute_error(entry_start + offset, error);
                    break 'superblocks;
                }
            }
        }
    }
    row.motion_folded = matches!(row.failure, ReconRowFailure::None);
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

    /// Folds one tile of a multi-tile scheduled frame into the frame's filter
    /// state and loop-restoration records, in tile order as [`decode_tiles`]
    /// does, and keeps the tile's segment ids for the frame's map.
    ///
    /// Returns the tile's unit count.
    pub(super) fn merge_scheduled_tile(
        self,
        tile: &DecodeTileWorkUnit,
        parse_progress: &ParseProgress,
        (cdef, gdf, ccso): &mut (CdefState, GdfState, CcsoState),
        frame_filter_records: &mut crate::filters::wienerns_lr::FrameFilterRecords,
        segment_ids: &mut Vec<TileSegmentIdState>,
    ) -> Result<usize> {
        let mut output = self.output;
        merge_tile_filter_state(
            cdef,
            gdf,
            ccso,
            None,
            &output,
            tile.mi_row_range().start as usize..tile.mi_row_range().end as usize,
            tile.mi_col_range().start as usize..tile.mi_col_range().end as usize,
        )?;
        append_lr_records(
            &mut frame_filter_records.lr_source_blocks,
            &mut frame_filter_records.lr_unit_filters,
            &mut output.traversal.active_source_blocks,
            &mut output.traversal.unit_filters,
        )?;
        let TileParserOutput {
            cdef_state,
            gdf_state,
            ccso_state,
            segment_id_state,
            traversal,
        } = output;
        segment_ids
            .try_reserve(1)
            .map_err(|_| inter_allocation!("inter tile segment ids"))?;
        segment_ids.push(segment_id_state);
        let mut state = parse_progress.parser.lock();
        state.traversal = traversal;
        state.spent_filters = Some((cdef_state, gdf_state, ccso_state));
        Ok(self.unit_count)
    }
}

/// Lays one tile's filter states out of the frame's, over the states the last
/// tile spent.
pub(super) fn tile_filter_states(
    (cdef, gdf, ccso): (&CdefState, &GdfState, &CcsoState),
    tile: &DecodeTileWorkUnit,
    spent: Option<(CdefState, GdfState, CcsoState)>,
) -> Result<(CdefState, GdfState, CcsoState)> {
    let rows = tile.mi_row_range().start as usize..tile.mi_row_range().end as usize;
    let cols = tile.mi_col_range().start as usize..tile.mi_col_range().end as usize;
    let (spent_cdef, spent_gdf, spent_ccso) = match spent {
        Some((cdef, gdf, ccso)) => (Some(cdef), Some(gdf), Some(ccso)),
        None => (None, None, None),
    };
    Ok((
        cdef.try_for_tile(rows.clone(), cols.clone(), spent_cdef)?,
        gdf.for_tile(rows.clone(), cols.clone(), spent_gdf)?,
        ccso.try_for_tile(rows, cols, spent_ccso)?,
    ))
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

/// The frame geometry the § 8.2 parser settles before it reads its first
/// unit, which is everything the scheduled reconstruction needs to lay out
/// batches and surfaces. Units are numbered in frame superblock raster order,
/// plus one trailing empty unit.
pub(crate) struct TileGeometry {
    pub(super) mi_rows: Range<usize>,
    pub(super) mi_cols: Range<usize>,
    pub(super) unit_count: usize,
    /// Every tile in tile order.
    pub(super) tiles: Vec<GeometryTile>,
}

#[derive(Clone)]
pub(super) struct GeometryTile {
    pub(super) tile_offset: ByteOffset,
    pub(super) mi_rows: Range<usize>,
    pub(super) mi_cols: Range<usize>,
}

/// Parsed units stored at their ordinal, which tiles reach out of order.
#[derive(Default)]
struct ParsedRows {
    slots: Vec<Option<ReconRow>>,
    /// Every unit before this one has been published.
    prefix: usize,
}

#[derive(Default)]
pub(crate) struct ParseProgress {
    /// The decode's storage, which lends the units their row buffers.
    buffers: Option<Arc<crate::support::decode_buffers::DecodeBuffers>>,
    finished: splot_parallel::WatermarkCell,
    rows: Mutex<ParsedRows>,
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
        buffers: &Arc<crate::support::decode_buffers::DecodeBuffers>,
    ) -> bool {
        let rows = self.rows.get_mut();
        rows.slots.clear();
        rows.prefix = 0;
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
        *geometry = match core::mem::take(geometry) {
            GeometryState::Published(geometry) | GeometryState::Spare(geometry) => {
                GeometryState::Spare(geometry)
            }
            GeometryState::Unpublished => GeometryState::Unpublished,
        };
        self.finished.reset();
        let records = self.records.get_mut();
        records.clear();
        records.reserve_from(buffers.tile_record_capacities());
        self.buffers = Some(Arc::clone(buffers));
        true
    }

    /// Stores one finished unit at its ordinal and publishes how many units
    /// from the first are now all present.
    ///
    /// Tiles parse one after another, so a later tile fills slots before an
    /// earlier superblock row is complete; the watermark only covers the
    /// contiguous prefix, which is what the scheduler claims in order.
    pub(super) fn publish_row(&self, row: ReconRow) -> Result<()> {
        let finished = {
            let mut rows = self.rows.lock();
            let ParsedRows { slots, prefix } = &mut *rows;
            let ordinal = row.ordinal;
            match slots.get_mut(ordinal) {
                Some(slot @ None) if ordinal >= *prefix => *slot = Some(row),
                _ => return Err(invalid_inter_tile_scheduling_state()),
            }
            while slots.get(*prefix).is_some_and(Option::is_some) {
                *prefix += 1;
            }
            *prefix
        };
        self.finished.publish(finished);
        Ok(())
    }

    /// Publishes a unit's deblock and tx-skip records; units are pulled in
    /// order, so the frame's lists stay in leaf order.
    pub(super) fn publish_records(
        &self,
        row: &mut ReconRow,
        chroma_format: ChromaFormatIdc,
    ) -> Result<()> {
        let mut records = self.records.lock();
        let records = &mut *records;
        row.filter_records.publish_into(
            &mut records.deblock_blocks,
            &mut records.chroma_deblock_blocks,
            &mut records.tx_skip_records,
            &row.residual_blocks,
            chroma_format,
        )
    }

    /// Takes the unit at `index`, which a caller may claim exactly once.
    pub(super) fn take_row(&self, index: usize) -> Option<ReconRow> {
        self.rows.lock().slots.get_mut(index).and_then(Option::take)
    }

    /// Moves the deblock records published so far into `deblock`.
    pub(super) fn take_deblock_records(&self, deblock: &mut crate::filters::deblock::FrameDeblock) {
        let mut records = self.records.lock();
        let records = &mut *records;
        deblock.append_records(
            &mut records.deblock_blocks,
            &mut records.chroma_deblock_blocks,
        );
    }

    /// Moves the parsed records into the frame.
    ///
    /// The frame-sized tx-skip list moves whole when the frame has none yet.
    /// The frontier has drained the deblock lists by now, and their capacity
    /// is freed: it only ever held records in transit, so a slot keeping it
    /// would hold its worst frame's backlog for every frame after.
    pub(super) fn append_records(
        &self,
        target: &mut crate::filters::wienerns_lr::FrameFilterRecords,
    ) {
        let mut records = self.records.lock();
        if let Some(buffers) = target.buffers.as_ref() {
            buffers.note_tile_record_capacities(records.capacities());
        }
        if target.tx_skip_records.is_empty() {
            core::mem::swap(&mut target.tx_skip_records, &mut records.tx_skip_records);
        }
        target.append(&mut records);
    }

    /// Lays out an empty slot for each of the frame's units up front.
    pub(super) fn reserve(&self, capacity: usize) -> Result<()> {
        let mut rows = self.rows.lock();
        rows.slots.clear();
        rows.prefix = 0;
        rows.slots
            .try_reserve_exact(capacity)
            .map_err(|_| inter_allocation!("inter parsed rows"))?;
        rows.slots.resize_with(capacity, || None);
        Ok(())
    }

    /// Hands back the row buffers of a unit that is not published: a tile's
    /// trailing empty unit, which only the frame's last tile keeps.
    fn recycle_unpublished(&self, row: ReconRow) {
        let ReconRow {
            superblocks,
            residual_coeffs,
            entries,
            slots,
            residual_blocks,
            temporal,
            residual_planes,
            filter_records,
            ..
        } = row;
        if let Some(shared) = &self.buffers {
            shared.give_unit_buffers(ReconRowBuffers {
                superblocks,
                residual_coeffs,
                entries,
                slots,
                residual_blocks,
                temporal,
                residual_planes,
                filter_records,
                lengths: Default::default(),
            });
        }
    }

    /// Publishes the frame geometry `fill` lays out, which the parser settles
    /// before its first unit. A retired frame's geometry is laid out over, so
    /// its tile list keeps its storage.
    pub(super) fn publish_geometry(
        &self,
        fill: impl FnOnce(&mut TileGeometry) -> Result<()>,
    ) -> Result<()> {
        let mut published = self.geometry.lock();
        let mut geometry = match core::mem::take(&mut *published) {
            GeometryState::Unpublished => Arc::new(TileGeometry {
                mi_rows: 0..0,
                mi_cols: 0..0,
                unit_count: 0,
                tiles: Vec::new(),
            }),
            GeometryState::Spare(output) => output,
            GeometryState::Published(output) => {
                *published = GeometryState::Published(output);
                return Err(invalid_inter_tile_scheduling_state());
            }
        };
        let filled = Arc::get_mut(&mut geometry)
            .ok_or_else(invalid_inter_tile_scheduling_state)
            .and_then(fill);
        *published = if filled.is_ok() {
            GeometryState::Published(geometry)
        } else {
            GeometryState::Spare(geometry)
        };
        filled
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

    /// Lends one unit the decode's spent row buffers, so the decode holds a
    /// set per unit parsed and not yet committed rather than one per unit of
    /// every frame in flight.
    fn take_row_buffers(&self) -> ReconRowBuffers {
        let Some(shared) = &self.buffers else {
            return ReconRowBuffers::default();
        };
        shared.take_unit_buffers().unwrap_or_else(|| {
            let mut cold = ReconRowBuffers::default();
            cold.fit(shared.unit_lengths());
            cold
        })
    }

    pub(super) fn return_row_buffers(&self, mut buffers: ReconRowBuffers) {
        if let Some(shared) = &self.buffers {
            buffers.fit(shared.note_unit_lengths(buffers.lengths));
            shared.give_unit_buffers(buffers);
        }
    }

    /// The filter states the last merged tile spent.
    pub(super) fn take_spent_filters(&self) -> Option<(CdefState, GdfState, CcsoState)> {
        self.parser.lock().spent_filters.take()
    }

    /// The cell a batch waits on for its own units.
    pub(crate) fn cell(&self) -> &splot_parallel::WatermarkCell {
        &self.finished
    }
}

/// Settles the frame geometry and publishes it, before anything reads a unit.
///
/// The scheduled reconstruction is built from this alone, and it is promoted
/// while the § 8.2 pass still runs, so this must be called before the walk is
/// promoted -- not as the pass's first act, which would race it. The raster
/// numbering needs every tile to start on a superblock.
pub(crate) fn publish_tile_geometry(
    tiles: &[DecodeTileWorkUnit],
    params: &TileWalkParams,
    parse_progress: &ParseProgress,
) -> Result<()> {
    let frame_rows = 0..params.mi_rows;
    let frame_cols = 0..params.mi_cols;
    let capacity = tile_unit_capacity(
        &frame_rows,
        &frame_cols,
        params.mi_rows,
        params.mi_cols,
        params.sb_h4,
    );
    parse_progress.reserve(capacity)?;
    parse_progress.publish_geometry(|geometry| {
        geometry.tiles.clear();
        geometry
            .tiles
            .try_reserve(tiles.len())
            .map_err(|_| inter_allocation!("inter frame tile geometry"))?;
        let mut units = 1;
        for tile in tiles {
            let mi_rows = tile.mi_row_range().start as usize..tile.mi_row_range().end as usize;
            let mi_cols = tile.mi_col_range().start as usize..tile.mi_col_range().end as usize;
            if mi_rows.start % params.sb_h4 != 0 || mi_cols.start % params.sb_h4 != 0 {
                return Err(invalid_inter_tile_scheduling_state());
            }
            units += tile_unit_capacity(
                &mi_rows,
                &mi_cols,
                params.mi_rows,
                params.mi_cols,
                params.sb_h4,
            ) - 1;
            geometry.tiles.push(GeometryTile {
                tile_offset: tile.tile_byte_span().start,
                mi_rows,
                mi_cols,
            });
        }
        if units != capacity {
            return Err(invalid_inter_tile_scheduling_state());
        }
        geometry.mi_rows = frame_rows;
        geometry.mi_cols = frame_cols;
        geometry.unit_count = capacity;
        Ok(())
    })
}

impl<'payload> TileParser<'payload> {
    /// Opens tile `index` of a scheduled frame of `tiles` tiles.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn scheduled<T: ReconSample>(
        tile: &mut DecodeTileWorkUnit,
        tile_bytes: &'payload [u8],
        context: &TileDecodeContext<'_, T>,
        cdef_state: CdefState,
        gdf_state: GdfState,
        ccso_state: CcsoState,
        parse_progress: &ParseProgress,
        (index, tiles): (usize, usize),
    ) -> Result<Self> {
        let unit_count = parse_progress
            .geometry()
            .ok_or_else(invalid_inter_tile_scheduling_state)?
            .unit_count;
        let mut parser = Self::new(
            tile,
            tile_bytes,
            context,
            cdef_state,
            gdf_state,
            ccso_state,
            core::mem::take(&mut *parse_progress.parser.lock()),
        )?;
        parser.mv_grid.keep_edges_only();
        let sb_h4 = context.params.sb_h4;
        let rows = tile.mi_row_range().start as usize..tile.mi_row_range().end as usize;
        let cols = tile.mi_col_range().start as usize..tile.mi_col_range().end as usize;
        let tile_sb_cols = cols
            .end
            .min(context.params.mi_cols)
            .saturating_sub(cols.start)
            .div_ceil(sb_h4);
        parser.numbering = Some(UnitNumbering {
            sb_row0: rows.start / sb_h4,
            sb_col0: cols.start / sb_h4,
            tile_sb_cols: tile_sb_cols.max(1),
            frame_sb_cols: context.params.mi_cols.div_ceil(sb_h4),
            tile_sbs: tile_unit_capacity(
                &rows,
                &cols,
                context.params.mi_rows,
                context.params.mi_cols,
                sb_h4,
            ) - 1,
            trailing: (index + 1 == tiles).then(|| unit_count.saturating_sub(1)),
        });
        parser.tile = index;
        Ok(parser)
    }

    /// Parses and publishes the next unit, returning whether the tile ended.
    pub(super) fn parse_scheduled_unit<T: ReconSample>(
        &mut self,
        tile: &mut DecodeTileWorkUnit,
        context: &TileDecodeContext<'_, T>,
        parse_progress: &ParseProgress,
    ) -> Result<bool> {
        let row_set = parse_progress.take_row_buffers();
        let (mut row, last) = match self.next_unit(tile, context, Some(row_set), false) {
            ParserStep::More(row) => (row, false),
            ParserStep::Last(row) => (row, true),
        };
        row.return_terminal_error()?;
        if last
            && self
                .numbering
                .is_some_and(|numbering| numbering.trailing.is_none())
        {
            if !row.superblocks.is_empty() {
                return Err(invalid_inter_tile_scheduling_state());
            }
            parse_progress.recycle_unpublished(row);
        } else {
            parse_progress.publish_row(row)?;
        }
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

#[cfg(test)]
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

pub(crate) fn invalid_inter_tile_scheduling_state() -> crate::DecodeError {
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
        mut pending_rows,
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
            &mut parse_state.commit_block_decoded,
            context.sequence.general.chroma_format_idc,
            &context.params,
            tile.mi_col_range().end as usize,
            tile.mi_row_range().end as usize,
        )?;
        let commit_block_decoded = core::mem::take(&mut parse_state.commit_block_decoded);
        let reusable_surface_source = spent_surface_source.take();
        let quantizer = FrameQuantizerSnapshot::capture();
        let rows = tile.mi_row_range().start as usize..tile.mi_row_range().end as usize;
        let cols = tile.mi_col_range().start as usize..tile.mi_col_range().end as usize;
        let unit_count = tile_unit_capacity(&rows, &cols, mi_rows, mi_cols, sb_h4);
        let tile_mi_rows = rows;
        let tile_mi_cols = cols;
        parse_state.row_buffers.reset(admission::PARSE_AHEAD_ROWS);
        let row_buffers = core::mem::take(&mut parse_state.row_buffers);
        let (tile_cdef, tile_gdf, tile_ccso) = tile_filter_states(
            (&cdef_state, &gdf_state, &ccso_state),
            tile,
            parse_state.spent_filters.take(),
        )?;
        let mut parser = TileParser::new(
            tile,
            tile_bytes,
            &context,
            tile_cdef,
            tile_gdf,
            tile_ccso,
            parse_state,
        )?;
        let mut resolve_state = TileResolveState::new(sequence);
        let info = workspace.info();
        let mut surface_source = reusable_surface_source
            .filter(|source| std::sync::Arc::strong_count(source) == 1)
            .unwrap_or_else(|| {
                std::sync::Arc::new(admission::SurfaceSource::new(
                    info,
                    Vec::new(),
                    workers.spare_surfaces(),
                ))
            });
        let Some(source) = std::sync::Arc::get_mut(&mut surface_source) else {
            return Err(invalid_inter_tile_scheduling_state());
        };
        let rects = source.reset(info, workers.spare_surfaces());
        rects.clear();
        let commit = TileCommit::direct(
            ordered,
            workspace,
            commit_block_decoded,
            decoded_any,
            core::mem::take(frame_filter_records),
        );
        let commit = admission::run_ordinary_tile(
            &mut parser,
            tile,
            &mut resolve_state,
            tile_offset,
            unit_count,
            &context,
            temporal_context,
            &quantizer,
            &row_gate,
            &row_buffers,
            &motion,
            &mut pending_rows,
            commit,
        )?;
        let (next_ordered, next_workspace, next_decoded, next_records, spent_block_decoded) =
            commit.finish_direct();
        ordered = next_ordered;
        workspace = next_workspace;
        decoded_any = next_decoded;
        *frame_filter_records = next_records;
        let (output, next_parse_state) = parser.into_output();
        parse_state = next_parse_state;
        pixel_commit::take_tile_filter_records(
            frame_filter_records,
            &mut parse_state.filter_records,
        );
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
        parse_state.spent_filters = Some((output.cdef_state, output.gdf_state, output.ccso_state));
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
            pending_rows,
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

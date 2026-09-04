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
pub(crate) use admission::ScheduledTileRecon;
use admission::TileCommit;
pub(super) use admission::prepare_scheduled_tile;

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
    pub(super) segment_ids: FrameSegmentIdMap,
    pub(super) motion_field: TemporalMotionField,
}

/// Folds one tile's walk-parsed filter grids into the frame-level state.
#[allow(clippy::too_many_arguments)]
fn merge_tile_filter_state(
    cdef_state: &mut CdefState,
    gdf_state: &mut GdfState,
    ccso_state: &mut CcsoState,
    segment_ids: &mut FrameSegmentIdMap,
    tile: &TileParserOutput,
    mi_rows: Range<usize>,
    mi_cols: Range<usize>,
) -> Result<()> {
    cdef_state.merge_tile(&tile.cdef_state, mi_rows.clone(), mi_cols.clone())?;
    gdf_state.merge_tile(&tile.gdf_state, mi_rows.clone(), mi_cols.clone())?;
    ccso_state.merge_tile(&tile.ccso_state, mi_rows, mi_cols)?;
    segment_ids.merge_tile(&tile.segment_id_state);
    Ok(())
}

fn append_lr_records(
    blocks: &mut Vec<crate::bitstream::tile_payload::WienerNsLrSourceBlock>,
    filters: &mut Vec<crate::bitstream::tile_payload::WienerNsLrUnitFilter>,
    mut tile_blocks: Vec<crate::bitstream::tile_payload::WienerNsLrSourceBlock>,
    mut tile_filters: Vec<crate::bitstream::tile_payload::WienerNsLrUnitFilter>,
) -> Result<()> {
    let filter_base = filters.len();
    for block in &tile_blocks {
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

    for block in &mut tile_blocks {
        if let Some(index) = block.unit_filter_index {
            block.unit_filter_index = Some(
                filter_base
                    .checked_add(index)
                    .ok_or(crate::DecodeHeaderStateError::InvalidLoopRestorationFilterState)?,
            );
        }
    }
    blocks.append(&mut tile_blocks);
    filters.append(&mut tile_filters);
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
    fn context<'a, T: ReconSample>(
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
            limits: self.limits,
            mi_rows: self.mi_rows,
            mi_cols: self.mi_cols,
            sb_h4: self.sb_h4,
            max_drl_bits_minus_1: self.max_drl_bits_minus_1,
            frame_interpolation_filter: self.frame_interpolation_filter,
            residual_tool_policy: self.residual_tool_policy,
            num_total_refs: self.num_total_refs,
            reference_select: self.reference_select,
            num_same_ref_compound: self.num_same_ref_compound,
            luma_use_tcq: self.luma_use_tcq,
            residual_use_ddt: self.residual_use_ddt,
            bit_depth: self.bit_depth,
            enable_adaptive_mvd: self.enable_adaptive_mvd,
            allow_bawp: self.allow_bawp,
            allow_warpmv_mode: self.allow_warpmv_mode,
            frame_is_switch: self.frame_is_switch,
            current_order_hint: self.current_order_hint,
            tip_ref_pair: self.tip_ref_pair,
        }
    }
}

struct TileDecodeContext<'a, T: ReconSample> {
    sequence: &'a SequenceHeader,
    core: &'a FrameHeaderCore,
    limits: crate::DecodeLimits,
    mi_rows: usize,
    mi_cols: usize,
    sb_h4: usize,
    max_drl_bits_minus_1: u32,
    frame_interpolation_filter: FrameInterpolationFilter,
    residual_tool_policy: TransformToolResidualPolicy,
    num_total_refs: usize,
    reference_select: bool,
    num_same_ref_compound: u8,
    reference: &'a InterReferenceState<T>,
    luma_use_tcq: bool,
    residual_use_ddt: bool,
    ref_frame_idx: &'a [u32],
    bit_depth: BitDepth,
    enable_adaptive_mvd: bool,
    allow_bawp: bool,
    allow_warpmv_mode: bool,
    frame_is_switch: bool,
    current_order_hint: u32,
    tip_ref_pair: Option<(i8, i8)>,
}

struct TileParser<'tile, 'payload> {
    tile: &'tile mut DecodeTileWorkUnit<'payload>,
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
    active_source_blocks: Vec<crate::bitstream::tile_payload::WienerNsLrSourceBlock>,
    unit_filters: Vec<crate::bitstream::tile_payload::WienerNsLrUnitFilter>,
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

impl<'tile, 'payload> TileParser<'tile, 'payload> {
    fn new<T: ReconSample>(
        tile: &'tile mut DecodeTileWorkUnit<'payload>,
        context: &TileDecodeContext<'_, T>,
        cdef_state: CdefState,
        gdf_state: GdfState,
        ccso_state: CcsoState,
    ) -> Result<Self> {
        let tile_offset = tile.tile_byte_span().start;
        let chroma = context.sequence.general.chroma_format_idc;
        let tile_rows = tile.mi_row_range().start as usize
            ..(tile.mi_row_range().end as usize).min(context.mi_rows);
        let tile_cols = tile.mi_col_range().start as usize
            ..(tile.mi_col_range().end as usize).min(context.mi_cols);
        let coeff_ctx = TileCoeffContextState::new_for_tile_chroma(
            tile_rows.clone(),
            tile_cols.clone(),
            chroma,
        )
        .map_err(|error| inter_tile_coeff_context_error(&error))?;
        let delta_q_state = DeltaQState::new(context.sequence, context.core)?;
        let intrabc_state = TileIntrabcPreludeState::new_for_tile(
            (context.mi_rows, context.mi_cols),
            tile_rows.clone(),
            tile_cols.clone(),
            context.sequence,
            context.core.frame_is_intra == Some(true),
            crate::filters::wienerns_lr::intrabc_records::frame_allows_intrabc(context.core),
        )?;
        let segment_id_state =
            TileSegmentIdState::new_for_tile(tile_rows.clone(), tile_cols.clone())
                .map_err(|error| inter_tile_segment_id_error(&error))?;
        let mv_grid = NeighbourMvGrid::new_for_tile(tile_rows.clone(), tile_cols.clone())
            .map_err(|error| inter_tile_grid_error(&error, "inter parser MV grid"))?;
        let y_smooth = crate::prediction::intra_edge::TileYSmoothGrid::new_for_tile(
            tile_rows.clone(),
            tile_cols.clone(),
        )
        .map_err(|error| inter_tile_grid_error(&error, "inter luma smooth grid"))?;
        let (chroma_rows, chroma_cols) =
            super::chroma_smooth_tile_ranges(tile_rows, tile_cols, chroma);
        let chroma_smooth = crate::prediction::intra_edge::TileChromaSmoothGrid::new_for_tile(
            chroma_rows,
            chroma_cols,
        )
        .map_err(|error| inter_tile_grid_error(&error, "inter chroma smooth grid"))?;
        let walk =
            GeneralIntraMultiblockCursor::new(tile, context.sequence, context.core, context.limits)
                .map_err(|error| {
                    map_inter_multiblock_error(
                        GeneralIntraMultiblockError::<crate::DecodeError>::Setup(error),
                        tile_offset,
                    )
                })?;
        Ok(Self {
            tile,
            walk: TileParserWalk::Active(walk),
            coeff_ctx,
            residual_scratch: InterResidualParseScratch::default(),
            delta_q_state,
            intrabc_state,
            mv_grid,
            y_smooth,
            chroma_smooth,
            filter_records: TileFilterRecords::default(),
            residual_planes: crate::residual::pipeline::ResidualPlaneArena::new(),
            output: TileParserOutput {
                cdef_state,
                gdf_state,
                ccso_state,
                segment_id_state,
                active_source_blocks: Vec::new(),
                unit_filters: Vec::new(),
            },
            parser_ordinal: 0,
        })
    }

    fn next_unit<T: ReconSample>(
        &mut self,
        context: &TileDecodeContext<'_, T>,
        buffers: Option<ReconRowBuffers>,
    ) -> ParserStep<ReconRow> {
        let tile_offset = self.tile.tile_byte_span().start;
        let ReconRowBuffers {
            superblocks,
            entries,
            residual_blocks,
            temporal,
            motion_grids,
            flag_log,
            filter_records,
            residual_planes,
        } = buffers.unwrap_or_default();
        self.filter_records = filter_records;
        self.residual_planes = residual_planes;
        let mut recon_row = ReconRow {
            ordinal: self.parser_ordinal,
            superblocks,
            entries,
            residual_blocks,
            temporal,
            motion_grids,
            flag_log,
            filter_records: TileFilterRecords::default(),
            residual_planes: crate::residual::pipeline::ResidualPlaneArena::new(),
            motion_folded: false,
            motion_derived: false,
            failure: ReconRowFailure::None,
        };
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
                |work_unit: &mut DecodeTileWorkUnit<'_>,
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
                        &mut self.output.gdf_state,
                        &mut self.output.cdef_state,
                        &mut self.output.ccso_state,
                        &mut self.delta_q_state,
                        &mut self.intrabc_state,
                        &mut self.output.segment_id_state,
                        &mut self.mv_grid,
                        context.tip_ref_pair,
                        &mut self.y_smooth,
                        &mut self.chroma_smooth,
                        context.sb_h4,
                        context.mi_rows,
                        context.mi_cols,
                        context.max_drl_bits_minus_1,
                        context.frame_interpolation_filter,
                        context.residual_tool_policy,
                        context.num_total_refs,
                        context.reference_select,
                        context.num_same_ref_compound,
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
                        context.luma_use_tcq,
                        context.residual_use_ddt,
                        context.ref_frame_idx,
                        context.reference,
                        context.bit_depth,
                        context.enable_adaptive_mvd,
                        context.allow_bawp,
                        context.allow_warpmv_mode,
                        context.frame_is_switch,
                        context.current_order_hint,
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
                        state: Some(ReconEntryState::Resolve(Some(resolve))),
                        motion: None,
                        temporal: 0..0,
                    },
                );
            };
            walk.decode_next_superblock(self.tile, &mut decode_leaf, &mut on_published)
                .map(|superblock| superblock.is_some())
        };
        crate::bitstream::tile_payload::coeff_arena::seal();
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
                    active_source_blocks,
                    unit_filters,
                } = walk.into_output();
                if let Err(error) = finish_tile_symbols(symbols, tile_offset) {
                    recon_row.record_terminal_error(error);
                }
                self.output.active_source_blocks = active_source_blocks;
                self.output.unit_filters = unit_filters;
                ParserStep::Last(recon_row)
            }
        }
    }

    fn into_output(self) -> TileParserOutput {
        self.output
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
                max_drl_bits_minus_1: context.max_drl_bits_minus_1,
                frame_precision: 0,
                tile_offset,
            },
            context.sb_h4,
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
    Resolve(Option<LeafResolveRecord>),
    Command(Option<ReconCommand>),
}

impl ReconRowEntry {
    pub(super) fn command(&self) -> Option<&ReconCommand> {
        match self.state.as_ref()? {
            ReconEntryState::Command(Some(command))
            | ReconEntryState::Resolve(Some(
                LeafResolveRecord::Reseed(command) | LeafResolveRecord::NonInter { command, .. },
            )) => Some(command),
            ReconEntryState::Resolve(
                Some(LeafResolveRecord::Inter(_) | LeafResolveRecord::Intrabc(_)) | None,
            )
            | ReconEntryState::Command(None) => None,
        }
    }

    pub(super) fn take_resolve(&mut self) -> Option<LeafResolveRecord> {
        self.take_state(ReconEntryState::as_resolve)
    }

    pub(super) fn store_command(&mut self, command: ReconCommand) {
        self.state = Some(ReconEntryState::Command(Some(command)));
    }

    pub(super) fn take_command(&mut self) -> Option<ReconCommand> {
        self.take_state(ReconEntryState::as_command)
    }

    fn take_state<T>(
        &mut self,
        extract: impl FnOnce(&mut ReconEntryState) -> Option<T>,
    ) -> Option<T> {
        let state = self.state.as_mut()?;
        let value = extract(state)?;
        self.state = None;
        Some(value)
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
        grid: Option<super::super::mc::CompoundMotionGrid>,
        grids: &mut Vec<Option<super::super::mc::CompoundMotionGrid>>,
    ) {
        self.motion = grid.and_then(|grid| {
            grids.push(Some(grid));
            NonZeroUsize::new(grids.len())
        });
    }

    pub(super) fn take_motion(
        &mut self,
        grids: &mut [Option<super::super::mc::CompoundMotionGrid>],
    ) -> Option<super::super::mc::CompoundMotionGrid> {
        let index = self.motion.take()?.get().checked_sub(1)?;
        grids.get_mut(index)?.take()
    }
}

impl ReconEntryState {
    fn as_resolve(&mut self) -> Option<LeafResolveRecord> {
        match self {
            Self::Resolve(resolve) => resolve.take(),
            Self::Command(_) => None,
        }
    }

    fn as_command(&mut self) -> Option<ReconCommand> {
        match self {
            Self::Command(command) => command.take(),
            Self::Resolve(_) => None,
        }
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

pub(super) struct ReconRow {
    pub(super) ordinal: usize,
    pub(super) superblocks: Vec<ReconSuperblock>,
    pub(super) entries: Vec<ReconRowEntry>,
    pub(super) residual_blocks: Vec<InterResidualBlock>,
    pub(super) temporal: Vec<TemporalMotionBlock>,
    pub(super) motion_grids: Vec<Option<super::super::mc::CompoundMotionGrid>>,
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
pub(super) struct ReconRowBuffers {
    pub(super) superblocks: Vec<ReconSuperblock>,
    pub(super) entries: Vec<ReconRowEntry>,
    pub(super) residual_blocks: Vec<InterResidualBlock>,
    pub(super) temporal: Vec<TemporalMotionBlock>,
    pub(super) motion_grids: Vec<Option<super::super::mc::CompoundMotionGrid>>,
    pub(super) flag_log: Vec<NeighbourFlagRecord>,
    pub(super) filter_records: TileFilterRecords,
    pub(super) residual_planes: crate::residual::pipeline::ResidualPlaneArena,
}

struct ReconRowBufferPool {
    available: Mutex<Vec<ReconRowBuffers>>,
}

/// Row buffer sets kept between units and frames.
///
/// Global, not per worker: a unit is parsed on one worker and replayed on
/// another, so a thread-local set is returned to a thread that will not ask
/// for it again. The cap is a whole-process one for the same reason the
/// frame-plane pool's is.
const MAX_RETAINED_ROW_BUFFERS: usize = 256;

static RETAINED_ROW_BUFFERS: Mutex<Vec<ReconRowBuffers>> = Mutex::new(Vec::new());

/// Takes a retained row buffer set, or a fresh one.
fn take_retained_row_buffers() -> ReconRowBuffers {
    RETAINED_ROW_BUFFERS.lock().pop().unwrap_or_default()
}

/// Returns a spent row buffer set, whose vectors are already cleared.
pub(super) fn retain_row_buffers(buffers: ReconRowBuffers) {
    let mut retained = RETAINED_ROW_BUFFERS.lock();
    if retained.len() < MAX_RETAINED_ROW_BUFFERS {
        retained.push(buffers);
    }
}

impl Drop for ReconRowBufferPool {
    fn drop(&mut self) {
        let available = core::mem::take(&mut *self.available.lock());
        for buffers in available {
            retain_row_buffers(buffers);
        }
    }
}

impl ReconRowBufferPool {
    fn new(slots: usize) -> Self {
        let mut available = Vec::with_capacity(slots);
        {
            let mut retained = RETAINED_ROW_BUFFERS.lock();
            while available.len() < slots {
                let Some(buffers) = retained.pop() else {
                    break;
                };
                available.push(buffers);
            }
        }
        available.resize_with(slots, ReconRowBuffers::default);
        Self {
            available: Mutex::new(available),
        }
    }

    fn take(&self) -> ReconRowBuffers {
        if let Some(buffers) = self.available.lock().pop() {
            return buffers;
        }
        take_retained_row_buffers()
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

struct InterReconScratchPool<T: ReconSample> {
    available: Mutex<Vec<deferred_recon::InterReconScratch<T>>>,
}

impl<T: ReconSample> InterReconScratchPool<T> {
    fn ensure_workers(&mut self, workers: usize) {
        let available = self.available.get_mut();
        if available.len() < workers {
            available.resize_with(workers, deferred_recon::InterReconScratch::default);
        }
    }

    fn with_scratch<R>(&self, f: impl FnOnce(&mut deferred_recon::InterReconScratch<T>) -> R) -> R {
        let mut scratch = self.available.lock().pop().unwrap_or_default();
        let result = f(&mut scratch);
        self.available.lock().push(scratch);
        result
    }

    fn take_reusable(&self) -> Self {
        let available = core::mem::take(&mut *self.available.lock());
        Self {
            available: Mutex::new(available),
        }
    }
}

impl<T: ReconSample> Default for InterReconScratchPool<T> {
    fn default() -> Self {
        Self {
            available: Mutex::new(Vec::new()),
        }
    }
}

#[derive(Default)]
pub(in crate::prediction::inter) struct TileDecodeScratch<T: ReconSample> {
    ordered: deferred_recon::InterReconScratch<T>,
    workers: InterReconScratchPool<T>,
    surfaces: Vec<splot_recon::OwnedFrameRect<T>>,
}

impl<T: ReconSample> TileDecodeScratch<T> {
    fn from_scheduled(
        ordered: deferred_recon::InterReconScratch<T>,
        workers: &InterReconScratchPool<T>,
        surfaces: Vec<splot_recon::OwnedFrameRect<T>>,
    ) -> Self {
        Self {
            ordered,
            workers: workers.take_reusable(),
            surfaces,
        }
    }

    fn clear_incompatible_surface_layout(
        &mut self,
        info: splot_recon::DecodedFrameInfo,
        rects: &[splot_recon::PlaneRect],
    ) {
        let complete_layout = self.surfaces.len() == rects.len()
            && self
                .surfaces
                .iter()
                .rev()
                .zip(rects)
                .all(|(surface, rect)| surface.info() == info && surface.luma_rect() == *rect);
        if !complete_layout {
            self.surfaces.clear();
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
    if row.has_terminal_error() {
        return row;
    }
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
            let start = row.temporal.len();
            let result = if row.motion_derived {
                scratch.reconstruct_from_motion(
                    &command,
                    surface,
                    block_decoded,
                    entry.take_motion(&mut row.motion_grids),
                    &row.residual_blocks,
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
            } else {
                scratch.reconstruct_logged(
                    &command,
                    surface,
                    block_decoded,
                    &mut row.temporal,
                    &row.residual_blocks,
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
            if let Some(clear) = entry.temporal_clear_record(mi_rows, mi_cols, current_order_hint) {
                let start = row.temporal.len();
                row.temporal.push(clear);
                entry.temporal = start..row.temporal.len();
            }
        }
        motion.fold_unit(row.ordinal, &row.temporal);
        motion.unit_landed_for(row.ordinal);
    }
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
    mi_rows: Range<usize>,
    mi_cols: Range<usize>,
    unit_count: usize,
    output: TileParserOutput,
}

impl ParsedTile {
    /// How many unit buffers the tile is holding, which bounds the split
    /// path's per-frame memory.
    pub(super) const fn unit_count(&self) -> usize {
        self.unit_count
    }

    /// Folds the tile's walk-parsed filter grids and loop-restoration records
    /// into the frame-level state, which the entropy pass alone settles.
    ///
    /// The fused walk does this once the tile's reconstruction is done; a split
    /// walk does it at the end of the parse pass instead, so the driver already
    /// holds the frame's filter grids while the reconstruction is still owed.
    /// The two write disjoint parts of the frame's records.
    pub(super) fn merge_filter_state(
        &mut self,
        frame_filter_records: &mut crate::filters::wienerns_lr::FrameFilterRecords,
        cdef_state: &mut CdefState,
        gdf_state: &mut GdfState,
        ccso_state: &mut CcsoState,
        segment_ids: &mut FrameSegmentIdMap,
    ) -> Result<()> {
        let output = &mut self.output;
        merge_tile_filter_state(
            cdef_state,
            gdf_state,
            ccso_state,
            segment_ids,
            output,
            self.mi_rows.clone(),
            self.mi_cols.clone(),
        )?;
        append_lr_records(
            &mut frame_filter_records.lr_source_blocks,
            &mut frame_filter_records.lr_unit_filters,
            core::mem::take(&mut output.active_source_blocks),
            core::mem::take(&mut output.unit_filters),
        )?;
        Ok(())
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

/// Monotone count of § 8.2 parse units a tile has finished, published as the
/// parser produces them.
///
/// Units are emitted in order, so a consumer can gate on the prefix it needs
/// instead of on the whole tile. The cell is the scheduler's own watermark, so
/// a batch waits through `Condition::watermark` rather than through a poll.
/// The tile geometry the § 8.2 parser settles before it reads its first
/// unit, which is everything the admission scheduler needs to lay out
/// batches and surfaces.
pub(crate) struct TileGeometry {
    pub(super) tile_offset: ByteOffset,
    pub(super) mi_rows: Range<usize>,
    pub(super) mi_cols: Range<usize>,
    pub(super) block_decoded: TileBlockDecodedState,
    pub(super) unit_count: usize,
}

#[derive(Default)]
pub(crate) struct ParseProgress {
    finished: splot_parallel::WatermarkCell,
    rows: Mutex<Vec<Option<ReconRow>>>,
    geometry: Mutex<Option<Arc<TileGeometry>>>,
    records: Mutex<crate::filters::wienerns_lr::FrameFilterRecords>,
}

impl ParseProgress {
    /// Hands one finished unit to the scheduler and publishes the new count.
    ///
    /// The frame's § 7.17 and loop-restoration records leave the unit here, in
    /// parse order, because the scheduler claims units on its own schedule and
    /// the frame-level detach must not depend on when it does.
    pub(super) fn publish_row(&self, mut row: ReconRow) {
        pixel_commit::detach_row_filter_records(&mut row, &mut self.records.lock());
        let finished = {
            let mut rows = self.rows.lock();
            rows.push(Some(row));
            rows.len()
        };
        self.finished.publish(finished);
    }

    /// Takes the unit at `index`, which a caller may claim exactly once.
    pub(super) fn take_row(&self, index: usize) -> Option<ReconRow> {
        self.rows.lock().get_mut(index).and_then(Option::take)
    }

    /// Takes the filter records detached from units already handed out.
    pub(super) fn take_records(&self) -> crate::filters::wienerns_lr::FrameFilterRecords {
        core::mem::take(&mut self.records.lock())
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
    pub(super) fn publish_geometry(&self, geometry: TileGeometry) {
        *self.geometry.lock() = Some(Arc::new(geometry));
    }

    /// The published tile geometry, if the parser has reached its first unit.
    pub(super) fn geometry(&self) -> Option<Arc<TileGeometry>> {
        self.geometry.lock().clone()
    }

    /// Releases every waiter after a failed pass.
    ///
    /// Batches and resolve steps wait on unit thresholds the pass will now
    /// never reach, so the watermark is driven past all of them.
    pub(crate) fn fail(&self) {
        self.finished.publish(splot_parallel::WatermarkCell::FAILED);
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
pub(crate) fn publish_tile_geometry<T: ReconSample>(
    tile: &DecodeTileWorkUnit<'_>,
    params: &TileWalkParams,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    reference: &InterReferenceState<T>,
    ref_frame_idx: &[u32],
    parse_progress: &ParseProgress,
) -> Result<()> {
    let context = &params.context(sequence, core, reference, ref_frame_idx);
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
    parse_progress.publish_geometry(TileGeometry {
        tile_offset: tile.tile_byte_span().start,
        mi_rows,
        mi_cols,
        block_decoded: tile_block_decoded(tile, context)?,
        unit_count: capacity,
    });
    Ok(())
}

/// Runs one tile's entropy pass to the end, keeping every unit.
///
/// Each unit carries the flag-plane publications it made, so a resolve pass on
/// another grid replays exactly what the fused walk published before resolving
/// the same unit.
#[allow(clippy::too_many_arguments)]
pub(super) fn parse_tile_units<T: ReconSample>(
    tile: &mut DecodeTileWorkUnit<'_>,
    params: &TileWalkParams,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    reference: &InterReferenceState<T>,
    ref_frame_idx: &[u32],
    cdef_state: &CdefState,
    gdf_state: &GdfState,
    ccso_state: &CcsoState,
    parse_progress: &Arc<ParseProgress>,
) -> Result<ParsedTile> {
    let context = &params.context(sequence, core, reference, ref_frame_idx);
    let geometry = parse_progress
        .geometry()
        .ok_or_else(invalid_inter_tile_scheduling_state)?;
    let mi_rows = geometry.mi_rows.clone();
    let mi_cols = geometry.mi_cols.clone();
    let mut unit_count = 0usize;
    let mut parser = TileParser::new(
        tile,
        context,
        cdef_state.try_for_tile(mi_rows.clone(), mi_cols.clone())?,
        gdf_state.for_tile(mi_rows.clone(), mi_cols.clone())?,
        ccso_state.try_for_tile(mi_rows.clone(), mi_cols.clone())?,
    )?;
    parser.mv_grid.log_flags();
    loop {
        let step = parser.next_unit(context, Some(take_retained_row_buffers()));
        let (mut row, last) = match step {
            ParserStep::More(row) => (row, false),
            ParserStep::Last(row) => (row, true),
        };
        row.return_terminal_error()?;
        unit_count += 1;
        parse_progress.publish_row(row);
        if last {
            break;
        }
    }
    Ok(ParsedTile {
        mi_rows,
        mi_cols,
        unit_count,
        output: parser.into_output(),
    })
}

fn tile_block_decoded<T: ReconSample>(
    tile: &DecodeTileWorkUnit<'_>,
    context: &TileDecodeContext<'_, T>,
) -> Result<TileBlockDecodedState> {
    let chroma = context.sequence.general.chroma_format_idc;
    let (subsampling_x, subsampling_y) = chroma_subsampling(chroma);
    TileBlockDecodedState::new(
        if chroma == ChromaFormatIdc::Monochrome {
            1
        } else {
            3
        },
        usize::from(subsampling_x),
        usize::from(subsampling_y),
        context.sb_h4,
        (tile.mi_col_range().end as usize).min(context.mi_cols),
        (tile.mi_row_range().end as usize).min(context.mi_rows),
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
    let bounds = luma_rect(mi_rows, mi_cols, workspace)?;
    let side = sb_h4 * 4;
    let rows = bounds.height().div_ceil(side);
    let cols = bounds.width().div_ceil(side);
    let count = rows * cols;
    let mut rects = Vec::new();
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
    Ok(rects)
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
    work_units: &mut [DecodeTileWorkUnit<'_>],
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
) -> Result<(
    TileDecodeScratch<T>,
    CurrentFrameWorkspace<T>,
    TileDecodeOutput,
)> {
    let TileDecodeScratch {
        mut ordered,
        mut workers,
        surfaces: mut recycled_surfaces,
    } = scratch;
    workers.ensure_workers(
        splot_parallel::current_pool_width()
            .saturating_sub(1)
            .max(1),
    );
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
    let mut segment_ids = frame_segment_id_map(mi_rows, mi_cols)?;
    let row_gate = row_gate::RowReferenceGate::new(
        reference,
        core,
        ref_frame_idx,
        workspace.info(),
        temporal_context,
    );
    let global_intrabc = super::intrabc::global_intrabc_enabled(core.intrabc);
    for tile in work_units.iter_mut() {
        let tile_offset = tile.tile_byte_span().start;
        let block_decoded = tile_block_decoded(tile, &context)?;
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
        let mut parser = TileParser::new(
            tile,
            &context,
            cdef_state.try_for_tile(tile_mi_rows.clone(), tile_mi_cols.clone())?,
            gdf_state.for_tile(tile_mi_rows.clone(), tile_mi_cols.clone())?,
            ccso_state.try_for_tile(tile_mi_rows.clone(), tile_mi_cols.clone())?,
        )?;
        let mut resolve_state = TileResolveState::new(sequence);
        let row_buffers = ReconRowBufferPool::new(
            splot_parallel::current_pool_width()
                .saturating_mul(3)
                .max(1),
        );
        let mut tile_scratch = TileDecodeScratch {
            ordered,
            workers,
            surfaces: recycled_surfaces,
        };
        let info = workspace.info();
        let rects = if global_intrabc {
            Vec::new()
        } else {
            let superblock_rects =
                superblock_luma_rects(&tile_mi_rows, &tile_mi_cols, &workspace, sb_h4)?;
            tile_scratch.clear_incompatible_surface_layout(info, &superblock_rects);
            superblock_rects
        };
        let TileDecodeScratch {
            ordered: tile_ordered,
            workers: tile_workers,
            surfaces: tile_surfaces,
        } = tile_scratch;
        let surface_source = std::sync::Arc::new(Mutex::new(admission::SurfaceSource::new(
            info,
            rects,
            tile_surfaces,
        )));
        let commit = TileCommit::direct(
            tile_ordered,
            workspace,
            block_decoded.clone(),
            decoded_any,
            std::sync::Arc::clone(&surface_source),
            core::mem::take(frame_filter_records),
        );
        let commit = admission::run_ordinary_tile(
            &mut parser,
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
            &tile_workers,
            &block_decoded,
            &motion,
            commit,
        )?;
        let (next_ordered, next_workspace, next_decoded, next_surfaces, next_records) =
            commit.finish_direct();
        ordered = next_ordered;
        workspace = next_workspace;
        decoded_any = next_decoded;
        recycled_surfaces = next_surfaces;
        if !global_intrabc {
            recycled_surfaces.reverse();
        }
        *frame_filter_records = next_records;
        workers = tile_workers;
        let output = parser.into_output();
        merge_tile_filter_state(
            &mut cdef_state,
            &mut gdf_state,
            &mut ccso_state,
            &mut segment_ids,
            &output,
            tile_mi_rows,
            tile_mi_cols,
        )?;
        append_lr_records(
            &mut frame_filter_records.lr_source_blocks,
            &mut frame_filter_records.lr_unit_filters,
            output.active_source_blocks,
            output.unit_filters,
        )?;
    }
    if !decoded_any {
        return Err(no_decoded_block_error());
    }

    Ok((
        TileDecodeScratch {
            ordered,
            workers,
            surfaces: recycled_surfaces,
        },
        workspace,
        TileDecodeOutput {
            cdef_state,
            gdf_state,
            ccso_state,
            segment_ids,
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

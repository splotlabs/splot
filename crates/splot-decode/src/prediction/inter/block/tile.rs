// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Tile-local block decode and reconstruction.
//!
//! Feature tracking: `INFRA-DECODE-PARALLEL-STAGES`.

use std::ops::Range;
use std::sync::Mutex;

use splot_recon::{PlaneId, ReconError};

use super::*;

mod ready_rows;

use ready_rows::{
    OrderedDone, ParserStep, ReadyRowPipelineError, run_ready_row_pipeline_serial,
    run_ready_row_prepass_with_commit,
};

pub(super) struct TileDecodeOutput {
    pub(super) cdef_state: CdefState,
    pub(super) gdf_state: GdfState,
    pub(super) ccso_state: CcsoState,
    pub(super) motion_field: TemporalMotionField,
}

/// Folds one tile's walk-parsed filter grids into the frame-level state.
fn merge_tile_filter_state(
    cdef_state: &mut CdefState,
    gdf_state: &mut GdfState,
    ccso_state: &mut CcsoState,
    tile: &TileParserOutput,
    mi_rows: Range<usize>,
    mi_cols: Range<usize>,
    tile_offset: ByteOffset,
) -> Result<()> {
    cdef_state.merge_tile(
        &tile.cdef_state,
        mi_rows.clone(),
        mi_cols.clone(),
        tile_offset,
    )?;
    gdf_state.merge_tile(
        &tile.gdf_state,
        mi_rows.clone(),
        mi_cols.clone(),
        tile_offset,
    )?;
    ccso_state.merge_tile(&tile.ccso_state, mi_rows, mi_cols, tile_offset)
}

fn append_lr_records(
    blocks: &mut Vec<crate::bitstream::tile_payload::WienerNsLrSourceBlock>,
    filters: &mut Vec<crate::bitstream::tile_payload::WienerNsLrUnitFilter>,
    mut tile_blocks: Vec<crate::bitstream::tile_payload::WienerNsLrSourceBlock>,
    tile_filters: Vec<crate::bitstream::tile_payload::WienerNsLrUnitFilter>,
) -> Option<()> {
    let filter_base = filters.len();
    for block in &mut tile_blocks {
        if let Some(index) = block.unit_filter_index {
            if index >= tile_filters.len() {
                return None;
            }
            block.unit_filter_index = Some(filter_base.checked_add(index)?);
        }
    }
    blocks.extend(tile_blocks);
    filters.extend(tile_filters);
    Some(())
}

#[derive(Default)]
pub(super) struct TileFilterRecords {
    pub(super) deblock_blocks: Vec<crate::filters::deblock::DeblockBlock>,
    pub(super) chroma_deblock_blocks: [Vec<crate::filters::deblock::DeblockBlock>; 2],
    pub(super) tx_skip_records: Vec<crate::filters::wienerns_lr::WienerNsLrTxSkipTransformRecord>,
}

/// The frame-level facts every tile phase reads, all owned so a resolve pass
/// that runs after the driver moved on can rebuild its tile context.
#[derive(Clone, Copy)]
pub(super) struct TileWalkParams {
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
    walk: Option<GeneralIntraMultiblockCursor<'payload>>,
    coeff_ctx: TileCoeffContextState,
    residual_scratch: InterResidualParseScratch,
    delta_q_state: DeltaQState,
    intrabc_state: TileIntrabcPreludeState,
    segment_id_state: TileSegmentIdState,
    mv_grid: NeighbourMvGrid,
    y_smooth: crate::prediction::intra_edge::TileYSmoothGrid,
    chroma_smooth: crate::prediction::intra_edge::TileChromaSmoothGrid,
    cdef_state: CdefState,
    gdf_state: GdfState,
    ccso_state: CcsoState,
    filter_records: TileFilterRecords,
    tile_walk_output: Option<(
        Vec<crate::bitstream::tile_payload::WienerNsLrSourceBlock>,
        Vec<crate::bitstream::tile_payload::WienerNsLrUnitFilter>,
    )>,
    parser_ordinal: usize,
    entry_capacity: usize,
    superblock_capacity: usize,
}

struct TileParserOutput {
    cdef_state: CdefState,
    gdf_state: GdfState,
    ccso_state: CcsoState,
    active_source_blocks: Vec<crate::bitstream::tile_payload::WienerNsLrSourceBlock>,
    unit_filters: Vec<crate::bitstream::tile_payload::WienerNsLrUnitFilter>,
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
        .map_err(|_| {
            inter_cap!(
                "inter_coeff_context_state",
                tile_offset,
                "inter.residual_context_state",
                SPEC_MODE_INFO
            )
        })?;
        let delta_q_state = DeltaQState::new(context.sequence, context.core, tile_offset)?;
        let intrabc_state = TileIntrabcPreludeState::new_for_tile(
            (context.mi_rows, context.mi_cols),
            tile_rows.clone(),
            tile_cols.clone(),
            context.sequence,
            context.core.frame_is_intra == Some(true),
            crate::filters::wienerns_lr::intrabc_records::frame_allows_intrabc(context.core),
            tile_offset,
        )?;
        let segment_id_state =
            TileSegmentIdState::new_for_tile(tile_rows.clone(), tile_cols.clone()).map_err(
                |_| {
                    inter_missing!(
                        "inter_segment_id_grid",
                        tile_offset,
                        "inter.segment_id_grid",
                        SPEC_MODE_INFO
                    )
                },
            )?;
        let mv_grid = NeighbourMvGrid::new_for_tile(tile_rows.clone(), tile_cols.clone())
            .ok_or_else(|| {
                inter_cap!(
                    "inter_mv_grid",
                    tile_offset,
                    "inter.mv_grid",
                    SPEC_MODE_INFO
                )
            })?;
        let y_smooth = crate::prediction::intra_edge::TileYSmoothGrid::new_for_tile(
            tile_rows.clone(),
            tile_cols.clone(),
        )
        .ok_or_else(|| {
            inter_cap!(
                "inter_y_smooth_grid",
                tile_offset,
                "inter.y_smooth_grid",
                SPEC_MODE_INFO
            )
        })?;
        let (chroma_rows, chroma_cols) =
            super::chroma_smooth_tile_ranges(tile_rows, tile_cols, chroma);
        let chroma_smooth = crate::prediction::intra_edge::TileChromaSmoothGrid::new_for_tile(
            chroma_rows,
            chroma_cols,
        )
        .ok_or_else(|| {
            inter_cap!(
                "inter_chroma_smooth_grid",
                tile_offset,
                "inter.chroma_smooth_grid",
                SPEC_MODE_INFO
            )
        })?;
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
            walk: Some(walk),
            coeff_ctx,
            residual_scratch: InterResidualParseScratch::default(),
            delta_q_state,
            intrabc_state,
            segment_id_state,
            mv_grid,
            y_smooth,
            chroma_smooth,
            cdef_state,
            gdf_state,
            ccso_state,
            filter_records: TileFilterRecords::default(),
            tile_walk_output: None,
            parser_ordinal: 0,
            entry_capacity: 0,
            superblock_capacity: 0,
        })
    }

    fn next_unit<T: ReconSample>(
        &mut self,
        context: &TileDecodeContext<'_, T>,
        granularity: ParserGranularity,
        buffers: Option<ReconRowBuffers>,
    ) -> ParserStep<ReconRow> {
        let _row_phase = crate::timing::WalkPhaseScope::new(crate::timing::WalkPhase::Row);
        let tile_offset = self.tile.tile_byte_span().start;
        let tile_cols = self.tile.mi_col_range();
        let row_superblocks = (tile_cols.end as usize)
            .saturating_sub(tile_cols.start as usize)
            .div_ceil(context.sb_h4);
        let ReconRowBuffers {
            superblocks,
            entries,
            motion_queue,
            pending_inter,
            residual_blocks,
            temporal,
            flag_log,
            filter_records,
        } = buffers.unwrap_or_default();
        self.filter_records = filter_records;
        let mut recon_row = ReconRow {
            ordinal: self.parser_ordinal,
            superblocks,
            entries,
            motion_queue,
            pending_inter,
            residual_blocks,
            temporal,
            flag_log,
            filter_records: TileFilterRecords::default(),
            terminal: None,
        };
        self.parser_ordinal = self.parser_ordinal.saturating_add(1);
        let decoded_row = if let Some(walk) = self.walk.as_mut() {
            let mut decode_leaf =
                |work_unit: &mut DecodeTileWorkUnit<'_>,
                 symbols: &mut SymbolDecoder<'_>,
                 frontier: &DecodeBlockFrontier,
                 joint_modes: &TileIntraJointModeState,
                 uses_mrls: &TileUsesMrlsState,
                 use_dip: &crate::bitstream::tile_payload::TileUseDipState,
                 fsc_modes: &TileFscModeState,
                 palette_state: &crate::bitstream::tile_payload::TileLumaPaletteState,
                 is_cfl_ctx: IsCflContext,
                 _decoded_leaf: DecodedLeafPublication| {
                    decode_block(
                        work_unit,
                        symbols,
                        frontier,
                        context.sequence,
                        context.core,
                        &mut self.coeff_ctx,
                        &mut self.residual_scratch,
                        &mut recon_row.residual_blocks,
                        &mut self.gdf_state,
                        &mut self.cdef_state,
                        &mut self.ccso_state,
                        &mut self.delta_q_state,
                        &mut self.intrabc_state,
                        &mut self.segment_id_state,
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
            let mut on_published = |publication: DecodedLeafPublication, leaf: ParsedLeaf| {
                let origin = publication.superblock_origin();
                push_recon_entry(
                    &mut recon_row.superblocks,
                    &mut recon_row.entries,
                    origin,
                    leaf.dependency,
                    ReconRowEntry {
                        publication,
                        command: leaf.command,
                        temporal: 0..0,
                        error: None,
                    },
                );
                recon_row.motion_queue.push(leaf.motion);
                if let Some(pending) = leaf.pending {
                    recon_row.pending_inter.push(pending);
                }
            };
            match granularity {
                ParserGranularity::Row => {
                    let mut decoded = false;
                    let mut error = None;
                    for _ in 0..row_superblocks {
                        let superblock = match walk.decode_next_superblock(
                            self.tile,
                            &mut decode_leaf,
                            &mut on_published,
                        ) {
                            Ok(superblock) => superblock,
                            Err(cause) => {
                                error = Some(cause);
                                break;
                            }
                        };
                        if superblock.is_none() {
                            break;
                        }
                        decoded = true;
                    }
                    error.map_or(Ok(decoded), Err)
                }
                ParserGranularity::Superblock => walk
                    .decode_next_superblock(self.tile, &mut decode_leaf, &mut on_published)
                    .map(|superblock| superblock.is_some()),
            }
        } else {
            recon_row.terminal = Some(inter_cap!(
                "inter_row_parser_missing_walk",
                tile_offset,
                "inter.row.parser_state",
                SPEC_MODE_INFO
            ));
            return ParserStep::Last(recon_row);
        };
        recon_row.filter_records = core::mem::take(&mut self.filter_records);
        self.mv_grid.take_flag_log(&mut recon_row.flag_log);
        self.entry_capacity = self.entry_capacity.max(recon_row.entries.capacity());
        self.superblock_capacity = self
            .superblock_capacity
            .max(recon_row.superblocks.capacity());
        match decoded_row {
            Ok(true) => ParserStep::More(recon_row),
            Err(error) => {
                recon_row.terminal = Some(map_inter_multiblock_error(error, tile_offset));
                ParserStep::Last(recon_row)
            }
            Ok(false) => {
                let Some(walk) = self.walk.take() else {
                    recon_row.terminal = Some(inter_cap!(
                        "inter_row_parser_finish_missing_walk",
                        tile_offset,
                        "inter.row.parser_state",
                        SPEC_MODE_INFO
                    ));
                    return ParserStep::Last(recon_row);
                };
                let crate::bitstream::tile_payload::GeneralIntraMultiblockOutput {
                    symbols,
                    active_source_blocks,
                    unit_filters,
                } = walk.into_output();
                recon_row.terminal = symbols.exit_symbol().err().map(|_| {
                    if context.reference_select {
                        compound_cap!(
                            "compound_exit_symbol",
                            tile_offset,
                            "inter.compound.exit_symbol",
                            SPEC_MODE_INFO
                        )
                    } else {
                        inter_cap!(
                            "inter_exit_symbol",
                            tile_offset,
                            "inter.exit_symbol",
                            SPEC_MODE_INFO
                        )
                    }
                });
                self.tile_walk_output = Some((active_source_blocks, unit_filters));
                ParserStep::Last(recon_row)
            }
        }
    }

    fn into_output(self) -> Result<TileParserOutput> {
        let tile_offset = self.tile.tile_byte_span().start;
        let Some((active_source_blocks, unit_filters)) = self.tile_walk_output else {
            return Err(inter_cap!(
                "inter_row_parser_output",
                tile_offset,
                "inter.row.parser_output",
                SPEC_MODE_INFO
            ));
        };
        Ok(TileParserOutput {
            cdef_state: self.cdef_state,
            gdf_state: self.gdf_state,
            ccso_state: self.ccso_state,
            active_source_blocks,
            unit_filters,
        })
    }
}

#[derive(Clone, Copy)]
enum ParserGranularity {
    Row,
    Superblock,
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
    /// It runs even when parsing stopped early, because the leaves parsed
    /// before the failure are exactly the ones the fused walk would have
    /// resolved; a resolve failure is therefore the earlier one and the caller
    /// lets it win over any later parse or exit-symbol failure.
    fn resolve_unit<T: ReconSample>(
        &mut self,
        grid: &mut NeighbourMvGrid,
        context: &TileDecodeContext<'_, T>,
        temporal_context: &TemporalMvContext,
        row: &mut ReconRow,
        tile_offset: ByteOffset,
    ) -> Result<()> {
        resolve_parsed_leaves(
            &mut row.motion_queue,
            &mut row.pending_inter,
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

/// Pairs one resolved unit with the shadow surface its reconstruction writes
/// into and the reference rows that reconstruction reads.
fn admit_ready_row<'surface, T: ReconSample>(
    step: ParserStep<ReconRow>,
    surfaces: &mut impl Iterator<Item = splot_recon::CurrentFrameRect<'surface, T>>,
    row_gate: &row_gate::RowReferenceGate<'_, T>,
) -> ParserStep<ReadyReconRow<'surface, T>> {
    let (row, last) = match step {
        ParserStep::More(row) => (row, false),
        ParserStep::Last(row) => (row, true),
    };
    let surface = if row.superblocks.is_empty() {
        None
    } else {
        surfaces.next()
    };
    let bounds = row_gate.bounds_for_row(&row);
    let ready = ReadyReconRow {
        row,
        surface,
        bounds,
    };
    if last {
        ParserStep::Last(ready)
    } else {
        ParserStep::More(ready)
    }
}

/// Runs one parse unit's resolve pass over the step the parse pass produced,
/// letting a resolve failure end the unit stream ahead of any later parse or
/// exit-symbol failure the step already carries.
fn resolve_parser_step(
    step: ParserStep<ReconRow>,
    resolve: impl FnOnce(&mut ReconRow) -> Result<()>,
) -> ParserStep<ReconRow> {
    let (mut row, last) = match step {
        ParserStep::More(row) => (row, false),
        ParserStep::Last(row) => (row, true),
    };
    if let Err(error) = resolve(&mut row) {
        row.terminal = Some(error);
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
    pub(super) command: Option<ReconCommand>,
    pub(super) temporal: Range<usize>,
    pub(super) error: Option<crate::DecodeError>,
}

pub(super) struct ReconSuperblock {
    pub(super) origin: [usize; 2],
    dependency: ReconDependency,
    pub(super) entries: Range<usize>,
}

fn push_recon_entry<Entry>(
    superblocks: &mut Vec<ReconSuperblock>,
    entries: &mut Vec<Entry>,
    origin: [usize; 2],
    dependency: ReconDependency,
    entry: Entry,
) {
    let entry_index = entries.len();
    entries.push(entry);
    if let Some(superblock) = superblocks.last_mut().filter(|sb| sb.origin == origin) {
        superblock.dependency = superblock.dependency.max(dependency);
        superblock.entries.end = entries.len();
    } else {
        superblocks.push(ReconSuperblock {
            origin,
            dependency,
            entries: entry_index..entries.len(),
        });
    }
}

pub(super) struct ReconRow {
    pub(super) ordinal: usize,
    pub(super) superblocks: Vec<ReconSuperblock>,
    pub(super) entries: Vec<ReconRowEntry>,
    /// One § 7.12 work item per parsed leaf, drained by the resolve pass
    /// before the row leaves the parser.
    pub(super) motion_queue: Vec<LeafMotion>,
    /// The inter leaves' § 7.12 records, in the queue's own order.
    pub(super) pending_inter: Vec<PendingInterBlock>,
    pub(super) residual_blocks: Vec<InterResidualBlock>,
    pub(super) temporal: Vec<TemporalMotionBlock>,
    /// The unit's flag-plane publications, replayed by a resolve pass that runs
    /// on a grid of its own. Empty unless the parser was logging.
    pub(super) flag_log: Vec<NeighbourFlagRecord>,
    pub(super) filter_records: TileFilterRecords,
    pub(super) terminal: Option<crate::DecodeError>,
}

#[derive(Default)]
pub(super) struct ReconRowBuffers {
    pub(super) superblocks: Vec<ReconSuperblock>,
    pub(super) entries: Vec<ReconRowEntry>,
    pub(super) motion_queue: Vec<LeafMotion>,
    pub(super) pending_inter: Vec<PendingInterBlock>,
    pub(super) residual_blocks: Vec<InterResidualBlock>,
    pub(super) temporal: Vec<TemporalMotionBlock>,
    pub(super) flag_log: Vec<NeighbourFlagRecord>,
    pub(super) filter_records: TileFilterRecords,
}

struct ReconRowBufferPool {
    available: Mutex<Vec<ReconRowBuffers>>,
}

const MAX_RETAINED_RECON_ROW_BUFFERS: usize = 128;
static RETAINED_RECON_ROW_BUFFERS: Mutex<Vec<ReconRowBuffers>> = Mutex::new(Vec::new());

impl ReconRowBufferPool {
    fn new(slots: usize) -> Self {
        let mut available = Vec::with_capacity(slots);
        let mut retained = RETAINED_RECON_ROW_BUFFERS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while available.len() < slots {
            let Some(buffers) = retained.pop() else {
                break;
            };
            available.push(buffers);
        }
        drop(retained);
        available.resize_with(slots, ReconRowBuffers::default);
        Self {
            available: Mutex::new(available),
        }
    }

    fn take(&self) -> ReconRowBuffers {
        if let Some(buffers) = self
            .available
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pop()
        {
            return buffers;
        }
        RETAINED_RECON_ROW_BUFFERS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pop()
            .unwrap_or_default()
    }

    fn recycle(&self, buffers: ReconRowBuffers) {
        self.available
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(buffers);
    }
}

/// Takes one recycled set of per-unit buffers from the process-wide retention.
///
/// The parse-ahead path holds every unit of a frame at once, so it cannot draw
/// from a per-tile pool; it borrows from the same retention the fused walk's
/// pools fill and returns each set as its unit commits.
fn take_retained_recon_row_buffers() -> ReconRowBuffers {
    RETAINED_RECON_ROW_BUFFERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .pop()
        .unwrap_or_default()
}

impl Drop for ReconRowBufferPool {
    fn drop(&mut self) {
        let available = self
            .available
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut retained = RETAINED_RECON_ROW_BUFFERS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while retained.len() < MAX_RETAINED_RECON_ROW_BUFFERS {
            let Some(buffers) = available.pop() else {
                break;
            };
            retained.push(buffers);
        }
    }
}

impl OrderedDone for ReconRow {
    fn ordinal(&self) -> usize {
        self.ordinal
    }
}

struct ReadyReconRow<'a, T: ReconSample> {
    row: ReconRow,
    surface: Option<splot_recon::CurrentFrameRect<'a, T>>,
    bounds: row_gate::RowReferenceBounds,
}

struct InterReconScratchPool<T: ReconSample> {
    available: Mutex<Vec<deferred_recon::InterReconScratch<T>>>,
}

impl<T: ReconSample> InterReconScratchPool<T> {
    fn ensure_workers(&mut self, workers: usize) {
        let available = self
            .available
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if available.len() < workers {
            available.resize_with(workers, deferred_recon::InterReconScratch::default);
        }
    }

    fn with_scratch<R>(&self, f: impl FnOnce(&mut deferred_recon::InterReconScratch<T>) -> R) -> R {
        let mut scratch = self
            .available
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pop()
            .unwrap_or_default();
        let result = f(&mut scratch);
        self.available
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(scratch);
        result
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
pub(super) struct TileDecodeScratch<T: ReconSample> {
    ordered: deferred_recon::InterReconScratch<T>,
    workers: InterReconScratchPool<T>,
}

impl<T: ReconSample> OrderedDone for ReadyReconRow<'_, T> {
    fn ordinal(&self) -> usize {
        self.row.ordinal
    }
}

const fn select_prepass_entry(dependency: ReconDependency, footprint_contained: bool) -> bool {
    matches!(dependency, ReconDependency::ReferenceOnly) && footprint_contained
}

#[allow(clippy::too_many_arguments)]
fn precompute_recon_row<'surface, T: ReconSample>(
    mut ready: ReadyReconRow<'surface, T>,
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
) -> ReadyReconRow<'surface, T> {
    let Some(surface) = ready.surface.as_mut() else {
        return ready;
    };
    let mut surface = super::super::mc::WorkspaceSink::Rect(surface);
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
    let _quantizer_scopes = quantizer.install_frame();
    let info = surface.info();
    let temporal_capacity = row.entries.iter().fold(0usize, |capacity, entry| {
        capacity.saturating_add(
            entry
                .command
                .as_ref()
                .map_or(0, ReconCommand::temporal_record_capacity),
        )
    });
    let _ = row.temporal.try_reserve(temporal_capacity);
    'superblocks: for superblock in &mut row.superblocks {
        let Some(entries) = row.entries.get_mut(superblock.entries.clone()) else {
            break;
        };
        for entry in entries {
            let safe = entry.command.as_ref().is_some_and(|command| {
                select_prepass_entry(
                    command.dependency(),
                    matches!(
                        command,
                        ReconCommand::Inter(command)
                            if command.prepass_write_is_contained(
                                superblock.origin,
                                sb_h4,
                                info,
                                &row.residual_blocks,
                            )
                    ),
                )
            });
            if !safe {
                continue;
            }
            let command = match entry.command.take() {
                Some(ReconCommand::Inter(command)) => command,
                command => {
                    entry.command = command;
                    break 'superblocks;
                }
            };
            let start = row.temporal.len();
            let result = scratch.reconstruct_logged(
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
            );
            match result {
                Ok(()) => {
                    entry.temporal = start..row.temporal.len();
                }
                Err(error) => {
                    row.temporal.truncate(start);
                    entry.error = Some(error);
                    break 'superblocks;
                }
            }
        }
    }
    row
}

/// The prepass state one tile carries across calls: the out-of-order surface
/// its precompute writes into, the block-decoded snapshot that precompute
/// reads, and the ordered commit frontier.
///
/// A fused walk runs one call per tile and could keep these on its stack; a
/// batched driver resumes the same tile once its next unit's references have
/// been published, and must find the frontier exactly where it left it.
struct PrepassCursor<T: ReconSample> {
    shadow: CurrentFrameWorkspace<T>,
    prepass_block_decoded: TileBlockDecodedState,
    recon_ordinal: usize,
    current_block_decoded_superblock: Option<[usize; 2]>,
}

impl<T: ReconSample> PrepassCursor<T> {
    /// Opens the cursor for one tile, before any unit is committed.
    fn new(
        workspace: &CurrentFrameWorkspace<T>,
        block_decoded: &TileBlockDecodedState,
    ) -> Result<Self> {
        Ok(Self {
            shadow: CurrentFrameWorkspace::new(workspace.info(), T::default())?,
            prepass_block_decoded: block_decoded.clone(),
            recon_ordinal: 0,
            current_block_decoded_superblock: None,
        })
    }
}

/// Everything one superblock prepass commits into.
struct PrepassSinks<'a, T: ReconSample> {
    ordered: &'a mut deferred_recon::InterReconScratch<T>,
    workspace: &'a mut CurrentFrameWorkspace<T>,
    block_decoded: &'a mut TileBlockDecodedState,
    motion_field: &'a mut TemporalMotionField,
    frame_filter_records: &'a mut crate::filters::wienerns_lr::FrameFilterRecords,
    decoded_any: &'a mut bool,
}

/// Drives one tile's superblock units through precompute-into-shadow-surface,
/// row-gated admission and ordered commit.
///
/// `next_unit` supplies the units already resolved, so the same driver serves
/// the fused walk, which resolves each unit as it parses it, and the deferred
/// resolve pass, which resolves units the driver parsed earlier.
#[allow(clippy::too_many_arguments)]
fn run_superblock_prepass<T: ReconSample, P>(
    mut next_unit: P,
    rects: &[splot_recon::PlaneRect],
    done_limit: usize,
    tile_offset: ByteOffset,
    context: &TileDecodeContext<'_, T>,
    temporal_context: &TemporalMvContext,
    quantizer: &FrameQuantizerSnapshot,
    row_gate: &row_gate::RowReferenceGate<'_, T>,
    row_buffers: &ReconRowBufferPool,
    workers: &InterReconScratchPool<T>,
    cursor: &mut PrepassCursor<T>,
    sinks: &mut PrepassSinks<'_, T>,
) -> Result<()>
where
    P: FnMut() -> ParserStep<ReconRow> + Send,
{
    let PrepassCursor {
        shadow,
        prepass_block_decoded,
        recon_ordinal,
        current_block_decoded_superblock,
    } = cursor;
    let mut surfaces = shadow.rect_surfaces(rects)?.into_iter();
    let prepass_block_decoded = &*prepass_block_decoded;
    let parse_ready = || admit_ready_row(next_unit(), &mut surfaces, row_gate);
    let timer = crate::timing::start();
    let tally = crate::timing::WorkerTally::new();
    let first_commit_ns = std::sync::atomic::AtomicU64::new(0);
    let first_commit_ns = &first_commit_ns;
    let prepared = run_ready_row_prepass_with_commit(
        parse_ready,
        |ready| {
            tally.note_worker();
            workers.with_scratch(|scratch| {
                precompute_recon_row(
                    ready,
                    scratch,
                    prepass_block_decoded,
                    quantizer,
                    temporal_context,
                    context.reference,
                    context.ref_frame_idx,
                    context.sequence,
                    context.core,
                    context.sb_h4,
                    context.mi_rows,
                    context.mi_cols,
                    context.current_order_hint,
                    context.luma_use_tcq,
                    context.residual_use_ddt,
                    context.bit_depth,
                )
            })
        },
        |ready| {
            let _commit_scope =
                crate::timing::WalkPhaseScope::new(crate::timing::WalkPhase::Commit);
            if first_commit_ns.load(std::sync::atomic::Ordering::Relaxed) == 0
                && let Some(started) = timer
            {
                first_commit_ns.store(
                    u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX),
                    std::sync::atomic::Ordering::Relaxed,
                );
            }
            if let Some(surface) = ready.surface.as_ref() {
                let _scope =
                    crate::timing::WalkPhaseScope::new(crate::timing::WalkPhase::CommitPublish);
                surface.publish_into(sinks.workspace)?;
            }
            let buffers = pixel_commit::replay_recon_row(
                ready.row,
                recon_ordinal,
                sinks.decoded_any,
                quantizer,
                sinks.ordered,
                sinks.workspace,
                sinks.block_decoded,
                current_block_decoded_superblock,
                sinks.motion_field,
                sinks.frame_filter_records,
                temporal_context,
                context.reference,
                context.ref_frame_idx,
                context.sequence,
                context.core,
                context.mi_rows,
                context.mi_cols,
                context.current_order_hint,
                context.luma_use_tcq,
                context.residual_use_ddt,
                context.bit_depth,
                tile_offset,
            )?;
            row_buffers.recycle(buffers);
            Ok(())
        },
        done_limit,
        |ready: &ReadyReconRow<'_, T>| row_gate.admits(&ready.bounds),
        || row_gate.is_ready(),
        || row_gate.wait("arm=rows"),
    )
    .map_err(|error| match error {
        ReadyRowPipelineError::Parallel => inter_cap!(
            "inter_row_prepass_scope",
            tile_offset,
            "inter.row.task_scope",
            SPEC_MODE_INFO
        ),
        ReadyRowPipelineError::Capacity => inter_cap!(
            "inter_row_prepass_capacity",
            tile_offset,
            "inter.row.task_capacity",
            SPEC_MODE_INFO
        ),
        ReadyRowPipelineError::Codec(error) => error,
    })?;
    if timer.is_some() {
        crate::timing::report_detail(
            "inter_row_prepass",
            timer,
            &format!(
                "units={} committed={} c_first_ms={:.3} threads={} workers_used={} max_pending={} max_deferred={} max_active={} settled_arm={} {}",
                prepared.committed,
                prepared.committed,
                first_commit_ns.load(std::sync::atomic::Ordering::Relaxed) as f64 / 1.0e6,
                splot_parallel::current_pool_width(),
                tally.workers_used(),
                prepared.max_pending,
                prepared.max_deferred,
                prepared.max_active,
                u8::from(prepared.settled),
                row_gate.fallback_summary(),
            ),
        );
    }
    let active_limit = splot_parallel::current_pool_width()
        .saturating_sub(1)
        .max(1);
    if prepared.max_pending > prepared.ready_limit || prepared.max_active > active_limit {
        return Err(inter_cap!(
            "inter_row_prepass_bounds",
            tile_offset,
            "inter.row.task_capacity",
            SPEC_MODE_INFO
        ));
    }
    Ok(())
}

/// One tile's units after the entropy pass, owned so the § 7.12 resolve pass
/// and the reconstruction pass can run once the driver has moved on.
///
/// The parse pass reads no reference sample and no projected motion field, so
/// everything here is settled by the bitstream alone; what is still owed is the
/// resolve pass (which needs the frame's temporal prelude) and reconstruction
/// (which needs reference pixels).
pub(super) struct ParsedTile {
    tile_offset: ByteOffset,
    mi_rows: Range<usize>,
    mi_cols: Range<usize>,
    rows: Vec<ReconRow>,
    block_decoded: TileBlockDecodedState,
    /// `None` when the unit stream ended early, exactly the case in which the
    /// fused walk never reached its own `into_output`.
    output: Option<TileParserOutput>,
}

impl ParsedTile {
    /// How many unit buffers the tile is holding, which bounds the split
    /// path's per-frame memory.
    pub(super) fn unit_count(&self) -> usize {
        self.rows.len()
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
    ) -> Result<()> {
        let tile_offset = self.tile_offset;
        let output = self.output.take().ok_or_else(|| {
            inter_cap!(
                "inter_parsed_tile_output",
                tile_offset,
                "inter.row.parser_output",
                SPEC_MODE_INFO
            )
        })?;
        merge_tile_filter_state(
            cdef_state,
            gdf_state,
            ccso_state,
            &output,
            self.mi_rows.clone(),
            self.mi_cols.clone(),
            tile_offset,
        )?;
        append_lr_records(
            &mut frame_filter_records.lr_source_blocks,
            &mut frame_filter_records.lr_unit_filters,
            output.active_source_blocks,
            output.unit_filters,
        )
        .ok_or_else(|| {
            inter_cap!(
                "inter_lr_filter_index_split",
                tile_offset,
                "inter.tile.lr_unit_filter_index",
                SPEC_MODE_INFO
            )
        })
    }
}

/// How many parse units one tile yields, plus the terminating empty unit.
fn tile_unit_capacity(
    mi_rows: &Range<usize>,
    mi_cols: &Range<usize>,
    params: &TileWalkParams,
    granularity: ParserGranularity,
    tile_offset: ByteOffset,
) -> Result<usize> {
    let sb_rows = mi_rows
        .end
        .min(params.mi_rows)
        .saturating_sub(mi_rows.start)
        .div_ceil(params.sb_h4);
    let sb_cols = mi_cols
        .end
        .min(params.mi_cols)
        .saturating_sub(mi_cols.start)
        .div_ceil(params.sb_h4);
    let units = match granularity {
        ParserGranularity::Row => sb_rows,
        ParserGranularity::Superblock => sb_rows.saturating_mul(sb_cols),
    };
    units.checked_add(1).ok_or_else(|| {
        inter_cap!(
            "inter_parsed_row_capacity",
            tile_offset,
            "inter.tile.task_capacity",
            SPEC_MODE_INFO
        )
    })
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
    superblock_units: bool,
) -> Result<ParsedTile> {
    let context = &params.context(sequence, core, reference, ref_frame_idx);
    let granularity = if superblock_units {
        ParserGranularity::Superblock
    } else {
        ParserGranularity::Row
    };
    let tile_offset = tile.tile_byte_span().start;
    let mi_rows = tile.mi_row_range().start as usize..tile.mi_row_range().end as usize;
    let mi_cols = tile.mi_col_range().start as usize..tile.mi_col_range().end as usize;
    let block_decoded = tile_block_decoded(tile, context)?;
    let capacity = tile_unit_capacity(&mi_rows, &mi_cols, params, granularity, tile_offset)?;
    let mut rows = Vec::new();
    rows.try_reserve_exact(capacity).map_err(|_| {
        inter_cap!(
            "inter_parsed_row_allocation",
            tile_offset,
            "inter.tile.task_capacity",
            SPEC_MODE_INFO
        )
    })?;
    let mut parser = TileParser::new(
        tile,
        context,
        cdef_state.try_for_tile(mi_rows.clone(), mi_cols.clone(), tile_offset)?,
        gdf_state.for_tile(mi_rows.clone(), mi_cols.clone(), tile_offset)?,
        ccso_state.try_for_tile(mi_rows.clone(), mi_cols.clone(), tile_offset)?,
    )?;
    parser.mv_grid.log_flags();
    let started = crate::timing::start();
    loop {
        let step = parser.next_unit(
            context,
            granularity,
            Some(take_retained_recon_row_buffers()),
        );
        let (row, last) = match step {
            ParserStep::More(row) => (row, false),
            ParserStep::Last(row) => (row, true),
        };
        rows.push(row);
        if last {
            break;
        }
    }
    crate::timing::report("pass1_parse", started);
    Ok(ParsedTile {
        tile_offset,
        mi_rows,
        mi_cols,
        rows,
        block_decoded,
        output: parser.into_output().ok(),
    })
}

/// Runs one parsed tile's AV2 § 7.12 resolve pass on a grid of its own.
///
/// The pass replays each unit's flag plane before resolving it, so the § 7.12
/// probes see the same published-but-unresolved frontier the fused walk saw,
/// and shares nothing with the parser's grid. A unit that fails ends the unit
/// stream exactly where the fused walk's would have: the units behind it were
/// never resolved there, so they are dropped here.
fn resolve_parsed_tile<T: ReconSample>(
    parsed: &mut ParsedTile,
    context: &TileDecodeContext<'_, T>,
    temporal_context: &TemporalMvContext,
) -> Result<()> {
    let started = crate::timing::start();
    let tile_offset = parsed.tile_offset;
    let mut grid = NeighbourMvGrid::new_for_tile(parsed.mi_rows.clone(), parsed.mi_cols.clone())
        .ok_or_else(|| {
            inter_cap!(
                "inter_parsed_mv_grid",
                tile_offset,
                "inter.mv_grid",
                SPEC_MODE_INFO
            )
        })?;
    let mut resolve_state = TileResolveState::new(context.sequence);
    let mut resolved = 0usize;
    for row in &mut parsed.rows {
        grid.replay_flag_log(&row.flag_log);
        resolved += 1;
        let outcome =
            resolve_state.resolve_unit(&mut grid, context, temporal_context, row, tile_offset);
        if let Err(error) = outcome {
            row.terminal = Some(error);
            break;
        }
        if row.terminal.is_some() {
            break;
        }
    }
    parsed.rows.truncate(resolved);
    crate::timing::report("pass2_resolve", started);
    Ok(())
}

/// Reconstructs one parsed tile: its § 7.12 resolve pass, then the same
/// row-gated prepass and ordered commit the fused walk runs.
///
/// This is the tail of a walk whose entropy pass ran elsewhere, so it takes the
/// frame-level state by reference exactly as [`decode_tiles`] does.
#[allow(clippy::too_many_arguments)]
pub(super) fn reconstruct_parsed_tile<T: ReconSample>(
    scratch: &mut TileDecodeScratch<T>,
    frame_filter_records: &mut crate::filters::wienerns_lr::FrameFilterRecords,
    mut parsed: ParsedTile,
    params: &TileWalkParams,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    temporal_context: &TemporalMvContext,
    reference: &InterReferenceState<T>,
    ref_frame_idx: &[u32],
    workspace: &mut CurrentFrameWorkspace<T>,
    mut motion_field: TemporalMotionField,
) -> Result<TemporalMotionField> {
    scratch.workers.ensure_workers(
        splot_parallel::current_pool_width()
            .saturating_sub(1)
            .max(1),
    );
    let TileDecodeScratch { ordered, workers } = scratch;
    let context = params.context(sequence, core, reference, ref_frame_idx);
    let tile_offset = parsed.tile_offset;
    let row_gate = row_gate::RowReferenceGate::new(
        reference,
        core,
        ref_frame_idx,
        workspace.info(),
        temporal_context,
    );
    let quantizer = FrameQuantizerSnapshot::capture();
    resolve_parsed_tile(&mut parsed, &context, temporal_context)?;
    let rects = superblock_luma_rects(
        &parsed.mi_rows,
        &parsed.mi_cols,
        workspace,
        params.sb_h4,
        tile_offset,
    )?;
    let done_limit = parsed.rows.len().checked_add(1).ok_or_else(|| {
        inter_cap!(
            "inter_parsed_done_limit_overflow",
            tile_offset,
            "inter.superblock.task_capacity",
            SPEC_MODE_INFO
        )
    })?;
    let mut cursor = PrepassCursor::new(workspace, &parsed.block_decoded)?;
    let mut block_decoded = parsed.block_decoded.clone();
    let row_buffers = ReconRowBufferPool::new(0);
    let mut decoded_any = false;
    let mut units = core::mem::take(&mut parsed.rows).into_iter();
    let mut fallback_ordinal = 0usize;
    let next_unit = || {
        let Some(row) = units.next() else {
            fallback_ordinal = fallback_ordinal.saturating_add(1);
            return ParserStep::Last(terminal_recon_row(
                fallback_ordinal,
                inter_cap!(
                    "inter_parsed_unit_stream",
                    tile_offset,
                    "inter.row.parser_state",
                    SPEC_MODE_INFO
                ),
            ));
        };
        fallback_ordinal = row.ordinal.saturating_add(1);
        if units.len() == 0 {
            ParserStep::Last(row)
        } else {
            ParserStep::More(row)
        }
    };
    run_superblock_prepass(
        next_unit,
        &rects,
        done_limit,
        tile_offset,
        &context,
        temporal_context,
        &quantizer,
        &row_gate,
        &row_buffers,
        workers,
        &mut cursor,
        &mut PrepassSinks {
            ordered,
            workspace,
            block_decoded: &mut block_decoded,
            motion_field: &mut motion_field,
            frame_filter_records,
            decoded_any: &mut decoded_any,
        },
    )?;
    if !decoded_any {
        return Err(inter_missing!(
            "inter_parsed_no_decoded_block",
            tile_offset,
            "inter.block",
            SPEC_MODE_INFO
        ));
    }
    Ok(motion_field)
}

/// One unit carrying nothing but the diagnostic that ends its tile's stream.
fn terminal_recon_row(ordinal: usize, terminal: crate::DecodeError) -> ReconRow {
    ReconRow {
        ordinal,
        superblocks: Vec::new(),
        entries: Vec::new(),
        motion_queue: Vec::new(),
        pending_inter: Vec::new(),
        residual_blocks: Vec::new(),
        temporal: Vec::new(),
        flag_log: Vec::new(),
        filter_records: TileFilterRecords::default(),
        terminal: Some(terminal),
    }
}

struct PreparedTile {
    tile_num: u32,
    tile_offset: ByteOffset,
    mi_rows: Range<usize>,
    mi_cols: Range<usize>,
    rows: Vec<ReconRow>,
    quantizer: FrameQuantizerSnapshot,
    block_decoded: TileBlockDecodedState,
    output: TileParserOutput,
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
    .map_err(|_| {
        inter_cap!(
            "inter_tile_block_decoded_init",
            tile.tile_byte_span().start,
            "inter.partition_walk",
            SPEC_MODE_INFO
        )
    })
}

fn tile_luma_rect<T: ReconSample>(
    tile: &DecodeTileWorkUnit<'_>,
    workspace: &CurrentFrameWorkspace<T>,
) -> Result<splot_recon::PlaneRect> {
    let mi_rows = tile.mi_row_range().start as usize..tile.mi_row_range().end as usize;
    let mi_cols = tile.mi_col_range().start as usize..tile.mi_col_range().end as usize;
    luma_rect(&mi_rows, &mi_cols, workspace)
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

fn tile_superblock_luma_rects<T: ReconSample>(
    tile: &DecodeTileWorkUnit<'_>,
    workspace: &CurrentFrameWorkspace<T>,
    sb_h4: usize,
) -> Result<Vec<splot_recon::PlaneRect>> {
    let mi_rows = tile.mi_row_range().start as usize..tile.mi_row_range().end as usize;
    let mi_cols = tile.mi_col_range().start as usize..tile.mi_col_range().end as usize;
    superblock_luma_rects(
        &mi_rows,
        &mi_cols,
        workspace,
        sb_h4,
        tile.tile_byte_span().start,
    )
}

fn superblock_luma_rects<T: ReconSample>(
    mi_rows: &Range<usize>,
    mi_cols: &Range<usize>,
    workspace: &CurrentFrameWorkspace<T>,
    sb_h4: usize,
    tile_offset: ByteOffset,
) -> Result<Vec<splot_recon::PlaneRect>> {
    let bounds = luma_rect(mi_rows, mi_cols, workspace)?;
    let side = sb_h4.checked_mul(4).ok_or_else(|| {
        inter_cap!(
            "inter_superblock_surface_size",
            tile_offset,
            "inter.superblock.task_capacity",
            SPEC_MODE_INFO
        )
    })?;
    let rows = bounds.height().div_ceil(side);
    let cols = bounds.width().div_ceil(side);
    let count = rows.checked_mul(cols).ok_or_else(|| {
        inter_cap!(
            "inter_superblock_surface_count",
            tile_offset,
            "inter.superblock.task_capacity",
            SPEC_MODE_INFO
        )
    })?;
    let mut rects = Vec::new();
    rects.try_reserve_exact(count).map_err(|_| {
        inter_cap!(
            "inter_superblock_surface_allocation",
            tile_offset,
            "inter.superblock.task_capacity",
            SPEC_MODE_INFO
        )
    })?;
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

#[allow(clippy::too_many_arguments)]
fn prepare_tile<T: ReconSample>(
    tile: &mut DecodeTileWorkUnit<'_>,
    mut surface: splot_recon::CurrentFrameRect<'_, T>,
    context: &TileDecodeContext<'_, T>,
    temporal_context: &TemporalMvContext,
    cdef_state: &CdefState,
    gdf_state: &GdfState,
    ccso_state: &CcsoState,
    quantizer: FrameQuantizerSnapshot,
    scratch_pool: &InterReconScratchPool<T>,
) -> Result<PreparedTile> {
    let tile_num = tile.tile_num();
    let tile_offset = tile.tile_byte_span().start;
    let mi_rows = tile.mi_row_range().start as usize..tile.mi_row_range().end as usize;
    let mi_cols = tile.mi_col_range().start as usize..tile.mi_col_range().end as usize;
    let block_decoded = tile_block_decoded(tile, context)?;
    let row_count = mi_rows
        .end
        .min(context.mi_rows)
        .saturating_sub(mi_rows.start)
        .div_ceil(context.sb_h4)
        .checked_add(1)
        .ok_or_else(|| {
            inter_cap!(
                "inter_tile_row_capacity",
                tile_offset,
                "inter.tile.task_capacity",
                SPEC_MODE_INFO
            )
        })?;
    let mut rows = Vec::new();
    rows.try_reserve_exact(row_count).map_err(|_| {
        inter_cap!(
            "inter_tile_row_allocation",
            tile_offset,
            "inter.tile.task_capacity",
            SPEC_MODE_INFO
        )
    })?;
    let mut parser = TileParser::new(
        tile,
        context,
        cdef_state.try_for_tile(mi_rows.clone(), mi_cols.clone(), tile_offset)?,
        gdf_state.for_tile(mi_rows.clone(), mi_cols.clone(), tile_offset)?,
        ccso_state.try_for_tile(mi_rows.clone(), mi_cols.clone(), tile_offset)?,
    )?;
    let mut resolve_state = TileResolveState::new(context.sequence);
    let mut sink = super::super::mc::WorkspaceSink::Rect(&mut surface);
    loop {
        let _quantizer_scopes = quantizer.install_frame();
        let step = parser.next_unit(context, ParserGranularity::Row, None);
        let step = resolve_parser_step(step, |row| {
            resolve_state.resolve_unit(
                &mut parser.mv_grid,
                context,
                temporal_context,
                row,
                tile_offset,
            )
        });
        let (row, last) = match step {
            ParserStep::More(row) => (row, false),
            ParserStep::Last(row) => (row, true),
        };
        rows.push(scratch_pool.with_scratch(|scratch| {
            precompute_recon_row_on_surface(
                row,
                &mut sink,
                scratch,
                &block_decoded,
                &quantizer,
                temporal_context,
                context.reference,
                context.ref_frame_idx,
                context.sequence,
                context.core,
                context.sb_h4,
                context.mi_rows,
                context.mi_cols,
                context.current_order_hint,
                context.luma_use_tcq,
                context.residual_use_ddt,
                context.bit_depth,
            )
        }));
        if last {
            break;
        }
    }
    Ok(PreparedTile {
        tile_num,
        tile_offset,
        mi_rows,
        mi_cols,
        rows,
        quantizer,
        block_decoded,
        output: parser.into_output()?,
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn decode_tiles<T: ReconSample>(
    scratch: &mut TileDecodeScratch<T>,
    frame_filter_records: &mut crate::filters::wienerns_lr::FrameFilterRecords,
    work_units: &mut [DecodeTileWorkUnit<'_>],
    params: &TileWalkParams,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    temporal_context: &TemporalMvContext,
    reference: &InterReferenceState<T>,
    ref_frame_idx: &[u32],
    workspace: &mut CurrentFrameWorkspace<T>,
    mut cdef_state: CdefState,
    mut gdf_state: GdfState,
    mut ccso_state: CcsoState,
    mut motion_field: TemporalMotionField,
) -> Result<TileDecodeOutput> {
    scratch.workers.ensure_workers(
        splot_parallel::current_pool_width()
            .saturating_sub(1)
            .max(1),
    );
    let TileDecodeScratch { ordered, workers } = scratch;
    let context = params.context(sequence, core, reference, ref_frame_idx);
    let &TileWalkParams {
        mi_rows,
        mi_cols,
        sb_h4,
        luma_use_tcq,
        residual_use_ddt,
        bit_depth,
        current_order_hint,
        ..
    } = params;
    frame_filter_records.clear();
    let mut decoded_any = false;
    let chunk_offset = work_units
        .first()
        .map_or(ByteOffset::new(0), |tile| tile.tile_byte_span().start);
    let row_gate = row_gate::RowReferenceGate::new(
        reference,
        core,
        ref_frame_idx,
        workspace.info(),
        temporal_context,
    );
    let parallel_tiles = work_units.len() > 1
        && splot_parallel::current_pool_width() > 1
        && !super::intrabc::global_intrabc_enabled(core.intrabc);
    let parallel_prepass = splot_parallel::current_pool_width() >= 4
        && !super::intrabc::global_intrabc_enabled(core.intrabc);
    if parallel_tiles {
        row_gate.wait("arm=tiles")?;
        let mut luma_rects = Vec::new();
        luma_rects
            .try_reserve_exact(work_units.len())
            .map_err(|_| {
                inter_cap!(
                    "inter_tile_surface_allocation",
                    chunk_offset,
                    "inter.tile.task_capacity",
                    SPEC_MODE_INFO
                )
            })?;
        for tile in work_units.iter() {
            luma_rects.push(tile_luma_rect(tile, workspace)?);
        }
        let surfaces = workspace.rect_surfaces(&luma_rects)?;
        let quantizer = FrameQuantizerSnapshot::capture();
        let timer = crate::timing::start();
        let tally = crate::timing::WorkerTally::new();
        let mut prepared_results = Vec::new();
        prepared_results
            .try_reserve_exact(work_units.len())
            .map_err(|_| {
                inter_cap!(
                    "inter_tile_result_slots_allocation",
                    chunk_offset,
                    "inter.tile.task_capacity",
                    SPEC_MODE_INFO
                )
            })?;
        prepared_results.resize_with(work_units.len(), || None);
        splot_parallel::ready_task_scope(|scope| {
            for ((tile, surface), result) in work_units
                .iter_mut()
                .zip(surfaces)
                .zip(&mut prepared_results)
            {
                let tally = &tally;
                let context = &context;
                let cdef_state = &cdef_state;
                let gdf_state = &gdf_state;
                let ccso_state = &ccso_state;
                let workers = &*workers;
                let quantizer = quantizer.clone();
                scope.spawn(move |_| {
                    tally.note_worker();
                    *result = Some(prepare_tile(
                        tile,
                        surface,
                        context,
                        temporal_context,
                        cdef_state,
                        gdf_state,
                        ccso_state,
                        quantizer,
                        workers,
                    ));
                });
            }
        })
        .map_err(|_| {
            inter_cap!(
                "inter_tile_prepare_scope",
                chunk_offset,
                "inter.tile.task_scope",
                SPEC_MODE_INFO
            )
        })?;
        if timer.is_some() {
            crate::timing::report_detail(
                "inter_tile_prepare",
                timer,
                &format!(
                    "units={} threads={} workers_used={}",
                    prepared_results.len(),
                    splot_parallel::current_pool_width(),
                    tally.workers_used()
                ),
            );
        }
        let mut prepared = Vec::new();
        prepared
            .try_reserve_exact(prepared_results.len())
            .map_err(|_| {
                inter_cap!(
                    "inter_tile_result_collection_allocation",
                    chunk_offset,
                    "inter.tile.task_capacity",
                    SPEC_MODE_INFO
                )
            })?;
        for result in prepared_results {
            let result = result.ok_or_else(|| {
                inter_cap!(
                    "inter_tile_result_missing",
                    chunk_offset,
                    "inter.tile.task_scope",
                    SPEC_MODE_INFO
                )
            })?;
            prepared.push(result?);
        }
        prepared.sort_by_key(|tile| tile.tile_num);
        for mut tile in prepared {
            let output = tile.output;
            merge_tile_filter_state(
                &mut cdef_state,
                &mut gdf_state,
                &mut ccso_state,
                &output,
                tile.mi_rows.clone(),
                tile.mi_cols.clone(),
                tile.tile_offset,
            )?;
            append_lr_records(
                &mut frame_filter_records.lr_source_blocks,
                &mut frame_filter_records.lr_unit_filters,
                output.active_source_blocks,
                output.unit_filters,
            )
            .ok_or_else(|| {
                inter_cap!(
                    "inter_lr_filter_index_parallel",
                    tile.tile_offset,
                    "inter.tile.lr_unit_filter_index",
                    SPEC_MODE_INFO
                )
            })?;

            let mut recon_ordinal = 0usize;
            let mut current_block_decoded_superblock = None;
            for row in tile.rows {
                drop(pixel_commit::replay_recon_row(
                    row,
                    &mut recon_ordinal,
                    &mut decoded_any,
                    &tile.quantizer,
                    ordered,
                    workspace,
                    &mut tile.block_decoded,
                    &mut current_block_decoded_superblock,
                    &mut motion_field,
                    frame_filter_records,
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
                    tile.tile_offset,
                )?);
            }
        }
        if !decoded_any {
            return Err(inter_missing!(
                "inter_no_decoded_block_parallel",
                chunk_offset,
                "inter.block",
                SPEC_MODE_INFO
            ));
        }
        return Ok(TileDecodeOutput {
            cdef_state,
            gdf_state,
            ccso_state,
            motion_field,
        });
    }
    if !parallel_prepass {
        row_gate.wait("arm=serial")?;
    }
    for tile in work_units.iter_mut() {
        let tile_offset = tile.tile_byte_span().start;
        let mut block_decoded = tile_block_decoded(tile, &context)?;
        let mut current_block_decoded_superblock = None;
        let quantizer = FrameQuantizerSnapshot::capture();
        let mut recon_ordinal = 0usize;
        let superblock_rects = if parallel_prepass {
            Some(tile_superblock_luma_rects(tile, workspace, sb_h4)?)
        } else {
            None
        };
        let tile_mi_rows = tile.mi_row_range().start as usize..tile.mi_row_range().end as usize;
        let tile_mi_cols = tile.mi_col_range().start as usize..tile.mi_col_range().end as usize;
        let mut parser = TileParser::new(
            tile,
            &context,
            cdef_state.try_for_tile(tile_mi_rows.clone(), tile_mi_cols.clone(), tile_offset)?,
            gdf_state.for_tile(tile_mi_rows.clone(), tile_mi_cols.clone(), tile_offset)?,
            ccso_state.try_for_tile(tile_mi_rows.clone(), tile_mi_cols.clone(), tile_offset)?,
        )?;
        let mut resolve_state = TileResolveState::new(sequence);
        if parallel_prepass {
            let Some(rects) = superblock_rects else {
                return Err(inter_cap!(
                    "inter_superblock_surface_state",
                    tile_offset,
                    "inter.superblock.task_capacity",
                    SPEC_MODE_INFO
                ));
            };
            let row_buffers = ReconRowBufferPool::new(
                splot_parallel::current_pool_width()
                    .saturating_mul(3)
                    .max(1),
            );
            let next_unit = || {
                let _quantizer_scopes = quantizer.install_frame();
                let step = parser.next_unit(
                    &context,
                    ParserGranularity::Superblock,
                    Some(row_buffers.take()),
                );
                resolve_parser_step(step, |row| {
                    resolve_state.resolve_unit(
                        &mut parser.mv_grid,
                        &context,
                        temporal_context,
                        row,
                        tile_offset,
                    )
                })
            };
            let mut cursor = PrepassCursor::new(workspace, &block_decoded)?;
            let done_limit = rects.len().checked_add(1).ok_or_else(|| {
                inter_cap!(
                    "inter_superblock_done_limit_overflow",
                    tile_offset,
                    "inter.superblock.task_capacity",
                    SPEC_MODE_INFO
                )
            })?;
            run_superblock_prepass(
                next_unit,
                &rects,
                done_limit,
                tile_offset,
                &context,
                temporal_context,
                &quantizer,
                &row_gate,
                &row_buffers,
                workers,
                &mut cursor,
                &mut PrepassSinks {
                    ordered,
                    workspace,
                    block_decoded: &mut block_decoded,
                    motion_field: &mut motion_field,
                    frame_filter_records,
                    decoded_any: &mut decoded_any,
                },
            )?;
        } else {
            let row_buffers = ReconRowBufferPool::new(1);
            let parse_row = || {
                let _quantizer_scopes = quantizer.install_frame();
                let step =
                    parser.next_unit(&context, ParserGranularity::Row, Some(row_buffers.take()));
                resolve_parser_step(step, |row| {
                    resolve_state.resolve_unit(
                        &mut parser.mv_grid,
                        &context,
                        temporal_context,
                        row,
                        tile_offset,
                    )
                })
            };
            let replay_row = |row: ReconRow| -> Result<()> {
                let buffers = pixel_commit::replay_recon_row(
                    row,
                    &mut recon_ordinal,
                    &mut decoded_any,
                    &quantizer,
                    ordered,
                    workspace,
                    &mut block_decoded,
                    &mut current_block_decoded_superblock,
                    &mut motion_field,
                    frame_filter_records,
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
                    tile_offset,
                )?;
                row_buffers.recycle(buffers);
                Ok(())
            };
            run_ready_row_pipeline_serial(parse_row, replay_row)?;
        }
        let output = parser.into_output()?;
        merge_tile_filter_state(
            &mut cdef_state,
            &mut gdf_state,
            &mut ccso_state,
            &output,
            tile_mi_rows,
            tile_mi_cols,
            tile_offset,
        )?;
        append_lr_records(
            &mut frame_filter_records.lr_source_blocks,
            &mut frame_filter_records.lr_unit_filters,
            output.active_source_blocks,
            output.unit_filters,
        )
        .ok_or_else(|| {
            inter_cap!(
                "inter_lr_filter_index_serial",
                tile_offset,
                "inter.tile.lr_unit_filter_index",
                SPEC_MODE_INFO
            )
        })?;
    }
    if !decoded_any {
        return Err(inter_missing!(
            "inter_no_decoded_block",
            chunk_offset,
            "inter.block",
            SPEC_MODE_INFO
        ));
    }

    Ok(TileDecodeOutput {
        cdef_state,
        gdf_state,
        ccso_state,
        motion_field,
    })
}

#[cfg(test)]
#[path = "tile_ready_row_tests.rs"]
mod ready_row_tests;

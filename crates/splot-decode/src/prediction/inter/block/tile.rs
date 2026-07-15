// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Tile-local block decode and reconstruction.
//!
//! Feature tracking: `INFRA-DECODE-PARALLEL-STAGES`.

use std::collections::VecDeque;
use std::ops::Range;
use std::sync::{Mutex, MutexGuard};

use super::*;

const READY_ROW_CAPACITY: usize = 2;

enum ParserStep<Row> {
    More(Row),
    Last(Row),
}

#[derive(Debug)]
enum ReadyRowPipelineError<E> {
    Codec(E),
    Capacity,
    Parallel,
}

struct ReadyRowCoordinator<Parser, Ready, Done> {
    parser: Option<Parser>,
    ready: VecDeque<Ready>,
    done: Vec<Done>,
    done_limit: usize,
    capacity_error: bool,
    parser_active: bool,
    parser_done: bool,
    active_tasks: usize,
    active_limit: usize,
    max_pending: usize,
    max_active: usize,
}

fn lock_ready_rows<Parser, Ready, Done>(
    coordinator: &Mutex<ReadyRowCoordinator<Parser, Ready, Done>>,
) -> MutexGuard<'_, ReadyRowCoordinator<Parser, Ready, Done>> {
    coordinator
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn schedule_ready_rows<'scope, Parser, Work, Ready, Done>(
    scope: &splot_parallel::TaskScope<'_, 'scope>,
    coordinator: &'scope Mutex<ReadyRowCoordinator<Parser, Ready, Done>>,
    work: &'scope Work,
) where
    Parser: FnMut() -> ParserStep<Ready> + Send + 'scope,
    Work: Fn(Ready) -> Done + Sync + 'scope,
    Ready: Send + 'scope,
    Done: Send + 'scope,
{
    let (spawn_parser, ready) = {
        let mut state = lock_ready_rows(coordinator);
        if state.capacity_error {
            (false, None)
        } else {
            let spawn_parser = !state.parser_done
                && !state.parser_active
                && state.parser.is_some()
                && state.ready.len() < READY_ROW_CAPACITY;
            if spawn_parser {
                state.parser_active = true;
            }
            let ready = if state.active_tasks < state.active_limit {
                state.ready.pop_front()
            } else {
                None
            };
            if ready.is_some() {
                state.active_tasks += 1;
                state.max_active = state.max_active.max(state.active_tasks);
            }
            (spawn_parser, ready)
        }
    };
    if spawn_parser {
        scope.spawn(move |scope| {
            let mut parser = lock_ready_rows(coordinator).parser.take();
            let Some(parser_state) = parser.as_mut() else {
                let mut state = lock_ready_rows(coordinator);
                state.parser_active = false;
                state.capacity_error = true;
                return;
            };
            let (row, last) = match parser_state() {
                ParserStep::More(row) => (row, false),
                ParserStep::Last(row) => (row, true),
            };
            let mut overflow = None;
            {
                let mut state = lock_ready_rows(coordinator);
                state.parser = parser;
                state.parser_active = false;
                state.parser_done |= last;
                if state.capacity_error || state.ready.len() >= READY_ROW_CAPACITY {
                    state.capacity_error = true;
                    overflow = Some(row);
                } else {
                    state.ready.push_back(row);
                    state.max_pending = state.max_pending.max(state.ready.len());
                }
            }
            drop(overflow);
            schedule_ready_rows(scope, coordinator, work);
        });
    }
    if let Some(ready) = ready {
        scope.spawn(move |scope| {
            let done = work(ready);
            let mut overflow = None;
            {
                let mut state = lock_ready_rows(coordinator);
                state.active_tasks = state.active_tasks.saturating_sub(1);
                if state.capacity_error || state.done.len() >= state.done_limit {
                    state.capacity_error = true;
                    overflow = Some(done);
                } else {
                    state.done.push(done);
                }
            }
            drop(overflow);
            schedule_ready_rows(scope, coordinator, work);
        });
    }
}

struct PreparedRows<Row> {
    rows: Vec<Row>,
    max_pending: usize,
    max_active: usize,
}

fn run_ready_row_prepass_parallel<Parser, Work, Ready, Done>(
    parser: Parser,
    work: Work,
    done_limit: usize,
) -> core::result::Result<PreparedRows<Done>, ReadyRowPipelineError<()>>
where
    Parser: FnMut() -> ParserStep<Ready> + Send,
    Work: Fn(Ready) -> Done + Send + Sync,
    Ready: Send,
    Done: Send,
{
    let active_limit = splot_parallel::current_pool_width()
        .saturating_sub(1)
        .max(1);
    let mut done = Vec::new();
    done.try_reserve_exact(done_limit)
        .map_err(|_| ReadyRowPipelineError::Capacity)?;
    let coordinator = Mutex::new(ReadyRowCoordinator {
        parser: Some(parser),
        ready: VecDeque::with_capacity(READY_ROW_CAPACITY),
        done,
        done_limit,
        capacity_error: false,
        parser_active: false,
        parser_done: false,
        active_tasks: 0,
        active_limit,
        max_pending: 0,
        max_active: 0,
    });
    splot_parallel::ready_task_scope(|scope| schedule_ready_rows(scope, &coordinator, &work))
        .map_err(|_| ReadyRowPipelineError::Parallel)?;
    let state = coordinator
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if state.capacity_error
        || !state.parser_done
        || state.parser_active
        || state.active_tasks != 0
        || !state.ready.is_empty()
    {
        return Err(ReadyRowPipelineError::Capacity);
    }
    Ok(PreparedRows {
        rows: state.done,
        max_pending: state.max_pending,
        max_active: state.max_active,
    })
}

fn run_ready_row_pipeline_serial<Parser, Recon, Row, E>(
    mut parser: Parser,
    mut recon: Recon,
) -> core::result::Result<(), ReadyRowPipelineError<E>>
where
    Parser: FnMut() -> ParserStep<Row>,
    Recon: FnMut(Row) -> core::result::Result<(), E>,
{
    loop {
        let (row, last) = match parser() {
            ParserStep::More(row) => (row, false),
            ParserStep::Last(row) => (row, true),
        };
        recon(row).map_err(ReadyRowPipelineError::Codec)?;
        if last {
            return Ok(());
        }
    }
}

pub(super) struct TileDecodeOutput {
    pub(super) cdef_state: CdefState,
    pub(super) gdf_state: GdfState,
    pub(super) ccso_state: CcsoState,
    pub(super) motion_field: TemporalMotionField,
    pub(super) deblock_blocks: Vec<crate::filters::deblock::DeblockBlock>,
    pub(super) chroma_deblock_blocks: [Vec<crate::filters::deblock::DeblockBlock>; 2],
    pub(super) tx_skip_records: Vec<crate::filters::wienerns_lr::WienerNsLrTxSkipTransformRecord>,
    pub(super) active_source_blocks: Vec<crate::bitstream::tile_payload::WienerNsLrSourceBlock>,
    pub(super) unit_filters: Vec<crate::bitstream::tile_payload::WienerNsLrUnitFilter>,
}

struct ReconRowEntry<T: ReconSample> {
    publication: DecodedLeafPublication,
    command: Option<ReconCommand<T>>,
    temporal: Range<usize>,
    error: Option<crate::DecodeError>,
}

struct ReconSuperblock<Entry> {
    origin: [usize; 2],
    dependency: ReconDependency,
    entries: Vec<Entry>,
}

fn push_recon_entry<Entry>(
    superblocks: &mut Vec<ReconSuperblock<Entry>>,
    origin: [usize; 2],
    dependency: ReconDependency,
    entry: Entry,
) {
    if let Some(superblock) = superblocks.last_mut().filter(|sb| sb.origin == origin) {
        superblock.dependency = superblock.dependency.max(dependency);
        superblock.entries.push(entry);
    } else {
        superblocks.push(ReconSuperblock {
            origin,
            dependency,
            entries: vec![entry],
        });
    }
}

struct ReconRow<T: ReconSample> {
    ordinal: usize,
    superblocks: Vec<ReconSuperblock<ReconRowEntry<T>>>,
    temporal: Vec<TemporalMotionBlock>,
    terminal: Option<crate::DecodeError>,
}

impl<T: ReconSample> ReconRow<T> {
    fn push(&mut self, publication: DecodedLeafPublication, command: ReconCommand<T>) {
        let origin = publication.superblock_origin();
        let dependency = command.dependency();
        push_recon_entry(
            &mut self.superblocks,
            origin,
            dependency,
            ReconRowEntry {
                publication,
                command: Some(command),
                temporal: 0..0,
                error: None,
            },
        );
    }
}

struct ReadyReconRow<'a, T: ReconSample> {
    row: ReconRow<T>,
    band: Option<splot_recon::CurrentFrameRowBand<'a, T>>,
}

const fn select_prepass_superblock(
    dependency: ReconDependency,
    has_entries: bool,
    footprints_contained: bool,
) -> bool {
    matches!(dependency, ReconDependency::ReferenceOnly) && has_entries && footprints_contained
}

#[allow(clippy::too_many_arguments)]
fn precompute_recon_row<T: ReconSample>(
    ready: ReadyReconRow<'_, T>,
    block_decoded: &TileBlockDecodedState,
    quantizer: &FrameQuantizerSnapshot,
    temporal_context: &TemporalMvContext,
    reference: &InterReferenceState<'_, T>,
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
) -> ReconRow<T> {
    let ReadyReconRow { mut row, band } = ready;
    let Some(mut band) = band else {
        return row;
    };
    let _quantizer_scopes = quantizer.install_frame();
    let info = band.info();
    let mut scratch = deferred_recon::InterReconScratch::default();
    let mut stopped = false;
    for superblock in &mut row.superblocks {
        let footprints_contained = superblock.entries.iter().all(|entry| {
            matches!(
                entry.command.as_ref(),
                Some(ReconCommand::Inter(command))
                    if command.prepass_write_is_contained(superblock.origin, sb_h4, info)
            )
        });
        let safe = select_prepass_superblock(
            superblock.dependency,
            !superblock.entries.is_empty(),
            footprints_contained,
        );
        if !safe || stopped {
            continue;
        }
        for entry in &mut superblock.entries {
            let command = match entry.command.take() {
                Some(ReconCommand::Inter(command)) => command,
                command => {
                    entry.command = command;
                    stopped = true;
                    break;
                }
            };
            let start = row.temporal.len();
            let result = scratch.reconstruct_logged(
                command,
                &mut super::super::mc::WorkspaceSink::Row(&mut band),
                block_decoded,
                &mut row.temporal,
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
                    stopped = true;
                    break;
                }
            }
        }
    }
    row
}

#[allow(clippy::too_many_arguments)]
fn replay_recon_row<T: ReconSample>(
    mut row: ReconRow<T>,
    expected_ordinal: &mut usize,
    decoded_any: &mut bool,
    quantizer: &FrameQuantizerSnapshot,
    scratch: &mut deferred_recon::InterReconScratch<T>,
    workspace: &mut CurrentFrameWorkspace<T>,
    block_decoded: &mut TileBlockDecodedState,
    current_superblock: &mut Option<[usize; 2]>,
    motion_field: &mut TemporalMotionField,
    temporal_context: &TemporalMvContext,
    reference: &InterReferenceState<'_, T>,
    ref_frame_idx: &[u32],
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    mi_rows: usize,
    mi_cols: usize,
    current_order_hint: u32,
    luma_use_tcq: bool,
    residual_use_ddt: bool,
    bit_depth: BitDepth,
    tile_offset: ByteOffset,
) -> Result<()> {
    if row.ordinal != *expected_ordinal {
        return Err(inter_cap!(
            "inter_row_recon_order",
            tile_offset,
            "inter.row.recon_order",
            SPEC_MODE_INFO
        ));
    }
    *expected_ordinal = expected_ordinal.saturating_add(1);
    let terminal = row.terminal.take();
    let row_has_entries = !row.superblocks.is_empty();
    let _quantizer_scopes = quantizer.install_frame();
    let ReconRow {
        superblocks,
        temporal,
        ..
    } = row;
    for superblock in superblocks {
        debug_assert!(
            superblock
                .entries
                .iter()
                .all(|entry| entry.publication.superblock_origin() == superblock.origin)
        );
        let fully_precomputed = superblock.dependency == ReconDependency::ReferenceOnly
            && !superblock.entries.is_empty()
            && superblock
                .entries
                .iter()
                .all(|entry| entry.command.is_none() && entry.error.is_none());
        if fully_precomputed {
            let (Some(first), Some(last)) = (superblock.entries.first(), superblock.entries.last())
            else {
                continue;
            };
            let records = temporal
                .get(first.temporal.start..last.temporal.end)
                .ok_or_else(|| {
                    inter_cap!(
                        "inter_row_precomputed_temporal_range",
                        tile_offset,
                        "inter.row.task_capacity",
                        SPEC_MODE_INFO
                    )
                })?;
            super::temporal::commit_temporal_motion_blocks(motion_field, records);
            continue;
        }
        for mut entry in superblock.entries {
            entry
                .publication
                .prepare_block_decoded(block_decoded, current_superblock);
            if let Some(error) = entry.error.take() {
                return Err(error);
            }
            if let Some(command) = entry.command {
                match command {
                    ReconCommand::GeneralIntra(command) => {
                        command.reconstruct(workspace, block_decoded)?;
                    }
                    ReconCommand::Intrabc(command) => command.reconstruct(workspace)?,
                    ReconCommand::Inter(command) => scratch.reconstruct(
                        command,
                        workspace,
                        block_decoded,
                        motion_field,
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
                    )?,
                }
            } else {
                let records = temporal.get(entry.temporal).ok_or_else(|| {
                    inter_cap!(
                        "inter_row_replay_temporal_range",
                        tile_offset,
                        "inter.row.task_capacity",
                        SPEC_MODE_INFO
                    )
                })?;
                super::temporal::commit_temporal_motion_blocks(motion_field, records);
            }
            entry
                .publication
                .publish_block_decoded(block_decoded)
                .map_err(|_| {
                    inter_cap!(
                        "inter_row_block_decoded_publish",
                        tile_offset,
                        "inter.partition_walk",
                        SPEC_MODE_INFO
                    )
                })?;
        }
    }
    *decoded_any |= row_has_entries;
    if let Some(error) = terminal {
        return Err(error);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn decode_tiles<T: ReconSample>(
    work_units: &mut [DecodeTileWorkUnit<'_>],
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    limits: crate::DecodeLimits,
    chroma_smooth_rows: usize,
    chroma_smooth_cols: usize,
    mi_rows: usize,
    mi_cols: usize,
    sb_h4: usize,
    max_drl_bits_minus_1: u32,
    frame_interpolation_filter: FrameInterpolationFilter,
    residual_tool_policy: TransformToolResidualPolicy,
    num_total_refs: usize,
    reference_select: bool,
    num_same_ref_compound: u8,
    temporal_context: &TemporalMvContext,
    reference: &InterReferenceState<'_, T>,
    workspace: &mut CurrentFrameWorkspace<T>,
    luma_use_tcq: bool,
    residual_use_ddt: bool,
    ref_frame_idx: &[u32],
    bit_depth: BitDepth,
    enable_adaptive_mvd: bool,
    allow_bawp: bool,
    allow_warpmv_mode: bool,
    frame_is_switch: bool,
    current_order_hint: u32,
    mut cdef_state: CdefState,
    mut gdf_state: GdfState,
    mut ccso_state: CcsoState,
    mut motion_field: TemporalMotionField,
) -> Result<TileDecodeOutput> {
    let chroma = sequence.general.chroma_format_idc;
    let mut deblock_blocks = Vec::new();
    let mut chroma_deblock_blocks = [Vec::new(), Vec::new()];
    let mut tx_skip_records = Vec::new();
    let (mut active_source_blocks, mut unit_filters) = (Vec::new(), Vec::new());
    let mut decoded_any = false;
    let chunk_offset = work_units
        .first()
        .map_or(ByteOffset::new(0), |tile| tile.tile_byte_span().start);
    for tile in work_units.iter_mut() {
        let tile_offset = tile.tile_byte_span().start;
        let mut coeff_ctx =
            TileCoeffContextState::new_chroma(mi_rows, mi_cols, chroma).map_err(|_| {
                inter_cap!(
                    "inter_coeff_context_state",
                    tile_offset,
                    "inter.residual_context_state",
                    SPEC_MODE_INFO
                )
            })?;
        let mut delta_q_state = DeltaQState::new(sequence, core, tile_offset)?;
        let mut intrabc_state = TileIntrabcPreludeState::new(
            mi_rows,
            mi_cols,
            sequence,
            core.frame_is_intra == Some(true),
            tile_offset,
        )?;
        let mut segment_id_state = TileSegmentIdState::new(mi_rows, mi_cols).map_err(|_| {
            inter_missing!(
                "inter_segment_id_grid",
                tile_offset,
                "inter.segment_id_grid",
                SPEC_MODE_INFO
            )
        })?;
        let mut mv_grid = NeighbourMvGrid::new(mi_rows, mi_cols).ok_or_else(|| {
            inter_cap!(
                "inter_mv_grid",
                tile_offset,
                "inter.mv_grid",
                SPEC_MODE_INFO
            )
        })?;
        let mut y_smooth = crate::prediction::intra_edge::TileYSmoothGrid::new(mi_rows, mi_cols)
            .ok_or_else(|| {
                inter_cap!(
                    "inter_y_smooth_grid",
                    tile_offset,
                    "inter.y_smooth_grid",
                    SPEC_MODE_INFO
                )
            })?;
        let mut chroma_smooth = crate::prediction::intra_edge::TileChromaSmoothGrid::new(
            chroma_smooth_rows,
            chroma_smooth_cols,
        )
        .ok_or_else(|| {
            inter_cap!(
                "inter_chroma_smooth_grid",
                tile_offset,
                "inter.chroma_smooth_grid",
                SPEC_MODE_INFO
            )
        })?;
        let mut inter_recon_scratch = deferred_recon::InterReconScratch::default();
        let mut ref_mv_bank = sequence
            .inter
            .as_ref()
            .is_some_and(|inter| inter.enable_refmvbank)
            .then(super::super::find_mv_stack::RefMvBank::new);
        let mut warp_param_bank = super::super::find_mv_stack::WarpParamBank::new();
        let walk =
            GeneralIntraMultiblockCursor::new(tile, sequence, core, limits).map_err(|error| {
                map_inter_multiblock_error(
                    GeneralIntraMultiblockError::<crate::DecodeError>::Setup(error),
                    tile_offset,
                )
            })?;
        let (subsampling_x, subsampling_y) = chroma_subsampling(chroma);
        let mut block_decoded = TileBlockDecodedState::new(
            if chroma == ChromaFormatIdc::Monochrome {
                1
            } else {
                3
            },
            usize::from(subsampling_x),
            usize::from(subsampling_y),
            sb_h4,
            (tile.mi_col_range().end as usize).min(mi_cols),
            (tile.mi_row_range().end as usize).min(mi_rows),
        )
        .map_err(|_| {
            inter_cap!(
                "inter_row_block_decoded_init",
                tile_offset,
                "inter.partition_walk",
                SPEC_MODE_INFO
            )
        })?;
        let mut current_block_decoded_superblock = None;
        let quantizer = FrameQuantizerSnapshot::capture();
        let mut walk = Some(walk);
        let mut parser_ordinal = 0usize;
        let mut recon_ordinal = 0usize;
        let mut tile_walk_output = None;
        let tile_rows = tile.mi_row_range();
        let tile_first_band = tile_rows.start as usize / sb_h4;
        let mut parse_row = || {
            let _quantizer_scopes = quantizer.install_frame();
            let mut recon_row = ReconRow {
                ordinal: parser_ordinal,
                superblocks: Vec::new(),
                temporal: Vec::new(),
                terminal: None,
            };
            parser_ordinal = parser_ordinal.saturating_add(1);
            let decoded_row = if let Some(walk) = walk.as_mut() {
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
                            sequence,
                            core,
                            &mut coeff_ctx,
                            &mut gdf_state,
                            &mut cdef_state,
                            &mut ccso_state,
                            &mut delta_q_state,
                            &mut intrabc_state,
                            &mut segment_id_state,
                            &mut mv_grid,
                            temporal_context,
                            &mut y_smooth,
                            &mut chroma_smooth,
                            &mut ref_mv_bank,
                            &mut warp_param_bank,
                            sb_h4,
                            mi_rows,
                            mi_cols,
                            max_drl_bits_minus_1,
                            frame_interpolation_filter,
                            residual_tool_policy,
                            num_total_refs,
                            reference_select,
                            num_same_ref_compound,
                            joint_modes,
                            uses_mrls,
                            use_dip,
                            fsc_modes,
                            palette_state,
                            is_cfl_ctx,
                            &mut deblock_blocks,
                            &mut chroma_deblock_blocks,
                            &mut tx_skip_records,
                            luma_use_tcq,
                            residual_use_ddt,
                            ref_frame_idx,
                            reference,
                            bit_depth,
                            enable_adaptive_mvd,
                            allow_bawp,
                            allow_warpmv_mode,
                            frame_is_switch,
                            current_order_hint,
                            tile_offset,
                        )
                    };
                let mut on_published = |publication, command| {
                    recon_row.push(publication, command);
                };
                walk.decode_next_sb_row(tile, &mut decode_leaf, &mut on_published)
            } else {
                recon_row.terminal = Some(inter_cap!(
                    "inter_row_parser_missing_walk",
                    tile_offset,
                    "inter.row.parser_state",
                    SPEC_MODE_INFO
                ));
                return ParserStep::Last(recon_row);
            };
            match decoded_row {
                Ok(Some(_)) => ParserStep::More(recon_row),
                Err(error) => {
                    recon_row.terminal = Some(map_inter_multiblock_error(error, tile_offset));
                    ParserStep::Last(recon_row)
                }
                Ok(None) => {
                    let Some(walk) = walk.take() else {
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
                        active_source_blocks: tile_source_blocks,
                        unit_filters: tile_unit_filters,
                    } = walk.into_output();
                    recon_row.terminal = symbols.exit_symbol().err().map(|_| {
                        if reference_select {
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
                    tile_walk_output = Some((tile_source_blocks, tile_unit_filters));
                    ParserStep::Last(recon_row)
                }
            }
        };
        let parallel_prepass = splot_parallel::on_multiworker_pool()
            && !super::intrabc::global_intrabc_enabled(core.intrabc);
        if parallel_prepass {
            let sb_size = match sb_h4 {
                16 => splot_core::headers::sequence::SuperblockSize::Block64x64,
                32 => splot_core::headers::sequence::SuperblockSize::Block128x128,
                64 => splot_core::headers::sequence::SuperblockSize::Block256x256,
                _ => {
                    return Err(inter_cap!(
                        "inter_row_invalid_superblock_height",
                        tile_offset,
                        "inter.row.task_capacity",
                        SPEC_MODE_INFO
                    ));
                }
            };
            let mut bands = workspace.sb_row_bands(sb_size).skip(tile_first_band);
            let done_limit = (tile_rows.end as usize)
                .min(mi_rows)
                .saturating_sub(tile_rows.start as usize)
                .div_ceil(sb_h4)
                .checked_add(1)
                .ok_or_else(|| {
                    inter_cap!(
                        "inter_row_done_limit_overflow",
                        tile_offset,
                        "inter.row.task_capacity",
                        SPEC_MODE_INFO
                    )
                })?;
            let parse_ready = || {
                let step = parse_row();
                let (row, last) = match step {
                    ParserStep::More(row) => (row, false),
                    ParserStep::Last(row) => (row, true),
                };
                let band = if row.superblocks.is_empty() {
                    None
                } else {
                    bands.next()
                };
                let ready = ReadyReconRow { row, band };
                if last {
                    ParserStep::Last(ready)
                } else {
                    ParserStep::More(ready)
                }
            };
            let mut prepared = run_ready_row_prepass_parallel(
                parse_ready,
                |ready| {
                    precompute_recon_row(
                        ready,
                        &block_decoded,
                        &quantizer,
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
                    )
                },
                done_limit,
            )
            .map_err(|error| match error {
                ReadyRowPipelineError::Parallel => inter_cap!(
                    "inter_row_prepass_scope",
                    tile_offset,
                    "inter.row.task_scope",
                    SPEC_MODE_INFO
                ),
                ReadyRowPipelineError::Capacity | ReadyRowPipelineError::Codec(()) => inter_cap!(
                    "inter_row_prepass_capacity",
                    tile_offset,
                    "inter.row.task_capacity",
                    SPEC_MODE_INFO
                ),
            })?;
            let active_limit = splot_parallel::current_pool_width().saturating_sub(1);
            if prepared.max_pending > READY_ROW_CAPACITY || prepared.max_active > active_limit {
                return Err(inter_cap!(
                    "inter_row_prepass_bounds",
                    tile_offset,
                    "inter.row.task_capacity",
                    SPEC_MODE_INFO
                ));
            }
            prepared.rows.sort_by_key(|row| row.ordinal);
            for row in prepared.rows {
                replay_recon_row(
                    row,
                    &mut recon_ordinal,
                    &mut decoded_any,
                    &quantizer,
                    &mut inter_recon_scratch,
                    workspace,
                    &mut block_decoded,
                    &mut current_block_decoded_superblock,
                    &mut motion_field,
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
            }
        } else {
            let replay_row = |row: ReconRow<T>| {
                replay_recon_row(
                    row,
                    &mut recon_ordinal,
                    &mut decoded_any,
                    &quantizer,
                    &mut inter_recon_scratch,
                    workspace,
                    &mut block_decoded,
                    &mut current_block_decoded_superblock,
                    &mut motion_field,
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
                )
            };
            run_ready_row_pipeline_serial(parse_row, replay_row).map_err(|error| match error {
                ReadyRowPipelineError::Codec(error) => error,
                ReadyRowPipelineError::Capacity => inter_cap!(
                    "inter_row_serial_capacity",
                    tile_offset,
                    "inter.row.task_capacity",
                    SPEC_MODE_INFO
                ),
                ReadyRowPipelineError::Parallel => inter_cap!(
                    "inter_row_serial_scope",
                    tile_offset,
                    "inter.row.task_scope",
                    SPEC_MODE_INFO
                ),
            })?;
        }
        let Some((tile_source_blocks, tile_unit_filters)) = tile_walk_output else {
            return Err(inter_cap!(
                "inter_row_parser_output",
                tile_offset,
                "inter.row.parser_output",
                SPEC_MODE_INFO
            ));
        };
        active_source_blocks.extend(tile_source_blocks);
        unit_filters.extend(tile_unit_filters);
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
        deblock_blocks,
        chroma_deblock_blocks,
        tx_skip_records,
        active_source_blocks,
        unit_filters,
    })
}

#[cfg(test)]
mod ready_row_tests {
    #![allow(clippy::expect_used)]

    use std::num::NonZeroUsize;
    use std::sync::{Arc, Barrier};

    use splot_parallel::{ThreadCount, WorkerPool};

    use super::*;

    #[test]
    fn recon_entries_are_bucketed_by_contiguous_superblock_without_reordering() {
        let mut superblocks = Vec::new();
        push_recon_entry(&mut superblocks, [0, 0], ReconDependency::ReferenceOnly, 0);
        push_recon_entry(&mut superblocks, [0, 0], ReconDependency::CurrentFrame, 1);
        push_recon_entry(&mut superblocks, [0, 16], ReconDependency::ReferenceOnly, 2);
        push_recon_entry(
            &mut superblocks,
            [0, 0],
            ReconDependency::GlobalIntrabcFence,
            3,
        );

        assert_eq!(
            superblocks
                .iter()
                .map(|superblock| superblock.origin)
                .collect::<Vec<_>>(),
            [[0, 0], [0, 16], [0, 0]]
        );
        assert_eq!(
            superblocks
                .iter()
                .flat_map(|superblock| superblock.entries.iter().copied())
                .collect::<Vec<_>>(),
            [0, 1, 2, 3]
        );
    }

    #[test]
    fn recon_superblock_retains_the_strongest_dependency() {
        let mut superblocks = Vec::new();
        for dependency in [
            ReconDependency::ReferenceOnly,
            ReconDependency::GlobalIntrabcFence,
            ReconDependency::CurrentFrame,
        ] {
            push_recon_entry(&mut superblocks, [0, 0], dependency, ());
        }

        assert_eq!(superblocks.len(), 1);
        assert_eq!(
            superblocks[0].dependency,
            ReconDependency::GlobalIntrabcFence
        );
    }

    #[test]
    fn mixed_superblocks_are_skipped_and_reference_rows_resume_exactly_once() {
        assert!(!select_prepass_superblock(
            ReconDependency::CurrentFrame,
            true,
            true
        ));
        let selected = [
            ReconDependency::ReferenceOnly,
            ReconDependency::CurrentFrame,
            ReconDependency::ReferenceOnly,
        ]
        .map(|dependency| select_prepass_superblock(dependency, true, true));
        assert_eq!(selected, [true, false, true]);
        assert_eq!(selected.into_iter().filter(|selected| *selected).count(), 2);
    }

    #[test]
    fn ready_rows_respect_capacity_and_active_bounds() {
        let mut next = 0usize;
        let parser = move || {
            let row = next;
            next += 1;
            if row == 5 {
                ParserStep::Last(row)
            } else {
                ParserStep::More(row)
            }
        };
        let barrier = Arc::new(Barrier::new(3));
        let work = move |row| {
            barrier.wait();
            row
        };
        let pool = WorkerPool::new(ThreadCount::Fixed(
            NonZeroUsize::new(4).expect("four workers"),
        ))
        .expect("worker pool");

        let mut prepared = pool
            .install(|| run_ready_row_prepass_parallel(parser, work, 6))
            .expect("row pipeline");

        assert!(prepared.max_pending <= READY_ROW_CAPACITY);
        assert_eq!(prepared.max_active, 3);
        prepared.rows.sort_unstable();
        assert_eq!(prepared.rows, [0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn completed_row_overflow_fails_closed() {
        let mut next = 0usize;
        let parser = move || {
            let row = next;
            next += 1;
            if row == 1 {
                ParserStep::Last(row)
            } else {
                ParserStep::More(row)
            }
        };
        let pool = WorkerPool::new(ThreadCount::Fixed(
            NonZeroUsize::new(2).expect("two workers"),
        ))
        .expect("worker pool");

        let result = pool.install(|| run_ready_row_prepass_parallel(parser, |row| row, 1));

        assert!(matches!(result, Err(ReadyRowPipelineError::Capacity)));
    }

    #[test]
    fn reconstruction_error_precedes_terminal_parser_error() {
        let result = run_ready_row_pipeline_serial(
            || ParserStep::Last(Some("parser error")),
            |_| Err("reconstruction error"),
        );

        assert!(matches!(
            result,
            Err(ReadyRowPipelineError::Codec("reconstruction error"))
        ));
    }

    #[test]
    fn completed_rows_are_replayed_in_canonical_order() {
        let mut rows = [
            (2, vec![20, 21], Some("third")),
            (0, vec![0], None),
            (1, vec![10, 11], Some("second")),
        ];
        rows.sort_by_key(|row| row.0);

        let logs = rows
            .iter()
            .flat_map(|row| row.1.iter().copied())
            .collect::<Vec<_>>();
        let first_error = rows.iter().find_map(|row| row.2);
        assert_eq!(logs, [0, 10, 11, 20, 21]);
        assert_eq!(first_error, Some("second"));
    }

    #[test]
    fn zero_record_done_entries_form_one_empty_canonical_range() {
        let ranges = [0..0, 0..0, 0..0];
        let canonical = ranges
            .first()
            .zip(ranges.last())
            .map(|(first, last)| first.start..last.end);

        assert_eq!(canonical, Some(0..0));
    }

    #[test]
    fn prepass_error_excludes_sb_without_restoring_consumed_command() {
        let command: Option<()> = None;
        let error = Some("injected prepass error");
        let fully_precomputed = command.is_none() && error.is_none();

        assert!(!fully_precomputed);
        assert!(
            command.is_none(),
            "the partially written command must not retry"
        );
    }
}

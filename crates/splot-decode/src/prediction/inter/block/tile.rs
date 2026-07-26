// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Tile-local block decode and reconstruction.
//!
//! Feature tracking: `INFRA-DECODE-PARALLEL-STAGES`.

use std::collections::VecDeque;
use std::ops::Range;
use std::sync::{Mutex, MutexGuard};

use splot_recon::{PlaneId, ReconError};

use super::*;

const READY_JOB_CAPACITY_PER_WORKER: usize = 2;

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

trait OrderedDone {
    fn ordinal(&self) -> usize;
}

impl OrderedDone for usize {
    fn ordinal(&self) -> usize {
        *self
    }
}

struct ReadyRowCoordinator<Parser, Ready, Done, Commit, E> {
    parser: Option<Parser>,
    ready: VecDeque<Ready>,
    ready_limit: usize,
    deferred: VecDeque<Ready>,
    settled: bool,
    done: Vec<Option<Done>>,
    done_limit: usize,
    committed: usize,
    next_commit: usize,
    commit: Option<Commit>,
    commit_error: Option<E>,
    commit_active: bool,
    capacity_error: bool,
    parser_active: bool,
    parser_done: bool,
    active_tasks: usize,
    active_limit: usize,
    max_pending: usize,
    max_deferred: usize,
    max_active: usize,
    parse_timer: Option<std::time::Instant>,
    drain_timer: Option<std::time::Instant>,
    flip_timer: Option<std::time::Instant>,
}

fn lock_ready_rows<Parser, Ready, Done, Commit, E>(
    coordinator: &Mutex<ReadyRowCoordinator<Parser, Ready, Done, Commit, E>>,
) -> MutexGuard<'_, ReadyRowCoordinator<Parser, Ready, Done, Commit, E>> {
    coordinator
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Moves every admitted deferred row onto the ready queue, scanning them in
/// parse order and leaving the rest queued in that same order.
///
/// Reconstruction is order-free — each row precomputes into its own surface —
/// so a row whose references are short holds back only its own reconstruction,
/// not the rows behind it. The ordered commit frontier still publishes rows in
/// parse order, and a row's own commit runs after its own admission, so the
/// watermark it was admitted against has only grown by then.
///
/// Returns whether any row was released.
fn release_ready_rows<Parser, Ready, Done, Commit, E>(
    state: &mut ReadyRowCoordinator<Parser, Ready, Done, Commit, E>,
    gate: &impl Fn(&Ready) -> bool,
) -> bool {
    let mut released = false;
    let mut scanned = state.deferred.len();
    while scanned > 0 && state.ready.len() < state.ready_limit {
        scanned -= 1;
        let Some(row) = state.deferred.pop_front() else {
            break;
        };
        if !state.settled && !gate(&row) {
            state.deferred.push_back(row);
            continue;
        }
        state.ready.push_back(row);
        state.max_pending = state.max_pending.max(state.ready.len());
        released = true;
        if state.flip_timer.is_some() {
            crate::timing::report("walk_refs_flip", state.flip_timer.take());
        }
    }
    released
}

fn take_ordered_commit<Parser, Ready, Done, Commit, E>(
    state: &mut ReadyRowCoordinator<Parser, Ready, Done, Commit, E>,
) -> Option<(Done, Commit)> {
    if state.commit_active || state.capacity_error || state.commit_error.is_some() {
        return None;
    }
    let index = state.next_commit;
    let done = state.done.get_mut(index).and_then(Option::take)?;
    let Some(commit) = state.commit.take() else {
        state.capacity_error = true;
        if let Some(slot) = state.done.get_mut(index) {
            *slot = Some(done);
        }
        return None;
    };
    state.commit_active = true;
    Some((done, commit))
}

fn schedule_ready_rows<'scope, Parser, Work, Gate, Ready, Done, Commit, E>(
    scope: &splot_parallel::TaskScope<'_, 'scope>,
    coordinator: &'scope Mutex<ReadyRowCoordinator<Parser, Ready, Done, Commit, E>>,
    work: &'scope Work,
    gate: &'scope Gate,
) where
    Parser: FnMut() -> ParserStep<Ready> + Send + 'scope,
    Work: Fn(Ready) -> Done + Sync + 'scope,
    Gate: Fn(&Ready) -> bool + Sync + 'scope,
    Ready: Send + 'scope,
    Done: OrderedDone + Send + 'scope,
    Commit: FnMut(Done) -> core::result::Result<(), E> + Send + 'scope,
    E: Send + 'scope,
{
    let (spawn_parser, ordered_commit) = {
        let mut state = lock_ready_rows(coordinator);
        release_ready_rows(&mut state, gate);
        if state.capacity_error || state.commit_error.is_some() {
            (false, None)
        } else {
            let spawn_parser = !state.parser_done
                && !state.parser_active
                && state.parser.is_some()
                && state.ready.len() < state.ready_limit;
            if spawn_parser {
                state.parser_active = true;
            }
            let ordered_commit = take_ordered_commit(&mut state);
            (spawn_parser, ordered_commit)
        }
    };
    if let Some((done, mut commit)) = ordered_commit {
        scope.spawn(move |scope| {
            let result = commit(done);
            {
                let mut state = lock_ready_rows(coordinator);
                state.commit = Some(commit);
                state.commit_active = false;
                match result {
                    Ok(()) => {
                        state.committed = state.committed.saturating_add(1);
                        state.next_commit = state.next_commit.saturating_add(1);
                    }
                    Err(error) => state.commit_error = Some(error),
                }
            }
            schedule_ready_rows(scope, coordinator, work, gate);
        });
    }
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
            let overflow = {
                let mut state = lock_ready_rows(coordinator);
                state.parser = parser;
                state.parser_active = false;
                if last {
                    crate::timing::report("walk_parse", state.parse_timer.take());
                    state.drain_timer = crate::timing::start();
                }
                state.parser_done |= last;
                admit_parsed_row(&mut state, row)
            };
            drop(overflow);
            schedule_ready_rows(scope, coordinator, work, gate);
        });
    }
    loop {
        let ready = {
            let mut state = lock_ready_rows(coordinator);
            take_ready_row(&mut state)
        };
        let Some(ready) = ready else {
            break;
        };
        scope.spawn(move |scope| {
            let done = work(ready);
            let mut overflow = None;
            {
                let mut state = lock_ready_rows(coordinator);
                state.active_tasks = state.active_tasks.saturating_sub(1);
                let ordinal = done.ordinal();
                let slot_available = state.done.get(ordinal).is_some_and(Option::is_none);
                if state.capacity_error || ordinal >= state.done_limit || !slot_available {
                    state.capacity_error = true;
                    overflow = Some(done);
                } else if let Some(slot) = state.done.get_mut(ordinal) {
                    *slot = Some(done);
                } else {
                    state.capacity_error = true;
                    overflow = Some(done);
                }
            }
            drop(overflow);
            schedule_ready_rows(scope, coordinator, work, gate);
        });
    }
}

/// Queues one parsed row behind the rows still waiting for their references.
fn admit_parsed_row<Parser, Ready, Done, Commit, E>(
    state: &mut ReadyRowCoordinator<Parser, Ready, Done, Commit, E>,
    row: Ready,
) -> Option<Ready> {
    if state.capacity_error || state.deferred.len() >= state.done_limit {
        state.capacity_error = true;
        return Some(row);
    }
    state.deferred.push_back(row);
    state.max_deferred = state.max_deferred.max(state.deferred.len());
    None
}

/// Claims one released row for a precompute task.
fn take_ready_row<Parser, Ready, Done, Commit, E>(
    state: &mut ReadyRowCoordinator<Parser, Ready, Done, Commit, E>,
) -> Option<Ready> {
    if state.capacity_error
        || state.commit_error.is_some()
        || state.active_tasks >= state.active_limit
    {
        return None;
    }
    let ready = state.ready.pop_front()?;
    state.active_tasks += 1;
    state.max_active = state.max_active.max(state.active_tasks);
    Some(ready)
}

struct ReadyPipelineStats {
    committed: usize,
    ready_limit: usize,
    max_pending: usize,
    max_deferred: usize,
    max_active: usize,
    /// Whether the drain had to fall back to settling whole reference frames.
    settled: bool,
}

/// Runs one tile's parse-ahead prepass, holding each parsed row back until
/// `gate` reports the reference rows that row reads have been published.
///
/// Parsing never waits on the gate: rows parsed while their references are
/// short queue up, and every row the gate admits is released in parse order
/// while the rest stay queued in that same order.
///
/// When parsing runs out with rows still held, the driver donates its wait to
/// the pool — the references' own filter phases run there — and re-tests
/// between steps. `settled` bounds that loop: every reference settles in the
/// end and a settled reference admits every row, so reaching `settled` with a
/// row still held means the gate could not classify what holds it, and `settle`
/// then blocks the driver once and admits every remaining row.
fn run_ready_row_prepass_with_commit<Parser, Work, Gate, Settled, Settle, Ready, Done, Commit, E>(
    parser: Parser,
    work: Work,
    commit: Commit,
    done_limit: usize,
    gate: Gate,
    settled: Settled,
    settle: Settle,
) -> core::result::Result<ReadyPipelineStats, ReadyRowPipelineError<E>>
where
    Parser: FnMut() -> ParserStep<Ready> + Send,
    Work: Fn(Ready) -> Done + Send + Sync,
    Gate: Fn(&Ready) -> bool + Send + Sync,
    Settled: Fn() -> bool,
    Settle: FnOnce() -> core::result::Result<(), E>,
    Ready: Send,
    Done: OrderedDone + Send,
    Commit: FnMut(Done) -> core::result::Result<(), E> + Send,
    E: Send,
{
    let active_limit = splot_parallel::current_pool_width()
        .saturating_sub(1)
        .max(1);
    let ready_limit = active_limit
        .saturating_mul(READY_JOB_CAPACITY_PER_WORKER)
        .min(done_limit)
        .max(1);
    let mut done = Vec::new();
    done.try_reserve_exact(done_limit)
        .map_err(|_| ReadyRowPipelineError::Capacity)?;
    done.resize_with(done_limit, || None);
    let coordinator = Mutex::new(ReadyRowCoordinator {
        parser: Some(parser),
        ready: VecDeque::with_capacity(ready_limit),
        ready_limit,
        deferred: VecDeque::new(),
        settled: false,
        done,
        done_limit,
        committed: 0,
        next_commit: 0,
        commit: Some(commit),
        commit_error: None,
        commit_active: false,
        capacity_error: false,
        parser_active: false,
        parser_done: false,
        active_tasks: 0,
        active_limit,
        max_pending: 0,
        max_deferred: 0,
        max_active: 0,
        parse_timer: crate::timing::start(),
        drain_timer: None,
        flip_timer: crate::timing::start(),
    });
    let run_scope = || {
        splot_parallel::ready_task_scope(|scope| {
            schedule_ready_rows(scope, &coordinator, &work, &gate);
        })
        .map_err(|_| ReadyRowPipelineError::Parallel)
    };
    run_scope()?;
    let drain_timer = crate::timing::start();
    while ready_rows_await_references(&coordinator) {
        let released = {
            let mut state = lock_ready_rows(&coordinator);
            release_ready_rows(&mut state, &gate)
        };
        if released {
            run_scope()?;
        } else if settled() {
            break;
        } else {
            splot_parallel::assist_pool_or_park();
        }
    }
    crate::timing::report("walk_refs_drain", drain_timer);
    if ready_rows_await_references(&coordinator) {
        settle().map_err(ReadyRowPipelineError::Codec)?;
        {
            let mut state = lock_ready_rows(&coordinator);
            state.settled = true;
            crate::timing::report("walk_refs_flip", state.flip_timer.take());
        }
        run_scope()?;
    }
    let state = coordinator
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    crate::timing::report("walk_commit_drain", state.drain_timer);
    if let Some(error) = state.commit_error {
        return Err(ReadyRowPipelineError::Codec(error));
    }
    if state.capacity_error
        || !state.parser_done
        || state.parser_active
        || state.commit_active
        || state.active_tasks != 0
        || !state.ready.is_empty()
        || !state.deferred.is_empty()
        || state.done.iter().any(Option::is_some)
    {
        return Err(ReadyRowPipelineError::Capacity);
    }
    Ok(ReadyPipelineStats {
        committed: state.committed,
        ready_limit: state.ready_limit,
        max_pending: state.max_pending,
        max_deferred: state.max_deferred,
        max_active: state.max_active,
        settled: state.settled,
    })
}

/// Whether the prepass ran out of parse work with rows still waiting for the
/// reference rows they read.
fn ready_rows_await_references<Parser, Ready, Done, Commit, E>(
    coordinator: &Mutex<ReadyRowCoordinator<Parser, Ready, Done, Commit, E>>,
) -> bool {
    let state = lock_ready_rows(coordinator);
    !state.deferred.is_empty() && !state.capacity_error && state.commit_error.is_none()
}

fn run_ready_row_pipeline_serial<Parser, Recon, Row, E>(
    mut parser: Parser,
    mut recon: Recon,
) -> core::result::Result<(), E>
where
    Parser: FnMut() -> ParserStep<Row>,
    Recon: FnMut(Row) -> core::result::Result<(), E>,
{
    loop {
        let (row, last) = match parser() {
            ParserStep::More(row) => (row, false),
            ParserStep::Last(row) => (row, true),
        };
        recon(row)?;
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
    temporal_context: &'a TemporalMvContext,
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
    ref_mv_bank: Option<super::super::find_mv_stack::RefMvBank>,
    warp_param_bank: super::super::find_mv_stack::WarpParamBank,
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
        let ref_mv_bank = context
            .sequence
            .inter
            .as_ref()
            .is_some_and(|inter| inter.enable_refmvbank)
            .then(super::super::find_mv_stack::RefMvBank::new);
        let warp_param_bank = super::super::find_mv_stack::WarpParamBank::new();
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
            ref_mv_bank,
            warp_param_bank,
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
                        context.temporal_context,
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
        let resolved = self.resolve_unit(context, &mut recon_row);
        self.entry_capacity = self.entry_capacity.max(recon_row.entries.capacity());
        self.superblock_capacity = self
            .superblock_capacity
            .max(recon_row.superblocks.capacity());
        let step = match decoded_row {
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
        };
        let Err(error) = resolved else {
            return step;
        };
        let mut row = match step {
            ParserStep::More(row) | ParserStep::Last(row) => row,
        };
        row.terminal = Some(error);
        ParserStep::Last(row)
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
        context: &TileDecodeContext<'_, T>,
        row: &mut ReconRow,
    ) -> Result<()> {
        resolve_parsed_leaves(
            &mut row.motion_queue,
            &mut row.pending_inter,
            &mut row.entries,
            &mut MvResolutionState {
                grid: &mut self.mv_grid,
                ref_mv_bank: &mut self.ref_mv_bank,
                warp_param_bank: &mut self.warp_param_bank,
                core: context.core,
                temporal: frame_uses_temporal_mvs(context.core).then_some(context.temporal_context),
                order_hints: context.temporal_context.order_hint_mv_context(),
                drl_reorder: sequence_drl_reorder(context.sequence),
                max_drl_bits_minus_1: context.max_drl_bits_minus_1,
                frame_precision: 0,
                tile_offset: self.tile.tile_byte_span().start,
            },
            context.sb_h4,
        )
    }

    fn next_row<T: ReconSample>(
        &mut self,
        context: &TileDecodeContext<'_, T>,
    ) -> ParserStep<ReconRow> {
        self.next_unit(context, ParserGranularity::Row, None)
    }

    fn next_row_reusing<T: ReconSample>(
        &mut self,
        context: &TileDecodeContext<'_, T>,
        buffers: ReconRowBuffers,
    ) -> ParserStep<ReconRow> {
        self.next_unit(context, ParserGranularity::Row, Some(buffers))
    }

    fn next_superblock_reusing<T: ReconSample>(
        &mut self,
        context: &TileDecodeContext<'_, T>,
        buffers: ReconRowBuffers,
    ) -> ParserStep<ReconRow> {
        self.next_unit(context, ParserGranularity::Superblock, Some(buffers))
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
    let storage = workspace.plane(PlaneId::Y)?.storage_size();
    let mi_rows = tile.mi_row_range();
    let mi_cols = tile.mi_col_range();
    let x = (mi_cols.start as usize)
        .checked_mul(4)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "tile rectangle luma x",
        })?;
    let y = (mi_rows.start as usize)
        .checked_mul(4)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "tile rectangle luma y",
        })?;
    let right = (mi_cols.end as usize)
        .checked_mul(4)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "tile rectangle luma right edge",
        })?
        .min(storage.width());
    let bottom = (mi_rows.end as usize)
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
    let tile_offset = tile.tile_byte_span().start;
    let bounds = tile_luma_rect(tile, workspace)?;
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
    let mut sink = super::super::mc::WorkspaceSink::Rect(&mut surface);
    loop {
        let _quantizer_scopes = quantizer.install_frame();
        let step = parser.next_row(context);
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
                context.temporal_context,
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
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
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
    temporal_context: &TemporalMvContext,
    reference: &InterReferenceState<T>,
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
    scratch.workers.ensure_workers(
        splot_parallel::current_pool_width()
            .saturating_sub(1)
            .max(1),
    );
    let TileDecodeScratch { ordered, workers } = scratch;
    let context = TileDecodeContext {
        sequence,
        core,
        limits,
        mi_rows,
        mi_cols,
        sb_h4,
        max_drl_bits_minus_1,
        frame_interpolation_filter,
        residual_tool_policy,
        num_total_refs,
        reference_select,
        num_same_ref_compound,
        temporal_context,
        reference,
        luma_use_tcq,
        residual_use_ddt,
        ref_frame_idx,
        bit_depth,
        enable_adaptive_mvd,
        allow_bawp,
        allow_warpmv_mode,
        frame_is_switch,
        current_order_hint,
    };
    frame_filter_records.clear();
    let mut decoded_any = false;
    let chunk_offset = work_units
        .first()
        .map_or(ByteOffset::new(0), |tile| tile.tile_byte_span().start);
    let row_gate =
        row_gate::RowReferenceGate::new(reference, core, ref_frame_idx, workspace.info());
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
                        tile, surface, context, cdef_state, gdf_state, ccso_state, quantizer,
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
            let TileParserOutput {
                cdef_state: tile_cdef_state,
                gdf_state: tile_gdf_state,
                ccso_state: tile_ccso_state,
                active_source_blocks: tile_source_blocks,
                unit_filters: tile_unit_filters,
            } = tile.output;
            cdef_state.merge_tile(
                &tile_cdef_state,
                tile.mi_rows.clone(),
                tile.mi_cols.clone(),
                tile.tile_offset,
            )?;
            gdf_state.merge_tile(
                &tile_gdf_state,
                tile.mi_rows.clone(),
                tile.mi_cols.clone(),
                tile.tile_offset,
            )?;
            ccso_state.merge_tile(
                &tile_ccso_state,
                tile.mi_rows.clone(),
                tile.mi_cols.clone(),
                tile.tile_offset,
            )?;
            append_lr_records(
                &mut frame_filter_records.lr_source_blocks,
                &mut frame_filter_records.lr_unit_filters,
                tile_source_blocks,
                tile_unit_filters,
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
        if parallel_prepass {
            let Some(rects) = superblock_rects else {
                return Err(inter_cap!(
                    "inter_superblock_surface_state",
                    tile_offset,
                    "inter.superblock.task_capacity",
                    SPEC_MODE_INFO
                ));
            };
            let done_limit = rects.len().checked_add(1).ok_or_else(|| {
                inter_cap!(
                    "inter_superblock_done_limit_overflow",
                    tile_offset,
                    "inter.superblock.task_capacity",
                    SPEC_MODE_INFO
                )
            })?;
            let mut shadow = CurrentFrameWorkspace::new(workspace.info(), T::default())?;
            let mut surfaces = shadow.rect_surfaces(&rects)?.into_iter();
            let prepass_block_decoded = block_decoded.clone();
            let row_buffers = ReconRowBufferPool::new(
                splot_parallel::current_pool_width()
                    .saturating_mul(3)
                    .max(1),
            );
            let parse_ready = || {
                let _quantizer_scopes = quantizer.install_frame();
                let step = parser.next_superblock_reusing(&context, row_buffers.take());
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
            };
            let timer = crate::timing::start();
            let tally = crate::timing::WorkerTally::new();
            let prepared = run_ready_row_prepass_with_commit(
                parse_ready,
                |ready| {
                    tally.note_worker();
                    workers.with_scratch(|scratch| {
                        precompute_recon_row(
                            ready,
                            scratch,
                            &prepass_block_decoded,
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
                    })
                },
                |ready| {
                    if let Some(surface) = ready.surface.as_ref() {
                        surface.publish_into(workspace)?;
                    }
                    let buffers = pixel_commit::replay_recon_row(
                        ready.row,
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
                        "units={} committed={} threads={} workers_used={} max_pending={} max_deferred={} max_active={} settled_arm={} {}",
                        prepared.committed,
                        prepared.committed,
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
        } else {
            let row_buffers = ReconRowBufferPool::new(1);
            let parse_row = || {
                let _quantizer_scopes = quantizer.install_frame();
                parser.next_row_reusing(&context, row_buffers.take())
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
        let TileParserOutput {
            cdef_state: tile_cdef_state,
            gdf_state: tile_gdf_state,
            ccso_state: tile_ccso_state,
            active_source_blocks: tile_source_blocks,
            unit_filters: tile_unit_filters,
        } = parser.into_output()?;
        cdef_state.merge_tile(
            &tile_cdef_state,
            tile_mi_rows.clone(),
            tile_mi_cols.clone(),
            tile_offset,
        )?;
        gdf_state.merge_tile(
            &tile_gdf_state,
            tile_mi_rows.clone(),
            tile_mi_cols.clone(),
            tile_offset,
        )?;
        ccso_state.merge_tile(&tile_ccso_state, tile_mi_rows, tile_mi_cols, tile_offset)?;
        append_lr_records(
            &mut frame_filter_records.lr_source_blocks,
            &mut frame_filter_records.lr_unit_filters,
            tile_source_blocks,
            tile_unit_filters,
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

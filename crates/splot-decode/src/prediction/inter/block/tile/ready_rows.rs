// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The ordered ready-row pipeline that drives one tile's parse units.
//!
//! The coordinator is generic over the unit type: it pulls units from one
//! parser at a time, holds each back until a gate admits it, reconstructs the
//! admitted ones in parallel and commits them in parse order. Nothing here
//! knows any AV2 semantics, which is why the fused walk and the deferred
//! resolve pass drive it with different parsers.

use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard};

const READY_JOB_CAPACITY_PER_WORKER: usize = 2;

pub(super) enum ParserStep<Row> {
    More(Row),
    Last(Row),
}

#[derive(Debug)]
pub(super) enum ReadyRowPipelineError<E> {
    Codec(E),
    Capacity,
    Parallel,
}

pub(super) trait OrderedDone {
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
    let mut scanned = 0;
    while scanned < state.deferred.len() && state.ready.len() < state.ready_limit {
        if !state.settled && !state.deferred.get(scanned).is_some_and(gate) {
            scanned += 1;
            continue;
        }
        let Some(row) = state.deferred.remove(scanned) else {
            break;
        };
        state.ready.push_back(row);
        state.max_pending = state.max_pending.max(state.ready.len());
        released = true;
        if state.flip_timer.is_some() {
            crate::timing::report("walk_refs_flip", state.flip_timer.take());
        }
    }
    released
}

/// Takes the next unit the ordered commit frontier is waiting for, so a commit
/// task that has just published one unit keeps the frontier on its own stack
/// instead of paying a task dispatch per unit.
fn take_next_ordered_done<Parser, Ready, Done, Commit, E>(
    state: &mut ReadyRowCoordinator<Parser, Ready, Done, Commit, E>,
) -> Option<Done> {
    if state.capacity_error || state.commit_error.is_some() {
        return None;
    }
    let index = state.next_commit;
    state.done.get_mut(index).and_then(Option::take)
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
    if let Some((done, commit)) = ordered_commit {
        scope.spawn(move |scope| {
            let mut held = Some(commit);
            let mut next = Some(done);
            while let (Some(mut commit), Some(done)) = (held.take(), next.take()) {
                let result = commit(done);
                {
                    let mut state = lock_ready_rows(coordinator);
                    match result {
                        Ok(()) => {
                            state.committed = state.committed.saturating_add(1);
                            state.next_commit = state.next_commit.saturating_add(1);
                        }
                        Err(error) => state.commit_error = Some(error),
                    }
                    next = take_next_ordered_done(&mut state);
                    if next.is_some() {
                        held = Some(commit);
                    } else {
                        state.commit = Some(commit);
                        state.commit_active = false;
                    }
                }
                schedule_ready_rows(scope, coordinator, work, gate);
            }
        });
    }
    if spawn_parser {
        scope.spawn(move |scope| {
            let mut parser = lock_ready_rows(coordinator).parser.take();
            loop {
                let Some(step) = parser.as_mut().map(|parser_state| parser_state()) else {
                    let mut state = lock_ready_rows(coordinator);
                    state.parser_active = false;
                    state.capacity_error = true;
                    return;
                };
                let (row, last) = match step {
                    ParserStep::More(row) => (row, false),
                    ParserStep::Last(row) => (row, true),
                };
                let (overflow, exhausted) = {
                    let mut state = lock_ready_rows(coordinator);
                    if last {
                        crate::timing::report("walk_parse", state.parse_timer.take());
                        state.drain_timer = crate::timing::start();
                    }
                    state.parser_done |= last;
                    let overflow = admit_parsed_row(&mut state, row);
                    let exhausted = last
                        || overflow.is_some()
                        || state.capacity_error
                        || state.commit_error.is_some()
                        || state.ready.len() >= state.ready_limit;
                    if exhausted {
                        state.parser = parser.take();
                        state.parser_active = false;
                    }
                    (overflow, exhausted)
                };
                drop(overflow);
                schedule_ready_rows(scope, coordinator, work, gate);
                if exhausted {
                    return;
                }
            }
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

pub(super) struct ReadyPipelineStats {
    pub(super) committed: usize,
    pub(super) ready_limit: usize,
    pub(super) max_pending: usize,
    pub(super) max_deferred: usize,
    pub(super) max_active: usize,
    /// Whether the drain had to fall back to settling whole reference frames.
    pub(super) settled: bool,
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
pub(super) fn run_ready_row_prepass_with_commit<
    Parser,
    Work,
    Gate,
    Settled,
    Settle,
    Ready,
    Done,
    Commit,
    E,
>(
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

pub(super) fn run_ready_row_pipeline_serial<Parser, Recon, Row, E>(
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

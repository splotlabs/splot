// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Ready-row coordinator tests for [`super`]: capacity bounds, ordered commit,
//! per-row reference admission, and the drain's settle fallback.

#![allow(clippy::expect_used)]

use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex};

use splot_core::symbol::SymbolDecoder;
use splot_core::symbol_encoder::SymbolEncoder;
use splot_parallel::{ThreadCount, WorkerPool};

use super::ready_rows::{ReadyRowPipelineError, run_ready_row_prepass_with_commit};
use super::*;

fn terminal_row(error: crate::DecodeError) -> ReconRow {
    ReconRow {
        ordinal: 0,
        superblocks: Vec::new(),
        entries: Vec::new(),
        residual_blocks: Vec::new(),
        temporal: Vec::new(),
        motion_grids: Vec::new(),
        flag_log: Vec::new(),
        filter_records: TileFilterRecords::default(),
        motion_folded: false,
        motion_derived: false,
        failure: ReconRowFailure::Terminal(error),
    }
}

#[test]
fn tile_symbol_exit_accepts_writer_output_and_reports_eof_as_malformed() {
    let offset = ByteOffset::new(37);
    let payload = SymbolEncoder::new()
        .finish()
        .expect("empty symbol stream must finalize")
        .into_bytes();
    finish_tile_symbols(
        SymbolDecoder::new(&payload).expect("writer output must initialize"),
        offset,
    )
    .expect("writer output must pass exit validation");

    let error = finish_tile_symbols(
        SymbolDecoder::new(&[]).expect("empty payload initializes bounded decoder"),
        offset,
    )
    .expect_err("empty payload must fail exit validation");
    assert!(matches!(
        error,
        crate::DecodeError::MalformedSource { issue }
            if issue.kind() == crate::DecodeSourceIssueKind::TilePayloadParseError
                && issue.spec_section() == Some("8.2.4")
                && issue.offset() == Some(offset)
    ));
}

#[test]
fn terminal_parse_error_prevents_resolve_and_remains_first() {
    let offset = ByteOffset::new(43);
    let terminal = finish_tile_symbols(
        SymbolDecoder::new(&[]).expect("empty payload initializes bounded decoder"),
        offset,
    )
    .expect_err("empty payload must fail exit validation");
    let mut resolved = false;
    let step = resolve_parser_step(ParserStep::Last(terminal_row(terminal)), |_| {
        resolved = true;
        Err(crate::DecodeHeaderStateError::IncompleteInterFrame.into())
    });

    assert!(!resolved);
    assert!(matches!(&step, ParserStep::Last(_)));
    let ParserStep::Last(mut row) = step else {
        return;
    };
    assert!(matches!(
        row.failure.take_terminal(),
        Some(crate::DecodeError::MalformedSource { issue })
            if issue.kind() == crate::DecodeSourceIssueKind::TilePayloadParseError
                && issue.spec_section() == Some("8.2.4")
                && issue.offset() == Some(offset)
    ));
}

#[test]
fn terminal_parse_error_prevents_commit_side_effects() {
    let offset = ByteOffset::new(47);
    let terminal = finish_tile_symbols(
        SymbolDecoder::new(&[]).expect("empty payload initializes bounded decoder"),
        offset,
    )
    .expect_err("empty payload must fail exit validation");
    let mut row = terminal_row(terminal);
    let mut published = false;
    let result: crate::Result<()> = (|| {
        row.return_terminal_error()?;
        published = true;
        Ok(())
    })();

    assert!(!published);
    assert!(matches!(
        result,
        Err(crate::DecodeError::MalformedSource { issue })
            if issue.kind() == crate::DecodeSourceIssueKind::TilePayloadParseError
                && issue.spec_section() == Some("8.2.4")
                && issue.offset() == Some(offset)
    ));
}

#[test]
fn terminal_failure_cannot_be_overwritten_by_precompute_failure() {
    let mut failure = ReconRowFailure::None;
    failure.record_terminal(crate::DecodeHeaderStateError::InvalidInterTileTraversalState.into());
    failure.record_terminal(crate::DecodeHeaderStateError::InvalidInterTileSchedulingState.into());
    failure.record_precompute(
        11,
        crate::DecodeHeaderStateError::InvalidInterTileSchedulingState.into(),
    );

    assert!(matches!(
        failure.take_terminal(),
        Some(crate::DecodeError::HeaderState {
            source: crate::DecodeHeaderStateError::InvalidInterTileTraversalState,
        })
    ));
    assert!(failure.take_precompute().is_none());
}

#[test]
fn terminal_failure_replaces_precompute_failure() {
    let mut failure = ReconRowFailure::None;
    failure.record_precompute(
        13,
        crate::DecodeHeaderStateError::InvalidInterTileSchedulingState.into(),
    );
    failure.record_terminal(crate::DecodeHeaderStateError::InvalidInterTileTraversalState.into());

    assert!(matches!(
        failure.take_terminal(),
        Some(crate::DecodeError::HeaderState {
            source: crate::DecodeHeaderStateError::InvalidInterTileTraversalState,
        })
    ));
    assert!(failure.take_precompute().is_none());
}

#[test]
fn tile_parser_walk_reuse_and_second_finish_are_typed_errors() {
    let mut walk = TileParserWalk::Active(23);
    assert_eq!(*walk.active_mut().expect("walk starts active"), 23);
    assert_eq!(walk.finish().expect("active walk finishes once"), 23);

    for error in [
        walk.active_mut()
            .expect_err("finished walk cannot be reused"),
        walk.finish()
            .expect_err("finished walk cannot finish twice"),
    ] {
        assert!(matches!(
            error,
            crate::DecodeError::HeaderState {
                source: crate::DecodeHeaderStateError::InvalidInterTileTraversalState,
            }
        ));
    }
}

#[test]
fn no_decoded_block_error_is_typed_and_has_no_diagnostic() {
    let error = no_decoded_block_error();
    assert!(matches!(
        &error,
        crate::DecodeError::HeaderState {
            source: crate::DecodeHeaderStateError::InvalidInterTileTraversalState,
        }
    ));
    assert!(crate::DecodeDiagnosticReport::from_decode_error(&error).is_none());
}

#[test]
fn ready_row_pipeline_calls_a_terminal_parser_exactly_once() {
    let parsed = Arc::new(AtomicUsize::new(0));
    let parsed_by_callback = Arc::clone(&parsed);
    let committed = Arc::new(AtomicUsize::new(0));
    let committed_by_callback = Arc::clone(&committed);
    let pool = WorkerPool::new(ThreadCount::Fixed(
        NonZeroUsize::new(2).expect("two workers"),
    ))
    .expect("worker pool");

    pool.install(|| {
        run_ready_row_prepass_with_commit(
            move || {
                parsed_by_callback.fetch_add(1, Ordering::SeqCst);
                ParserStep::Last(0usize)
            },
            |row| row,
            move |_| {
                committed_by_callback.fetch_add(1, Ordering::SeqCst);
                Ok::<_, ()>(())
            },
            1,
            |_: &usize| true,
            || true,
            || Ok(()),
        )
    })
    .expect("terminal row commits");

    assert_eq!(parsed.load(Ordering::SeqCst), 1);
    assert_eq!(committed.load(Ordering::SeqCst), 1);
}

#[test]
fn recon_row_layout_stays_compact() {
    assert_eq!(core::mem::size_of::<ReconRow>(), 336);
    assert_eq!(core::mem::align_of::<ReconRow>(), 8);
    assert_eq!(core::mem::size_of::<ReconRowEntry>(), 424);
    assert_eq!(core::mem::align_of::<ReconRowEntry>(), 8);
}

#[test]
fn recon_entries_are_bucketed_by_contiguous_superblock_without_reordering() {
    let mut superblocks = Vec::new();
    let mut entries = Vec::new();
    push_recon_entry(
        &mut superblocks,
        &mut entries,
        [0, 0],
        ReconDependency::ReferenceOnly,
        0,
    );
    push_recon_entry(
        &mut superblocks,
        &mut entries,
        [0, 0],
        ReconDependency::CurrentFrame,
        1,
    );
    push_recon_entry(
        &mut superblocks,
        &mut entries,
        [0, 16],
        ReconDependency::ReferenceOnly,
        2,
    );
    push_recon_entry(
        &mut superblocks,
        &mut entries,
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
            .map(|superblock| superblock.entries.clone())
            .collect::<Vec<_>>(),
        [0..2, 2..3, 3..4]
    );
    assert_eq!(
        superblocks
            .iter()
            .flat_map(|superblock| entries[superblock.entries.clone()].iter().copied())
            .collect::<Vec<_>>(),
        [0, 1, 2, 3]
    );
}

#[test]
fn recon_superblock_retains_the_strongest_dependency() {
    let mut superblocks = Vec::new();
    let mut entries = Vec::new();
    for dependency in [
        ReconDependency::ReferenceOnly,
        ReconDependency::GlobalIntrabcFence,
        ReconDependency::CurrentFrame,
    ] {
        push_recon_entry(&mut superblocks, &mut entries, [0, 0], dependency, ());
    }

    assert_eq!(superblocks.len(), 1);
    assert_eq!(
        superblocks[0].dependency,
        ReconDependency::GlobalIntrabcFence
    );
}

#[test]
fn recon_row_buffer_pool_reuses_row_arena_storage() {
    let pool = ReconRowBufferPool::new(0);
    let mut buffers = ReconRowBuffers::default();
    buffers.temporal.reserve(8);
    let pointer = buffers.temporal.as_ptr();
    pool.recycle(buffers);

    let reused = pool.take();
    assert_eq!(reused.temporal.capacity(), 8);
    assert!(core::ptr::eq(reused.temporal.as_ptr(), pointer));
}

#[test]
fn inter_recon_scratch_pool_reuses_worker_context() {
    let mut pool = InterReconScratchPool::<u8>::default();
    pool.ensure_workers(1);
    let first = pool.with_scratch(core::ptr::from_mut);
    let second = pool.with_scratch(core::ptr::from_mut);

    assert_eq!(first, second);
}

#[test]
fn scheduled_tile_context_moves_the_bounded_worker_pool_for_reuse() {
    let mut pool = InterReconScratchPool::<u8>::default();
    pool.ensure_workers(3);

    let reused = TileDecodeScratch::from_scheduled(
        deferred_recon::InterReconScratch::default(),
        &pool,
        Vec::new(),
    );

    assert_eq!(pool.available_len(), 0);
    assert_eq!(reused.workers.available_len(), 3);
}

#[test]
fn mixed_superblock_prepass_selects_every_independent_entry() {
    assert!(select_prepass_entry(ReconDependency::ReferenceOnly, true));
    assert!(!select_prepass_entry(ReconDependency::CurrentFrame, true));
    assert!(!select_prepass_entry(ReconDependency::ReferenceOnly, false));
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
    let committed = Arc::new(Mutex::new(Vec::new()));
    let committed_for_frontier = Arc::clone(&committed);

    let prepared = pool
        .install(|| {
            run_ready_row_prepass_with_commit(
                parser,
                work,
                move |row| {
                    committed_for_frontier.lock().expect("commit log").push(row);
                    Ok::<_, ()>(())
                },
                6,
                |_: &usize| true,
                || true,
                || Ok(()),
            )
        })
        .expect("row pipeline");

    assert!(prepared.max_pending <= prepared.ready_limit);
    assert_eq!(prepared.max_active, 3);
    assert_eq!(*committed.lock().expect("commit log"), [0, 1, 2, 3, 4, 5]);
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

    let result = pool.install(|| {
        run_ready_row_prepass_with_commit(
            parser,
            |row| row,
            |_| Ok::<_, crate::DecodeError>(()),
            1,
            |_: &usize| true,
            || true,
            || Ok(()),
        )
    });

    let error = result
        .err()
        .expect("completion-slot overflow must fail closed");
    let error = inter_ready_row_pipeline_error(error);
    assert!(matches!(
        &error,
        crate::DecodeError::HeaderState {
            source: crate::DecodeHeaderStateError::InvalidInterTileSchedulingState,
        }
    ));
    assert!(crate::DecodeDiagnosticReport::from_decode_error(&error).is_none());
}

#[test]
fn ready_row_reservation_failure_stays_an_allocation_error() {
    let result = run_ready_row_prepass_with_commit(
        || ParserStep::Last(0usize),
        |row| row,
        |_| Ok::<_, crate::DecodeError>(()),
        usize::MAX,
        |_: &usize| true,
        || true,
        || Ok(()),
    );
    let error = result
        .err()
        .expect("an impossible completion-slot reservation must fail");
    assert!(matches!(&error, ReadyRowPipelineError::Allocation));

    let error = inter_ready_row_pipeline_error(error);
    assert!(matches!(
        &error,
        crate::DecodeError::Reconstruction {
            source: splot_recon::ReconError::WorkspaceAllocationFailed {
                plane: PlaneId::Y,
                context: "inter ready-row completion slots",
            },
        }
    ));
    assert!(crate::DecodeDiagnosticReport::from_decode_error(&error).is_none());
}

#[test]
fn ready_row_scope_preserves_off_pool_error() {
    let result = run_ready_row_prepass_with_commit(
        || ParserStep::Last(0usize),
        |row| row,
        |_| Ok::<_, crate::DecodeError>(()),
        1,
        |_: &usize| true,
        || true,
        || Ok(()),
    );
    let error = result.err().expect("an off-pool ready scope must fail");
    assert!(matches!(
        &error,
        ReadyRowPipelineError::Parallel(splot_parallel::ParallelError::NotOnWorkerPool)
    ));

    let error = inter_ready_row_pipeline_error(error);
    assert!(matches!(
        &error,
        crate::DecodeError::Pool {
            source: splot_parallel::ParallelError::NotOnWorkerPool,
        }
    ));
    assert!(crate::DecodeDiagnosticReport::from_decode_error(&error).is_none());
}

#[test]
fn ordered_commit_frontier_publishes_every_job_canonically() {
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
    let committed = Arc::new(Mutex::new(Vec::new()));
    let committed_for_frontier = Arc::clone(&committed);
    let pool = WorkerPool::new(ThreadCount::Fixed(
        NonZeroUsize::new(4).expect("four workers"),
    ))
    .expect("worker pool");

    pool.install(|| {
        run_ready_row_prepass_with_commit(
            parser,
            |row| row,
            move |row| {
                committed_for_frontier.lock().expect("commit log").push(row);
                Ok::<_, ()>(())
            },
            6,
            |_: &usize| true,
            || true,
            || Ok(()),
        )
    })
    .expect("ordered pipeline");

    assert_eq!(*committed.lock().expect("commit log"), [0, 1, 2, 3, 4, 5]);
}

#[test]
fn a_shut_reference_gate_defers_rows_and_commits_them_in_parse_order() {
    let parsed = Arc::new(AtomicUsize::new(0));
    let parsed_for_parser = Arc::clone(&parsed);
    let parser = move || {
        let row = parsed_for_parser.fetch_add(1, Ordering::SeqCst);
        if row == 5 {
            ParserStep::Last(row)
        } else {
            ParserStep::More(row)
        }
    };
    let committed = Arc::new(Mutex::new(Vec::new()));
    let committed_for_frontier = Arc::clone(&committed);
    let pool = WorkerPool::new(ThreadCount::Fixed(
        NonZeroUsize::new(4).expect("four workers"),
    ))
    .expect("worker pool");

    pool.install(|| {
        run_ready_row_prepass_with_commit(
            parser,
            |row| row,
            move |row| {
                committed_for_frontier.lock().expect("commit log").push(row);
                Ok::<_, ()>(())
            },
            6,
            |_: &usize| parsed.load(Ordering::SeqCst) >= 3,
            || false,
            || Ok(()),
        )
    })
    .expect("gated pipeline");

    assert_eq!(*committed.lock().expect("commit log"), [0, 1, 2, 3, 4, 5]);
}

#[test]
fn a_row_waiting_for_its_references_does_not_hold_back_the_rows_behind_it() {
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
    let committed = Arc::new(Mutex::new(Vec::new()));
    let committed_for_frontier = Arc::clone(&committed);
    let admitted = Arc::new(AtomicUsize::new(0));
    let admitted_for_gate = Arc::clone(&admitted);
    let admitted_for_settle = Arc::clone(&admitted);
    let pool = WorkerPool::new(ThreadCount::Fixed(
        NonZeroUsize::new(4).expect("four workers"),
    ))
    .expect("worker pool");

    pool.install(|| {
        run_ready_row_prepass_with_commit(
            parser,
            |row| row,
            move |row: usize| {
                committed_for_frontier.lock().expect("commit log").push(row);
                Ok::<_, ()>(())
            },
            6,
            move |row: &usize| *row != 2 || admitted_for_gate.load(Ordering::SeqCst) == 1,
            || true,
            move || {
                admitted_for_settle.store(1, Ordering::SeqCst);
                Ok(())
            },
        )
    })
    .expect("out-of-order pipeline");

    assert_eq!(admitted.load(Ordering::SeqCst), 1);
    assert_eq!(*committed.lock().expect("commit log"), [0, 1, 2, 3, 4, 5]);
}

#[test]
fn a_gate_that_outlives_parsing_settles_once_and_then_drains() {
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
    let committed = Arc::new(Mutex::new(Vec::new()));
    let committed_for_frontier = Arc::clone(&committed);
    let settled = Arc::new(AtomicUsize::new(0));
    let settled_for_wait = Arc::clone(&settled);
    let pool = WorkerPool::new(ThreadCount::Fixed(
        NonZeroUsize::new(4).expect("four workers"),
    ))
    .expect("worker pool");

    pool.install(|| {
        run_ready_row_prepass_with_commit(
            parser,
            |row| row,
            move |row| {
                committed_for_frontier.lock().expect("commit log").push(row);
                Ok::<_, ()>(())
            },
            6,
            |_: &usize| false,
            || true,
            move || {
                settled_for_wait.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
        )
    })
    .expect("settled pipeline");

    assert_eq!(settled.load(Ordering::SeqCst), 1);
    assert_eq!(*committed.lock().expect("commit log"), [0, 1, 2, 3, 4, 5]);
}

#[test]
fn a_failed_reference_settle_surfaces_the_codec_diagnostic() {
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
        NonZeroUsize::new(4).expect("four workers"),
    ))
    .expect("worker pool");

    let result = pool.install(|| {
        run_ready_row_prepass_with_commit(
            parser,
            |row| row,
            |_| Ok::<_, &str>(()),
            2,
            |_: &usize| false,
            || true,
            || Err("reference filter phase failed"),
        )
    });

    assert!(matches!(
        result,
        Err(ReadyRowPipelineError::Codec(
            "reference filter phase failed"
        ))
    ));
}

const ORDERHINT_WRAP_FIXTURE: &[u8] = include_bytes!(
    "../../../../../../tests/conformance/vectors/valid/syn-orderhint-wrap-64x64.ivf"
);

#[test]
fn orderhint_wrap_fixture_decodes_identically_across_thread_counts() {
    let decode_hashes = |threads: usize| -> Vec<String> {
        let options = crate::DecodeOptions::default();
        let context =
            crate::DecodeContext::new(crate::DecodeRuntimeConfig::new(ThreadCount::from(threads)))
                .expect("context");
        let plan = context
            .plan_bytes(ORDERHINT_WRAP_FIXTURE, options)
            .expect("plan");
        let frames = context
            .pool()
            .install(|| {
                crate::pipeline::decode_frames_from_plan(ORDERHINT_WRAP_FIXTURE, &options, &plan)
            })
            .expect("decode");
        frames
            .iter()
            .map(|output| match output.ready_frame().expect("ready") {
                crate::pipeline::PipelineDecodedFrame::Eight(frame) => {
                    splot_recon::DecodedFrameHashInput::new(&frame)
                        .compute_hash()
                        .to_hex()
                }
                crate::pipeline::PipelineDecodedFrame::Ten(frame) => {
                    splot_recon::DecodedFrameHashInput::new(&frame)
                        .compute_hash()
                        .to_hex()
                }
            })
            .collect()
    };
    let single = decode_hashes(1);
    assert_eq!(single.len(), 121, "fixture decodes 121 output frames");
    assert_eq!(
        single,
        decode_hashes(8),
        "entries skipped by the superblock prepass must keep later \
         overlapping writes in walk order"
    );
}

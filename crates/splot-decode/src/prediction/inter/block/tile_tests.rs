// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Tile parse, reconstruction-state, and production-path determinism tests.

#![allow(clippy::expect_used)]

use splot_core::span::ByteOffset;
use splot_core::symbol::SymbolDecoder;
use splot_core::symbol_encoder::SymbolEncoder;
use splot_parallel::ThreadCount;

use super::*;

fn malformed_tile_error(offset: ByteOffset) -> crate::DecodeError {
    finish_tile_symbols(
        SymbolDecoder::new(&[]).expect("empty payload initializes bounded decoder"),
        offset,
    )
    .expect_err("empty payload must fail exit validation")
}

fn assert_tile_payload_error(error: &crate::DecodeError, offset: ByteOffset) {
    assert!(matches!(
        error,
        crate::DecodeError::MalformedSource { issue }
            if issue.kind() == crate::DecodeSourceIssueKind::TilePayloadParseError
                && issue.spec_section() == Some("8.2.4")
                && issue.offset() == Some(offset)
    ));
}

fn assert_invalid_tile_traversal(error: &crate::DecodeError) {
    assert!(matches!(
        error,
        crate::DecodeError::HeaderState {
            source: crate::DecodeHeaderStateError::InvalidInterTileTraversalState,
        }
    ));
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
    assert_tile_payload_error(&malformed_tile_error(offset), offset);
}

#[test]
fn terminal_parse_error_prevents_resolve_and_commit_side_effects() {
    let offset = ByteOffset::new(43);
    let row = ReconRow {
        residual_source: None,
        ordinal: 0,
        residual_coeffs: Vec::new(),
        superblocks: Vec::new(),
        entries: Vec::new(),
        residual_blocks: Vec::new(),
        temporal: Vec::new(),
        motion_grids: Vec::new(),
        motion_storage: None,
        flag_log: Vec::new(),
        filter_records: TileFilterRecords::default(),
        residual_planes: crate::residual::pipeline::ResidualPlaneArena::new(),
        motion_folded: false,
        motion_derived: false,
        failure: ReconRowFailure::Terminal(malformed_tile_error(offset)),
    };
    let mut resolved = false;
    let step = resolve_parser_step(ParserStep::Last(row), |_| {
        resolved = true;
        Err(crate::DecodeHeaderStateError::IncompleteInterFrame.into())
    });
    assert!(!resolved);
    assert!(matches!(&step, ParserStep::Last(_)));
    let ParserStep::Last(mut row) = step else {
        return;
    };
    let mut published = false;
    let result = row.return_terminal_error().map(|()| published = true);
    assert!(!published);
    assert_tile_payload_error(&result.expect_err("terminal error returned"), offset);
}

fn assert_terminal_failure_wins(precompute_first: bool) {
    let mut failure = ReconRowFailure::default();
    if precompute_first {
        failure.record_precompute(
            13,
            crate::DecodeHeaderStateError::InvalidInterTileSchedulingState.into(),
        );
    }
    failure.record_terminal(crate::DecodeHeaderStateError::InvalidInterTileTraversalState.into());
    failure.record_terminal(crate::DecodeHeaderStateError::InvalidInterTileSchedulingState.into());
    if !precompute_first {
        failure.record_precompute(
            11,
            crate::DecodeHeaderStateError::InvalidInterTileSchedulingState.into(),
        );
    }
    let terminal = failure.take_terminal().expect("terminal failure retained");
    assert_invalid_tile_traversal(&terminal);
    assert!(failure.take_precompute().is_none());
}

#[test]
fn terminal_failure_cannot_be_overwritten_by_precompute_failure() {
    assert_terminal_failure_wins(false);
}

#[test]
fn terminal_failure_replaces_precompute_failure() {
    assert_terminal_failure_wins(true);
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
        assert_invalid_tile_traversal(&error);
    }
}

#[test]
fn no_decoded_block_error_is_typed_and_has_no_diagnostic() {
    let error = no_decoded_block_error();
    assert_invalid_tile_traversal(&error);
    assert!(crate::DecodeDiagnosticReport::from_decode_error(&error).is_none());
}

#[test]
fn recon_entries_keep_contiguous_superblock_order() {
    let mut superblocks = Vec::new();
    let mut entries = Vec::new();
    for (origin, entry) in [([0, 0], 0), ([0, 0], 1), ([0, 16], 2), ([0, 0], 3)] {
        push_recon_entry(&mut superblocks, &mut entries, origin, entry);
    }

    assert_eq!(
        superblocks
            .iter()
            .map(|superblock| (superblock.origin, superblock.entries.clone()))
            .collect::<Vec<_>>(),
        [([0, 0], 0..2), ([0, 16], 2..3), ([0, 0], 3..4)]
    );
}

#[test]
fn reconstruction_pools_reuse_owned_storage() {
    let rows = {
        let mut pool = ReconRowBufferPool::default();
        pool.reset(0);
        pool
    };
    let mut buffers = ReconRowBuffers::default();
    buffers.temporal.reserve(8);
    let pointer = buffers.temporal.as_ptr();
    rows.recycle(buffers);
    let reused = rows.take();
    assert_eq!(reused.temporal.capacity(), 8);
    assert!(core::ptr::eq(reused.temporal.as_ptr(), pointer));

    let workers = InterReconScratchPool::<u8>::default();
    workers.ensure_workers(1);
    let first = workers
        .with_scratch(core::ptr::from_mut)
        .expect("available scratch");
    let second = workers
        .with_scratch(core::ptr::from_mut)
        .expect("available scratch");
    assert_eq!(first, second);
}

const ORDERHINT_WRAP_FIXTURE: &[u8] = include_bytes!(
    "../../../../../../tests/conformance/vectors/valid/syn-orderhint-wrap-64x64.ivf"
);
const LARGE_INTER_FIXTURE: &[u8] = include_bytes!(
    "../../../../../../tests/conformance/vectors/valid/syn-2frame-lr-switchable-768x256-8bit.ivf"
);
const TWO_TILE_INTER_FIXTURE: &[u8] = include_bytes!(
    "../../../../../../tests/conformance/vectors/valid/syn-2tile-inter-128x64-q80.ivf"
);

fn decode_hashes(bytes: &[u8], threads: usize) -> Vec<String> {
    let options = crate::DecodeOptions::default();
    let context =
        crate::DecodeContext::new(crate::DecodeRuntimeConfig::new(ThreadCount::from(threads)))
            .expect("context");
    let plan = context.plan_bytes(bytes, options).expect("plan");
    context
        .pool()
        .install(|| crate::pipeline::decode_frames_from_plan(bytes, &options, &plan))
        .expect("decode")
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
}

#[test]
fn orderhint_wrap_fixture_decodes_identically_across_thread_counts() {
    let single = decode_hashes(ORDERHINT_WRAP_FIXTURE, 1);
    assert_eq!(single.len(), 121, "fixture decodes 121 output frames");
    for threads in [4, 8, 10] {
        assert_eq!(
            single,
            decode_hashes(ORDERHINT_WRAP_FIXTURE, threads),
            "mismatch at {threads} threads"
        );
    }
}

#[test]
fn bounded_admission_fixture_decodes_identically_across_thread_counts() {
    let single = decode_hashes(LARGE_INTER_FIXTURE, 1);
    assert_eq!(single.len(), 2, "fixture decodes two output frames");
    for threads in [2, 4] {
        assert_eq!(
            single,
            decode_hashes(LARGE_INTER_FIXTURE, threads),
            "mismatch at {threads} threads"
        );
    }
}

#[test]
fn two_tile_fixture_decodes_identically_across_worker_widths() {
    let single = decode_hashes(TWO_TILE_INTER_FIXTURE, 1);
    assert_eq!(single.len(), 2, "fixture decodes two output frames");
    for threads in [2, 3, 4, 8, 10] {
        assert_eq!(
            single,
            decode_hashes(TWO_TILE_INTER_FIXTURE, threads),
            "mismatch at {threads} threads"
        );
    }
}

fn corrupted_tile_payload_error(bytes: &[u8], threads: usize) -> crate::DecodeError {
    let mut corrupted = bytes.to_vec();
    let last = corrupted.len() - 1;
    corrupted[last] ^= u8::MAX;
    let options = crate::DecodeOptions::default();
    let context =
        crate::DecodeContext::new(crate::DecodeRuntimeConfig::new(ThreadCount::from(threads)))
            .expect("context");
    let plan = context
        .plan_bytes(&corrupted, options)
        .expect("length-preserving tile-payload corruption remains planner-valid");
    context
        .pool()
        .install(|| crate::pipeline::decode_frames_from_plan(&corrupted, &options, &plan))
        .err()
        .expect("planner-valid tile payload corruption must fail during decode")
}

#[test]
fn corrupted_multi_tile_payload_has_the_same_error_across_worker_widths() {
    let expected = corrupted_tile_payload_error(TWO_TILE_INTER_FIXTURE, 1);
    for threads in [2, 3, 4, 8, 10] {
        let actual = corrupted_tile_payload_error(TWO_TILE_INTER_FIXTURE, threads);
        assert_eq!(format!("{actual:?}"), format!("{expected:?}"));
        assert_eq!(actual.to_string(), expected.to_string());
    }
}

#[test]
fn frame_slots_retain_their_row_payloads_across_reset() {
    use crate::support::decode_buffers::DecodeBuffers;

    let mut decoding = super::ParseProgress::default();
    let other_decode = super::ParseProgress::default();
    let mut retained = Vec::new();
    for capacity in [4096, 8192] {
        let mut spent = super::ReconRowBuffers::default();
        spent.residual_coeffs.reserve(capacity);
        retained.push(spent.residual_coeffs.as_ptr());
        decoding.row_buffers.get_mut().push(Some(spent));
    }
    for _ in 0..128 {
        decoding.reset(&DecodeBuffers::new());
        assert!(other_decode.take_row_buffers(0).is_err());
        assert!(decoding.take_row_buffers(2).is_err());
        assert!(
            decoding
                .return_row_buffers(0, super::ReconRowBuffers::default())
                .is_err()
        );
        for index in [1, 0] {
            let payload = decoding.take_row_buffers(index).expect("retained row slot");
            assert_eq!(payload.residual_coeffs.as_ptr(), retained[index]);
            assert!(decoding.take_row_buffers(index).is_err());
            decoding
                .return_row_buffers(index, payload)
                .expect("vacant row slot");
        }
    }
}

#[test]
fn frame_geometry_waits_for_readers_then_reuses_hidden_backing() {
    use crate::support::decode_buffers::DecodeBuffers;

    let buffers = DecodeBuffers::new();
    let mut progress = super::ParseProgress::default();
    assert!(progress.reset(&buffers));
    progress
        .publish_geometry(
            super::tile::TileGeometry {
                tile_offset: ByteOffset::new(3),
                mi_rows: 0..2,
                mi_cols: 0..3,
                unit_count: 2,
            },
            0,
        )
        .expect("first geometry publication");
    let held = progress.geometry().expect("published geometry");
    let identity = Arc::as_ptr(&held);
    assert!(!progress.reset(&buffers));
    assert_eq!(
        progress
            .geometry()
            .expect("held geometry remains visible")
            .tile_offset,
        ByteOffset::new(3)
    );
    drop(held);

    assert!(progress.reset(&buffers));
    assert!(progress.geometry().is_none());
    progress
        .publish_geometry(
            super::tile::TileGeometry {
                tile_offset: ByteOffset::new(9),
                mi_rows: 1..5,
                mi_cols: 2..7,
                unit_count: 4,
            },
            0,
        )
        .expect("reused geometry publication");
    let reused = progress.geometry().expect("republished geometry");
    assert_eq!(Arc::as_ptr(&reused), identity);
    assert_eq!(reused.tile_offset, ByteOffset::new(9));
    assert_eq!(reused.unit_count, 4);
}

#[test]
fn a_tile_row_pool_without_a_decode_still_hands_out_buffers() {
    let mut pool = super::ReconRowBufferPool::default();
    pool.reset(2);
    assert_eq!(pool.take().residual_coeffs.capacity(), 0);
}

#[cfg(test)]
#[test]
fn reconstruction_scratch_is_bounded_while_all_workers_hold_it() {
    let workers = InterReconScratchPool::<u16>::default();
    workers.ensure_workers(12);
    let barrier = std::sync::Barrier::new(13);
    std::thread::scope(|scope| {
        let mut joins = Vec::new();
        for _ in 0..12 {
            let workers = &workers;
            let barrier = &barrier;
            joins.push(scope.spawn(move || {
                workers.with_scratch(|_| {
                    barrier.wait();
                    barrier.wait();
                })
            }));
        }
        barrier.wait();
        workers.ensure_workers(12);
        let allocated = workers.available.lock().0;
        let exhausted = workers.with_scratch(|_| ()).is_err();
        barrier.wait();
        for join in joins {
            join.join().expect("worker").expect("scratch");
        }
        assert_eq!(allocated, 12);
        assert!(exhausted);
    });
    let pool = workers.available.lock();
    assert_eq!(pool.0, 12);
    assert_eq!(pool.1.len(), 12);
}

#[test]
fn superblock_coefficients_reserve_plane_coverage_once() {
    for (chroma, samples) in [
        (ChromaFormatIdc::Monochrome, 4096),
        (ChromaFormatIdc::Yuv420, 6144),
        (ChromaFormatIdc::Yuv422, 8192),
        (ChromaFormatIdc::Yuv444, 12288),
    ] {
        let capacity = superblock_coefficient_capacity(16, chroma).expect("coefficient bound");
        assert_eq!(capacity, samples);
        let mut row = ReconRowBuffers::default();
        row.reserve_coefficients(capacity)
            .expect("reserve coefficients");
        let storage = row.residual_coeffs.as_ptr();
        for cycle in 0..1200 {
            row.residual_coeffs.clear();
            row.reserve_coefficients(capacity)
                .expect("reuse coefficients");
            row.residual_coeffs
                .resize(if cycle % 2 == 0 { samples } else { 16 }, 1);
            assert_eq!(row.residual_coeffs.as_ptr(), storage);
        }
    }
    assert!(superblock_coefficient_capacity(usize::MAX, ChromaFormatIdc::Yuv444).is_err());
}

#[cfg(test)]
#[test]
fn frame_coefficient_snapshots_release_the_lock_and_pin_retirement() {
    let buffers = crate::support::decode_buffers::DecodeBuffers::default();
    let mut progress = ParseProgress::default();
    progress.residuals.lock().coefficients.reserve_exact(64);
    let storage = progress.residuals.lock().coefficients.as_ptr();
    let mut snapshot = Vec::new();
    snapshot.reserve_exact(16);
    let snapshot_storage = snapshot.as_ptr();
    for cycle in 0..1200 {
        assert!(progress.reset(&buffers));
        progress
            .residuals
            .lock()
            .coefficients
            .extend_from_slice(&[1, cycle, 3, 4]);
        let source = RowResiduals {
            frame: Arc::clone(&progress.residuals),
            planes: crate::residual::pipeline::ResidualPlaneSpan::default(),
            range: 1..4,
            capacity: 16,
        };
        source.copy_into(&mut snapshot).expect("snapshot");
        assert_eq!(snapshot, [cycle, 3, 4]);
        assert_eq!(snapshot.as_ptr(), snapshot_storage);
        assert!(!progress.reset(&buffers));
        source
            .copy_into(&mut snapshot)
            .expect("retained publication");
        assert_eq!(snapshot, [cycle, 3, 4]);
        assert!(progress.residuals.try_lock().is_some());
        std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    progress
                        .residuals
                        .lock()
                        .coefficients
                        .extend_from_slice(&[9; 16]);
                })
                .join()
                .expect("concurrent append");
        });
        assert_eq!(snapshot, [cycle, 3, 4]);
        assert_eq!(progress.residuals.lock().coefficients.as_ptr(), storage);
        drop(source);
    }
    let source = RowResiduals {
        frame: Arc::clone(&progress.residuals),
        planes: crate::residual::pipeline::ResidualPlaneSpan::default(),
        range: 0..65,
        capacity: 64,
    };
    assert!(source.copy_into(&mut snapshot).is_err());
    let source = RowResiduals {
        range: 0..4,
        capacity: 3,
        ..source
    };
    assert!(source.copy_into(&mut snapshot).is_err());
    drop(source);
    assert!(progress.reset(&buffers));
}

#[test]
fn frame_filter_publication_returns_producer_capacity_before_row_replay() {
    let mut progress = ParseProgress::default();
    let buffers = crate::support::decode_buffers::DecodeBuffers::default();
    let mut records = TileFilterRecords::default();
    records.deblock_blocks.reserve_exact(32);
    records.chroma_deblock_blocks.reserve_records(32);
    records.tx_skip_records.reserve_exact(32);
    let storage = records.tx_skip_records.as_ptr();
    for cycle in 0..1200 {
        assert!(progress.reset(&buffers));
        records.tx_skip_records.push(
            crate::filters::wienerns_lr::WienerNsLrTxSkipTransformRecord {
                row: cycle,
                col: 0,
                rows: 1,
                cols: 1,
                skip_flag: false,
                eob: 1,
            },
        );
        let row = ReconRow {
            residual_source: None,
            ordinal: cycle,
            residual_coeffs: Vec::new(),
            superblocks: Vec::new(),
            entries: Vec::new(),
            residual_blocks: Vec::new(),
            temporal: Vec::new(),
            motion_grids: Vec::new(),
            motion_storage: None,
            flag_log: Vec::new(),
            filter_records: records,
            residual_planes: crate::residual::pipeline::ResidualPlaneArena::new(),
            motion_folded: false,
            motion_derived: false,
            failure: ReconRowFailure::None,
        };
        records = progress.publish_row(row);
        assert!(records.tx_skip_records.is_empty());
        assert_eq!(records.tx_skip_records.as_ptr(), storage);
        assert!(records.deblock_blocks.capacity() >= 32);
        assert!(records.chroma_deblock_blocks.capacity() >= 32);
        let delayed_row = progress.take_row(0).expect("published row");
        assert_eq!(delayed_row.filter_records.deblock_blocks.capacity(), 0);
        assert_eq!(
            delayed_row.filter_records.chroma_deblock_blocks.capacity(),
            0
        );
        assert_eq!(delayed_row.filter_records.tx_skip_records.capacity(), 0);
        let frame = progress.records.lock();
        assert_eq!(frame.tx_skip_records.len(), 1);
        assert_eq!(frame.tx_skip_records[0].row, cycle);
    }
}

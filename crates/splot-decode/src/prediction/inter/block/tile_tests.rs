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
        ordinal: 0,
        residual_coeffs: Vec::new(),
        superblocks: Vec::new(),
        entries: Vec::new(),
        residual_blocks: Vec::new(),
        temporal: Vec::new(),
        motion_grids: Vec::new(),
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
    let rows = ReconRowBufferPool::new(0);
    let mut buffers = ReconRowBuffers::default();
    buffers.temporal.reserve(8);
    let pointer = buffers.temporal.as_ptr();
    rows.recycle(buffers);
    let reused = rows.take();
    assert_eq!(reused.temporal.capacity(), 8);
    assert!(core::ptr::eq(reused.temporal.as_ptr(), pointer));

    let mut workers = InterReconScratchPool::<u8>::default();
    workers.ensure_workers(1);
    let first = workers.with_scratch(core::ptr::from_mut);
    let second = workers.with_scratch(core::ptr::from_mut);
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

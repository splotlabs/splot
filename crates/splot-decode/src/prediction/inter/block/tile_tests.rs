// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Tile parse, reconstruction-state, and production-path determinism tests.

#![allow(clippy::expect_used)]

use splot_core::symbol::SymbolDecoder;
use splot_core::symbol_encoder::SymbolEncoder;
use splot_parallel::ThreadCount;

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
fn terminal_failure_precedes_precompute_failure() {
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
        context
            .pool()
            .install(|| {
                crate::pipeline::decode_frames_from_plan(ORDERHINT_WRAP_FIXTURE, &options, &plan)
            })
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
    };
    let single = decode_hashes(1);
    assert_eq!(single.len(), 121, "fixture decodes 121 output frames");
    for threads in [4, 8, 10] {
        assert_eq!(
            single,
            decode_hashes(threads),
            "mismatch at {threads} threads"
        );
    }
}

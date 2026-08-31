// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Tile parse, reconstruction-state, and production-path determinism tests.

#![allow(clippy::expect_used)]

use splot_parallel::ThreadCount;

use super::*;

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

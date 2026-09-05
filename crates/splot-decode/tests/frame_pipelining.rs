// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Frame-pipeline equivalence tests.
//!
//! Every worker width and frame-delay capacity uses the same admission-scheduled
//! parse, reconstruction, and filter path, so all configurations must produce
//! identical bytes and diagnostics.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use core::num::NonZeroU64;
use parking_lot::{Condvar, Mutex};
use std::sync::Arc;
use std::time::{Duration, Instant};

use splot_decode::{
    DecodeContext, DecodeDiagnosticReport, DecodeError, DecodeLimitThreshold, DecodeLimits,
    DecodeOptions, DecodeRuntimeConfig,
};
use splot_parallel::{FrameDelay, ThreadCount};

const MULTIREF: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-3frame-multiref-64x64.ivf");
const EIGHT_FRAME: &[u8] = include_bytes!(
    "../../../tests/conformance/vectors/valid/syn-8frame-opfl-refine-all-64x64-q120.ivf"
);
const SEF_FAMILIES: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-frame-sef-families-64x64.ivf");
const TIP_FAMILIES: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-frame-tip-families-64x64.ivf");
const BRIDGE: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-bridge-celu-64x64.ivf");
const TEN_BIT_INTER: &[u8] = include_bytes!(
    "../../../tests/conformance/vectors/valid/syn-3frame-deblock-subpu-chroma-64x32-10bit-q90.ivf"
);
const ORDER_HINT_WRAP: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-orderhint-wrap-64x64.ivf");

const FIXTURES: &[(&str, &[u8])] = &[
    ("syn-3frame-multiref-64x64", MULTIREF),
    ("syn-8frame-opfl-refine-all-64x64-q120", EIGHT_FRAME),
    ("syn-frame-sef-families-64x64", SEF_FAMILIES),
    ("syn-frame-tip-families-64x64", TIP_FAMILIES),
    ("syn-bridge-celu-64x64", BRIDGE),
    (
        "syn-3frame-deblock-subpu-chroma-64x32-10bit-q90",
        TEN_BIT_INTER,
    ),
];

fn context(threads: usize, frame_delay: FrameDelay) -> DecodeContext {
    DecodeContext::new(
        DecodeRuntimeConfig::new(ThreadCount::from(threads)).with_frame_delay(frame_delay),
    )
    .unwrap()
}

fn depth_one() -> DecodeContext {
    context(1, FrameDelay::from(1usize))
}

fn collect_raw(
    context: &DecodeContext,
    bytes: &[u8],
    options: DecodeOptions,
) -> Result<Vec<u8>, DecodeError> {
    let mut raw = Vec::new();
    context.decode_raw_bytes(bytes, options, &mut raw)?;
    Ok(raw)
}

fn collect_raw_with_timeout(
    threads: usize,
    frame_delay: FrameDelay,
    bytes: Vec<u8>,
) -> Result<Vec<u8>, DecodeError> {
    let result = Arc::new((Mutex::new(None), Condvar::new()));
    let worker_result = Arc::clone(&result);
    std::thread::spawn(move || {
        let decoded = collect_raw(
            &context(threads, frame_delay),
            &bytes,
            DecodeOptions::default(),
        );
        let (slot, ready) = &*worker_result;
        *slot.lock() = Some(decoded);
        ready.notify_one();
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    let (slot, ready) = &*result;
    let mut slot = slot.lock();
    loop {
        if let Some(decoded) = slot.take() {
            return decoded;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "{threads}-worker decode did not settle within five seconds"
        );
        let timeout = ready.wait_for(&mut slot, remaining);
        assert!(
            !(timeout.timed_out() && slot.is_none()),
            "{threads}-worker decode did not settle within five seconds"
        );
    }
}

#[test]
fn raw_output_matches_depth_one_at_every_capacity() {
    let depth_one = depth_one();
    for (name, fixture) in FIXTURES {
        let expected = collect_raw(&depth_one, fixture, DecodeOptions::default())
            .unwrap_or_else(|error| panic!("depth-one decode of {name} failed: {error}"));
        assert!(!expected.is_empty(), "{name} decoded to no bytes");

        for depth in [1usize, 2, 4, 64] {
            let context = context(4, FrameDelay::from(depth));
            let actual = collect_raw(&context, fixture, DecodeOptions::default())
                .unwrap_or_else(|error| panic!("depth {depth} decode of {name} failed: {error}"));
            assert_eq!(actual, expected, "{name} diverged at frame delay {depth}");
        }
    }
}

#[test]
fn hash_report_matches_depth_one_at_every_capacity() {
    let depth_one = depth_one();
    for (name, fixture) in FIXTURES {
        let expected: Vec<String> = depth_one
            .decode_hash_report_bytes(fixture, DecodeOptions::default())
            .unwrap()
            .frames
            .into_iter()
            .map(|frame| frame.hashes[0].digest_hex.clone())
            .collect();

        for depth in [1usize, 2, 4, 64] {
            let actual: Vec<String> = context(4, FrameDelay::from(depth))
                .decode_hash_report_bytes(fixture, DecodeOptions::default())
                .unwrap()
                .frames
                .into_iter()
                .map(|frame| frame.hashes[0].digest_hex.clone())
                .collect();
            assert_eq!(actual, expected, "{name} diverged at frame delay {depth}");
        }
    }
}

#[test]
fn y4m_output_matches_depth_one() {
    let mut expected = Vec::new();
    depth_one()
        .decode_y4m_bytes(MULTIREF, DecodeOptions::default(), &mut expected)
        .unwrap();

    let mut actual = Vec::new();
    context(4, FrameDelay::Auto)
        .decode_y4m_bytes(MULTIREF, DecodeOptions::default(), &mut actual)
        .unwrap();

    assert_eq!(actual, expected);
}

#[test]
fn key_filter_publication_repeatedly_admits_sef_reconstruction() {
    let expected = collect_raw(&depth_one(), SEF_FAMILIES, DecodeOptions::default()).unwrap();
    for _ in 0..32 {
        let actual = collect_raw(
            &context(8, FrameDelay::Auto),
            SEF_FAMILIES,
            DecodeOptions::default(),
        )
        .unwrap();
        assert_eq!(actual, expected);
    }
}

#[test]
fn an_early_output_limit_break_matches_depth_one() {
    let options =
        DecodeOptions::default().with_output_frame_limit(Some(NonZeroU64::new(2).unwrap()));
    let expected = collect_raw(&depth_one(), EIGHT_FRAME, options).unwrap();

    for depth in [2usize, 4] {
        let actual =
            collect_raw(&context(4, FrameDelay::from(depth)), EIGHT_FRAME, options).unwrap();
        assert_eq!(actual, expected, "limited decode diverged at depth {depth}");
    }
}

#[test]
fn frame_delay_resolves_once_against_the_pool_width() {
    assert_eq!(
        context(4, FrameDelay::from(64usize)).frame_delay().get(),
        64
    );
    assert_eq!(context(4, FrameDelay::Auto).frame_delay().get(), 4);
    assert_eq!(context(4, FrameDelay::from(2usize)).frame_delay().get(), 2);
    assert_eq!(context(2, FrameDelay::Auto).frame_delay().get(), 2);
    assert_eq!(context(1, FrameDelay::Auto).frame_delay().get(), 1);
}

#[test]
fn fixed_one_and_auto_complete_on_one_worker() {
    let expected = collect_raw(
        &context(4, FrameDelay::Auto),
        MULTIREF,
        DecodeOptions::default(),
    )
    .unwrap();
    for frame_delay in [FrameDelay::from(1usize), FrameDelay::Auto] {
        let actual = collect_raw_with_timeout(1, frame_delay, MULTIREF.to_vec()).unwrap();
        assert_eq!(actual, expected, "output diverged at {frame_delay}");
    }
}

#[test]
fn fixed_one_and_auto_match_across_worker_widths() {
    let expected = collect_raw(&depth_one(), MULTIREF, DecodeOptions::default()).unwrap();
    for threads in [1usize, 2, 3, 4, 8, 10] {
        for frame_delay in [FrameDelay::from(1usize), FrameDelay::Auto] {
            let actual = collect_raw(
                &context(threads, frame_delay),
                MULTIREF,
                DecodeOptions::default(),
            )
            .unwrap();
            assert_eq!(
                actual, expected,
                "output diverged at {threads} worker(s), {frame_delay}"
            );
        }
    }
}

#[test]
fn a_planner_valid_corrupt_stream_fails_identically_at_every_width_and_capacity() {
    let depth_one = depth_one();
    let mut checked = 0usize;
    for offset in (MULTIREF.len() - 24)..MULTIREF.len() {
        let mut bytes = MULTIREF.to_vec();
        bytes[offset] ^= 0xff;
        if depth_one
            .plan_bytes(&bytes, DecodeOptions::default())
            .is_err()
        {
            continue;
        }
        let Err(expected) = collect_raw(&depth_one, &bytes, DecodeOptions::default()) else {
            continue;
        };
        checked += 1;

        for depth in [2usize, 4] {
            let actual = collect_raw(
                &context(4, FrameDelay::from(depth)),
                &bytes,
                DecodeOptions::default(),
            )
            .expect_err("a corrupt stream must fail at every frame delay");
            assert_eq!(
                format!("{actual:?}"),
                format!("{expected:?}"),
                "byte {offset} diverged at frame delay {depth}"
            );
        }
        if checked == 1 {
            let expected_report = DecodeDiagnosticReport::from_decode_error(&expected)
                .expect("planner-valid corruption must produce a typed diagnostic");
            for threads in [1usize, 2, 3, 4, 8, 10] {
                let actual = collect_raw_with_timeout(threads, FrameDelay::Auto, bytes.clone())
                    .expect_err("a planner-valid corrupt stream must fail during decode");
                assert_eq!(
                    core::mem::discriminant(&actual),
                    core::mem::discriminant(&expected),
                    "error variant diverged at {threads} workers"
                );
                let actual_report = DecodeDiagnosticReport::from_decode_error(&actual)
                    .expect("planner-valid corruption must produce a typed diagnostic");
                assert_eq!(
                    &actual_report, &expected_report,
                    "typed diagnostic diverged at {threads} workers"
                );
                assert_eq!(
                    format!("{actual:?}"),
                    format!("{expected:?}"),
                    "decode error diverged at {threads} workers"
                );
            }
        }
    }
    assert!(checked > 0, "no mutated byte produced a decode failure");
}

#[test]
fn orderhint_output_matches_depth_one_at_every_capacity() {
    let expected = collect_raw(&depth_one(), ORDER_HINT_WRAP, DecodeOptions::default()).unwrap();

    for depth in [1usize, 2, 4, 8, 64] {
        let actual = collect_raw(
            &context(8, FrameDelay::from(depth)),
            ORDER_HINT_WRAP,
            DecodeOptions::default(),
        )
        .unwrap();
        assert_eq!(actual, expected, "output diverged at frame delay {depth}");
    }
}

fn reference_store_capped(bytes: u64) -> DecodeOptions {
    DecodeOptions::new(
        DecodeLimits::default().with_max_reference_store_bytes(DecodeLimitThreshold::Max(bytes)),
    )
}

/// The driver charges one decoded frame per live entry of its frame vector and
/// checks `max_reference_store_bytes` before it reserves the next frame's
/// storage, so the peak is the live set plus the frame being admitted, and
/// `reclaim_unowned_frames` retires an entry only when the reference slots, the
/// output scheduler, the emission queue and the in-flight ring have all released
/// it, it has settled, and its storage has a single handle.
///
/// The scheduled path adds exactly two bounded terms to the depth-one live set.
///
/// * The in-flight ring. `InflightRing::reserve` harvests only while the ring is
///   at capacity, and its capacity is the smaller of the resolved requested
///   frame delay and the worker count, so the ring holds that many of the most
///   recent frames at every reclaim. This is
///   SCALE-022's finding: the scheduled walk records a frame's reference update
///   from its parse products, so a frame the reference slots have dropped can
///   still be a selected reference of a reconstruction the driver has not run.
///   The two other depth knobs add nothing on top: the entropy-context limit
///   and the reconstruction lane depth are both bounded by that effective
///   capacity, and every frame they hold reserved its slot through
///   `reserve_pending_slot`, which pushed it on the ring. Unsettled frames are
///   ring frames too, since a harvest waits for the slot to settle.
///
/// * The output handoff. A pipelined hash report shares an emitted frame's
///   storage into a pool task, which keeps the driver's own entry alive through
///   the single-handle rule. That term is capped at
///   `MAX_OUTSTANDING_HASH_FRAMES` in `output/hash.rs`; before that cap it was
///   set by how far the pool trailed the driver, which is why a faster parse
///   (SCALE-036) pushed this bound over by frames the pipeline never retained.
///
/// Peak live storage therefore stays inside the depth-one peak plus one decoded
/// frame per effective pipeline-capacity slot plus the capped handoff.
#[test]
fn pipelined_hash_decode_bounds_reference_storage_by_effective_capacity() {
    const SERIAL_PEAK: u64 = 110_592;
    const DECODED_FRAME: u64 = 12_288;
    const OUTSTANDING_HASH_FRAMES: u64 = 4;

    let depth_one = depth_one()
        .decode_hash_report_bytes(ORDER_HINT_WRAP, reference_store_capped(SERIAL_PEAK))
        .unwrap_or_else(|error| panic!("depth-one decode exceeded {SERIAL_PEAK}: {error}"));
    assert_eq!(depth_one.frames.len(), 121);

    for (threads, requested) in [(2usize, 2usize), (2, 3), (3, 2), (3, 3), (8, 2), (8, 3)] {
        let decoder = context(threads, FrameDelay::from(requested));
        assert_eq!(decoder.frame_delay().get(), requested);
        let effective_capacity = decoder.frame_delay().get().min(threads) as u64;
        let retained_slots = effective_capacity + OUTSTANDING_HASH_FRAMES;
        let ceiling = SERIAL_PEAK + DECODED_FRAME * retained_slots;

        let report = decoder
            .decode_hash_report_bytes(ORDER_HINT_WRAP, reference_store_capped(ceiling))
            .unwrap_or_else(|error| {
                panic!("{threads} workers at frame delay {requested} exceeded {ceiling}: {error}")
            });

        assert_eq!(report.frames.len(), 121);
    }
}

// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Steady-state allocation behaviour of a warmed decode.
//!
//! A global allocator counts every thread's requests, so a decode worker's
//! allocation is counted alongside the calling thread's -- but it also counts
//! whatever else the process is doing, so these checks live in their own test
//! binary and run as one sequential test. A second test running beside them
//! would land its own allocations inside the measured region.

#![allow(clippy::unwrap_used, clippy::panic)]

use splot_core::stream_reader::{StreamUnit, TemporalUnitReader};
use splot_decode::{
    DataProps, DecodeContext, DecodeOptions, DecodeRuntimeConfig, Decoder, DecoderSettings,
    SendStatus,
};
use splot_parallel::{FrameDelay, ThreadCount, WorkerPool};
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};

#[global_allocator]
static ALLOCATOR: &StatsAlloc<std::alloc::System> = &INSTRUMENTED_SYSTEM;

const EIGHT_FRAME: &[u8] = include_bytes!(
    "../../../tests/conformance/vectors/valid/syn-8frame-opfl-refine-all-64x64-q120.ivf"
);

/// The allocation requests one warmed eight-frame decode makes at depth one.
///
/// The count is exact and repeatable on this path, so an increase is a
/// regression to look at rather than a number to raise. Lowering it next to a
/// reduction is the point of having it.
const WARMED_DECODE_ALLOCATIONS: usize = 41;

/// The allocation requests a warmed streaming decoder makes for one more pass
/// over the eight-frame stream's temporal units, pictures dropped as taken.
///
/// The pass settles at 52 to 54: a picture the caller still holds when the
/// driver retires its frame costs the frame slot one handle allocation, and
/// that depends on how the two threads interleave. One allocation more per
/// frame still exceeds this bound.
const WARMED_STREAMING_ALLOCATIONS: usize = 56;

fn context(threads: usize, frame_delay: FrameDelay) -> DecodeContext {
    DecodeContext::new(
        DecodeRuntimeConfig::new(ThreadCount::from(threads)).with_frame_delay(frame_delay),
    )
    .unwrap()
}

/// Decodes once into `reuse`, reporting only that decode's allocation requests.
///
/// `reuse` keeps its capacity across calls so writing the decoded samples is
/// not charged to the decode, and the counts leave the measured region as plain
/// integers so the caller's assertion formatting cannot pollute them.
fn allocations_for_one_decode(context: &DecodeContext, reuse: &mut Vec<u8>) -> usize {
    reuse.clear();
    let region = Region::new(ALLOCATOR);
    let decoded = context.decode_raw_bytes(EIGHT_FRAME, DecodeOptions::default(), reuse);
    let allocations = region.change().allocations;
    assert!(decoded.is_ok(), "measured decode failed");
    allocations
}

#[test]
fn a_warmed_decode_holds_its_steady_allocation_count() {
    let pool = WorkerPool::new(ThreadCount::from(2usize)).unwrap();
    let region = Region::new(ALLOCATOR);
    pool.install(|| {
        let deliberate: Vec<u8> = Vec::with_capacity(8192);
        core::hint::black_box(&deliberate);
    });
    let on_worker = region.change().allocations;
    assert!(
        on_worker > 0,
        "the counter must see an allocation made on a worker thread, saw {on_worker}"
    );

    let single = context(1, FrameDelay::from(1usize));
    let mut reuse = Vec::new();
    for _ in 0..4 {
        allocations_for_one_decode(&single, &mut reuse);
    }
    let warmed = allocations_for_one_decode(&single, &mut reuse);
    assert!(
        warmed <= WARMED_DECODE_ALLOCATIONS,
        "one warmed eight-frame decode requested {warmed} allocations, \
         over the {WARMED_DECODE_ALLOCATIONS} this path settles at"
    );

    let pipelined = context(2, FrameDelay::from(2usize));
    for _ in 0..4 {
        allocations_for_one_decode(&pipelined, &mut reuse);
    }
    let settled = allocations_for_one_decode(&pipelined, &mut reuse);
    for _ in 0..8 {
        allocations_for_one_decode(&pipelined, &mut reuse);
    }
    let later = allocations_for_one_decode(&pipelined, &mut reuse);
    assert!(
        later <= settled.saturating_mul(2),
        "a warmed pipelined decode must not keep growing its allocation count: \
         settled at {settled}, {later} after eight more decodes"
    );

    let mut decoder = Decoder::new(
        DecoderSettings::default()
            .with_threads(ThreadCount::from(2usize))
            .with_frame_delay(FrameDelay::from(2usize)),
    )
    .unwrap();
    let units = units(EIGHT_FRAME);
    let mut pictures = 0;
    for _ in 0..4 {
        pictures += stream_once(&mut decoder, &units);
    }
    let streamed = (0..3)
        .map(|_| {
            let region = Region::new(ALLOCATOR);
            pictures += stream_once(&mut decoder, &units);
            region.change().allocations
        })
        .min()
        .unwrap_or(usize::MAX);
    assert!(
        streamed <= WARMED_STREAMING_ALLOCATIONS,
        "the quietest of three warmed streaming passes requested {streamed} allocations, \
         over the {WARMED_STREAMING_ALLOCATIONS} this path settles at"
    );
    decoder.drain();
    while decoder.get_picture().unwrap().is_some() {
        pictures += 1;
    }
    assert_eq!(pictures, 7 * 8);
}

fn units(ivf: &[u8]) -> Vec<Vec<u8>> {
    let mut reader = TemporalUnitReader::new(ivf);
    let mut units = Vec::new();
    while let Some(unit) = reader.next_unit().unwrap() {
        if let StreamUnit::IvfFrame { payload, .. } = unit {
            units.push(payload.to_vec());
        }
    }
    units
}

/// Sends `units` and drops every picture as it is taken; returns the count.
///
/// The count depends on how far the driver got, since `get_picture` does not
/// wait while the decoder has room for more input. Only a drain settles it.
fn stream_once(decoder: &mut Decoder, units: &[Vec<u8>]) -> usize {
    let mut pictures = 0;
    for unit in units {
        while decoder.send_data(unit, DataProps::default()).unwrap() == SendStatus::Full {
            while decoder.get_picture().unwrap().is_some() {
                pictures += 1;
            }
        }
        while decoder.get_picture().unwrap().is_some() {
            pictures += 1;
        }
    }
    pictures
}

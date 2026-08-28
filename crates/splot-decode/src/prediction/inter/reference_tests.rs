// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Banded reference-read tests.
//!
//! The band never changes a sample: a read admitted by the watermark returns
//! exactly the bytes the settled frame reports, because the filtered workspace
//! and the frozen planes are the same storage. These tests pin both halves of
//! that claim: whole decodes forced through the banded path stay byte-identical,
//! and a read past the watermark is refused instead of substituting a clamped
//! row.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::sync::{Mutex, MutexGuard, PoisonError};

use splot_core::span::ByteOffset;
use splot_recon::{
    BitDepth, CurrentFrameWorkspace, DecodedFrame, DecodedFrameInfo, InterpolationFilter,
    OutputIndex, PixelFormat, PlaneId, PlaneRect, PlaneSize,
};

use super::{ALL_ROWS, HeldFrameSamples, ReferenceSamples, set_forced_banded_reads};
use crate::pipeline::frame_progress::FrameProgress;
use crate::prediction::inter::Mv;
use crate::prediction::inter::mc::{
    InterBlockParams, McBlockRect, WorkspaceSink, motion_compensate_inter_block_into,
};
use crate::{DecodeContext, DecodeError, DecodeOptions, DecodeRuntimeConfig};
use splot_parallel::{FrameDelay, ThreadCount};

const WIDTH: usize = 64;
const HEIGHT: usize = 128;
const OFFSET: ByteOffset = ByteOffset::new(0);

fn collect_raw(
    context: &DecodeContext,
    bytes: &[u8],
    options: DecodeOptions,
) -> Result<Vec<u8>, DecodeError> {
    let mut raw = Vec::new();
    context.decode_raw_bytes(bytes, options, &mut raw)?;
    Ok(raw)
}

/// Serializes the process-wide forced-band flag between harness runs.
static FORCED_BAND: Mutex<()> = Mutex::new(());

fn info(width: usize, height: usize) -> DecodedFrameInfo {
    DecodedFrameInfo::new(
        OutputIndex::new(0),
        BitDepth::Eight,
        PixelFormat::Monochrome,
        PlaneSize::new(width, height).expect("frame size"),
        PlaneRect::new(0, 0, width, height).expect("visible rect"),
    )
    .expect("frame info")
}

/// Fills one workspace with a row-dependent ramp so a wrong row is visible.
fn fill_ramp(workspace: &mut CurrentFrameWorkspace<u8>, height: usize) {
    for row in 0..height {
        workspace
            .fill_rect(
                PlaneId::Y,
                PlaneRect::new(0, row, WIDTH, 1).expect("row rect"),
                (row % 251) as u8,
            )
            .expect("row fill");
    }
}

fn settled_frame() -> DecodedFrame<u8> {
    let mut workspace = CurrentFrameWorkspace::new(info(WIDTH, HEIGHT), 0u8).expect("workspace");
    fill_ramp(&mut workspace, HEIGHT);
    workspace.freeze().expect("frozen frame")
}

/// Opens a progress whose first stripe covers `published` rows of the ramp.
fn published_progress(published: usize) -> FrameProgress<u8> {
    let progress =
        std::sync::Arc::new(FrameProgress::new(info(WIDTH, HEIGHT)).expect("frame progress"));
    assert!(progress.begin(&[(0, published), (published, HEIGHT)]));
    let mut lease = progress.direct_stripe(0).expect("stripe lease");
    let mut target = lease.take_target().expect("stripe target");
    let mut y = target.take(PlaneId::Y).expect("luma target");
    let stride = y.width();
    let origin_y = y.origin_y();
    for (row, samples) in y
        .u8_samples_mut()
        .expect("u8 luma")
        .chunks_exact_mut(stride)
        .enumerate()
    {
        samples.fill(((origin_y + row) % 251) as u8);
    }
    drop(y);
    assert!(lease.submit());
    std::sync::Arc::into_inner(progress).expect("sole progress owner")
}

fn block(
    reference: ReferenceSamples<'_, u8>,
    luma_y: usize,
    luma_h: usize,
) -> InterBlockParams<'_, u8> {
    InterBlockParams::single(
        reference,
        McBlockRect::from_luma_rect(0, luma_y, WIDTH, luma_h),
        Mv::ZERO,
        InterpolationFilter::EightTap,
    )
    .with_chroma(false)
}

#[test]
fn a_banded_read_inside_the_watermark_returns_the_settled_samples() {
    let settled = settled_frame();
    let progress = published_progress(64);
    let held = HeldFrameSamples::Filtering(progress.read().expect("a published prefix"));
    let banded = held.samples().expect("banded samples");

    let (settled_view, settled_cols, settled_rows) = ReferenceSamples::settled(&settled)
        .plane_view(PlaneId::Y, 63, OFFSET)
        .expect("settled view");
    let (banded_view, banded_cols, banded_rows) = banded
        .plane_view(PlaneId::Y, 63, OFFSET)
        .expect("a read inside the watermark is admitted");

    assert_eq!((banded_cols, banded_rows), (settled_cols, settled_rows));
    assert_eq!(
        (banded_view.width(), banded_view.height()),
        (settled_view.width(), settled_view.height()),
        "a partial frame keeps the whole frame's geometry"
    );
    for row in 0..64 {
        for col in 0..settled_view.width() {
            assert_eq!(
                banded_view.sample(row, col),
                settled_view.sample(row, col),
                "sample ({row}, {col}) diverged"
            );
        }
    }
}

#[test]
fn a_banded_read_past_the_watermark_is_refused() {
    let progress = published_progress(64);
    let held = HeldFrameSamples::Filtering(progress.read().expect("a published prefix"));
    let banded = held.samples().expect("banded samples");

    let error = banded
        .plane_view(PlaneId::Y, 64, OFFSET)
        .expect_err("the first unpublished row is refused");
    assert!(
        format!("{error}").contains("published"),
        "unexpected diagnostic: {error}"
    );
    assert!(
        banded.plane_view(PlaneId::Y, ALL_ROWS, OFFSET).is_err(),
        "a whole-plane reader is refused while rows are missing"
    );
}

#[test]
fn a_refused_banded_read_propagates_out_of_motion_compensation() {
    let progress = published_progress(64);
    let held = HeldFrameSamples::Filtering(progress.read().expect("a published prefix"));
    let banded = held.samples().expect("banded samples");
    let mut output = CurrentFrameWorkspace::new(info(WIDTH, HEIGHT), 0u8).expect("workspace");

    motion_compensate_inter_block_into(
        &mut WorkspaceSink::Frame(&mut output),
        block(banded, 96, 16),
        OFFSET,
    )
    .expect_err("a block below the watermark cannot be predicted");

    motion_compensate_inter_block_into(
        &mut WorkspaceSink::Frame(&mut output),
        block(banded, 0, 16),
        OFFSET,
    )
    .expect("a block inside the watermark is predicted");
}

#[test]
fn a_settled_frame_reads_the_same_bytes_through_the_banded_path() {
    let settled = settled_frame();
    let mut plain = CurrentFrameWorkspace::new(info(WIDTH, HEIGHT), 0u8).expect("workspace");
    motion_compensate_inter_block_into(
        &mut WorkspaceSink::Frame(&mut plain),
        block(ReferenceSamples::settled(&settled), 0, HEIGHT),
        OFFSET,
    )
    .expect("settled prediction");

    let mut forced = CurrentFrameWorkspace::new(info(WIDTH, HEIGHT), 0u8).expect("workspace");
    let _guard = forced_band_scope();
    motion_compensate_inter_block_into(
        &mut WorkspaceSink::Frame(&mut forced),
        block(ReferenceSamples::settled(&settled), 0, HEIGHT),
        OFFSET,
    )
    .expect("forced banded prediction");

    assert_eq!(
        plain.samples(PlaneId::Y).expect("settled samples"),
        forced.samples(PlaneId::Y).expect("banded samples")
    );
}

/// Turns forced banded reads on for the caller's scope.
fn forced_band_scope() -> ForcedBand {
    let guard = FORCED_BAND.lock().unwrap_or_else(PoisonError::into_inner);
    set_forced_banded_reads(true);
    ForcedBand { _guard: guard }
}

struct ForcedBand {
    _guard: MutexGuard<'static, ()>,
}

impl Drop for ForcedBand {
    fn drop(&mut self) {
        set_forced_banded_reads(false);
    }
}

const MULTIREF: &[u8] =
    include_bytes!("../../../../../tests/conformance/vectors/valid/syn-3frame-multiref-64x64.ivf");
const OPFL: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/syn-8frame-opfl-refine-all-64x64-q120.ivf"
);
const TIP: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/syn-frame-tip-families-64x64.ivf"
);
const SUBPEL: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/syn-2frame-subpel-inter-64x64.ivf"
);
const WARP: &[u8] =
    include_bytes!("../../../../../tests/conformance/vectors/valid/syn-warp-inter-128x128.ivf");
const COMPOUND: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/syn-3frame-compound-average-64x64.ivf"
);
const BRIDGE: &[u8] =
    include_bytes!("../../../../../tests/conformance/vectors/valid/syn-bridge-celu-64x64.ivf");

const FIXTURES: &[(&str, &[u8])] = &[
    ("syn-3frame-multiref-64x64", MULTIREF),
    ("syn-8frame-opfl-refine-all-64x64-q120", OPFL),
    ("syn-frame-tip-families-64x64", TIP),
    ("syn-2frame-subpel-inter-64x64", SUBPEL),
    ("syn-warp-inter-128x128", WARP),
    ("syn-3frame-compound-average-64x64", COMPOUND),
    ("syn-bridge-celu-64x64", BRIDGE),
];

#[test]
fn forced_banded_reads_decode_every_inter_fixture_byte_identically() {
    let context = DecodeContext::new(
        DecodeRuntimeConfig::new(ThreadCount::from(4usize))
            .with_frame_delay(FrameDelay::from(4usize)),
    )
    .expect("decode context");

    let expected: Vec<Vec<u8>> = FIXTURES
        .iter()
        .map(|(name, fixture)| {
            let output = collect_raw(&context, fixture, DecodeOptions::default())
                .unwrap_or_else(|error| panic!("settled decode of {name} failed: {error}"));
            assert!(!output.is_empty(), "{name} decoded to no bytes");
            output
        })
        .collect();

    let _guard = forced_band_scope();
    for ((name, fixture), expected) in FIXTURES.iter().zip(&expected) {
        let actual = collect_raw(&context, fixture, DecodeOptions::default())
            .unwrap_or_else(|error| panic!("banded decode of {name} failed: {error}"));
        assert_eq!(&actual, expected, "{name} diverged under banded reads");
    }
}

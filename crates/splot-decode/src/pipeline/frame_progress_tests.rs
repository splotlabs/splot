// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Watermark and published-prefix tests for [`super::FrameProgress`].

#![allow(clippy::expect_used)]

use splot_recon::{BitDepth, DecodedFrameInfo, OutputIndex, PixelFormat, PlaneRect, PlaneSize};

use super::FrameProgress;

fn info(width: usize, height: usize, format: PixelFormat) -> DecodedFrameInfo {
    DecodedFrameInfo::new(
        OutputIndex::new(0),
        BitDepth::Eight,
        format,
        PlaneSize::new(width, height).expect("frame size"),
        PlaneRect::new(0, 0, width, height).expect("visible rect"),
    )
    .expect("frame info")
}

fn new_progress(width: usize, height: usize, format: PixelFormat) -> FrameProgress<u8> {
    FrameProgress::new(info(width, height, format)).expect("frame progress")
}

#[test]
fn out_of_order_stripes_advance_only_the_contiguous_prefix() {
    let progress = new_progress(64, 192, PixelFormat::Monochrome);
    assert!(progress.begin(&[(0, 64), (64, 128), (128, 192)]));
    assert_eq!(progress.published_luma_rows(), 0);

    progress.publish(2);
    assert_eq!(
        progress.published_luma_rows(),
        0,
        "a late stripe alone publishes no prefix"
    );

    progress.publish(1);
    assert_eq!(
        progress.published_luma_rows(),
        0,
        "the prefix still misses the frame top"
    );

    progress.publish(0);
    assert_eq!(
        progress.published_luma_rows(),
        192,
        "closing the top completes every landed stripe at once"
    );
}

#[test]
fn the_watermark_advances_one_stripe_at_a_time_in_order() {
    let progress = new_progress(64, 160, PixelFormat::Monochrome);
    assert!(progress.begin(&[(0, 64), (64, 128), (128, 160)]));

    for (stripe, expected) in [(0usize, 64usize), (1, 128), (2, 160)] {
        progress.publish(stripe);
        assert_eq!(progress.published_luma_rows(), expected);
    }
}

#[test]
fn a_repeated_or_out_of_range_publish_never_moves_the_watermark_backwards() {
    let progress = new_progress(64, 128, PixelFormat::Monochrome);
    assert!(progress.begin(&[(0, 64), (64, 128)]));

    progress.publish(0);
    assert_eq!(progress.published_luma_rows(), 64);
    progress.publish(0);
    assert_eq!(progress.published_luma_rows(), 64);
    progress.publish(7);
    assert_eq!(progress.published_luma_rows(), 64);
    progress.publish(1);
    assert_eq!(progress.published_luma_rows(), 128);
}

#[test]
fn a_non_contiguous_geometry_is_refused_and_publishes_nothing() {
    let progress = new_progress(64, 192, PixelFormat::Monochrome);
    assert!(
        !progress.begin(&[(0, 64), (128, 192)]),
        "a gap must be refused"
    );
    progress.publish(0);
    assert_eq!(progress.published_luma_rows(), 0);

    let descending = new_progress(64, 128, PixelFormat::Monochrome);
    assert!(!descending.begin(&[(64, 128), (0, 64)]));

    let empty_stripe = new_progress(64, 128, PixelFormat::Monochrome);
    assert!(!empty_stripe.begin(&[(0, 0), (0, 128)]));
}

#[test]
fn the_geometry_installs_once() {
    let progress = new_progress(64, 128, PixelFormat::Monochrome);
    assert!(progress.begin(&[(0, 64), (64, 128)]));
    assert!(
        !progress.begin(&[(0, 128)]),
        "a second geometry must not replace the first"
    );
    progress.publish(0);
    assert_eq!(progress.published_luma_rows(), 64);
}

#[test]
fn chroma_rows_truncate_to_the_fully_published_luma_pairs() {
    let progress = new_progress(64, 192, PixelFormat::Yuv420);
    assert!(progress.begin(&[(0, 65), (65, 192)]));

    progress.publish(0);
    assert_eq!(progress.published_luma_rows(), 65);
    assert_eq!(
        progress.read().expect("a published prefix").chroma_rows(),
        32,
        "luma row 64 alone does not complete chroma row 32"
    );

    let full = new_progress(64, 128, PixelFormat::Yuv420);
    assert!(full.begin(&[(0, 64), (64, 128)]));
    full.publish(0);
    assert_eq!(full.read().expect("a published prefix").chroma_rows(), 32);

    let unsubsampled = new_progress(64, 128, PixelFormat::Yuv444);
    assert!(unsubsampled.begin(&[(0, 64), (64, 128)]));
    unsubsampled.publish(0);
    assert_eq!(
        unsubsampled
            .read()
            .expect("a published prefix")
            .chroma_rows(),
        64
    );
}

#[test]
fn reads_are_refused_before_the_first_stripe_and_after_the_freeze() {
    let progress = new_progress(64, 128, PixelFormat::Monochrome);
    assert!(progress.begin(&[(0, 64), (64, 128)]));
    assert!(
        progress.read().is_none(),
        "an unpublished frame exposes no rows"
    );

    progress.publish(0);
    let published = progress.read().expect("a published prefix");
    assert_eq!(published.luma_rows(), 64);
    assert!(published.workspace().is_ok());
    drop(published);

    let frame = progress
        .freeze_workspace(|frame| frame)
        .expect("the frozen frame");
    assert!(
        progress.read().is_none(),
        "the freeze closes every banded read"
    );
    assert!(
        progress.freeze_workspace(|frame| frame).is_err(),
        "the workspace is frozen once"
    );
    assert!(
        progress.publish_stripe(0, Box::new(|_| Ok(()))).is_err(),
        "a publish after the freeze fails closed"
    );
    drop(frame);
}

#[test]
fn a_failed_phase_publishes_no_readable_row() {
    let progress = new_progress(64, 128, PixelFormat::Yuv420);
    assert!(progress.begin(&[(0, 64), (64, 128)]));
    progress.publish(0);
    assert_eq!(progress.published_luma_rows(), 64);

    progress.publish_terminal(false);
    assert_eq!(
        progress.published_luma_rows(),
        0,
        "a failed phase names no readable row, not the whole frame"
    );
    assert!(
        progress.read().is_none(),
        "the never-frozen workspace of a failed phase must not be readable"
    );
}

#[test]
fn a_finished_phase_publishes_the_whole_frame() {
    let progress = new_progress(64, 128, PixelFormat::Monochrome);
    assert!(progress.begin(&[(0, 64), (64, 128)]));

    progress.publish_terminal(true);
    assert_eq!(progress.published_luma_rows(), 128);
    assert_eq!(
        progress.read().expect("a published frame").luma_rows(),
        128,
        "a phase that filtered every row publishes every row"
    );
}

#[test]
fn the_freeze_publishes_before_it_releases_the_workspace() {
    let progress = new_progress(64, 128, PixelFormat::Monochrome);
    assert!(progress.begin(&[(0, 64), (64, 128)]));
    progress.publish(0);

    let published_inside = progress
        .freeze_workspace(|frame| {
            drop(frame);
            true
        })
        .expect("the frozen frame");

    assert!(
        published_inside,
        "a reader arriving mid-freeze blocks on the workspace lock, so what this hook publishes is visible before any read resumes"
    );
}

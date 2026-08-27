// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Selection and retention policies for the filter-source buffer caches.

#![allow(clippy::expect_used)]

use super::{
    DeblockedSource, DeblockedWindow, DeblockedWindowSequence, FramePlane, StripePlane,
    intersect_rows, select_buffer_index, window_bounds,
};
use splot_recon::{
    BitDepth, CurrentFrameWorkspace, DecodedFrameInfo, OutputIndex, PixelFormat, PlaneId,
    PlaneRect, PlaneSize,
};
use std::any::Any;
use std::sync::Arc;
#[cfg(not(miri))]
use std::sync::{Barrier, Mutex};

fn workspace(width: usize, height: usize) -> CurrentFrameWorkspace<u16> {
    let info = DecodedFrameInfo::new(
        OutputIndex::new(0),
        BitDepth::Eight,
        PixelFormat::Yuv420,
        PlaneSize::new(width, height).expect("frame size"),
        PlaneRect::new(0, 0, width, height).expect("visible rect"),
    )
    .expect("frame info");
    let mut workspace = CurrentFrameWorkspace::new(info, 0).expect("workspace");
    for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
        let size = workspace.plane(plane).expect("plane").storage_size();
        for y in 0..size.height() {
            for x in 0..size.width() {
                workspace
                    .set_reconstructed_sample(plane, x, y, ((y * 17 + x * 3) & 255) as u16)
                    .expect("sample");
            }
        }
    }
    workspace
}

#[test]
fn deblocked_source_keeps_read_leases_disjoint_from_later_writes() {
    let mut source = DeblockedSource::new(workspace(16, 64));
    assert!(source.publish_final_rows(32));
    let lease = source.lease(0, 16, 8).expect("final-row lease");
    assert!(
        source
            .with_plane_rows_mut(PlaneId::Y, 31, 40, |_, _, _, _, _| ())
            .is_none()
    );

    #[cfg(miri)]
    let actual = {
        let planes = lease.planes().expect("leased planes");
        let row = planes.y.row(15).expect("leased row");
        source
            .with_plane_rows_mut(PlaneId::Y, 32, 40, |samples, _, _, _, _| {
                samples.fill(0);
            })
            .expect("disjoint mutable rows");
        row.iter().copied().sum::<u16>()
    };

    #[cfg(not(miri))]
    let actual = {
        let ready = Arc::new(Barrier::new(2));
        let reader_ready = Arc::clone(&ready);
        let read_sum = Arc::new(Mutex::new(None));
        let reader_sum = Arc::clone(&read_sum);
        let pool = splot_parallel::WorkerPool::new(splot_parallel::ThreadCount::Fixed(
            2.try_into().expect("two workers"),
        ))
        .expect("worker pool");
        pool.install(|| {
            splot_parallel::ready_task_scope(|scope| {
                scope.spawn(move |_| {
                    reader_ready.wait();
                    let sum = lease
                        .planes()
                        .expect("leased planes")
                        .y
                        .row(15)
                        .expect("leased row")
                        .iter()
                        .copied()
                        .sum::<u16>();
                    *reader_sum
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(sum);
                });
                ready.wait();
                source
                    .with_plane_rows_mut(PlaneId::Y, 32, 40, |samples, _, _, _, _| {
                        samples.fill(0);
                    })
                    .expect("disjoint mutable rows");
            })
            .expect("ready task scope");
        });
        read_sum
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .expect("reader result")
    };

    let expected = (0..16).map(|x| (15 * 17 + x * 3) & 255).sum::<usize>() as u16;
    assert_eq!(actual, expected);
}

#[test]
fn window_cache_selection_matches_type_and_capacity() {
    let buffers: Vec<Box<dyn Any + Send>> = vec![
        Box::new(Vec::<u8>::with_capacity(96)),
        Box::new(Vec::<u16>::with_capacity(32)),
        Box::new(Vec::<u16>::with_capacity(128)),
        Box::new(Vec::<u16>::with_capacity(256)),
    ];
    let capacities = || {
        buffers.iter().enumerate().filter_map(|(index, buffer)| {
            buffer
                .downcast_ref::<Vec<u16>>()
                .map(|buffer| (index, buffer.capacity()))
        })
    };

    assert_eq!(select_buffer_index(capacities(), 96, false), Some(2));
    assert_eq!(select_buffer_index(capacities(), 512, false), None);
    assert_eq!(select_buffer_index(capacities(), 512, true), Some(3));
}

#[test]
fn stripe_cache_selection_uses_fresh_storage_until_full() {
    let capacities = [(0, 32), (1, 128), (2, 256)];

    assert_eq!(select_buffer_index(capacities, 96, false), Some(1));
    assert_eq!(select_buffer_index(capacities, 512, false), None);
    assert_eq!(select_buffer_index(capacities, 512, true), Some(2));
}

#[test]
fn stripe_rect_mut_rejects_a_rectangle_overhanging_the_row() {
    let mut stripe = StripePlane::from_samples(4, 2, 0, vec![0; 8]).expect("a valid stripe");
    let rect = PlaneRect::new(3, 0, 2, 1).expect("a valid rectangle");

    assert!(stripe.rect_mut(rect).is_none());
}

#[test]
fn stripe_copy_preserves_u8_source_samples() {
    let source = [1_u8, 2, 3, 4, 5, 6, 7, 8];
    let plane = FramePlane::window(&source, 4, 2, 0, 2).expect("a valid source plane");

    let stripe = StripePlane::copy_from(plane, 0, 2).expect("a valid stripe copy");

    assert_eq!(stripe.samples(), [1, 2, 3, 4, 5, 6, 7, 8]);
}

#[test]
fn u8_direct_stripe_flushes_checked_filter_samples() {
    let info = DecodedFrameInfo::new(
        OutputIndex::new(0),
        BitDepth::Eight,
        PixelFormat::Monochrome,
        PlaneSize::new(8, 4).expect("frame size"),
        PlaneRect::new(0, 0, 8, 4).expect("visible rect"),
    )
    .expect("frame info");
    let progress = Arc::new(
        crate::pipeline::frame_progress::FrameProgress::<u8>::new(info).expect("frame progress"),
    );
    assert!(progress.begin(&[(0, 4)]));
    let mut lease = progress.direct_stripe(0).expect("stripe lease");
    let mut target = lease.take_target().expect("stripe target");
    let source = StripePlane::from_samples(8, 4, 0, (0_u16..32).collect()).expect("source stripe");
    let mut output = source
        .copy_rows_into(0, 4, target.take(PlaneId::Y))
        .expect("direct stripe");

    assert!(output.is_direct());
    output.samples_mut()[7] = 201;
    output.finish_direct().expect("u8 flush");
    drop(output);
    assert!(lease.submit());

    let frame = progress
        .freeze_workspace(core::convert::identity)
        .expect("frozen frame");
    let mut expected: Vec<u8> = (0_u8..32).collect();
    expected[7] = 201;
    assert_eq!(frame.y().samples(), expected);
}

#[test]
fn u8_direct_stripe_rejects_unrepresentable_filter_samples_without_publication() {
    let info = DecodedFrameInfo::new(
        OutputIndex::new(0),
        BitDepth::Eight,
        PixelFormat::Monochrome,
        PlaneSize::new(4, 1).expect("frame size"),
        PlaneRect::new(0, 0, 4, 1).expect("visible rect"),
    )
    .expect("frame info");
    let progress = Arc::new(
        crate::pipeline::frame_progress::FrameProgress::<u8>::new(info).expect("frame progress"),
    );
    assert!(progress.begin(&[(0, 1)]));
    let mut lease = progress.direct_stripe(0).expect("stripe lease");
    let mut target = lease.take_target().expect("stripe target");
    let source = StripePlane::from_samples(4, 1, 0, vec![1, 2, 256, 4]).expect("source stripe");
    let mut output = source
        .copy_rows_into(0, 1, target.take(PlaneId::Y))
        .expect("direct stripe");

    assert!(output.finish_direct().is_err());
    drop(output);
    drop(target);
    drop(lease);
    assert_eq!(progress.published_luma_rows(), 0);
    assert!(progress.direct_stripe(0).is_some(), "the lease is reusable");
}

#[test]
fn adjacent_windows_reuse_their_immutable_boundary_rows() {
    let workspace = workspace(16, 184);
    let ranges = [(0, 56), (56, 120), (120, 184)];
    let margin = 10;
    let mut sequence = DeblockedWindowSequence::default();
    let windows: Vec<_> = (0..ranges.len())
        .map(|stripe| {
            sequence
                .extract(&workspace, &ranges, stripe, margin)
                .expect("shared window")
        })
        .collect();

    for (stripe, window) in windows.iter().enumerate() {
        let independent =
            DeblockedWindow::extract(&workspace, ranges[stripe].0, ranges[stripe].1, margin)
                .expect("independent window");
        let actual = window.planes().expect("shared planes");
        let expected = independent.planes().expect("independent planes");
        for (actual, expected) in [
            (Some(actual.y), Some(expected.y)),
            (actual.u, expected.u),
            (actual.v, expected.v),
        ] {
            let (Some(actual), Some(expected)) = (actual, expected) else {
                continue;
            };
            for y in expected.origin_y()..expected.end_y() {
                assert_eq!(actual.row(y), expected.row(y), "stripe {stripe}, row {y}");
            }
        }
    }

    let middle_luma = windows[1].planes().expect("middle planes").y;
    assert!(middle_luma.contiguous_rows(66, 80).is_some());
    assert!(middle_luma.contiguous_rows(62, 70).is_none());
    assert!(middle_luma.row(62).is_some());
    assert!(middle_luma.packed_plane(54, 66).is_some());
    assert!(middle_luma.packed_plane(62, 74).is_none());
    let copied = StripePlane::copy_from(middle_luma, ranges[1].0, ranges[1].1)
        .expect("two-span stripe copy");
    for (offset, row) in copied.samples().chunks_exact(copied.width()).enumerate() {
        assert_eq!(row, middle_luma.row(ranges[1].0 + offset).expect("row"));
    }

    let plane_geometry = [(16, 184, 0), (8, 92, 1), (8, 92, 1)];
    let independent_samples: usize = plane_geometry
        .iter()
        .map(|&(width, height, shift)| {
            ranges
                .iter()
                .map(|&range| {
                    let bounds = window_bounds(range, shift, margin, height).expect("bounds");
                    (bounds.1 - bounds.0) * width
                })
                .sum::<usize>()
        })
        .sum();
    let repeated_samples: usize = plane_geometry
        .iter()
        .map(|&(width, height, shift)| {
            ranges
                .windows(2)
                .map(|pair| {
                    let left = window_bounds(pair[0], shift, margin, height).expect("left");
                    let right = window_bounds(pair[1], shift, margin, height).expect("right");
                    let overlap = intersect_rows(left, right).expect("overlap");
                    (overlap.1 - overlap.0) * width
                })
                .sum::<usize>()
        })
        .sum();
    assert_eq!(
        sequence.copied_samples(),
        independent_samples - repeated_samples
    );
}

#[test]
fn overlapping_top_and_bottom_boundaries_fall_back_to_one_contiguous_copy() {
    let workspace = workspace(16, 24);
    let ranges = [(0, 8), (8, 16), (16, 24)];
    let margin = 10;
    let mut sequence = DeblockedWindowSequence::default();

    for stripe in 0..ranges.len() {
        let window = sequence
            .extract(&workspace, &ranges, stripe, margin)
            .expect("shared window");
        let independent =
            DeblockedWindow::extract(&workspace, ranges[stripe].0, ranges[stripe].1, margin)
                .expect("independent window");
        let actual = window.planes().expect("shared planes").y;
        let expected = independent.planes().expect("independent planes").y;
        assert_eq!(
            actual.contiguous_rows(actual.origin_y(), actual.end_y()),
            expected.contiguous_rows(expected.origin_y(), expected.end_y())
        );
    }
}

#[test]
fn window_sequence_rejects_out_of_order_extraction() {
    let workspace = workspace(16, 32);
    let ranges = [(0, 16), (16, 32)];
    let mut sequence = DeblockedWindowSequence::default();

    assert!(sequence.extract(&workspace, &ranges, 1, 10).is_err());
}

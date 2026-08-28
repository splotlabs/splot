// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Selection and retention policies for the filter-source buffer caches.

#![allow(clippy::expect_used)]

use super::{
    DeblockedSource, DeblockedWindow, DeblockedWindowSequence, FramePlane, StripePlane,
    intersect_rows, recycle_stripe_sample_buffer, select_buffer_index, take_stripe_sample_buffer,
    window_bounds,
};
use splot_recon::{
    BitDepth, CurrentFrameWorkspace, DecodedFrameInfo, OutputIndex, PixelFormat, PlaneId,
    PlaneRect, PlaneSize,
};
use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(not(miri))]
use std::sync::{Barrier, Mutex};

fn workspace(width: usize, height: usize) -> CurrentFrameWorkspace<u16> {
    workspace_with_format(width, height, PixelFormat::Yuv420)
}

fn workspace_with_format(
    width: usize,
    height: usize,
    format: PixelFormat,
) -> CurrentFrameWorkspace<u16> {
    let info = DecodedFrameInfo::new(
        OutputIndex::new(0),
        BitDepth::Eight,
        format,
        PlaneSize::new(width, height).expect("frame size"),
        PlaneRect::new(0, 0, width, height).expect("visible rect"),
    )
    .expect("frame info");
    let mut workspace = CurrentFrameWorkspace::new(info, 0).expect("workspace");
    for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
        let Ok(view) = workspace.plane(plane) else {
            continue;
        };
        let size = view.storage_size();
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
fn finalized_leases_cover_first_middle_and_terminal_margins_for_all_formats() {
    let ranges = [(0, 56), (56, 120), (120, 129)];
    for format in [
        PixelFormat::Monochrome,
        PixelFormat::Yuv420,
        PixelFormat::Yuv444,
    ] {
        let mut source = DeblockedSource::new(workspace_with_format(16, 129, format));
        assert!(source.publish_final_rows(129));
        for (start, end) in ranges {
            let lease = source.lease(start, end, 10).expect("final stripe lease");
            let planes = lease.planes().expect("leased planes");
            let expected_y = window_bounds((start, end), 0, 10, 129).expect("luma bounds");
            assert_eq!((planes.y.origin_y(), planes.y.end_y()), expected_y);
            if format.is_monochrome() {
                assert!(planes.u.is_none() && planes.v.is_none());
                continue;
            }
            let shift = usize::from(format.subsampling_y());
            let chroma_height = 129usize.div_ceil(1 << shift);
            let expected =
                window_bounds((start, end), shift, 10, chroma_height).expect("chroma bounds");
            for plane in [planes.u.expect("u plane"), planes.v.expect("v plane")] {
                assert_eq!((plane.origin_y(), plane.end_y()), expected);
            }
        }
    }
}

#[test]
fn serial_lease_retarget_keeps_checked_ranges_and_source_owner() {
    let mut source = DeblockedSource::new(workspace(16, 129));
    assert!(source.publish_final_rows(129));
    let mut lease = source.lease(0, 56, 10).expect("first stripe lease");
    assert!(source.retarget_lease(&mut lease, 56, 120, 10));
    let middle = lease.planes().expect("middle stripe planes");
    assert_eq!((middle.y.origin_y(), middle.y.end_y()), (46, 129));

    assert!(!source.retarget_lease(&mut lease, 120, 130, 10));
    let unchanged = lease.planes().expect("unchanged stripe planes");
    assert_eq!((unchanged.y.origin_y(), unchanged.y.end_y()), (46, 129));

    let mut other = DeblockedSource::new(workspace(16, 129));
    assert!(other.publish_final_rows(129));
    assert!(!other.retarget_lease(&mut lease, 0, 56, 10));
    let unchanged = lease.planes().expect("original source planes");
    assert_eq!((unchanged.y.origin_y(), unchanged.y.end_y()), (46, 129));
}

#[test]
fn multiple_immutable_leases_keep_the_source_alive_until_the_last_drop() {
    let recycled = Arc::new(AtomicBool::new(false));
    let mut source = DeblockedSource::new_with_recycle_probe(
        workspace_with_format(16, 129, PixelFormat::Yuv420),
        Arc::clone(&recycled),
    );
    assert!(source.publish_final_rows(129));
    let first = Arc::new(source.lease(0, 56, 10).expect("first lease"));
    let middle = Arc::new(source.lease(56, 120, 10).expect("middle lease"));

    #[cfg(not(miri))]
    {
        let ready = Arc::new(Barrier::new(3));
        let first_reader = Arc::clone(&first);
        let first_ready = Arc::clone(&ready);
        let middle_reader = Arc::clone(&middle);
        let middle_ready = Arc::clone(&ready);
        let pool = splot_parallel::WorkerPool::new(splot_parallel::ThreadCount::Fixed(
            3.try_into().expect("three workers"),
        ))
        .expect("worker pool");
        pool.install(|| {
            splot_parallel::ready_task_scope(|scope| {
                scope.spawn(move |_| {
                    first_ready.wait();
                    assert!(
                        first_reader
                            .planes()
                            .and_then(|planes| planes.y.row(55))
                            .is_some()
                    );
                });
                scope.spawn(move |_| {
                    middle_ready.wait();
                    assert!(
                        middle_reader
                            .planes()
                            .and_then(|planes| planes.y.row(56))
                            .is_some()
                    );
                });
                ready.wait();
            })
            .expect("ready task scope");
        });
    }

    assert_eq!(
        first
            .planes()
            .expect("first planes")
            .y
            .row(55)
            .expect("first lease row")
            .len(),
        16
    );
    assert_eq!(
        middle
            .planes()
            .expect("middle planes")
            .y
            .row(56)
            .expect("middle lease row")
            .len(),
        16
    );

    drop(source);
    assert!(!recycled.load(Ordering::SeqCst));
    drop(first);
    assert!(!recycled.load(Ordering::SeqCst));
    drop(middle);
    assert!(recycled.load(Ordering::SeqCst));
}

#[test]
fn refused_leases_leave_the_source_fail_closed_and_recyclable() {
    let recycled = Arc::new(AtomicBool::new(false));
    let mut source = DeblockedSource::new_with_recycle_probe(
        workspace_with_format(16, 65, PixelFormat::Yuv420),
        Arc::clone(&recycled),
    );
    assert!(source.lease(0, 16, 10).is_none());
    assert!(source.publish_final_rows(32));
    assert!(source.lease(16, 16, 0).is_none());
    assert!(source.lease(32, 66, 0).is_none());
    assert!(source.lease(16, 32, usize::MAX).is_none());
    assert!(!source.publish_final_rows(31));
    assert!(source.lease(16, 32, 0).is_some());
    drop(source);
    assert!(recycled.load(Ordering::SeqCst));
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
fn u8_direct_stripe_initializes_contiguous_u16_source() {
    let info = DecodedFrameInfo::new(
        OutputIndex::new(0),
        BitDepth::Eight,
        PixelFormat::Monochrome,
        PlaneSize::new(4, 2).expect("frame size"),
        PlaneRect::new(0, 0, 4, 2).expect("visible rect"),
    )
    .expect("frame info");
    let progress = Arc::new(
        crate::pipeline::frame_progress::FrameProgress::<u8>::new(info).expect("frame progress"),
    );
    assert!(progress.begin(&[(0, 2)]));
    let source_samples = [1_u16, 2, 3, 4, 5, 6, 7, 8];
    let source = FramePlane::window(&source_samples, 4, 2, 0, 2).expect("source plane");
    let mut lease = progress.direct_stripe(0).expect("stripe lease");
    let mut target = lease.take_target().expect("stripe target");
    let mut output =
        StripePlane::copy_from_into(source, 0, 2, target.take(PlaneId::Y)).expect("direct stripe");

    assert_eq!(output.samples(), source_samples);
    output.finish_direct().expect("u8 flush");
    drop(output);
    assert!(lease.submit());
    let frame = progress
        .freeze_workspace(core::convert::identity)
        .expect("frozen frame");
    assert_eq!(frame.y().samples(), [1, 2, 3, 4, 5, 6, 7, 8]);
}

#[test]
fn u8_direct_stripe_initializes_strided_u8_rows() {
    let info = DecodedFrameInfo::new(
        OutputIndex::new(0),
        BitDepth::Eight,
        PixelFormat::Monochrome,
        PlaneSize::new(4, 2).expect("frame size"),
        PlaneRect::new(0, 0, 4, 2).expect("visible rect"),
    )
    .expect("frame info");
    let progress = Arc::new(
        crate::pipeline::frame_progress::FrameProgress::<u8>::new(info).expect("frame progress"),
    );
    assert!(progress.begin(&[(0, 2)]));
    let source_samples = [1_u8, 2, 3, 4, 99, 99, 5, 6, 7, 8, 99, 99];
    let source = FramePlane {
        width: 4,
        height: 2,
        stride: 6,
        origin_y: 0,
        storage_origin_y: 0,
        storage_rows: 2,
        samples: &source_samples,
        secondary: &[],
    };
    let mut lease = progress.direct_stripe(0).expect("stripe lease");
    let mut target = lease.take_target().expect("stripe target");
    let mut output =
        StripePlane::copy_from_into(source, 0, 2, target.take(PlaneId::Y)).expect("direct stripe");

    assert_eq!(output.samples(), [1, 2, 3, 4, 5, 6, 7, 8]);
    output.finish_direct().expect("u8 flush");
    drop(output);
    assert!(lease.submit());
    let frame = progress
        .freeze_workspace(core::convert::identity)
        .expect("frozen frame");
    assert_eq!(frame.y().samples(), [1, 2, 3, 4, 5, 6, 7, 8]);
}

#[test]
fn partial_u8_source_failure_recycles_length_zero_staging() {
    let width = 4;
    let height = 2_049;
    let valid_rows = height / 2;
    let sample_count = width * height;
    let info = DecodedFrameInfo::new(
        OutputIndex::new(0),
        BitDepth::Eight,
        PixelFormat::Monochrome,
        PlaneSize::new(width, height).expect("frame size"),
        PlaneRect::new(0, 0, width, height).expect("visible rect"),
    )
    .expect("frame info");
    let progress = Arc::new(
        crate::pipeline::frame_progress::FrameProgress::<u8>::new(info).expect("frame progress"),
    );
    assert!(progress.begin(&[(0, height)]));
    let source_samples = vec![73_u8; width * valid_rows];
    let malformed_source = FramePlane {
        width,
        height,
        stride: width,
        origin_y: 0,
        storage_origin_y: 0,
        storage_rows: height,
        samples: &source_samples,
        secondary: &[],
    };
    let mut lease = progress.direct_stripe(0).expect("stripe lease");
    let mut target = lease.take_target().expect("stripe target");

    assert!(
        StripePlane::copy_from_into(malformed_source, 0, height, target.take(PlaneId::Y),).is_err()
    );
    drop(target);
    drop(lease);
    assert_eq!(progress.published_luma_rows(), 0);
    assert!(progress.direct_stripe(0).is_some(), "the lease is reusable");

    let first = take_stripe_sample_buffer(sample_count).expect("recycled failed staging");
    assert_eq!(first.len(), 0);
    let allocation = first.as_ptr();
    recycle_stripe_sample_buffer(first);
    let second = take_stripe_sample_buffer(sample_count).expect("reused staging allocation");
    assert_eq!(second.len(), 0);
    assert_eq!(second.as_ptr(), allocation);
    recycle_stripe_sample_buffer(second);
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

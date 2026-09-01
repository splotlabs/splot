// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Selection and retention policies for the filter-source buffer caches.

#![allow(clippy::expect_used)]

use super::{
    DeblockedSource, FramePlane, StripeOutputPlane, StripePlane, take_stripe_sample_buffer,
    window_bounds,
};
use splot_recon::{
    BitDepth, CurrentFrameWorkspace, DecodedFrameInfo, OutputIndex, PixelFormat, PlaneId,
    PlaneRect, PlaneSize,
};
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
        storage_origin_y: 0,
        storage_rows: 2,
        samples: &source_samples,
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
        storage_origin_y: 0,
        storage_rows: height,
        samples: &source_samples,
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

    let staging = take_stripe_sample_buffer(sample_count).expect("recycled failed staging");
    assert_eq!(staging.len(), 0);
    drop(staging);
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
fn completed_direct_u8_and_staged_fallback_planes_publish_together() {
    let workspace = crate::test_support::yuv420_workspace(8, 8, 91);
    let progress = Arc::new(
        crate::pipeline::frame_progress::FrameProgress::<u8>::new(workspace.info())
            .expect("frame progress"),
    );
    assert!(progress.begin(&[(0, 8)]));
    let mut lease = progress.direct_stripe(0).expect("stripe lease");
    let mut target = lease.take_target().expect("stripe target");

    let y_source = FramePlane::new(&workspace, PlaneId::Y).expect("luma source");
    let mut y =
        StripePlane::copy_from_into(y_source, 0, 8, target.take(PlaneId::Y)).expect("staged luma");
    let u_source = FramePlane::new(&workspace, PlaneId::U).expect("U source");
    let u_reference = StripePlane::copy_from(u_source, 0, 4).expect("U geometry");
    let mut u =
        StripeOutputPlane::direct_u8(target.take(PlaneId::U).expect("U target"), &u_reference)
            .expect("direct U output");
    let v_source = FramePlane::new(&workspace, PlaneId::V).expect("V source");
    let mut v =
        StripePlane::copy_from_into(v_source, 0, 4, target.take(PlaneId::V)).expect("staged V");

    let rect =
        PlaneRect::new(0, 0, u.width(), u.end_y().expect("U stripe end")).expect("U rectangle");
    u.u8_rect_mut(rect).expect("direct U rectangle").0.fill(77);
    y.finish_direct().expect("luma flush");
    v.finish_direct().expect("V flush");
    u.finish_direct().expect("direct U completion");
    drop((y, u, v, target));
    assert!(lease.submit());

    let frame = progress
        .freeze_workspace(core::convert::identity)
        .expect("frozen frame");
    assert!(frame.y().samples().iter().all(|&sample| sample == 91));
    assert!(
        frame
            .u()
            .expect("U plane")
            .samples()
            .iter()
            .all(|&sample| sample == 77)
    );
    assert!(
        frame
            .v()
            .expect("V plane")
            .samples()
            .iter()
            .all(|&sample| sample == 91)
    );
}

#[test]
fn invalid_direct_u8_geometry_drops_without_publication_and_releases_lease() {
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
    let source = StripePlane::from_samples(4, 2, 0, vec![0; 8]).expect("source geometry");
    let mut lease = progress.direct_stripe(0).expect("stripe lease");
    let mut target = lease.take_target().expect("stripe target");
    target.shorten_for_test(PlaneId::Y);

    assert!(
        StripeOutputPlane::direct_u8(target.take(PlaneId::Y).expect("luma target"), &source)
            .is_err()
    );
    drop((target, lease));
    assert_eq!(progress.published_luma_rows(), 0);
    assert!(progress.direct_stripe(0).is_some(), "the lease is reusable");
}

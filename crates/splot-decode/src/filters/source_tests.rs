// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Selection and retention policies for the filter-source buffer caches.

#![allow(clippy::expect_used)]

use super::{DeblockedWindow, FramePlane, StripePlane, take_stripe_sample_buffer, window_bounds};
use splot_recon::{
    BitDepth, CurrentFrameWorkspace, DecodedFrameInfo, OutputIndex, PixelFormat, PlaneId,
    PlaneRect, PlaneSize,
};
use std::sync::Arc;

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
fn stripe_window_includes_reconstructed_padding_beyond_coded_height() {
    let info = workspace(18, 14)
        .info()
        .with_storage_luma_size(PlaneSize::new(24, 16).expect("storage size"))
        .expect("padded frame info");
    let (_progress, mut rows) = crate::test_support::frontier_rows(
        CurrentFrameWorkspace::new(info, 117u16).expect("workspace"),
    );
    let (mut window, mut carry) = (DeblockedWindow::default(), DeblockedWindow::default());
    assert!(rows.publish_final_rows(14));
    assert!(window.fill(&mut rows, &mut carry, (8, 16), 0).is_err());
    assert!(!rows.publish_final_rows(13));
    assert!(rows.publish_final_rows(16));
    window
        .fill(&mut rows, &mut carry, (8, 16), 0)
        .expect("padded stripe window");
    let planes = window.planes().expect("window planes");
    assert_eq!(planes.y.row(15), Some([117; 24].as_slice()));
    assert_eq!(planes.u.expect("chroma").row(7), Some([117; 12].as_slice()));
}

#[test]
fn stripe_windows_cover_first_middle_and_terminal_margins_for_all_formats() {
    let pattern = |y: usize| {
        (0..16)
            .map(|x| ((y * 17 + x * 3) & 255) as u16)
            .collect::<Vec<_>>()
    };
    for format in [
        PixelFormat::Monochrome,
        PixelFormat::Yuv420,
        PixelFormat::Yuv444,
    ] {
        let (_progress, mut rows) =
            crate::test_support::frontier_rows(workspace_with_format(16, 129, format));
        assert!(rows.publish_final_rows(129));
        let mut carry = DeblockedWindow::default();
        for (start, end) in [(0, 56), (56, 120), (120, 129)] {
            let mut window = DeblockedWindow::default();
            window
                .fill(&mut rows, &mut carry, (start, end), 10)
                .expect("final stripe window");
            let planes = window.planes().expect("window planes");
            let expected_y = window_bounds((start, end), 0, 10, 129).expect("luma bounds");
            assert_eq!((planes.y.origin_y(), planes.y.end_y()), expected_y);
            for y in expected_y.0..expected_y.1 {
                assert_eq!(planes.y.row(y), Some(pattern(y).as_slice()));
            }
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
                let width = plane.width();
                assert_eq!(plane.row(expected.0), Some(&pattern(expected.0)[..width]));
            }
        }
    }
}

#[test]
fn unread_window_planes_hold_no_rows_and_a_later_read_fails_closed() {
    let pattern = |y: usize| {
        (0..16)
            .map(|x| ((y * 17 + x * 3) & 255) as u16)
            .collect::<Vec<_>>()
    };
    let stripes = [(0, 56), (56, 120), (120, 129)];
    for slide in [false, true] {
        let (_progress, mut rows) = crate::test_support::frontier_rows(workspace(16, 129));
        assert!(rows.publish_final_rows(129));
        let (mut window, mut carry) = (DeblockedWindow::default(), DeblockedWindow::default());
        for (start, end) in stripes {
            let carry = (!slide).then_some(&mut carry);
            window
                .copy_window(
                    &mut rows,
                    carry,
                    (start, end),
                    2,
                    [true, false, false],
                    [false; 3],
                )
                .expect("luma-only stripe window");
            let planes = window.planes().expect("window planes");
            let (first, last) = window_bounds((start, end), 0, 2, 129).expect("luma bounds");
            for y in first..last {
                assert_eq!(planes.y.row(y), Some(pattern(y).as_slice()));
            }
            for plane in [planes.u.expect("u plane"), planes.v.expect("v plane")] {
                assert!(plane.samples().is_empty());
                assert!((0..65).all(|y| plane.row(y).is_none()));
            }
        }
        let (_progress, mut rows) = crate::test_support::frontier_rows(workspace(16, 129));
        assert!(rows.publish_final_rows(129));
        let (mut window, mut carry) = (DeblockedWindow::default(), DeblockedWindow::default());
        window
            .copy_window(
                &mut rows,
                Some(&mut carry),
                stripes[0],
                2,
                [true, false, false],
                [false; 3],
            )
            .expect("luma-only stripe window");
        let carry = (!slide).then_some(&mut carry);
        assert!(
            window
                .copy_window(&mut rows, carry, stripes[1], 2, [true; 3], [false; 3])
                .is_err()
        );
    }
}

#[test]
fn skipped_window_planes_keep_only_the_rows_the_next_stripe_shares() {
    let pattern = |y: usize| {
        (0..16)
            .map(|x| ((y * 17 + x * 3) & 255) as u16)
            .collect::<Vec<_>>()
    };
    for slide in [false, true] {
        let (_progress, mut rows) = crate::test_support::frontier_rows(workspace(16, 129));
        assert!(rows.publish_final_rows(129));
        let (mut window, mut carry) = (DeblockedWindow::default(), DeblockedWindow::default());
        for ((start, end), skip) in [((0, 56), true), ((56, 120), false), ((120, 129), true)] {
            let carry = (!slide).then_some(&mut carry);
            window
                .copy_window(
                    &mut rows,
                    carry,
                    (start, end),
                    2,
                    [true; 3],
                    [false, skip, skip],
                )
                .expect("stripe window");
            let planes = window.planes().expect("window planes");
            let (first, last) = window_bounds((start, end), 0, 2, 129).expect("luma bounds");
            assert!((first..last).all(|y| planes.y.row(y) == Some(pattern(y).as_slice())));
            let (first, last) = window_bounds((start, end), 1, 2, 65).expect("chroma bounds");
            let tail = ((end >> 1) - 2).clamp(first, last);
            for plane in [planes.u.expect("u plane"), planes.v.expect("v plane")] {
                let held = if skip { tail } else { first };
                assert_eq!((plane.origin_y(), plane.end_y()), (held, last));
                assert!((held..last).all(|y| plane.row(y) == Some(&pattern(y)[..8])));
            }
        }
    }
}

#[test]
fn stripe_rect_mut_rejects_a_rectangle_overhanging_the_row() {
    let mut stripe = StripePlane::from_samples(4, 2, 0, vec![0u16; 8]).expect("a valid stripe");
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

fn monochrome_u8_progress(
    width: usize,
    height: usize,
) -> Arc<crate::pipeline::frame_progress::FrameProgress<u8>> {
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
    progress.begin(&[(0, height)]).expect("stripe geometry");
    progress
}

#[test]
fn u8_direct_stripe_initializes_strided_u8_rows() {
    let progress = monochrome_u8_progress(4, 2);
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

    assert!(output.is_direct());
    output.row_mut(1).expect("second row")[3] = 201;
    drop(output);
    assert!(lease.submit());
    let frame = progress
        .freeze_workspace(core::convert::identity)
        .expect("frozen frame");
    assert_eq!(
        crate::test_support::packed(frame.y()),
        [1, 2, 3, 4, 5, 6, 7, 201]
    );
}

#[test]
fn failed_direct_copies_leave_the_lease_reusable() {
    let (width, height) = (4, 2_049);
    let progress = monochrome_u8_progress(width, height);
    let source_samples = vec![73_u8; width * (height / 2)];
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
        StripePlane::copy_from_into(malformed_source, 0, height, target.take(PlaneId::Y)).is_err()
    );
    drop((target, lease));
    assert!(progress.direct_stripe(0).is_some(), "the lease is reusable");

    let source = StripePlane::from_samples(width, height, 0, vec![0; width * height])
        .expect("source geometry");
    let mut lease = progress.direct_stripe(0).expect("stripe lease");
    let mut target = lease.take_target().expect("stripe target");
    target.shorten_for_test(PlaneId::Y);
    assert!(
        source
            .copy_rows_into(0, height, target.take(PlaneId::Y))
            .is_err()
    );
    drop((target, lease));
    assert_eq!(progress.published_luma_rows(), 0);
    assert!(progress.direct_stripe(0).is_some(), "the lease is reusable");
}

#[test]
fn spent_buffers_return_to_their_own_depth() {
    let ten = take_stripe_sample_buffer::<u16>(4096).expect("u16 buffer");
    let eight = take_stripe_sample_buffer::<u8>(4096).expect("u8 buffer");
    let (ten_at, eight_at) = (ten.as_ptr(), eight.as_ptr());
    drop(StripePlane::from_samples(64, 64, 0, ten));
    drop(StripePlane::from_samples(64, 64, 0, eight));
    let (ten, eight) = (
        take_stripe_sample_buffer::<u16>(16).expect("u16"),
        take_stripe_sample_buffer::<u8>(16).expect("u8"),
    );
    assert_eq!((ten.as_ptr(), eight.as_ptr()), (ten_at, eight_at));
}

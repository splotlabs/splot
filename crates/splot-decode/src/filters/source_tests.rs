// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Selection and retention policies for the filter-source buffer caches.

#![allow(clippy::expect_used)]

use super::{
    DeblockedWindow, FramePlane, StripeOutputPlane, StripePlane, take_stripe_sample_buffer,
    window_bounds,
};
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
    assert!(window.fill(&mut rows, &mut carry, (8, 16), 0).is_none());
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
    let staging = y.samples().as_ptr();
    drop((y, u, v, target));
    assert!(super::STRIPE_STAGING.with(|slots| {
        slots
            .borrow()
            .iter()
            .any(|buffer| buffer.as_ptr() == staging)
    }));
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

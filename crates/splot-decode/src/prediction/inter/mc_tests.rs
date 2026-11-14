// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::expect_used, clippy::panic)]

use super::*;
use crate::prediction::inter::mv_scaling::derive_plane_scaling;
use splot_recon::{
    DecodedFrame, DecodedFrameInfo, FramePlanes, OutputIndex, PixelFormat, Plane, PlaneSize,
    wedge_mask_plane_sample,
};

fn compound_recycler_box<T: 'static>() -> Option<*const ()> {
    MC_SAMPLES_RECYCLER.with(|cell| {
        cell.borrow()
            .iter()
            .flatten()
            .find(|any| any.is::<Vec<T>>())
            .map(|any| std::ptr::from_ref::<dyn std::any::Any>(&**any).cast::<()>())
    })
}

#[test]
fn motion_compensation_planes_follow_output_chroma_format() {
    assert_eq!(
        mc_planes(PixelFormat::Yuv420),
        [(PlaneId::Y, 0, 0), (PlaneId::U, 1, 1), (PlaneId::V, 1, 1)]
    );
    assert_eq!(
        mc_planes(PixelFormat::Yuv422),
        [(PlaneId::Y, 0, 0), (PlaneId::U, 1, 0), (PlaneId::V, 1, 0)]
    );
    assert_eq!(
        mc_planes(PixelFormat::Yuv444),
        [(PlaneId::Y, 0, 0), (PlaneId::U, 0, 0), (PlaneId::V, 0, 0)]
    );
}

#[test]
fn intrabc_integer_copy_clips_bottom_edge_to_coded_storage() {
    let mut workspace = workspace(8, 6);
    let source = PlaneRect::new(0, 0, 4, 4).expect("source rect");
    let target = PlaneRect::new(0, 4, 4, 4).expect("nominal target rect");
    let samples = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
    workspace
        .write_rect(PlaneId::Y, source, &samples, 4)
        .expect("source samples");

    intrabc_copy_plane_into(&mut workspace, PlaneId::Y, source, target)
        .expect("edge-clipped IntraBC copy");

    let stride = workspace.plane(PlaneId::Y).expect("luma").stride_samples();
    let y = workspace.samples(PlaneId::Y).expect("luma samples");
    assert_eq!(&y[4 * stride..4 * stride + 4], [1, 2, 3, 4]);
    assert_eq!(&y[5 * stride..5 * stride + 4], [5, 6, 7, 8]);
}

/// A bilinear IntraBC prediction reads the row below only at a non-zero
/// vertical phase, and its reads stay clipped to the plane.
#[test]
fn intrabc_bilinear_rows_cover_only_weighted_rows() {
    let params = |start_y, h| splot_recon::SubpelPredictParams {
        interp: splot_recon::InterpolationFilter::Bilinear,
        w: 8,
        h,
        start_x: 0,
        start_y,
        step_x: 1 << 10,
        step_y: 1 << 10,
        first_x: 0,
        first_y: 0,
        last_x: 63,
        last_y: 63,
        bit_depth: splot_recon::BitDepth::Ten,
    };
    assert_eq!(
        crate::filters::wienerns_lr::recon::full_recon::intrabc_bilinear_rows(&params(16 << 10, 8)),
        (16, 23)
    );
    assert_eq!(
        crate::filters::wienerns_lr::recon::full_recon::intrabc_bilinear_rows(&params(
            (16 << 10) + 512,
            8
        )),
        (16, 24)
    );
    assert_eq!(
        crate::filters::wienerns_lr::recon::full_recon::intrabc_bilinear_rows(&params(
            (60 << 10) + 512,
            8
        )),
        (60, 63)
    );
}

#[test]
fn compound_sample_recycler_keeps_its_box_and_sample_storage() {
    MC_SAMPLES_RECYCLER.with(|cell| *cell.borrow_mut() = [None, None]);

    let mut samples = take_mc_samples::<u16>();
    samples.resize(64, 0);
    let storage = samples.as_ptr();
    recycle_mc_samples(&mut samples);
    assert!(samples.is_empty());
    let recycler_box = compound_recycler_box::<u16>().expect("u16 recycler box");

    let mut reused = take_mc_samples::<u16>();
    assert_eq!(reused.as_ptr(), storage);
    assert_eq!(compound_recycler_box::<u16>(), Some(recycler_box));
    recycle_mc_samples(&mut reused);

    let mut u8_samples = take_mc_samples::<u8>();
    u8_samples.resize(32, 0);
    let u8_storage = u8_samples.as_ptr();
    recycle_mc_samples(&mut u8_samples);
    let mut u16_samples = take_mc_samples::<u16>();
    let mut u8_samples = take_mc_samples::<u8>();
    assert_eq!(u16_samples.as_ptr(), storage);
    assert_eq!(u8_samples.as_ptr(), u8_storage);
    recycle_mc_samples(&mut u16_samples);
    recycle_mc_samples(&mut u8_samples);

    MC_SAMPLES_RECYCLER.with(|cell| *cell.borrow_mut() = [None, None]);
}

#[test]
fn compound_sample_recycler_keeps_two_same_type_buffers() {
    MC_SAMPLES_RECYCLER.with(|cell| *cell.borrow_mut() = [None, None]);
    let mut first = take_mc_samples::<u16>();
    let mut second = take_mc_samples::<u16>();
    first.resize(64, 0);
    second.resize(32, 0);
    let pointers = [first.as_ptr(), second.as_ptr()];
    recycle_mc_samples(&mut first);
    recycle_mc_samples(&mut second);

    let mut reused_first = take_mc_samples::<u16>();
    let mut reused_second = take_mc_samples::<u16>();
    assert_eq!([reused_first.as_ptr(), reused_second.as_ptr()], pointers);
    recycle_mc_samples(&mut reused_first);
    recycle_mc_samples(&mut reused_second);
    MC_SAMPLES_RECYCLER.with(|cell| *cell.borrow_mut() = [None, None]);
}

#[test]
fn mc_worker_scratch_reuses_all_installed_buffers() {
    COMPOUND_PREDICTION_BUFFERS.with(|slot| slot.set(None));
    INITIAL_LUMA_PREDICTIONS.with(|slot| slot.set(None));
    SUBPEL_PREDICTION_BUFFER.with(|slot| slot.set(None));
    MC_SAMPLES_RECYCLER.with(|slot| *slot.borrow_mut() = [None, None]);
    let mut scratch = McScratch::default();
    let use_buffers = || {
        let compound = COMPOUND_PREDICTION_BUFFERS.with(|slot| {
            let mut buffers = slot.take().unwrap_or_default();
            buffers[0].resize(64, 0);
            let pointer = buffers[0].as_ptr() as usize;
            slot.set(Some(buffers));
            pointer
        });
        let initial = INITIAL_LUMA_PREDICTIONS.with(|slot| {
            let mut samples = slot.take().unwrap_or_default();
            samples.resize(64, 0);
            let pointer = samples.as_ptr() as usize;
            slot.set(Some(samples));
            pointer
        });
        let subpel = SUBPEL_PREDICTION_BUFFER.with(|slot| {
            let mut samples = slot.take().unwrap_or_default();
            samples.resize(64, 0);
            let pointer = samples.as_ptr() as usize;
            slot.set(Some(samples));
            pointer
        });
        let mut output = take_mc_samples::<u16>();
        output.resize(64, 0);
        let output_pointer = output.as_ptr() as usize;
        recycle_mc_samples(&mut output);
        [compound, initial, subpel, output_pointer]
    };

    let first = scratch.with_installed(use_buffers);
    let second = scratch.with_installed(use_buffers);

    assert_eq!(second, first);
    COMPOUND_PREDICTION_BUFFERS.with(|slot| slot.set(None));
    INITIAL_LUMA_PREDICTIONS.with(|slot| slot.set(None));
    SUBPEL_PREDICTION_BUFFER.with(|slot| slot.set(None));
    MC_SAMPLES_RECYCLER.with(|slot| *slot.borrow_mut() = [None, None]);
}

#[test]
fn nested_mc_install_keeps_the_outer_worker_buffers_active() {
    COMPOUND_PREDICTION_BUFFERS.with(|slot| slot.set(None));
    INITIAL_LUMA_PREDICTIONS.with(|slot| slot.set(None));
    SUBPEL_PREDICTION_BUFFER.with(|slot| slot.set(None));
    MC_SAMPLES_RECYCLER.with(|slot| *slot.borrow_mut() = [None, None]);
    let mut outer = McScratch::default();
    let mut nested = McScratch::default();

    let pointers = outer.with_installed(|| {
        let outer_pointer = COMPOUND_PREDICTION_BUFFERS.with(|slot| {
            slot.take()
                .map(|buffers| {
                    let pointer = buffers[0].as_ptr();
                    slot.set(Some(buffers));
                    pointer
                })
                .expect("outer compound buffers")
        });
        let nested_pointer = nested.with_installed(|| {
            COMPOUND_PREDICTION_BUFFERS.with(|slot| {
                slot.take()
                    .map(|buffers| {
                        let pointer = buffers[0].as_ptr();
                        slot.set(Some(buffers));
                        pointer
                    })
                    .expect("nested compound buffers")
            })
        });
        (outer_pointer, nested_pointer)
    });

    assert_eq!(pointers.0, pointers.1);
    MC_INSTALL_DEPTH.with(|depth| assert_eq!(depth.get(), 0));
    COMPOUND_PREDICTION_BUFFERS.with(|slot| slot.set(None));
    INITIAL_LUMA_PREDICTIONS.with(|slot| slot.set(None));
    SUBPEL_PREDICTION_BUFFER.with(|slot| slot.set(None));
    MC_SAMPLES_RECYCLER.with(|slot| *slot.borrow_mut() = [None, None]);
}

#[test]
fn panicking_mc_install_restores_worker_buffers_and_depth() {
    COMPOUND_PREDICTION_BUFFERS.with(|slot| slot.set(None));
    INITIAL_LUMA_PREDICTIONS.with(|slot| slot.set(None));
    SUBPEL_PREDICTION_BUFFER.with(|slot| slot.set(None));
    MC_SAMPLES_RECYCLER.with(|slot| *slot.borrow_mut() = [None, None]);
    let mut scratch = McScratch::default();

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        scratch.with_installed(|| panic!("installed MC panic"));
    }));

    assert!(result.is_err());
    MC_INSTALL_DEPTH.with(|depth| assert_eq!(depth.get(), 0));
    let buffers_restored = scratch.with_installed(|| {
        COMPOUND_PREDICTION_BUFFERS.with(|slot| {
            let buffers = slot.take().expect("restored compound buffers");
            let restored = buffers[0].capacity() == MAX_MC_BLOCK_SAMPLES;
            slot.set(Some(buffers));
            restored
        })
    });
    assert!(buffers_restored);
    MC_INSTALL_DEPTH.with(|depth| assert_eq!(depth.get(), 0));
    COMPOUND_PREDICTION_BUFFERS.with(|slot| slot.set(None));
    INITIAL_LUMA_PREDICTIONS.with(|slot| slot.set(None));
    SUBPEL_PREDICTION_BUFFER.with(|slot| slot.set(None));
    MC_SAMPLES_RECYCLER.with(|slot| *slot.borrow_mut() = [None, None]);
}

#[test]
fn dispatcher_copies_non420_reference_planes() {
    for format in [PixelFormat::Yuv422, PixelFormat::Yuv444] {
        let reference = flat_frame_with_format(format, 8, 8, 40, 90, 120);
        let mut workspace = workspace_with_format(format, 8, 8);

        motion_compensate_inter_block_into(
            &mut super::WorkspaceSink::Frame(&mut workspace),
            InterBlockParams::single(
                ReferenceSamples::settled(&reference),
                McBlockRect::from_luma_rect(0, 0, 8, 8),
                Mv::ZERO,
                InterpolationFilter::EightTap,
            ),
            ByteOffset::new(0),
        )
        .expect("non-4:2:0 single-reference dispatcher");

        let decoded = workspace.freeze().expect("freeze non-4:2:0 workspace");
        for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
            assert_eq!(
                visible_samples(&decoded, plane),
                visible_samples(&reference, plane),
                "format={format:?} plane={plane:?}"
            );
        }
    }
}

#[test]
fn compound_and_warp_copy_non420_chroma_at_native_resolution() {
    for format in [PixelFormat::Yuv422, PixelFormat::Yuv444] {
        let reference = flat_frame_with_format(format, 8, 8, 40, 90, 120);
        for params in [
            InterBlockParams::compound_average(
                ReferenceSamples::settled(&reference),
                ReferenceSamples::settled(&reference),
                McBlockRect::from_luma_rect(0, 0, 8, 8),
                Mv::ZERO,
                Mv::ZERO,
                InterpolationFilter::EightTap,
                CompoundBlend::default(),
            ),
            InterBlockParams::single_warp(
                ReferenceSamples::settled(&reference),
                McBlockRect::from_luma_rect(0, 0, 8, 8),
                Mv::ZERO,
                InterpolationFilter::EightTap,
                crate::prediction::inter::find_mv_stack::DEFAULT_WARP_PARAMS,
            ),
        ] {
            let mut workspace = workspace_with_format(format, 8, 8);
            motion_compensate_inter_block_into(
                &mut super::WorkspaceSink::Frame(&mut workspace),
                params,
                ByteOffset::new(0),
            )
            .expect("non-4:2:0 compound or warp dispatcher");

            let decoded = workspace.freeze().expect("freeze non-4:2:0 workspace");
            for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
                assert_eq!(
                    visible_samples(&decoded, plane),
                    visible_samples(&reference, plane),
                    "format={format:?} plane={plane:?}"
                );
            }
        }
    }
}

#[test]
fn dispatcher_zero_mv_copies_single_reference_planes() {
    let reference = patterned_frame(8, 8);
    let mut workspace = workspace(8, 8);

    motion_compensate_inter_block_into(
        &mut super::WorkspaceSink::Frame(&mut workspace),
        InterBlockParams::single(
            ReferenceSamples::settled(&reference),
            McBlockRect::from_luma_rect(0, 0, 8, 8),
            Mv { row: 0, col: 0 },
            InterpolationFilter::EightTap,
        ),
        ByteOffset::new(0),
    )
    .expect("single-reference dispatcher");

    let decoded = workspace.freeze().expect("freeze dispatched workspace");
    assert_eq!(
        visible_samples(&decoded, PlaneId::Y),
        visible_samples(&reference, PlaneId::Y)
    );
    assert_eq!(
        visible_samples(&decoded, PlaneId::U),
        visible_samples(&reference, PlaneId::U)
    );
    assert_eq!(
        visible_samples(&decoded, PlaneId::V),
        visible_samples(&reference, PlaneId::V)
    );
}

#[test]
fn dispatcher_zero_mv_copies_odd_reference_chroma_extents() {
    let reference = patterned_frame(9, 11);
    let mut workspace = workspace(9, 11);

    motion_compensate_inter_block_into(
        &mut super::WorkspaceSink::Frame(&mut workspace),
        InterBlockParams::single(
            ReferenceSamples::settled(&reference),
            McBlockRect::from_luma_rect(0, 0, 9, 11),
            Mv::ZERO,
            InterpolationFilter::EightTap,
        ),
        ByteOffset::new(0),
    )
    .expect("odd single-reference dispatcher");

    let decoded = workspace.freeze().expect("freeze odd workspace");
    for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
        assert_eq!(
            visible_samples(&decoded, plane),
            visible_samples(&reference, plane)
        );
    }
}

#[test]
fn dispatcher_clips_single_prediction_at_frame_edge() {
    let reference = patterned_frame(8, 8);
    let mut workspace = workspace(8, 8);

    motion_compensate_inter_block_into(
        &mut WorkspaceSink::Frame(&mut workspace),
        InterBlockParams::single(
            ReferenceSamples::settled(&reference),
            McBlockRect::from_luma_rect(4, 4, 8, 8),
            Mv::ZERO,
            InterpolationFilter::EightTap,
        ),
        ByteOffset::new(0),
    )
    .expect("edge-clipped single-reference dispatcher");

    let decoded = workspace.freeze().expect("freeze edge-clipped workspace");
    for (plane, width, start) in [(PlaneId::Y, 8, 4), (PlaneId::U, 4, 2), (PlaneId::V, 4, 2)] {
        let reference_samples = visible_samples(&reference, plane);
        let mut expected = vec![0; reference_samples.len()];
        for y in start..width {
            for x in start..width {
                expected[y * width + x] = reference_samples[y * width + x];
            }
        }
        assert_eq!(visible_samples(&decoded, plane), expected);
    }
}

#[test]
fn converted_prediction_write_is_fail_atomic() {
    let mut workspace = workspace(8, 8);
    let mut samples = vec![7; 64];
    samples[63] = 256;

    let err = WorkspaceSink::Frame(&mut workspace)
        .write_u16_rect(
            PlaneId::Y,
            PlaneRect::new(0, 0, 8, 8).expect("full plane"),
            &samples,
            8,
        )
        .expect_err("unrepresentable late sample must reject the whole write");
    assert!(matches!(
        err,
        ReconError::SampleValueUnsupportedStorage { value: 256, .. }
    ));
    assert!(
        workspace
            .rect_rows(PlaneId::Y, PlaneRect::new(0, 0, 8, 8).expect("full plane"))
            .expect("untouched luma rows")
            .flatten()
            .all(|&sample| sample == 0)
    );

    let luma_size = PlaneSize::new(8, 8).expect("luma size");
    let visible = PlaneRect::new(0, 0, 8, 8).expect("visible rect");
    let info = DecodedFrameInfo::new(
        OutputIndex::new(0),
        BitDepth::Ten,
        PixelFormat::Yuv420,
        luma_size,
        visible,
    )
    .expect("ten-bit frame info");
    let mut workspace = CurrentFrameWorkspace::<u16>::new(info, 0).expect("ten-bit workspace");
    let plane_rect = PlaneRect::new(0, 0, 8, 8).expect("full plane");
    let mut samples = vec![900; 64];
    samples[63] = 1024;

    let err = WorkspaceSink::Frame(&mut workspace)
        .write_u16_rect(PlaneId::Y, plane_rect, &samples, 8)
        .expect_err("out-of-range late sample must reject the whole write");
    assert!(matches!(
        err,
        ReconError::SampleOutOfRange {
            sample_index: 63,
            value: 1024,
            max: 1023,
            ..
        }
    ));
    assert!(
        workspace
            .rect_rows(PlaneId::Y, plane_rect)
            .expect("untouched ten-bit luma rows")
            .flatten()
            .all(|&sample| sample == 0)
    );
}

#[test]
fn dispatcher_blends_compound_average_planes() {
    let reference0 = flat_frame(8, 8, 40, 90, 120);
    let reference1 = flat_frame(8, 8, 80, 110, 140);
    let samples = dispatch_compound_samples(
        ReferenceSamples::settled(&reference0),
        ReferenceSamples::settled(&reference1),
        CompoundBlend::default(),
    );
    assert_eq!(samples.0, vec![60; 64]);
    assert_eq!(samples.1, vec![100; 16]);
    assert_eq!(samples.2, vec![130; 16]);
}

#[test]
fn dispatcher_blends_compound_average_with_odd_chroma_extents() {
    let reference0 = flat_frame(9, 11, 40, 90, 120);
    let reference1 = flat_frame(9, 11, 80, 110, 140);
    let mut workspace = workspace(9, 11);

    motion_compensate_inter_block_into(
        &mut super::WorkspaceSink::Frame(&mut workspace),
        InterBlockParams::compound_average(
            ReferenceSamples::settled(&reference0),
            ReferenceSamples::settled(&reference1),
            McBlockRect::from_luma_rect(0, 0, 9, 11),
            Mv::ZERO,
            Mv::ZERO,
            InterpolationFilter::EightTap,
            CompoundBlend::default(),
        ),
        ByteOffset::new(0),
    )
    .expect("odd compound dispatcher");

    let decoded = workspace.freeze().expect("freeze odd compound workspace");
    assert_eq!(visible_samples(&decoded, PlaneId::Y), vec![60; 9 * 11]);
    assert_eq!(visible_samples(&decoded, PlaneId::U), vec![100; 5 * 6]);
    assert_eq!(visible_samples(&decoded, PlaneId::V), vec![130; 5 * 6]);
}

#[test]
fn single_reference_u8_frame_and_band_surfaces_match_across_chroma_formats_and_edges() {
    for format in [
        PixelFormat::Yuv420,
        PixelFormat::Yuv422,
        PixelFormat::Yuv444,
    ] {
        let width = 17;
        let height = 15;
        let reference = patterned_frame_with_format(format, width, height, 23);
        for (block_rect, mv, interp) in [
            (
                McBlockRect::from_luma_rect(0, 0, width, height),
                Mv::ZERO,
                InterpolationFilter::EightTap,
            ),
            (
                McBlockRect::from_luma_rect(0, 1, width, height - 1),
                Mv { row: 5, col: -3 },
                InterpolationFilter::EightTapSharp,
            ),
        ] {
            SUBPEL_PREDICTION_BUFFER.with(|slot| slot.set(None));
            let block = InterBlockParams::single(
                ReferenceSamples::settled(&reference),
                block_rect,
                mv,
                interp,
            );
            let mut direct = workspace_with_format(format, width, height);
            let mut staged = workspace_with_format(format, width, height);
            let luma_rect = PlaneRect::new(
                block_rect.luma_x,
                block_rect.luma_y,
                block_rect.luma_w,
                block_rect.luma_h,
            )
            .expect("surface rectangle");
            {
                let mut surfaces = staged
                    .rect_surfaces(&[luma_rect])
                    .expect("rectangle surface");
                let surface = surfaces.first_mut().expect("one rectangle surface");
                for mut sink in [
                    WorkspaceSink::Frame(&mut direct),
                    WorkspaceSink::Rect(surface),
                ] {
                    motion_compensate_inter_block_into(&mut sink, block, ByteOffset::new(0))
                        .expect("u8 single-reference prediction");
                    SUBPEL_PREDICTION_BUFFER.with(|slot| {
                        assert!(slot.take().is_none(), "surface used packed u16 storage");
                    });
                }
            }

            let direct = direct.freeze().expect("freeze direct workspace");
            let staged = staged.freeze().expect("freeze staged workspace");
            for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
                assert_eq!(
                    visible_samples(&direct, plane),
                    visible_samples(&staged, plane),
                    "{format:?} {plane:?} {block_rect:?} {mv:?}"
                );
            }
        }
    }
    SUBPEL_PREDICTION_BUFFER.with(|slot| slot.set(None));
}

#[test]
fn a_frame_edge_overhanging_eight_bit_block_stages_as_bytes() {
    let width = 17;
    let height = 15;
    let reference = patterned_frame_with_format(PixelFormat::Yuv420, width, height, 23);
    let block_rect = McBlockRect::from_luma_rect(12, 10, 8, 8);
    let block = InterBlockParams::single(
        ReferenceSamples::settled(&reference),
        block_rect,
        Mv { row: 5, col: -3 },
        InterpolationFilter::EightTapSharp,
    );

    SUBPEL_PREDICTION_BUFFER.with(|slot| slot.set(None));
    let mut workspace = workspace_with_format(PixelFormat::Yuv420, width, height);
    motion_compensate_inter_block_into(
        &mut super::WorkspaceSink::Frame(&mut workspace),
        block,
        ByteOffset::new(0),
    )
    .expect("clipped u8 single-reference prediction");

    SUBPEL_PREDICTION_BUFFER.with(|slot| {
        assert!(
            slot.take().is_none(),
            "a clipped eight-bit rectangle staged through u16 storage"
        );
    });
}

#[test]
fn dispatcher_sub8x8_chroma_uses_only_the_first_reference() {
    let reference0 = flat_frame(8, 8, 40, 90, 120);
    let reference1 = flat_frame(8, 8, 80, 110, 140);
    let mut workspace = workspace(8, 8);

    motion_compensate_inter_block_into(
        &mut super::WorkspaceSink::Frame(&mut workspace),
        InterBlockParams::compound_average(
            ReferenceSamples::settled(&reference0),
            ReferenceSamples::settled(&reference1),
            McBlockRect::from_luma_rect(0, 0, 8, 8),
            Mv::ZERO,
            Mv::ZERO,
            InterpolationFilter::EightTap,
            CompoundBlend::default(),
        )
        .with_sub8x8_chroma(true),
        ByteOffset::new(0),
    )
    .expect("sub8x8 chroma dispatcher");

    let decoded = workspace.freeze().expect("freeze sub8x8 workspace");
    assert_eq!(visible_samples(&decoded, PlaneId::Y), vec![60; 64]);
    assert_eq!(visible_samples(&decoded, PlaneId::U), vec![90; 16]);
    assert_eq!(visible_samples(&decoded, PlaneId::V), vec![120; 16]);
}

#[test]
fn dispatcher_sub8x8_chroma_drops_warp() {
    let reference = patterned_frame(16, 16);
    let mut warped = workspace(16, 16);
    let mut translational = workspace(16, 16);
    let rect = McBlockRect::from_luma_rect(0, 0, 8, 8);
    let mv = Mv::ZERO;

    motion_compensate_inter_block_into(
        &mut super::WorkspaceSink::Frame(&mut warped),
        InterBlockParams::single_warp(
            ReferenceSamples::settled(&reference),
            rect,
            mv,
            InterpolationFilter::EightTap,
            [1 << 16, 0, 1 << 16, 0, 0, 1 << 16],
        )
        .with_sub8x8_chroma(true),
        ByteOffset::new(0),
    )
    .expect("sub8x8 chroma warp dispatcher");
    motion_compensate_inter_block_into(
        &mut super::WorkspaceSink::Frame(&mut translational),
        InterBlockParams::single(
            ReferenceSamples::settled(&reference),
            rect,
            mv,
            InterpolationFilter::EightTap,
        ),
        ByteOffset::new(0),
    )
    .expect("translational dispatcher");

    let warped = warped.freeze().expect("freeze warped workspace");
    let translational = translational
        .freeze()
        .expect("freeze translational workspace");
    assert_ne!(
        visible_samples(&warped, PlaneId::Y),
        visible_samples(&translational, PlaneId::Y)
    );
    for plane in [PlaneId::U, PlaneId::V] {
        assert_eq!(
            visible_samples(&warped, plane),
            visible_samples(&translational, plane)
        );
    }
}

#[test]
fn dispatcher_blends_compound_average_with_cwp_weight() {
    let reference0 = flat_frame(8, 8, 40, 90, 120);
    let reference1 = flat_frame(8, 8, 80, 110, 140);
    let samples = dispatch_compound_samples(
        ReferenceSamples::settled(&reference0),
        ReferenceSamples::settled(&reference1),
        CompoundBlend::default().average_with_cwp_weight(12),
    );
    assert_eq!(samples.0, vec![50; 64]);
    assert_eq!(samples.1, vec![95; 16]);
    assert_eq!(samples.2, vec![125; 16]);
}

#[test]
fn dispatcher_rebuilds_optflow_compound_planes_from_refined_grid() {
    let reference0 = flat_frame(8, 8, 40, 90, 120);
    let reference1 = flat_frame(8, 8, 80, 110, 140);
    let mut workspace = workspace(8, 8);
    motion_compensate_inter_block_into(
        &mut super::WorkspaceSink::Frame(&mut workspace),
        InterBlockParams::compound_average(
            ReferenceSamples::settled(&reference0),
            ReferenceSamples::settled(&reference1),
            McBlockRect::from_luma_rect(0, 0, 8, 8),
            Mv::ZERO,
            Mv::ZERO,
            InterpolationFilter::EightTap,
            CompoundBlend::default(),
        )
        .with_optflow_distances(Some([1, -1])),
        ByteOffset::new(0),
    )
    .expect("optical-flow compound dispatcher");

    let decoded = workspace.freeze().expect("freeze optical-flow workspace");
    assert_eq!(visible_samples(&decoded, PlaneId::Y), vec![60; 64]);
    assert_eq!(visible_samples(&decoded, PlaneId::U), vec![100; 16]);
    assert_eq!(visible_samples(&decoded, PlaneId::V), vec![130; 16]);
}

#[test]
fn optflow_reuses_the_luma_motion_grid_for_every_chroma_format() {
    for format in [
        PixelFormat::Yuv420,
        PixelFormat::Yuv422,
        PixelFormat::Yuv444,
    ] {
        let reference0 = flat_frame_with_format(format, 8, 8, 40, 90, 120);
        let reference1 = flat_frame_with_format(format, 8, 8, 80, 110, 140);
        let mut workspace = workspace_with_format(format, 8, 8);
        motion_compensate_inter_block_into(
            &mut super::WorkspaceSink::Frame(&mut workspace),
            InterBlockParams::compound_average(
                ReferenceSamples::settled(&reference0),
                ReferenceSamples::settled(&reference1),
                McBlockRect::from_luma_rect(0, 0, 8, 8),
                Mv::ZERO,
                Mv::ZERO,
                InterpolationFilter::EightTapSharp,
                CompoundBlend::default(),
            )
            .with_optflow_distances(Some([1, -1])),
            ByteOffset::new(0),
        )
        .expect("optical-flow chroma dispatcher");

        let decoded = workspace.freeze().expect("freeze optical-flow workspace");
        assert_eq!(visible_samples(&decoded, PlaneId::Y), vec![60; 64]);
        let chroma_len = 64 >> (format.subsampling_x() + format.subsampling_y());
        assert_eq!(visible_samples(&decoded, PlaneId::U), vec![100; chroma_len]);
        assert_eq!(visible_samples(&decoded, PlaneId::V), vec![130; chroma_len]);
    }
}

#[test]
fn optflow_derivation_is_independent_of_the_final_interpolation_filter() {
    let width = 32;
    let height = 32;
    let pattern = |x: usize, y: usize| ((x * 19 + y * 37 + x * y * 3) % 251) as u8;
    let reference0_y: Vec<u8> = (0..height)
        .flat_map(|y| (0..width).map(move |x| pattern(x, y)))
        .collect();
    let reference1_y: Vec<u8> = (0..height)
        .flat_map(|y| (0..width).map(move |x| pattern((x + 2).min(width - 1), y)))
        .collect();
    let chroma = vec![128; width.div_ceil(2) * height.div_ceil(2)];
    let reference0 = frame(width, height, reference0_y, chroma.clone(), chroma.clone());
    let reference1 = frame(width, height, reference1_y, chroma.clone(), chroma);
    let derive = |interp| {
        let mut workspace = workspace(width, height);
        refined_units(
            &super::WorkspaceSink::Frame(&mut workspace),
            InterBlockParams::compound_average(
                ReferenceSamples::settled(&reference0),
                ReferenceSamples::settled(&reference1),
                McBlockRect::from_luma_rect(8, 8, 8, 8),
                Mv { row: 3, col: 5 },
                Mv { row: -5, col: -3 },
                interp,
                CompoundBlend::default(),
            )
            .with_optflow_distances(Some([1, -1])),
        )
        .cell_mvs(0)
        .expect("optical-flow motion cell")
    };

    assert_eq!(
        derive(InterpolationFilter::EightTapSharp),
        derive(InterpolationFilter::Bilinear)
    );
}

#[test]
fn dispatcher_returns_tip_output_optflow_mvs_for_storage() {
    let reference0 = flat_frame(8, 8, 40, 90, 120);
    let reference1 = flat_frame(8, 8, 80, 110, 140);
    let mut workspace = workspace(8, 8);
    let grid = refined_units(
        &super::WorkspaceSink::Frame(&mut workspace),
        InterBlockParams::compound_average(
            ReferenceSamples::settled(&reference0),
            ReferenceSamples::settled(&reference1),
            McBlockRect::from_luma_rect(0, 0, 8, 8),
            Mv::ZERO,
            Mv::ZERO,
            InterpolationFilter::EightTap,
            CompoundBlend::default(),
        )
        .with_optflow_distances(Some([1, -1])),
    );

    assert_eq!(
        grid.temporal_mvs_at_luma_offset(0, 0)
            .expect("stored motion vectors"),
        [Mv::ZERO; 2]
    );
}

/// The refined units [`refine_block_units`] derives for the compound `block`.
fn refined_units<T: ReconSample>(
    sink: &WorkspaceSink<'_, '_, T>,
    block: InterBlockParams<'_, T>,
) -> RefinedUnits {
    let compound = block.into_compound().expect("compound block");
    let mut units = RefinedUnits::default();
    refine_block_units(sink, &compound, &mut units, ByteOffset::new(0)).expect("refined units");
    units
}

#[test]
fn dispatcher_returns_default_refinemv_motion_grid() {
    let width = 32;
    let height = 32;
    let pattern = |x: usize, y: usize| ((x * 19 + y * 37 + x * y * 3) % 251) as u8;
    let reference0_y: Vec<u8> = (0..height)
        .flat_map(|y| (0..width).map(move |x| pattern(x, y)))
        .collect();
    let reference1_y: Vec<u8> = (0..height)
        .flat_map(|y| (0..width).map(move |x| pattern((x + 2).min(width - 1), y)))
        .collect();
    let chroma = vec![128; width.div_ceil(2) * height.div_ceil(2)];
    let reference0 = frame(width, height, reference0_y, chroma.clone(), chroma.clone());
    let reference1 = frame(width, height, reference1_y, chroma.clone(), chroma);
    let mut workspace = workspace(width, height);

    let grid = refined_units(
        &super::WorkspaceSink::Frame(&mut workspace),
        InterBlockParams::compound_average(
            ReferenceSamples::settled(&reference0),
            ReferenceSamples::settled(&reference1),
            McBlockRect::from_luma_rect(8, 8, 16, 16),
            Mv::ZERO,
            Mv::ZERO,
            InterpolationFilter::EightTapSharp,
            CompoundBlend::default(),
        )
        .with_refinemv(true),
    );

    assert_eq!(
        grid.temporal_mvs_at_luma_offset(0, 0)
            .expect("stored refine-MVs"),
        [Mv { row: 0, col: 8 }, Mv { row: 0, col: -8 }]
    );
}

#[test]
fn dispatcher_switchable_refinemv_excludes_low_sad_center() {
    let width = 32;
    let height = 32;
    let reference0 = flat_frame(width, height, 80, 128, 128);
    let reference1 = flat_frame(width, height, 80, 128, 128);
    let mut workspace = workspace(width, height);

    let grid = refined_units(
        &super::WorkspaceSink::Frame(&mut workspace),
        InterBlockParams::compound_average(
            ReferenceSamples::settled(&reference0),
            ReferenceSamples::settled(&reference1),
            McBlockRect::from_luma_rect(8, 8, 16, 16),
            Mv::ZERO,
            Mv::ZERO,
            InterpolationFilter::EightTapSharp,
            CompoundBlend::default(),
        )
        .with_refinemv(true)
        .with_switchable_refinemv(true),
    );

    assert_eq!(
        grid.temporal_mvs_at_luma_offset(0, 0)
            .expect("stored switchable refine-MVs"),
        [Mv { row: -16, col: -16 }, Mv { row: 16, col: 16 }]
    );
}

#[test]
fn dispatcher_skips_refinemv_search_without_disabling_refinemv() {
    let width = 32;
    let height = 32;
    let pattern = |x: usize, y: usize| ((x * 19 + y * 37 + x * y * 3) % 251) as u8;
    let reference0_y: Vec<u8> = (0..height)
        .flat_map(|y| (0..width).map(move |x| pattern(x, y)))
        .collect();
    let reference1_y: Vec<u8> = (0..height)
        .flat_map(|y| (0..width).map(move |x| pattern((x + 2).min(width - 1), y)))
        .collect();
    let chroma = vec![128; width.div_ceil(2) * height.div_ceil(2)];
    let reference0 = frame(width, height, reference0_y, chroma.clone(), chroma.clone());
    let reference1 = frame(width, height, reference1_y, chroma.clone(), chroma);
    let mut workspace = workspace(width, height);

    let grid = refined_units(
        &super::WorkspaceSink::Frame(&mut workspace),
        InterBlockParams::compound_average(
            ReferenceSamples::settled(&reference0),
            ReferenceSamples::settled(&reference1),
            McBlockRect::from_luma_rect(8, 8, 16, 16),
            Mv::ZERO,
            Mv::ZERO,
            InterpolationFilter::EightTapSharp,
            CompoundBlend::default(),
        )
        .with_refinemv(true)
        .with_refinemv_search(false),
    );

    assert_eq!(
        grid.temporal_mvs_at_luma_offset(0, 0)
            .expect("stored refine-MVs"),
        [Mv::ZERO, Mv::ZERO]
    );
}

#[test]
fn chroma_geometry_rounds_odd_luma_extents_up() {
    assert_eq!(
        McBlockRect::from_luma_rect(0, 0, 17, 19).plane_rect(PlaneId::U, 1, 1),
        (0, 0, 9, 10)
    );
    assert_eq!(
        McBlockRect::from_luma_rect(1, 1, 8, 8).plane_rect(PlaneId::V, 1, 1),
        (0, 0, 5, 5)
    );
}

#[test]
fn dispatcher_subsamples_luma_diff_weighted_mask_for_chroma() {
    let reference0 = flat_frame(8, 8, 100, 0, 0);
    let reference1 = flat_frame(8, 8, 100, 200, 200);
    let samples = dispatch_compound_samples(
        ReferenceSamples::settled(&reference0),
        ReferenceSamples::settled(&reference1),
        CompoundBlend::DiffWeighted { inverse: false },
    );
    assert_eq!(samples.0, vec![100; 64]);
    assert_eq!(samples.1, vec![81; 16]);
    assert_eq!(samples.2, vec![81; 16]);
}

#[test]
fn dispatcher_blends_compound_wedge_planes() {
    let reference0 = flat_frame(8, 8, 64, 64, 64);
    let reference1 = flat_frame(8, 8, 0, 0, 0);

    let samples = dispatch_compound_samples(
        ReferenceSamples::settled(&reference0),
        ReferenceSamples::settled(&reference1),
        CompoundBlend::Wedge {
            index: 0,
            sign: false,
        },
    );
    assert_eq!(
        u16::from(samples.0[0]),
        wedge_mask_plane_sample(8, 8, 0, false, 0, 0, 0, 0).expect("luma wedge mask")
    );
    assert_eq!(
        u16::from(samples.1[0]),
        wedge_mask_plane_sample(8, 8, 0, false, 1, 1, 0, 0).expect("chroma wedge mask")
    );

    let inverse = dispatch_compound_samples(
        ReferenceSamples::settled(&reference0),
        ReferenceSamples::settled(&reference1),
        CompoundBlend::Wedge {
            index: 0,
            sign: true,
        },
    );
    assert_eq!(
        u16::from(inverse.0[0]),
        wedge_mask_plane_sample(8, 8, 0, true, 0, 0, 0, 0).expect("inverse wedge mask")
    );
}

#[test]
fn dispatcher_uses_implicit_mask_for_offscreen_compound_refs() {
    let reference = patterned_frame(32, 8);
    let mut workspace = workspace(32, 8);

    motion_compensate_inter_block_into(
        &mut super::WorkspaceSink::Frame(&mut workspace),
        InterBlockParams::compound_average(
            ReferenceSamples::settled(&reference),
            ReferenceSamples::settled(&reference),
            McBlockRect::from_luma_rect(0, 0, 32, 4),
            Mv { row: 0, col: 32 },
            Mv { row: 0, col: -128 },
            InterpolationFilter::EightTap,
            CompoundBlend::average_with_implicit_mask(true),
        ),
        ByteOffset::new(0),
    )
    .expect("implicit-mask compound dispatcher");

    let decoded = workspace.freeze().expect("freeze implicit mask workspace");
    let y = visible_samples(&decoded, PlaneId::Y);
    assert_eq!(y[0], 4);
    assert_eq!(y[20], 14);
    assert_eq!(y[28], 12);
}

#[test]
fn u8_translational_compound_direct_fallback_matches_the_general_path() {
    let (width, height) = (17, 15);
    let reference0 = patterned_frame_with_format(PixelFormat::Yuv420, width, height, 11);
    let reference1 = patterned_frame_with_format(PixelFormat::Yuv420, width, height, 71);
    let mut block = InterBlockParams::compound_average(
        ReferenceSamples::settled(&reference0),
        ReferenceSamples::settled(&reference1),
        McBlockRect::from_luma_rect(0, 0, 8, 8),
        Mv::ZERO,
        Mv::ZERO,
        InterpolationFilter::EightTap,
        CompoundBlend::default(),
    )
    .into_compound()
    .expect("compound block");
    block.rect.chroma_luma_x = 16;

    let mut direct = workspace_for::<u8>(BitDepth::Eight, PixelFormat::Yuv420, width, height);
    assert!(
        !compound_average::predict_translational_direct(
            &mut WorkspaceSink::Frame(&mut direct),
            &block,
            ByteOffset::new(0),
        )
        .expect("geometry fallback")
    );
    predict_compound_into(
        &mut WorkspaceSink::Frame(&mut direct),
        &block,
        ByteOffset::new(0),
    )
    .expect("general path after the fallback");
    let mut general = workspace_for::<u8>(BitDepth::Eight, PixelFormat::Yuv420, width, height);
    predict_compound_into(
        &mut WorkspaceSink::Frame(&mut general),
        &block,
        ByteOffset::new(0),
    )
    .expect("general path");
    for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
        assert_eq!(
            direct.samples(plane).expect("direct plane"),
            general.samples(plane).expect("general plane"),
            "{plane:?} differs after the geometry fallback"
        );
    }
}

#[test]
fn direct_compound_blend_preserves_sample_storage_width() {
    let pred0 = [20 * 16, 60 * 16, 255 * 16];
    let pred1 = [44 * 16, 120 * 16, 255 * 16];
    let mut eight = vec![0; pred0.len()];
    blend_compound_average_weighted_samples::<u8>(
        &pred0,
        &pred1,
        BitDepth::Eight,
        CWP_EQUAL,
        &mut eight,
    )
    .expect("eight-bit compound blend");
    let mut wide = vec![0; pred0.len()];
    blend_compound_average_weighted_samples::<u16>(
        &pred0,
        &pred1,
        BitDepth::Eight,
        CWP_EQUAL,
        &mut wide,
    )
    .expect("wide eight-bit compound blend");
    assert_eq!(
        eight.iter().copied().map(u16::from).collect::<Vec<_>>(),
        wide
    );

    let mut ten_bit = vec![0; 1];
    blend_compound_average_weighted_samples::<u16>(
        &[900 * 16],
        &[1000 * 16],
        BitDepth::Ten,
        CWP_EQUAL,
        &mut ten_bit,
    )
    .expect("ten-bit compound blend");
    assert_eq!(ten_bit, [950]);
}

fn weighted_rows_match_the_per_sample_blend<T: ReconSample>(bit_depth: BitDepth) {
    for w in [3usize, 4, 6, 8, 17, 24] {
        let pred0: Vec<i32> = (0..w as i32)
            .map(|i| (i * 7919 % 4500) * 16 - 3000)
            .collect();
        let pred1: Vec<i32> = (0..w as i32)
            .map(|i| (i * 104_729 % 5000) * 16 - 9000)
            .collect();
        for cwp_weight in [2, 8, 12, 14] {
            let mut output = vec![T::default(); w];
            blend_compound_average_weighted_samples(
                &pred0,
                &pred1,
                bit_depth,
                cwp_weight,
                &mut output,
            )
            .expect("weighted row");
            for (col, sample) in output.iter().enumerate() {
                let want = splot_recon::blend_compound_average_weighted_sample(
                    pred0[col], pred1[col], bit_depth, cwp_weight,
                );
                assert_eq!(
                    sample.to_u16(),
                    want,
                    "{bit_depth:?} w={w} cwp={cwp_weight} {col}"
                );
            }
        }
    }
}

#[test]
fn weighted_rows_match_the_per_sample_blend_for_every_width() {
    weighted_rows_match_the_per_sample_blend::<u8>(BitDepth::Eight);
    weighted_rows_match_the_per_sample_blend::<u16>(BitDepth::Ten);
}

#[test]
fn scaled_compound_references_disable_implicit_mask_blending() {
    let pred0 = [20 * 16, 60 * 16, 100 * 16, 140 * 16];
    let pred1 = [44 * 16, 120 * 16, 80 * 16, 180 * 16];
    let scaling = derive_plane_scaling(4, 4, 0, 0, 0, 0, 64, 64, 32, 32);

    let mut got = vec![0; pred0.len()];
    blend_compound_average::<u16>(
        &pred0,
        &pred1,
        BitDepth::Eight,
        2,
        2,
        CompoundBlend::average_with_implicit_mask(true),
        scaling,
        scaling,
        32,
        32,
        &mut got,
        2,
    )
    .expect("scaled compound blend");
    let mut expected = vec![0; pred0.len()];
    blend_compound_average_weighted_samples::<u16>(
        &pred0,
        &pred1,
        BitDepth::Eight,
        CWP_EQUAL,
        &mut expected,
    )
    .expect("weighted compound blend");

    assert_eq!(got, expected);
}

#[test]
fn warp_skips_prediction_units_beyond_the_current_frame() {
    let reference = patterned_frame(16, 8);
    let mut workspace = workspace(16, 8);

    motion_compensate_inter_block_into(
        &mut super::WorkspaceSink::Frame(&mut workspace),
        InterBlockParams::single_warp(
            ReferenceSamples::settled(&reference),
            McBlockRect::from_luma_rect(8, 0, 16, 8),
            Mv::ZERO,
            InterpolationFilter::EightTap,
            crate::prediction::inter::find_mv_stack::DEFAULT_WARP_PARAMS,
        )
        .with_chroma(false),
        ByteOffset::new(0),
    )
    .expect("edge-clipped warp dispatcher");

    let decoded = workspace
        .freeze()
        .expect("freeze edge-clipped warp workspace");
    let y = visible_samples(&decoded, PlaneId::Y);
    let reference_y = visible_samples(&reference, PlaneId::Y);
    for row in 0..8 {
        assert_eq!(&y[row * 16..row * 16 + 8], &[0; 8]);
        assert_eq!(
            &y[row * 16 + 8..row * 16 + 16],
            &reference_y[row * 16 + 8..row * 16 + 16]
        );
    }
}

#[test]
fn edge_clipped_sheared_warp_writes_its_visible_sections() {
    let (width, height) = (40usize, 24usize);
    let reference = patterned_frame_with_format(PixelFormat::Yuv420, width, height, 7);
    let mut workspace = workspace(width, height);
    let warp_params = [
        3 << 15,
        -(1 << 15),
        (1 << 16) + 448,
        -320,
        256,
        (1 << 16) - 192,
    ];
    motion_compensate_inter_block_into(
        &mut super::WorkspaceSink::Frame(&mut workspace),
        InterBlockParams::single_warp(
            ReferenceSamples::settled(&reference),
            McBlockRect::from_luma_rect(24, 8, 32, 32),
            Mv::ZERO,
            InterpolationFilter::EightTap,
            warp_params,
        )
        .with_chroma(false),
        ByteOffset::new(0),
    )
    .expect("edge-clipped sheared warp dispatcher");

    let decoded = workspace.freeze().expect("freeze sheared warp workspace");
    let y = visible_samples(&decoded, PlaneId::Y);
    let reference_y = visible_samples(&reference, PlaneId::Y);
    let view = ReferencePlaneView::new(&reference_y, width, height).expect("reference view");
    for (x, y0) in [(24, 8), (32, 8), (24, 16), (32, 16)] {
        let params = WarpPredictBlockParams {
            warp_params,
            block_x: x as i32,
            block_y: y0 as i32,
            subsampling_x: 0,
            subsampling_y: 0,
            reference_scale_x: 1 << 14,
            reference_scale_y: 1 << 14,
            first_x: 0,
            first_y: 0,
            last_x: width as i32 - 1,
            last_y: height as i32 - 1,
            bit_depth: BitDepth::Eight,
        };
        let section = splot_recon::warp_predict_block(&view, &params, false).expect("section");
        for (row, values) in section.chunks_exact(8).enumerate() {
            let start = (y0 + row) * width + x;
            let want = values.iter().map(|&value| value.clamp(0, 255) as u8);
            assert!(y[start..start + 8].iter().copied().eq(want), "({x}, {y0})");
        }
    }
}

#[test]
fn extended_warp_skips_prediction_units_beyond_the_current_frame() {
    let reference = patterned_frame(16, 8);
    let mut workspace = workspace(16, 8);

    motion_compensate_inter_block_into(
        &mut super::WorkspaceSink::Frame(&mut workspace),
        InterBlockParams::single_warp(
            ReferenceSamples::settled(&reference),
            McBlockRect::from_luma_rect(12, 0, 8, 4),
            Mv::ZERO,
            InterpolationFilter::EightTap,
            crate::prediction::inter::find_mv_stack::DEFAULT_WARP_PARAMS,
        )
        .with_chroma(false),
        ByteOffset::new(0),
    )
    .expect("edge-clipped extended-warp dispatcher");

    let decoded = workspace
        .freeze()
        .expect("freeze edge-clipped extended-warp workspace");
    let y = visible_samples(&decoded, PlaneId::Y);
    let reference_y = visible_samples(&reference, PlaneId::Y);
    for row in 0..4 {
        assert_eq!(&y[row * 16..row * 16 + 12], &[0; 12]);
        assert_eq!(
            &y[row * 16 + 12..row * 16 + 16],
            &reference_y[row * 16 + 12..row * 16 + 16]
        );
    }
    assert!(y[4 * 16..].iter().all(|&sample| sample == 0));
}

fn workspace(width: usize, height: usize) -> CurrentFrameWorkspace<u8> {
    workspace_with_format(PixelFormat::Yuv420, width, height)
}

fn workspace_with_format(
    pixel_format: PixelFormat,
    width: usize,
    height: usize,
) -> CurrentFrameWorkspace<u8> {
    workspace_for(BitDepth::Eight, pixel_format, width, height)
}

pub(super) fn workspace_for<T: ReconSample>(
    bit_depth: BitDepth,
    pixel_format: PixelFormat,
    width: usize,
    height: usize,
) -> CurrentFrameWorkspace<T> {
    let luma_size = PlaneSize::new(width, height).expect("luma size");
    let visible = PlaneRect::new(0, 0, width, height).expect("visible rect");
    let info = DecodedFrameInfo::new(
        OutputIndex::new(0),
        bit_depth,
        pixel_format,
        luma_size,
        visible,
    )
    .expect("frame info");
    CurrentFrameWorkspace::new(info, T::default()).expect("workspace")
}

fn dispatch_compound_samples(
    reference0: ReferenceSamples<'_, u8>,
    reference1: ReferenceSamples<'_, u8>,
    blend: CompoundBlend,
) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let mut workspace = workspace(8, 8);
    motion_compensate_inter_block_into(
        &mut super::WorkspaceSink::Frame(&mut workspace),
        InterBlockParams::compound_average(
            reference0,
            reference1,
            McBlockRect::from_luma_rect(0, 0, 8, 8),
            Mv { row: 0, col: 0 },
            Mv { row: 0, col: 0 },
            InterpolationFilter::EightTap,
            blend,
        ),
        ByteOffset::new(0),
    )
    .expect("compound dispatcher");

    let decoded = workspace.freeze().expect("freeze compound workspace");
    (
        visible_samples(&decoded, PlaneId::Y),
        visible_samples(&decoded, PlaneId::U),
        visible_samples(&decoded, PlaneId::V),
    )
}

fn patterned_frame(width: usize, height: usize) -> DecodedFrame<u8> {
    let y: Vec<u8> = (0..width * height)
        .map(|sample| u8::try_from(sample).expect("luma sample"))
        .collect();
    let chroma_width = width.div_ceil(2);
    let chroma_height = height.div_ceil(2);
    let u: Vec<u8> = (0..chroma_width * chroma_height)
        .map(|sample| 100 + u8::try_from(sample).expect("u sample"))
        .collect();
    let v: Vec<u8> = (0..chroma_width * chroma_height)
        .map(|sample| 150 + u8::try_from(sample).expect("v sample"))
        .collect();
    frame(width, height, y, u, v)
}

fn patterned_frame_with_format(
    pixel_format: PixelFormat,
    width: usize,
    height: usize,
    seed: usize,
) -> DecodedFrame<u8> {
    let samples = |len: usize, offset: usize| {
        (0..len)
            .map(|index| ((index * 29 + seed + offset) % 251) as u8)
            .collect()
    };
    let chroma_width = width.div_ceil(1 << pixel_format.subsampling_x());
    let chroma_height = height.div_ceil(1 << pixel_format.subsampling_y());
    frame_for(
        BitDepth::Eight,
        pixel_format,
        width,
        height,
        samples(width * height, 0),
        samples(chroma_width * chroma_height, 37),
        samples(chroma_width * chroma_height, 83),
    )
}

fn flat_frame(width: usize, height: usize, y: u8, u: u8, v: u8) -> DecodedFrame<u8> {
    flat_frame_with_format(PixelFormat::Yuv420, width, height, y, u, v)
}

fn flat_frame_with_format(
    pixel_format: PixelFormat,
    width: usize,
    height: usize,
    y: u8,
    u: u8,
    v: u8,
) -> DecodedFrame<u8> {
    let chroma_width = width.div_ceil(1 << pixel_format.subsampling_x());
    let chroma_height = height.div_ceil(1 << pixel_format.subsampling_y());
    frame_with_format(
        pixel_format,
        width,
        height,
        vec![y; width * height],
        vec![u; chroma_width * chroma_height],
        vec![v; chroma_width * chroma_height],
    )
}

fn frame(width: usize, height: usize, y: Vec<u8>, u: Vec<u8>, v: Vec<u8>) -> DecodedFrame<u8> {
    frame_with_format(PixelFormat::Yuv420, width, height, y, u, v)
}

fn frame_with_format(
    pixel_format: PixelFormat,
    width: usize,
    height: usize,
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
) -> DecodedFrame<u8> {
    frame_for(BitDepth::Eight, pixel_format, width, height, y, u, v)
}

pub(super) fn frame_for<T: ReconSample>(
    bit_depth: BitDepth,
    pixel_format: PixelFormat,
    width: usize,
    height: usize,
    y: Vec<T>,
    u: Vec<T>,
    v: Vec<T>,
) -> DecodedFrame<T> {
    let luma_size = PlaneSize::new(width, height).expect("luma size");
    let luma_rect = PlaneRect::new(0, 0, width, height).expect("luma rect");
    let chroma_width = width.div_ceil(1 << pixel_format.subsampling_x());
    let chroma_height = height.div_ceil(1 << pixel_format.subsampling_y());
    let chroma_size = PlaneSize::new(chroma_width, chroma_height).expect("chroma size");
    let chroma_rect = PlaneRect::new(0, 0, chroma_width, chroma_height).expect("chroma rect");
    let info = DecodedFrameInfo::new(
        OutputIndex::new(0),
        bit_depth,
        pixel_format,
        luma_size,
        luma_rect,
    )
    .expect("frame info");

    let planes = if pixel_format == PixelFormat::Monochrome {
        FramePlanes::new(plane(luma_size, width, luma_rect, y), None, None)
    } else {
        FramePlanes::new(
            plane(luma_size, width, luma_rect, y),
            Some(plane(chroma_size, chroma_width, chroma_rect, u)),
            Some(plane(chroma_size, chroma_width, chroma_rect, v)),
        )
    };

    DecodedFrame::try_new(info, planes).expect("decoded frame")
}

fn plane<T: ReconSample>(
    size: PlaneSize,
    stride: usize,
    visible: PlaneRect,
    samples: Vec<T>,
) -> Plane<T> {
    Plane::from_vec(size, stride, visible, samples).expect("plane")
}

fn visible_samples<T: ReconSample>(frame: &DecodedFrame<T>, plane: PlaneId) -> Vec<T> {
    frame
        .plane(plane)
        .expect("frame plane")
        .visible_rows()
        .flat_map(|row| row.iter().copied())
        .collect()
}

#[test]
fn prediction_reads_padded_reference_samples_and_masks_at_storage_edges() {
    let info = DecodedFrameInfo::new(
        OutputIndex::new(0),
        BitDepth::Eight,
        PixelFormat::Monochrome,
        PlaneSize::new(10, 10).expect("coded size"),
        PlaneRect::new(0, 0, 10, 10).expect("visible rect"),
    )
    .expect("frame info")
    .with_storage_luma_size(PlaneSize::new(16, 16).expect("storage size"))
    .expect("storage bounds");
    let mut reference = CurrentFrameWorkspace::new(info, 20u8).expect("reference workspace");
    reference
        .write_rect(
            PlaneId::Y,
            PlaneRect::new(12, 12, 4, 4).expect("padding rect"),
            &[100; 16],
            4,
        )
        .expect("padding samples");
    let reference = reference.freeze().expect("reference frame");
    let samples = ReferenceSamples::settled(&reference);
    let rect = McBlockRect::from_luma_rect(0, 0, 4, 4);
    for compound in [false, true] {
        let mut output = CurrentFrameWorkspace::new(info, 0u8).expect("output workspace");
        let block = if compound {
            InterBlockParams::compound_average(
                samples,
                samples,
                rect,
                Mv { row: 96, col: 96 },
                Mv::ZERO,
                InterpolationFilter::EightTap,
                CompoundBlend::average_with_implicit_mask(true),
            )
        } else {
            InterBlockParams::single(
                samples,
                rect,
                Mv { row: 96, col: 96 },
                InterpolationFilter::EightTap,
            )
        }
        .with_chroma(false);
        motion_compensate_inter_block_into(
            &mut WorkspaceSink::Frame(&mut output),
            block,
            ByteOffset::new(0),
        )
        .expect("prediction");
        let expected = if compound { 60 } else { 100 };
        let stride = output.plane(PlaneId::Y).expect("luma").stride_samples();
        for row in 0..4 {
            assert_eq!(
                &output.samples(PlaneId::Y).expect("luma")[row * stride..row * stride + 4],
                &[expected; 4]
            );
        }
    }
}

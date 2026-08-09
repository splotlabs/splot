// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::expect_used, clippy::panic)]

use super::*;
use splot_recon::{
    DecodedFrame, DecodedFrameInfo, FramePlanes, OutputIndex, PixelFormat, Plane, PlaneSize,
    wedge_mask_plane_sample,
};

fn motion_grid_prediction<T: ReconSample>(
    sink: &mut WorkspaceSink<'_, '_, T>,
    block: InterBlockParams<'_, T>,
    optflow_unit_size: Option<usize>,
    offset: ByteOffset,
) -> Result<Option<CompoundMotionGrid>> {
    let motion = inter_block_motion_grid(sink, block, optflow_unit_size, offset)?;
    predict_inter_block_from_grid(sink, block, motion, offset)
}

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
                rect(0, 0, 8, 8),
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
                rect(0, 0, 8, 8),
                Mv::ZERO,
                Mv::ZERO,
                InterpolationFilter::EightTap,
                CompoundBlend::default(),
            ),
            InterBlockParams::single_warp(
                ReferenceSamples::settled(&reference),
                rect(0, 0, 8, 8),
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
            rect(0, 0, 8, 8),
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
            rect(0, 0, 9, 11),
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
            rect(4, 4, 8, 8),
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

    let err = super::sink::write_u16_rect(
        &mut WorkspaceSink::Frame(&mut workspace),
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

    let err = super::sink::write_u16_rect(
        &mut WorkspaceSink::Frame(&mut workspace),
        PlaneId::Y,
        plane_rect,
        &samples,
        8,
    )
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
            rect(0, 0, 9, 11),
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
fn dispatcher_sub8x8_chroma_uses_only_the_first_reference() {
    let reference0 = flat_frame(8, 8, 40, 90, 120);
    let reference1 = flat_frame(8, 8, 80, 110, 140);
    let mut workspace = workspace(8, 8);

    motion_compensate_inter_block_into(
        &mut super::WorkspaceSink::Frame(&mut workspace),
        InterBlockParams::compound_average(
            ReferenceSamples::settled(&reference0),
            ReferenceSamples::settled(&reference1),
            rect(0, 0, 8, 8),
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
    let rect = rect(0, 0, 8, 8);
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
            rect(0, 0, 8, 8),
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
                rect(0, 0, 8, 8),
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
        motion_grid_prediction(
            &mut super::WorkspaceSink::Frame(&mut workspace),
            InterBlockParams::compound_average(
                ReferenceSamples::settled(&reference0),
                ReferenceSamples::settled(&reference1),
                rect(8, 8, 8, 8),
                Mv { row: 3, col: 5 },
                Mv { row: -5, col: -3 },
                interp,
                CompoundBlend::default(),
            )
            .with_optflow_distances(Some([1, -1])),
            None,
            ByteOffset::new(0),
        )
        .expect("optical-flow dispatcher")
        .expect("optical-flow motion grid")
        .at_luma_offset(0, 0)
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
    let compound = InterBlockParams::compound_average(
        ReferenceSamples::settled(&reference0),
        ReferenceSamples::settled(&reference1),
        rect(0, 0, 8, 8),
        Mv::ZERO,
        Mv::ZERO,
        InterpolationFilter::EightTap,
        CompoundBlend::default(),
    )
    .with_optflow_distances(Some([1, -1]))
    .into_compound()
    .expect("compound block");
    let grid = compound_block_motion_grid(
        &super::WorkspaceSink::Frame(&mut workspace),
        compound,
        Some(8),
        ByteOffset::new(0),
    )
    .expect("TIP output optical-flow dispatcher")
    .expect("TIP output optical-flow motion grid");

    assert_eq!(
        grid.stored_mvs_at_luma_offset(0, 0)
            .expect("TIP output stored motion vectors"),
        [Mv::ZERO; 2]
    );
}

#[test]
fn deferred_compound_prediction_matches_direct_publication() {
    let reference0 = flat_frame(8, 8, 40, 90, 120);
    let reference1 = flat_frame(8, 8, 80, 110, 140);
    let block = InterBlockParams::compound_average(
        ReferenceSamples::settled(&reference0),
        ReferenceSamples::settled(&reference1),
        rect(0, 0, 8, 8),
        Mv::ZERO,
        Mv::ZERO,
        InterpolationFilter::EightTap,
        CompoundBlend::default(),
    )
    .with_optflow_distances(Some([1, -1]));
    let offset = ByteOffset::new(0);

    let mut direct = workspace(8, 8);
    motion_compensate_inter_block_into(
        &mut super::WorkspaceSink::Frame(&mut direct),
        block,
        offset,
    )
    .expect("direct TIP optical-flow prediction");

    let mut deferred = workspace(8, 8);
    let compound = block.into_compound().expect("compound block");
    let motion = compound_block_motion_grid(
        &super::WorkspaceSink::Frame(&mut deferred),
        compound,
        Some(8),
        offset,
    )
    .expect("deferred TIP motion grid");
    let output = predict_compound_average_block(
        &super::WorkspaceSink::Frame(&mut deferred),
        compound,
        motion,
        offset,
    )
    .expect("deferred TIP optical-flow prediction");
    let deferred_mvs = output
        .metadata
        .stored_mvs_at_origin()
        .expect("deferred stored motion vectors");
    output
        .publish(&mut super::WorkspaceSink::Frame(&mut deferred))
        .expect("publish deferred TIP prediction");

    let mut short = [0u8; 95];
    let err = predict_compound_from_grid(
        &super::WorkspaceSink::Frame(&mut deferred),
        compound,
        None,
        offset,
        &mut short,
    )
    .expect_err("short deferred output must fail");
    assert!(matches!(
        err,
        crate::error::DecodeError::Reconstruction {
            source: ReconError::BufferLengthMismatch {
                expected: 96,
                actual: 95
            }
        }
    ));

    let mut borrowed = workspace(8, 8);
    let mut arena_chunk = vec![u8::MAX; 103];
    let motion = compound_block_motion_grid(
        &super::WorkspaceSink::Frame(&mut borrowed),
        compound,
        Some(8),
        offset,
    )
    .expect("borrowed TIP motion grid");
    let metadata = predict_compound_from_grid(
        &super::WorkspaceSink::Frame(&mut borrowed),
        compound,
        motion,
        offset,
        &mut arena_chunk,
    )
    .expect("borrowed TIP optical-flow prediction");
    let borrowed_mvs = metadata
        .stored_mvs_at_origin()
        .expect("borrowed stored motion vectors");
    metadata
        .publish(
            &arena_chunk,
            &mut super::WorkspaceSink::Frame(&mut borrowed),
        )
        .expect("publish borrowed TIP prediction");
    assert_eq!(&arena_chunk[96..], &[u8::MAX; 7]);

    assert_eq!(deferred_mvs, borrowed_mvs);
    let direct = direct.freeze().expect("freeze direct workspace");
    let deferred = deferred.freeze().expect("freeze deferred workspace");
    let borrowed = borrowed.freeze().expect("freeze borrowed workspace");
    for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
        assert_eq!(
            visible_samples(&deferred, plane),
            visible_samples(&direct, plane)
        );
        assert_eq!(
            visible_samples(&borrowed, plane),
            visible_samples(&direct, plane)
        );
    }
}

#[test]
fn tip_optflow_skips_low_sad_predictors() {
    let pred0 = vec![55; 64];
    let mut pred1 = pred0.clone();
    pred1.iter_mut().take(10).for_each(|sample| *sample -= 1);
    let chroma = vec![128; 16];
    let reference0 = frame(8, 8, pred0, chroma.clone(), chroma.clone());
    let reference1 = frame(8, 8, pred1, chroma.clone(), chroma);
    let mut workspace = workspace(8, 8);
    let compound = InterBlockParams::compound_average(
        ReferenceSamples::settled(&reference0),
        ReferenceSamples::settled(&reference1),
        rect(0, 0, 8, 8),
        Mv::ZERO,
        Mv::ZERO,
        InterpolationFilter::Bilinear,
        CompoundBlend::default(),
    )
    .with_optflow_distances(Some([1, -1]))
    .with_optflow_sad_threshold(Some(15))
    .into_compound()
    .expect("compound block");
    let grid = compound_block_motion_grid(
        &super::WorkspaceSink::Frame(&mut workspace),
        compound,
        Some(8),
        ByteOffset::new(0),
    )
    .expect("TIP optical-flow SAD gate");

    assert!(grid.is_none());
}

#[test]
fn optflow_sad_gate_preserves_refinemv_motion_grid() {
    let reference0 = flat_frame(8, 8, 55, 128, 128);
    let reference1 = flat_frame(8, 8, 55, 128, 128);
    let mut workspace = workspace(8, 8);
    let grid = motion_grid_prediction(
        &mut super::WorkspaceSink::Frame(&mut workspace),
        InterBlockParams::compound_average(
            ReferenceSamples::settled(&reference0),
            ReferenceSamples::settled(&reference1),
            rect(0, 0, 8, 8),
            Mv::ZERO,
            Mv::ZERO,
            InterpolationFilter::Bilinear,
            CompoundBlend::default(),
        )
        .with_refinemv(true)
        .with_refinemv_search(false)
        .with_optflow_distances(Some([1, -1]))
        .with_optflow_sad_threshold(Some(1)),
        None,
        ByteOffset::new(0),
    )
    .expect("refine-MV optical-flow SAD gate")
    .expect("preserved refine-MV motion grid");

    assert_eq!(
        grid.temporal_mvs_at_luma_offset(0, 0)
            .expect("stored refine-MVs"),
        [Mv::ZERO; 2]
    );
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

    let grid = motion_grid_prediction(
        &mut super::WorkspaceSink::Frame(&mut workspace),
        InterBlockParams::compound_average(
            ReferenceSamples::settled(&reference0),
            ReferenceSamples::settled(&reference1),
            rect(8, 8, 16, 16),
            Mv::ZERO,
            Mv::ZERO,
            InterpolationFilter::EightTapSharp,
            CompoundBlend::default(),
        )
        .with_refinemv(true),
        None,
        ByteOffset::new(0),
    )
    .expect("default refine-MV dispatcher")
    .expect("refine-MV motion grid");

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

    let grid = motion_grid_prediction(
        &mut super::WorkspaceSink::Frame(&mut workspace),
        InterBlockParams::compound_average(
            ReferenceSamples::settled(&reference0),
            ReferenceSamples::settled(&reference1),
            rect(8, 8, 16, 16),
            Mv::ZERO,
            Mv::ZERO,
            InterpolationFilter::EightTapSharp,
            CompoundBlend::default(),
        )
        .with_refinemv(true)
        .with_switchable_refinemv(true),
        None,
        ByteOffset::new(0),
    )
    .expect("switchable refine-MV dispatcher")
    .expect("refine-MV motion grid");

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

    let grid = motion_grid_prediction(
        &mut super::WorkspaceSink::Frame(&mut workspace),
        InterBlockParams::compound_average(
            ReferenceSamples::settled(&reference0),
            ReferenceSamples::settled(&reference1),
            rect(8, 8, 16, 16),
            Mv::ZERO,
            Mv::ZERO,
            InterpolationFilter::EightTapSharp,
            CompoundBlend::default(),
        )
        .with_refinemv(true)
        .with_refinemv_search(false),
        None,
        ByteOffset::new(0),
    )
    .expect("refine-MV dispatcher without search")
    .expect("refine-MV motion grid");

    assert_eq!(
        grid.temporal_mvs_at_luma_offset(0, 0)
            .expect("stored refine-MVs"),
        [Mv::ZERO, Mv::ZERO]
    );
}

#[test]
fn chroma_geometry_rounds_odd_luma_extents_up() {
    assert_eq!(
        rect(0, 0, 17, 19).plane_rect(PlaneId::U, 1, 1),
        (0, 0, 9, 10)
    );
    assert_eq!(rect(1, 1, 8, 8).plane_rect(PlaneId::V, 1, 1), (0, 0, 5, 5));
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
            rect(0, 0, 32, 4),
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

#[allow(clippy::too_many_arguments)]
fn per_pixel_reference_blend(
    pred0: &[i32],
    pred1: &[i32],
    w: usize,
    motion: &CompoundMotionGrid,
    plane_x: usize,
    plane_y: usize,
    scalings: [crate::prediction::inter::mv_scaling::PlaneScaling; 2],
    frame_w: usize,
    frame_h: usize,
) -> Vec<u16> {
    let shift = 1 + compound_inter_post_round();
    let mut expected = vec![0u16; pred0.len()];
    for (idx, slot) in expected.iter_mut().enumerate() {
        let row = idx / w;
        let col = idx % w;
        let mvs = motion.at_luma_offset(col, row).expect("cell lookup");
        let starts: [(i32, i32); 2] = core::array::from_fn(|reference| {
            let scaling = scalings[reference].with_prescaled_mv(
                (plane_x + col) as i32,
                (plane_y + row) as i32,
                mvs[reference][0],
                mvs[reference][1],
                0,
                0,
            );
            (scaling.start_x >> 10, scaling.start_y >> 10)
        });
        let onscreen = |start: (i32, i32)| {
            (0..=(frame_w as i32 - 1)).contains(&start.0)
                && (0..=(frame_h as i32 - 1)).contains(&start.1)
        };
        let mask = match (onscreen(starts[0]), onscreen(starts[1])) {
            (true, false) => 2,
            (false, true) => 0,
            _ => 1,
        };
        let sample = round2_i32(mask * pred0[idx] + (2 - mask) * pred1[idx], shift);
        *slot = sample.clamp(0, 255) as u16;
    }
    expected
}

#[test]
fn multi_span_implicit_mask_blend_matches_per_pixel_reference() {
    let w = 40usize;
    let h = 2usize;
    let pred0: Vec<i32> = (0..w * h)
        .map(|index| (index as i32 * 7 % 900) * 16)
        .collect();
    let pred1: Vec<i32> = (0..w * h)
        .map(|index| (index as i32 * 11 % 800) * 16)
        .collect();
    let cells = vec![
        MotionCell::from_refinemv([Mv::ZERO; 2]),
        MotionCell::from_refinemv([Mv { row: 0, col: -640 }, Mv::ZERO]),
        MotionCell::from_refinemv([Mv { row: 0, col: 96 }, Mv { row: 640, col: 0 }]),
    ];
    let motion = CompoundMotionGrid::from_refinemv(3, [Mv::ZERO; 2], cells);
    let (plane_x, plane_y) = (4usize, 4usize);
    let (frame_w, frame_h) = (48usize, 8usize);
    let blend = CompoundBlend::average_with_implicit_mask(true);
    let scaling = derive_plane_scaling(
        plane_x as i32,
        plane_y as i32,
        0,
        0,
        0,
        0,
        frame_w as i32,
        frame_h as i32,
        frame_w as i32,
        frame_h as i32,
    );
    let mut output = vec![0u16; w * h];
    blend_compound_average::<u16>(
        &pred0,
        &pred1,
        BitDepth::Eight,
        w,
        h,
        blend,
        w,
        h,
        Some(&motion),
        plane_x,
        plane_y,
        scaling,
        scaling,
        frame_w,
        frame_h,
        None,
        0,
        0,
        &mut output,
    )
    .expect("multi-span implicit-mask blend");
    let expected = per_pixel_reference_blend(
        &pred0,
        &pred1,
        w,
        &motion,
        plane_x,
        plane_y,
        [scaling, scaling],
        frame_w,
        frame_h,
    );
    assert_eq!(output, expected);
    assert!(output.windows(2).any(|pair| pair[0] != pair[1]));
}

#[test]
fn uniform_implicit_mask_fast_path_matches_per_sample_path() {
    let pred0 = [20 * 16, 60 * 16, 100 * 16, 140 * 16];
    let pred1 = [44 * 16, 120 * 16, 80 * 16, 180 * 16];
    let mvs = [Mv::ZERO; 2];
    let uniform = CompoundMotionGrid::from_refinemv(1, mvs, vec![MotionCell::from_refinemv(mvs)]);
    let multiple =
        CompoundMotionGrid::from_refinemv(2, mvs, vec![MotionCell::from_refinemv(mvs); 2]);
    let scaling = derive_plane_scaling(4, 4, 0, 0, 0, 0, 32, 32, 32, 32);
    let blend = CompoundBlend::average_with_implicit_mask(true);
    let run = |motion| {
        let mut output = vec![0; pred0.len()];
        blend_compound_average::<u16>(
            &pred0,
            &pred1,
            BitDepth::Eight,
            2,
            2,
            blend,
            2,
            2,
            Some(motion),
            4,
            4,
            scaling,
            scaling,
            32,
            32,
            None,
            0,
            0,
            &mut output,
        )
        .expect("implicit-mask blend");
        output
    };

    assert_eq!(run(&uniform), run(&multiple));
}

#[test]
fn uniform_motion_direct_average_matches_materialized_path() {
    let width = 16usize;
    let height = 16usize;
    let chroma_len = width.div_ceil(2) * height.div_ceil(2);
    let reference0 = frame_for(
        BitDepth::Ten,
        PixelFormat::Yuv420,
        width,
        height,
        (0..width * height)
            .map(|index| 128 + (index * 13 % 700) as u16)
            .collect(),
        vec![384; chroma_len],
        vec![512; chroma_len],
    );
    let reference1 = frame_for(
        BitDepth::Ten,
        PixelFormat::Yuv420,
        width,
        height,
        (0..width * height)
            .map(|index| 192 + (index * 17 % 600) as u16)
            .collect(),
        vec![448; chroma_len],
        vec![576; chroma_len],
    );
    let rect = rect(4, 4, 8, 8);
    let mvs = [Mv { row: 1, col: 1 }, Mv { row: -1, col: 2 }];
    let cell = MotionCell::from_refinemv(mvs);
    let uniform = CompoundMotionGrid::from_refinemv(1, mvs, vec![cell]);
    let multiple = CompoundMotionGrid::from_refinemv(2, mvs, vec![cell; 2]);
    let run = |motion| {
        let mut workspace = workspace_for::<u16>(BitDepth::Ten, PixelFormat::Yuv420, width, height);
        let mut output = vec![0; rect.luma_w * rect.luma_h];
        predict_compound_plane_output(
            &WorkspaceSink::Frame(&mut workspace),
            ReferenceSamples::settled(&reference0),
            ReferenceSamples::settled(&reference1),
            PlaneId::Y,
            rect,
            mvs[0],
            mvs[1],
            InterpolationFilter::EightTap,
            CompoundBlend::average_with_implicit_mask(true),
            [None; 2],
            0,
            0,
            None,
            Some(motion),
            ByteOffset::new(0),
            &mut output,
        )
        .expect("compound motion prediction");
        output
    };

    assert_eq!(run(&uniform), run(&multiple));
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
        2,
        2,
        None,
        4,
        4,
        scaling,
        scaling,
        32,
        32,
        None,
        0,
        0,
        &mut got,
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
            rect(8, 0, 16, 8),
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
fn extended_warp_skips_prediction_units_beyond_the_current_frame() {
    let reference = patterned_frame(16, 8);
    let mut workspace = workspace(16, 8);

    motion_compensate_inter_block_into(
        &mut super::WorkspaceSink::Frame(&mut workspace),
        InterBlockParams::single_warp(
            ReferenceSamples::settled(&reference),
            rect(12, 0, 8, 4),
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

fn workspace_for<T: ReconSample>(
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
            rect(0, 0, 8, 8),
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

fn frame_for<T: ReconSample>(
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

    DecodedFrame::try_new(
        info,
        FramePlanes::new(
            plane(luma_size, width, luma_rect, y),
            Some(plane(chroma_size, chroma_width, chroma_rect, u)),
            Some(plane(chroma_size, chroma_width, chroma_rect, v)),
        ),
    )
    .expect("decoded frame")
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

const fn rect(luma_x: usize, luma_y: usize, luma_w: usize, luma_h: usize) -> McBlockRect {
    McBlockRect {
        luma_x,
        luma_y,
        luma_w,
        luma_h,
        chroma_luma_x: luma_x,
        chroma_luma_y: luma_y,
        chroma_luma_w: luma_w,
        chroma_luma_h: luma_h,
    }
}

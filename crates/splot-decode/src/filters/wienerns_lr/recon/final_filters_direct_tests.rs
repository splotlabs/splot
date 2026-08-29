// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use super::*;
use crate::filters::source::StripeInitialization;
use splot_recon::{
    BitDepth, CurrentFrameWorkspace, DecodedFrameInfo, OutputIndex, PixelFormat, PlaneId,
    PlaneRect, PlaneSize, ReconSample,
};
use std::sync::Arc;

const POISON: u16 = 0xdead;

fn workspace(format: PixelFormat, width: usize, height: usize) -> CurrentFrameWorkspace<u16> {
    workspace_with(BitDepth::Ten, format, width, height, |x, y, plane| {
        64 + ((x * 7 + y * 11 + plane.index() * 23) % 900) as u16
    })
}

fn workspace_with<T: ReconSample>(
    bit_depth: BitDepth,
    format: PixelFormat,
    width: usize,
    height: usize,
    mut sample_at: impl FnMut(usize, usize, PlaneId) -> T,
) -> CurrentFrameWorkspace<T> {
    let info = DecodedFrameInfo::new(
        OutputIndex::new(0),
        bit_depth,
        format,
        PlaneSize::new(width, height).unwrap(),
        PlaneRect::new(0, 0, width, height).unwrap(),
    )
    .unwrap();
    let mut workspace = CurrentFrameWorkspace::new(info, T::default()).unwrap();
    for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
        let Ok(view) = workspace.plane(plane) else {
            continue;
        };
        let size = view.storage_size();
        for y in 0..size.height() {
            for x in 0..size.width() {
                workspace
                    .set_reconstructed_sample(plane, x, y, sample_at(x, y, plane))
                    .unwrap();
            }
        }
    }
    workspace
}

fn lr_core(restoration: [FrameRestorationType; 3]) -> FrameHeaderCore {
    let fixture = include_bytes!(
        "../../../../../../tests/conformance/vectors/valid/\
         syn-2frame-lr-switchable-768x256-8bit.ivf"
    );
    let mut core = crate::prediction::inter::test_support::fixture_sequence_and_key_core(fixture).1;
    let params = core.lr_params.as_mut().unwrap();
    for (plane, restoration_type) in params.planes.iter_mut().zip(restoration) {
        plane.restoration_type = restoration_type;
        plane.frame_filters_on = false;
    }
    core
}

fn cdef_frame(workspace: &CurrentFrameWorkspace<u16>) -> CdefFrame<'_, u16> {
    let y = FramePlane::new(workspace, PlaneId::Y).unwrap();
    let u = FramePlane::new(workspace, PlaneId::U);
    let v = FramePlane::new(workspace, PlaneId::V);
    CdefFrame {
        deblocked_y: y,
        deblocked_u: u,
        deblocked_v: v,
        filtered_y: StripePlane::copy_from(y, 0, y.frame_height()).unwrap(),
        filtered_u: u.map(|plane| StripePlane::copy_from(plane, 0, plane.frame_height()).unwrap()),
        filtered_v: v.map(|plane| StripePlane::copy_from(plane, 0, plane.frame_height()).unwrap()),
    }
}

fn poison_progress(progress: &Arc<crate::pipeline::frame_progress::FrameProgress<u16>>) {
    let mut lease = progress.direct_stripe(0).unwrap();
    let mut target = lease.take_target().unwrap();
    for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
        if let Some(mut plane) = target.take(plane) {
            plane.u16_samples_mut().unwrap().fill(POISON);
        }
    }
}

fn block(
    plane: PlaneId,
    restoration_type: LrUnitRestorationType,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
) -> WienerNsLrSourceBlock {
    WienerNsLrSourceBlock {
        restoration_type,
        plane: plane.index(),
        unit_row: 0,
        unit_col: 0,
        unit_filter_index: None,
        tile_mi_row_start: 0,
        tile_mi_row_end: 64,
        tile_mi_col_end: 64,
        x,
        y,
        width,
        height,
        luma_start_x: 0,
        luma_end_x: 255,
        luma_start_y: 0,
        luma_end_y: 255,
        luma_stripe_start_y: 0,
        luma_stripe_end_y: 255,
    }
}

fn blocks_for_target(
    target: &crate::pipeline::frame_progress::DirectPlaneTarget,
    plane: PlaneId,
) -> Vec<WienerNsLrSourceBlock> {
    let end_y = target.end_y().unwrap();
    let mut blocks = Vec::new();
    for y in (target.origin_y()..end_y).step_by(4) {
        for x in (0..target.width()).step_by(4) {
            blocks.push(block(
                plane,
                LrUnitRestorationType::WienerNonsep,
                x,
                y,
                4.min(target.width() - x),
                4.min(end_y - y),
            ));
        }
    }
    blocks
}

fn write_blocks(plane: &mut StripePlane, blocks: &[WienerNsLrSourceBlock], value: u16) {
    for block in blocks {
        filter_lr_block_into::<u16>(plane, block, |output, stride| {
            for row in 0..block.height {
                for col in 0..block.width {
                    output[row * stride + col] = value;
                }
            }
            Ok(())
        })
        .unwrap();
    }
}

#[test]
fn poisoned_u16_targets_are_fully_written_for_all_plane_formats_and_odd_dimensions() {
    for format in [
        PixelFormat::Monochrome,
        PixelFormat::Yuv420,
        PixelFormat::Yuv422,
        PixelFormat::Yuv444,
    ] {
        let workspace = workspace(format, 9, 7);
        let progress = Arc::new(
            crate::pipeline::frame_progress::FrameProgress::<u16>::new(workspace.info()).unwrap(),
        );
        assert!(progress.begin(&[(0, 7)]));
        poison_progress(&progress);
        let mut lease = progress.direct_stripe(0).unwrap();
        let target = lease.take_target().unwrap();
        let active = [
            true,
            format != PixelFormat::Monochrome,
            format != PixelFormat::Monochrome,
        ];
        let block_sets: [Vec<WienerNsLrSourceBlock>; 3] = core::array::from_fn(|index| {
            let plane = [PlaneId::Y, PlaneId::U, PlaneId::V][index];
            target
                .get(plane)
                .map_or_else(Vec::new, |target| blocks_for_target(target, plane))
        });
        let block_slices = [
            block_sets[0].as_slice(),
            block_sets[1].as_slice(),
            block_sets[2].as_slice(),
        ];
        let core = lr_core([FrameRestorationType::WienerNonsep; 3]);
        let initializations = lr_initializations(&core, active, block_slices, &target);
        for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
            if active[plane.index()] {
                assert_eq!(
                    initializations[plane.index()],
                    StripeInitialization::FullyOverwritten,
                    "{format:?} {plane:?}"
                );
            }
        }
        let mut frame = LrFrame::from_cdef(
            cdef_frame(&workspace),
            active,
            [false; 3],
            initializations,
            target,
        )
        .unwrap();
        for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
            if !active[plane.index()] {
                continue;
            }
            let output = match plane {
                PlaneId::Y => frame.post_lr_y.as_mut(),
                PlaneId::U => frame.post_lr_u.as_mut(),
                PlaneId::V => frame.post_lr_v.as_mut(),
            }
            .and_then(StripeOutputPlane::as_u16_mut)
            .unwrap();
            assert!(output.samples().iter().all(|&sample| sample == POISON));
            write_blocks(
                output,
                block_slices[plane.index()],
                700 + plane.index() as u16,
            );
            assert!(output.samples().iter().all(|&sample| sample != POISON));
        }
        let mut filtered = frame.into_filtered();
        filtered.y.finish_direct().unwrap();
        if let Some(plane) = filtered.u.as_mut() {
            plane.finish_direct().unwrap();
        }
        if let Some(plane) = filtered.v.as_mut() {
            plane.finish_direct().unwrap();
        }
        drop(filtered);
        assert!(lease.submit());
        let frame = progress.freeze_workspace(core::convert::identity).unwrap();
        for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
            if let Some(plane) = frame.plane(plane) {
                assert!(plane.samples().iter().all(|&sample| sample != POISON));
            }
        }
    }
}

#[test]
fn a_coverage_hole_selects_copy_all_and_replaces_poison_with_cdef() {
    let workspace = workspace(PixelFormat::Monochrome, 8, 8);
    let progress = Arc::new(
        crate::pipeline::frame_progress::FrameProgress::<u16>::new(workspace.info()).unwrap(),
    );
    assert!(progress.begin(&[(0, 8)]));
    poison_progress(&progress);
    let mut lease = progress.direct_stripe(0).unwrap();
    let target = lease.take_target().unwrap();
    let mut blocks = blocks_for_target(target.get(PlaneId::Y).unwrap(), PlaneId::Y);
    blocks.remove(1);
    let core = lr_core([
        FrameRestorationType::WienerNonsep,
        FrameRestorationType::None,
        FrameRestorationType::None,
    ]);
    let initializations =
        lr_initializations(&core, [true, false, false], [&blocks, &[], &[]], &target);
    assert_eq!(initializations[0], StripeInitialization::CopyAll);
    let frame = LrFrame::from_cdef(
        cdef_frame(&workspace),
        [true, false, false],
        [false; 3],
        initializations,
        target,
    )
    .unwrap();
    assert_eq!(
        frame
            .post_lr_y
            .as_ref()
            .unwrap()
            .as_u16()
            .unwrap()
            .samples(),
        workspace.samples(PlaneId::Y).unwrap()
    );
}

#[test]
fn every_post_lr_plane_is_preflighted_before_any_target_mutation() {
    let workspace = workspace(PixelFormat::Yuv420, 8, 8);
    let progress = Arc::new(
        crate::pipeline::frame_progress::FrameProgress::<u16>::new(workspace.info()).unwrap(),
    );
    assert!(progress.begin(&[(0, 8)]));
    poison_progress(&progress);
    let mut lease = progress.direct_stripe(0).unwrap();
    let mut target = lease.take_target().unwrap();
    target.shorten_for_test(PlaneId::V);
    assert!(
        LrFrame::from_cdef(
            cdef_frame(&workspace),
            [true; 3],
            [false; 3],
            [StripeInitialization::FullyOverwritten; 3],
            target,
        )
        .is_err()
    );
    drop(lease);

    let mut lease = progress.direct_stripe(0).unwrap();
    let mut target = lease.take_target().unwrap();
    for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
        assert!(
            target
                .take(plane)
                .unwrap()
                .u16_samples_mut()
                .unwrap()
                .iter()
                .all(|&sample| sample == POISON),
            "plane {plane:?} changed before V preflight failed"
        );
    }
}

fn lossless_block() -> crate::filters::deblock::DeblockBlock {
    let prediction = crate::filters::deblock::DeblockPredictionUnit {
        base_r: 0,
        base_c: 0,
        default_sub_pu_tx: 0,
    };
    crate::filters::deblock::DeblockBlock {
        r: 0,
        c: 0,
        luma_prediction: prediction,
        chroma_prediction: prediction,
        chroma_base_r: 0,
        chroma_base_c: 0,
        n4w: 1,
        n4h: 1,
        luma_tx: 0,
        chroma_tx: Some(0),
        sub_pu_size: None,
        chroma_transform_only: false,
        qindex: 0,
        skip: false,
        lossless: true,
    }
}

#[test]
fn lossless_samples_are_restored_inside_a_fully_overwritten_stripe() {
    let workspace = workspace(PixelFormat::Monochrome, 8, 8);
    let records = [lossless_block()];
    let lossless = crate::filters::lossless::LosslessBlockGrid::from_deblock_blocks(
        2,
        2,
        &records,
        [&[], &[]],
    )
    .unwrap();
    let plane_sizes = [PlaneId::Y, PlaneId::U, PlaneId::V].map(|plane| {
        workspace
            .plane(plane)
            .ok()
            .map(splot_recon::CurrentFramePlane::storage_size)
    });
    let chain = StripeChain {
        bit_depth: BitDepth::Ten,
        cfl_ds_filter_index: 0,
        luma_width: 8,
        luma_height: 8,
        pixel_format: PixelFormat::Monochrome,
        cdef_grid: None,
        ccso_grid: None,
        gdf_grid: None,
        tx_skip_grid: None,
        gdf_reference: None,
        lossless_grid: Some(&lossless),
        plane_sizes,
        max_sample_fits: true,
    };
    let progress = Arc::new(
        crate::pipeline::frame_progress::FrameProgress::<u16>::new(workspace.info()).unwrap(),
    );
    assert!(progress.begin(&[(0, 8)]));
    poison_progress(&progress);
    let mut lease = progress.direct_stripe(0).unwrap();
    let target = lease.take_target().unwrap();
    let block = block(PlaneId::Y, LrUnitRestorationType::WienerNonsep, 0, 0, 8, 8);
    assert!(lr_plane_fully_overwritten(
        &[block],
        PlaneId::Y,
        FrameRestorationType::WienerNonsep,
        8,
        8,
        0,
        8,
    ));
    let mut frame = LrFrame::from_cdef(
        cdef_frame(&workspace),
        [true, false, false],
        [false; 3],
        [
            StripeInitialization::FullyOverwritten,
            StripeInitialization::CopyAll,
            StripeInitialization::CopyAll,
        ],
        target,
    )
    .unwrap();
    let output = frame
        .post_lr_y
        .as_mut()
        .and_then(StripeOutputPlane::as_u16_mut)
        .unwrap();
    let curr = FramePlane::new(&workspace, PlaneId::Y).unwrap();
    filter_lr_block_into::<u16>(output, &block, |samples, stride| {
        for row in 0..block.height {
            samples[row * stride..row * stride + block.width].fill(777);
        }
        chain.preserve_lossless_lr_samples(
            PlaneId::Y,
            &block,
            curr,
            samples,
            stride,
            |slot, sample| *slot = sample,
        )
    })
    .unwrap();
    for y in 0..8 {
        for x in 0..8 {
            let actual = output.samples()[y * 8 + x];
            let expected = if x < 4 && y < 4 {
                workspace.samples(PlaneId::Y).unwrap()[y * 8 + x]
            } else {
                777
            };
            assert_eq!(actual, expected, "sample ({x}, {y})");
        }
    }
}

#[test]
fn u8_lossless_luma_samples_write_exactly_to_both_output_representations() {
    let workspace = workspace_with(
        BitDepth::Eight,
        PixelFormat::Monochrome,
        8,
        8,
        |x, y, plane| 20 + ((x * 7 + y * 11 + plane.index() * 23) % 200) as u8,
    );
    let records = [lossless_block()];
    let lossless = crate::filters::lossless::LosslessBlockGrid::from_deblock_blocks(
        2,
        2,
        &records,
        [&[], &[]],
    )
    .unwrap();
    let plane_sizes = [PlaneId::Y, PlaneId::U, PlaneId::V].map(|plane| {
        workspace
            .plane(plane)
            .ok()
            .map(splot_recon::CurrentFramePlane::storage_size)
    });
    let chain = StripeChain {
        bit_depth: BitDepth::Eight,
        cfl_ds_filter_index: 0,
        luma_width: 8,
        luma_height: 8,
        pixel_format: PixelFormat::Monochrome,
        cdef_grid: None,
        ccso_grid: None,
        gdf_grid: None,
        tx_skip_grid: None,
        gdf_reference: None,
        lossless_grid: Some(&lossless),
        plane_sizes,
        max_sample_fits: true,
    };
    let block = block(PlaneId::Y, LrUnitRestorationType::WienerNonsep, 0, 0, 8, 8);
    let curr = FramePlane::new(&workspace, PlaneId::Y).unwrap();
    let mut output = [777u16; 64];
    let mut direct = [0xeeu8; 64];

    chain
        .preserve_lossless_lr_samples(PlaneId::Y, &block, curr, &mut output, 8, |slot, sample| {
            *slot = sample.to_u16();
        })
        .unwrap();
    chain
        .preserve_lossless_lr_samples(PlaneId::Y, &block, curr, &mut direct, 8, |slot, sample| {
            *slot = sample;
        })
        .unwrap();

    for y in 0..8 {
        for x in 0..8 {
            let expected = if x < 4 && y < 4 {
                u16::from(workspace.samples(PlaneId::Y).unwrap()[y * 8 + x])
            } else {
                777
            };
            assert_eq!(output[y * 8 + x], expected, "sample ({x}, {y})");
            let expected = if x < 4 && y < 4 {
                workspace.samples(PlaneId::Y).unwrap()[y * 8 + x]
            } else {
                0xee
            };
            assert_eq!(direct[y * 8 + x], expected, "direct sample ({x}, {y})");
        }
    }
}

#[test]
#[should_panic(expected = "omitted LR block write escaped poison oracle")]
fn poison_oracle_exposes_one_suppressed_lr_block_write() {
    let workspace = workspace(PixelFormat::Monochrome, 8, 8);
    let progress = Arc::new(
        crate::pipeline::frame_progress::FrameProgress::<u16>::new(workspace.info()).unwrap(),
    );
    assert!(progress.begin(&[(0, 8)]));
    poison_progress(&progress);
    let mut lease = progress.direct_stripe(0).unwrap();
    let target = lease.take_target().unwrap();
    let blocks = blocks_for_target(target.get(PlaneId::Y).unwrap(), PlaneId::Y);
    let mut frame = LrFrame::from_cdef(
        cdef_frame(&workspace),
        [true, false, false],
        [false; 3],
        [
            StripeInitialization::FullyOverwritten,
            StripeInitialization::CopyAll,
            StripeInitialization::CopyAll,
        ],
        target,
    )
    .unwrap();
    let output = frame
        .post_lr_y
        .as_mut()
        .and_then(StripeOutputPlane::as_u16_mut)
        .unwrap();
    write_blocks(output, &blocks[1..], 777);
    assert!(
        output.samples().iter().all(|&sample| sample != POISON),
        "omitted LR block write escaped poison oracle"
    );
}

#[test]
#[should_panic(expected = "omitted direct-u8 LR block write escaped poison oracle")]
fn direct_u8_poison_oracle_exposes_one_suppressed_lr_block_write() {
    let workspace = workspace_with(BitDepth::Eight, PixelFormat::Monochrome, 8, 8, |x, y, _| {
        ((x * 17 + y * 29) % 256) as u8
    });
    let progress = Arc::new(
        crate::pipeline::frame_progress::FrameProgress::<u8>::new(workspace.info()).unwrap(),
    );
    assert!(progress.begin(&[(0, 8)]));
    let mut poison_lease = progress.direct_stripe(0).unwrap();
    let mut poison_target = poison_lease.take_target().unwrap();
    poison_target
        .take(PlaneId::Y)
        .unwrap()
        .u8_samples_mut()
        .unwrap()
        .fill(0xde);
    drop(poison_target);
    drop(poison_lease);

    let mut lease = progress.direct_stripe(0).unwrap();
    let mut target = lease.take_target().unwrap();
    let target = target.take(PlaneId::Y).unwrap();
    let source =
        StripePlane::copy_from(FramePlane::new(&workspace, PlaneId::Y).unwrap(), 0, 8).unwrap();
    let blocks = blocks_for_target(&target, PlaneId::Y);
    let mut output = StripeOutputPlane::direct_u8(target, &source).unwrap();
    for block in &blocks[1..] {
        let rect = PlaneRect::new(block.x, block.y, block.width, block.height).unwrap();
        let (samples, stride) = output.u8_rect_mut(rect).unwrap();
        for row in 0..block.height {
            samples[row * stride..row * stride + block.width].fill(77);
        }
    }
    assert!(
        match output {
            StripeOutputPlane::DirectU8(mut target) => target
                .u8_samples_mut()
                .unwrap()
                .iter()
                .all(|&sample| sample != 0xde),
            StripeOutputPlane::U16(_) => false,
        },
        "omitted direct-u8 LR block write escaped poison oracle"
    );
}

#[test]
fn gdf_active_luma_keeps_the_full_overwrite_initialization() {
    let workspace = workspace(PixelFormat::Monochrome, 8, 8);
    let mut core = lr_core([
        FrameRestorationType::WienerNonsep,
        FrameRestorationType::None,
        FrameRestorationType::None,
    ]);
    let gdf = core.gdf_params.as_mut().unwrap();
    gdf.gdf_frame_enable = true;
    gdf.gdf_per_block = Some(false);
    gdf.gdf_pic_qc_idx = Some(0);
    gdf.gdf_pic_scale_idx = Some(0);
    let progress = Arc::new(
        crate::pipeline::frame_progress::FrameProgress::<u16>::new(workspace.info()).unwrap(),
    );
    assert!(progress.begin(&[(0, 8)]));
    let mut lease = progress.direct_stripe(0).unwrap();
    let target = lease.take_target().unwrap();
    let blocks = blocks_for_target(target.get(PlaneId::Y).unwrap(), PlaneId::Y);
    assert_eq!(
        lr_initializations(&core, [true, false, false], [&blocks, &[], &[]], &target)[0],
        StripeInitialization::FullyOverwritten
    );
}

#[test]
fn terminal_luma_direct_u8_eligibility_keeps_every_non_class_a_fallback() {
    let workspace_u8 = workspace_with(BitDepth::Eight, PixelFormat::Monochrome, 9, 7, |x, y, _| {
        ((x * 17 + y * 29) % 256) as u8
    });
    let progress = Arc::new(
        crate::pipeline::frame_progress::FrameProgress::<u8>::new(workspace_u8.info()).unwrap(),
    );
    assert!(progress.begin(&[(0, 7)]));
    let mut lease = progress.direct_stripe(0).unwrap();
    let target = lease.take_target().unwrap();
    let target = target.get(PlaneId::Y).unwrap();
    let mut blocks = blocks_for_target(target, PlaneId::Y);

    assert!(terminal_luma_wiener_direct_u8(
        BitDepth::Eight,
        Some(FrameRestorationType::WienerNonsep),
        false,
        &blocks,
        target,
    ));
    assert!(!terminal_luma_wiener_direct_u8(
        BitDepth::Eight,
        Some(FrameRestorationType::WienerNonsep),
        true,
        &blocks,
        target,
    ));
    assert!(!terminal_luma_wiener_direct_u8(
        BitDepth::Eight,
        Some(FrameRestorationType::PcWiener),
        false,
        &blocks,
        target,
    ));
    blocks.remove(1);
    assert!(!terminal_luma_wiener_direct_u8(
        BitDepth::Eight,
        Some(FrameRestorationType::WienerNonsep),
        false,
        &blocks,
        target,
    ));

    let workspace = workspace(PixelFormat::Monochrome, 9, 7);
    let progress = Arc::new(
        crate::pipeline::frame_progress::FrameProgress::<u16>::new(workspace.info()).unwrap(),
    );
    assert!(progress.begin(&[(0, 7)]));
    let mut lease = progress.direct_stripe(0).unwrap();
    let target = lease.take_target().unwrap();
    let target = target.get(PlaneId::Y).unwrap();
    let blocks = blocks_for_target(target, PlaneId::Y);
    assert!(!terminal_luma_wiener_direct_u8(
        BitDepth::Ten,
        Some(FrameRestorationType::WienerNonsep),
        false,
        &blocks,
        target,
    ));
}

#[test]
fn active_gdf_state_is_checked_before_output_selection() {
    let mut core = lr_core([
        FrameRestorationType::WienerNonsep,
        FrameRestorationType::None,
        FrameRestorationType::None,
    ]);
    let gdf = core.gdf_params.as_mut().unwrap();
    gdf.gdf_frame_enable = true;
    gdf.gdf_per_block = Some(false);
    gdf.gdf_pic_qc_idx = Some(0);
    gdf.gdf_pic_scale_idx = Some(0);
    assert!(crate::filters::gdf::is_active(&core, None, BitDepth::Eight, None).unwrap());

    core.gdf_params.as_mut().unwrap().gdf_per_block = None;
    assert!(crate::filters::gdf::is_active(&core, None, BitDepth::Eight, None).is_err());
}

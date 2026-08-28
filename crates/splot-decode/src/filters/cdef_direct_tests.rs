// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use std::sync::Arc;

use splot_recon::{
    BitDepth, CurrentFrameWorkspace, DecodedFrameInfo, OutputIndex, PixelFormat, PlaneId,
    PlaneRect, PlaneSize,
};

use super::tests::{constant_cdef_grid, deblock_block};
use super::*;
use crate::filters::source::DeblockedPlanes;
use crate::pipeline::frame_progress::{DirectStripeTarget, FrameProgress};
use crate::test_support::yuv420_workspace;

fn poison_u16_target(target: &mut DirectStripeTarget, poison: u16) {
    for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
        target
            .take(plane)
            .unwrap()
            .u16_samples_mut()
            .unwrap()
            .fill(poison);
    }
}

fn poison_u16_progress(progress: &Arc<FrameProgress<u16>>, poison: u16) {
    let mut lease = progress.direct_stripe(0).unwrap();
    let mut target = lease.take_target().unwrap();
    poison_u16_target(&mut target, poison);
}

fn patterned_10bit_workspace(
    pixel_format: PixelFormat,
    width: usize,
    height: usize,
) -> CurrentFrameWorkspace<u16> {
    let info = DecodedFrameInfo::new(
        OutputIndex::new(0),
        BitDepth::Ten,
        pixel_format,
        PlaneSize::new(width, height).unwrap(),
        PlaneRect::new(0, 0, width, height).unwrap(),
    )
    .unwrap();
    let mut workspace = CurrentFrameWorkspace::new(info, 0_u16).unwrap();
    for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
        let size = workspace.plane(plane).unwrap().storage_size();
        for y in 0..size.height() {
            for x in 0..size.width() {
                let sample = 320 + ((x * 17 + y * 29 + plane.index() * 43) % 384) as u16;
                workspace
                    .set_reconstructed_sample(plane, x, y, sample)
                    .unwrap();
            }
        }
    }
    workspace
}

fn cdef_frame_samples(frame: &CdefFrame<'_, u16>) -> [Vec<u16>; 3] {
    [
        frame.filtered_y.samples().to_vec(),
        frame.filtered_u.as_ref().unwrap().samples().to_vec(),
        frame.filtered_v.as_ref().unwrap().samples().to_vec(),
    ]
}

fn direct_cdef_10bit(
    workspace: &CurrentFrameWorkspace<u16>,
    params: &[CdefFrameParams],
    grid: &CdefUnitGrid,
    skip_grid: Option<&CdefSkipGrid>,
    lossless_grid: Option<&crate::filters::lossless::LosslessBlockGrid>,
    expected_initialization: StripeInitialization,
) -> [Vec<u16>; 3] {
    let size = workspace.plane(PlaneId::Y).unwrap().storage_size();
    let pixel_format = workspace.info().pixel_format();
    let subsampling = (
        usize::from(pixel_format.subsampling_x()),
        usize::from(pixel_format.subsampling_y()),
    );
    let mi_size = (
        size.height().div_ceil(MI_SIZE),
        size.width().div_ceil(MI_SIZE),
    );
    let progress = Arc::new(FrameProgress::<u16>::new(workspace.info()).unwrap());
    assert!(progress.begin(&[(0, size.height())]));
    poison_u16_progress(&progress, 0xdead);
    let mut lease = progress.direct_stripe(0).unwrap();
    let target = lease.take_target().unwrap();
    let lookup = CdefBlockLookup {
        strengths: params,
        grid,
        tile_row_starts: None,
        tile_col_starts: None,
        skip_grid,
        lossless_grid,
        mi_rows: mi_size.0,
        mi_cols: mi_size.1,
        sub_x: subsampling.0,
        sub_y: subsampling.1,
        has_chroma: true,
        coeff_shift: 2,
        max_sample: 1023,
    };
    let geometry = [
        Some(CdefPlaneGeometry {
            width: size.width(),
            frame_height: size.height(),
            origin_y: 0,
            end_y: size.height(),
        }),
        workspace.plane(PlaneId::U).ok().map(|plane| {
            let size = plane.storage_size();
            CdefPlaneGeometry {
                width: size.width(),
                frame_height: size.height(),
                origin_y: 0,
                end_y: size.height(),
            }
        }),
        workspace.plane(PlaneId::V).ok().map(|plane| {
            let size = plane.storage_size();
            CdefPlaneGeometry {
                width: size.width(),
                frame_height: size.height(),
                origin_y: 0,
                end_y: size.height(),
            }
        }),
    ];
    assert_eq!(
        cdef_initializations(Some(&lookup), Some(&target), geometry, (0, size.height())).unwrap(),
        [expected_initialization; 3]
    );
    let frame = cdef_stripe_into(
        DeblockedPlanes::frame(workspace).unwrap(),
        Some(params),
        Some(grid),
        skip_grid,
        lossless_grid,
        mi_size,
        subsampling,
        BitDepth::Ten,
        None,
        0,
        size.height(),
        Some(target),
    )
    .unwrap();
    let samples = cdef_frame_samples(&frame);
    drop(frame);
    assert!(lease.submit());
    let frame = progress.freeze_workspace(core::convert::identity).unwrap();
    for (plane, expected) in [PlaneId::Y, PlaneId::U, PlaneId::V]
        .into_iter()
        .zip(&samples)
    {
        let actual = match plane {
            PlaneId::Y => frame.y().samples(),
            PlaneId::U => frame.u().unwrap().samples(),
            PlaneId::V => frame.v().unwrap().samples(),
        };
        assert_eq!(actual, expected);
    }
    samples
}

fn owned_cdef_10bit(
    workspace: &CurrentFrameWorkspace<u16>,
    params: &[CdefFrameParams],
    grid: &CdefUnitGrid,
    skip_grid: Option<&CdefSkipGrid>,
    lossless_grid: Option<&crate::filters::lossless::LosslessBlockGrid>,
) -> [Vec<u16>; 3] {
    let size = workspace.plane(PlaneId::Y).unwrap().storage_size();
    let pixel_format = workspace.info().pixel_format();
    let subsampling = (
        usize::from(pixel_format.subsampling_x()),
        usize::from(pixel_format.subsampling_y()),
    );
    let frame = cdef_stripe(
        DeblockedPlanes::frame(workspace).unwrap(),
        Some(params),
        Some(grid),
        skip_grid,
        lossless_grid,
        (
            size.height().div_ceil(MI_SIZE),
            size.width().div_ceil(MI_SIZE),
        ),
        subsampling,
        BitDepth::Ten,
        None,
        0,
        size.height(),
    )
    .unwrap();
    cdef_frame_samples(&frame)
}

fn active_params() -> [CdefFrameParams; 1] {
    [CdefFrameParams {
        y_pri: 4,
        y_sec: 4,
        uv_pri: 2,
        uv_sec: 4,
        damping: 4,
    }]
}

#[test]
fn complete_direct_u16_cdef_matches_owned_on_odd_edges() {
    let workspace = patterned_10bit_workspace(PixelFormat::Yuv420, 20, 18);
    let params = active_params();
    let grid = constant_cdef_grid(5, 5, 0).unwrap();
    assert_eq!(
        direct_cdef_10bit(
            &workspace,
            &params,
            &grid,
            None,
            None,
            StripeInitialization::FullyOverwritten
        ),
        owned_cdef_10bit(&workspace, &params, &grid, None, None)
    );
}

#[test]
fn complete_direct_u16_cdef_matches_owned_for_chroma_subsampling() {
    let params = active_params();
    let grid = constant_cdef_grid(5, 5, 0).unwrap();
    for pixel_format in [PixelFormat::Yuv422, PixelFormat::Yuv444] {
        let workspace = patterned_10bit_workspace(pixel_format, 20, 18);
        assert_eq!(
            direct_cdef_10bit(
                &workspace,
                &params,
                &grid,
                None,
                None,
                StripeInitialization::FullyOverwritten
            ),
            owned_cdef_10bit(&workspace, &params, &grid, None, None),
            "{pixel_format:?}"
        );
    }
}

#[test]
fn one_disabled_cdef_unit_keeps_copy_initialization() {
    let workspace = patterned_10bit_workspace(PixelFormat::Yuv420, 68, 18);
    let params = active_params();
    let grid = CdefUnitGrid::new(1, 2, vec![Some(0), None]).unwrap();
    assert_eq!(
        direct_cdef_10bit(
            &workspace,
            &params,
            &grid,
            None,
            None,
            StripeInitialization::CopyAll
        ),
        owned_cdef_10bit(&workspace, &params, &grid, None, None)
    );
}

#[test]
#[should_panic(expected = "omitted block write escaped poison oracle")]
fn poison_oracle_rejects_one_omitted_block_write() {
    let workspace = patterned_10bit_workspace(PixelFormat::Yuv420, 16, 16);
    let params = active_params();
    let grid = constant_cdef_grid(4, 4, 0).unwrap();
    let mut mutated = direct_cdef_10bit(
        &workspace,
        &params,
        &grid,
        None,
        None,
        StripeInitialization::FullyOverwritten,
    );
    for row in 0..8 {
        mutated[PlaneId::Y.index()][row * 16..row * 16 + 8].fill(0xdead);
    }
    assert_eq!(
        mutated,
        owned_cdef_10bit(&workspace, &params, &grid, None, None),
        "omitted block write escaped poison oracle"
    );
}

#[test]
fn partial_skip_and_lossless_cdef_keep_poison_out_of_direct_output() {
    let workspace = patterned_10bit_workspace(PixelFormat::Yuv420, 20, 18);
    let params = active_params();
    let grid = constant_cdef_grid(5, 5, 0).unwrap();
    let mut skipped = vec![false; 25];
    for index in [0, 1, 5, 6] {
        skipped[index] = true;
    }
    let skip_grid = CdefSkipGrid::new(5, 5, skipped).unwrap();
    assert_eq!(
        direct_cdef_10bit(
            &workspace,
            &params,
            &grid,
            Some(&skip_grid),
            None,
            StripeInitialization::CopyAll
        ),
        owned_cdef_10bit(&workspace, &params, &grid, Some(&skip_grid), None)
    );

    let blocks = [deblock_block(0, 0, 2, 2, true)];
    let lossless = crate::filters::lossless::LosslessBlockGrid::from_deblock_blocks(
        5,
        5,
        &blocks,
        [&blocks, &blocks],
    )
    .unwrap();
    assert_eq!(
        direct_cdef_10bit(
            &workspace,
            &params,
            &grid,
            None,
            Some(&lossless),
            StripeInitialization::CopyAll
        ),
        owned_cdef_10bit(&workspace, &params, &grid, None, Some(&lossless))
    );
}

#[test]
fn disabled_cdef_initializes_u8_direct_staging_from_source() {
    let workspace = yuv420_workspace(18, 14, 91);
    let height = workspace.plane(PlaneId::Y).unwrap().storage_size().height();
    let progress = Arc::new(FrameProgress::<u8>::new(workspace.info()).unwrap());
    assert!(progress.begin(&[(0, height)]));
    {
        let mut lease = progress.direct_stripe(0).unwrap();
        let mut target = lease.take_target().unwrap();
        for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
            target
                .take(plane)
                .unwrap()
                .u8_samples_mut()
                .unwrap()
                .fill(0xde);
        }
    }
    let mut lease = progress.direct_stripe(0).unwrap();
    let target = lease.take_target().unwrap();
    let mut frame = cdef_stripe_into(
        DeblockedPlanes::frame(&workspace).unwrap(),
        None,
        None,
        None,
        None,
        (height.div_ceil(MI_SIZE), 18_usize.div_ceil(MI_SIZE)),
        (1, 1),
        BitDepth::Eight,
        None,
        0,
        height,
        Some(target),
    )
    .unwrap();
    for filtered in [
        Some(&mut frame.filtered_y),
        frame.filtered_u.as_mut(),
        frame.filtered_v.as_mut(),
    ]
    .into_iter()
    .flatten()
    {
        assert!(filtered.samples().iter().all(|&sample| sample == 91));
        filtered.finish_direct().unwrap();
    }
    drop(frame);
    assert!(lease.submit());
    let frame = progress.freeze_workspace(core::convert::identity).unwrap();
    assert!(frame.y().samples().iter().all(|&sample| sample == 91));
    assert!(
        frame
            .u()
            .unwrap()
            .samples()
            .iter()
            .all(|&sample| sample == 91)
    );
    assert!(
        frame
            .v()
            .unwrap()
            .samples()
            .iter()
            .all(|&sample| sample == 91)
    );
}

#[test]
fn every_direct_plane_is_preflighted_before_luma_mutation() {
    let workspace = patterned_10bit_workspace(PixelFormat::Yuv420, 16, 16);
    let params = active_params();
    let grid = constant_cdef_grid(4, 4, 0).unwrap();
    let progress = Arc::new(FrameProgress::<u16>::new(workspace.info()).unwrap());
    assert!(progress.begin(&[(0, 16)]));
    poison_u16_progress(&progress, 0xdead);

    let mut lease = progress.direct_stripe(0).unwrap();
    let mut target = lease.take_target().unwrap();
    target.shorten_for_test(PlaneId::V);
    assert!(
        cdef_stripe_into(
            DeblockedPlanes::frame(&workspace).unwrap(),
            Some(&params),
            Some(&grid),
            None,
            None,
            (4, 4),
            (1, 1),
            BitDepth::Ten,
            None,
            0,
            16,
            Some(target),
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
                .all(|&sample| sample == 0xdead),
            "plane {plane:?} changed before V preflight failed"
        );
    }
}

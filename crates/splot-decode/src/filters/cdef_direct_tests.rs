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
    progress.begin(&[(0, size.height())]).unwrap();
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
        fill_flat: [true; 3],
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
    progress.begin(&[(0, height)]).unwrap();
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
    progress.begin(&[(0, 16)]).unwrap();
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

fn flat_workspace(
    width: usize,
    columns: core::ops::Range<usize>,
    spike: Option<(PlaneId, usize, usize)>,
) -> CurrentFrameWorkspace<u16> {
    let mut workspace = patterned_10bit_workspace(PixelFormat::Yuv420, width, 24);
    for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
        let shift = usize::from(plane != PlaneId::Y);
        for y in (4 >> shift)..(20 >> shift) {
            for x in (columns.start >> shift)..(columns.end >> shift) {
                workspace
                    .set_reconstructed_sample(plane, x, y, 512)
                    .unwrap();
            }
        }
    }
    if let Some((plane, x, y)) = spike {
        workspace
            .set_reconstructed_sample(plane, x, y, 528)
            .unwrap();
    }
    workspace
}

fn assert_flat(
    samples: &[u16],
    width: usize,
    columns: core::ops::Range<usize>,
    rows: core::ops::Range<usize>,
) {
    for y in rows {
        assert!(
            samples[y * width + columns.start..y * width + columns.end]
                .iter()
                .all(|&sample| sample == 512)
        );
    }
}

#[test]
fn flat_segment_fills_fully_overwritten_direct_output() {
    let workspace = flat_workspace(144, 60..132, None);
    let params = active_params();
    let grid = constant_cdef_grid(6, 36, 0).unwrap();
    let owned = owned_cdef_10bit(&workspace, &params, &grid, None, None);
    assert_eq!(
        direct_cdef_10bit(
            &workspace,
            &params,
            &grid,
            None,
            None,
            StripeInitialization::FullyOverwritten
        ),
        owned
    );
    assert_flat(&owned[0], 144, 64..128, 8..16);
    assert_flat(&owned[1], 72, 32..64, 4..8);
    assert_flat(&owned[2], 72, 32..64, 4..8);
}

#[test]
fn flat_segment_with_one_tap_reach_spike_is_filtered() {
    let workspace = flat_workspace(144, 60..132, Some((PlaneId::Y, 62, 11)));
    let grid = constant_cdef_grid(6, 36, 0).unwrap();
    let owned = owned_cdef_10bit(&workspace, &active_params(), &grid, None, None);
    assert_eq!(owned[PlaneId::Y.index()][11 * 144 + 64], 513);
}

#[test]
fn flat_window_spans_exactly_the_tap_reach() {
    let tile = ((0, 0), (144, 24));
    let window = |spike: Option<(usize, usize)>, start: (usize, usize)| {
        let workspace = flat_workspace(144, 60..132, spike.map(|(x, y)| (PlaneId::Y, x, y)));
        let plane = FramePlane::new(&workspace, PlaneId::Y).unwrap();
        flat_window::<u16, 64, 8>(plane, (64, 8), start, tile.1)
    };
    assert_eq!(window(None, tile.0), Some(512));
    for corner in [(62, 6), (129, 6), (62, 17), (129, 17)] {
        assert_eq!(window(Some(corner), tile.0), None, "{corner:?}");
    }
    for outside in [(61, 11), (130, 11), (64, 5), (64, 18)] {
        assert_eq!(window(Some(outside), tile.0), Some(512), "{outside:?}");
    }
    assert_eq!(window(None, (63, 0)), None);
    for (spike, expected) in [
        (None, Some(90)),
        (Some((129, 17)), None),
        (Some((130, 11)), Some(90)),
    ] {
        let mut workspace = yuv420_workspace(144, 24, 90);
        if let Some((x, y)) = spike {
            workspace
                .set_reconstructed_sample(PlaneId::Y, x, y, 91)
                .unwrap();
        }
        let plane = FramePlane::new(&workspace, PlaneId::Y).unwrap();
        assert_eq!(
            flat_window::<u8, 64, 8>(plane, (64, 8), tile.0, tile.1),
            expected,
            "{spike:?}"
        );
    }
}

#[test]
fn flat_blocks_fill_fully_overwritten_direct_output() {
    let workspace = flat_workspace(32, 4..20, None);
    let params = active_params();
    let grid = constant_cdef_grid(6, 8, 0).unwrap();
    let owned = owned_cdef_10bit(&workspace, &params, &grid, None, None);
    assert_eq!(
        direct_cdef_10bit(
            &workspace,
            &params,
            &grid,
            None,
            None,
            StripeInitialization::FullyOverwritten
        ),
        owned
    );
    assert_flat(&owned[0], 32, 8..16, 8..16);
    assert_flat(&owned[1], 16, 4..8, 4..8);
    assert_flat(&owned[2], 16, 4..8, 4..8);
}

#[test]
fn chroma_pair_with_one_flat_plane_is_filtered() {
    let workspace = flat_workspace(32, 4..20, Some((PlaneId::V, 2, 5)));
    let grid = constant_cdef_grid(6, 8, 0).unwrap();
    let owned = owned_cdef_10bit(&workspace, &active_params(), &grid, None, None);
    assert_eq!(owned[PlaneId::V.index()][5 * 16 + 4], 513);
    assert_flat(&owned[1], 16, 4..8, 4..8);
}

/// A flat block writes nothing into a direct target that holds the deblocked
/// rows, on the per-block path (width 32) and the segment path (width 144).
#[test]
fn flat_blocks_keep_a_deblocked_direct_target() {
    const POISON: u16 = 1023;
    let params = active_params();
    for (width, flat, block) in [(32, 4..20, 8..16), (144, 52..68, 56..64)] {
        let workspace = flat_workspace(width, flat.clone(), None);
        let mi_size = (6, width / MI_SIZE);
        let grid = constant_cdef_grid(mi_size.0, mi_size.1, 0).unwrap();
        let filled = direct_cdef_10bit(
            &workspace,
            &params,
            &grid,
            None,
            None,
            StripeInitialization::FullyOverwritten,
        );
        let block_rows = |plane: usize| {
            let shift = usize::from(plane != 0);
            let xs = block.start >> shift..block.end >> shift;
            (width >> shift, xs, 8 >> shift..16 >> shift)
        };
        let mut held = flat_workspace(width, flat, None);
        for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
            let (_, xs, ys) = block_rows(plane.index());
            let rect = PlaneRect::new(xs.start, ys.start, xs.len(), ys.len()).unwrap();
            held.fill_rect(plane, rect, POISON).unwrap();
        }
        let progress = Arc::new(FrameProgress::<u16>::new(workspace.info()).unwrap());
        progress.begin(&[(0, 24)]).unwrap();
        let mut rows = progress.frontier_rows().unwrap();
        rows.copy_rows_from(&held, 0..24).unwrap();
        assert!(rows.publish_final_rows(24) && rows.release_rows(24));
        let mut lease = progress.direct_stripe(0).unwrap();
        let frame = cdef_stripe_into(
            DeblockedPlanes::frame(&workspace).unwrap(),
            Some(&params),
            Some(&grid),
            None,
            None,
            mi_size,
            (1, 1),
            BitDepth::Ten,
            None,
            0,
            24,
            lease.take_target(),
        )
        .unwrap();
        let mut kept = cdef_frame_samples(&frame);
        for (plane, samples) in kept.iter_mut().enumerate() {
            let (stride, xs, ys) = block_rows(plane);
            for y in ys {
                let row = &mut samples[y * stride + xs.start..y * stride + xs.end];
                assert!(
                    row.iter().all(|&sample| sample == POISON),
                    "{width} {plane}"
                );
                row.fill(512);
            }
        }
        assert_eq!(kept, filled, "{width}");
    }
}

#[test]
fn flat_pad_gives_direction_zero_and_variance_zero() {
    for value in [0, 37, 512, 1023] {
        assert_eq!(cdef_direction_padded(&[value; CDEF_PADDED_AREA], 2), (0, 0));
    }
}

/// Filters the segment at MI `(2, c)` of `workspace` through the segment path
/// or block by block, into a copy of the deblocked planes.
fn run_segment(
    workspace: &CurrentFrameWorkspace<u16>,
    params: CdefFrameParams,
    lossless: &crate::filters::lossless::LosslessBlockGrid,
    c: usize,
    segment: bool,
) -> [Vec<u16>; 3] {
    let deblocked = DeblockedPlanes::frame(workspace).unwrap();
    let mut frame = cdef_stripe(
        deblocked,
        None,
        None,
        None,
        None,
        (6, 36),
        (1, 1),
        BitDepth::Ten,
        None,
        0,
        24,
    )
    .unwrap();
    let grid = constant_cdef_grid(6, 36, 0).unwrap();
    let lookup = CdefBlockLookup {
        strengths: &[params],
        grid: &grid,
        tile_row_starts: None,
        tile_col_starts: None,
        skip_grid: None,
        lossless_grid: Some(lossless),
        mi_rows: 6,
        mi_cols: 36,
        sub_x: 1,
        sub_y: 1,
        has_chroma: true,
        coeff_shift: 2,
        max_sample: 1023,
        fill_flat: [true; 3],
    };
    if segment {
        let mut scratch = CdefSegmentScratch {
            luma: [0; LUMA_SEGMENT_AREA],
            pair: [0; PAIR_SEGMENT_AREA],
        };
        assert!(cdef_segment(&lookup, params, (2, c), (0, 6), &mut scratch, &mut frame).unwrap());
    } else {
        let mut pad = [0u16; CDEF_PADDED_AREA];
        for block in 0..CDEF_SEGMENT_BLOCKS {
            let Some(ctx) = lookup.at(2, c + 2 * block, Some(params), (0, 6)).unwrap() else {
                continue;
            };
            compute_cdef_block::<u16>(
                &ctx,
                &mut pad,
                deblocked.y,
                deblocked.u,
                deblocked.v,
                &mut frame.filtered_y,
                frame.filtered_u.as_mut(),
                frame.filtered_v.as_mut(),
            )
            .unwrap();
        }
    }
    cdef_frame_samples(&frame)
}

#[test]
fn segment_path_matches_per_block_path() {
    let workspace = flat_workspace(144, 52..68, None);
    let input = run_segment(
        &workspace,
        active_params()[0],
        &crate::filters::lossless::LosslessBlockGrid::from_deblock_blocks(6, 36, &[], [&[], &[]])
            .unwrap(),
        8,
        false,
    );
    let blocks = [deblock_block(2, 10, 2, 2, true)];
    let lossless = crate::filters::lossless::LosslessBlockGrid::from_deblock_blocks(
        6,
        36,
        &blocks,
        [&blocks, &blocks],
    )
    .unwrap();
    let mixed = |y_pri, y_sec, uv_pri, uv_sec| CdefFrameParams {
        y_pri,
        y_sec,
        uv_pri,
        uv_sec,
        damping: 4,
    };
    for params in [
        active_params()[0],
        mixed(0, 0, 3, 0),
        mixed(7, 0, 0, 2),
        mixed(0, 3, 0, 0),
    ] {
        let segment = run_segment(&workspace, params, &lossless, 8, true);
        assert_eq!(
            segment,
            run_segment(&workspace, params, &lossless, 8, false),
            "{params:?}"
        );
        assert_ne!(segment, input, "{params:?}");
    }
}

#[test]
fn edge_segment_falls_back_to_the_per_block_path() {
    let workspace = patterned_10bit_workspace(PixelFormat::Yuv420, 144, 24);
    let grid = constant_cdef_grid(6, 36, 0).unwrap();
    let lookup = CdefBlockLookup {
        strengths: &active_params(),
        grid: &grid,
        tile_row_starts: None,
        tile_col_starts: Some(&[0, 8, 36]),
        skip_grid: None,
        lossless_grid: None,
        mi_rows: 6,
        mi_cols: 36,
        sub_x: 1,
        sub_y: 1,
        has_chroma: true,
        coeff_shift: 2,
        max_sample: 1023,
        fill_flat: [true; 3],
    };
    let mut frame = cdef_stripe(
        DeblockedPlanes::frame(&workspace).unwrap(),
        None,
        None,
        None,
        None,
        (6, 36),
        (1, 1),
        BitDepth::Ten,
        None,
        0,
        24,
    )
    .unwrap();
    let before = cdef_frame_samples(&frame);
    let mut scratch = CdefSegmentScratch {
        luma: [0; LUMA_SEGMENT_AREA],
        pair: [0; PAIR_SEGMENT_AREA],
    };
    let params = active_params()[0];
    for (r, c) in [(0, 16), (2, 0), (2, 8), (2, 32), (4, 16)] {
        assert!(
            !cdef_segment(&lookup, params, (r, c), (0, 6), &mut scratch, &mut frame).unwrap(),
            "({r}, {c})"
        );
    }
    assert_eq!(cdef_frame_samples(&frame), before);
    assert!(cdef_segment(&lookup, params, (2, 16), (0, 6), &mut scratch, &mut frame).unwrap());
    assert_ne!(cdef_frame_samples(&frame), before);
}

#[test]
fn luma_segment_gather_returns_each_block_window_range() {
    let (width, stride) = (44, 48);
    let mut state = 0x2468_ace1u32;
    let samples: Vec<u16> = (0..stride * 14)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 22) as u16
        })
        .collect();
    let mut pad = [0u16; LUMA_SEGMENT_AREA];
    let ranges = gather_luma_segment(&samples, width, stride, &mut pad, (4, 3)).unwrap();
    for (block, range) in ranges.into_iter().enumerate() {
        let window = (1..13).flat_map(|y| {
            let start = y * stride + 2 + 8 * block;
            samples[start..start + 12].iter().copied()
        });
        let (min, max) = (window.clone().min().unwrap(), window.max().unwrap());
        assert_eq!(range, [min, max], "block {block}");
    }
    for row in 0..12 {
        let start = (row + 1) * stride + 2;
        assert_eq!(
            &pad[row * CDEF_SEGMENT_STRIDE..(row + 1) * CDEF_SEGMENT_STRIDE],
            &samples[start..start + CDEF_SEGMENT_STRIDE]
        );
    }
}

#[test]
fn segment_path_matches_per_block_path_on_narrow_windows() {
    let mut workspace = patterned_10bit_workspace(PixelFormat::Yuv420, 144, 24);
    for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
        let size = workspace.plane(plane).unwrap().storage_size();
        for y in 0..size.height() {
            for x in 0..size.width() {
                let sample = 700 + ((x * 37 + y * 59 + plane.index() * 11) % 241) as u16;
                workspace
                    .set_reconstructed_sample(plane, x, y, sample)
                    .unwrap();
            }
        }
    }
    let lossless =
        crate::filters::lossless::LosslessBlockGrid::from_deblock_blocks(6, 36, &[], [&[], &[]])
            .unwrap();
    for damping in [3, 4, 5, 6] {
        for (y_pri, y_sec) in [(4, 4), (15, 0), (0, 2), (1, 1)] {
            let params = CdefFrameParams {
                y_pri,
                y_sec,
                uv_pri: 2,
                uv_sec: 4,
                damping,
            };
            assert_eq!(
                run_segment(&workspace, params, &lossless, 8, true),
                run_segment(&workspace, params, &lossless, 8, false),
                "{params:?}"
            );
        }
    }
}

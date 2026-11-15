// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use splot_recon::{DecodedFrameInfo, OutputIndex, PlaneRect, PlaneSize};

use super::*;
use crate::bitstream::tile_payload::LumaCoeffBlock;

fn interior() -> NeighbourAvailability {
    NeighbourAvailability::new(true, true, 0, 0)
}

fn workspace_sized(
    pixel_format: PixelFormat,
    width: usize,
    height: usize,
) -> CurrentFrameWorkspace<u8> {
    let luma = PlaneSize::new(width, height).unwrap();
    let visible = PlaneRect::new(0, 0, width, height).unwrap();
    let info = DecodedFrameInfo::new(
        OutputIndex::new(0),
        BitDepth::Eight,
        pixel_format,
        luma,
        visible,
    )
    .unwrap();
    CurrentFrameWorkspace::new(info, 0).unwrap()
}

fn workspace(pixel_format: PixelFormat) -> CurrentFrameWorkspace<u8> {
    workspace_sized(pixel_format, 16, 16)
}

fn workspace_10bit_420(width: usize, height: usize) -> CurrentFrameWorkspace<u16> {
    let luma = PlaneSize::new(width, height).unwrap();
    let visible = PlaneRect::new(0, 0, width, height).unwrap();
    let info = DecodedFrameInfo::new(
        OutputIndex::new(0),
        BitDepth::Ten,
        PixelFormat::Yuv420,
        luma,
        visible,
    )
    .unwrap();
    CurrentFrameWorkspace::new(info, 0).unwrap()
}

fn seeded_cfl_scratch() -> crate::pipeline::general_intra::GeneralIntraReconScratch<u8> {
    let mut scratch = crate::pipeline::general_intra::GeneralIntraReconScratch::default();
    scratch.cfl_luma_ac.extend([11, 12]);
    scratch.cfl_prediction.extend([21, 22]);
    scratch.mhccp_chroma[0].extend([31, 32]);
    scratch.mhccp_chroma[1].extend([41, 42]);
    scratch
}

fn zero_block() -> LumaCoeffBlock {
    LumaCoeffBlock {
        eob: 0,
        quant_start: 0,
        quant_len: 0,
        zero_tail: 0,
        intra_ist: None,
        cctx_type: None,
        plane_tx_type: 0,
        use_tcq: false,
        lossless: false,
    }
}

fn derived_alpha() -> CflParams {
    CflParams::DerivedAlpha
}

/// Binds `$luma` to the [`CflLuma`] of `$frame` and filter `$filter`.
macro_rules! with_luma {
    ($frame:expr, $filter:expr, |$luma:ident| $body:expr) => {{
        let frame = $frame;
        let plane = frame.plane(PlaneId::Y).unwrap();
        let filter = cfl_filter_index($filter).unwrap();
        if let Some(samples) = ReconSample::u8_slice(plane.samples()) {
            let $luma = CflLuma::<_, RUNTIME_WEIGHTS>::new(frame, samples, filter).unwrap();
            $body
        } else {
            let samples = ReconSample::u16_slice(plane.samples()).unwrap();
            let $luma = CflLuma::<_, RUNTIME_WEIGHTS>::new(frame, samples, filter).unwrap();
            $body
        }
    }};
}

#[allow(clippy::too_many_arguments)]
fn q3<T: ReconSample>(
    frame: &CurrentFrameWorkspace<T>,
    filter: u8,
    chroma_x: usize,
    chroma_y: usize,
    clamp_x: bool,
    clamp_y: bool,
    min_luma_ref_y: Option<isize>,
) -> i32 {
    with_luma!(frame, filter, |luma| {
        let rows = luma.row(chroma_y, clamp_y, min_luma_ref_y).unwrap();
        luma.q3(&rows, chroma_x, clamp_x)
    })
}

#[allow(clippy::too_many_arguments)]
fn cfl_block(
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    sb_mib: usize,
    bit_depth: BitDepth,
) -> CflBlock {
    CflBlock::new(
        x,
        y,
        width.trailing_zeros(),
        height.trailing_zeros(),
        0,
        sb_mib,
        bit_depth,
    )
    .unwrap()
}

/// The luma edges, `lumaAvg` and luma AC of a block.
#[allow(clippy::too_many_arguments)]
fn analyse<T: ReconSample>(
    frame: &CurrentFrameWorkspace<T>,
    block: CflBlock,
    filter: u8,
    neighbours: NeighbourAvailability,
) -> (CflLumaEdges, i32, Vec<i16>) {
    let coeffs = zero_block();
    let planes = [(PlaneId::U, coeffs.view(&[]), neighbours)];
    let mut edges = CflLumaEdges::default();
    let mut ac = Vec::new();
    let average = with_luma!(frame, filter, |luma| baseline_simd!(simd => {
        edges.gather_above(simd, &luma, &planes, block).unwrap();
        let left = (block.x > 0).then_some(&mut edges.left[..block.height]);
        luma.fill_luma(simd, block, left, &mut ac).unwrap();
        edges.average_q3(block, neighbours).unwrap()
    }));
    for luma in &mut ac {
        *luma -= average as i16;
    }
    (edges, average, ac)
}

fn implicit_alpha<T: ReconSample>(
    edges: &CflLumaEdges,
    frame: &CurrentFrameWorkspace<T>,
    block: CflBlock,
) -> i32 {
    let chroma = PlaneSamples::of(frame, PlaneId::U).unwrap();
    edges
        .plane_chroma_model(&chroma, block, interior(), true)
        .unwrap()
        .1
}

fn explicit_prediction(
    dc: i32,
    alpha: i8,
    ac: &[i16],
    width: usize,
    bit_depth: BitDepth,
) -> Vec<u16> {
    let mut prediction = vec![0u16; ac.len()];
    let max = i32::from(bit_depth.max_sample());
    let model = [dc, i32::from(alpha) * CFL_ALPHA_SCALE, 0, max];
    cfl_predict_block(model, ac, width, &mut prediction, width).unwrap();
    prediction
}

#[test]
fn invalid_cfl_filter_preserves_prediction_scratch() {
    let mut frame = workspace(PixelFormat::Yuv420);
    let mut scratch = seeded_cfl_scratch();
    let before = (
        scratch.cfl_luma_ac.clone(),
        scratch.cfl_prediction.clone(),
        scratch.mhccp_chroma.clone(),
    );

    let result = reconstruct_general_intra_chroma_cfl_block_into(
        &mut scratch,
        &mut frame,
        zero_block().view(&[]),
        PlaneId::U,
        0,
        0,
        2,
        2,
        128,
        derived_alpha(),
        4,
        4,
        interior(),
        BitDepth::Eight,
    );

    assert!(matches!(
        result,
        Err(GeneralIntraResidualError::InvalidReconstructionState {
            context: "CfL filter"
        })
    ));
    assert_eq!(
        (
            scratch.cfl_luma_ac,
            scratch.cfl_prediction,
            scratch.mhccp_chroma
        ),
        before
    );
}

#[test]
fn invalid_mhccp_filter_preserves_prediction_scratch() {
    let mut frame = workspace(PixelFormat::Yuv420);
    let mut scratch = seeded_cfl_scratch();
    let before = (
        scratch.cfl_luma_ac.clone(),
        scratch.cfl_prediction.clone(),
        scratch.mhccp_chroma.clone(),
    );

    let result = reconstruct_general_intra_chroma_cfl_block_into(
        &mut scratch,
        &mut frame,
        zero_block().view(&[]),
        PlaneId::U,
        0,
        0,
        2,
        2,
        128,
        CflParams::Multi {
            direction: CflMultiDirection::Direct,
        },
        4,
        4,
        interior(),
        BitDepth::Eight,
    );

    assert!(matches!(
        result,
        Err(GeneralIntraResidualError::InvalidReconstructionState {
            context: "MHCCP filter"
        })
    ));
    assert_eq!(
        (
            scratch.cfl_luma_ac,
            scratch.cfl_prediction,
            scratch.mhccp_chroma
        ),
        before
    );
}

#[test]
fn cfl_420_simd_matches_scalar_at_block_and_frame_edges() {
    let mut frame = workspace_10bit_420(130, 10);
    let size = frame.plane(PlaneId::Y).unwrap().storage_size();
    let (plane_width, plane_height) = (size.width(), size.height());
    let luma: Vec<u16> = (0..plane_width * plane_height)
        .map(|index| ((index * 17 + 11) & 1023) as u16)
        .collect();
    for y in 0..plane_height {
        for x in 0..plane_width {
            frame
                .set_reconstructed_sample(PlaneId::Y, x, y, luma[y * plane_width + x])
                .unwrap();
        }
    }
    let average_q3 = 37;
    for (filter, filter_weights) in CFL_FILTERS_420.iter().enumerate() {
        for (x, y, width, height) in [(0, 0, 32, 4), (1, 1, 64, 4), (31, 2, 32, 4), (50, 4, 16, 4)]
        {
            let block = cfl_block(x, y, width, height, 16, BitDepth::Ten);
            let mut actual = Vec::new();
            with_luma!(&frame, filter as u8, |luma| baseline_simd!(simd => {
                luma.fill_luma(simd, block, None, &mut actual)
            }))
            .unwrap();
            for luma in &mut actual {
                *luma -= average_q3 as i16;
            }
            let mut expected = Vec::with_capacity(width * height);
            for row in 0..height {
                let luma_y = 2 * (y + row).min((plane_height - 1) >> 1);
                let clamp_y = row == 0 || luma_y.is_multiple_of(64);
                let rows = [
                    if clamp_y { luma_y } else { luma_y - 1 },
                    luma_y,
                    (luma_y + 1).min(plane_height - 1),
                ];
                for col in 0..width {
                    let luma_x = 2 * (x + col).min((plane_width - 1) >> 1);
                    let center = luma_x.min(plane_width - 1);
                    let left = if col == 0 || luma_x.is_multiple_of(64) {
                        center
                    } else {
                        luma_x.saturating_sub(1).min(plane_width - 1)
                    };
                    let right = (luma_x + 1).min(plane_width - 1);
                    let mut total = -average_q3;
                    for (tap_row, weights) in rows.iter().zip(filter_weights) {
                        for (tap_col, weight) in [left, center, right].iter().zip(weights) {
                            total += weight * i32::from(luma[tap_row * plane_width + tap_col]);
                        }
                    }
                    expected.push(total as i16);
                }
            }
            assert_eq!(
                actual, expected,
                "filter={filter} x={x} y={y} width={width} height={height}"
            );
        }
    }
}
#[test]
fn cfl_luma_sample_repeats_last_downsampled_row_at_frame_edge() {
    let mut frame = workspace_sized(PixelFormat::Yuv420, 8, 6);
    frame
        .fill_rect(PlaneId::Y, PlaneRect::new(0, 0, 8, 6).unwrap(), 100)
        .unwrap();
    frame
        .fill_rect(PlaneId::Y, PlaneRect::new(0, 5, 8, 1).unwrap(), 200)
        .unwrap();

    let last = q3(&frame, 1, 1, 2, false, false, None);
    assert_eq!(last, 1_200);
    assert_eq!(q3(&frame, 1, 1, 3, false, false, None), last);
}
#[test]
fn cfl_above_average_uses_block_left_not_internal_64_pixel_boundaries() {
    let mut frame = workspace_10bit_420(256, 256);
    frame
        .fill_rect(PlaneId::Y, PlaneRect::new(0, 0, 256, 256).unwrap(), 100)
        .unwrap();
    frame
        .set_reconstructed_sample(PlaneId::Y, 63, 64, 200)
        .unwrap();
    frame
        .set_reconstructed_sample(PlaneId::Y, 127, 64, 133)
        .unwrap();
    frame
        .set_reconstructed_sample(PlaneId::Y, 65, 68, 132)
        .unwrap();

    let min_luma_ref_y = cfl_above_min_luma_ref_y(33, 32, PixelFormat::Yuv420);
    assert_eq!(q3(&frame, 1, 32, 32, true, false, min_luma_ref_y), 800);
    assert_eq!(q3(&frame, 1, 64, 32, false, false, min_luma_ref_y), 833);

    let block = cfl_block(32, 33, 64, 32, 32, BitDepth::Ten);
    let (_, average, luma_ac) = analyse(&frame, block, 1, interior());
    assert_eq!(average, 801);
    let none = NeighbourAvailability::new(false, false, 0, 0);
    let (_, tile_start, _) = analyse(&frame, block, 1, none);
    assert_eq!(tile_start, i32::from(8u16 << (BitDepth::Ten.bits() - 1)));
    let left = NeighbourAvailability::new(false, true, 0, 0);
    let (_, left_only, _) = analyse(&frame, block, 1, left);
    assert_ne!(left_only, average);

    assert_eq!(luma_ac[65], 31);
    assert_eq!(
        explicit_prediction(508, -1, &luma_ac, 64, BitDepth::Ten)[65],
        508
    );
}

#[test]
fn cfl_left_average_uses_block_top_not_internal_64_pixel_boundaries() {
    let mut frame = workspace_10bit_420(256, 256);
    frame
        .fill_rect(PlaneId::Y, PlaneRect::new(0, 0, 256, 256).unwrap(), 100)
        .unwrap();
    frame
        .fill_rect(PlaneId::U, PlaneRect::new(0, 0, 128, 128).unwrap(), 100)
        .unwrap();
    frame
        .set_reconstructed_sample(PlaneId::Y, 64, 55, 200)
        .unwrap();
    frame
        .set_reconstructed_sample(PlaneId::Y, 64, 63, 900)
        .unwrap();
    frame
        .set_reconstructed_sample(PlaneId::U, 32, 32, 200)
        .unwrap();

    assert_eq!(q3(&frame, 2, 32, 28, false, true, None), 800);
    assert_eq!(q3(&frame, 2, 32, 32, false, false, None), 1_600);

    let block = cfl_block(33, 28, 16, 64, 32, BitDepth::Ten);
    let (edges, average, luma_ac) = analyse(&frame, block, 2, interior());
    assert_eq!(average, 816);
    assert_eq!(luma_ac[4 * 16], -16);
    assert_eq!(implicit_alpha(&edges, &frame, block), 255);
    assert_eq!(
        explicit_prediction(508, -16, &luma_ac, 16, BitDepth::Ten)[4 * 16],
        512
    );
}

#[test]
fn cfl_derived_alpha_above_uses_transform_local_boundary() {
    let mut frame = workspace_10bit_420(256, 256);
    frame
        .fill_rect(PlaneId::Y, PlaneRect::new(0, 0, 256, 256).unwrap(), 100)
        .unwrap();
    frame
        .fill_rect(PlaneId::U, PlaneRect::new(0, 0, 128, 128).unwrap(), 100)
        .unwrap();
    frame
        .set_reconstructed_sample(PlaneId::Y, 55, 64, 200)
        .unwrap();
    frame
        .set_reconstructed_sample(PlaneId::Y, 63, 64, 200)
        .unwrap();
    frame
        .set_reconstructed_sample(PlaneId::U, 32, 32, 112)
        .unwrap();

    let min_luma_ref_y = cfl_above_min_luma_ref_y(33, 32, PixelFormat::Yuv420);
    assert_eq!(q3(&frame, 1, 28, 32, true, false, min_luma_ref_y), 800);
    assert_eq!(q3(&frame, 1, 32, 32, false, false, min_luma_ref_y), 900);
    let block = cfl_block(28, 33, 64, 16, 32, BitDepth::Ten);
    let (edges, _, _) = analyse(&frame, block, 1, interior());
    assert_eq!(implicit_alpha(&edges, &frame, block), 255);
}

#[test]
fn cfl_reconstruction_supports_non_420_chroma_geometry() {
    let mut scratch = crate::pipeline::general_intra::GeneralIntraReconScratch::default();
    let mut retained = None;
    for pixel_format in [PixelFormat::Yuv422, PixelFormat::Yuv444] {
        let mut frame = workspace(pixel_format);

        reconstruct_general_intra_chroma_cfl_block_into(
            &mut scratch,
            &mut frame,
            zero_block().view(&[]),
            PlaneId::U,
            0,
            0,
            2,
            2,
            0,
            derived_alpha(),
            0,
            16,
            NeighbourAvailability::new(false, false, 0, 0),
            BitDepth::Eight,
        )
        .unwrap();

        assert_eq!(frame.reconstructed_sample(PlaneId::U, 0, 0).unwrap(), 128);
        let storage = (
            scratch.cfl_luma_ac.as_ptr(),
            scratch.cfl_prediction.as_ptr(),
        );
        if let Some(previous) = retained {
            assert_eq!(storage, previous);
        }
        retained = Some(storage);
    }
}

#[test]
fn cfl_luma_sample_uses_422_and_444_filters() {
    let luma = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
    for (pixel_format, expected) in [(PixelFormat::Yuv422, 52), (PixelFormat::Yuv444, 40)] {
        let mut frame = workspace(pixel_format);
        frame
            .write_rect_block(
                PlaneId::Y,
                0,
                0,
                IntraRectBlockSize::new(2, 2).unwrap(),
                &luma,
            )
            .unwrap();

        assert_eq!(q3(&frame, 0, 1, 1, false, false, None), expected);
    }
}

#[allow(clippy::too_many_arguments)]
fn mhccp_geometry<T: ReconSample>(
    frame: &CurrentFrameWorkspace<T>,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    sb_mib: usize,
    neighbours: NeighbourAvailability,
) -> core::result::Result<(usize, usize, usize, usize), GeneralIntraResidualError> {
    let block = cfl_block(x, y, width, height, sb_mib, BitDepth::Eight);
    MhccpGeometry::new(frame, PlaneId::U, block, neighbours).map(|refs| {
        let ref_width = refs.reference_width.max(refs.left + width);
        let ref_height = refs.reference_height.max(refs.above + height);
        (ref_width, ref_height, refs.above, refs.left)
    })
}

#[test]
fn mhccp_reference_extensions_use_luma_pixel_threshold_for_420() {
    let frame = workspace_sized(PixelFormat::Yuv420, 128, 128);
    let bottom = NeighbourAvailability::new(false, true, 0, 1);
    let right = NeighbourAvailability::new(true, false, 1, 0);
    assert_eq!(
        mhccp_geometry(&frame, 32, 0, 16, 4, 16, bottom).unwrap(),
        (18, 8, 0, 2)
    );
    assert_eq!(
        mhccp_geometry(&frame, 0, 4, 4, 16, 16, right).unwrap(),
        (8, 18, 2, 0)
    );
    let yuv444 = workspace_sized(PixelFormat::Yuv444, 128, 128);
    assert_eq!(
        mhccp_geometry(&yuv444, 8, 0, 8, 4, 16, bottom).unwrap().1,
        4
    );
}

#[test]
fn mhccp_reference_extensions_use_non_420_subsampling() {
    for pixel_format in [PixelFormat::Yuv422, PixelFormat::Yuv444] {
        let frame = workspace_sized(pixel_format, 128, 128);
        let bottom = NeighbourAvailability::new(false, true, 0, 1);
        assert_eq!(
            mhccp_geometry(&frame, 32, 0, 16, 8, 16, bottom).unwrap(),
            (18, 12, 0, 2)
        );
    }
}

#[test]
fn mhccp_bottom_edge_keeps_full_prediction_with_clipped_reference_extent() {
    let mut frame = workspace_sized(PixelFormat::Yuv420, 1920, 1080);
    let mut scratch = crate::pipeline::general_intra::GeneralIntraReconScratch::default();
    frame
        .fill_rect(PlaneId::U, PlaneRect::new(128, 528, 32, 12).unwrap(), 77)
        .unwrap();
    reconstruct_general_intra_chroma_cfl_block_into(
        &mut scratch,
        &mut frame,
        zero_block().view(&[]),
        PlaneId::U,
        128,
        528,
        5,
        4,
        0,
        CflParams::Multi {
            direction: CflMultiDirection::Direct,
        },
        0,
        32,
        interior(),
        BitDepth::Eight,
    )
    .unwrap();
    for y in 528..540 {
        for x in 128..160 {
            assert_eq!(frame.reconstructed_sample(PlaneId::U, x, y).unwrap(), 0);
        }
    }
}

#[test]
fn mhccp_rejects_reference_origin_outside_frame() {
    let frame = workspace_sized(PixelFormat::Yuv420, 1920, 1080);
    let left = NeighbourAvailability::new(false, true, 0, 0);
    assert!(matches!(
        mhccp_geometry(&frame, 960, 0, 32, 16, 32, left),
        Err(GeneralIntraResidualError::InvalidReconstructionState {
            context: "MHCCP reference geometry"
        })
    ));
}

fn lcg(state: &mut u64) -> usize {
    *state = state
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    (*state >> 33) as usize
}

fn random_frame_10bit(state: &mut u64, width: usize, height: usize) -> CurrentFrameWorkspace<u16> {
    let mut frame = workspace_10bit_420(width, height);
    for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
        let size = frame.plane(plane).unwrap().storage_size();
        for y in 0..size.height() {
            for x in 0..size.width() {
                let sample = (lcg(state) % 1024) as u16;
                frame.set_reconstructed_sample(plane, x, y, sample).unwrap();
            }
        }
    }
    frame
}

#[test]
fn cfl_pair_matches_single_plane_reconstruction() {
    let source = || random_frame_10bit(&mut 5, 96, 80);
    let neighbours = NeighbourAvailability::new(true, true, 2, 1);
    let mut scratch = crate::pipeline::general_intra::GeneralIntraReconScratch::default();
    let block = zero_block();
    for params in [
        CflParams::Explicit {
            alpha_u: 5,
            alpha_v: -3,
        },
        CflParams::DerivedAlpha,
        CflParams::Multi {
            direction: CflMultiDirection::Direct,
        },
        CflParams::Multi {
            direction: CflMultiDirection::Above,
        },
        CflParams::Multi {
            direction: CflMultiDirection::Left,
        },
    ] {
        for (x, y, log2_width, log2_height) in [(8, 8, 3, 3), (4, 12, 2, 3), (16, 4, 4, 2)] {
            let mut pair = source();
            reconstruct_general_intra_chroma_cfl_pair_into(
                &mut scratch,
                &mut pair,
                block.view(&[]),
                block.view(&[]),
                x,
                y,
                log2_width,
                log2_height,
                0,
                params,
                1,
                16,
                neighbours,
                neighbours,
                BitDepth::Ten,
            )
            .unwrap();
            let mut single = source();
            for plane in [PlaneId::U, PlaneId::V] {
                reconstruct_general_intra_chroma_cfl_block_into(
                    &mut scratch,
                    &mut single,
                    block.view(&[]),
                    plane,
                    x,
                    y,
                    log2_width,
                    log2_height,
                    0,
                    params,
                    1,
                    16,
                    neighbours,
                    BitDepth::Ten,
                )
                .unwrap();
            }
            for plane in [PlaneId::U, PlaneId::V] {
                assert_eq!(
                    pair.plane(plane).unwrap().samples(),
                    single.plane(plane).unwrap().samples(),
                    "{params:?} {plane:?} x={x} y={y}"
                );
            }
            assert_ne!(
                pair.plane(PlaneId::U).unwrap().samples(),
                source().plane(PlaneId::U).unwrap().samples()
            );
        }
    }
}

#[test]
fn cfl_predict_lanes_match_scalar_over_random_inputs() {
    let mut state = 7;
    for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
        let max = i32::from(bit_depth.max_sample());
        for _ in 0..400 {
            let width = 4 << (lcg(&mut state) % 5);
            let len = width * 2 * (1 + lcg(&mut state) % 4);
            let dc = (lcg(&mut state) % (max as usize + 1)) as i32;
            let alpha_q3 = (lcg(&mut state) % 65_536) as i32 - 32_768;
            let luma_ac: Vec<i16> = (0..len)
                .map(|_| ((lcg(&mut state) % (16 * max as usize + 1)) as i32 - 8 * max) as i16)
                .collect();
            let mut actual = vec![0u16; len];
            let model = [dc, alpha_q3, 0, max];
            cfl_predict_block(model, &luma_ac, width, &mut actual, width).unwrap();
            let expected: Vec<u16> = luma_ac
                .iter()
                .map(|&ac| cfl_predict(model, i32::from(ac)) as u16)
                .collect();
            assert_eq!(actual, expected, "dc={dc} alpha={alpha_q3}");
        }
    }
}

fn check_cfl_predict_block<T: ReconSample>(max: i32, state: &mut u64) {
    let width = 4 << (lcg(state) % 5);
    let height = 4 << (lcg(state) % 5);
    let stride = width + lcg(state) % 9;
    let sample = |state: &mut u64| (lcg(state) % (8 * max as usize + 1)) as i32;
    let model = [
        (lcg(state) % (max as usize + 1)) as i32,
        (lcg(state) % 4095) as i32 - 2047,
        sample(state),
        max,
    ];
    let luma: Vec<i16> = (0..width * height).map(|_| sample(state) as i16).collect();
    let sentinel = T::try_from_u16(7).unwrap();
    let mut out = vec![sentinel; (height - 1) * stride + width];
    cfl_predict_block(model, &luma, width, &mut out, stride).unwrap();
    for (i, &got) in out.iter().enumerate() {
        let (row, col) = (i / stride, i % stride);
        let expected = if col < width {
            cfl_predict(model, i32::from(luma[row * width + col])) as u16
        } else {
            7
        };
        assert_eq!(
            got.to_u16(),
            expected,
            "{width}x{height} model={model:?} at {i}"
        );
    }
}

#[test]
fn cfl_predict_blocks_match_scalar_for_pmulhrsw_alphas() {
    let mut state = 11;
    for _ in 0..300 {
        check_cfl_predict_block::<u8>(255, &mut state);
        check_cfl_predict_block::<u16>(1023, &mut state);
    }
}

/// [`cfl_predict_block`] at the extreme `alpha`, `lumaAvg`, `dc` and `L`.
fn check_cfl_predict_extremes<T: ReconSample>(max: i32) {
    for alpha in [-2047, -1024, -1, 0, 1, 1024, 2047] {
        for average in [0, 4 * max, 8 * max] {
            for dc in [0, max / 2, max] {
                let model = [dc, alpha, average, max];
                let near = [
                    0,
                    8 * max,
                    average,
                    (average + 1).min(8 * max),
                    (average - 1).max(0),
                ];
                let luma: Vec<i16> = (0..64)
                    .map(|i| near.get(i % 6).map_or(4 * max + i as i32, |&l| l) as i16)
                    .collect();
                for width in [4, 8, 16, 64] {
                    let mut out = vec![T::default(); 64];
                    cfl_predict_block(model, &luma, width, &mut out, width).unwrap();
                    for (got, &l) in out.iter().zip(&luma) {
                        let expected = cfl_predict(model, i32::from(l)) as u16;
                        assert_eq!(got.to_u16(), expected, "{model:?} L={l} width={width}");
                    }
                }
            }
        }
    }
}

#[test]
fn cfl_predict_blocks_match_scalar_at_extremes() {
    check_cfl_predict_extremes::<u8>(255);
    check_cfl_predict_extremes::<u16>(1023);
}

/// [`CflLuma::q3_run`] over chroma row `y - 1` with the 4:2:0 filter `F`.
#[allow(clippy::too_many_arguments)]
fn specialised_run<const F: usize>(
    frame: &CurrentFrameWorkspace<u16>,
    y: usize,
    clamp_y: bool,
    min_luma_ref_y: Option<isize>,
    x0: usize,
    clamp_first: bool,
    len: usize,
) -> Vec<i16> {
    let samples = frame.plane(PlaneId::Y).unwrap().samples();
    let luma = CflLuma::<_, F>::new(frame, samples, F).unwrap();
    let rows = luma.row(y - 1, clamp_y, min_luma_ref_y).unwrap();
    let mut run = vec![0; len];
    baseline_simd!(simd => luma.q3_run(simd, &rows, x0, clamp_first, false, &mut run));
    run
}

#[test]
fn cfl_luma_runs_match_per_sample_q3_over_random_planes() {
    let mut state = 11;
    let mut lane_runs = 0;
    for _ in 0..40 {
        let width = 8 + 2 * (lcg(&mut state) % 70);
        let height = 8 + 2 * (lcg(&mut state) % 70);
        let frame = random_frame_10bit(&mut state, width, height);
        for filter in 0..3u8 {
            with_luma!(&frame, filter, |luma| {
                for _ in 0..20 {
                    let y = 1 + lcg(&mut state) % (height / 2 - 1);
                    let min_luma_ref_y = lcg(&mut state)
                        .is_multiple_of(2)
                        .then_some(2 * y as isize - 1);
                    let clamp_y = lcg(&mut state).is_multiple_of(2);
                    let rows = luma.row(y - 1, clamp_y, min_luma_ref_y).unwrap();
                    let x0 = lcg(&mut state) % (width / 2);
                    let clamp_first = lcg(&mut state).is_multiple_of(2);
                    let mut run = vec![0; (width / 2 - x0).min(70)];
                    baseline_simd!(simd => luma.q3_run(simd, &rows, x0, clamp_first, false, &mut run));
                    for (i, &q3) in run.iter().enumerate() {
                        let expected = luma.q3(&rows, x0 + i, clamp_first && i == 0);
                        assert_eq!(i32::from(q3), expected, "x0={x0} i={i}");
                    }
                    lane_runs += usize::from(x0 > 0 && run.len() > 8);
                    let args = (y, clamp_y, min_luma_ref_y, x0, clamp_first, run.len());
                    let specialised = match filter {
                        0 => specialised_run::<0>,
                        1 => specialised_run::<1>,
                        _ => specialised_run::<2>,
                    };
                    let (y, clamp_y, min, x0, clamp_first, len) = args;
                    assert_eq!(
                        specialised(&frame, y, clamp_y, min, x0, clamp_first, len),
                        run
                    );
                }
            });
        }
    }
    assert!(lane_runs > 500, "{lane_runs}");
}

fn workspace_of<T: ReconSample>(
    pixel_format: PixelFormat,
    bit_depth: BitDepth,
    width: usize,
    height: usize,
) -> CurrentFrameWorkspace<T> {
    let luma = PlaneSize::new(width, height).unwrap();
    let visible = PlaneRect::new(0, 0, width, height).unwrap();
    let info = DecodedFrameInfo::new(OutputIndex::new(0), bit_depth, pixel_format, luma, visible);
    CurrentFrameWorkspace::new(info.unwrap(), T::default()).unwrap()
}

/// Every format and depth, with random, extreme and asymmetric samples.
fn random_frames(state: &mut u64, mut check: impl FnMut(&mut u64, &dyn AnyFrame)) {
    for round in 0..6 {
        for pixel_format in [
            PixelFormat::Yuv420,
            PixelFormat::Yuv422,
            PixelFormat::Yuv444,
        ] {
            let (width, height) = (20 + 2 * (lcg(state) % 60), 12 + 2 * (lcg(state) % 40));
            let mut frame = workspace_of::<u8>(pixel_format, BitDepth::Eight, width, height);
            randomise(&mut frame, state, if round == 0 { 1 } else { 255 });
            if round == 1 {
                extreme(&mut frame, 255);
            }
            check(state, &frame);
            let mut frame = workspace_of::<u16>(pixel_format, BitDepth::Ten, width, height);
            randomise(&mut frame, state, 1023);
            if round == 1 {
                extreme(&mut frame, 1023);
            }
            check(state, &frame);
        }
    }
}

/// Alternating zero and maximum samples in a checkerboard of 3x2 cells.
fn extreme<T: ReconSample>(frame: &mut CurrentFrameWorkspace<T>, max: u16) {
    for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
        let size = frame.plane(plane).unwrap().storage_size();
        for y in 0..size.height() {
            for x in 0..size.width() {
                let sample = T::try_from_u16(if (x / 3 + y / 2) % 2 == 0 { max } else { 0 });
                frame
                    .set_reconstructed_sample(plane, x, y, sample.unwrap())
                    .unwrap();
            }
        }
    }
}

/// A frame of either sample type, for the format-generic checks.
trait AnyFrame {
    fn with_u8(&self) -> Option<&CurrentFrameWorkspace<u8>>;
    fn with_u16(&self) -> Option<&CurrentFrameWorkspace<u16>>;
}

impl AnyFrame for CurrentFrameWorkspace<u8> {
    fn with_u8(&self) -> Option<&CurrentFrameWorkspace<u8>> {
        Some(self)
    }
    fn with_u16(&self) -> Option<&CurrentFrameWorkspace<u16>> {
        None
    }
}

impl AnyFrame for CurrentFrameWorkspace<u16> {
    fn with_u8(&self) -> Option<&CurrentFrameWorkspace<u8>> {
        None
    }
    fn with_u16(&self) -> Option<&CurrentFrameWorkspace<u16>> {
        Some(self)
    }
}

/// Runs `$body` with `$frame` bound to the frame of either sample type.
macro_rules! any_frame {
    ($any:expr, |$frame:ident| $body:expr) => {
        if let Some($frame) = $any.with_u8() {
            $body
        } else if let Some($frame) = $any.with_u16() {
            $body
        }
    };
}

#[test]
fn cfl_luma_runs_match_per_sample_q3_in_every_format() {
    let mut state = 0x5eed_0fc3;
    let mut past_edge = 0;
    random_frames(&mut state, |state, any| {
        any_frame!(any, |frame| for filter in 0..3u8 {
            with_luma!(frame, filter, |luma| for _ in 0..30 {
                let chroma = frame.plane(PlaneId::U).unwrap().storage_size();
                let edge = (luma.plane.width - 1) >> luma.sub_x;
                let rows = luma.row(
                    lcg(state) % chroma.height(),
                    lcg(state).is_multiple_of(2),
                    None,
                );
                let rows = rows.unwrap();
                let x0 = lcg(state) % (chroma.width() + 2);
                let (clamp_first, periodic) =
                    (lcg(state).is_multiple_of(2), lcg(state).is_multiple_of(2));
                let mut run = vec![0; 1 + lcg(state) % 70];
                baseline_simd!(simd => luma.q3_run(simd, &rows, x0, clamp_first, periodic, &mut run));
                for (i, &q3) in run.iter().enumerate() {
                    let column = (x0 + i).min(edge) << luma.sub_x;
                    let clamp = (clamp_first && i == 0) || (periodic && column % 64 == 0);
                    let expected = luma.q3(&rows, x0 + i, clamp);
                    assert_eq!(
                        i32::from(q3),
                        expected,
                        "{filter} x0={x0} i={i} {clamp_first} {periodic}"
                    );
                }
                past_edge += usize::from(x0 + run.len() > edge + 1);
            });
        });
    });
    assert!(past_edge > 500, "{past_edge}");
}

/// [`CflLuma::fill_luma`] of random blocks, some past the right or bottom
/// edge, against per-sample [`CflLuma::q3`]: `L` and `lumaLeft`.
#[test]
fn cfl_block_luma_matches_per_sample_q3_in_every_format() {
    let mut state = 0x00b1_0c4a;
    random_frames(&mut state, |state, any| {
        any_frame!(any, |frame| for filter in 0..3u8 {
            with_luma!(frame, filter, |luma| for _ in 0..12 {
                let chroma = frame.plane(PlaneId::U).unwrap().storage_size();
                let (log2_w, log2_h) = (2 + lcg(state) % 5, 2 + lcg(state) % 5);
                let (w, h) = (1usize << log2_w, 1usize << log2_h);
                let x = 4 * (lcg(state) % chroma.width().div_ceil(4));
                let y = 4 * (lcg(state) % chroma.height().div_ceil(4));
                let block = cfl_block(x, y, w, h, 16, frame.info().bit_depth());
                let mut left = [0; MAX_CFL_AXIS];
                let mut actual = Vec::new();
                baseline_simd!(simd => luma.fill_luma(simd, block, (x > 0).then_some(&mut left[..h]), &mut actual))
                    .unwrap();
                let (max_x, max_y) = (luma.plane.width - 1, luma.plane.height - 1);
                for row in 0..h {
                    let luma_y = clamped_cfl_luma_coordinate(y + row, luma.sub_y, max_y);
                    let rows = luma
                        .row(y + row, row == 0 || luma_y.is_multiple_of(64), None)
                        .unwrap();
                    for col in 0..w {
                        let luma_x = clamped_cfl_luma_coordinate(x + col, luma.sub_x, max_x);
                        let expected =
                            luma.q3(&rows, x + col, col == 0 || luma_x.is_multiple_of(64));
                        let got = i32::from(actual[row * w + col]);
                        assert_eq!(
                            got, expected,
                            "{filter} ({x},{y}) {w}x{h} row {row} col {col}"
                        );
                    }
                    if x > 0 {
                        let rows = luma.row(y + row, row == 0, None).unwrap();
                        assert_eq!(
                            i32::from(left[row]),
                            luma.q3(&rows, x - 1, false),
                            "left {row}"
                        );
                    }
                }
            });
        });
    });
}

/// [`CflLuma::fill_luma`] of random blocks inside `frame` against per-sample
/// [`CflLuma::q3`] with the 4:2:0 filter `F`.
fn check_u8_block_luma<const F: usize>(frame: &CurrentFrameWorkspace<u8>, state: &mut u64) {
    let samples = frame.plane(PlaneId::Y).unwrap().samples();
    let luma = CflLuma::<u8, F>::new(frame, samples, F).unwrap();
    let size = frame.plane(PlaneId::U).unwrap().storage_size();
    let (width, height) = (size.width(), size.height());
    for _ in 0..40 {
        let log2 = |state: &mut u64, limit: usize| {
            1usize << (2 + lcg(state) % 5).min(limit.ilog2() as usize)
        };
        let block_width = log2(state, width);
        let block_height = log2(state, height);
        let x = lcg(state) % (width - block_width + 1);
        let y = lcg(state) % (height - block_height + 1);
        let block = cfl_block(x, y, block_width, block_height, 16, BitDepth::Eight);
        let mut left = [0; MAX_CFL_AXIS];
        let mut actual = Vec::new();
        baseline_simd!(simd => luma.fill_luma(
            simd,
            block,
            (x > 0).then_some(&mut left[..block_height]),
            &mut actual,
        ))
        .unwrap();
        for row in 0..block_height {
            let luma_y = 2 * (y + row);
            let rows = luma
                .row(y + row, row == 0 || luma_y.is_multiple_of(64), None)
                .unwrap();
            for col in 0..block_width {
                let clamp_x = col == 0 || (2 * (x + col)).is_multiple_of(64);
                assert_eq!(
                    i32::from(actual[row * block_width + col]),
                    luma.q3(&rows, x + col, clamp_x),
                    "F={F} x={x} y={y} {block_width}x{block_height} row={row} col={col}"
                );
            }
            if x > 0 {
                let rows = luma.row(y + row, row == 0, None).unwrap();
                assert_eq!(
                    i32::from(left[row]),
                    luma.q3(&rows, x - 1, false),
                    "F={F} left row {row}"
                );
            }
        }
    }
}

/// [`CflLumaEdges::gather`] of random blocks inside `frame`, whose one-pass
/// form starts with the above row, against per-sample [`CflLuma::q3`].
fn check_u8_gathered_edges<const F: usize>(frame: &CurrentFrameWorkspace<u8>, state: &mut u64) {
    let samples = frame.plane(PlaneId::Y).unwrap().samples();
    let luma = CflLuma::<u8, F>::new(frame, samples, F).unwrap();
    let size = frame.plane(PlaneId::U).unwrap().storage_size();
    let (width, height) = (size.width(), size.height());
    let coeffs = zero_block();
    let planes = [(PlaneId::U, coeffs.view(&[]), interior())];
    let mut folded = 0;
    for _ in 0..40 {
        let side = |state: &mut u64, limit: usize| {
            4usize << (lcg(state) % 4).min(limit.ilog2() as usize - 2)
        };
        let (block_width, block_height) = (side(state, width), side(state, height));
        let x = 4 * (lcg(state) % ((width - block_width) / 4 + 1));
        let y = 4 * (lcg(state) % ((height - block_height) / 4 + 1));
        let sb_mib = [2, 8, 16][lcg(state) % 3];
        let block = cfl_block(x, y, block_width, block_height, sb_mib, BitDepth::Eight);
        let mut edges = CflLumaEdges::default();
        let mut ac = Vec::new();
        let start =
            baseline_simd!(simd => edges.gather(simd, &luma, &planes, block, &mut ac)).unwrap();
        folded += usize::from(start > 0);
        if y > 0 {
            let min_y = cfl_above_min_luma_ref_y(y, sb_mib, PixelFormat::Yuv420);
            let rows = luma.row(y - 1, false, min_y).unwrap();
            for col in 0..block_width {
                assert_eq!(
                    i32::from(edges.above[col]),
                    luma.q3(&rows, x + col, col == 0),
                    "F={F} above x={x} y={y} {block_width}x{block_height} sb={sb_mib} col={col}"
                );
            }
        }
        for row in 0..block_height {
            let luma_y = 2 * (y + row);
            let rows = luma
                .row(y + row, row == 0 || luma_y.is_multiple_of(64), None)
                .unwrap();
            for col in 0..block_width {
                let clamp_x = col == 0 || (2 * (x + col)).is_multiple_of(64);
                assert_eq!(
                    i32::from(ac[start + row * block_width + col]),
                    luma.q3(&rows, x + col, clamp_x),
                    "F={F} x={x} y={y} {block_width}x{block_height} row={row} col={col}"
                );
            }
            if x > 0 {
                let rows = luma.row(y + row, row == 0, None).unwrap();
                assert_eq!(
                    i32::from(edges.left[row]),
                    luma.q3(&rows, x - 1, false),
                    "F={F} left row {row}"
                );
            }
        }
    }
    assert!(
        folded > 5 || cfg!(not(any(target_arch = "x86_64", target_arch = "aarch64"))),
        "{folded}"
    );
}

#[test]
fn cfl_420_u8_gathered_edges_match_per_sample_q3() {
    let mut state = 31;
    for round in 0..30 {
        let width = 32 + 4 * (lcg(&mut state) % 60);
        let height = 32 + 4 * (lcg(&mut state) % 40);
        let mut frame = workspace_sized(PixelFormat::Yuv420, width, height);
        for y in 0..height {
            for x in 0..width {
                let sample = (lcg(&mut state) % 256) as u8 | if round % 5 == 0 { 255 } else { 0 };
                frame
                    .set_reconstructed_sample(PlaneId::Y, x, y, sample)
                    .unwrap();
            }
        }
        check_u8_gathered_edges::<0>(&frame, &mut state);
        check_u8_gathered_edges::<1>(&frame, &mut state);
        check_u8_gathered_edges::<2>(&frame, &mut state);
    }
}

#[test]
fn cfl_420_u8_block_luma_matches_per_sample_q3() {
    let mut state = 23;
    for round in 0..30 {
        let width = 16 + 4 * (lcg(&mut state) % 60);
        let height = 16 + 4 * (lcg(&mut state) % 40);
        let mut frame = workspace_sized(PixelFormat::Yuv420, width, height);
        for y in 0..height {
            for x in 0..width {
                let sample = (lcg(&mut state) % 256) as u8 | if round % 5 == 0 { 255 } else { 0 };
                frame
                    .set_reconstructed_sample(PlaneId::Y, x, y, sample)
                    .unwrap();
            }
        }
        check_u8_block_luma::<0>(&frame, &mut state);
        check_u8_block_luma::<1>(&frame, &mut state);
        check_u8_block_luma::<2>(&frame, &mut state);
    }
}

/// [`CflLuma::block_luma_420`] with only first-row and first-column clamps
/// (the § 7.13.6 MHCCP reference rows) against [`CflLuma::q3_run`].
fn check_u8_reference_luma<const F: usize>(frame: &CurrentFrameWorkspace<u8>, state: &mut u64) {
    let samples = frame.plane(PlaneId::Y).unwrap().samples();
    let luma = CflLuma::<u8, F>::new(frame, samples, F).unwrap();
    let size = frame.plane(PlaneId::U).unwrap().storage_size();
    let (width, height) = (size.width(), size.height());
    let mut fast_runs = 0;
    for _ in 0..60 {
        let block_width = 1 + lcg(state) % width.min(40);
        let block_height = 1 + lcg(state) % height.min(20);
        let x = lcg(state) % (width - block_width + 1);
        let y = lcg(state) % (height - block_height + 1);
        let (clamp_row0, clamp_col0) = (lcg(state).is_multiple_of(2), lcg(state).is_multiple_of(2));
        let mut actual = vec![0; block_width * block_height];
        let clamps = [false, clamp_row0, clamp_col0];
        if !luma.block_luma_420(x, y, block_width, clamps, None, &mut None, &mut actual) {
            continue;
        }
        fast_runs += 1;
        for (row, actual) in actual.chunks_exact(block_width).enumerate() {
            let rows = luma.row(y + row, clamp_row0 && row == 0, None).unwrap();
            let mut expected = vec![0; block_width];
            baseline_simd!(simd => luma.q3_run(simd, &rows, x, clamp_col0, false, &mut expected));
            assert_eq!(
                actual, expected,
                "F={F} x={x} y={y} {block_width}x{block_height} row={row}"
            );
        }
    }
    assert!(fast_runs > 10, "{fast_runs}");
}

#[test]
fn mhccp_420_u8_reference_luma_matches_q3_runs() {
    let mut state = 29;
    for round in 0..20 {
        let width = 64 + 4 * (lcg(&mut state) % 40);
        let height = 32 + 4 * (lcg(&mut state) % 30);
        let mut frame = workspace_sized(PixelFormat::Yuv420, width, height);
        for y in 0..height {
            for x in 0..width {
                let sample = (lcg(&mut state) % 256) as u8 | if round % 5 == 0 { 255 } else { 0 };
                frame
                    .set_reconstructed_sample(PlaneId::Y, x, y, sample)
                    .unwrap();
            }
        }
        check_u8_reference_luma::<0>(&frame, &mut state);
        check_u8_reference_luma::<1>(&frame, &mut state);
        check_u8_reference_luma::<2>(&frame, &mut state);
    }
}

/// The § 7.13.6 MHCCP prediction of one plane sample by sample: `CflRef`
/// from the per-sample [`q3`] and the frame, the § 7.13.7 edge samples in
/// spec order for the shared solve, then the § 7.13.6 sample formula.
#[allow(
    clippy::many_single_char_names,
    clippy::needless_range_loop,
    reason = "the spec's names and loops"
)]
fn spec_mhccp<T: ReconSample>(
    frame: &CurrentFrameWorkspace<T>,
    plane_id: PlaneId,
    block: CflBlock,
    neighbours: NeighbourAvailability,
    mh_dir: u8,
    filter: u8,
) -> Vec<i32> {
    let g = MhccpGeometry::new(frame, plane_id, block, neighbours).unwrap();
    let (w, h) = (block.width, block.height);
    let (above, left, rw, rh) = (g.above, g.left, g.reference_width, g.reference_height);
    let mut luma = vec![vec![0i32; rw.max(left + w)]; rh.max(above + h)];
    let mut chroma = luma.clone();
    let plane = PlaneSamples::of(frame, plane_id).unwrap();
    for i in 0..rh {
        for j in 0..rw {
            let (x, y) = (block.x + j - left, block.y + i - above);
            if i < above || j < left {
                let row = plane
                    .row_or_err(y.max(g.min_chroma_ref_y).min(plane.height - 1))
                    .unwrap();
                chroma[i][j] = i32::from(row[x.min(plane.width - 1)].to_u16());
            }
            if (i < above || j < left + w) && (i < above + h || j < left) {
                luma[i][j] = q3(frame, filter, x, y, j == 0, i == 0, g.min_luma_ref_y) >> 3;
            }
        }
    }
    let v0 = |i: usize, j: usize| match mh_dir {
        0 => luma[i][j],
        1 => luma[i.saturating_sub(1)][j],
        _ => luma[i][j.saturating_sub(1)],
    };
    let mut edges: [[Vec<i16>; 2]; 2] = Default::default();
    let mut targets: [Vec<u16>; 2] = Default::default();
    for i in 1..rh.saturating_sub(1) {
        for j in 1..rw.saturating_sub(1) {
            if i < above || j < left {
                let e = usize::from(i >= above);
                edges[e][0].push((8 * v0(i, j)) as i16);
                edges[e][1].push((8 * luma[i][j]) as i16);
                targets[e].push(chroma[i][j] as u16);
            }
        }
    }
    let edges = [0, 1].map(|e| MhccpEdge {
        linear: &edges[e][0],
        centre: &edges[e][1],
    });
    let [params] = derive_mhccp_params(edges, [[&targets[0], &targets[1]]], block.bit_depth);
    let bits = u32::from(block.bit_depth.bits());
    let mut out = Vec::new();
    for i in above..above + h {
        for j in left..left + w {
            let a = luma[i][j];
            let v = [
                v0(i, j),
                splot_recon::math::round2_i32(a * a, bits),
                1 << (bits - 1),
            ];
            let t = (0..3).fold(0i32, |t, k| {
                t + splot_recon::mul_fixed32_adapt(params[k], v[k], splot_recon::MHCCP_BITS)
            });
            out.push(t.clamp(0, i32::from(block.bit_depth.max_sample())));
        }
    }
    out
}

fn check_mhccp_against_spec<T: ReconSample>(
    frame: &mut CurrentFrameWorkspace<T>,
    state: &mut u64,
    bit_depth: BitDepth,
) -> usize {
    let size = frame.plane(PlaneId::U).unwrap().storage_size();
    let mut scratch = crate::pipeline::general_intra::GeneralIntraReconScratch::default();
    let mut checked = 0;
    for _ in 0..120 {
        let (log2_w, log2_h) = [
            (2, 2),
            (2, 3),
            (3, 2),
            (3, 3),
            (4, 3),
            (3, 4),
            (4, 4),
            (5, 4),
            (5, 5),
            (2, 4),
        ][lcg(state) % 10];
        let (w, h) = (1 << log2_w, 1 << log2_h);
        let x = (4 * (lcg(state) % (size.width() / 4))) & !(w - 1);
        let y = (4 * (lcg(state) % (size.height() / 4))) & !(h - 1);
        let neighbours = NeighbourAvailability::new(
            y > 0 && !lcg(state).is_multiple_of(5),
            x > 0 && !lcg(state).is_multiple_of(5),
            lcg(state) % 9,
            lcg(state) % 9,
        );
        let (mh_dir, filter, sb_mib) = (
            lcg(state) % 3,
            (lcg(state) % 3) as u8,
            [16, 32][lcg(state) % 2],
        );
        let direction = [
            CflMultiDirection::Direct,
            CflMultiDirection::Above,
            CflMultiDirection::Left,
        ][mh_dir];
        let block = cfl_block(x, y, w, h, sb_mib, bit_depth);
        let expected = MhccpGeometry::new(frame, PlaneId::U, block, neighbours)
            .is_ok()
            .then(|| spec_mhccp(frame, PlaneId::U, block, neighbours, mh_dir as u8, filter));
        let result = reconstruct_general_intra_chroma_cfl_block_into(
            &mut scratch,
            frame,
            zero_block().view(&[]),
            PlaneId::U,
            x,
            y,
            log2_w,
            log2_h,
            0,
            CflParams::Multi { direction },
            filter,
            sb_mib,
            neighbours,
            bit_depth,
        );
        let Some(expected) = expected else {
            assert!(result.is_err());
            continue;
        };
        result.unwrap();
        for (i, &expected) in expected.iter().enumerate() {
            let (sx, sy) = (x + i % w, y + i / w);
            if sx < size.width() && sy < size.height() {
                let got = frame
                    .reconstructed_sample(PlaneId::U, sx, sy)
                    .unwrap()
                    .to_u16();
                assert_eq!(
                    i32::from(got),
                    expected,
                    "{w}x{h} at ({x},{y}) dir {mh_dir} filter {filter} {neighbours:?} sample {i}"
                );
            }
        }
        checked += 1;
    }
    checked
}

fn randomise<T: ReconSample>(frame: &mut CurrentFrameWorkspace<T>, state: &mut u64, max: usize) {
    for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
        let size = frame.plane(plane).unwrap().storage_size();
        for y in 0..size.height() {
            for x in 0..size.width() {
                let smooth = (x * 3 + y * 5 + lcg(state) % 9) % (max + 1);
                let sample = if lcg(state).is_multiple_of(4) {
                    lcg(state) % (max + 1)
                } else {
                    smooth
                };
                let sample = T::try_from_u16(sample as u16).unwrap();
                frame.set_reconstructed_sample(plane, x, y, sample).unwrap();
            }
        }
    }
}

#[test]
fn mhccp_blocks_match_spec_capture_derivation_and_prediction() {
    let mut state = 41;
    for (pixel_format, width, height) in [
        (PixelFormat::Yuv420, 136, 100),
        (PixelFormat::Yuv422, 104, 72),
        (PixelFormat::Yuv444, 72, 68),
    ] {
        let mut frame = workspace_sized(pixel_format, width, height);
        randomise(&mut frame, &mut state, 255);
        assert!(check_mhccp_against_spec(&mut frame, &mut state, BitDepth::Eight) > 60);
    }
    let mut frame = workspace_10bit_420(136, 100);
    randomise(&mut frame, &mut state, 1023);
    assert!(check_mhccp_against_spec(&mut frame, &mut state, BitDepth::Ten) > 60);
}

/// The scalar `lumaAvg`, DC and `implicitAlpha` of § 7.13.5 sample by
/// sample: the oracle of [`CflLumaEdges::average_q3`] and
/// [`CflLumaEdges::chroma_models`].
fn scalar_models<T: ReconSample>(
    edges: &CflLumaEdges,
    above: Option<&[T]>,
    left: Option<&[T]>,
    block: CflBlock,
    neighbours: NeighbourAvailability,
    derive: bool,
) -> (i32, (i32, i32)) {
    let (w, h) = (block.width, block.height);
    let stepped = |len: usize| if len > 32 { 2 } else { 1 };
    let sum = |edge: &[i32]| -> u32 {
        edge.iter()
            .step_by(stepped(edge.len()))
            .map(|&v| v as u32)
            .sum()
    };
    let luma = |edge: &[i16]| edge.iter().map(|&v| i32::from(v)).collect::<Vec<_>>();
    let chroma = |edge: &[T]| {
        edge.iter()
            .map(|v| i32::from(v.to_u16()))
            .collect::<Vec<_>>()
    };
    let (mut luma_sum, mut count) = (0, 0);
    if neighbours.has_above() && block.y > 0 {
        luma_sum += sum(&luma(&edges.above[..w]));
        count += (w / stepped(w)) as u32;
    }
    if neighbours.has_left() && block.x > 0 {
        luma_sum += sum(&luma(&edges.left[..h]));
        count += (h / stepped(h)) as u32;
    }
    let bits = block.bit_depth.bits();
    let average = if count == 0 {
        8 << (bits - 1)
    } else {
        i32::from(approx_divide(luma_sum, count).unwrap().min((8 << bits) - 1))
    };
    let dc: T = splot_recon::predict_intra_dc_value_from_edge_sums(
        block.bit_depth,
        block.size,
        left.map(|left| sum(&chroma(&left[..h]))),
        above.map(|above| sum(&chroma(&above[..w]))),
    )
    .unwrap();
    let (mut num_above, mut num_left) = (0, 0);
    if derive {
        let (a, l) = (neighbours.has_above(), neighbours.has_left());
        (num_above, num_left) = match (a, l) {
            (true, true) if w > 2 * h => (8, 0),
            (true, true) if h > 2 * w => (0, 8),
            (true, true) => (4, 4),
            _ => (if a { 8 } else { 0 }, if l { 8 } else { 0 }),
        };
    }
    let (num_above, num_left) = (num_above.min(w), num_left.min(h));
    let (mut sx, mut sxx, mut sy, mut sxy) = (0, 0, 0, 0);
    for (num, len, luma, samples) in [
        (num_above, w, &edges.above, above),
        (num_left, h, &edges.left, left),
    ] {
        if num == 0 {
            continue;
        }
        let samples = samples.unwrap();
        let step = len / num;
        for i in (step / 2..len).step_by(step) {
            let (x, y) = (i32::from(luma[i] >> 3), i32::from(samples[i].to_u16()));
            (sx, sxx, sy, sxy) = (sx + x, sxx + x * x, sy + y, sxy + x * y);
        }
    }
    let count = (num_above + num_left) as i32;
    let mut alpha = 0;
    if count > 0 {
        let (der, nor) = (sxx - sx * sx / count, sxy - sx * sy / count);
        if der != 0 && nor != 0 {
            alpha = i32::from(resolve_division(nor, der, CFL_DERIVED_ALPHA_SHIFT));
        }
    }
    (average, (i32::from(dc.to_u16()), alpha))
}

fn check_lane_models<T: ReconSample>(state: &mut u64, bit_depth: BitDepth) -> usize {
    let max = u64::from(bit_depth.max_sample());
    let mut checked = 0;
    for round in 0..3000 {
        let (w, h) = (4usize << (lcg(state) % 5), 4usize << (lcg(state) % 5));
        let pick = |state: &mut u64| match round % 4 {
            0 => (lcg(state) as u64 % 2) * max,
            1 => max,
            _ => lcg(state) as u64 % (max + 1),
        };
        let mut edges = CflLumaEdges::default();
        for v in edges.above.iter_mut().chain(edges.left.iter_mut()) {
            *v = (8 * pick(state) + lcg(state) as u64 % 8) as i16;
        }
        let chroma = |state: &mut u64, len: usize| -> Vec<T> {
            (0..len)
                .map(|_| T::try_from_u16(pick(state) as u16).unwrap())
                .collect()
        };
        let lens = [w + lcg(state) % 20, h + lcg(state) % 20];
        let (above, left) = (chroma(state, lens[0]), chroma(state, lens[1]));
        let neighbours = NeighbourAvailability::new(
            !lcg(state).is_multiple_of(4),
            !lcg(state).is_multiple_of(4),
            0,
            0,
        );
        let (x, y) = (4 * (lcg(state) % 3), 4 * (lcg(state) % 3));
        let block = cfl_block(x, y, w, h, 16, bit_depth);
        let derive = lcg(state).is_multiple_of(2);
        let above = (neighbours.has_above() && y > 0).then_some(&above[..]);
        let left = (neighbours.has_left() && x > 0).then_some(&left[..]);
        let models = edges.chroma_models(
            above.map(|a| [a]),
            left.map(|l| [l]),
            block,
            neighbours,
            derive,
        );
        let pair = edges.chroma_models(
            above.map(|a| [a, a]),
            left.map(|l| [l, l]),
            block,
            neighbours,
            derive,
        );
        if models.is_err() {
            assert!(derive && pair.is_err());
            continue;
        }
        let expected = scalar_models(&edges, above, left, block, neighbours, derive);
        let average = edges.average_q3(block, neighbours).unwrap();
        assert_eq!(
            (average, models.unwrap()[0]),
            expected,
            "{w}x{h} ({x},{y}) {neighbours:?} {derive}"
        );
        assert_eq!(pair.unwrap(), [expected.1; 2]);
        checked += 1;
    }
    checked
}

#[test]
fn cfl_lane_models_match_scalar_sums() {
    let mut state = 0xd0c_a1fa;
    assert!(check_lane_models::<u8>(&mut state, BitDepth::Eight) > 1000);
    assert!(check_lane_models::<u16>(&mut state, BitDepth::Ten) > 1000);
}

fn check_cfl_predict_pair<T: ReconSample>(max: i32, state: &mut u64) {
    let (width, height) = (4 << (lcg(state) % 5), 4 << (lcg(state) % 5));
    let strides = [width + lcg(state) % 9, width + lcg(state) % 9];
    let sample = |state: &mut u64| (lcg(state) % (8 * max as usize + 1)) as i32;
    let average = sample(state);
    let alpha = |state: &mut u64| match lcg(state) % 4 {
        0 => [-2047, 2047, 0, -4096][lcg(state) % 4],
        _ => (lcg(state) % 4095) as i32 - 2047,
    };
    let models = [0, 1].map(|_| {
        [
            (lcg(state) % (max as usize + 1)) as i32,
            alpha(state),
            average,
            max,
        ]
    });
    let luma: Vec<i16> = (0..width * height).map(|_| sample(state) as i16).collect();
    let sentinel = T::try_from_u16(7).unwrap();
    let mut outs = strides.map(|stride| vec![sentinel; (height - 1) * stride + width]);
    let [u, v] = &mut outs;
    cfl_predict_pair(
        models,
        &luma,
        width,
        [
            (u.as_mut_slice(), strides[0]),
            (v.as_mut_slice(), strides[1]),
        ],
    )
    .unwrap();
    for ((out, model), stride) in outs.iter().zip(models).zip(strides) {
        for (i, &got) in out.iter().enumerate() {
            let (row, col) = (i / stride, i % stride);
            let expected = if col < width {
                cfl_predict(model, i32::from(luma[row * width + col])) as u16
            } else {
                7
            };
            assert_eq!(got.to_u16(), expected, "{width}x{height} {model:?} at {i}");
        }
    }
}

#[test]
fn cfl_predict_pairs_match_per_sample_formula() {
    let mut state = 0x9a12;
    for _ in 0..400 {
        check_cfl_predict_pair::<u8>(255, &mut state);
        check_cfl_predict_pair::<u16>(255, &mut state);
        check_cfl_predict_pair::<u16>(1023, &mut state);
    }
}

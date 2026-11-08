// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

const ZERO_CHROMA: [i16; WIENER_NS_CHROMA_COEFFS] = [0; WIENER_NS_CHROMA_COEFFS];

#[test]
fn chroma_tap_radius_covers_config_table() {
    let max_offset = WIENER_NS_CONFIG_UV
        .iter()
        .map(|&(dy, dx, _)| dy.unsigned_abs().max(dx.unsigned_abs()))
        .max()
        .unwrap();
    assert_eq!(WIENER_NS_CHROMA_TAP_RADIUS, max_offset);
}

fn chroma_params(
    width: usize,
    height: usize,
    output_stride: usize,
    bit_depth: BitDepth,
    coeffs: &[i16; WIENER_NS_CHROMA_COEFFS],
) -> WienerNsChromaFilter<'_> {
    WienerNsChromaFilter {
        x: 0,
        y: 0,
        width,
        height,
        output_stride,
        bit_depth,
        coeffs,
        subsampling_x: 0,
        subsampling_y: 0,
        luma_start_x: 0,
        luma_end_x: 15,
        mi_rows: 4,
        cfl_ds_filter_index: 0,
    }
}

#[test]
fn padded_u16_chroma_lanes_match_the_scalar_kernel() {
    const RADIUS: usize = WIENER_NS_CHROMA_TAP_RADIUS;
    let mut coeffs = ZERO_CHROMA;
    for (index, coeff) in coeffs.iter_mut().enumerate() {
        *coeff = (index as i16 * 13 % 41) - 20;
    }
    for &bit_depth in &[BitDepth::Eight, BitDepth::Ten] {
        let max = bit_depth.max_sample();
        for &width in &[64, 65, 70, 127, 128, 191, 200] {
            let height = 3;
            let stride = width + 2 * RADIUS;
            let plane = |seed: usize| -> Vec<u16> {
                (0..stride * (height + 2 * RADIUS))
                    .map(|index| match (index + seed) % 4 {
                        0 => max,
                        1 => 0,
                        _ => ((index * 37 + seed) % (usize::from(max) + 1)) as u16,
                    })
                    .collect()
            };
            let chroma = plane(0);
            let luma_ds = plane(3);
            let params = chroma_params(width, height, width, bit_depth, &coeffs);
            let mut lanes = vec![0u16; width * height];
            let mut scalar = vec![0u16; width * height];
            filter_chroma_padded_simd(&mut lanes, &params, &chroma, stride, &luma_ds, stride, max);
            filter_chroma_padded_u16_scalar(
                &mut scalar,
                &params,
                &chroma,
                stride,
                &luma_ds,
                stride,
                max,
            );
            assert_eq!(lanes, scalar, "{bit_depth:?} width {width}");
        }
    }
}

#[test]
fn zero_coefficients_copy_source_samples() {
    let mut output = [9u8; 6];
    let params = chroma_params(2, 2, 3, BitDepth::Eight, &ZERO_CHROMA);

    wiener_ns_filter_chroma_block(
        &mut output,
        &params,
        |x, y| u8::try_from(20 + y * 10 + x).unwrap(),
        |_x, _y| 0,
    )
    .unwrap();

    assert_eq!(output, [20, 21, 9, 30, 31, 9]);
}

#[test]
fn hand_computed_chroma_tap_accumulation() {
    let mut coeffs = ZERO_CHROMA;
    coeffs[0] = 4;
    let mut output = [0u8; 1];
    let params = chroma_params(1, 1, 1, BitDepth::Eight, &coeffs);

    wiener_ns_filter_chroma_block(
        &mut output,
        &params,
        |x, y| {
            if x == 0 && y == 1 { 120 } else { 100 }
        },
        |_x, _y| 0,
    )
    .unwrap();

    assert_eq!(output, [101]);
}

#[test]
fn hand_computed_luma_tap_contribution_without_subsampling() {
    let mut coeffs = ZERO_CHROMA;
    coeffs[6] = 4;
    let mut output = [0u8; 1];
    let params = chroma_params(1, 1, 1, BitDepth::Eight, &coeffs);

    wiener_ns_filter_chroma_block(
        &mut output,
        &params,
        |_x, _y| 100,
        |x, y| {
            if x == 0 && y == 1 { 70 } else { 50 }
        },
    )
    .unwrap();

    assert_eq!(output, [101]);
}

/// Runs the 4:2:0 `get_luma_sample` single-output case for one
/// `cfl_ds_filter_index`, asserting the filtered sample lands on 101.
fn assert_luma_420_filters_to_101(cfl_ds_filter_index: u8, luma_at: impl Fn(isize, isize) -> u8) {
    let mut coeffs = ZERO_CHROMA;
    coeffs[6] = 4;
    let mut output = [0u8; 1];
    let mut params = chroma_params(1, 1, 1, BitDepth::Eight, &coeffs);
    params.subsampling_x = 1;
    params.subsampling_y = 1;
    params.luma_end_x = 7;
    params.mi_rows = 2;
    params.cfl_ds_filter_index = cfl_ds_filter_index;

    wiener_ns_filter_chroma_block(&mut output, &params, |_x, _y| 100, luma_at).unwrap();

    assert_eq!(output, [101]);
}

fn two_by_two_luma_at(x: isize, y: isize) -> u8 {
    match (x, y) {
        (0, 0) => 10,
        (1, 0) => 14,
        (0, 1) => 18,
        (1, 1) => 22,
        (0, 2) => 30,
        (1, 2) => 34,
        (0, 3) => 38,
        (1, 3) => 42,
        _ => 0,
    }
}

#[test]
fn luma_420_filter_index_zero_averages_two_by_two() {
    assert_luma_420_filters_to_101(0, two_by_two_luma_at);
}

#[test]
fn luma_420_filter_index_one_uses_vertical_left_column() {
    assert_luma_420_filters_to_101(1, |x, y| match (x, y) {
        (0, 0) => 10,
        (0, 1) => 30,
        (0, 2) => 50,
        (0, 3) => 70,
        _ => 0,
    });
}

#[test]
fn luma_420_filter_index_three_maps_to_zero() {
    assert_luma_420_filters_to_101(3, two_by_two_luma_at);
}

#[test]
fn luma_422_reads_scaled_direct_luma_sample_without_420_filter() {
    let mut coeffs = ZERO_CHROMA;
    coeffs[6] = 4;
    let mut output = [0u8; 1];
    let mut params = chroma_params(1, 1, 1, BitDepth::Eight, &coeffs);
    params.subsampling_x = 1;
    params.subsampling_y = 0;
    params.luma_end_x = 7;

    wiener_ns_filter_chroma_block(
        &mut output,
        &params,
        |_x, _y| 100,
        |x, y| {
            if x == 0 && y == 1 { 70 } else { 50 }
        },
    )
    .unwrap();

    assert_eq!(output, [101]);
}

/// A multi-sample block (shared downsampled-luma cache) must be
/// bit-identical to filtering every sample as its own 1x1 block (fresh
/// cache per sample), across chroma and luma taps at 4:2:0 subsampling.
#[test]
fn block_filtering_matches_per_sample_blocks_bit_exactly() {
    let width = 9;
    let height = 5;
    let mut coeffs = ZERO_CHROMA;
    coeffs[0] = 13;
    coeffs[2] = -7;
    coeffs[5] = 3;
    coeffs[6] = 21;
    coeffs[10] = -9;
    coeffs[17] = 5;
    let chroma_at =
        |x: isize, y: isize| -> u16 { ((x * 31 + y * 17 + 512).rem_euclid(1024)) as u16 };
    let luma_at = |x: isize, y: isize| -> u16 { ((x * 13 + y * 41 + 700).rem_euclid(1024)) as u16 };
    let mut params = chroma_params(width, height, width, BitDepth::Ten, &coeffs);
    params.x = 6;
    params.y = 4;
    params.subsampling_x = 1;
    params.subsampling_y = 1;
    params.luma_start_x = 0;
    params.luma_end_x = 63;
    params.mi_rows = 16;
    params.cfl_ds_filter_index = 0;

    let mut block_output = vec![0u16; width * height];
    wiener_ns_filter_chroma_block(&mut block_output, &params, chroma_at, luma_at).unwrap();

    for r in 0..height {
        for c in 0..width {
            let mut single = [0u16; 1];
            let mut single_params = params;
            single_params.x = params.x + c;
            single_params.y = params.y + r;
            single_params.width = 1;
            single_params.height = 1;
            single_params.output_stride = 1;
            wiener_ns_filter_chroma_block(&mut single, &single_params, chroma_at, luma_at).unwrap();
            assert_eq!(
                block_output[r * width + c],
                single[0],
                "cached block filtering diverged at ({c}, {r})"
            );
        }
    }
}

/// The padded kernel must match the callback § 7.20.3 reference across the
/// AV2 chroma formats, block positions that hit both the clipped and
/// unclipped downsample paths, and every `cfl_ds_filter_index`.
#[test]
fn padded_filter_matches_callback_path() {
    let height = 5;
    for width in [9, 21] {
        let mut coeffs = ZERO_CHROMA;
        for (index, coeff) in coeffs.iter_mut().enumerate() {
            *coeff = (index as i16 * 7 % 23) - 11;
        }
        let chroma_at =
            |x: isize, y: isize| -> u16 { ((x * 31 + y * 17 + 512).rem_euclid(1024)) as u16 };
        let luma_at =
            |x: isize, y: isize| -> u16 { ((x * 13 + y * 41 + 700).rem_euclid(1024)) as u16 };
        for (sub_x, sub_y) in [(1u8, 1u8), (1, 0), (0, 0)] {
            for (block_x, block_y) in [(0usize, 0usize), (8, 8)] {
                let mut params = chroma_params(width, height, width, BitDepth::Ten, &coeffs);
                params.x = block_x;
                params.y = block_y;
                params.subsampling_x = sub_x;
                params.subsampling_y = sub_y;
                params.luma_start_x = 0;
                params.luma_end_x = 64;
                params.mi_rows = 16;

                let chroma_stride = width + 2 * WIENER_NS_CHROMA_TAP_RADIUS;
                let chroma_height = height + 2 * WIENER_NS_CHROMA_TAP_RADIUS;
                let chroma_origin_x = params.x as isize - WIENER_NS_CHROMA_TAP_RADIUS as isize;
                let chroma_origin_y = params.y as isize - WIENER_NS_CHROMA_TAP_RADIUS as isize;
                let chroma: Vec<u16> = (0..chroma_height)
                    .flat_map(|row| {
                        (0..chroma_stride).map(move |col| {
                            chroma_at(
                                chroma_origin_x + col as isize,
                                chroma_origin_y + row as isize,
                            )
                        })
                    })
                    .collect();
                let luma_stride = chroma_stride << sub_x;
                let luma_height = chroma_height << sub_y;
                let luma_origin_x = chroma_origin_x << sub_x;
                let luma_origin_y = chroma_origin_y << sub_y;
                let luma: Vec<u16> = (0..luma_height)
                    .flat_map(|row| {
                        (0..luma_stride).map(move |col| {
                            luma_at(luma_origin_x + col as isize, luma_origin_y + row as isize)
                        })
                    })
                    .collect();
                let geometry = (width, height, (sub_x, sub_y));
                let sources = [WienerNsChromaPaddedSource::new(
                    &chroma,
                    chroma_stride,
                    &luma,
                    luma_stride,
                    geometry.0,
                    geometry.1,
                    geometry.2,
                )
                .unwrap()];
                let chroma8: Vec<u8> = chroma.iter().map(|&value| value as u8).collect();
                let luma8: Vec<u8> = luma.iter().map(|&value| value as u8).collect();
                let source8 = WienerNsChromaPaddedSource::new(
                    &chroma8,
                    chroma_stride,
                    &luma8,
                    luma_stride,
                    geometry.0,
                    geometry.1,
                    geometry.2,
                )
                .unwrap();
                let mut scratch = WienerNsChromaScratch::default();
                let mut scratch8 = WienerNsChromaScratch::default();

                for filter_index in [0, 1, 2, 3] {
                    params.cfl_ds_filter_index = filter_index;
                    let mut callback = vec![0u16; width * height];
                    wiener_ns_filter_chroma_block(&mut callback, &params, chroma_at, luma_at)
                        .unwrap();
                    for source in &sources {
                        let mut padded = vec![0u16; width * height];
                        wiener_ns_filter_chroma_block_padded_u16_into(
                            &mut padded,
                            &params,
                            source,
                            &mut scratch,
                        )
                        .unwrap();
                        assert_eq!(
                            padded, callback,
                            "sub=({sub_x},{sub_y}) block=({block_x},{block_y}) \
                         cfl_ds_filter_index={filter_index}"
                        );
                    }
                    let mut params8 = params;
                    params8.bit_depth = BitDepth::Eight;
                    let mut callback8 = vec![0u8; width * height];
                    wiener_ns_filter_chroma_block(
                        &mut callback8,
                        &params8,
                        |x, y| chroma_at(x, y) as u8,
                        |x, y| luma_at(x, y) as u8,
                    )
                    .unwrap();
                    let mut padded8 = vec![0u8; width * height];
                    wiener_ns_filter_chroma_block_padded_u8_into(
                        &mut padded8,
                        &params8,
                        &source8,
                        &mut scratch8,
                    )
                    .unwrap();
                    assert_eq!(padded8, callback8, "u8 cfl_ds_filter_index={filter_index}");
                }
            }
        }
    }
}

#[test]
fn padded_u16_filter_writes_strided_output() {
    let width = 65;
    let height = 3;
    let source_stride = width + 2 * WIENER_NS_CHROMA_TAP_RADIUS;
    let source_rows = height + 2 * WIENER_NS_CHROMA_TAP_RADIUS;
    let chroma: Vec<u16> = (0..source_stride * source_rows)
        .map(|index| (index * 37 % 1024) as u16)
        .collect();
    let luma: Vec<u16> = (0..source_stride * source_rows)
        .map(|index| (index * 53 % 1024) as u16)
        .collect();
    let source = WienerNsChromaPaddedSource::new(
        &chroma,
        source_stride,
        &luma,
        source_stride,
        width,
        height,
        (0, 0),
    )
    .unwrap();
    let mut coeffs = ZERO_CHROMA;
    for (index, coeff) in coeffs.iter_mut().enumerate() {
        *coeff = (index as i16 * 7 % 23) - 11;
    }

    let mut packed_params = chroma_params(width, height, width, BitDepth::Ten, &coeffs);
    packed_params.luma_end_x = 127;
    packed_params.mi_rows = 16;
    let mut packed = vec![0; width * height];
    wiener_ns_filter_chroma_block_padded_u16_into(
        &mut packed,
        &packed_params,
        &source,
        &mut WienerNsChromaScratch::default(),
    )
    .unwrap();

    let output_stride = width + 7;
    let mut strided_params = packed_params;
    strided_params.output_stride = output_stride;
    let mut strided = vec![u16::MAX; output_stride * height];
    let mut scratch = WienerNsChromaScratch::default();
    wiener_ns_filter_chroma_block_padded_u16_into(
        &mut strided,
        &strided_params,
        &source,
        &mut scratch,
    )
    .unwrap();

    for row in 0..height {
        assert_eq!(
            &strided[row * output_stride..row * output_stride + width],
            &packed[row * width..(row + 1) * width],
        );
        assert!(
            strided[row * output_stride + width..(row + 1) * output_stride]
                .iter()
                .all(|&sample| sample == u16::MAX)
        );
    }
}

fn packed_and_u16_strided_match<T: ReconSample>(
    bit_depth: BitDepth,
    width: usize,
    height: usize,
    (sub_x, sub_y): (u8, u8),
    filter_index: u8,
    coeffs: &[i16; WIENER_NS_CHROMA_COEFFS],
) {
    let max = usize::from(bit_depth.max_sample());
    let chroma_stride = width + 2 * WIENER_NS_CHROMA_TAP_RADIUS;
    let chroma_rows = height + 2 * WIENER_NS_CHROMA_TAP_RADIUS;
    let luma_stride = chroma_stride << sub_x;
    let luma_rows = chroma_rows << sub_y;
    let samples = |len: usize, factor: usize| {
        (0..len)
            .map(|index| T::try_from_u16(((index * factor + 17) % (max + 1)) as u16).unwrap())
            .collect::<Vec<_>>()
    };
    let chroma = samples(chroma_stride * chroma_rows, 37);
    let luma = samples(luma_stride * luma_rows, 53);
    let source = WienerNsChromaPaddedSource::new(
        &chroma,
        chroma_stride,
        &luma,
        luma_stride,
        width,
        height,
        (sub_x, sub_y),
    )
    .unwrap();
    let mut packed_params = chroma_params(width, height, width, bit_depth, coeffs);
    packed_params.subsampling_x = sub_x;
    packed_params.subsampling_y = sub_y;
    packed_params.cfl_ds_filter_index = filter_index;
    packed_params.luma_end_x = (chroma_stride << sub_x) - 1;
    packed_params.mi_rows = (chroma_rows << sub_y).div_ceil(MI_SIZE);

    let mut packed = vec![0u16; width * height];
    wiener_ns_filter_chroma_block_padded_u16_into(
        &mut packed,
        &packed_params,
        &source,
        &mut WienerNsChromaScratch::default(),
    )
    .unwrap();

    let stride = width + 11;
    let mut strided_params = packed_params;
    strided_params.output_stride = stride;
    let mut strided = vec![u16::MAX; stride * height];
    let mut scratch = WienerNsChromaScratch::default();
    wiener_ns_filter_chroma_block_padded_u16_into(
        &mut strided,
        &strided_params,
        &source,
        &mut scratch,
    )
    .unwrap();

    for row in 0..height {
        assert_eq!(
            &strided[row * stride..row * stride + width],
            &packed[row * width..(row + 1) * width],
            "{} {width}x{height} sub=({sub_x},{sub_y}) filter={filter_index}",
            T::TYPE_NAME,
        );
        assert!(
            strided[row * stride + width..(row + 1) * stride]
                .iter()
                .all(|&sample| sample == u16::MAX)
        );
    }

    if bit_depth == BitDepth::Eight {
        let mut direct_u8 = vec![0xde; stride * height];
        wiener_ns_filter_chroma_block_padded_u8_into(
            &mut direct_u8,
            &strided_params,
            &source,
            &mut WienerNsChromaScratch::default(),
        )
        .unwrap();
        for row in 0..height {
            let expected: Vec<_> = packed[row * width..(row + 1) * width]
                .iter()
                .map(|sample| sample.to_u16() as u8)
                .collect();
            assert_eq!(
                &direct_u8[row * stride..row * stride + width],
                expected,
                "direct u8 {width}x{height} sub=({sub_x},{sub_y}) filter={filter_index}",
            );
            assert!(
                direct_u8[row * stride + width..(row + 1) * stride]
                    .iter()
                    .all(|&sample| sample == 0xde)
            );
        }
    }
}

#[test]
fn packed_and_u16_strided_chroma_outputs_match_all_runtime_shapes() {
    let mins = [
        -24, -24, -14, -14, -16, -16, -16, -16, -16, -16, -8, -8, -8, -8, -8, -8, -8, -8,
    ];
    let maxs = [
        39, 39, 17, 17, 15, 15, 15, 15, 15, 15, 7, 7, 7, 7, 7, 7, 7, 7,
    ];
    let shapes = [(32, 4), (64, 8), (128, 28), (256, 56), (19, 3)];
    for (case, &(width, height)) in shapes.iter().enumerate() {
        for (sub_x, sub_y) in [(1, 1), (1, 0), (0, 0)] {
            for filter_index in 0..=3 {
                let coeffs = if (case + usize::from(filter_index)) % 2 == 0 {
                    &mins
                } else {
                    &maxs
                };
                packed_and_u16_strided_match::<u8>(
                    BitDepth::Eight,
                    width,
                    height,
                    (sub_x, sub_y),
                    filter_index,
                    coeffs,
                );
                packed_and_u16_strided_match::<u16>(
                    BitDepth::Ten,
                    width,
                    height,
                    (sub_x, sub_y),
                    filter_index,
                    coeffs,
                );
            }
        }
    }
}

#[test]
fn u8_source_direct_simd_u16_output_has_a_mutation_sensitive_oracle() {
    let width = 64;
    let height = 1;
    let source_stride = width + 2 * WIENER_NS_CHROMA_TAP_RADIUS;
    let source_rows = height + 2 * WIENER_NS_CHROMA_TAP_RADIUS;
    let mut chroma = vec![100u8; source_stride * source_rows];
    chroma[3 * source_stride + 2] = 120;
    let luma = vec![0u8; source_stride * source_rows];
    let source = WienerNsChromaPaddedSource::new(
        &chroma,
        source_stride,
        &luma,
        source_stride,
        width,
        height,
        (0, 0),
    )
    .unwrap();
    let mut coeffs = ZERO_CHROMA;
    coeffs[0] = 4;
    let output_stride = width + 7;
    let params = chroma_params(width, height, output_stride, BitDepth::Eight, &coeffs);
    let mut output = vec![u16::MAX; output_stride];
    let mut scratch = WienerNsChromaScratch::default();

    wiener_ns_filter_chroma_block_padded_u16_into(&mut output, &params, &source, &mut scratch)
        .unwrap();

    assert_eq!(output[0], 101);
    assert!(output[1..width].iter().all(|&sample| sample == 100));
    assert!(output[width..].iter().all(|&sample| sample == u16::MAX));
}

#[test]
fn u8_source_direct_u16_output_validates_the_full_destination_first() {
    let width = 2;
    let height = 2;
    let source_stride = width + 2 * WIENER_NS_CHROMA_TAP_RADIUS;
    let source_rows = height + 2 * WIENER_NS_CHROMA_TAP_RADIUS;
    let chroma = vec![0u8; source_stride * source_rows];
    let luma = vec![0u8; source_stride * source_rows];
    let source = WienerNsChromaPaddedSource::new(
        &chroma,
        source_stride,
        &luma,
        source_stride,
        width,
        height,
        (0, 0),
    )
    .unwrap();
    let params = chroma_params(width, height, 4, BitDepth::Eight, &ZERO_CHROMA);
    let mut output = [77u16; 5];

    let error = wiener_ns_filter_chroma_block_padded_u16_into(
        &mut output,
        &params,
        &source,
        &mut WienerNsChromaScratch::default(),
    )
    .unwrap_err();

    assert_eq!(
        error,
        ReconError::WienerNsFilterOutputTooSmall {
            expected: 6,
            actual: 5,
        }
    );
    assert_eq!(output, [77; 5]);
}

#[test]
fn padded_u16_filter_validates_every_source_before_direct_output() {
    let width = 2;
    let height = 2;
    let source_stride = width + 2 * WIENER_NS_CHROMA_TAP_RADIUS;
    let source_rows = height + 2 * WIENER_NS_CHROMA_TAP_RADIUS;
    let mut chroma = vec![0u16; source_stride * source_rows];
    *chroma.last_mut().unwrap() = 1024;
    let luma = vec![0u16; source_stride * source_rows];
    let source = WienerNsChromaPaddedSource::new(
        &chroma,
        source_stride,
        &luma,
        source_stride,
        width,
        height,
        (0, 0),
    )
    .unwrap();
    let params = chroma_params(width, height, width + 1, BitDepth::Ten, &ZERO_CHROMA);
    let mut output = vec![77u16; params.output_stride * height];
    let mut scratch = WienerNsChromaScratch::default();

    let error =
        wiener_ns_filter_chroma_block_padded_u16_into(&mut output, &params, &source, &mut scratch)
            .unwrap_err();

    assert_eq!(
        error,
        ReconError::WienerNsFilterSourceSampleOutOfRange {
            x: source_stride as isize - 1,
            y: source_rows as isize - 1,
            value: 1024,
            max: 1023,
        }
    );
    assert_eq!(output, vec![77; params.output_stride * height]);
}

#[test]
fn chroma_only_source_filters_without_luma_taps_and_rejects_luma_taps() {
    let (width, height) = (9, 3);
    let stride = width + 2 * WIENER_NS_CHROMA_TAP_RADIUS;
    let chroma: Vec<u16> = (0..stride * (height + 2 * WIENER_NS_CHROMA_TAP_RADIUS))
        .map(|index| (index * 37 % 256) as u16)
        .collect();
    let chroma8: Vec<u8> = chroma.iter().map(|&value| value as u8).collect();
    let mut coeffs = [3, -2, 5, 1, -4, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    let params = chroma_params(width, height, width, BitDepth::Eight, &coeffs);
    let mut expected = vec![0u8; width * height];
    wiener_ns_filter_chroma_block(
        &mut expected,
        &params,
        |x, y| chroma8[(y + 2) as usize * stride + (x + 2) as usize],
        |_x, _y| 0,
    )
    .unwrap();
    let source = WienerNsChromaPaddedSource::chroma_only(&chroma, stride, width, height).unwrap();
    let mut simd = vec![0u8; width * height];
    wiener_ns_filter_chroma_block_padded_u8_into(
        &mut simd,
        &params,
        &source,
        &mut WienerNsChromaScratch::default(),
    )
    .unwrap();
    assert_eq!(simd, expected);

    coeffs[WIENER_NS_CHROMA_COEFFS - 1] = 1;
    let params = chroma_params(width, height, width, BitDepth::Eight, &coeffs);
    let mut output = vec![77u8; width * height];
    let error = wiener_ns_filter_chroma_block_padded_u8_into(
        &mut output,
        &params,
        &source,
        &mut WienerNsChromaScratch::default(),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        ReconError::WienerNsFilterOutputStrideTooSmall {
            stride_samples: 0,
            ..
        }
    ));
    assert_eq!(output, vec![77; width * height]);
}

#[test]
fn clip1_clamps_ten_bit_output() {
    let mut coeffs = ZERO_CHROMA;
    coeffs[0] = 512;
    let mut output = [0u16; 1];
    let params = chroma_params(1, 1, 1, BitDepth::Ten, &coeffs);

    wiener_ns_filter_chroma_block(
        &mut output,
        &params,
        |x, y| {
            if x == 0 && y == 1 { 1023 } else { 1000 }
        },
        |_x, _y| 0,
    )
    .unwrap();

    assert_eq!(output, [1023]);
}

#[test]
fn rejects_invalid_output_stride_fail_atomically() {
    let params = chroma_params(2, 2, 1, BitDepth::Eight, &ZERO_CHROMA);
    let mut output = [77u8; 4];

    let err =
        wiener_ns_filter_chroma_block(&mut output, &params, |_x, _y| 0, |_x, _y| 0).unwrap_err();

    assert_eq!(
        err,
        ReconError::WienerNsFilterOutputStrideTooSmall {
            stride_samples: 1,
            width: 2,
        }
    );
    assert_eq!(output, [77; 4]);
}

#[test]
fn rejects_short_output_fail_atomically() {
    let params = chroma_params(2, 2, 3, BitDepth::Eight, &ZERO_CHROMA);
    let mut output = [77u8; 4];

    let err =
        wiener_ns_filter_chroma_block(&mut output, &params, |_x, _y| 0, |_x, _y| 0).unwrap_err();

    assert_eq!(
        err,
        ReconError::WienerNsFilterOutputTooSmall {
            expected: 5,
            actual: 4,
        }
    );
    assert_eq!(output, [77; 4]);
}

#[test]
fn rejects_invalid_subsampling_fail_atomically() {
    let mut params = chroma_params(1, 1, 1, BitDepth::Eight, &ZERO_CHROMA);
    params.subsampling_x = 2;
    let mut output = [77u8; 1];

    let err =
        wiener_ns_filter_chroma_block(&mut output, &params, |_x, _y| 0, |_x, _y| 0).unwrap_err();

    assert_eq!(
        err,
        ReconError::LoopRestorationSourceInvalidSubsampling {
            subsampling_x: 2,
            subsampling_y: 0,
        }
    );
    assert_eq!(output, [77]);
}

#[test]
fn rejects_inverted_luma_x_bounds_fail_atomically() {
    let mut params = chroma_params(1, 1, 1, BitDepth::Eight, &ZERO_CHROMA);
    params.luma_start_x = 8;
    params.luma_end_x = 7;
    let mut output = [77u8; 1];

    let err =
        wiener_ns_filter_chroma_block(&mut output, &params, |_x, _y| 0, |_x, _y| 0).unwrap_err();

    assert_eq!(
        err,
        ReconError::LoopRestorationSourceInvalidBounds {
            field: "Wiener NS chroma luma x range",
        }
    );
    assert_eq!(output, [77]);
}

#[test]
fn rejects_luma_last_x_underflow_fail_atomically() {
    let mut params = chroma_params(1, 1, 1, BitDepth::Eight, &ZERO_CHROMA);
    params.subsampling_x = 1;
    params.luma_end_x = 0;
    let mut output = [77u8; 1];

    let err =
        wiener_ns_filter_chroma_block(&mut output, &params, |_x, _y| 0, |_x, _y| 0).unwrap_err();

    assert_eq!(
        err,
        ReconError::LoopRestorationSourceInvalidBounds {
            field: "Wiener NS chroma luma last x",
        }
    );
    assert_eq!(output, [77]);
}

#[test]
fn rejects_luma_last_y_underflow_fail_atomically() {
    let mut params = chroma_params(1, 1, 1, BitDepth::Eight, &ZERO_CHROMA);
    params.subsampling_y = 1;
    params.mi_rows = 0;
    let mut output = [77u8; 1];

    let err =
        wiener_ns_filter_chroma_block(&mut output, &params, |_x, _y| 0, |_x, _y| 0).unwrap_err();

    assert_eq!(
        err,
        ReconError::LoopRestorationSourceInvalidBounds {
            field: "Wiener NS chroma luma last y",
        }
    );
    assert_eq!(output, [77]);
}

#[test]
fn rejects_invalid_cfl_filter_index_fail_atomically() {
    let mut params = chroma_params(1, 1, 1, BitDepth::Eight, &ZERO_CHROMA);
    params.cfl_ds_filter_index = 4;
    let mut output = [77u8; 1];

    let err =
        wiener_ns_filter_chroma_block(&mut output, &params, |_x, _y| 0, |_x, _y| 0).unwrap_err();

    assert_eq!(
        err,
        ReconError::WienerNsFilterInvalidCflDsFilterIndex { index: 4 }
    );
    assert_eq!(output, [77]);
}

#[test]
fn rejects_source_sample_out_of_range_fail_atomically() {
    let params = chroma_params(1, 1, 1, BitDepth::Ten, &ZERO_CHROMA);
    let mut output = [77u16; 1];

    let err = wiener_ns_filter_chroma_block(&mut output, &params, |_x, _y| 1024u16, |_x, _y| 0)
        .unwrap_err();

    assert_eq!(
        err,
        ReconError::WienerNsFilterSourceSampleOutOfRange {
            x: 0,
            y: 0,
            value: 1024,
            max: 1023,
        }
    );
    assert_eq!(output, [77]);
}

#[test]
fn rejects_luma_sample_out_of_range_fail_atomically() {
    let mut coeffs = ZERO_CHROMA;
    coeffs[6] = 1;
    let params = chroma_params(1, 1, 1, BitDepth::Ten, &coeffs);
    let mut output = [77u16; 1];

    let err =
        wiener_ns_filter_chroma_block(&mut output, &params, |_x, _y| 100u16, |_x, _y| 1024u16)
            .unwrap_err();

    assert_eq!(
        err,
        ReconError::WienerNsFilterSourceSampleOutOfRange {
            x: 0,
            y: 0,
            value: 1024,
            max: 1023,
        }
    );
    assert_eq!(output, [77]);
}

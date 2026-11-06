// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::panic)]

use super::*;

const ZERO: [i16; WIENER_NS_LUMA_COEFFS] = [0; WIENER_NS_LUMA_COEFFS];

fn padded_from<T: ReconSample>(
    height: usize,
    stride: usize,
    source_at: impl Fn(isize, isize) -> T,
) -> Vec<T> {
    let radius = WIENER_NS_LUMA_TAP_RADIUS;
    let source_at = &source_at;
    (0..height + 2 * radius)
        .flat_map(move |row| {
            (0..stride).map(move |col| {
                source_at(
                    col as isize - radius as isize,
                    row as isize - radius as isize,
                )
            })
        })
        .collect()
}

/// Covers every lane group and tail of the `u16` SIMD segment kernel
/// against the per-sample callback reference, over randomized samples,
/// coefficients, and subclass runs.
#[test]
fn padded_u16_lane_groups_match_the_callback_reference() {
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for &bit_depth in &[BitDepth::Eight, BitDepth::Ten] {
        let max = u64::from(bit_depth.max_sample()) + 1;
        for &(width, height) in &[
            (1, 1),
            (3, 2),
            (4, 4),
            (7, 5),
            (16, 3),
            (31, 2),
            (32, 2),
            (37, 3),
            (64, 2),
            (67, 2),
            (100, 2),
            (128, 2),
        ] {
            let coeffs: Vec<[i16; WIENER_NS_LUMA_COEFFS]> = (0..3)
                .map(|_| core::array::from_fn(|_| (next() % 65) as i16 - 32))
                .collect();
            let subclasses: Vec<usize> = (0..width * height)
                .map(|_| (next() % coeffs.len() as u64) as usize)
                .collect();
            let stride = width + 2 * WIENER_NS_LUMA_TAP_RADIUS + (next() % 3) as usize;
            let padded: Vec<u16> = (0..(height + 2 * WIENER_NS_LUMA_TAP_RADIUS) * stride)
                .map(|index| match index % 4 {
                    0 => bit_depth.max_sample(),
                    1 => 0,
                    _ => (next() % max) as u16,
                })
                .collect();
            let source_at = |x: isize, y: isize| -> u16 {
                let radius = WIENER_NS_LUMA_TAP_RADIUS as isize;
                padded[((y + radius) as usize) * stride + (x + radius) as usize]
            };
            for map in [None, Some(subclasses.as_slice())] {
                let params = params(width, height, width, bit_depth, &coeffs, map);
                let mut reference = vec![0u16; width * height];
                wiener_ns_filter_luma_block(&mut reference, &params, source_at).unwrap();
                let mut actual = vec![0u16; width * height];
                let source = WienerNsLumaPaddedSource::new(&padded, stride, width, height).unwrap();
                wiener_ns_filter_luma_block_padded(&mut actual, &params, &source).unwrap();
                assert_eq!(actual, reference, "{bit_depth:?} {width}x{height}");
            }
        }
    }
}

fn packed_and_strided_u16_luma_match<T: ReconSample>(
    bit_depth: BitDepth,
    width: usize,
    height: usize,
    class_count: usize,
) {
    let mins = [
        -24, -24, -14, -14, -16, -16, -8, -8, -8, -8, -8, -8, -8, -8, -8, -8,
    ];
    let maxs = [39, 39, 17, 17, 15, 15, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7];
    let coeffs: Vec<_> = (0..class_count)
        .map(|class| if class % 2 == 0 { mins } else { maxs })
        .collect();
    let source_stride = width + 2 * WIENER_NS_LUMA_TAP_RADIUS + 3;
    let source_rows = height + 2 * WIENER_NS_LUMA_TAP_RADIUS;
    let source_samples: Vec<T> = (0..source_stride * source_rows)
        .map(|index| {
            T::try_from_u16(
                ((index * 37 + index / source_stride * 53 + 17)
                    % (usize::from(bit_depth.max_sample()) + 1)) as u16,
            )
            .unwrap()
        })
        .collect();
    let source =
        WienerNsLumaPaddedSource::new(&source_samples, source_stride, width, height).unwrap();
    let cells: Vec<u8> = (0..width.div_ceil(4) * height.div_ceil(4))
        .map(|index| (index % class_count) as u8)
        .collect();

    let packed_params = params(width, height, width, bit_depth, &coeffs, None);
    let mut packed = vec![T::default(); width * height];
    let mut packed_scratch = WienerNsLumaScratch::default();
    if class_count == 1 {
        wiener_ns_filter_luma_block_padded_into(
            &mut packed,
            &packed_params,
            &source,
            &mut packed_scratch,
        )
        .unwrap();
    } else {
        wiener_ns_filter_luma_block_padded_cells_into(
            &mut packed,
            &packed_params,
            &source,
            &cells,
            &mut packed_scratch,
        )
        .unwrap();
    }

    let shifted: Vec<Vec<T>> = source_samples
        .chunks(source_stride)
        .map(|row| {
            [T::default(); 2]
                .into_iter()
                .chain(row.iter().copied())
                .collect()
        })
        .collect();
    let table: Vec<&[T]> = shifted.iter().map(Vec::as_slice).collect();
    let table_source = WienerNsLumaPaddedSource::from_rows(&table, 2, width, height).unwrap();
    let mut from_table = vec![T::default(); width * height];
    wiener_ns_filter_luma_block_padded_cells_into(
        &mut from_table,
        &packed_params,
        &table_source,
        &cells,
        &mut WienerNsLumaScratch::default(),
    )
    .unwrap();
    assert!(
        from_table
            .iter()
            .map(|sample| sample.to_u16())
            .eq(packed.iter().map(|sample| sample.to_u16()))
    );
    assert!(WienerNsLumaPaddedSource::from_rows(&table[1..], 2, width, height).is_err());

    let output_stride = width + 11;
    let direct_params = params(width, height, output_stride, bit_depth, &coeffs, None);
    let mut direct = vec![u16::MAX; output_stride * height];
    let mut direct_scratch = WienerNsLumaScratch::default();
    if class_count == 1 {
        wiener_ns_filter_luma_block_padded_u16_into(
            &mut direct,
            &direct_params,
            &source,
            &mut direct_scratch,
        )
        .unwrap();
    } else {
        wiener_ns_filter_luma_block_padded_cells_u16_into(
            &mut direct,
            &direct_params,
            &source,
            &cells,
            &mut direct_scratch,
        )
        .unwrap();
    }

    for row in 0..height {
        assert_eq!(
            &direct[row * output_stride..row * output_stride + width],
            &packed[row * width..(row + 1) * width]
                .iter()
                .map(|sample| sample.to_u16())
                .collect::<Vec<_>>(),
            "{} {bit_depth:?} {width}x{height} classes={class_count}",
            T::TYPE_NAME,
        );
        assert!(
            direct[row * output_stride + width..(row + 1) * output_stride]
                .iter()
                .all(|&sample| sample == u16::MAX)
        );
    }
}

#[test]
fn packed_and_strided_u16_luma_match_runtime_class_counts_and_shapes() {
    let class_counts = [1, 2, 3, 4, 6, 8, 12, 16];
    let shapes = [
        (1, 1),
        (3, 3),
        (31, 8),
        (32, 56),
        (37, 64),
        (64, 8),
        (67, 56),
        (128, 64),
        (255, 3),
        (256, 56),
    ];
    for class_count in class_counts {
        for &(width, height) in &shapes {
            packed_and_strided_u16_luma_match::<u8>(BitDepth::Eight, width, height, class_count);
            packed_and_strided_u16_luma_match::<u16>(BitDepth::Eight, width, height, class_count);
            packed_and_strided_u16_luma_match::<u16>(BitDepth::Ten, width, height, class_count);
        }
    }
}

#[test]
fn direct_u8_and_u16_staging_match_runtime_class_counts_and_shapes() {
    for class_count in [1, 2, 4, 8, 16] {
        for (width, height) in [(1, 1), (3, 3), (31, 8), (37, 63), (128, 56)] {
            let coeffs: Vec<_> = (0..class_count)
                .map(|class| {
                    core::array::from_fn(|index| ((class * 11 + index * 7) % 33) as i16 - 16)
                })
                .collect();
            let source_stride = width + 2 * WIENER_NS_LUMA_TAP_RADIUS + 3;
            let source_rows = height + 2 * WIENER_NS_LUMA_TAP_RADIUS;
            let source_samples: Vec<u8> = (0..source_stride * source_rows)
                .map(|index| ((index * 37 + index / source_stride * 53 + 17) % 256) as u8)
                .collect();
            let source =
                WienerNsLumaPaddedSource::new(&source_samples, source_stride, width, height)
                    .unwrap();
            let source_samples_u16: Vec<u16> = source_samples
                .iter()
                .map(|&sample| u16::from(sample))
                .collect();
            let source_u16 =
                WienerNsLumaPaddedSource::new(&source_samples_u16, source_stride, width, height)
                    .unwrap();
            let cells: Vec<u8> = (0..width.div_ceil(4) * height.div_ceil(4))
                .map(|index| (index % class_count) as u8)
                .collect();
            let output_stride = width + 5;
            let params = params(width, height, output_stride, BitDepth::Eight, &coeffs, None);
            let mut staged = vec![u16::MAX; output_stride * height];
            let mut direct = vec![0xa5; output_stride * height];
            let mut direct_from_u16 = vec![0xa5; output_stride * height];
            if class_count == 1 {
                wiener_ns_filter_luma_block_padded_u16_into(
                    &mut staged,
                    &params,
                    &source,
                    &mut WienerNsLumaScratch::default(),
                )
                .unwrap();
                wiener_ns_filter_luma_block_padded_u8_into(
                    &mut direct,
                    &params,
                    &source,
                    &mut WienerNsLumaScratch::default(),
                )
                .unwrap();
                wiener_ns_filter_luma_block_padded_u8_into(
                    &mut direct_from_u16,
                    &params,
                    &source_u16,
                    &mut WienerNsLumaScratch::default(),
                )
                .unwrap();
            } else {
                wiener_ns_filter_luma_block_padded_cells_u16_into(
                    &mut staged,
                    &params,
                    &source,
                    &cells,
                    &mut WienerNsLumaScratch::default(),
                )
                .unwrap();
                wiener_ns_filter_luma_block_padded_cells_u8_into(
                    &mut direct,
                    &params,
                    &source,
                    &cells,
                    &mut WienerNsLumaScratch::default(),
                )
                .unwrap();
                wiener_ns_filter_luma_block_padded_cells_u8_into(
                    &mut direct_from_u16,
                    &params,
                    &source_u16,
                    &cells,
                    &mut WienerNsLumaScratch::default(),
                )
                .unwrap();
            }
            for row in 0..height {
                assert_eq!(
                    &direct[row * output_stride..row * output_stride + width],
                    &staged[row * output_stride..row * output_stride + width]
                        .iter()
                        .map(|&sample| sample as u8)
                        .collect::<Vec<_>>(),
                    "{width}x{height} classes={class_count}",
                );
                assert_eq!(
                    &direct_from_u16[row * output_stride..row * output_stride + width],
                    &direct[row * output_stride..row * output_stride + width],
                    "u16 source {width}x{height} classes={class_count}",
                );
                assert!(
                    direct[row * output_stride + width..(row + 1) * output_stride]
                        .iter()
                        .all(|&sample| sample == 0xa5)
                );
                assert!(
                    direct_from_u16[row * output_stride + width..(row + 1) * output_stride]
                        .iter()
                        .all(|&sample| sample == 0xa5)
                );
            }
        }
    }
}

#[test]
fn u8_source_strided_u16_simd_has_a_mutation_sensitive_oracle() {
    let width = 32;
    let height = 1;
    let source_stride = width + 2 * WIENER_NS_LUMA_TAP_RADIUS;
    let source_rows = height + 2 * WIENER_NS_LUMA_TAP_RADIUS;
    let mut samples = vec![100u8; source_stride * source_rows];
    let center_row = WIENER_NS_LUMA_TAP_RADIUS;
    samples[(center_row + 1) * source_stride + WIENER_NS_LUMA_TAP_RADIUS] = 120;
    let source = WienerNsLumaPaddedSource::new(&samples, source_stride, width, height).unwrap();
    let mut class = ZERO;
    class[0] = 4;
    let coeffs = [class];
    let output_stride = width + 7;
    let params = params(width, height, output_stride, BitDepth::Eight, &coeffs, None);
    let mut output = vec![u16::MAX; output_stride];

    wiener_ns_filter_luma_block_padded_u16_into(
        &mut output,
        &params,
        &source,
        &mut WienerNsLumaScratch::default(),
    )
    .unwrap();

    assert_eq!(output[0], 101);
    assert!(output[1..width].iter().all(|&sample| sample == 100));
    assert!(output[width..].iter().all(|&sample| sample == u16::MAX));

    let mut direct = vec![0xa5; output_stride];
    wiener_ns_filter_luma_block_padded_u8_into(
        &mut direct,
        &params,
        &source,
        &mut WienerNsLumaScratch::default(),
    )
    .unwrap();
    assert_eq!(direct[0], 101);
    assert!(direct[1..width].iter().all(|&sample| sample == 100));
    assert!(direct[width..].iter().all(|&sample| sample == 0xa5));
}

#[test]
fn strided_u16_luma_validates_every_input_before_writing() {
    let width = 2;
    let height = 2;
    let source_stride = width + 2 * WIENER_NS_LUMA_TAP_RADIUS;
    let source_rows = height + 2 * WIENER_NS_LUMA_TAP_RADIUS;
    let samples = vec![0u8; source_stride * source_rows];
    let valid_source =
        WienerNsLumaPaddedSource::new(&samples, source_stride, width, height).unwrap();
    let short_source = WienerNsLumaPaddedSource {
        rows: PaddedRows::Strided {
            samples: &samples[..samples.len() - 1],
            stride: source_stride,
        },
    };
    let coeffs = [ZERO];
    let short_cells: [u8; 0] = [];

    let mut output = [77u16; 8];
    let invalid_stride_params = params(width, height, 1, BitDepth::Eight, &coeffs, None);
    let error = wiener_ns_filter_luma_block_padded_cells_u16_into(
        &mut output,
        &invalid_stride_params,
        &short_source,
        &short_cells,
        &mut WienerNsLumaScratch::default(),
    )
    .unwrap_err();
    assert_eq!(
        error,
        ReconError::WienerNsFilterOutputStrideTooSmall {
            stride_samples: 1,
            width,
        }
    );
    assert_eq!(output, [77; 8]);

    let params = params(width, height, 4, BitDepth::Eight, &coeffs, None);
    let mut output = [77u16; 5];
    let error = wiener_ns_filter_luma_block_padded_cells_u16_into(
        &mut output,
        &params,
        &short_source,
        &short_cells,
        &mut WienerNsLumaScratch::default(),
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

    let mut output = [77u16; 8];
    let error = wiener_ns_filter_luma_block_padded_cells_u16_into(
        &mut output,
        &params,
        &valid_source,
        &short_cells,
        &mut WienerNsLumaScratch::default(),
    )
    .unwrap_err();
    assert_eq!(
        error,
        ReconError::WienerNsFilterSubclassMapTooShort {
            expected: 1,
            actual: 0,
        }
    );
    assert_eq!(output, [77; 8]);

    let cells = [0u8];
    let error = wiener_ns_filter_luma_block_padded_cells_u16_into(
        &mut output,
        &params,
        &short_source,
        &cells,
        &mut WienerNsLumaScratch::default(),
    )
    .unwrap_err();
    assert_eq!(
        error,
        ReconError::WienerNsFilterOutputTooSmall {
            expected: source_stride * source_rows,
            actual: samples.len() - 1,
        }
    );
    assert_eq!(output, [77; 8]);
}

#[test]
fn strided_luma_u16_source_errors_are_fail_atomic() {
    let width = 2;
    let height = 2;
    let source_stride = width + 2 * WIENER_NS_LUMA_TAP_RADIUS;
    let source_rows = height + 2 * WIENER_NS_LUMA_TAP_RADIUS;
    let mut samples = vec![23u16; source_stride * source_rows];
    let invalid = (source_rows - 1) * source_stride + WIENER_NS_LUMA_TAP_RADIUS;
    samples[invalid] = 1024;
    let source = WienerNsLumaPaddedSource::new(&samples, source_stride, width, height).unwrap();
    let coeffs = [ZERO];
    let ten_bit_params = params(width, height, 4, BitDepth::Ten, &coeffs, None);
    let mut output = [77u16; 8];

    let error = wiener_ns_filter_luma_block_padded_u16_into(
        &mut output,
        &ten_bit_params,
        &source,
        &mut WienerNsLumaScratch::default(),
    )
    .unwrap_err();

    assert_eq!(
        error,
        ReconError::WienerNsFilterSourceSampleOutOfRange {
            x: 0,
            y: 5,
            value: 1024,
            max: 1023,
        }
    );
    assert_eq!(output, [77; 8]);

    let mut direct = [0xa5u8; 8];
    let error = wiener_ns_filter_luma_block_padded_u8_into(
        &mut direct,
        &ten_bit_params,
        &source,
        &mut WienerNsLumaScratch::default(),
    )
    .unwrap_err();
    assert_eq!(
        error,
        ReconError::SampleTypeUnsupportedBitDepth {
            sample_type: "u8",
            bit_depth: BitDepth::Ten,
        }
    );
    assert_eq!(direct, [0xa5; 8]);

    let mut eight_bit_samples = vec![23u16; source_stride * source_rows];
    let center = WIENER_NS_LUMA_TAP_RADIUS * source_stride + WIENER_NS_LUMA_TAP_RADIUS;
    eight_bit_samples[center] = 256;
    let eight_bit_source =
        WienerNsLumaPaddedSource::new(&eight_bit_samples, source_stride, width, height).unwrap();
    let eight_bit_params = params(width, height, 4, BitDepth::Eight, &coeffs, None);
    let error = wiener_ns_filter_luma_block_padded_u8_into(
        &mut direct,
        &eight_bit_params,
        &eight_bit_source,
        &mut WienerNsLumaScratch::default(),
    )
    .unwrap_err();
    assert_eq!(
        error,
        ReconError::WienerNsFilterSourceSampleOutOfRange {
            x: 0,
            y: 0,
            value: 256,
            max: 255,
        }
    );
    assert_eq!(direct, [0xa5; 8]);
}
#[test]
fn padded_and_callback_filters_match_bit_exactly() {
    let width = 7;
    let height = 5;
    let radius = WIENER_NS_LUMA_TAP_RADIUS;
    let stride = width + 2 * radius;
    let mut class_a = ZERO;
    class_a[0] = 4;
    class_a[7] = -3;
    class_a[15] = 9;
    let mut class_b = ZERO;
    class_b[3] = -6;
    class_b[10] = 2;
    let coeffs = [class_a, class_b];
    let subclasses: Vec<usize> = (0..width * height).map(|i| i % 2).collect();
    let source_at =
        |x: isize, y: isize| -> u16 { ((x * 31 + y * 17 + 512).rem_euclid(1024)) as u16 };
    let padded: Vec<u16> = padded_from(height, stride, source_at);
    let params = params(
        width,
        height,
        width + 3,
        BitDepth::Ten,
        &coeffs,
        Some(&subclasses),
    );

    let mut callback_output = vec![0u16; (height - 1) * (width + 3) + width];
    wiener_ns_filter_luma_block(&mut callback_output, &params, source_at).unwrap();

    let mut padded_output = vec![0u16; (height - 1) * (width + 3) + width];
    let source = WienerNsLumaPaddedSource::new(&padded, stride, width, height).unwrap();
    wiener_ns_filter_luma_block_padded(&mut padded_output, &params, &source).unwrap();

    assert_eq!(callback_output, padded_output);
}

#[test]
fn padded_and_callback_filters_match_bit_exactly_eight_bit_wide_block() {
    let width = 23;
    let height = 6;
    let radius = WIENER_NS_LUMA_TAP_RADIUS;
    let stride = width + 2 * radius + 2;
    let mut class_a = ZERO;
    class_a[1] = 7;
    class_a[12] = -5;
    let mut class_b = ZERO;
    class_b[6] = 11;
    class_b[14] = -2;
    let coeffs = [class_a, class_b];
    let subclasses: Vec<usize> = (0..width * height)
        .map(|i| ((i % width) / 4 + i / width / 4) % 2)
        .collect();
    let source_at = |x: isize, y: isize| -> u8 { ((x * 7 + y * 13 + 90).rem_euclid(256)) as u8 };
    let padded: Vec<u8> = padded_from(height, stride, source_at);
    let params = params(
        width,
        height,
        width,
        BitDepth::Eight,
        &coeffs,
        Some(&subclasses),
    );

    let mut callback_output = vec![0u8; width * height];
    wiener_ns_filter_luma_block(&mut callback_output, &params, source_at).unwrap();

    let mut padded_output = vec![0u8; width * height];
    let source = WienerNsLumaPaddedSource::new(&padded, stride, width, height).unwrap();
    wiener_ns_filter_luma_block_padded(&mut padded_output, &params, &source).unwrap();

    assert_eq!(callback_output, padded_output);
}

#[test]
fn cell_subclasses_match_per_sample_expansion() {
    let width = 10;
    let height = 6;
    let radius = WIENER_NS_LUMA_TAP_RADIUS;
    let stride = width + 2 * radius;
    let mut class_a = ZERO;
    class_a[0] = 7;
    class_a[8] = -3;
    let mut class_b = ZERO;
    class_b[4] = -5;
    class_b[15] = 9;
    let coeffs = [class_a, class_b];
    let cell_subclasses = [0u8, 0, 1, 1, 1, 0];
    let cell_cols = width.div_ceil(4);
    let subclasses: Vec<usize> = (0..height)
        .flat_map(|row| {
            (0..width).map(move |col| usize::from(cell_subclasses[(row / 4) * cell_cols + col / 4]))
        })
        .collect();
    let source_at = |x: isize, y: isize| -> u8 { ((x * 11 + y * 19 + 120).rem_euclid(256)) as u8 };
    let padded: Vec<u8> = padded_from(height, stride, source_at);
    let source = WienerNsLumaPaddedSource::new(&padded, stride, width, height).unwrap();
    let per_sample = params(
        width,
        height,
        width,
        BitDepth::Eight,
        &coeffs,
        Some(&subclasses),
    );
    let per_cell = params(width, height, width, BitDepth::Eight, &coeffs, None);
    let mut expected = vec![0u8; width * height];
    let mut actual = vec![0u8; width * height];
    let mut expected_scratch = WienerNsLumaScratch::default();
    let mut actual_scratch = WienerNsLumaScratch::default();

    wiener_ns_filter_luma_block_padded_into(
        &mut expected,
        &per_sample,
        &source,
        &mut expected_scratch,
    )
    .unwrap();
    wiener_ns_filter_luma_block_padded_cells_into(
        &mut actual,
        &per_cell,
        &source,
        &cell_subclasses,
        &mut actual_scratch,
    )
    .unwrap();

    assert_eq!(actual, expected);
}

#[test]
fn out_of_range_sample_in_unread_window_corner_still_filters() {
    let width = 5;
    let height = 4;
    let radius = WIENER_NS_LUMA_TAP_RADIUS;
    let stride = width + 2 * radius;
    let source_at =
        |x: isize, y: isize| -> u16 { ((x * 19 + y * 5 + 300).rem_euclid(1024)) as u16 };
    let mut padded: Vec<u16> = padded_from(height, stride, source_at);
    padded[0] = u16::MAX;
    let mut class = ZERO;
    class[0] = 6;
    class[13] = -9;
    let coeffs = [class];
    let params = params(width, height, width, BitDepth::Ten, &coeffs, None);

    let mut expected = vec![0u16; width * height];
    wiener_ns_filter_luma_block(&mut expected, &params, source_at).unwrap();

    let mut padded_output = vec![0u16; width * height];
    let source = WienerNsLumaPaddedSource::new(&padded, stride, width, height).unwrap();
    wiener_ns_filter_luma_block_padded(&mut padded_output, &params, &source).unwrap();

    assert_eq!(expected, padded_output);
}

#[test]
fn padded_source_rejects_short_buffer() {
    let padded = vec![0u16; 10];
    let err = WienerNsLumaPaddedSource::new(&padded, 9, 1, 1).unwrap_err();
    assert_eq!(
        err,
        ReconError::WienerNsFilterOutputTooSmall {
            expected: 8 * 9 + 9,
            actual: 10,
        }
    );
}

#[test]
fn padded_source_rejects_narrow_stride() {
    let padded = vec![0u16; 100];
    let err = WienerNsLumaPaddedSource::new(&padded, 8, 1, 1).unwrap_err();
    assert_eq!(
        err,
        ReconError::WienerNsFilterOutputStrideTooSmall {
            stride_samples: 8,
            width: 9,
        }
    );
}

#[test]
fn padded_scratch_recovers_after_source_sample_error() {
    let width = 2;
    let height = 2;
    let radius = WIENER_NS_LUMA_TAP_RADIUS;
    let stride = width + 2 * radius;
    let mut padded = vec![23u16; stride * (height + 2 * radius)];
    let invalid_sample = (height + 2 * radius - 1) * stride + radius;
    padded[invalid_sample] = 1024;
    let coeffs = [ZERO];
    let params = params(width, height, width, BitDepth::Ten, &coeffs, None);
    let source = WienerNsLumaPaddedSource::new(&padded, stride, width, height).unwrap();
    let mut scratch = WienerNsLumaScratch::default();
    let mut output = [77u16; 4];

    let err = wiener_ns_filter_luma_block_padded_into(&mut output, &params, &source, &mut scratch)
        .unwrap_err();

    assert_eq!(
        err,
        ReconError::WienerNsFilterSourceSampleOutOfRange {
            x: 0,
            y: 5,
            value: 1024,
            max: 1023,
        }
    );
    assert_eq!(output, [77; 4]);

    padded[invalid_sample] = 23;
    let source = WienerNsLumaPaddedSource::new(&padded, stride, width, height).unwrap();
    wiener_ns_filter_luma_block_padded_into(&mut output, &params, &source, &mut scratch).unwrap();
    assert_eq!(output, [23; 4]);
}

#[test]
fn padded_into_reuses_maximum_restoration_stripe_scratch() {
    let width = 512;
    let height = 64;
    let stride = width + 2 * WIENER_NS_LUMA_TAP_RADIUS;
    let mut padded = vec![91u16; stride * (height + 2 * WIENER_NS_LUMA_TAP_RADIUS)];
    padded[0] = 1024;
    let source = WienerNsLumaPaddedSource::new(&padded, stride, width, height).unwrap();
    let coeffs = [ZERO];
    let large_params = params(width, height, width, BitDepth::Ten, &coeffs, None);
    let mut scratch = WienerNsLumaScratch::default();
    let mut output = vec![0u16; width * height];

    wiener_ns_filter_luma_block_padded_into(&mut output, &large_params, &source, &mut scratch)
        .unwrap();

    assert!(output.iter().all(|&sample| sample == 91));
    assert!(scratch.filtered.capacity() >= width * height);
    assert!(scratch.clean_rows.capacity() >= height + 2 * WIENER_NS_LUMA_TAP_RADIUS);
    assert!(scratch.prepared_classes.capacity() >= coeffs.len());
    let clean_rows_capacity = scratch.clean_rows.capacity();
    let filtered_ptr = scratch.filtered.as_ptr();

    let width = 2;
    let height = 2;
    let stride = width + 2 * WIENER_NS_LUMA_TAP_RADIUS;
    let padded = vec![37u16; stride * (height + 2 * WIENER_NS_LUMA_TAP_RADIUS)];
    let source = WienerNsLumaPaddedSource::new(&padded, stride, width, height).unwrap();
    let params = params(width, height, width, BitDepth::Ten, &coeffs, None);
    let mut output = [u16::MAX; 4];
    wiener_ns_filter_luma_block_padded_into(&mut output, &params, &source, &mut scratch).unwrap();

    assert_eq!(output, [37; 4]);
    assert_eq!(scratch.clean_rows.capacity(), clean_rows_capacity);
    assert_eq!(scratch.filtered.as_ptr(), filtered_ptr);
}

#[test]
fn padded_cells_partial_cell_rows_match_the_callback_reference() {
    let coeffs: Vec<[i16; WIENER_NS_LUMA_COEFFS]> = (0..3)
        .map(|class| core::array::from_fn(|index| ((class * 5 + index * 3) % 15) as i16 - 7))
        .collect();
    for (width, height) in [(37, 13), (64, 6), (9, 3)] {
        let stride = width + 2 * WIENER_NS_LUMA_TAP_RADIUS;
        let padded: Vec<u16> = padded_from(height, stride, |x, y| {
            ((x * 29 + y * 41 + x * y).rem_euclid(1024)) as u16
        });
        let cell_cols = width.div_ceil(4);
        let cells: Vec<u8> = (0..cell_cols * height.div_ceil(4))
            .map(|index| (index * 7 / 3 % coeffs.len()) as u8)
            .collect();
        let expanded: Vec<usize> = (0..width * height)
            .map(|index| usize::from(cells[(index / width / 4) * cell_cols + index % width / 4]))
            .collect();
        let radius = WIENER_NS_LUMA_TAP_RADIUS as isize;
        let source_at =
            |x: isize, y: isize| padded[((y + radius) as usize) * stride + (x + radius) as usize];
        let reference_params = params(
            width,
            height,
            width,
            BitDepth::Ten,
            &coeffs,
            Some(&expanded),
        );
        let mut reference = vec![0u16; width * height];
        wiener_ns_filter_luma_block(&mut reference, &reference_params, source_at).unwrap();
        let cell_params = params(width, height, width, BitDepth::Ten, &coeffs, None);
        let source = WienerNsLumaPaddedSource::new(&padded, stride, width, height).unwrap();
        let mut actual = vec![0u16; width * height];
        wiener_ns_filter_luma_block_padded_cells_into(
            &mut actual,
            &cell_params,
            &source,
            &cells,
            &mut WienerNsLumaScratch::default(),
        )
        .unwrap();
        assert_eq!(actual, reference, "{width}x{height}");
    }
}

#[test]
fn padded_center_scale_outside_i16_matches_the_callback_reference() {
    let (width, height) = (37, 2);
    let coeffs = [[2000; WIENER_NS_LUMA_COEFFS]];
    let stride = width + 2 * WIENER_NS_LUMA_TAP_RADIUS;
    let padded: Vec<u16> = (0..(height + 2 * WIENER_NS_LUMA_TAP_RADIUS) * stride)
        .map(|index| (index * 37 % 1024) as u16)
        .collect();
    let source_at = |x: isize, y: isize| {
        let radius = WIENER_NS_LUMA_TAP_RADIUS as isize;
        padded[((y + radius) as usize) * stride + (x + radius) as usize]
    };
    let params = params(width, height, width, BitDepth::Ten, &coeffs, None);
    let mut reference = vec![0u16; width * height];
    wiener_ns_filter_luma_block(&mut reference, &params, source_at).unwrap();
    let mut actual = vec![0u16; width * height];
    let source = WienerNsLumaPaddedSource::new(&padded, stride, width, height).unwrap();
    wiener_ns_filter_luma_block_padded(&mut actual, &params, &source).unwrap();
    assert_eq!(actual, reference);
}

#[test]
fn config_y_is_symmetric_pairs() {
    for (j, &(dy, dx, coeff_index)) in WIENER_NS_CONFIG_Y_PAIRS.iter().enumerate() {
        assert_eq!(WIENER_NS_CONFIG_Y[2 * j], (dy, dx, coeff_index));
        assert_eq!(WIENER_NS_CONFIG_Y[2 * j + 1], (-dy, -dx, coeff_index));
    }
}

#[test]
fn luma_tap_radius_covers_config_table() {
    let max_offset = WIENER_NS_CONFIG_Y
        .iter()
        .map(|&(dy, dx, _)| dy.unsigned_abs().max(dx.unsigned_abs()))
        .max()
        .unwrap();
    assert_eq!(WIENER_NS_LUMA_TAP_RADIUS, max_offset);
}
fn params<'a>(
    width: usize,
    height: usize,
    output_stride: usize,
    bit_depth: BitDepth,
    coeffs_by_class: &'a [[i16; WIENER_NS_LUMA_COEFFS]],
    subclasses: Option<&'a [usize]>,
) -> WienerNsLumaFilter<'a> {
    WienerNsLumaFilter {
        width,
        height,
        output_stride,
        bit_depth,
        coeffs_by_class,
        subclasses,
    }
}

#[test]
fn zero_coefficients_copy_source_samples() {
    let coeffs = [ZERO];
    let mut output = [9u8; 6];
    let params = params(2, 2, 3, BitDepth::Eight, &coeffs, None);

    wiener_ns_filter_luma_block(&mut output, &params, |x, y| {
        if (0..=1).contains(&x) && (0..=1).contains(&y) {
            u8::try_from(20 + y * 10 + x).unwrap()
        } else {
            0
        }
    })
    .unwrap();

    assert_eq!(output, [20, 21, 9, 30, 31, 9]);
}

#[test]
fn hand_computed_luma_tap_accumulation() {
    let mut class = ZERO;
    class[0] = 4;
    let coeffs = [class];
    let mut output = [0u8; 1];
    let params = params(1, 1, 1, BitDepth::Eight, &coeffs, None);

    wiener_ns_filter_luma_block(&mut output, &params, |x, y| {
        if x == 0 && y == 1 { 120 } else { 100 }
    })
    .unwrap();

    assert_eq!(output, [101]);
}

#[test]
fn subclass_selects_coefficients_per_sample() {
    let mut class_one = ZERO;
    class_one[0] = 4;
    let coeffs = [ZERO, class_one];
    let subclasses = [0, 1];
    let mut output = [0u8; 2];
    let params = params(2, 1, 2, BitDepth::Eight, &coeffs, Some(&subclasses));

    wiener_ns_filter_luma_block(&mut output, &params, |x, y| {
        if y == 1 {
            120
        } else {
            u8::try_from(100 + x).unwrap()
        }
    })
    .unwrap();

    assert_eq!(output, [100, 102]);
}

#[test]
fn clip1_clamps_eight_bit_output() {
    let mut class = ZERO;
    class[0] = 512;
    let coeffs = [class];
    let mut output = [0u8; 1];
    let params = params(1, 1, 1, BitDepth::Eight, &coeffs, None);

    wiener_ns_filter_luma_block(&mut output, &params, |x, y| {
        if x == 0 && y == 1 { 255 } else { 250 }
    })
    .unwrap();

    assert_eq!(output, [255]);
}

#[test]
fn ten_bit_u16_output_is_supported_and_clamped() {
    let mut class = ZERO;
    class[0] = 512;
    let coeffs = [class];
    let mut output = [0u16; 1];
    let params = params(1, 1, 1, BitDepth::Ten, &coeffs, None);

    wiener_ns_filter_luma_block(&mut output, &params, |x, y| {
        if x == 0 && y == 1 { 1023 } else { 1000 }
    })
    .unwrap();

    assert_eq!(output, [1023]);
}

#[test]
fn rejects_invalid_output_stride_fail_atomically() {
    let coeffs = [ZERO];
    let params = params(2, 2, 1, BitDepth::Eight, &coeffs, None);
    let mut output = [77u8; 4];

    let err = wiener_ns_filter_luma_block(&mut output, &params, |_x, _y| 0).unwrap_err();

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
    let coeffs = [ZERO];
    let params = params(2, 2, 3, BitDepth::Eight, &coeffs, None);
    let mut output = [77u8; 4];

    let err = wiener_ns_filter_luma_block(&mut output, &params, |_x, _y| 0).unwrap_err();

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
fn rejects_missing_coefficient_classes_fail_atomically() {
    let coeffs: [[i16; WIENER_NS_LUMA_COEFFS]; 0] = [];
    let params = params(1, 1, 1, BitDepth::Eight, &coeffs, None);
    let mut output = [77u8; 1];

    let err = wiener_ns_filter_luma_block(&mut output, &params, |_x, _y| 0).unwrap_err();

    assert_eq!(err, ReconError::WienerNsFilterMissingClasses);
    assert_eq!(output, [77]);
}

#[test]
fn rejects_short_subclass_map_fail_atomically() {
    let coeffs = [ZERO];
    let subclasses = [0usize];
    let params = params(2, 1, 2, BitDepth::Eight, &coeffs, Some(&subclasses));
    let mut output = [77u8; 2];

    let err = wiener_ns_filter_luma_block(&mut output, &params, |_x, _y| 0).unwrap_err();

    assert_eq!(
        err,
        ReconError::WienerNsFilterSubclassMapTooShort {
            expected: 2,
            actual: 1,
        }
    );
    assert_eq!(output, [77; 2]);
}

#[test]
fn rejects_out_of_range_subclass_fail_atomically() {
    let coeffs = [ZERO];
    let subclasses = [0usize, 1];
    let params = params(2, 1, 2, BitDepth::Eight, &coeffs, Some(&subclasses));
    let mut output = [77u8; 2];

    let err = wiener_ns_filter_luma_block(&mut output, &params, |_x, _y| 0).unwrap_err();

    assert_eq!(
        err,
        ReconError::WienerNsFilterSubclassOutOfRange {
            sample_index: 1,
            subclass: 1,
            classes: 1,
        }
    );
    assert_eq!(output, [77; 2]);
}

#[test]
fn rejects_source_sample_out_of_range_fail_atomically() {
    let coeffs = [ZERO];
    let params = params(1, 1, 1, BitDepth::Ten, &coeffs, None);
    let mut output = [77u16; 1];

    let err = wiener_ns_filter_luma_block(&mut output, &params, |_x, _y| 1024u16).unwrap_err();

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
fn rejects_u8_storage_for_ten_bit() {
    let coeffs = [ZERO];
    let params = params(1, 1, 1, BitDepth::Ten, &coeffs, None);
    let mut output = [77u8; 1];

    let err = wiener_ns_filter_luma_block(&mut output, &params, |_x, _y| 0).unwrap_err();

    assert_eq!(
        err,
        ReconError::SampleTypeUnsupportedBitDepth {
            sample_type: "u8",
            bit_depth: BitDepth::Ten,
        }
    );
    assert_eq!(output, [77]);
}

/// The 8-bit byte-lane kernel matches the per-sample callback reference for
/// random coefficients across the § 5.20.2.1 tap ranges, including classes
/// past its exactness bounds that fall back, random cell maps of up to 16
/// classes, extreme samples, and both output types.
#[test]
fn eight_bit_byte_lanes_match_the_callback_reference() {
    const MIN: [i16; WIENER_NS_LUMA_COEFFS] = [
        -24, -24, -14, -14, -16, -16, -8, -8, -8, -8, -8, -8, -8, -8, -8, -8,
    ];
    const K: [u32; WIENER_NS_LUMA_COEFFS] = [6, 6, 5, 5, 5, 5, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4];
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let mut next = move |limit: u64| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state % limit
    };
    for case in 0..40 {
        let classes = 1 + next(16) as usize;
        let coeffs: Vec<[i16; WIENER_NS_LUMA_COEFFS]> = (0..classes)
            .map(|_| {
                let extreme = next(4) == 0;
                core::array::from_fn(|j| {
                    let span = 1u64 << K[j];
                    let value = if extreme { span - 1 } else { next(span) };
                    MIN[j] + value as i16
                })
            })
            .collect();
        let width = [32, 64, 96, 100, 128, 256, 47][case % 7];
        let height = 1 + next(16) as usize;
        let stride = width + 2 * WIENER_NS_LUMA_TAP_RADIUS + next(3) as usize;
        let style = next(3);
        let padded: Vec<u16> = padded_from(height, stride, |x, y| match style {
            0 => (x * 7 + y * 13).rem_euclid(256) as u16,
            1 => [0, 255][((x ^ y) & 1) as usize],
            _ => ((x * x + 3 * y) * 11).rem_euclid(256) as u16,
        });
        let cell_cols = width.div_ceil(4);
        let cells: Vec<u8> = (0..cell_cols * height.div_ceil(4))
            .map(|index| {
                if next(3) == 0 {
                    0
                } else {
                    (index * 5 / 3 % classes) as u8
                }
            })
            .collect();
        let expanded: Vec<usize> = (0..width * height)
            .map(|index| usize::from(cells[(index / width / 4) * cell_cols + index % width / 4]))
            .collect();
        let radius = WIENER_NS_LUMA_TAP_RADIUS as isize;
        let source_at =
            |x: isize, y: isize| padded[((y + radius) as usize) * stride + (x + radius) as usize];
        let source = WienerNsLumaPaddedSource::new(&padded, stride, width, height).unwrap();
        let mut scratch = WienerNsLumaScratch::default();
        for layout in 0..2 {
            let subclasses = (layout == 1).then_some(expanded.as_slice());
            let used = if layout == 1 {
                &coeffs[..]
            } else {
                &coeffs[..1]
            };
            let mut reference = vec![0u16; width * height];
            let reference_params = params(width, height, width, BitDepth::Eight, used, subclasses);
            wiener_ns_filter_luma_block(&mut reference, &reference_params, source_at).unwrap();
            let block = params(width, height, width, BitDepth::Eight, used, None);
            let mut wide = vec![0u16; width * height];
            let mut narrow = vec![0u8; width * height];
            if layout == 1 {
                wiener_ns_filter_luma_block_padded_cells_into(
                    &mut wide,
                    &block,
                    &source,
                    &cells,
                    &mut scratch,
                )
                .unwrap();
                wiener_ns_filter_luma_block_padded_cells_u8_into(
                    &mut narrow,
                    &block,
                    &source,
                    &cells,
                    &mut scratch,
                )
                .unwrap();
            } else {
                wiener_ns_filter_luma_block_padded_into(&mut wide, &block, &source, &mut scratch)
                    .unwrap();
                wiener_ns_filter_luma_block_padded_u8_into(
                    &mut narrow,
                    &block,
                    &source,
                    &mut scratch,
                )
                .unwrap();
            }
            assert_eq!(wide, reference, "case {case} layout {layout} u16");
            let narrow: Vec<u16> = narrow.into_iter().map(u16::from).collect();
            assert_eq!(narrow, reference, "case {case} layout {layout} u8");
        }
    }
}

/// The byte-lane exactness bounds accept every class of the § 5.20.2.1
/// ranges whose taps stay near a low-pass filter, and reject the extremes.
#[test]
fn eight_bit_unit_weights_bound_the_extreme_classes() {
    let low_pass = [20, 18, 4, 3, -2, -1, 1, 0, 0, 1, -1, 0, 0, 0, 0, 0];
    assert!(eight_bit::unit_weights(&low_pass).is_some());
    let extreme = [39, 39, 17, 17, 15, 15, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7];
    assert!(eight_bit::unit_weights(&extreme).is_none());
}

// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

#[test]
fn classify_read_radius_covers_lead_lag_neighborhood() {
    assert_eq!(
        PC_WIENER_CLASSIFY_READ_RADIUS,
        (PC_WIENER_LAG + 1).unsigned_abs()
    );
    assert!(PC_WIENER_CLASSIFY_READ_RADIUS >= (PC_WIENER_LEAD + 1).unsigned_abs());
}

fn params(bit_depth: BitDepth) -> PcWienerClassifyParams {
    PcWienerClassifyParams {
        x: 4,
        y: 4,
        bit_depth,
        base_q_idx: 0,
        block_start_x: 0,
        block_end_x: 63,
        luma_stripe_start_y: 0,
        luma_stripe_end_y: 63,
        tile_start_y: 0,
        tile_end_y: 63,
    }
}

#[test]
fn flat_source_without_skips_classifies_to_lut_zero() {
    let result =
        pc_wiener_classify::<u8, _, _>(&params(BitDepth::Eight), |_, _| Ok(12), |_| Ok(0)).unwrap();

    assert_eq!(result.raw_features, [0, 0, 0, 0]);
    assert_eq!(result.features, [0, 0, 0, 0]);
    assert_eq!(result.raw_tx_skip_sum, 0);
    assert_eq!(result.tx_skip, 0);
    assert_eq!(result.lut_input, 0);
    assert_eq!(result.class, 83);
}

#[test]
fn quadratic_ten_bit_source_accumulates_features_and_tx_skip() {
    let result = pc_wiener_classify::<u16, _, _>(
        &params(BitDepth::Ten),
        |x, y| {
            let x = i32::try_from(x).unwrap();
            let y = i32::try_from(y).unwrap();
            Ok(u16::try_from(100 + x * x + 2 * y * y + 3 * x * y).unwrap())
        },
        |_| Ok(1),
    )
    .unwrap();

    assert_eq!(result.raw_features, [0, 144, 0, 432]);
    assert_eq!(result.features, [0, 134_604, 0, 331_992]);
    assert_eq!(result.raw_tx_skip_sum, 36);
    assert_eq!(result.tx_skip, 252);
    assert_eq!(result.lut_input, 128);
    assert_eq!(result.class, 243);
}

#[test]
fn maximum_ten_bit_feature_window_fits_i32_classification_state() {
    let result = pc_wiener_classify::<u16, _, _>(
        &params(BitDepth::Ten),
        |_, y| Ok(if y & 1 == 0 { 1023 } else { 0 }),
        |_| Ok(1),
    )
    .unwrap();

    assert_eq!(result.raw_features, [0, 73_656, 73_656, 73_656]);
    assert_eq!(result.features, [0, 68_849_946, 60_269_022, 56_604_636]);
    assert_eq!(result.raw_tx_skip_sum, 36);
    assert_eq!(result.tx_skip, 252);
}

#[test]
fn grid_classification_matches_scalar_cells_and_reuses_features() {
    let mut params = params(BitDepth::Eight);
    params.x = 52;
    params.block_end_x = 61;
    let cell_cols = 5;
    let cell_rows = 3;
    let source = |x: isize, y: isize| {
        let value = (3 * x + 5 * y + x * y).rem_euclid(200) + 20;
        Ok(u8::try_from(value).unwrap())
    };
    let tx_skip =
        |lookup: PcWienerTxSkipLookup| Ok(i32::try_from((lookup.row + lookup.col) & 1).unwrap());
    let mut grid_source_calls = 0;
    let grid = pc_wiener_classify_grid::<u8, _, _>(
        &params,
        cell_cols,
        cell_rows,
        |x, y| {
            grid_source_calls += 1;
            source(x, y)
        },
        tx_skip,
    )
    .unwrap();

    let mut scalar_source_calls = 0;
    let mut scalar = Vec::new();
    for row in 0..cell_rows {
        for col in 0..cell_cols {
            let mut cell = params;
            cell.x += isize::try_from(col * PC_WIENER_BLOCK_SIZE).unwrap();
            cell.y += isize::try_from(row * PC_WIENER_BLOCK_SIZE).unwrap();
            scalar.push(
                pc_wiener_classify::<u8, _, _>(
                    &cell,
                    |x, y| {
                        scalar_source_calls += 1;
                        source(x, y)
                    },
                    tx_skip,
                )
                .unwrap(),
            );
        }
    }

    assert_eq!(grid, scalar);
    assert_eq!(grid_source_calls, 240);
    assert!(grid_source_calls * 7 < scalar_source_calls);
}

#[test]
fn grid_classification_matches_scalar_cells_across_shapes_and_clamps() {
    let source = |x: isize, y: isize| {
        Ok(u8::try_from((7 * x - 11 * y + 3 * x * y).rem_euclid(233) + 11).unwrap())
    };
    let tx_skip =
        |lookup: PcWienerTxSkipLookup| Ok(i32::from((lookup.row * 3 + lookup.col * 5) % 3 == 1));
    for cell_cols in 1..=10usize {
        for cell_rows in 1..=4usize {
            for block_end_x in [2usize, 7, 27, 63, 200] {
                let mut params = params(BitDepth::Eight);
                params.x = 8;
                params.y = 8;
                params.base_q_idx = 96;
                params.block_end_x = block_end_x;
                let grid = pc_wiener_classify_grid::<u8, _, _>(
                    &params, cell_cols, cell_rows, source, tx_skip,
                )
                .unwrap();
                let mut scalar = Vec::new();
                for row in 0..cell_rows {
                    for col in 0..cell_cols {
                        let mut cell = params;
                        cell.x += isize::try_from(col * PC_WIENER_BLOCK_SIZE).unwrap();
                        cell.y += isize::try_from(row * PC_WIENER_BLOCK_SIZE).unwrap();
                        scalar
                            .push(pc_wiener_classify::<u8, _, _>(&cell, source, tx_skip).unwrap());
                    }
                }
                assert_eq!(grid, scalar, "{cell_cols}x{cell_rows} end {block_end_x}");
            }
        }
    }
}

#[test]
fn tx_skip_lookup_clips_to_block_stripe_and_tile_bounds() {
    let params = PcWienerClassifyParams {
        x: 70,
        y: -4,
        bit_depth: BitDepth::Eight,
        base_q_idx: 0,
        block_start_x: 64,
        block_end_x: 80,
        luma_stripe_start_y: 4,
        luma_stripe_end_y: 20,
        tile_start_y: 8,
        tile_end_y: 16,
    };
    let mut first = None;
    pc_wiener_classify::<u8, _, _>(
        &params,
        |_, _| Ok(0),
        |lookup| {
            first.get_or_insert(lookup);
            Ok(0)
        },
    )
    .unwrap();

    assert_eq!(
        first,
        Some(PcWienerTxSkipLookup {
            x: 69,
            y: 8,
            row: 2,
            col: 17,
        })
    );
}

#[test]
fn rejects_source_samples_outside_bit_depth() {
    let err = pc_wiener_classify::<u16, _, _>(&params(BitDepth::Eight), |_, _| Ok(256), |_| Ok(0))
        .unwrap_err();

    assert_eq!(
        err,
        ReconError::PcWienerSourceSampleOutOfRange {
            x: 3,
            y: 3,
            value: 256,
            max: 255,
        }
    );
}

#[test]
fn propagates_source_sample_errors() {
    let err = pc_wiener_classify::<u8, _, _>(
        &params(BitDepth::Eight),
        |_, _| {
            Err(ReconError::ArithmeticOverflow {
                context: "test source sample",
            })
        },
        |_| Ok(0),
    )
    .unwrap_err();

    assert_eq!(
        err,
        ReconError::ArithmeticOverflow {
            context: "test source sample",
        }
    );
}

#[test]
fn propagates_tx_skip_lookup_errors() {
    let err = pc_wiener_classify::<u8, _, _>(
        &params(BitDepth::Eight),
        |_, _| Ok(0),
        |_| {
            Err(ReconError::ArithmeticOverflow {
                context: "test tx-skip lookup",
            })
        },
    )
    .unwrap_err();

    assert_eq!(
        err,
        ReconError::ArithmeticOverflow {
            context: "test tx-skip lookup",
        }
    );
}

#[test]
fn rejects_non_boolean_tx_skip_values() {
    let err = pc_wiener_classify::<u8, _, _>(&params(BitDepth::Eight), |_, _| Ok(0), |_| Ok(2))
        .unwrap_err();

    assert_eq!(
        err,
        ReconError::PcWienerInvalidTxSkip {
            x: 3,
            y: 3,
            row: 0,
            col: 0,
            value: 2,
        }
    );
}

#[test]
fn rejects_u8_storage_for_ten_bit_classification() {
    let err = pc_wiener_classify::<u8, _, _>(&params(BitDepth::Ten), |_, _| Ok(0), |_| Ok(0))
        .unwrap_err();

    assert_eq!(
        err,
        ReconError::SampleTypeUnsupportedBitDepth {
            sample_type: "u8",
            bit_depth: BitDepth::Ten,
        }
    );
}

#[test]
fn rejects_inverted_bounds() {
    let mut params = params(BitDepth::Eight);
    params.block_start_x = 12;
    params.block_end_x = 10;
    let err = pc_wiener_classify::<u8, _, _>(&params, |_, _| Ok(0), |_| Ok(0)).unwrap_err();

    assert_eq!(
        err,
        ReconError::PcWienerInvalidBounds {
            field: "block x range",
        }
    );
}

#[test]
fn fixed_filter_matches_hand_computed_quadratic_sample() {
    let mut output = [0u16; 1];
    let params = PcWienerFilter {
        width: 1,
        height: 1,
        output_stride: 1,
        bit_depth: BitDepth::Ten,
        filter_set_index: 0,
        subclass_block_size: 1,
        subclasses: &[2],
    };
    pc_wiener_filter_block(&mut output, &params, |x, y| {
        u16::try_from(500 + 10 * x * x + 20 * y * y + 5 * x * y).map_err(|_| {
            ReconError::ArithmeticOverflow {
                context: "test PC-Wiener source",
            }
        })
    })
    .unwrap();

    assert_eq!(output, [499]);
}

#[test]
fn fixed_filter_rejects_out_of_range_subclass_without_writing() {
    let mut output = [7u8; 1];
    let params = PcWienerFilter {
        width: 1,
        height: 1,
        output_stride: 1,
        bit_depth: BitDepth::Eight,
        filter_set_index: 0,
        subclass_block_size: 1,
        subclasses: &[PC_WIENER_FULL_CLASSES],
    };
    let err = pc_wiener_filter_block(&mut output, &params, |_, _| Ok(0)).unwrap_err();

    assert_eq!(
        err,
        ReconError::PcWienerInvalidBounds {
            field: "PC-Wiener filter index",
        }
    );
    assert_eq!(output, [7]);
}

#[test]
fn padded_u16_lane_groups_match_the_callback_reference() {
    let radius = PC_WIENER_FILTER_TAP_RADIUS;
    for &bit_depth in &[BitDepth::Eight, BitDepth::Ten] {
        let max = i64::from(bit_depth.max_sample());
        for &width in &[4, 7, 8, 16, 31, 32, 64, 67, 100, 128, 191] {
            let height = 3;
            let source_at = |x: isize, y: isize| -> u16 {
                let index = x as i64 * 7 + y as i64 * 3;
                match index.rem_euclid(4) {
                    0 => max as u16,
                    1 => 0,
                    _ => (index * 29).rem_euclid(max + 1) as u16,
                }
            };
            for (filter_set_index, run) in [(0, 1), (1, 5), (2, 16), (3, width)] {
                let subclasses: Vec<usize> = (0..width * height)
                    .map(|index| (index / run) % 64)
                    .collect();
                let params = PcWienerFilter {
                    width,
                    height,
                    output_stride: width,
                    bit_depth,
                    filter_set_index,
                    subclass_block_size: 1,
                    subclasses: &subclasses,
                };
                let mut reference = vec![0u16; width * height];
                pc_wiener_filter_block(&mut reference, &params, |x, y| Ok(source_at(x, y)))
                    .unwrap();
                let stride = width + 2 * radius + 1;
                let padded: Vec<u16> = (0..(height + 2 * radius) * stride)
                    .map(|index| {
                        source_at(
                            (index % stride) as isize - radius as isize,
                            (index / stride) as isize - radius as isize,
                        )
                    })
                    .collect();
                let source = PcWienerPaddedSource::new(&padded, stride, width, height).unwrap();
                let mut actual = vec![0u16; width * height];
                pc_wiener_filter_block_padded(&mut actual, &params, &source).unwrap();
                assert_eq!(
                    actual, reference,
                    "{bit_depth:?} width {width} {filter_set_index}"
                );
            }
        }
    }
}

#[test]
fn padded_and_callback_filters_match_bit_exactly() {
    let width = 31;
    let height = 5;
    let radius = PC_WIENER_FILTER_TAP_RADIUS;
    let stride = width + 2 * radius + 1;
    let subclasses: Vec<usize> = (0..width * height).map(|i| (i / width) % 5).collect();
    let source_at =
        |x: isize, y: isize| -> u16 { ((x * 23 + y * 11 + 400).rem_euclid(1024)) as u16 };
    let params = PcWienerFilter {
        width,
        height,
        output_stride: width,
        bit_depth: BitDepth::Ten,
        filter_set_index: 1,
        subclass_block_size: 1,
        subclasses: &subclasses,
    };

    let mut callback_output = vec![0u16; width * height];
    pc_wiener_filter_block(&mut callback_output, &params, |x, y| Ok(source_at(x, y))).unwrap();

    let rows = height + 2 * radius;
    let mut padded = Vec::with_capacity(stride * rows);
    for row in 0..rows {
        for col in 0..stride {
            padded.push(source_at(
                col as isize - radius as isize,
                row as isize - radius as isize,
            ));
        }
    }
    let source = PcWienerPaddedSource::new(&padded, stride, width, height).unwrap();
    let mut padded_output = vec![0u16; width * height];
    pc_wiener_filter_block_padded(&mut padded_output, &params, &source).unwrap();

    assert_eq!(callback_output, padded_output);
}

#[test]
fn cell_subclasses_match_expanded_subclasses() {
    let width = 7;
    let height = 5;
    let radius = PC_WIENER_FILTER_TAP_RADIUS;
    let stride = width + 2 * radius;
    let rows = height + 2 * radius;
    let source_values: Vec<u16> = (0..stride * rows)
        .map(|index| ((index * 37 + 29) % 1024) as u16)
        .collect();
    let source = PcWienerPaddedSource::new(&source_values, stride, width, height).unwrap();
    let cell_subclasses = [1, 3, 4, 2];
    let expanded_subclasses: Vec<usize> = (0..height)
        .flat_map(|row| {
            (0..width).map(move |col| {
                cell_subclasses[(row / PC_WIENER_BLOCK_SIZE) * 2 + col / PC_WIENER_BLOCK_SIZE]
            })
        })
        .collect();
    let mut cell_output = vec![0u16; width * height];
    let mut expanded_output = vec![0u16; width * height];
    pc_wiener_filter_block_padded(
        &mut cell_output,
        &PcWienerFilter {
            width,
            height,
            output_stride: width,
            bit_depth: BitDepth::Ten,
            filter_set_index: 1,
            subclass_block_size: PC_WIENER_BLOCK_SIZE,
            subclasses: &cell_subclasses,
        },
        &source,
    )
    .unwrap();
    pc_wiener_filter_block_padded(
        &mut expanded_output,
        &PcWienerFilter {
            width,
            height,
            output_stride: width,
            bit_depth: BitDepth::Ten,
            filter_set_index: 1,
            subclass_block_size: 1,
            subclasses: &expanded_subclasses,
        },
        &source,
    )
    .unwrap();

    assert_eq!(cell_output, expanded_output);
}

#[test]
fn padded_filter_preserves_out_of_range_error_order_and_output() {
    let width = 1;
    let height = 1;
    let radius = PC_WIENER_FILTER_TAP_RADIUS;
    let stride = width + 2 * radius;
    let rows = height + 2 * radius;
    let params = PcWienerFilter {
        width,
        height,
        output_stride: width,
        bit_depth: BitDepth::Eight,
        filter_set_index: 0,
        subclass_block_size: 1,
        subclasses: &[0],
    };

    for (index, x, y) in [(radius * stride + radius, 0, 0), (radius * stride, -3, 0)] {
        let mut padded = vec![0_u16; stride * rows];
        padded[index] = 256;
        let source = PcWienerPaddedSource::new(&padded, stride, width, height).unwrap();
        let mut output = [77_u16];
        let error = pc_wiener_filter_block_padded(&mut output, &params, &source).unwrap_err();

        assert_eq!(
            error,
            ReconError::PcWienerSourceSampleOutOfRange {
                x,
                y,
                value: 256,
                max: 255,
            }
        );
        assert_eq!(output, [77]);
    }
}

#[test]
fn padded_and_callback_classify_grids_match_bit_exactly() {
    let mut params = params(BitDepth::Ten);
    params.x = 40;
    params.y = 24;
    params.base_q_idx = 96;
    let cell_cols = 3;
    let cell_rows = 4;
    let source_at =
        |x: isize, y: isize| -> u16 { ((x * 37 + y * 19 + 512).rem_euclid(1024)) as u16 };

    let callback = pc_wiener_classify_grid::<u16, _, _>(
        &params,
        cell_cols,
        cell_rows,
        |x, y| Ok(source_at(x, y)),
        alternating_tx_skip,
    )
    .unwrap();

    let (buffer, stride, origin_x, origin_y) = padded_classify_fixture(&params);
    let source = PcWienerClassifyPaddedSource::new(&buffer, stride, origin_x, origin_y);
    let padded = pc_wiener_classify_grid_padded::<u16, _>(
        &params,
        cell_cols,
        cell_rows,
        &source,
        alternating_tx_skip,
    )
    .unwrap();
    let mut scratch = PcWienerClassifyScratch::default();
    let reused = pc_wiener_classify_grid_padded_into::<u16, _>(
        &params,
        cell_cols,
        cell_rows,
        &source,
        alternating_tx_skip,
        &mut scratch,
    )
    .unwrap();

    assert_eq!(callback, padded);
    assert_eq!(padded, reused);
}

fn padded_classify_fixture(params: &PcWienerClassifyParams) -> (Vec<u16>, usize, isize, isize) {
    let origin_x = params.x - PC_WIENER_CLASSIFY_READ_RADIUS as isize;
    let origin_y = params.y - PC_WIENER_CLASSIFY_READ_RADIUS as isize;
    let stride = 64;
    let rows = 64;
    let mut buffer = Vec::with_capacity(stride * rows);
    for row in 0..rows {
        for col in 0..stride {
            let x = origin_x + col as isize;
            let y = origin_y + row as isize;
            buffer.push(((x * 37 + y * 19 + 512).rem_euclid(1024)) as u16);
        }
    }
    (buffer, stride, origin_x, origin_y)
}

fn alternating_tx_skip(lookup: PcWienerTxSkipLookup) -> Result<i32> {
    i32::try_from((lookup.row * 3 + lookup.col) & 1).map_err(|_| ReconError::ArithmeticOverflow {
        context: "test PC-Wiener tx-skip",
    })
}

#[test]
fn padded_into_reuses_capacity_from_large_to_small_grid() {
    let mut params = params(BitDepth::Eight);
    params.x = 40;
    params.y = 24;
    let (buffer, stride, origin_x, origin_y) = padded_classify_fixture(&params);
    let buffer: Vec<u8> = buffer.into_iter().map(|value| value as u8).collect();
    let source = PcWienerClassifyPaddedSource::new(&buffer, stride, origin_x, origin_y);
    let mut scratch = PcWienerClassifyScratch::default();
    let large_output_ptr = pc_wiener_classify_grid_padded_into::<u8, _>(
        &params,
        8,
        8,
        &source,
        alternating_tx_skip,
        &mut scratch,
    )
    .unwrap()
    .as_ptr();
    let source_ptr = scratch.source_cache.as_ptr();
    let feature_ptr = scratch.feature_grid.as_ptr();
    let capacities = (
        scratch.source_cache.capacity(),
        scratch.feature_grid.capacity(),
        scratch.skip_row.capacity(),
        scratch.classifications.capacity(),
    );

    let small_output_ptr = pc_wiener_classify_grid_padded_into::<u8, _>(
        &params,
        1,
        1,
        &source,
        alternating_tx_skip,
        &mut scratch,
    )
    .unwrap()
    .as_ptr();

    assert_eq!(scratch.source_cache.as_ptr(), source_ptr);
    assert_eq!(scratch.feature_grid.as_ptr(), feature_ptr);
    assert_eq!(small_output_ptr, large_output_ptr);
    assert_eq!(
        (
            scratch.source_cache.capacity(),
            scratch.feature_grid.capacity(),
            scratch.skip_row.capacity(),
            scratch.classifications.capacity(),
        ),
        capacities
    );
}

#[test]
fn padded_into_clears_output_after_error_and_empty_grid() {
    let mut params = params(BitDepth::Ten);
    params.x = 40;
    params.y = 24;
    let (buffer, stride, origin_x, origin_y) = padded_classify_fixture(&params);
    let source = PcWienerClassifyPaddedSource::new(&buffer, stride, origin_x, origin_y);
    let mut scratch = PcWienerClassifyScratch::default();
    pc_wiener_classify_grid_padded_into::<u16, _>(
        &params,
        3,
        3,
        &source,
        alternating_tx_skip,
        &mut scratch,
    )
    .unwrap();

    let error = pc_wiener_classify_grid_padded_into::<u16, _>(
        &params,
        2,
        2,
        &source,
        |_| Ok(2),
        &mut scratch,
    )
    .unwrap_err();
    assert!(matches!(error, ReconError::PcWienerInvalidTxSkip { .. }));
    assert!(scratch.classifications.is_empty());

    let recovered = pc_wiener_classify_grid_padded_into::<u16, _>(
        &params,
        1,
        1,
        &source,
        alternating_tx_skip,
        &mut scratch,
    )
    .unwrap();
    assert_eq!(recovered.len(), 1);
    let capacity = scratch.classifications.capacity();

    let empty = pc_wiener_classify_grid_padded_into::<u16, _>(
        &params,
        0,
        3,
        &source,
        alternating_tx_skip,
        &mut scratch,
    )
    .unwrap();

    assert!(empty.is_empty());
    assert_eq!(scratch.classifications.capacity(), capacity);
}

#[test]
fn classify_padded_source_rejects_origin_past_region() {
    let mut params = params(BitDepth::Ten);
    params.x = 40;
    params.y = 24;
    let buffer = vec![0u16; 64];
    let source = PcWienerClassifyPaddedSource::new(&buffer, 8, params.x + 100, params.y + 100);
    let err = pc_wiener_classify_grid_padded::<u16, _>(
        &params,
        2,
        2,
        &source,
        |lookup: PcWienerTxSkipLookup| Ok(i32::from((lookup.row + lookup.col) as u8 & 1)),
    )
    .unwrap_err();
    assert!(matches!(err, ReconError::PcWienerInvalidBounds { .. }));
}

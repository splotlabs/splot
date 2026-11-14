// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::expect_used)]

use super::*;
use crate::filters::gdf::simd::{WINDOW_ROWS, sym_rows, sym_rows_portable};

/// A 16-sample row-pair kernel.
type Kernel =
    fn(&[&[u16; 28]; WINDOW_ROWS], [&mut [u16; 16]; 2], &[GdfClass; 8], &GdfBlock, &GdfSymParams);

/// The target's kernel and the portable one.
const KERNELS: [Kernel; 2] = [sym_rows, sym_rows_portable];

/// A two-row window of `width` samples with the full tap reach around it.
fn test_source(width: usize, max_sample: u16, case: usize) -> (Vec<u16>, usize) {
    let stride = width + GDF_READ_RADIUS * 2;
    let samples = (0..stride * (2 + GDF_READ_RADIUS * 2))
        .map(|index| match case {
            0 => 0,
            1 => max_sample,
            2 => {
                if index.is_multiple_of(2) {
                    0
                } else {
                    max_sample
                }
            }
            3 => ((index * 73 + index / stride * 29) % (usize::from(max_sample) + 1)) as u16,
            _ => {
                let spread = [9, 100, 256, 400][case - 4].min(usize::from(max_sample) + 1);
                (usize::from(max_sample) / 3 + (index * 73 + index / stride * 29) % spread) as u16
            }
        })
        .collect();
    (samples, stride)
}

fn test_block(width: usize, bit_depth: BitDepth, ref_dst_idx: usize, qp_idx: usize) -> GdfBlock {
    GdfBlock {
        x: 0,
        y: 0,
        width,
        height: 2,
        frame_width: width,
        frame_height: 2,
        base_origin_y: 0,
        base_stride: width,
        bit_depth,
        qp_idx,
        ref_dst_idx,
        pix_scale: 4,
        max_sample: i32::from(bit_depth.max_sample()),
    }
}

const ORIGIN: (usize, usize) = (GDF_READ_RADIUS, GDF_READ_RADIUS);

/// Filters two rows at `ORIGIN` with `wide` and with the scalar `gdf_sample`.
fn filter_both_ways<const W: usize, const WIN: usize>(
    samples: &[u16],
    stride: usize,
    block: &GdfBlock,
    classes: &[GdfClass],
    wide: impl FnOnce(&[&[u16; WIN]; WINDOW_ROWS], [&mut [u16; W]; 2]),
) -> ([[u16; W]; 2], [[u16; W]; 2]) {
    let source = GdfSource {
        samples,
        stride,
        origin_x: 0,
        origin_y: 0,
    };
    let origin = ORIGIN;
    let max_sample = block.max_sample as usize;
    let base_luma: Vec<u16> = (0..W * 2)
        .map(|index| ((index * 61) % max_sample) as u16)
        .collect();
    let base = core::array::from_fn(|row| core::array::from_fn(|col| base_luma[row * W + col]));
    let rows = source_rows(&source, origin, WIN).expect("valid rows");
    let window = window_at::<WIN>(&rows, 0).expect("valid window");
    let mut filtered = base;
    let [top, bottom] = &mut filtered;
    wide(&window, [top, bottom]);
    let tap_offsets = gdf_tap_offsets(stride).expect("valid tap offsets");
    let scalar = core::array::from_fn(|row| {
        core::array::from_fn(|col| {
            let position = (origin.0 + col, origin.1 + row);
            let class = classes[col >> 1];
            gdf_sample(
                &base_luma,
                &source,
                &tap_offsets,
                block,
                row,
                col,
                position,
                class,
            )
        })
    });
    (filtered, scalar)
}

/// Class patterns of eight class blocks: each class alone, then mixes.
fn class_patterns() -> [[u8; 8]; 6] {
    [
        [0; 8],
        [1; 8],
        [2; 8],
        [3; 8],
        [0, 1, 2, 3, 3, 2, 1, 0],
        [3, 0, 2, 1, 1, 3, 0, 2],
    ]
}

#[test]
fn sixteen_lanes_match_scalar_samples_for_all_tables_and_classes() {
    for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
        for case in 0..8 {
            let (samples, stride) = test_source(16, bit_depth.max_sample(), case);
            for ref_dst_idx in 0..GDF_ALPHA.len() {
                for qp_idx in 0..GDF_ALPHA[0].len() {
                    let block = test_block(16, bit_depth, ref_dst_idx, qp_idx);
                    let params = GdfSymParams::new(&block);
                    for (pattern, kernel) in class_patterns()
                        .into_iter()
                        .flat_map(|pattern| KERNELS.map(|kernel| (pattern, kernel)))
                    {
                        let classes: [GdfClass; 8] = core::array::from_fn(|index| {
                            let delta = i32::try_from(index).unwrap_or_default() * 37;
                            GdfClass::new(pattern[index], 511 - delta)
                        });
                        let (actual, expected) = filter_both_ways::<16, 28>(
                            &samples,
                            stride,
                            &block,
                            &classes,
                            |window, output| kernel(window, output, &classes, &block, &params),
                        );
                        assert_eq!(
                            actual, expected,
                            "{bit_depth:?}, case {case}, reference {ref_dst_idx}, qp {qp_idx}, \
                             classes {pattern:?}"
                        );
                    }
                }
            }
        }
    }
}

/// A flat window skips the taps; one sample that the farthest tap of the
/// last lane reads, one above the rest, does not. Both match the scalar
/// samples for every table and bit depth, with class gradient biases that
/// cross error-table digits and with zero biases.
#[test]
fn flat_windows_match_scalar_samples() {
    let stride = 16 + GDF_READ_RADIUS * 2;
    let far = (ORIGIN.1 + 1 + TAP_REACH) * stride + ORIGIN.0 + 15;
    for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
        for (differ, spread) in [0, 1].into_iter().flat_map(|d| [(d, 1500), (d, 0)]) {
            let mut samples = vec![bit_depth.max_sample() / 3; stride * (2 + GDF_READ_RADIUS * 2)];
            samples[far] += differ;
            for ref_dst_idx in 0..GDF_ALPHA.len() {
                for qp_idx in 0..GDF_ALPHA[0].len() {
                    let block = test_block(16, bit_depth, ref_dst_idx, qp_idx);
                    let params = GdfSymParams::new(&block);
                    let classes: [GdfClass; 8] = core::array::from_fn(|index| {
                        let bias = i32::try_from(index * spread).unwrap_or(0);
                        GdfClass::new((index % 4) as u8, bias)
                    });
                    for kernel in KERNELS {
                        let (actual, expected) = filter_both_ways::<16, 28>(
                            &samples,
                            stride,
                            &block,
                            &classes,
                            |window, output| kernel(window, output, &classes, &block, &params),
                        );
                        assert_eq!(
                            actual, expected,
                            "{bit_depth:?}, differ {differ}, spread {spread}, reference \
                             {ref_dst_idx}, qp {qp_idx}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn narrow_widths_match_scalar_samples_for_all_tables() {
    let classes = [
        GdfClass::new(0, 511),
        GdfClass::new(1, 389),
        GdfClass::new(2, 257),
        GdfClass::new(3, 127),
    ];
    for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
        for case in 0..4 {
            for ref_dst_idx in 0..GDF_ALPHA.len() {
                for qp_idx in 0..GDF_ALPHA[0].len() {
                    narrow_case::<8, 20>(bit_depth, case, ref_dst_idx, qp_idx, &classes);
                    let pair = [classes[3 - case], classes[case]];
                    narrow_case::<4, 16>(bit_depth, case, ref_dst_idx, qp_idx, &pair);
                    narrow_case::<2, 14>(bit_depth, case, ref_dst_idx, qp_idx, &pair[1..]);
                }
            }
        }
    }
}

fn narrow_case<const W: usize, const WIN: usize>(
    bit_depth: BitDepth,
    case: usize,
    ref_dst_idx: usize,
    qp_idx: usize,
    classes: &[GdfClass],
) {
    let (samples, stride) = test_source(W, bit_depth.max_sample(), case);
    let block = test_block(W, bit_depth, ref_dst_idx, qp_idx);
    let params = GdfSymParams::new(&block);
    let (actual, expected) = filter_both_ways::<W, WIN>(
        &samples,
        stride,
        &block,
        classes,
        |window, [top, bottom]| {
            let rows = window.map(|row| &row[..]);
            let result = sym_rows_narrow(&rows, 0, W, [top, bottom], classes, &block, &params);
            assert!(result.is_some());
        },
    );
    assert_eq!(actual, expected, "width {W}, {bit_depth:?}, case {case}");
}

#[test]
fn segment_columns_with_two_wide_tail_match_scalar_samples() {
    let (width, height) = (22, 2);
    let stride = width + GDF_READ_RADIUS * 2;
    let samples: Vec<u16> = (0..stride * (height + GDF_READ_RADIUS * 2))
        .map(|index| ((index * 73 + index / stride * 29) % 256) as u16)
        .collect();
    let radius = GDF_READ_RADIUS as isize;
    let source = GdfSource {
        samples: &samples,
        stride,
        origin_x: -radius,
        origin_y: -radius,
    };
    let block = GdfBlock {
        pix_scale: 2,
        max_sample: 255,
        ..test_block(width, BitDepth::Eight, 0, 1)
    };
    let classes: Vec<GdfClass> = (0..width / 2)
        .map(|col| GdfClass::new(if col < 8 { 1 } else { (col % 4) as u8 }, 37 * col as i32))
        .collect();
    let base: Vec<u16> = (0..width * height)
        .map(|index| ((index * 61) % 255) as u16)
        .collect();
    let tap_offsets = gdf_tap_offsets(stride).expect("valid tap offsets");
    let origin = (GDF_READ_RADIUS, GDF_READ_RADIUS);
    let mut expected = base.clone();
    for row in 0..height {
        for col in 0..width {
            let position = (origin.0 + col, origin.1 + row);
            expected[row * width + col] = gdf_sample(
                &base,
                &source,
                &tap_offsets,
                &block,
                row,
                col,
                position,
                classes[col >> 1],
            );
        }
    }
    let mut actual = base;
    let result = compute_enabled_segment(&source, &mut actual, &classes, &block, origin, 0..width);
    assert!(result.is_ok());
    assert_eq!(actual, expected);
}

#[test]
fn band_classes_match_reference_over_chunks_and_tail() {
    let mut scratch = simd::ClassScratch::default();
    for (width, height) in [(2, 2), (6, 4), (14, 2), (20, 4), (34, 6), (64, 2)] {
        let stride = width + GDF_READ_RADIUS * 2;
        let samples: Vec<u16> = (0..stride * (height + GDF_READ_RADIUS * 2))
            .map(|index| ((index * 73 + index / stride * 29) % 1024) as u16)
            .collect();
        let radius = GDF_READ_RADIUS as isize;
        let source = GdfSource {
            samples: &samples,
            stride,
            origin_x: -radius,
            origin_y: -radius,
        };
        let grad_cols = width + 2;
        let mut grad = Vec::new();
        band_gradients(&source, ORIGIN, height + 2, grad_cols, &mut grad).expect("valid gradients");
        for ref_dst_idx in 0..GDF_ALPHA.len() {
            for qp_idx in 0..GDF_ALPHA[0].len() {
                let block = GdfBlock {
                    height,
                    frame_height: height,
                    ..test_block(width, BitDepth::Ten, ref_dst_idx, qp_idx)
                };
                let mut expected = Vec::new();
                band_classes(&grad, grad_cols, &block, &mut expected).expect("valid reference");
                let mut actual = Vec::new();
                let result =
                    band_classes_from_source(&source, ORIGIN, &block, &mut actual, &mut scratch);
                assert!(result.is_ok());
                assert_eq!(
                    actual, expected,
                    "width {width}, reference {ref_dst_idx}, qp {qp_idx}"
                );
            }
        }
    }
}

#[test]
fn filter_run_matches_scalar_samples_with_band_classes() {
    let mut scratch = simd::ClassScratch::default();
    for (width, height, bit_depth) in [
        (38, 4, BitDepth::Eight),
        (48, 4, BitDepth::Eight),
        (64, 6, BitDepth::Ten),
        (6, 2, BitDepth::Ten),
    ] {
        let max_sample = bit_depth.max_sample();
        let stride = width + GDF_READ_RADIUS * 2;
        let samples: Vec<u16> = (0..stride * (height + GDF_READ_RADIUS * 2))
            .map(|index| ((index * 73 + index / stride * 29) % 1024) as u16)
            .collect();
        let radius = GDF_READ_RADIUS as isize;
        let source = GdfSource {
            samples: &samples,
            stride,
            origin_x: -radius,
            origin_y: -radius,
        };
        let block = GdfBlock {
            height,
            frame_height: height,
            pix_scale: 3,
            max_sample: i32::from(max_sample),
            ..test_block(width, bit_depth, 1, 2)
        };
        let mut classes = Vec::new();
        let classified =
            band_classes_from_source(&source, ORIGIN, &block, &mut classes, &mut scratch);
        assert!(classified.is_ok());
        let base: Vec<u16> = (0..width * height)
            .map(|index| ((index * 61) % (usize::from(max_sample) + 1)) as u16)
            .collect();
        let tap_offsets = gdf_tap_offsets(stride).expect("valid tap offsets");
        let mut expected = base.clone();
        for row in 0..height {
            for col in 0..width {
                let position = (ORIGIN.0 + col, ORIGIN.1 + row);
                let class = classes[(row >> 1) * (width >> 1) + (col >> 1)];
                expected[row * width + col] = gdf_sample(
                    &base,
                    &source,
                    &tap_offsets,
                    &block,
                    row,
                    col,
                    position,
                    class,
                );
            }
        }
        let mut bytes: Vec<u8> = base.iter().map(|&sample| sample as u8).collect();
        let mut tiled_samples = vec![0; simd::TILE_STRIDE * (height + GDF_READ_RADIUS * 2)];
        for (tiled, row) in tiled_samples
            .chunks_exact_mut(simd::TILE_STRIDE)
            .zip(samples.chunks_exact(stride))
        {
            tiled[..stride].copy_from_slice(row);
        }
        let tiled = GdfSource {
            samples: &tiled_samples,
            stride: simd::TILE_STRIDE,
            ..source
        };
        scratch.errors = vec![0; 2 * simd::ClassScratch::error_row(width)];
        let mut tiled_actual = base.clone();
        assert!(
            simd::filter_run(&tiled, ORIGIN, &block, &mut tiled_actual, &mut scratch).is_some()
        );
        assert_eq!(
            tiled_actual, expected,
            "tiled, width {width}, height {height}"
        );
        let mut actual = base;
        assert!(simd::filter_run(&source, ORIGIN, &block, &mut actual, &mut scratch).is_some());
        assert_eq!(actual, expected, "width {width}, height {height}");
        if bit_depth == BitDepth::Eight {
            let mut tiled_bytes = bytes.clone();
            assert!(simd::filter_run(&source, ORIGIN, &block, &mut bytes, &mut scratch).is_some());
            assert!(
                simd::filter_run(&tiled, ORIGIN, &block, &mut tiled_bytes, &mut scratch).is_some()
            );
            for (bytes, label) in [(bytes, "u8 rows"), (tiled_bytes, "tiled u8 rows")] {
                let bytes: Vec<u16> = bytes.into_iter().map(u16::from).collect();
                assert_eq!(bytes, expected, "{label}, width {width}, height {height}");
            }
        }
    }
}

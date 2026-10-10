// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::expect_used)]

use super::*;

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
            _ => ((index * 73 + index / stride * 29) % (usize::from(max_sample) + 1)) as u16,
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

#[test]
fn uniform_width_sixteen_matches_scalar_samples_for_all_tables_and_classes() {
    for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
        let (samples, stride) = test_source(16, bit_depth.max_sample(), 3);
        for (ref_dst_idx, alpha_by_qp) in GDF_ALPHA.iter().enumerate() {
            for qp_idx in 0..alpha_by_qp.len() {
                let block = test_block(16, bit_depth, ref_dst_idx, qp_idx);
                for class_index in 0..4_u8 {
                    let classes: [GdfClass; 8] = core::array::from_fn(|index| {
                        let delta = i32::try_from(index).unwrap_or_default() * 37;
                        GdfClass::new(class_index, 511 - delta)
                    });
                    let params = GdfUniformParams::new(&block, usize::from(class_index));
                    let (actual, expected) = filter_both_ways::<16, 28>(
                        &samples,
                        stride,
                        &block,
                        &classes,
                        |window, output| params.rows(window, 0, output, &classes, &block),
                    );
                    assert_eq!(
                        actual, expected,
                        "bit depth {bit_depth:?}, reference {ref_dst_idx}, qp {qp_idx}, class \
                         {class_index}"
                    );
                }
            }
        }
    }
}

#[test]
fn mixed_classes_match_scalar_samples_for_all_tables() {
    let classes = [
        GdfClass::new(0, 511),
        GdfClass::new(1, 389),
        GdfClass::new(2, 257),
        GdfClass::new(3, 127),
    ];
    for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
        for case in 0..4 {
            let (samples_8, stride_8) = test_source(8, bit_depth.max_sample(), case);
            let (samples_4, stride_4) = test_source(4, bit_depth.max_sample(), case);
            for (ref_dst_idx, alpha_by_qp) in GDF_ALPHA.iter().enumerate() {
                for qp_idx in 0..alpha_by_qp.len() {
                    let block = test_block(8, bit_depth, ref_dst_idx, qp_idx);
                    let (actual, expected) = filter_both_ways::<8, 20>(
                        &samples_8,
                        stride_8,
                        &block,
                        &classes,
                        |window, output| {
                            let params = GdfMixedParams::new(&block);
                            mixed_class_rows(window, output, &classes, &block, &params);
                        },
                    );
                    assert_eq!(actual, expected, "width 8, {bit_depth:?}, case {case}");
                    let block = test_block(4, bit_depth, ref_dst_idx, qp_idx);
                    let pair = [classes[3 - case], classes[case]];
                    let (actual, expected) = filter_both_ways::<4, 16>(
                        &samples_4,
                        stride_4,
                        &block,
                        &pair,
                        |window, output| {
                            let params = GdfMixedParams::new(&block);
                            mixed_class_rows(window, output, &pair, &block, &params);
                        },
                    );
                    assert_eq!(actual, expected, "width 4, {bit_depth:?}, case {case}");
                }
            }
        }
    }
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
    let (width, height) = (20, 4);
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
            let mut pairs = [Vec::new(), Vec::new()];
            let result = band_classes_from_source(
                &source,
                ORIGIN,
                &block,
                &mut actual,
                &mut pairs,
                &mut Vec::new(),
            );
            assert!(result.is_ok());
            assert_eq!(actual, expected, "reference {ref_dst_idx}, qp {qp_idx}");
        }
    }
}

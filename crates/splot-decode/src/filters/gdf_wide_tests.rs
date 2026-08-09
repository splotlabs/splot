// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

#[test]
fn uniform_width_sixteen_matches_scalar_samples_for_all_tables_and_classes() {
    let source_origin = (GDF_READ_RADIUS, GDF_READ_RADIUS);
    let stride = 16 + GDF_READ_RADIUS * 2;
    let source_len = stride * (1 + GDF_READ_RADIUS * 2);
    let tap_offsets_result = gdf_tap_offsets(stride);
    assert!(tap_offsets_result.is_ok());
    let Ok(tap_offsets) = tap_offsets_result else {
        return;
    };

    for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
        let max_sample = bit_depth.max_sample();
        let samples: Vec<u16> = (0..source_len)
            .map(|index| {
                ((index * 73 + index / stride * 29) % (usize::from(max_sample) + 1)) as u16
            })
            .collect();
        let source = GdfSource {
            samples: &samples,
            stride,
            origin_x: 0,
            origin_y: 0,
        };
        let base_luma: [u16; 16] =
            core::array::from_fn(|index| ((index * 61) % usize::from(max_sample)) as u16);
        for (ref_dst_idx, alpha_by_qp) in GDF_ALPHA.iter().enumerate() {
            for qp_idx in 0..alpha_by_qp.len() {
                let block = GdfBlock {
                    x: 0,
                    y: 0,
                    width: 16,
                    height: 1,
                    frame_width: 16,
                    frame_height: 1,
                    base_origin_y: 0,
                    bit_depth,
                    qp_idx,
                    ref_dst_idx,
                    pix_scale: 4,
                    max_sample: i32::from(max_sample),
                };
                for class_index in 0..4_u8 {
                    let classes: [GdfClass; 8] = core::array::from_fn(|index| {
                        let delta = i32::try_from(index).unwrap_or_default() * 37;
                        GdfClass::new(class_index, 511 - delta)
                    });
                    let params = GdfUniformParams::new(&block, usize::from(class_index));
                    let wide_result = gdf_uniform_width_rows::<16, 1>(
                        [base_luma],
                        &source,
                        &tap_offsets,
                        &classes,
                        &params,
                        &block,
                        source_origin,
                    );
                    assert!(wide_result.is_ok());
                    let Ok([actual]) = wide_result else {
                        return;
                    };
                    let expected = core::array::from_fn(|col| {
                        gdf_sample(
                            &base_luma,
                            &source,
                            &tap_offsets,
                            &block,
                            0,
                            col,
                            (source_origin.0 + col, source_origin.1),
                            classes[col >> 1],
                        )
                    });
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
fn mixed_width_eight_matches_scalar_samples_for_all_tables() {
    let source_origin = (GDF_READ_RADIUS, GDF_READ_RADIUS);
    let stride = 8 + GDF_READ_RADIUS * 2;
    let source_len = stride * (1 + GDF_READ_RADIUS * 2);
    let tap_offsets_result = gdf_tap_offsets(stride);
    assert!(tap_offsets_result.is_ok());
    let Ok(tap_offsets) = tap_offsets_result else {
        return;
    };

    for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
        let max_sample = bit_depth.max_sample();
        let samples: Vec<u16> = (0..source_len)
            .map(|index| {
                ((index * 73 + index / stride * 29) % (usize::from(max_sample) + 1)) as u16
            })
            .collect();
        let source = GdfSource {
            samples: &samples,
            stride,
            origin_x: 0,
            origin_y: 0,
        };
        let base_luma: [u16; 8] =
            core::array::from_fn(|index| ((index * 61) % usize::from(max_sample)) as u16);
        let classes = [
            GdfClass::new(0, 511),
            GdfClass::new(1, 389),
            GdfClass::new(2, 257),
            GdfClass::new(3, 127),
        ];
        for (ref_dst_idx, alpha_by_qp) in GDF_ALPHA.iter().enumerate() {
            for qp_idx in 0..alpha_by_qp.len() {
                let block = GdfBlock {
                    x: 0,
                    y: 0,
                    width: 8,
                    height: 1,
                    frame_width: 8,
                    frame_height: 1,
                    base_origin_y: 0,
                    bit_depth,
                    qp_idx,
                    ref_dst_idx,
                    pix_scale: 4,
                    max_sample: i32::from(max_sample),
                };
                let wide_result = gdf_width8_rows(
                    [base_luma],
                    &source,
                    &tap_offsets,
                    classes,
                    &block,
                    source_origin,
                );
                assert!(wide_result.is_ok());
                let Ok([actual]) = wide_result else {
                    return;
                };
                let expected = core::array::from_fn(|col| {
                    gdf_sample(
                        &base_luma,
                        &source,
                        &tap_offsets,
                        &block,
                        0,
                        col,
                        (source_origin.0 + col, source_origin.1),
                        classes[col >> 1],
                    )
                });
                assert_eq!(
                    actual, expected,
                    "bit depth {bit_depth:?}, reference {ref_dst_idx}, qp {qp_idx}"
                );
            }
        }
    }
}

#[test]
fn width_four_row_matches_legacy_samples_for_all_tables_and_classes() {
    let source_origin = (GDF_READ_RADIUS, GDF_READ_RADIUS);
    let stride = 4 + GDF_READ_RADIUS * 2;
    let source_len = stride * (1 + GDF_READ_RADIUS * 2);
    let tap_offsets_result = gdf_tap_offsets(stride);
    assert!(tap_offsets_result.is_ok());
    let Ok(tap_offsets) = tap_offsets_result else {
        return;
    };

    for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
        let max_sample = bit_depth.max_sample();
        for source_case in 0..4 {
            let samples: Vec<u16> = (0..source_len)
                .map(|index| match source_case {
                    0 => 0,
                    1 => max_sample,
                    2 => {
                        if index.is_multiple_of(2) {
                            0
                        } else {
                            max_sample
                        }
                    }
                    _ => {
                        ((index * 73 + index / stride * 29) % (usize::from(max_sample) + 1)) as u16
                    }
                })
                .collect();
            let source = GdfSource {
                samples: &samples,
                stride,
                origin_x: 0,
                origin_y: 0,
            };
            let base_luma = [0, max_sample, max_sample / 2, max_sample];
            for (ref_dst_idx, alpha_by_qp) in GDF_ALPHA.iter().enumerate() {
                for qp_idx in 0..alpha_by_qp.len() {
                    let block = GdfBlock {
                        x: 0,
                        y: 0,
                        width: 4,
                        height: 1,
                        frame_width: 4,
                        frame_height: 1,
                        base_origin_y: 0,
                        bit_depth,
                        qp_idx,
                        ref_dst_idx,
                        pix_scale: source_case + 1,
                        max_sample: i32::from(max_sample),
                    };
                    for class_index in 0..4_u8 {
                        let class_delta = i32::from(class_index);
                        let classes = [
                            GdfClass::new(class_index, 511 - class_delta),
                            GdfClass::new(3 - class_index, 256),
                        ];
                        let row_result = gdf_width4_rows(
                            [base_luma],
                            &source,
                            &tap_offsets,
                            classes,
                            &block,
                            0,
                            source_origin,
                        );
                        assert!(row_result.is_ok());
                        let Ok([actual]) = row_result else {
                            return;
                        };
                        let expected = core::array::from_fn(|col| {
                            gdf_sample(
                                &base_luma,
                                &source,
                                &tap_offsets,
                                &block,
                                0,
                                col,
                                (source_origin.0 + col, source_origin.1),
                                classes[col >> 1],
                            )
                        });
                        assert_eq!(
                            actual, expected,
                            "bit depth {bit_depth:?}, source {source_case}, reference \
                             {ref_dst_idx}, qp {qp_idx}, class {class_index}"
                        );
                    }
                }
            }
        }
    }
}

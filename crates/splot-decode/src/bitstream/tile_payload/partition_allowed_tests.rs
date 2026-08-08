// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

const BLOCK_16X16: usize = 6;
const BLOCK_32X32: usize = 9;

fn features() -> PartitionFeatureFlags {
    PartitionFeatureFlags::new(true, true)
}

fn input(b_size: usize) -> PartitionAllowedInput {
    PartitionAllowedInput::new(
        0,
        0,
        128,
        128,
        b_size,
        PartitionTreeType::Shared,
        false,
        false,
        features(),
        true,
        false,
        8,
        false,
        false,
        1,
        None,
    )
    .unwrap()
}

fn valid_index(result: PartitionSubsize) -> Option<usize> {
    result.valid().map(BlockSize::index)
}

#[test]
fn get_plane_residual_size_matches_spec_sentinel_cases() {
    assert_eq!(
        valid_index(
            get_plane_residual_size(BlockSize::new(BLOCK_4X4).unwrap(), 1, true, false).unwrap()
        ),
        Some(BLOCK_4X4)
    );
    assert_eq!(
        get_plane_residual_size(BlockSize::new(BLOCK_4X8).unwrap(), 1, true, false).unwrap(),
        PartitionSubsize::Invalid
    );
    assert_eq!(
        valid_index(
            get_plane_residual_size(BlockSize::new(BLOCK_4X16).unwrap(), 1, true, true).unwrap()
        ),
        Some(BLOCK_4X8)
    );
    assert_eq!(
        get_plane_residual_size(BlockSize::new(BLOCK_64X128).unwrap(), 1, true, false).unwrap(),
        PartitionSubsize::Invalid
    );
    assert_eq!(
        valid_index(
            get_plane_residual_size(BlockSize::new(BLOCK_64X128).unwrap(), 1, true, true,).unwrap()
        ),
        Some(10)
    );
}

#[test]
fn boundary_and_direct_implied_partitions_are_derived() {
    assert_eq!(
        partition_implied(input(BLOCK_4X4)).unwrap(),
        Some(PartitionType::None)
    );

    let right_edge = PartitionAllowedInput {
        c: 2,
        mi_cols: 4,
        ..input(BLOCK_16X16)
    };
    assert_eq!(
        partition_implied_at_boundary(right_edge).unwrap(),
        Some(PartitionType::Vert)
    );

    let bottom_edge = PartitionAllowedInput {
        r: 2,
        mi_rows: 4,
        ..input(BLOCK_16X16)
    };
    assert_eq!(
        partition_implied_at_boundary(bottom_edge).unwrap(),
        Some(PartitionType::Horz)
    );
}

#[test]
fn chroma_direct_implied_rules_are_derived() {
    let chroma_8x8 = PartitionAllowedInput {
        tree_type: PartitionTreeType::ChromaPart,
        ..input(BLOCK_8X8)
    };
    assert_eq!(
        partition_implied(chroma_8x8).unwrap(),
        Some(PartitionType::None)
    );

    let chroma_64x64 = PartitionAllowedInput {
        tree_type: PartitionTreeType::ChromaPart,
        known_chroma_luma_partition: Some(PartitionType::Vert3),
        ..input(BLOCK_64X64)
    };
    assert_eq!(
        partition_implied(chroma_64x64).unwrap(),
        Some(PartitionType::Vert3)
    );
}

#[test]
fn rect_type_implication_includes_chroma_part_special_cases() {
    assert_eq!(
        rect_type_implied_by_bsize(
            BlockSize::new(BLOCK_4X8).unwrap(),
            PartitionTreeType::Shared
        ),
        Some(RectPartitionType::Horz)
    );
    assert_eq!(
        rect_type_implied_by_bsize(
            BlockSize::new(BLOCK_8X16).unwrap(),
            PartitionTreeType::ChromaPart,
        ),
        Some(RectPartitionType::Horz)
    );
    assert_eq!(
        rect_type_implied_by_bsize(
            BlockSize::new(BLOCK_16X8).unwrap(),
            PartitionTreeType::ChromaPart,
        ),
        Some(RectPartitionType::Vert)
    );
    assert_eq!(
        rect_type_implied_by_bsize(
            BlockSize::new(BLOCK_16X16).unwrap(),
            PartitionTreeType::Shared
        ),
        None
    );
}

#[test]
fn partition_subsize_sentinels_and_mixed_4x4_are_rejected() {
    assert!(!is_partition_allowed(input(BLOCK_4X4), PartitionType::Horz).unwrap());

    let mixed = PartitionAllowedInput {
        frame_is_intra: false,
        mixed_region: true,
        ..input(BLOCK_4X8)
    };
    assert!(!is_partition_allowed(mixed, PartitionType::Horz).unwrap());
    let allowed = init_allowed_partitions(mixed).unwrap();
    assert!(allowed.contains(PartitionType::None));
}

#[test]
fn residual_invalid_and_aspect_ratio_cases_are_rejected() {
    let residual_invalid = PartitionAllowedInput {
        has_chroma: true,
        num_planes: 3,
        subsampling_x: true,
        ..input(BLOCK_64X128)
    };
    assert!(!is_partition_allowed(residual_invalid, PartitionType::None).unwrap());

    let aspect = PartitionAllowedInput {
        max_pb_aspect_ratio: 2,
        ..input(BLOCK_4X16)
    };
    assert!(!is_partition_allowed(aspect, PartitionType::None).unwrap());
}

#[test]
fn frame_edge_none_rejection_and_empty_fallback_are_derived() {
    let frame_edge = PartitionAllowedInput {
        r: 2,
        mi_rows: 4,
        ..input(BLOCK_16X16)
    };
    assert!(!is_partition_allowed(frame_edge, PartitionType::None).unwrap());

    let fallback = PartitionAllowedInput {
        max_pb_aspect_ratio: 0,
        ..input(BLOCK_4X4)
    };
    let allowed = init_allowed_partitions(fallback).unwrap();
    assert!(allowed.contains(PartitionType::None));
}

#[test]
fn extended_and_uneven_four_way_gates_are_derived() {
    let disabled_ext = PartitionAllowedInput {
        features: PartitionFeatureFlags::new(false, true),
        ..input(BLOCK_32X32)
    };
    assert!(!is_partition_allowed(disabled_ext, PartitionType::Horz3).unwrap());

    let disabled_uneven = PartitionAllowedInput {
        features: PartitionFeatureFlags::new(true, false),
        ..input(BLOCK_32X32)
    };
    assert!(is_partition_allowed(disabled_uneven, PartitionType::Horz3).unwrap());
    assert!(!is_partition_allowed(disabled_uneven, PartitionType::Horz4A).unwrap());
}

#[test]
fn chroma_part_rect_type_and_chroma_offset_block_coded_are_checked() {
    let chroma_rect = PartitionAllowedInput {
        tree_type: PartitionTreeType::ChromaPart,
        ..input(BLOCK_8X16)
    };
    assert!(!is_partition_allowed(chroma_rect, PartitionType::Vert).unwrap());

    let outside = PartitionAllowedInput {
        r: 2,
        mi_rows: 4,
        has_chroma: true,
        chroma_offset: true,
        num_planes: 3,
        ..input(BLOCK_16X16)
    };
    assert!(!is_partition_allowed(outside, PartitionType::Horz).unwrap());
}

#[test]
fn luma_part_large_blocks_use_check_chroma_path() {
    let luma = PartitionAllowedInput {
        tree_type: PartitionTreeType::LumaPart,
        has_chroma: true,
        num_planes: 3,
        ..input(BLOCK_64X64)
    };

    assert!(is_partition_allowed(luma, PartitionType::None).unwrap());
}

#[test]
fn coordinate_arithmetic_overflow_is_typed() {
    let overflow = PartitionAllowedInput {
        r: usize::MAX,
        ..input(BLOCK_16X16)
    };
    assert!(matches!(
        partition_implied_at_boundary(overflow).unwrap_err(),
        PartitionAllowedError::CoordinateOverflow {
            coordinate: "r",
            ..
        }
    ));
}

#[test]
fn bounded_caller_fact_space_never_panics() {
    let tree_types = [
        PartitionTreeType::Shared,
        PartitionTreeType::LumaPart,
        PartitionTreeType::ChromaPart,
    ];
    let feature_sets = [
        PartitionFeatureFlags::new(false, false),
        PartitionFeatureFlags::new(true, false),
        PartitionFeatureFlags::new(true, true),
    ];
    let coordinates = [0, 2, usize::MAX - 1];
    let limits = [(0, 0), (4, 4)];

    for b_size in 0..29 {
        for tree_type in tree_types {
            for features in feature_sets {
                for subsampling_x in [false, true] {
                    for subsampling_y in [false, true] {
                        for has_chroma in [false, true] {
                            for chroma_offset in [false, true] {
                                for max_pb_aspect_ratio in [0, 8] {
                                    for (mi_rows, mi_cols) in limits {
                                        for r in coordinates {
                                            for c in coordinates {
                                                let input = PartitionAllowedInput::new(
                                                    r,
                                                    c,
                                                    mi_rows,
                                                    mi_cols,
                                                    b_size,
                                                    tree_type,
                                                    subsampling_x,
                                                    subsampling_y,
                                                    features,
                                                    true,
                                                    false,
                                                    max_pb_aspect_ratio,
                                                    has_chroma,
                                                    chroma_offset,
                                                    if has_chroma { 3 } else { 1 },
                                                    Some(PartitionType::Split),
                                                )
                                                .unwrap();

                                                let _ = partition_decision_facts(input);
                                                for partition in PartitionType::ALL {
                                                    let _ = is_partition_allowed(input, partition);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

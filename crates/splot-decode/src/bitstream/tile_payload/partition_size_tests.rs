// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

const BLOCK_4X4: usize = 0;
const BLOCK_4X8: usize = 1;
const BLOCK_8X4: usize = 2;
const BLOCK_8X16: usize = 4;
const BLOCK_16X8: usize = 5;
const BLOCK_16X16: usize = 6;
const BLOCK_16X32: usize = 7;
const BLOCK_32X16: usize = 8;
const BLOCK_32X32: usize = 9;
const BLOCK_64X64: usize = 12;
const BLOCK_128X128: usize = 15;
const BLOCK_256X256: usize = 18;
const BLOCK_4X16: usize = 19;
const BLOCK_16X4: usize = 20;

fn block(index: usize) -> BlockSize {
    BlockSize::new(index).unwrap()
}

fn valid_index(result: Option<BlockSize>) -> Option<usize> {
    result.map(BlockSize::index)
}

#[test]
fn partition_none_is_identity_for_valid_block_sizes() {
    for index in 0..BLOCK_SIZES {
        let result = partition_subsize(PartitionType::None, block(index)).unwrap();
        assert_eq!(valid_index(result), Some(index));
    }
}

#[test]
fn partition_subsize_preserves_valid_and_invalid_entries() {
    assert_eq!(
        valid_index(partition_subsize(PartitionType::Horz, block(BLOCK_4X8)).unwrap()),
        Some(BLOCK_4X4)
    );
    assert_eq!(
        valid_index(partition_subsize(PartitionType::Vert, block(BLOCK_8X4)).unwrap()),
        Some(BLOCK_4X4)
    );
    assert_eq!(
        valid_index(partition_subsize(PartitionType::Split, block(BLOCK_128X128)).unwrap()),
        Some(BLOCK_64X64)
    );
    assert_eq!(
        valid_index(partition_subsize(PartitionType::Split, block(BLOCK_256X256)).unwrap()),
        Some(BLOCK_128X128)
    );
    assert_eq!(
        partition_subsize(PartitionType::Horz, block(BLOCK_4X4)).unwrap(),
        None
    );
    assert_eq!(
        partition_subsize(PartitionType::Vert, block(BLOCK_4X8)).unwrap(),
        None
    );
}

#[test]
fn extended_partition_subsizes_match_table_entries() {
    assert_eq!(
        valid_index(partition_subsize(PartitionType::Horz3, block(BLOCK_8X16)).unwrap()),
        Some(BLOCK_8X4)
    );
    assert_eq!(
        valid_index(partition_subsize(PartitionType::Vert3, block(BLOCK_16X8)).unwrap()),
        Some(BLOCK_4X8)
    );
    assert_eq!(
        valid_index(partition_subsize(PartitionType::Horz4A, block(BLOCK_16X32)).unwrap()),
        Some(BLOCK_16X4)
    );
    assert_eq!(
        valid_index(partition_subsize(PartitionType::Horz4B, block(BLOCK_16X32)).unwrap()),
        Some(BLOCK_16X4)
    );
    assert_eq!(
        valid_index(partition_subsize(PartitionType::Vert4A, block(BLOCK_32X16)).unwrap()),
        Some(BLOCK_4X16)
    );
    assert_eq!(
        valid_index(partition_subsize(PartitionType::Vert4B, block(BLOCK_32X16)).unwrap()),
        Some(BLOCK_4X16)
    );
}

#[test]
fn horizontal_partition_midsize_preserves_valid_and_invalid_entries() {
    assert_eq!(
        valid_index(h_partition_midsize(block(BLOCK_8X16)).unwrap()),
        Some(BLOCK_4X8)
    );
    assert_eq!(
        valid_index(h_partition_midsize(block(BLOCK_64X64)).unwrap()),
        Some(BLOCK_32X32)
    );
    assert_eq!(h_partition_midsize(block(BLOCK_4X4)).unwrap(), None);
}

#[test]
fn block_size_constructor_rejects_invalid_sentinel_and_large_values() {
    assert_eq!(
        BlockSize::new(BLOCK_SIZES).unwrap_err(),
        PartitionSizeError::BlockSizeOutOfRange {
            table: "bSize",
            b_size: BLOCK_SIZES,
            max_exclusive: BLOCK_SIZES,
        }
    );
    assert_eq!(
        BlockSize::new(usize::MAX).unwrap_err(),
        PartitionSizeError::BlockSizeOutOfRange {
            table: "bSize",
            b_size: usize::MAX,
            max_exclusive: BLOCK_SIZES,
        }
    );
}

#[test]
fn block_geometry_helpers_use_generated_tables() {
    assert_eq!(block(BLOCK_4X4).num_4x4_wide().unwrap(), 1);
    assert_eq!(block(BLOCK_4X4).num_4x4_high().unwrap(), 1);
    assert_eq!(block(BLOCK_32X32).width_samples().unwrap(), 32);
    assert_eq!(block(BLOCK_32X32).height_samples().unwrap(), 32);
    assert_eq!(block(BLOCK_16X32).mi_width_log2().unwrap(), 2);
    assert_eq!(block(BLOCK_16X32).mi_height_log2().unwrap(), 3);
    assert_eq!(
        BlockSize::from_4x4_dimensions(8, 8).unwrap(),
        Some(block(BLOCK_32X32))
    );
    assert_eq!(BlockSize::from_4x4_dimensions(32, 8).unwrap(), None);
}

#[test]
fn every_partition_type_has_a_generated_table_row() {
    for partition in PartitionType::ALL {
        let _ = partition_subsize(partition, block(BLOCK_16X16)).unwrap();
    }
}

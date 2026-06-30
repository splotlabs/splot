// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 9.2 partition subsize table boundary.
//!
//! Feature tracking: `DECODE-TILE-PARTITION-SIZE-TABLE-BOUNDARY`.

use splot_core::tables::conversion::{
    H_PARTITION_MIDSIZE, MI_HEIGHT_LOG2, MI_WIDTH_LOG2, NUM_4X4_BLOCKS_HIGH, NUM_4X4_BLOCKS_WIDE,
    PARTITION_SUBSIZE,
};

use super::partition::PartitionType;

const BLOCK_SIZES: usize = 29;
const BLOCK_INVALID: i32 = 29;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BlockSize(usize);

impl BlockSize {
    pub(crate) fn new(index: usize) -> Result<Self, PartitionSizeError> {
        if index >= BLOCK_SIZES {
            return Err(block_size_out_of_range("bSize", index));
        }
        Ok(Self(index))
    }

    pub(crate) const fn index(self) -> usize {
        self.0
    }

    pub(crate) fn num_4x4_wide(self) -> Result<usize, PartitionSizeError> {
        table_usize("Num_4x4_Blocks_Wide", &NUM_4X4_BLOCKS_WIDE, self)
    }

    pub(crate) fn num_4x4_high(self) -> Result<usize, PartitionSizeError> {
        table_usize("Num_4x4_Blocks_High", &NUM_4X4_BLOCKS_HIGH, self)
    }

    pub(crate) fn width_samples(self) -> Result<usize, PartitionSizeError> {
        Ok(self.num_4x4_wide()? * 4)
    }

    pub(crate) fn height_samples(self) -> Result<usize, PartitionSizeError> {
        Ok(self.num_4x4_high()? * 4)
    }

    pub(crate) fn mi_width_log2(self) -> Result<usize, PartitionSizeError> {
        table_usize("Mi_Width_Log2", &MI_WIDTH_LOG2, self)
    }

    pub(crate) fn mi_height_log2(self) -> Result<usize, PartitionSizeError> {
        table_usize("Mi_Height_Log2", &MI_HEIGHT_LOG2, self)
    }

    pub(crate) fn from_4x4_dimensions(
        width_4x4: usize,
        height_4x4: usize,
    ) -> Result<Option<Self>, PartitionSizeError> {
        for index in 0..BLOCK_SIZES {
            let block_size = Self(index);
            if block_size.num_4x4_wide()? == width_4x4 && block_size.num_4x4_high()? == height_4x4 {
                return Ok(Some(block_size));
            }
        }
        Ok(None)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PartitionSubsize {
    Valid(BlockSize),
    Invalid,
}

impl PartitionSubsize {
    pub(crate) const fn valid(self) -> Option<BlockSize> {
        match self {
            Self::Valid(block_size) => Some(block_size),
            Self::Invalid => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum PartitionSizeError {
    #[error("{table} block-size index {b_size} is outside 0..{max_exclusive}")]
    BlockSizeOutOfRange {
        table: &'static str,
        b_size: usize,
        max_exclusive: usize,
    },
    #[error("{table}[{partition:?}][{b_size}] value {value} is not a valid block-size entry")]
    TableValueOutOfRange {
        table: &'static str,
        partition: Option<usize>,
        b_size: usize,
        value: i32,
    },
}

pub(crate) fn partition_subsize(
    partition: PartitionType,
    b_size: BlockSize,
) -> Result<PartitionSubsize, PartitionSizeError> {
    let partition_index = partition.index();
    let value = partition_table_i32(
        "Partition_Subsize",
        &PARTITION_SUBSIZE,
        partition_index,
        b_size,
    )?;
    table_value("Partition_Subsize", Some(partition_index), b_size, value)
}

pub(crate) fn h_partition_midsize(
    b_size: BlockSize,
) -> Result<PartitionSubsize, PartitionSizeError> {
    let value = table_i32("H_Partition_Midsize", &H_PARTITION_MIDSIZE, b_size)?;
    table_value("H_Partition_Midsize", None, b_size, value)
}

fn table_value(
    table: &'static str,
    partition: Option<usize>,
    b_size: BlockSize,
    value: i32,
) -> Result<PartitionSubsize, PartitionSizeError> {
    if value == BLOCK_INVALID {
        return Ok(PartitionSubsize::Invalid);
    }
    let index = usize::try_from(value)
        .map_err(|_| table_value_out_of_range(table, partition, b_size, value))?;
    let block_size = BlockSize::new(index)
        .map_err(|_| table_value_out_of_range(table, partition, b_size, value))?;
    Ok(PartitionSubsize::Valid(block_size))
}

fn table_usize(
    table: &'static str,
    values: &[i32],
    b_size: BlockSize,
) -> Result<usize, PartitionSizeError> {
    let value = table_i32(table, values, b_size)?;
    usize::try_from(value).map_err(|_| table_value_out_of_range(table, None, b_size, value))
}

fn partition_table_i32(
    table: &'static str,
    values: &[[i32; BLOCK_SIZES]],
    partition: usize,
    b_size: BlockSize,
) -> Result<i32, PartitionSizeError> {
    values
        .get(partition)
        .and_then(|row| row.get(b_size.index()))
        .copied()
        .ok_or(block_size_out_of_range(table, b_size.index()))
}

fn table_i32(
    table: &'static str,
    values: &[i32],
    b_size: BlockSize,
) -> Result<i32, PartitionSizeError> {
    values
        .get(b_size.index())
        .copied()
        .ok_or(block_size_out_of_range(table, b_size.index()))
}

fn block_size_out_of_range(table: &'static str, b_size: usize) -> PartitionSizeError {
    PartitionSizeError::BlockSizeOutOfRange {
        table,
        b_size,
        max_exclusive: BLOCK_SIZES,
    }
}

fn table_value_out_of_range(
    table: &'static str,
    partition: Option<usize>,
    b_size: BlockSize,
    value: i32,
) -> PartitionSizeError {
    PartitionSizeError::TableValueOutOfRange {
        table,
        partition,
        b_size: b_size.index(),
        value,
    }
}

#[cfg(test)]
mod tests {
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

    fn valid_index(result: PartitionSubsize) -> Option<usize> {
        result.valid().map(BlockSize::index)
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
            PartitionSubsize::Invalid
        );
        assert_eq!(
            partition_subsize(PartitionType::Vert, block(BLOCK_4X8)).unwrap(),
            PartitionSubsize::Invalid
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
        assert_eq!(
            h_partition_midsize(block(BLOCK_4X4)).unwrap(),
            PartitionSubsize::Invalid
        );
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
}

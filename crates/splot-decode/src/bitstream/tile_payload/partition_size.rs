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
const MAX_LOG2_4X4: usize = 6;

/// Block size indexed by log2 of its 4x4 width and height; every AV2 block
/// side is a power of two. `BLOCK_SIZES` marks a shape with no block size.
const BLOCK_SIZE_BY_LOG2_4X4: [[u8; MAX_LOG2_4X4 + 1]; MAX_LOG2_4X4 + 1] = {
    let mut table = [[BLOCK_SIZES as u8; MAX_LOG2_4X4 + 1]; MAX_LOG2_4X4 + 1];
    let mut index = BLOCK_SIZES;
    while index > 0 {
        index -= 1;
        let wide = NUM_4X4_BLOCKS_WIDE[index] as u32;
        let high = NUM_4X4_BLOCKS_HIGH[index] as u32;
        table[wide.trailing_zeros() as usize][high.trailing_zeros() as usize] = index as u8;
    }
    table
};

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

    /// The block size with these 4x4 dimensions, if any.
    pub(crate) fn from_4x4_dimensions(width_4x4: usize, height_4x4: usize) -> Option<Self> {
        if !width_4x4.is_power_of_two() || !height_4x4.is_power_of_two() {
            return None;
        }
        let index = *BLOCK_SIZE_BY_LOG2_4X4
            .get(width_4x4.trailing_zeros() as usize)?
            .get(height_4x4.trailing_zeros() as usize)?;
        (usize::from(index) < BLOCK_SIZES).then_some(Self(usize::from(index)))
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
) -> Result<Option<BlockSize>, PartitionSizeError> {
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
) -> Result<Option<BlockSize>, PartitionSizeError> {
    let value = table_i32("H_Partition_Midsize", &H_PARTITION_MIDSIZE, b_size)?;
    table_value("H_Partition_Midsize", None, b_size, value)
}

fn table_value(
    table: &'static str,
    partition: Option<usize>,
    b_size: BlockSize,
    value: i32,
) -> Result<Option<BlockSize>, PartitionSizeError> {
    if value == BLOCK_INVALID {
        return Ok(None);
    }
    let index = usize::try_from(value)
        .map_err(|_| table_value_out_of_range(table, partition, b_size, value))?;
    let block_size = BlockSize::new(index)
        .map_err(|_| table_value_out_of_range(table, partition, b_size, value))?;
    Ok(Some(block_size))
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
#[path = "partition_size_tests.rs"]
mod tests;

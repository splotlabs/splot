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

/// A § 9.2 per-block-size table as `u8`, padded to 32 entries so a masked
/// [`BlockSize`] index reads it without a bounds check.
const fn padded_u8_table(values: &[i32; BLOCK_SIZES]) -> [u8; 32] {
    let mut table = [0; 32];
    let mut index = 0;
    while index < BLOCK_SIZES {
        table[index] = values[index] as u8;
        index += 1;
    }
    table
}

/// `[Num_4x4_Blocks_High, Num_4x4_Blocks_Wide]`, padded like [`padded_u8_table`].
const NUM_4X4: [[u8; 2]; 32] = {
    let (high, wide) = (
        padded_u8_table(&NUM_4X4_BLOCKS_HIGH),
        padded_u8_table(&NUM_4X4_BLOCKS_WIDE),
    );
    let mut table = [[0; 2]; 32];
    let mut index = 0;
    while index < 32 {
        table[index] = [high[index], wide[index]];
        index += 1;
    }
    table
};
const MI_WIDTH_LOG2_U8: [u8; 32] = padded_u8_table(&MI_WIDTH_LOG2);
const MI_HEIGHT_LOG2_U8: [u8; 32] = padded_u8_table(&MI_HEIGHT_LOG2);
const PARTITION_SUBSIZE_U8: [[u8; 32]; PARTITION_SUBSIZE.len()] = {
    let mut table = [[0; 32]; PARTITION_SUBSIZE.len()];
    let mut partition = 0;
    while partition < PARTITION_SUBSIZE.len() {
        table[partition] = padded_u8_table(&PARTITION_SUBSIZE[partition]);
        partition += 1;
    }
    table
};
const H_PARTITION_MIDSIZE_U8: [u8; 32] = padded_u8_table(&H_PARTITION_MIDSIZE);

/// A valid `bSize`: every constructor checks the index is below
/// `BLOCK_SIZES`, so the table reads below cannot fail.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BlockSize(u8);

impl BlockSize {
    pub(crate) fn new(index: usize) -> Result<Self, PartitionSizeError> {
        if index >= BLOCK_SIZES {
            return Err(block_size_out_of_range("bSize", index));
        }
        Ok(Self(index as u8))
    }

    pub(crate) const fn index(self) -> usize {
        self.0 as usize
    }

    #[inline]
    pub(crate) const fn entry(self, table: &[u8; 32]) -> usize {
        table[self.0 as usize & 31] as usize
    }

    #[allow(
        clippy::unnecessary_wraps,
        reason = "callers share the table-error plumbing"
    )]
    #[inline]
    pub(crate) const fn num_4x4_wide(self) -> Result<usize, PartitionSizeError> {
        Ok(self.num_4x4()[1])
    }

    #[allow(
        clippy::unnecessary_wraps,
        reason = "callers share the table-error plumbing"
    )]
    #[inline]
    pub(crate) const fn num_4x4_high(self) -> Result<usize, PartitionSizeError> {
        Ok(self.num_4x4()[0])
    }

    /// `[Num_4x4_Blocks_High, Num_4x4_Blocks_Wide]`.
    #[inline]
    pub(crate) const fn num_4x4(self) -> [usize; 2] {
        let [high, wide] = NUM_4X4[self.0 as usize & 31];
        [high as usize, wide as usize]
    }

    #[inline]
    pub(crate) fn width_samples(self) -> Result<usize, PartitionSizeError> {
        Ok(self.num_4x4_wide()? * 4)
    }

    #[inline]
    pub(crate) fn height_samples(self) -> Result<usize, PartitionSizeError> {
        Ok(self.num_4x4_high()? * 4)
    }

    #[allow(
        clippy::unnecessary_wraps,
        reason = "callers share the table-error plumbing"
    )]
    #[inline]
    pub(crate) const fn mi_width_log2(self) -> Result<usize, PartitionSizeError> {
        Ok(self.entry(&MI_WIDTH_LOG2_U8))
    }

    #[allow(
        clippy::unnecessary_wraps,
        reason = "callers share the table-error plumbing"
    )]
    #[inline]
    pub(crate) const fn mi_height_log2(self) -> Result<usize, PartitionSizeError> {
        Ok(self.entry(&MI_HEIGHT_LOG2_U8))
    }

    /// The block size with these 4x4 dimensions, if any.
    pub(crate) fn from_4x4_dimensions(width_4x4: usize, height_4x4: usize) -> Option<Self> {
        if !width_4x4.is_power_of_two() || !height_4x4.is_power_of_two() {
            return None;
        }
        let index = *BLOCK_SIZE_BY_LOG2_4X4
            .get(width_4x4.trailing_zeros() as usize)?
            .get(height_4x4.trailing_zeros() as usize)?;
        (usize::from(index) < BLOCK_SIZES).then_some(Self(index))
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
}

#[allow(
    clippy::unnecessary_wraps,
    reason = "callers share the table-error plumbing"
)]
pub(crate) fn partition_subsize(
    partition: PartitionType,
    b_size: BlockSize,
) -> Result<Option<BlockSize>, PartitionSizeError> {
    Ok(table_value(
        b_size.entry(&PARTITION_SUBSIZE_U8[partition.index()]),
    ))
}

#[allow(
    clippy::unnecessary_wraps,
    reason = "callers share the table-error plumbing"
)]
pub(crate) fn h_partition_midsize(
    b_size: BlockSize,
) -> Result<Option<BlockSize>, PartitionSizeError> {
    Ok(table_value(b_size.entry(&H_PARTITION_MIDSIZE_U8)))
}

const fn table_value(value: usize) -> Option<BlockSize> {
    if value < BLOCK_SIZES {
        Some(BlockSize(value as u8))
    } else {
        None
    }
}

fn block_size_out_of_range(table: &'static str, b_size: usize) -> PartitionSizeError {
    PartitionSizeError::BlockSizeOutOfRange {
        table,
        b_size,
        max_exclusive: BLOCK_SIZES,
    }
}

#[cfg(test)]
#[path = "partition_size_tests.rs"]
mod tests;

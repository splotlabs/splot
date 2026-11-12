// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 8.3.2 partition-entry CDF context derivation.

#[cfg(test)]
use splot_core::tables::conversion::{MI_WIDTH_LOG2, NUM_4X4_BLOCKS_HIGH, NUM_4X4_BLOCKS_WIDE};

use super::super::partition_size::BlockSize;
use super::TileCdfSelector;
#[cfg(test)]
use super::{DO_SPLIT_PLANE_CONTEXTS, TileCdfArray, TileCdfError, checked_plane};

#[cfg(test)]
const BLOCK_SIZES: usize = MI_WIDTH_LOG2.len();
const BLOCK_256X256_INDEX: usize = 18;

/// Padded to 32 entries so a masked [`BlockSize`] index reads it unchecked.
const PARTITION_SIZE_ADJUST: [u8; 32] = [
    0, 0, 0, 0, 1, 1, 1, 2, 2, 2, 3, 3, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 0, 0, 0, 0, 0,
    0, 0,
];

const PARTITION_SIZE_ADJUST_RECT_TYPE: [u8; 32] = [
    0, 0, 0, 0, 1, 2, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 13, 14, 13, 14, 0, 0, 0, 0,
    0, 0, 0,
];

/// § 5.20.3.2 `rect_type` direction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RectPartitionType {
    /// `RECT_HORZ`.
    Horz,
    /// `RECT_VERT`.
    Vert,
}

/// The § 8.3.2 partition contexts of one block: its `LeftMiSizes` /
/// `AboveMiSizes` neighbours, read once. Entries are partition-context bit
/// masks: bit `k` is set when the neighbour's log2 extent is at most `k`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PartitionContextInput {
    b_size: BlockSize,
    plane_start: u8,
    /// `LeftMiSizes[ PlaneStart ]` at `r` and `r + bh4 / 2`.
    left: [u8; 2],
    /// `AboveMiSizes[ PlaneStart ]` at `c` and `c + bw4 / 2`.
    above: [u8; 2],
}

impl PartitionContextInput {
    /// The contexts of a block whose `LeftMiSizes[ PlaneStart ]` entries at
    /// `r` and `r + bh4 / 2` are `left`, and `AboveMiSizes` likewise `above`.
    #[inline]
    pub(crate) const fn from_neighbours(
        b_size: BlockSize,
        plane_start: usize,
        left: [u8; 2],
        above: [u8; 2],
    ) -> Self {
        Self {
            b_size,
            plane_start: plane_start as u8,
            left,
            above,
        }
    }

    #[cfg(test)]
    pub(crate) fn new(
        b_size: usize,
        plane_start: usize,
        r: usize,
        c: usize,
        left_mi_sizes: [&[u8]; DO_SPLIT_PLANE_CONTEXTS],
        above_mi_sizes: [&[u8]; DO_SPLIT_PLANE_CONTEXTS],
    ) -> Result<Self, TileCdfError> {
        let b_size = checked_block_size(b_size)?;
        let b_index = b_size.index();
        let plane_start = checked_plane(TileCdfArray::DoSplit, plane_start)?;
        let half_h = NUM_4X4_BLOCKS_HIGH[b_index] as usize >> 1;
        let half_w = NUM_4X4_BLOCKS_WIDE[b_index] as usize >> 1;
        let (left, above) = (left_mi_sizes[plane_start], above_mi_sizes[plane_start]);
        Ok(Self {
            b_size,
            plane_start: plane_start as u8,
            left: [
                neighbour("LeftMiSizes", left, plane_start, r)?,
                neighbour("LeftMiSizes", left, plane_start, r + half_h)?,
            ],
            above: [
                neighbour("AboveMiSizes", above, plane_start, c)?,
                neighbour("AboveMiSizes", above, plane_start, c + half_w)?,
            ],
        })
    }

    #[inline]
    pub(crate) fn do_split_selector(self) -> TileCdfSelector {
        TileCdfSelector::DoSplit {
            plane_start: usize::from(self.plane_start),
            ctx: self.neighbour_context(&PARTITION_SIZE_ADJUST),
        }
    }

    #[inline]
    pub(crate) fn rect_type_selector(self) -> TileCdfSelector {
        TileCdfSelector::RectType {
            plane_start: usize::from(self.plane_start),
            ctx: self.neighbour_context(&PARTITION_SIZE_ADJUST_RECT_TYPE),
        }
    }

    #[inline]
    pub(crate) fn do_ext_partition_selector(self, rect_type: RectPartitionType) -> TileCdfSelector {
        TileCdfSelector::DoExtPartition {
            plane_start: usize::from(self.plane_start),
            ctx: self.extended_context(rect_type),
        }
    }

    #[inline]
    pub(crate) fn do_uneven_4way_partition_selector(
        self,
        rect_type: RectPartitionType,
    ) -> TileCdfSelector {
        TileCdfSelector::DoUneven4WayPartition {
            plane_start: usize::from(self.plane_start),
            ctx: self.extended_context(rect_type),
        }
    }

    fn neighbour_context(self, size_adjustments: &[u8; 32]) -> usize {
        let bsw = log2_wide(self.b_size).max(1);
        let bsh = log2_high(self.b_size).max(1);
        partition_context(
            self.b_size.entry(size_adjustments),
            smaller(self.left[0], bsh),
            smaller(self.above[0], bsw),
        )
    }

    fn extended_context(self, rect_type: RectPartitionType) -> usize {
        let (line, log2) = match rect_type {
            RectPartitionType::Horz => (self.left, log2_high(self.b_size)),
            RectPartitionType::Vert => (self.above, log2_wide(self.b_size)),
        };
        let threshold = log2.saturating_sub(1).max(1);
        partition_context(
            self.b_size.entry(&PARTITION_SIZE_ADJUST),
            smaller(line[0], threshold),
            smaller(line[1], threshold),
        )
    }
}

/// The § 8.3.2 `do_square_split` context: `MiSizes[ 0 ]` above and left of
/// the block are the last blocks written to the luma `AboveMiSizes` /
/// `LeftMiSizes` lines (see `TileMiSizeState`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SquareSplitContextInput {
    b_size: BlockSize,
    /// 0 for an unavailable neighbour, which gives the same context bit.
    above: u8,
    left: u8,
}

impl SquareSplitContextInput {
    #[inline]
    pub(crate) const fn from_neighbours(
        b_size: BlockSize,
        above: Option<u8>,
        left: Option<u8>,
    ) -> Self {
        Self {
            b_size,
            above: match above {
                Some(above) => above,
                None => 0,
            },
            left: match left {
                Some(left) => left,
                None => 0,
            },
        }
    }

    #[cfg(test)]
    pub(crate) fn new(
        b_size: usize,
        above: Option<u8>,
        left: Option<u8>,
    ) -> Result<Self, TileCdfError> {
        Ok(Self::from_neighbours(
            checked_block_size(b_size)?,
            above,
            left,
        ))
    }

    #[inline]
    pub(crate) fn do_square_split_selector(self) -> TileCdfSelector {
        let bsw = log2_wide(self.b_size);
        let bsh = log2_high(self.b_size);
        let above = smaller(self.above, bsw);
        let left = smaller(self.left, bsh);
        TileCdfSelector::DoSquareSplit {
            plane_start: 0,
            ctx: partition_context(
                usize::from(self.b_size.index() == BLOCK_256X256_INDEX),
                left,
                above,
            ),
        }
    }
}

#[cfg(test)]
fn checked_block_size(b_size: usize) -> Result<BlockSize, TileCdfError> {
    BlockSize::new(b_size).map_err(|_| TileCdfError::BlockSizeOutOfRange {
        table: "bSize",
        b_size,
        max_exclusive: BLOCK_SIZES,
    })
}

#[cfg(test)]
fn neighbour(
    array: &'static str,
    line: &[u8],
    plane_start: usize,
    index: usize,
) -> Result<u8, TileCdfError> {
    line.get(index)
        .copied()
        .ok_or(TileCdfError::PartitionNeighborOutOfRange {
            array,
            plane_start,
            index,
            len: line.len(),
        })
}

fn log2_wide(b_size: BlockSize) -> usize {
    b_size.mi_width_log2().unwrap_or(0)
}

fn log2_high(b_size: BlockSize) -> usize {
    b_size.mi_height_log2().unwrap_or(0)
}

/// Whether the neighbour coded in `context` has a log2 extent below `log2`.
const fn smaller(context: u8, log2: usize) -> usize {
    if log2 == 0 {
        0
    } else {
        (context >> (log2 - 1) & 1) as usize
    }
}

const fn partition_context(adj_size: usize, ctx1: usize, ctx2: usize) -> usize {
    adj_size * 4 + ctx1 * 2 + ctx2
}

#[cfg(test)]
#[path = "context_tests.rs"]
mod tests;

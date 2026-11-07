// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 8.3.2 partition-entry CDF context derivation.

use splot_core::tables::conversion::{
    MI_HEIGHT_LOG2, MI_WIDTH_LOG2, NUM_4X4_BLOCKS_HIGH, NUM_4X4_BLOCKS_WIDE,
};

use super::{DO_SPLIT_PLANE_CONTEXTS, TileCdfArray, TileCdfError, TileCdfSelector, checked_plane};

const BLOCK_SIZES: usize = MI_WIDTH_LOG2.len();
const BLOCK_256X256_INDEX: usize = 18;

const PARTITION_SIZE_ADJUST: [usize; BLOCK_SIZES] = [
    0, 0, 0, 0, 1, 1, 1, 2, 2, 2, 3, 3, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 0, 0, 0, 0,
];

const PARTITION_SIZE_ADJUST_RECT_TYPE: [usize; BLOCK_SIZES] = [
    0, 0, 0, 0, 1, 2, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 13, 14, 13, 14, 0, 0, 0, 0,
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
    b_size: usize,
    plane_start: usize,
    /// `LeftMiSizes[ PlaneStart ]` at `r` and `r + bh4 / 2`.
    left: [u8; 2],
    /// `AboveMiSizes[ PlaneStart ]` at `c` and `c + bw4 / 2`.
    above: [u8; 2],
}

impl PartitionContextInput {
    pub(crate) fn new(
        b_size: usize,
        plane_start: usize,
        r: usize,
        c: usize,
        left_mi_sizes: [&[u8]; DO_SPLIT_PLANE_CONTEXTS],
        above_mi_sizes: [&[u8]; DO_SPLIT_PLANE_CONTEXTS],
    ) -> Result<Self, TileCdfError> {
        let b_size = checked_block_size(b_size)?;
        let plane_start = checked_plane(TileCdfArray::DoSplit, plane_start)?;
        let half_h = NUM_4X4_BLOCKS_HIGH[b_size] as usize >> 1;
        let half_w = NUM_4X4_BLOCKS_WIDE[b_size] as usize >> 1;
        let (left, above) = (left_mi_sizes[plane_start], above_mi_sizes[plane_start]);
        Ok(Self {
            b_size,
            plane_start,
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
            plane_start: self.plane_start,
            ctx: self.neighbour_context(&PARTITION_SIZE_ADJUST),
        }
    }

    #[inline]
    pub(crate) fn rect_type_selector(self) -> TileCdfSelector {
        TileCdfSelector::RectType {
            plane_start: self.plane_start,
            ctx: self.neighbour_context(&PARTITION_SIZE_ADJUST_RECT_TYPE),
        }
    }

    #[inline]
    pub(crate) fn do_ext_partition_selector(self, rect_type: RectPartitionType) -> TileCdfSelector {
        TileCdfSelector::DoExtPartition {
            plane_start: self.plane_start,
            ctx: self.extended_context(rect_type),
        }
    }

    #[inline]
    pub(crate) fn do_uneven_4way_partition_selector(
        self,
        rect_type: RectPartitionType,
    ) -> TileCdfSelector {
        TileCdfSelector::DoUneven4WayPartition {
            plane_start: self.plane_start,
            ctx: self.extended_context(rect_type),
        }
    }

    fn neighbour_context(self, size_adjustments: &[usize; BLOCK_SIZES]) -> usize {
        let bsw = (MI_WIDTH_LOG2[self.b_size] as usize).max(1);
        let bsh = (MI_HEIGHT_LOG2[self.b_size] as usize).max(1);
        partition_context(
            size_adjustments[self.b_size],
            smaller(self.left[0], bsh),
            smaller(self.above[0], bsw),
        )
    }

    fn extended_context(self, rect_type: RectPartitionType) -> usize {
        let (line, log2) = match rect_type {
            RectPartitionType::Horz => (self.left, MI_HEIGHT_LOG2[self.b_size]),
            RectPartitionType::Vert => (self.above, MI_WIDTH_LOG2[self.b_size]),
        };
        let threshold = (log2 as usize).saturating_sub(1).max(1);
        partition_context(
            PARTITION_SIZE_ADJUST[self.b_size],
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
    b_size: usize,
    above: Option<u8>,
    left: Option<u8>,
}

impl SquareSplitContextInput {
    pub(crate) fn new(
        b_size: usize,
        above: Option<u8>,
        left: Option<u8>,
    ) -> Result<Self, TileCdfError> {
        Ok(Self {
            b_size: checked_block_size(b_size)?,
            above,
            left,
        })
    }

    #[inline]
    pub(crate) fn do_square_split_selector(self) -> TileCdfSelector {
        let bsw = MI_WIDTH_LOG2[self.b_size] as usize;
        let bsh = MI_HEIGHT_LOG2[self.b_size] as usize;
        let above = self.above.map_or(0, |above| smaller(above, bsw));
        let left = self.left.map_or(0, |left| smaller(left, bsh));
        TileCdfSelector::DoSquareSplit {
            plane_start: 0,
            ctx: partition_context(usize::from(self.b_size == BLOCK_256X256_INDEX), left, above),
        }
    }
}

fn checked_block_size(b_size: usize) -> Result<usize, TileCdfError> {
    if b_size >= BLOCK_SIZES {
        return Err(TileCdfError::BlockSizeOutOfRange {
            table: "bSize",
            b_size,
            max_exclusive: BLOCK_SIZES,
        });
    }
    Ok(b_size)
}

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

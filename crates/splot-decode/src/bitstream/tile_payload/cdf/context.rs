// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 8.3.2 partition-entry CDF context derivation.

use splot_core::tables::conversion::{
    MI_HEIGHT_LOG2, MI_WIDTH_LOG2, NUM_4X4_BLOCKS_HIGH, NUM_4X4_BLOCKS_WIDE,
};

use super::{
    DO_EXT_PARTITION_CONTEXTS, DO_SPLIT_CONTEXTS, DO_SPLIT_PLANE_CONTEXTS,
    DO_SQUARE_SPLIT_CONTEXTS, DO_UNEVEN_4WAY_PARTITION_CONTEXTS, RECT_TYPE_CONTEXTS, TileCdfArray,
    TileCdfError, TileCdfSelector, checked_plane, checked_square_split_plane,
};

const BLOCK_SIZES: usize = MI_WIDTH_LOG2.len();
const BLOCK_256X256_INDEX: usize = 18;

const PARTITION_SIZE_ADJUST: [usize; BLOCK_SIZES] = [
    0, 0, 0, 0, 1, 1, 1, 2, 2, 2, 3, 3, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 0, 0, 0, 0,
];

const PARTITION_SIZE_ADJUST_RECT_TYPE: [usize; BLOCK_SIZES] = [
    0, 0, 0, 0, 1, 2, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 13, 14, 13, 14, 0, 0, 0, 0,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RectPartitionType {
    Horz,
    Vert,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PartitionContextInput<'a> {
    b_size: BlockSizeIndex,
    plane_start: usize,
    r: usize,
    c: usize,
    left_mi_sizes: [&'a [usize]; DO_SPLIT_PLANE_CONTEXTS],
    above_mi_sizes: [&'a [usize]; DO_SPLIT_PLANE_CONTEXTS],
}

impl<'a> PartitionContextInput<'a> {
    pub(crate) fn new(
        b_size: usize,
        plane_start: usize,
        r: usize,
        c: usize,
        left_mi_sizes: [&'a [usize]; DO_SPLIT_PLANE_CONTEXTS],
        above_mi_sizes: [&'a [usize]; DO_SPLIT_PLANE_CONTEXTS],
    ) -> Result<Self, TileCdfError> {
        Ok(Self {
            b_size: BlockSizeIndex::new(b_size, "bSize")?,
            plane_start,
            r,
            c,
            left_mi_sizes,
            above_mi_sizes,
        })
    }

    pub(crate) fn do_split_selector(self) -> Result<TileCdfSelector, TileCdfError> {
        self.neighbor_partition_selector(
            TileCdfArray::DoSplit,
            &PARTITION_SIZE_ADJUST,
            DO_SPLIT_CONTEXTS,
            |plane_start, ctx| TileCdfSelector::DoSplit { plane_start, ctx },
        )
    }

    pub(crate) fn rect_type_selector(self) -> Result<TileCdfSelector, TileCdfError> {
        self.neighbor_partition_selector(
            TileCdfArray::RectType,
            &PARTITION_SIZE_ADJUST_RECT_TYPE,
            RECT_TYPE_CONTEXTS,
            |plane_start, ctx| TileCdfSelector::RectType { plane_start, ctx },
        )
    }

    pub(crate) fn do_ext_partition_selector(
        self,
        rect_type: RectPartitionType,
    ) -> Result<TileCdfSelector, TileCdfError> {
        self.extended_partition_selector(
            TileCdfArray::DoExtPartition,
            rect_type,
            DO_EXT_PARTITION_CONTEXTS,
            |plane_start, ctx| TileCdfSelector::DoExtPartition { plane_start, ctx },
        )
    }

    pub(crate) fn do_uneven_4way_partition_selector(
        self,
        rect_type: RectPartitionType,
    ) -> Result<TileCdfSelector, TileCdfError> {
        self.extended_partition_selector(
            TileCdfArray::DoUneven4WayPartition,
            rect_type,
            DO_UNEVEN_4WAY_PARTITION_CONTEXTS,
            |plane_start, ctx| TileCdfSelector::DoUneven4WayPartition { plane_start, ctx },
        )
    }

    fn neighbor_partition_selector(
        self,
        array: TileCdfArray,
        size_adjustments: &[usize; BLOCK_SIZES],
        max_exclusive: usize,
        selector: impl FnOnce(usize, usize) -> TileCdfSelector,
    ) -> Result<TileCdfSelector, TileCdfError> {
        let plane_start = checked_plane(array, self.plane_start)?;
        let ctx = self.neighbor_partition_context(
            array,
            plane_start,
            size_adjustments[self.b_size.index()],
            max_exclusive,
        )?;
        Ok(selector(plane_start, ctx))
    }

    fn extended_partition_selector(
        self,
        array: TileCdfArray,
        rect_type: RectPartitionType,
        max_exclusive: usize,
        selector: impl FnOnce(usize, usize) -> TileCdfSelector,
    ) -> Result<TileCdfSelector, TileCdfError> {
        let plane_start = checked_plane(array, self.plane_start)?;
        let ctx = self.extended_partition_context(array, plane_start, rect_type, max_exclusive)?;
        Ok(selector(plane_start, ctx))
    }

    fn neighbor_partition_context(
        self,
        array: TileCdfArray,
        plane_start: usize,
        adj_size: usize,
        max_exclusive: usize,
    ) -> Result<usize, TileCdfError> {
        let bsw = self.b_size.width_log2()?.max(1);
        let bsh = self.b_size.height_log2()?.max(1);
        let ctx1 = self.left_context(plane_start, self.r, bsh)?;
        let ctx2 = self.above_context(plane_start, self.c, bsw)?;
        partition_context(array, adj_size, ctx1, ctx2, max_exclusive)
    }

    fn extended_partition_context(
        self,
        array: TileCdfArray,
        plane_start: usize,
        rect_type: RectPartitionType,
        max_exclusive: usize,
    ) -> Result<usize, TileCdfError> {
        let (ctx1, ctx2) = match rect_type {
            RectPartitionType::Horz => {
                let bsh = self.b_size.height_log2()?.saturating_sub(1).max(1);
                let offset = self.b_size.blocks_high()? >> 1;
                let second = checked_neighbor_index("LeftMiSizes", plane_start, self.r, offset)?;
                (
                    self.left_context(plane_start, self.r, bsh)?,
                    self.left_context(plane_start, second, bsh)?,
                )
            }
            RectPartitionType::Vert => {
                let bsw = self.b_size.width_log2()?.saturating_sub(1).max(1);
                let offset = self.b_size.blocks_wide()? >> 1;
                let second = checked_neighbor_index("AboveMiSizes", plane_start, self.c, offset)?;
                (
                    self.above_context(plane_start, self.c, bsw)?,
                    self.above_context(plane_start, second, bsw)?,
                )
            }
        };

        partition_context(
            array,
            PARTITION_SIZE_ADJUST[self.b_size.index()],
            ctx1,
            ctx2,
            max_exclusive,
        )
    }

    fn left_context(
        self,
        plane_start: usize,
        index: usize,
        threshold: usize,
    ) -> Result<usize, TileCdfError> {
        neighbor_context(
            "LeftMiSizes",
            self.left_mi_sizes[plane_start],
            plane_start,
            index,
            threshold,
        )
    }

    fn above_context(
        self,
        plane_start: usize,
        index: usize,
        threshold: usize,
    ) -> Result<usize, TileCdfError> {
        neighbor_context(
            "AboveMiSizes",
            self.above_mi_sizes[plane_start],
            plane_start,
            index,
            threshold,
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SquareSplitContextInput<'a> {
    b_size: BlockSizeIndex,
    plane_start: usize,
    r: usize,
    c: usize,
    avail_u: bool,
    avail_l: bool,
    mi_sizes: [&'a [&'a [usize]]; DO_SPLIT_PLANE_CONTEXTS],
}

impl<'a> SquareSplitContextInput<'a> {
    pub(crate) fn new(
        b_size: usize,
        plane_start: usize,
        r: usize,
        c: usize,
        avail_u: bool,
        avail_l: bool,
        mi_sizes: [&'a [&'a [usize]]; DO_SPLIT_PLANE_CONTEXTS],
    ) -> Result<Self, TileCdfError> {
        Ok(Self {
            b_size: BlockSizeIndex::new(b_size, "bSize")?,
            plane_start,
            r,
            c,
            avail_u,
            avail_l,
            mi_sizes,
        })
    }

    pub(crate) fn do_square_split_selector(self) -> Result<TileCdfSelector, TileCdfError> {
        let plane_start = checked_square_split_plane(self.plane_start)?;
        let bsw = self.b_size.width_log2()?;
        let bsh = self.b_size.height_log2()?;
        let above = if self.avail_u {
            let row = checked_grid_coordinate("MiSizes", plane_start, "r", self.r, 1)?;
            grid_log2(
                "MiSizes",
                self.mi_sizes[plane_start],
                plane_start,
                row,
                self.c,
                "Mi_Width_Log2",
                &MI_WIDTH_LOG2,
            )? < bsw
        } else {
            false
        };
        let left = if self.avail_l {
            let col = checked_grid_coordinate("MiSizes", plane_start, "c", self.c, 1)?;
            grid_log2(
                "MiSizes",
                self.mi_sizes[plane_start],
                plane_start,
                self.r,
                col,
                "Mi_Height_Log2",
                &MI_HEIGHT_LOG2,
            )? < bsh
        } else {
            false
        };
        let ctx = partition_context(
            TileCdfArray::DoSquareSplit,
            context_bit(self.b_size.index() == BLOCK_256X256_INDEX),
            context_bit(left),
            context_bit(above),
            DO_SQUARE_SPLIT_CONTEXTS,
        )?;

        Ok(TileCdfSelector::DoSquareSplit { plane_start, ctx })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct BlockSizeIndex(usize);

impl BlockSizeIndex {
    fn new(index: usize, table: &'static str) -> Result<Self, TileCdfError> {
        if index >= BLOCK_SIZES {
            return Err(TileCdfError::BlockSizeOutOfRange {
                table,
                b_size: index,
                max_exclusive: BLOCK_SIZES,
            });
        }
        Ok(Self(index))
    }

    const fn index(self) -> usize {
        self.0
    }

    fn width_log2(self) -> Result<usize, TileCdfError> {
        conversion_table_value("Mi_Width_Log2", &MI_WIDTH_LOG2, self)
    }

    fn height_log2(self) -> Result<usize, TileCdfError> {
        conversion_table_value("Mi_Height_Log2", &MI_HEIGHT_LOG2, self)
    }

    fn blocks_wide(self) -> Result<usize, TileCdfError> {
        conversion_table_value("Num_4x4_Blocks_Wide", &NUM_4X4_BLOCKS_WIDE, self)
    }

    fn blocks_high(self) -> Result<usize, TileCdfError> {
        conversion_table_value("Num_4x4_Blocks_High", &NUM_4X4_BLOCKS_HIGH, self)
    }
}

fn conversion_table_value(
    table_name: &'static str,
    table: &'static [i32],
    b_size: BlockSizeIndex,
) -> Result<usize, TileCdfError> {
    let value = *table
        .get(b_size.index())
        .ok_or(TileCdfError::BlockSizeOutOfRange {
            table: table_name,
            b_size: b_size.index(),
            max_exclusive: table.len(),
        })?;
    usize::try_from(value).map_err(|_| TileCdfError::ConversionTableValueOutOfRange {
        table: table_name,
        b_size: b_size.index(),
        value,
    })
}

fn neighbor_partition_context(
    array: &'static str,
    neighbors: &[usize],
    plane_start: usize,
    index: usize,
) -> Result<usize, TileCdfError> {
    let context = *neighbors
        .get(index)
        .ok_or(TileCdfError::PartitionNeighborOutOfRange {
            array,
            plane_start,
            index,
            len: neighbors.len(),
        })?;
    if context >= 64 {
        return Err(TileCdfError::PartitionNeighborContextOutOfRange {
            array,
            plane_start,
            index,
            context,
            max_exclusive: 64,
        });
    }
    Ok(context)
}

fn neighbor_context(
    array: &'static str,
    neighbors: &[usize],
    plane_start: usize,
    index: usize,
    threshold: usize,
) -> Result<usize, TileCdfError> {
    let context = neighbor_partition_context(array, neighbors, plane_start, index)?;
    Ok((context >> threshold.saturating_sub(1)) & 1)
}

fn checked_grid_coordinate(
    array: &'static str,
    plane_start: usize,
    coordinate: &'static str,
    actual: usize,
    subtract: usize,
) -> Result<usize, TileCdfError> {
    actual
        .checked_sub(subtract)
        .ok_or(TileCdfError::PartitionGridCoordinateUnderflow {
            array,
            plane_start,
            coordinate,
            actual,
            subtract,
        })
}

fn grid_block_size(
    array: &'static str,
    grid: &[&[usize]],
    plane_start: usize,
    row: usize,
    col: usize,
) -> Result<BlockSizeIndex, TileCdfError> {
    let row_cells = grid
        .get(row)
        .ok_or(TileCdfError::PartitionGridRowOutOfRange {
            array,
            plane_start,
            row,
            rows: grid.len(),
        })?;
    let block_size = *row_cells
        .get(col)
        .ok_or(TileCdfError::PartitionGridColumnOutOfRange {
            array,
            plane_start,
            row,
            col,
            cols: row_cells.len(),
        })?;
    BlockSizeIndex::new(block_size, array).map_err(|_| {
        TileCdfError::PartitionGridBlockSizeOutOfRange {
            array,
            plane_start,
            row,
            col,
            block_size,
            max_exclusive: BLOCK_SIZES,
        }
    })
}

fn grid_log2(
    array: &'static str,
    grid: &[&[usize]],
    plane_start: usize,
    row: usize,
    col: usize,
    table_name: &'static str,
    table: &'static [i32],
) -> Result<usize, TileCdfError> {
    let block_size = grid_block_size(array, grid, plane_start, row, col)?;
    conversion_table_value(table_name, table, block_size)
}

fn checked_neighbor_index(
    array: &'static str,
    plane_start: usize,
    base: usize,
    offset: usize,
) -> Result<usize, TileCdfError> {
    base.checked_add(offset)
        .ok_or(TileCdfError::PartitionNeighborIndexOverflow {
            array,
            plane_start,
            base,
            offset,
        })
}

fn partition_context(
    array: TileCdfArray,
    adj_size: usize,
    ctx1: usize,
    ctx2: usize,
    max_exclusive: usize,
) -> Result<usize, TileCdfError> {
    let ctx = adj_size * 4 + ctx1 * 2 + ctx2;
    if ctx >= max_exclusive {
        return Err(TileCdfError::SelectorOutOfRange {
            array,
            index_name: "ctx",
            actual: ctx,
            max_exclusive,
        });
    }
    Ok(ctx)
}

const fn context_bit(value: bool) -> usize {
    if value { 1 } else { 0 }
}

#[cfg(test)]
#[path = "context_tests.rs"]
mod tests;

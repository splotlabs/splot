// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 8.3.2 partition-entry CDF context derivation.
//!
//! Feature tracking: `DECODE-TILE-CDF-SELECTION-BOUNDARY`.

use splot_core::tables::conversion::{
    MI_HEIGHT_LOG2, MI_WIDTH_LOG2, NUM_4X4_BLOCKS_HIGH, NUM_4X4_BLOCKS_WIDE,
};

use super::{
    DO_EXT_PARTITION_CONTEXTS, DO_SPLIT_CONTEXTS, DO_SPLIT_PLANE_CONTEXTS,
    DO_SQUARE_SPLIT_CONTEXTS, DO_UNEVEN_4WAY_PARTITION_CONTEXTS, RECT_TYPE_CONTEXTS, TileCdfArray,
    TileCdfError, TileCdfSelector, checked_plane, checked_square_split_plane,
};

const BLOCK_SIZES: usize = MI_WIDTH_LOG2.len();
// AV2 § 8.3.2 do_square_split uses the BLOCK_256X256 discriminant from the
// generated § 9.2 block-size conversion-table order.
const BLOCK_256X256_INDEX: usize = 18;

// AV2 § 8.3.2 Partition_Size_Adjust
// (`docs/spec/av2/1.0.0/08-parsing-process.md#s-8-3-2`).
const PARTITION_SIZE_ADJUST: [usize; BLOCK_SIZES] = [
    0, 0, 0, 0, 1, 1, 1, 2, 2, 2, 3, 3, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 0, 0, 0, 0,
];

// AV2 § 8.3.2 Partition_Size_Adjust_Rect_Type
// (`docs/spec/av2/1.0.0/08-parsing-process.md#s-8-3-2`).
const PARTITION_SIZE_ADJUST_RECT_TYPE: [usize; BLOCK_SIZES] = [
    0, 0, 0, 0, 1, 2, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 13, 14, 13, 14, 0, 0, 0, 0,
];

/// Rectangular partition direction used by AV2 § 8.3.2 extended contexts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RectPartitionType {
    /// `RECT_HORZ`; context is derived from left-neighbor heights.
    Horz,
    /// `RECT_VERT`; context is derived from above-neighbor widths.
    Vert,
}

/// Bounded inputs for AV2 § 8.3.2 partition-entry CDF context derivation.
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
    /// Creates context inputs from tile-neighbor block-size state.
    ///
    /// `bSize` is checked eagerly. `PlaneStart`, `r`, `c`, second-half offsets,
    /// and neighbor entries are checked by each selector method against its
    /// target CDF array.
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

    /// Derives `TileDoSplitCdf[PlaneStart][ctx]` for AV2 § 8.3.2.
    pub(crate) fn do_split_selector(self) -> Result<TileCdfSelector, TileCdfError> {
        let plane_start = checked_plane(TileCdfArray::DoSplit, self.plane_start)?;
        let bsw = self.mi_width_log2()?.max(1);
        let bsh = self.mi_height_log2()?.max(1);
        let ctx1 = context_bit(self.left_height_log2(plane_start, self.r)? < bsh);
        let ctx2 = context_bit(self.above_width_log2(plane_start, self.c)? < bsw);
        let ctx = partition_context(
            TileCdfArray::DoSplit,
            PARTITION_SIZE_ADJUST[self.b_size.index()],
            ctx1,
            ctx2,
            DO_SPLIT_CONTEXTS,
        )?;

        Ok(TileCdfSelector::DoSplit { plane_start, ctx })
    }

    /// Derives `TileRectTypeCdf[PlaneStart][ctx]` for AV2 § 8.3.2.
    pub(crate) fn rect_type_selector(self) -> Result<TileCdfSelector, TileCdfError> {
        let plane_start = checked_plane(TileCdfArray::RectType, self.plane_start)?;
        let bsw = self.mi_width_log2()?.max(1);
        let bsh = self.mi_height_log2()?.max(1);
        let ctx1 = context_bit(self.left_height_log2(plane_start, self.r)? < bsh);
        let ctx2 = context_bit(self.above_width_log2(plane_start, self.c)? < bsw);
        let ctx = partition_context(
            TileCdfArray::RectType,
            PARTITION_SIZE_ADJUST_RECT_TYPE[self.b_size.index()],
            ctx1,
            ctx2,
            RECT_TYPE_CONTEXTS,
        )?;

        Ok(TileCdfSelector::RectType { plane_start, ctx })
    }

    /// Derives `TileDoExtPartitionCdf[PlaneStart][ctx]` for AV2 § 8.3.2.
    pub(crate) fn do_ext_partition_selector(
        self,
        rect_type: RectPartitionType,
    ) -> Result<TileCdfSelector, TileCdfError> {
        let plane_start = checked_plane(TileCdfArray::DoExtPartition, self.plane_start)?;
        let ctx = self.extended_partition_context(
            TileCdfArray::DoExtPartition,
            plane_start,
            rect_type,
            DO_EXT_PARTITION_CONTEXTS,
        )?;

        Ok(TileCdfSelector::DoExtPartition { plane_start, ctx })
    }

    /// Derives `TileDoUneven4wayPartitionCdf[PlaneStart][ctx]` for AV2 § 8.3.2.
    pub(crate) fn do_uneven_4way_partition_selector(
        self,
        rect_type: RectPartitionType,
    ) -> Result<TileCdfSelector, TileCdfError> {
        let plane_start = checked_plane(TileCdfArray::DoUneven4WayPartition, self.plane_start)?;
        let ctx = self.extended_partition_context(
            TileCdfArray::DoUneven4WayPartition,
            plane_start,
            rect_type,
            DO_UNEVEN_4WAY_PARTITION_CONTEXTS,
        )?;

        Ok(TileCdfSelector::DoUneven4WayPartition { plane_start, ctx })
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
                let bsh = self.mi_height_log2()?.saturating_sub(1).max(1);
                let offset = conversion_table_value(
                    "Num_4x4_Blocks_High",
                    &NUM_4X4_BLOCKS_HIGH,
                    self.b_size,
                )? >> 1;
                let second = checked_neighbor_index("LeftMiSizes", plane_start, self.r, offset)?;
                (
                    context_bit(self.left_height_log2(plane_start, self.r)? < bsh),
                    context_bit(self.left_height_log2(plane_start, second)? < bsh),
                )
            }
            RectPartitionType::Vert => {
                let bsw = self.mi_width_log2()?.saturating_sub(1).max(1);
                let offset = conversion_table_value(
                    "Num_4x4_Blocks_Wide",
                    &NUM_4X4_BLOCKS_WIDE,
                    self.b_size,
                )? >> 1;
                let second = checked_neighbor_index("AboveMiSizes", plane_start, self.c, offset)?;
                (
                    context_bit(self.above_width_log2(plane_start, self.c)? < bsw),
                    context_bit(self.above_width_log2(plane_start, second)? < bsw),
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

    fn mi_width_log2(self) -> Result<usize, TileCdfError> {
        conversion_table_value("Mi_Width_Log2", &MI_WIDTH_LOG2, self.b_size)
    }

    fn mi_height_log2(self) -> Result<usize, TileCdfError> {
        conversion_table_value("Mi_Height_Log2", &MI_HEIGHT_LOG2, self.b_size)
    }

    fn left_height_log2(self, plane_start: usize, index: usize) -> Result<usize, TileCdfError> {
        let block_size = neighbor_block_size(
            "LeftMiSizes",
            self.left_mi_sizes[plane_start],
            plane_start,
            index,
        )?;
        conversion_table_value("Mi_Height_Log2", &MI_HEIGHT_LOG2, block_size)
    }

    fn above_width_log2(self, plane_start: usize, index: usize) -> Result<usize, TileCdfError> {
        let block_size = neighbor_block_size(
            "AboveMiSizes",
            self.above_mi_sizes[plane_start],
            plane_start,
            index,
        )?;
        conversion_table_value("Mi_Width_Log2", &MI_WIDTH_LOG2, block_size)
    }
}

/// Bounded inputs for AV2 § 8.3.2 `do_square_split` context derivation.
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
    /// Creates context inputs from caller-owned `MiSizes` block-size state.
    ///
    /// `bSize` is checked eagerly. `PlaneStart`, availability-gated neighbor
    /// coordinates, grid entries, and the final context are checked by
    /// [`Self::do_square_split_selector`].
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

    /// Derives `TileDoSquareSplitCdf[0][ctx]` for AV2 § 8.3.2.
    pub(crate) fn do_square_split_selector(self) -> Result<TileCdfSelector, TileCdfError> {
        let plane_start = checked_square_split_plane(self.plane_start)?;
        let bsw = self.mi_width_log2()?;
        let bsh = self.mi_height_log2()?;
        let above = if self.avail_u {
            let row = checked_grid_coordinate("MiSizes", plane_start, "r", self.r, 1)?;
            self.grid_width_log2(plane_start, row, self.c)? < bsw
        } else {
            false
        };
        let left = if self.avail_l {
            let col = checked_grid_coordinate("MiSizes", plane_start, "c", self.c, 1)?;
            self.grid_height_log2(plane_start, self.r, col)? < bsh
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

    fn mi_width_log2(self) -> Result<usize, TileCdfError> {
        conversion_table_value("Mi_Width_Log2", &MI_WIDTH_LOG2, self.b_size)
    }

    fn mi_height_log2(self) -> Result<usize, TileCdfError> {
        conversion_table_value("Mi_Height_Log2", &MI_HEIGHT_LOG2, self.b_size)
    }

    fn grid_width_log2(
        self,
        plane_start: usize,
        row: usize,
        col: usize,
    ) -> Result<usize, TileCdfError> {
        let block_size =
            grid_block_size("MiSizes", self.mi_sizes[plane_start], plane_start, row, col)?;
        conversion_table_value("Mi_Width_Log2", &MI_WIDTH_LOG2, block_size)
    }

    fn grid_height_log2(
        self,
        plane_start: usize,
        row: usize,
        col: usize,
    ) -> Result<usize, TileCdfError> {
        let block_size =
            grid_block_size("MiSizes", self.mi_sizes[plane_start], plane_start, row, col)?;
        conversion_table_value("Mi_Height_Log2", &MI_HEIGHT_LOG2, block_size)
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

fn neighbor_block_size(
    array: &'static str,
    neighbors: &[usize],
    plane_start: usize,
    index: usize,
) -> Result<BlockSizeIndex, TileCdfError> {
    let block_size = *neighbors
        .get(index)
        .ok_or(TileCdfError::PartitionNeighborOutOfRange {
            array,
            plane_start,
            index,
            len: neighbors.len(),
        })?;
    BlockSizeIndex::new(block_size, array).map_err(|_| {
        TileCdfError::PartitionNeighborBlockSizeOutOfRange {
            array,
            plane_start,
            index,
            block_size,
            max_exclusive: BLOCK_SIZES,
        }
    })
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
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use splot_core::tables::cdf::{
        DEFAULT_DO_EXT_PARTITION_CDF, DEFAULT_DO_SPLIT_CDF, DEFAULT_DO_SQUARE_SPLIT_CDF,
        DEFAULT_DO_UNEVEN_4WAY_PARTITION_CDF, DEFAULT_RECT_TYPE_CDF,
    };

    use super::super::{FrameCdfSubset, TileCdfError, TileCdfSelector};
    use super::*;

    const BLOCK_4X4: usize = 0;
    const BLOCK_16X16: usize = 6;
    const BLOCK_32X32: usize = 9;
    const BLOCK_256X256: usize = BLOCK_256X256_INDEX;

    #[test]
    fn derives_square_split_contexts_from_availability_gated_grid_neighbors() {
        let row0 = [BLOCK_256X256, BLOCK_4X4];
        let row1 = [BLOCK_4X4, BLOCK_256X256];
        let plane0 = [&row0[..], &row1[..]];
        let plane1 = [&row0[..], &row1[..]];
        let mi_sizes = [&plane0[..], &plane1[..]];

        assert_eq!(
            SquareSplitContextInput::new(BLOCK_16X16, 0, 1, 1, true, true, mi_sizes)
                .unwrap()
                .do_square_split_selector()
                .unwrap(),
            TileCdfSelector::DoSquareSplit {
                plane_start: 0,
                ctx: 3,
            }
        );
        assert_eq!(
            SquareSplitContextInput::new(BLOCK_16X16, 0, 1, 1, true, false, mi_sizes)
                .unwrap()
                .do_square_split_selector()
                .unwrap(),
            TileCdfSelector::DoSquareSplit {
                plane_start: 0,
                ctx: 1,
            }
        );
        assert_eq!(
            SquareSplitContextInput::new(BLOCK_16X16, 0, 1, 1, false, true, mi_sizes)
                .unwrap()
                .do_square_split_selector()
                .unwrap(),
            TileCdfSelector::DoSquareSplit {
                plane_start: 0,
                ctx: 2,
            }
        );

        let empty_plane: [&[usize]; 0] = [];
        let empty_mi_sizes = [&empty_plane[..], &empty_plane[..]];
        assert_eq!(
            SquareSplitContextInput::new(BLOCK_16X16, 0, 0, 0, false, false, empty_mi_sizes,)
                .unwrap()
                .do_square_split_selector()
                .unwrap(),
            TileCdfSelector::DoSquareSplit {
                plane_start: 0,
                ctx: 0,
            }
        );
    }

    #[test]
    fn derives_square_split_block_256_bonus_contexts() {
        let row0 = [BLOCK_256X256, BLOCK_4X4];
        let row1 = [BLOCK_4X4, BLOCK_256X256];
        let plane0 = [&row0[..], &row1[..]];
        let plane1 = [&row0[..], &row1[..]];
        let mi_sizes = [&plane0[..], &plane1[..]];

        assert_eq!(
            SquareSplitContextInput::new(BLOCK_256X256, 0, 1, 1, false, false, mi_sizes)
                .unwrap()
                .do_square_split_selector()
                .unwrap(),
            TileCdfSelector::DoSquareSplit {
                plane_start: 0,
                ctx: 4,
            }
        );
        assert_eq!(
            SquareSplitContextInput::new(BLOCK_256X256, 0, 1, 1, true, true, mi_sizes)
                .unwrap()
                .do_square_split_selector()
                .unwrap(),
            TileCdfSelector::DoSquareSplit {
                plane_start: 0,
                ctx: 7,
            }
        );
    }

    #[test]
    fn derives_do_split_and_rect_type_contexts_from_neighbors() {
        let left0 = [BLOCK_4X4];
        let left1 = [BLOCK_256X256];
        let above0 = [BLOCK_4X4];
        let above1 = [BLOCK_256X256];
        let input =
            PartitionContextInput::new(BLOCK_16X16, 0, 0, 0, [&left0, &left1], [&above0, &above1])
                .unwrap();

        assert_eq!(
            input.do_split_selector().unwrap(),
            TileCdfSelector::DoSplit {
                plane_start: 0,
                ctx: 7,
            }
        );
        assert_eq!(
            input.rect_type_selector().unwrap(),
            TileCdfSelector::RectType {
                plane_start: 0,
                ctx: 3,
            }
        );
    }

    #[test]
    fn derives_horizontal_ext_and_uneven_contexts_from_left_neighbors() {
        let mut left0 = [BLOCK_256X256; 8];
        let left1 = [BLOCK_256X256; 8];
        let above0 = [BLOCK_256X256; 8];
        let above1 = [BLOCK_256X256; 8];
        left0[1] = BLOCK_16X16;
        left0[5] = BLOCK_4X4;
        let input =
            PartitionContextInput::new(BLOCK_32X32, 0, 1, 0, [&left0, &left1], [&above0, &above1])
                .unwrap();

        assert_eq!(
            input
                .do_ext_partition_selector(RectPartitionType::Horz)
                .unwrap(),
            TileCdfSelector::DoExtPartition {
                plane_start: 0,
                ctx: 9,
            }
        );
        assert_eq!(
            input
                .do_uneven_4way_partition_selector(RectPartitionType::Horz)
                .unwrap(),
            TileCdfSelector::DoUneven4WayPartition {
                plane_start: 0,
                ctx: 9,
            }
        );
    }

    #[test]
    fn derives_vertical_ext_context_from_above_neighbors() {
        let left0 = [BLOCK_256X256; 8];
        let left1 = [BLOCK_256X256; 8];
        let mut above0 = [BLOCK_256X256; 8];
        let above1 = [BLOCK_256X256; 8];
        above0[2] = BLOCK_16X16;
        above0[6] = BLOCK_4X4;
        let input =
            PartitionContextInput::new(BLOCK_32X32, 0, 0, 2, [&left0, &left1], [&above0, &above1])
                .unwrap();

        assert_eq!(
            input
                .do_ext_partition_selector(RectPartitionType::Vert)
                .unwrap(),
            TileCdfSelector::DoExtPartition {
                plane_start: 0,
                ctx: 9,
            }
        );
    }

    #[test]
    fn rejects_invalid_context_inputs_before_table_use() {
        let empty: [usize; 0] = [];
        let err =
            PartitionContextInput::new(BLOCK_SIZES, 0, 0, 0, [&empty, &empty], [&empty, &empty])
                .unwrap_err();
        assert_eq!(
            err,
            TileCdfError::BlockSizeOutOfRange {
                table: "bSize",
                b_size: BLOCK_SIZES,
                max_exclusive: BLOCK_SIZES,
            }
        );

        let left0 = [BLOCK_4X4];
        let left1 = [BLOCK_4X4];
        let above0 = [BLOCK_4X4];
        let above1 = [BLOCK_4X4];
        let err =
            PartitionContextInput::new(BLOCK_16X16, 2, 0, 0, [&left0, &left1], [&above0, &above1])
                .unwrap()
                .do_split_selector()
                .unwrap_err();
        assert_eq!(
            err,
            TileCdfError::SelectorOutOfRange {
                array: TileCdfArray::DoSplit,
                index_name: "plane_start",
                actual: 2,
                max_exclusive: DO_SPLIT_PLANE_CONTEXTS,
            }
        );

        let err =
            PartitionContextInput::new(BLOCK_16X16, 0, 1, 0, [&left0, &left1], [&above0, &above1])
                .unwrap()
                .do_split_selector()
                .unwrap_err();
        assert_eq!(
            err,
            TileCdfError::PartitionNeighborOutOfRange {
                array: "LeftMiSizes",
                plane_start: 0,
                index: 1,
                len: 1,
            }
        );
    }

    #[test]
    fn rejects_square_split_invalid_inputs_before_grid_table_use() {
        let empty_plane: [&[usize]; 0] = [];
        let empty_mi_sizes = [&empty_plane[..], &empty_plane[..]];
        let err = SquareSplitContextInput::new(BLOCK_SIZES, 0, 0, 0, false, false, empty_mi_sizes)
            .unwrap_err();
        assert_eq!(
            err,
            TileCdfError::BlockSizeOutOfRange {
                table: "bSize",
                b_size: BLOCK_SIZES,
                max_exclusive: BLOCK_SIZES,
            }
        );

        let err = SquareSplitContextInput::new(BLOCK_16X16, 1, 0, 0, false, false, empty_mi_sizes)
            .unwrap()
            .do_square_split_selector()
            .unwrap_err();
        assert_eq!(
            err,
            TileCdfError::SelectorOutOfRange {
                array: TileCdfArray::DoSquareSplit,
                index_name: "plane_start",
                actual: 1,
                max_exclusive: 1,
            }
        );

        let err = SquareSplitContextInput::new(BLOCK_16X16, 0, 0, 0, true, false, empty_mi_sizes)
            .unwrap()
            .do_square_split_selector()
            .unwrap_err();
        assert_eq!(
            err,
            TileCdfError::PartitionGridCoordinateUnderflow {
                array: "MiSizes",
                plane_start: 0,
                coordinate: "r",
                actual: 0,
                subtract: 1,
            }
        );

        let err = SquareSplitContextInput::new(BLOCK_16X16, 0, 0, 0, false, true, empty_mi_sizes)
            .unwrap()
            .do_square_split_selector()
            .unwrap_err();
        assert_eq!(
            err,
            TileCdfError::PartitionGridCoordinateUnderflow {
                array: "MiSizes",
                plane_start: 0,
                coordinate: "c",
                actual: 0,
                subtract: 1,
            }
        );
    }

    #[test]
    fn rejects_square_split_missing_grid_cells_and_invalid_block_sizes() {
        let empty_plane: [&[usize]; 0] = [];
        let one_cell_row = [BLOCK_4X4];
        let one_cell_plane = [&one_cell_row[..]];
        let err = SquareSplitContextInput::new(
            BLOCK_16X16,
            0,
            1,
            0,
            true,
            false,
            [&empty_plane[..], &empty_plane[..]],
        )
        .unwrap()
        .do_square_split_selector()
        .unwrap_err();
        assert_eq!(
            err,
            TileCdfError::PartitionGridRowOutOfRange {
                array: "MiSizes",
                plane_start: 0,
                row: 0,
                rows: 0,
            }
        );

        let err = SquareSplitContextInput::new(
            BLOCK_16X16,
            0,
            1,
            1,
            true,
            false,
            [&one_cell_plane[..], &one_cell_plane[..]],
        )
        .unwrap()
        .do_square_split_selector()
        .unwrap_err();
        assert_eq!(
            err,
            TileCdfError::PartitionGridColumnOutOfRange {
                array: "MiSizes",
                plane_start: 0,
                row: 0,
                col: 1,
                cols: 1,
            }
        );

        let invalid_row = [BLOCK_4X4, BLOCK_SIZES];
        let row1 = [BLOCK_4X4, BLOCK_4X4];
        let invalid_plane = [&invalid_row[..], &row1[..]];
        let err = SquareSplitContextInput::new(
            BLOCK_16X16,
            0,
            1,
            1,
            true,
            false,
            [&invalid_plane[..], &invalid_plane[..]],
        )
        .unwrap()
        .do_square_split_selector()
        .unwrap_err();
        assert_eq!(
            err,
            TileCdfError::PartitionGridBlockSizeOutOfRange {
                array: "MiSizes",
                plane_start: 0,
                row: 0,
                col: 1,
                block_size: BLOCK_SIZES,
                max_exclusive: BLOCK_SIZES,
            }
        );
    }

    #[test]
    fn rejects_second_half_and_neighbor_block_size_bounds() {
        let left0 = [BLOCK_4X4; 4];
        let left1 = [BLOCK_4X4; 4];
        let above0 = [BLOCK_4X4; 4];
        let above1 = [BLOCK_4X4; 4];
        let err =
            PartitionContextInput::new(BLOCK_32X32, 0, 0, 0, [&left0, &left1], [&above0, &above1])
                .unwrap()
                .do_ext_partition_selector(RectPartitionType::Vert)
                .unwrap_err();
        assert_eq!(
            err,
            TileCdfError::PartitionNeighborOutOfRange {
                array: "AboveMiSizes",
                plane_start: 0,
                index: 4,
                len: 4,
            }
        );

        let bad_left0 = [BLOCK_SIZES];
        let err = PartitionContextInput::new(
            BLOCK_16X16,
            0,
            0,
            0,
            [&bad_left0, &left1],
            [&above0, &above1],
        )
        .unwrap()
        .do_split_selector()
        .unwrap_err();
        assert_eq!(
            err,
            TileCdfError::PartitionNeighborBlockSizeOutOfRange {
                array: "LeftMiSizes",
                plane_start: 0,
                index: 0,
                block_size: BLOCK_SIZES,
                max_exclusive: BLOCK_SIZES,
            }
        );
    }

    #[test]
    fn derived_selectors_index_generated_default_rows() {
        let left0 = [BLOCK_4X4; 8];
        let left1 = [BLOCK_256X256; 8];
        let above0 = [BLOCK_4X4; 8];
        let above1 = [BLOCK_256X256; 8];
        let square_row0 = [BLOCK_256X256, BLOCK_4X4];
        let square_row1 = [BLOCK_4X4, BLOCK_256X256];
        let square_plane0 = [&square_row0[..], &square_row1[..]];
        let square_plane1 = [&square_row0[..], &square_row1[..]];
        let mi_sizes = [&square_plane0[..], &square_plane1[..]];
        let frame = FrameCdfSubset::from_defaults();
        let tile = frame.tile_copy();

        let input =
            PartitionContextInput::new(BLOCK_16X16, 0, 0, 0, [&left0, &left1], [&above0, &above1])
                .unwrap();
        assert_eq!(
            tile.row(input.do_split_selector().unwrap()).unwrap(),
            DEFAULT_DO_SPLIT_CDF[0][7].as_slice()
        );
        assert_eq!(
            tile.row(input.rect_type_selector().unwrap()).unwrap(),
            DEFAULT_RECT_TYPE_CDF[0][3].as_slice()
        );
        let square_input =
            SquareSplitContextInput::new(BLOCK_16X16, 0, 1, 1, true, true, mi_sizes).unwrap();
        assert_eq!(
            tile.row(square_input.do_square_split_selector().unwrap())
                .unwrap(),
            DEFAULT_DO_SQUARE_SPLIT_CDF[0][3].as_slice()
        );

        let input =
            PartitionContextInput::new(BLOCK_32X32, 0, 0, 0, [&left0, &left1], [&above0, &above1])
                .unwrap();
        assert_eq!(
            tile.row(
                input
                    .do_ext_partition_selector(RectPartitionType::Horz)
                    .unwrap()
            )
            .unwrap(),
            DEFAULT_DO_EXT_PARTITION_CDF[0][11].as_slice()
        );
        assert_eq!(
            tile.row(
                input
                    .do_uneven_4way_partition_selector(RectPartitionType::Horz)
                    .unwrap()
            )
            .unwrap(),
            DEFAULT_DO_UNEVEN_4WAY_PARTITION_CDF[0][11].as_slice()
        );
    }
}

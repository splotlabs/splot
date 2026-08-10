// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::headers::sequence::ChromaFormatIdc;
use splot_core::tables::conversion::{NUM_4X4_BLOCKS_HIGH, NUM_4X4_BLOCKS_WIDE};
use splot_recon::{BitDepth, PlaneId};

use crate::bitstream::tile_payload::BlockSize;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ChromaSampling {
    Monochrome,
    Yuv420,
    Yuv422,
    Yuv444,
}

impl ChromaSampling {
    pub(crate) const fn from_chroma_format_idc(chroma: ChromaFormatIdc) -> Self {
        match chroma {
            ChromaFormatIdc::Monochrome => Self::Monochrome,
            ChromaFormatIdc::Yuv420 => Self::Yuv420,
            ChromaFormatIdc::Yuv422 => Self::Yuv422,
            ChromaFormatIdc::Yuv444 => Self::Yuv444,
        }
    }

    pub(crate) const fn subsampling(self, plane: PlaneId) -> (u32, u32) {
        match (self, plane) {
            (_, PlaneId::Y) | (Self::Yuv444, PlaneId::U | PlaneId::V) => (0, 0),
            (Self::Monochrome | Self::Yuv420, PlaneId::U | PlaneId::V) => (1, 1),
            (Self::Yuv422, PlaneId::U | PlaneId::V) => (1, 0),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BlockRect {
    row4: usize,
    col4: usize,
    width4: usize,
    height4: usize,
}

impl BlockRect {
    pub(crate) const fn new(row4: usize, col4: usize, width4: usize, height4: usize) -> Self {
        Self {
            row4,
            col4,
            width4,
            height4,
        }
    }

    pub(crate) const fn row4(self) -> usize {
        self.row4
    }

    pub(crate) const fn col4(self) -> usize {
        self.col4
    }

    pub(crate) const fn width4(self) -> usize {
        self.width4
    }

    pub(crate) const fn height4(self) -> usize {
        self.height4
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TxShape {
    width_log2: u32,
    height_log2: u32,
}

impl TxShape {
    pub(crate) fn from_av2_block_size(block_size: BlockSize) -> Self {
        let width4 = NUM_4X4_BLOCKS_WIDE[block_size.index()];
        let height4 = NUM_4X4_BLOCKS_HIGH[block_size.index()];
        Self {
            width_log2: width4.trailing_zeros() + 2,
            height_log2: height4.trailing_zeros() + 2,
        }
    }

    pub(crate) fn from_luma_4x4(width4: usize, height4: usize) -> Option<Self> {
        if width4 == 0 || height4 == 0 || !width4.is_power_of_two() || !height4.is_power_of_two() {
            return None;
        }
        Some(Self {
            width_log2: width4.trailing_zeros() + 2,
            height_log2: height4.trailing_zeros() + 2,
        })
    }

    pub(crate) const fn width_log2(self) -> u32 {
        self.width_log2
    }

    pub(crate) const fn height_log2(self) -> u32 {
        self.height_log2
    }

    pub(crate) const fn width4(self) -> usize {
        1usize << (self.width_log2 - 2)
    }

    pub(crate) const fn height4(self) -> usize {
        1usize << (self.height_log2 - 2)
    }

    pub(crate) const fn subsampled(self, sub_x: u32, sub_y: u32) -> Self {
        let width_log2 = self.width_log2.saturating_sub(sub_x);
        let height_log2 = self.height_log2.saturating_sub(sub_y);
        Self {
            width_log2: if width_log2 < 2 { 2 } else { width_log2 },
            height_log2: if height_log2 < 2 { 2 } else { height_log2 },
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PlaneBlock {
    x: usize,
    y: usize,
    width4: usize,
    height4: usize,
    tx: TxShape,
}

impl PlaneBlock {
    pub(crate) const fn x(self) -> usize {
        self.x
    }

    pub(crate) const fn y(self) -> usize {
        self.y
    }

    pub(crate) const fn width4(self) -> usize {
        self.width4
    }

    pub(crate) const fn height4(self) -> usize {
        self.height4
    }

    pub(crate) const fn tx(self) -> TxShape {
        self.tx
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NeighbourAvailability {
    has_above: bool,
    has_left: bool,
    num_above_right: usize,
    num_below_left: usize,
}

impl NeighbourAvailability {
    pub(crate) const fn without_corners(self) -> Self {
        Self {
            num_above_right: 0,
            num_below_left: 0,
            ..self
        }
    }

    pub(crate) const fn new(
        has_above: bool,
        has_left: bool,
        num_above_right: usize,
        num_below_left: usize,
    ) -> Self {
        Self {
            has_above,
            has_left,
            num_above_right,
            num_below_left,
        }
    }

    pub(crate) const fn has_above(self) -> bool {
        self.has_above
    }

    pub(crate) const fn has_left(self) -> bool {
        self.has_left
    }

    pub(crate) const fn num_above_right(self) -> usize {
        self.num_above_right
    }

    pub(crate) const fn num_below_left(self) -> usize {
        self.num_below_left
    }
}

const fn normalize_intra_corner_counts(
    plane: PlaneId,
    width_log2: u32,
    height_log2: u32,
    num_above_right: usize,
    num_below_left: usize,
) -> (usize, usize) {
    match plane {
        PlaneId::Y => (num_above_right, num_below_left),
        PlaneId::U | PlaneId::V => (
            if width_log2 > 5 { 0 } else { num_above_right },
            if height_log2 > 5 { 0 } else { num_below_left },
        ),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BlockCtx {
    block: BlockRect,
    tx: TxShape,
    chroma_ref: Option<(BlockRect, TxShape)>,
    frame_mi_cols: usize,
    frame_mi_rows: usize,
    tile_mi_col_start: usize,
    tile_mi_col_end: usize,
    tile_mi_row_start: usize,
    tile_mi_row_end: usize,
    bit_depth: BitDepth,
    chroma: ChromaSampling,
}

impl BlockCtx {
    pub(crate) const fn new(
        block: BlockRect,
        tx: TxShape,
        frame_mi_cols: usize,
        frame_mi_rows: usize,
        bit_depth: BitDepth,
        chroma: ChromaSampling,
    ) -> Self {
        Self {
            block,
            tx,
            chroma_ref: None,
            frame_mi_cols,
            frame_mi_rows,
            tile_mi_col_start: 0,
            tile_mi_col_end: frame_mi_cols,
            tile_mi_row_start: 0,
            tile_mi_row_end: frame_mi_rows,
            bit_depth,
            chroma,
        }
    }

    pub(crate) const fn block(self) -> BlockRect {
        self.block
    }

    pub(crate) const fn bit_depth(self) -> BitDepth {
        self.bit_depth
    }

    pub(crate) const fn frame_mi_cols(self) -> usize {
        self.frame_mi_cols
    }

    pub(crate) const fn frame_mi_rows(self) -> usize {
        self.frame_mi_rows
    }

    pub(crate) fn with_tile_bounds(
        mut self,
        row_start: usize,
        row_end: usize,
        col_start: usize,
        col_end: usize,
    ) -> Self {
        self.tile_mi_row_start = row_start.min(self.frame_mi_rows);
        self.tile_mi_row_end = row_end.min(self.frame_mi_rows).max(self.tile_mi_row_start);
        self.tile_mi_col_start = col_start.min(self.frame_mi_cols);
        self.tile_mi_col_end = col_end.min(self.frame_mi_cols).max(self.tile_mi_col_start);
        self
    }

    pub(crate) const fn with_tile_bounds_from(mut self, other: Self) -> Self {
        self.tile_mi_col_start = other.tile_mi_col_start;
        self.tile_mi_col_end = other.tile_mi_col_end;
        self.tile_mi_row_start = other.tile_mi_row_start;
        self.tile_mi_row_end = other.tile_mi_row_end;
        self
    }

    pub(crate) const fn chroma(self) -> ChromaSampling {
        self.chroma
    }

    pub(crate) const fn with_chroma_ref(mut self, block: BlockRect, tx: TxShape) -> Self {
        self.chroma_ref = Some((block, tx));
        self
    }

    pub(crate) const fn chroma_ref(self) -> Option<(BlockRect, TxShape)> {
        self.chroma_ref
    }

    pub(crate) fn plane_block(self, plane: PlaneId) -> PlaneBlock {
        let (block, tx) = self.plane_geometry(plane);
        let (sub_x, sub_y) = self.chroma.subsampling(plane);
        PlaneBlock {
            x: (block.col4 * 4) >> sub_x,
            y: (block.row4 * 4) >> sub_y,
            width4: (block.width4 >> sub_x).max(1),
            height4: (block.height4 >> sub_y).max(1),
            tx: tx.subsampled(sub_x, sub_y),
        }
    }

    pub(crate) fn neighbours(self, plane: PlaneId) -> NeighbourAvailability {
        let (block, _) = self.plane_geometry(plane);
        let plane_block = self.plane_block(plane);
        let (num_above_right, num_below_left) = normalize_intra_corner_counts(
            plane,
            plane_block.tx().width_log2(),
            plane_block.tx().height_log2(),
            self.uncapped_num_above_right(plane),
            0,
        );
        NeighbourAvailability::new(
            block.row4 > self.tile_mi_row_start,
            block.col4 > self.tile_mi_col_start,
            num_above_right,
            num_below_left,
        )
    }

    pub(crate) fn neighbours_from_block_decoded(
        self,
        plane: PlaneId,
        block_decoded: &crate::bitstream::tile_payload::TileBlockDecodedState,
    ) -> NeighbourAvailability {
        let (block, _) = self.plane_geometry(plane);
        let plane_block = self.plane_block(plane);
        let (sub_x, sub_y) = self.chroma.subsampling(plane);
        let sb_mask = block_decoded.sb_size4().saturating_sub(1);
        let x4 = (block.col4 & sb_mask) >> sub_x;
        let y4 = (block.row4 & sb_mask) >> sub_y;
        let num_above_right =
            block_decoded.count_top_right_avail(plane.index(), x4, y4, plane_block.width4());
        let num_below_left =
            block_decoded.count_bottom_left_avail(plane.index(), x4, y4, plane_block.height4());
        let (num_above_right, num_below_left) = normalize_intra_corner_counts(
            plane,
            plane_block.tx().width_log2(),
            plane_block.tx().height_log2(),
            num_above_right,
            num_below_left,
        );
        NeighbourAvailability::new(
            block.row4 > self.tile_mi_row_start,
            block.col4 > self.tile_mi_col_start,
            num_above_right,
            num_below_left,
        )
    }

    fn plane_geometry(self, plane: PlaneId) -> (BlockRect, TxShape) {
        match (plane, self.chroma_ref) {
            (PlaneId::Y, _) | (_, None) => (self.block, self.tx),
            (PlaneId::U | PlaneId::V, Some(chroma_ref)) => chroma_ref,
        }
    }

    fn uncapped_num_above_right(self, plane: PlaneId) -> usize {
        let (block, _) = self.plane_geometry(plane);
        let plane_block = self.plane_block(plane);
        let (sub_x, _) = self.chroma.subsampling(plane);
        let above_decoded_cols = self.tile_mi_col_end.saturating_sub(block.col4) >> sub_x;
        above_decoded_cols
            .saturating_sub(plane_block.width4())
            .min(plane_block.width4())
    }
}

#[cfg(test)]
#[path = "block_context_tests.rs"]
mod tests;

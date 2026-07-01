// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_recon::{BitDepth, PlaneId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ChromaSampling {
    Yuv420,
}

impl ChromaSampling {
    const fn subsampling(self, plane: PlaneId) -> (u32, u32) {
        match (self, plane) {
            (_, PlaneId::Y) => (0, 0),
            (Self::Yuv420, PlaneId::U | PlaneId::V) => (1, 1),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct BlockRect {
    row4: usize,
    col4: usize,
    width4: usize,
    height4: usize,
}

impl BlockRect {
    pub(super) const fn new(row4: usize, col4: usize, width4: usize, height4: usize) -> Self {
        Self {
            row4,
            col4,
            width4,
            height4,
        }
    }

    pub(super) const fn row4(self) -> usize {
        self.row4
    }

    pub(super) const fn col4(self) -> usize {
        self.col4
    }

    pub(super) const fn width4(self) -> usize {
        self.width4
    }

    pub(super) const fn height4(self) -> usize {
        self.height4
    }

    pub(super) const fn has_above(self) -> bool {
        self.row4 != 0
    }

    pub(super) const fn has_left(self) -> bool {
        self.col4 != 0
    }

    pub(super) const fn is_top_left(self) -> bool {
        !self.has_above() && !self.has_left()
    }

    pub(super) fn is_row_aligned_to(self, size4: usize) -> bool {
        size4 != 0 && self.row4.is_multiple_of(size4)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct TxShape {
    width_log2: u32,
    height_log2: u32,
}

impl TxShape {
    pub(super) fn from_luma_4x4(width4: usize, height4: usize) -> Option<Self> {
        if width4 == 0 || height4 == 0 || !width4.is_power_of_two() || !height4.is_power_of_two() {
            return None;
        }
        Some(Self {
            width_log2: width4.trailing_zeros() + 2,
            height_log2: height4.trailing_zeros() + 2,
        })
    }

    pub(super) const fn width_log2(self) -> u32 {
        self.width_log2
    }

    pub(super) const fn height_log2(self) -> u32 {
        self.height_log2
    }

    pub(super) const fn width4(self) -> usize {
        1usize << (self.width_log2 - 2)
    }

    pub(super) const fn height4(self) -> usize {
        1usize << (self.height_log2 - 2)
    }

    pub(super) const fn is_square(self) -> bool {
        self.width_log2 == self.height_log2
    }

    pub(super) const fn square_tx_index(self) -> Option<usize> {
        if self.is_square() && self.width_log2 >= 2 {
            Some((self.width_log2 - 2) as usize)
        } else {
            None
        }
    }

    pub(super) const fn subsampled(self, sub_x: u32, sub_y: u32) -> Self {
        Self {
            width_log2: self.width_log2.saturating_sub(sub_x),
            height_log2: self.height_log2.saturating_sub(sub_y),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct PlaneBlock {
    x: usize,
    y: usize,
    width4: usize,
    height4: usize,
    tx: TxShape,
}

impl PlaneBlock {
    pub(super) const fn x(self) -> usize {
        self.x
    }

    pub(super) const fn y(self) -> usize {
        self.y
    }

    pub(super) const fn width4(self) -> usize {
        self.width4
    }

    pub(super) const fn height4(self) -> usize {
        self.height4
    }

    pub(super) const fn tx(self) -> TxShape {
        self.tx
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct NeighbourAvailability {
    has_above: bool,
    has_left: bool,
    num_above_right: usize,
    num_below_left: usize,
}

impl NeighbourAvailability {
    const fn new(
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

    pub(super) const fn has_above(self) -> bool {
        self.has_above
    }

    pub(super) const fn has_left(self) -> bool {
        self.has_left
    }

    pub(super) const fn is_top_left(self) -> bool {
        !self.has_above && !self.has_left
    }

    pub(super) const fn num_above_right(self) -> usize {
        self.num_above_right
    }

    pub(super) const fn num_below_left(self) -> usize {
        self.num_below_left
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct BlockCtx {
    block: BlockRect,
    tx: TxShape,
    chroma_ref: Option<(BlockRect, TxShape)>,
    frame_mi_cols: usize,
    frame_mi_rows: usize,
    bit_depth: BitDepth,
    chroma: ChromaSampling,
}

impl BlockCtx {
    pub(super) const fn new(
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
            bit_depth,
            chroma,
        }
    }

    pub(super) const fn block(self) -> BlockRect {
        self.block
    }

    pub(super) const fn bit_depth(self) -> BitDepth {
        self.bit_depth
    }

    pub(super) const fn frame_mi_cols(self) -> usize {
        self.frame_mi_cols
    }

    pub(super) const fn frame_mi_rows(self) -> usize {
        self.frame_mi_rows
    }

    pub(super) const fn chroma(self) -> ChromaSampling {
        self.chroma
    }

    pub(super) const fn with_chroma_ref(mut self, block: BlockRect, tx: TxShape) -> Self {
        self.chroma_ref = Some((block, tx));
        self
    }

    pub(super) const fn is_top_left(self) -> bool {
        self.block.is_top_left()
    }

    pub(super) fn plane_block(self, plane: PlaneId) -> PlaneBlock {
        let (block, tx) = self.plane_geometry(plane);
        let (sub_x, sub_y) = self.chroma.subsampling(plane);
        PlaneBlock {
            x: (block.col4 * 4) >> sub_x,
            y: (block.row4 * 4) >> sub_y,
            width4: block.width4 >> sub_x,
            height4: block.height4 >> sub_y,
            tx: tx.subsampled(sub_x, sub_y),
        }
    }

    pub(super) fn neighbours(self, plane: PlaneId) -> NeighbourAvailability {
        let (block, _) = self.plane_geometry(plane);
        let plane_block = self.plane_block(plane);
        let (sub_x, _) = self.chroma.subsampling(plane);
        let above_decoded_cols = self.frame_mi_cols.saturating_sub(block.col4) >> sub_x;
        let num_above_right = above_decoded_cols
            .saturating_sub(plane_block.width4())
            .min(plane_block.width4());
        NeighbourAvailability::new(block.has_above(), block.has_left(), num_above_right, 0)
    }

    pub(super) fn neighbours_from_block_decoded(
        self,
        plane: PlaneId,
        block_decoded: &crate::tile_payload::TileBlockDecodedState,
    ) -> NeighbourAvailability {
        let (block, _) = self.plane_geometry(plane);
        let plane_block = self.plane_block(plane);
        let (sub_x, sub_y) = self.chroma.subsampling(plane);
        let sb_mask = block_decoded.sb_size4().saturating_sub(1);
        let x4 = (block.col4 & sb_mask) >> sub_x;
        let y4 = (block.row4 & sb_mask) >> sub_y;
        NeighbourAvailability::new(
            block.has_above(),
            block.has_left(),
            block_decoded.count_top_right_avail(plane.index(), x4, y4, plane_block.width4()),
            block_decoded.count_bottom_left_avail(plane.index(), x4, y4, plane_block.height4()),
        )
    }

    fn plane_geometry(self, plane: PlaneId) -> (BlockRect, TxShape) {
        match (plane, self.chroma_ref) {
            (PlaneId::Y, _) | (_, None) => (self.block, self.tx),
            (PlaneId::U | PlaneId::V, Some(chroma_ref)) => chroma_ref,
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    fn ctx(row4: usize, col4: usize, width4: usize, height4: usize) -> BlockCtx {
        let rect = BlockRect::new(row4, col4, width4, height4);
        BlockCtx::new(
            rect,
            TxShape::from_luma_4x4(width4, height4).expect("valid test transform"),
            32,
            32,
            BitDepth::Eight,
            ChromaSampling::Yuv420,
        )
    }

    #[test]
    fn classifies_frame_edges() {
        let cases = [
            (ctx(0, 0, 16, 16), false, false, 16),
            (ctx(0, 16, 16, 16), false, true, 0),
            (ctx(16, 0, 16, 16), true, false, 16),
            (ctx(16, 16, 16, 16), true, true, 0),
            (ctx(8, 8, 8, 8), true, true, 8),
            (ctx(24, 8, 8, 8), true, true, 8),
        ];

        for (ctx, has_above, has_left, above_right) in cases {
            let neighbours = ctx.neighbours(PlaneId::Y);
            assert_eq!(neighbours.has_above(), has_above);
            assert_eq!(neighbours.has_left(), has_left);
            assert_eq!(neighbours.num_above_right(), above_right);
            assert_eq!(neighbours.num_below_left(), 0);
        }
    }

    #[test]
    fn plane_blocks_scale_luma_and_420_chroma() {
        let ctx = ctx(8, 16, 16, 8);

        let y = ctx.plane_block(PlaneId::Y);
        assert_eq!((y.x(), y.y()), (64, 32));
        assert_eq!((y.width4(), y.height4()), (16, 8));
        assert_eq!((y.tx().width_log2(), y.tx().height_log2()), (6, 5));

        let u = ctx.plane_block(PlaneId::U);
        assert_eq!((u.x(), u.y()), (32, 16));
        assert_eq!((u.width4(), u.height4()), (8, 4));
        assert_eq!((u.tx().width_log2(), u.tx().height_log2()), (5, 4));
    }

    #[test]
    fn plane_blocks_use_chroma_ref_geometry_for_420_chroma() {
        let chroma_ref = BlockRect::new(24, 206, 2, 4);
        let chroma_tx = TxShape::from_luma_4x4(2, 4).expect("valid chroma reference transform");
        let ctx = ctx(24, 207, 1, 4).with_chroma_ref(chroma_ref, chroma_tx);

        let y = ctx.plane_block(PlaneId::Y);
        assert_eq!((y.x(), y.y()), (828, 96));
        assert_eq!((y.width4(), y.height4()), (1, 4));

        let u = ctx.plane_block(PlaneId::U);
        assert_eq!((u.x(), u.y()), (412, 48));
        assert_eq!((u.width4(), u.height4()), (1, 2));
        assert_eq!((u.tx().width_log2(), u.tx().height_log2()), (2, 3));
    }

    #[test]
    fn block_decoded_neighbours_cover_subpartition_above_right() {
        let mut block_decoded =
            crate::tile_payload::TileBlockDecodedState::new(3, 1, 1, 16, 32, 32)
                .expect("valid block decoded state");
        block_decoded.clear_superblock(0, 0);
        block_decoded.set_block(0, 0, 8, 8, 8);

        let bottom_left = ctx(8, 0, 8, 8);
        let neighbours = bottom_left.neighbours_from_block_decoded(PlaneId::Y, &block_decoded);

        assert!(neighbours.has_above());
        assert!(!neighbours.has_left());
        assert_eq!(neighbours.num_above_right(), 8);
        assert_eq!(neighbours.num_below_left(), 0);
    }
}

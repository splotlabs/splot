// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Inter block filter-record handoff.

use splot_core::headers::sequence::ChromaFormatIdc;
use splot_core::tables::conversion::{TX_HEIGHT, TX_WIDTH};

use super::tile::TileFilterRecords;
#[allow(clippy::wildcard_imports)]
use super::*;
use crate::bitstream::tile_payload::ChromaRefGeometry;
use crate::filters::wienerns_lr::WienerNsLrTxSkipTransformRecord;
use crate::support::decode_buffers::fit_list;

/// One inter leaf's deblock and tx-skip geometry, which the parse queues and
/// the in-order resolve expands, so the record building leaves the parse.
pub(super) struct InterDeblockLeaf {
    row: u16,
    col: u16,
    size: BlockSize,
    chroma_ref: ChromaRefGeometry,
    has_chroma: bool,
    residual: Option<InterResidual>,
    sub_pu_size: Option<crate::filters::deblock::DeblockSubPuSize>,
    qindex: u16,
    lossless: bool,
}

impl InterDeblockLeaf {
    pub(super) fn new(
        frontier: &DecodeBlockFrontier,
        residual: Option<&InterResidual>,
        sub_pu_size: Option<crate::filters::deblock::DeblockSubPuSize>,
        qindex: u32,
        lossless: bool,
    ) -> Self {
        Self {
            row: frontier.r as u16,
            col: frontier.c as u16,
            size: frontier.b_size,
            chroma_ref: frontier.chroma_ref_geometry(),
            has_chroma: frontier.has_chroma,
            residual: residual.cloned(),
            sub_pu_size,
            qindex: qindex as u16,
            lossless,
        }
    }
}

impl TileFilterRecords {
    fn marks(&self) -> [u32; 3] {
        [
            self.deblock_blocks.len() as u32,
            self.chroma_deblock_blocks.len() as u32,
            self.tx_skip_records.len() as u32,
        ]
    }

    /// Queues an inter leaf's geometry for [`Self::publish_into`].
    pub(super) fn defer_inter_leaf(
        &mut self,
        frontier: &DecodeBlockFrontier,
        residual: Option<&InterResidual>,
        sub_pu_size: Option<crate::filters::deblock::DeblockSubPuSize>,
        qindex: u32,
        lossless: bool,
    ) {
        if !self.needs.any() {
            return;
        }
        if !self.needs.deblock && residual.is_none() {
            self.tx_skip_records
                .push(WienerNsLrTxSkipTransformRecord::skipped_block(
                    frontier.r as u16,
                    frontier.c as u16,
                    frontier.b_size,
                ));
            return;
        }
        self.inter_leaves.push((
            self.marks(),
            InterDeblockLeaf::new(frontier, residual, sub_pu_size, qindex, lossless),
        ));
    }

    /// Moves the unit's records onto the end of `target`'s lists in leaf
    /// order, expanding the queued inter leaves between the records the parse
    /// built, and leaves these lists empty with their capacity.
    pub(super) fn publish_into(
        &mut self,
        deblock_blocks: &mut Vec<crate::filters::deblock::DeblockBlock>,
        chroma_deblock_blocks: &mut crate::filters::deblock::ChromaDeblockRecords,
        tx_skip_records: &mut Vec<crate::filters::wienerns_lr::WienerNsLrTxSkipTransformRecord>,
        residual_blocks: &[InterResidualBlock],
        chroma_format: ChromaFormatIdc,
    ) -> Result<()> {
        let marks = self.marks();
        self.published = [
            marks[0] as usize,
            marks[1] as usize,
            marks[2] as usize,
            self.inter_leaves.len(),
        ];
        let mut published = [0; 3];
        for &(marks, ref leaf) in &self.inter_leaves {
            if published != marks {
                self.copy_parsed(
                    published..marks,
                    deblock_blocks,
                    chroma_deblock_blocks,
                    tx_skip_records,
                );
                published = marks;
            }
            leaf.record_into(
                deblock_blocks,
                chroma_deblock_blocks,
                tx_skip_records,
                self.needs,
                chroma_format,
                residual_blocks,
            )?;
        }
        if published == [0; 3] {
            deblock_blocks.append(&mut self.deblock_blocks);
            chroma_deblock_blocks.append(&mut self.chroma_deblock_blocks);
            tx_skip_records.append(&mut self.tx_skip_records);
        } else {
            self.copy_parsed(
                published..marks,
                deblock_blocks,
                chroma_deblock_blocks,
                tx_skip_records,
            );
            self.deblock_blocks.clear();
            self.chroma_deblock_blocks.clear();
            self.tx_skip_records.clear();
        }
        self.inter_leaves.clear();
        Ok(())
    }

    pub(super) fn clear(&mut self) {
        self.deblock_blocks.clear();
        self.chroma_deblock_blocks.clear();
        self.tx_skip_records.clear();
        self.inter_leaves.clear();
    }

    /// Sizes the emptied lists to the decode's typical unit; see
    /// [`fit_list`].
    pub(super) fn fit(&mut self, [luma, chroma, tx_skip, leaves]: [usize; 4]) {
        fit_list(&mut self.deblock_blocks, luma);
        fit_list(&mut self.chroma_deblock_blocks, chroma);
        fit_list(&mut self.tx_skip_records, tx_skip);
        fit_list(&mut self.inter_leaves, leaves);
    }

    fn copy_parsed(
        &self,
        range: core::ops::Range<[u32; 3]>,
        deblock_blocks: &mut Vec<crate::filters::deblock::DeblockBlock>,
        chroma_deblock_blocks: &mut crate::filters::deblock::ChromaDeblockRecords,
        tx_skip_records: &mut Vec<crate::filters::wienerns_lr::WienerNsLrTxSkipTransformRecord>,
    ) {
        let span = |index: usize| range.start[index] as usize..range.end[index] as usize;
        if let Some(blocks) = self.deblock_blocks.get(span(0)) {
            deblock_blocks.extend_from_slice(blocks);
        }
        if let Some(blocks) = self.chroma_deblock_blocks.get(span(1)) {
            chroma_deblock_blocks.extend_from_slice(blocks);
        }
        if let Some(records) = self.tx_skip_records.get(span(2)) {
            tx_skip_records.extend_from_slice(records);
        }
    }
}

impl InterDeblockLeaf {
    pub(super) fn record_into(
        &self,
        deblock_blocks: &mut Vec<crate::filters::deblock::DeblockBlock>,
        chroma_deblock_blocks: &mut crate::filters::deblock::ChromaDeblockRecords,
        tx_skip_records: &mut Vec<WienerNsLrTxSkipTransformRecord>,
        needs: crate::filters::wienerns_lr::FilterRecordNeeds,
        chroma_format: ChromaFormatIdc,
        residual_blocks: &[InterResidualBlock],
    ) -> Result<()> {
        let (row, col) = (usize::from(self.row), usize::from(self.col));
        let (residual, sub_pu_size, qindex, lossless) = (
            self.residual.as_ref(),
            self.sub_pu_size,
            self.qindex,
            self.lossless,
        );
        let (sub_x, sub_y) = chroma_subsampling(chroma_format);
        let chroma_subsampling = (u32::from(sub_x), u32::from(sub_y));
        let chroma_ref = self.chroma_ref;
        let luma_prediction = crate::filters::deblock::DeblockPredictionUnit::new(
            row,
            col,
            super::residual::max_tx_size(self.size.index())?,
        );
        let chroma_prediction = crate::filters::deblock::DeblockPredictionUnit::new(
            chroma_ref.row(),
            chroma_ref.col(),
            super::residual::max_tx_size(chroma_ref.size().index())?,
        );
        let inherited_chroma =
            chroma_ref.row() != row || chroma_ref.col() != col || chroma_ref.size() != self.size;
        let inherited_chroma_metadata = needs.deblock && self.has_chroma && inherited_chroma;
        if inherited_chroma_metadata {
            let chroma_plane_size = get_plane_residual_size(chroma_ref.size(), 1, sub_x, sub_y)
                .map_err(|_| super::residual::residual_geometry_error())?
                .ok_or_else(super::residual::residual_geometry_error)?;
            let chroma_tx =
                super::residual::fixed_inter_residual_tx_size(chroma_plane_size.index(), lossless)?;
            let [chroma_n4h, chroma_n4w] = chroma_ref.size().num_4x4();
            let block = crate::filters::deblock::DeblockBlock {
                r: chroma_ref.row() as u16,
                c: chroma_ref.col() as u16,
                n4w: chroma_n4w as u8,
                n4h: chroma_n4h as u8,
                tx: chroma_tx as u8,
                skip: residual.is_none(),
                lossless,
                qindex,
                prediction: chroma_prediction,
                sub_pu_size,
            };
            chroma_deblock_blocks.push(block);
        }
        let Some(residual) = residual else {
            if needs.tx_skip {
                tx_skip_records.push(WienerNsLrTxSkipTransformRecord::skipped_block(
                    self.row, self.col, self.size,
                ));
            }
            if !needs.deblock {
                return Ok(());
            }
            let tx_size =
                super::residual::fixed_inter_residual_tx_size(self.size.index(), lossless)?;
            let tx_w4 = super::residual::tx_size_dimension(&TX_WIDTH, tx_size)? / MI_SIZE;
            let tx_h4 = super::residual::tx_size_dimension(&TX_HEIGHT, tx_size)? / MI_SIZE;
            let [n4h, n4w] = self.size.num_4x4();
            for row4 in crate::support::step_range(0..n4h, tx_h4) {
                for col4 in crate::support::step_range(0..n4w, tx_w4) {
                    let luma = crate::filters::deblock::DeblockBlock {
                        r: (row + row4) as u16,
                        c: (col + col4) as u16,
                        n4w: tx_w4 as u8,
                        n4h: tx_h4 as u8,
                        tx: tx_size as u8,
                        skip: true,
                        lossless,
                        qindex,
                        prediction: luma_prediction,
                        sub_pu_size,
                    };
                    deblock_blocks.push(luma);
                }
            }
            if self.has_chroma {
                record_skipped_chroma_deblock_geometry(
                    chroma_deblock_blocks,
                    chroma_ref.row(),
                    chroma_ref.col(),
                    chroma_ref.size(),
                    (sub_x, sub_y),
                    chroma_prediction,
                    sub_pu_size,
                    qindex,
                    lossless,
                )?;
            }
            return Ok(());
        };
        let blocks = residual
            .blocks(residual_blocks)
            .ok_or(DecodeHeaderStateError::InvalidInterResidualReconstruction)?;
        for block in blocks {
            let (log2_width, log2_height) = super::super::inter_residual_log2(block)
                .map_err(|_| super::residual::residual_geometry_error())?;
            match block.plane {
                ReconPlaneId::Y => {
                    let tx_w4 = (1usize << log2_width) / MI_SIZE;
                    let tx_h4 = (1usize << log2_height) / MI_SIZE;
                    let luma = crate::filters::deblock::DeblockBlock {
                        r: block.y / MI_SIZE as u16,
                        c: block.x / MI_SIZE as u16,
                        n4w: tx_w4 as u8,
                        n4h: tx_h4 as u8,
                        tx: block.tx_size,
                        skip: false,
                        lossless,
                        qindex,
                        prediction: luma_prediction,
                        sub_pu_size,
                    };
                    needs.record_luma(deblock_blocks, tx_skip_records, luma, block.coeffs.eob());
                }
                ReconPlaneId::U if needs.deblock => {
                    if let Some(mut record) =
                        crate::filters::wienerns_lr::chroma_transform_deblock_block(
                            usize::from(block.x),
                            usize::from(block.y),
                            usize::from(block.tx_size),
                            Some((log2_width, log2_height)),
                            chroma_subsampling,
                            qindex,
                            lossless,
                        )
                    {
                        record.prediction = chroma_prediction;
                        record.sub_pu_size = sub_pu_size;
                        chroma_deblock_blocks.push(record);
                    }
                }
                ReconPlaneId::V | ReconPlaneId::U => {}
            }
        }
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
fn record_skipped_chroma_deblock_geometry(
    chroma_deblock_blocks: &mut crate::filters::deblock::ChromaDeblockRecords,
    chroma_ref_row: usize,
    chroma_ref_col: usize,
    chroma_ref_size: BlockSize,
    chroma_subsampling: (bool, bool),
    chroma_prediction: crate::filters::deblock::DeblockPredictionUnit,
    sub_pu_size: Option<crate::filters::deblock::DeblockSubPuSize>,
    qindex: u16,
    lossless: bool,
) -> Result<()> {
    let (sub_x, sub_y) = chroma_subsampling;
    let plane_size = get_plane_residual_size(chroma_ref_size, 1, sub_x, sub_y)
        .map_err(|_| super::residual::residual_geometry_error())?
        .ok_or_else(super::residual::residual_geometry_error)?;
    let chroma_tx = super::residual::fixed_inter_residual_tx_size(plane_size.index(), lossless)?;
    let width = plane_size
        .width_samples()
        .map_err(|_| super::residual::residual_geometry_error())?;
    let height = plane_size
        .height_samples()
        .map_err(|_| super::residual::residual_geometry_error())?;
    let tx_width = super::residual::tx_size_dimension(&TX_WIDTH, chroma_tx)?;
    let tx_height = super::residual::tx_size_dimension(&TX_HEIGHT, chroma_tx)?;
    let base_x = (chroma_ref_col >> usize::from(sub_x)) * MI_SIZE;
    let base_y = (chroma_ref_row >> usize::from(sub_y)) * MI_SIZE;
    let subsampling = (u32::from(sub_x), u32::from(sub_y));
    for y in crate::support::step_range(0..height, tx_height) {
        for x in crate::support::step_range(0..width, tx_width) {
            if let Some(mut record) = crate::filters::wienerns_lr::chroma_transform_deblock_block(
                base_x + x,
                base_y + y,
                chroma_tx,
                None,
                subsampling,
                qindex,
                lossless,
            ) {
                record.prediction = chroma_prediction;
                record.sub_pu_size = sub_pu_size;
                record.skip = true;
                chroma_deblock_blocks.push(record);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn skipped_64x128_420_chroma_keeps_one_transform_across_luma_chunks() {
        let mut records = crate::filters::deblock::ChromaDeblockRecords::default();
        let prediction = crate::filters::deblock::DeblockPredictionUnit::new(128, 96, 0);

        record_skipped_chroma_deblock_geometry(
            &mut records,
            128,
            96,
            BlockSize::new(13).unwrap(),
            (true, true),
            prediction,
            Some(crate::filters::deblock::DeblockSubPuSize::square(4)),
            200,
            false,
        )
        .unwrap();

        assert_eq!(records.len(), 1);
        let record = records[0];
        assert_eq!((record.r, record.c), (128, 96));
        assert_eq!((record.n4h, record.n4w), (32, 16));
        assert!(record.skip);
        assert_eq!(record.qindex, 200);
    }

    /// § 5.20.6.1 gives a skipped lossless inter block `TxSize = TX_4X4` and
    /// § 5.20.7.23 its chroma `TX_4X4`, so the § 7.17.4 filter size of a
    /// horizontal edge between two skipped 16x16 blocks is 4 next to a lossless
    /// block, inside a superblock (row 4) and on its edge (row 16), and 16
    /// (luma) and 8 (4:2:0 chroma) between two lossy blocks.
    #[test]
    fn skipped_lossless_inter_blocks_record_4x4_transforms_for_deblocking() {
        let size = BlockSize::new(6).unwrap();
        let tx_height = |tx: u8| TX_HEIGHT[usize::from(tx)] as usize;
        for row in [4, 16] {
            for (above, below, luma, chroma) in [
                (true, false, 4, 4),
                (false, true, 4, 4),
                (true, true, 4, 4),
                (false, false, 16, 8),
            ] {
                let mut luma_records = Vec::new();
                let mut chroma_records = crate::filters::deblock::ChromaDeblockRecords::new();
                for (r, lossless) in [(row - 4, above), (row, below)] {
                    let mut skips = Vec::new();
                    InterDeblockLeaf::new(
                        &DecodeBlockFrontier::shared_leaf(r, 0, size),
                        None,
                        None,
                        100,
                        lossless,
                    )
                    .record_into(
                        &mut luma_records,
                        &mut chroma_records,
                        &mut skips,
                        crate::filters::wienerns_lr::FilterRecordNeeds::ALL,
                        ChromaFormatIdc::Yuv420,
                        &[],
                    )
                    .unwrap();
                    assert_eq!(skips.len(), 1);
                    let skip = skips[0];
                    assert_eq!(
                        (usize::from(skip.row), skip.col, skip.rows, skip.cols),
                        (r, 0, 4, 4)
                    );
                    assert!(skip.skip_flag && skip.empty);
                }
                let size_at = |records: &[crate::filters::deblock::DeblockBlock], r: usize| {
                    let record = records
                        .iter()
                        .find(|b| usize::from(b.r) <= r && r < usize::from(b.r + u16::from(b.n4h)))
                        .unwrap();
                    (tx_height(record.tx), record.lossless)
                };
                let edge = |records: &[_]| {
                    let ((prev, prev_ll), (curr, curr_ll)) =
                        (size_at(records, row - 1), size_at(records, row));
                    assert_eq!((prev_ll, curr_ll), (above, below), "row {row}");
                    prev.min(curr)
                };
                assert_eq!(edge(&luma_records), luma, "luma row {row} {above} {below}");
                assert_eq!(
                    edge(&chroma_records),
                    chroma,
                    "chroma row {row} {above} {below}"
                );
            }
        }
    }

    #[test]
    fn skip_only_filter_preserves_order_across_coded_and_skipped_blocks() {
        let size = BlockSize::new(BLOCK_8X8).unwrap();
        let mut unit = TileFilterRecords::default();
        unit.needs.deblock = false;
        let block = InterResidualBlock {
            plane: ReconPlaneId::Y,
            x: 8,
            y: 0,
            tx_size: 0,
            cctx_pair_delta: 0,
            coeffs: crate::bitstream::tile_payload::LumaCoeffBlock::empty(0, false),
        };
        let coded = InterResidual::new(0..1);
        for (col, size, residual) in [
            (0, size, None),
            (2, BlockSize::new(0).unwrap(), Some(&coded)),
            (4, size, None),
        ] {
            let leaf = DecodeBlockFrontier::shared_leaf(0, col, size);
            unit.defer_inter_leaf(&leaf, residual, None, 40, false);
        }
        let mut frame = TileFilterRecords::default();
        unit.publish_into(
            &mut frame.deblock_blocks,
            &mut frame.chroma_deblock_blocks,
            &mut frame.tx_skip_records,
            &[block],
            ChromaFormatIdc::Yuv420,
        )
        .unwrap();
        let records: Vec<_> = frame
            .tx_skip_records
            .iter()
            .map(|r| (r.col, r.cols, r.skip_flag, r.empty))
            .collect();
        assert_eq!(
            records,
            [(0, 2, true, true), (2, 1, false, true), (4, 2, true, true)]
        );
        assert!(frame.deblock_blocks.is_empty() && frame.chroma_deblock_blocks.is_empty());
    }

    #[test]
    fn deferred_inter_leaves_publish_in_leaf_order_between_parsed_records() {
        let chroma = ChromaFormatIdc::Yuv420;
        let size = BlockSize::new(BLOCK_8X8).unwrap();
        let leaves = [
            DecodeBlockFrontier::shared_leaf(0, 0, size),
            DecodeBlockFrontier::shared_leaf(2, 4, size),
        ];
        let parsed = |records: &mut TileFilterRecords, r: u16| {
            let block = crate::filters::wienerns_lr::chroma_transform_deblock_block(
                0,
                0,
                0,
                None,
                (1, 1),
                r,
                false,
            )
            .unwrap();
            records.deblock_blocks.push(block);
            records.chroma_deblock_blocks.push(block);
            records.tx_skip_records.push(
                crate::filters::wienerns_lr::WienerNsLrTxSkipTransformRecord {
                    row: r,
                    col: 0,
                    rows: 1,
                    cols: 1,
                    skip_flag: false,
                    empty: false,
                },
            );
        };
        let mut unit = TileFilterRecords::default();
        let mut expected = TileFilterRecords::default();
        for (index, leaf) in leaves.iter().enumerate() {
            parsed(&mut unit, 90 + index as u16);
            parsed(&mut expected, 90 + index as u16);
            for _ in 0..2 {
                unit.defer_inter_leaf(leaf, None, None, 40, false);
                InterDeblockLeaf::new(leaf, None, None, 40, false)
                    .record_into(
                        &mut expected.deblock_blocks,
                        &mut expected.chroma_deblock_blocks,
                        &mut expected.tx_skip_records,
                        crate::filters::wienerns_lr::FilterRecordNeeds::ALL,
                        chroma,
                        &[],
                    )
                    .unwrap();
            }
        }
        parsed(&mut unit, 99);
        parsed(&mut expected, 99);

        let mut frame = TileFilterRecords::default();
        parsed(&mut frame, 7);
        let mut want = TileFilterRecords::default();
        parsed(&mut want, 7);
        want.deblock_blocks.append(&mut expected.deblock_blocks);
        want.chroma_deblock_blocks
            .append(&mut expected.chroma_deblock_blocks);
        want.tx_skip_records.append(&mut expected.tx_skip_records);
        let capacity = unit.deblock_blocks.capacity();
        unit.publish_into(
            &mut frame.deblock_blocks,
            &mut frame.chroma_deblock_blocks,
            &mut frame.tx_skip_records,
            &[],
            chroma,
        )
        .unwrap();

        assert_eq!(frame.deblock_blocks, want.deblock_blocks);
        assert_eq!(frame.tx_skip_records, want.tx_skip_records);
        assert_eq!(frame.chroma_deblock_blocks, want.chroma_deblock_blocks);
        assert!(unit.deblock_blocks.is_empty() && unit.inter_leaves.is_empty());
        assert!(unit.chroma_deblock_blocks.is_empty() && unit.tx_skip_records.is_empty());
        assert_eq!(unit.deblock_blocks.capacity(), capacity);
    }
}

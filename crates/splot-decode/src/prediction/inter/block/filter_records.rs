// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Inter block filter-record handoff.

use splot_core::headers::sequence::ChromaFormatIdc;
use splot_core::tables::conversion::{TX_HEIGHT, TX_WIDTH};

use super::tile::TileFilterRecords;
#[allow(clippy::wildcard_imports)]
use super::*;
use crate::support::decode_buffers::fit_list;

/// One inter leaf's deblock and tx-skip geometry, which the parse queues and
/// the in-order resolve expands, so the record building leaves the parse.
pub(super) struct InterDeblockLeaf {
    frontier: DecodeBlockFrontier,
    residual: Option<InterResidual>,
    sub_pu_size: Option<crate::filters::deblock::DeblockSubPuSize>,
    block_size4: (u16, u16),
    qindex: u32,
    lossless: bool,
    /// The unit's luma, chroma and tx-skip record counts when the leaf was
    /// parsed, which place its records among the ones built during the parse.
    marks: [u32; 3],
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
        (n4w, n4h): (usize, usize),
        residual: Option<&InterResidual>,
        sub_pu_size: Option<crate::filters::deblock::DeblockSubPuSize>,
        qindex: u32,
        lossless: bool,
    ) {
        let marks = self.marks();
        self.inter_leaves.push(InterDeblockLeaf {
            frontier: *frontier,
            residual: residual.cloned(),
            sub_pu_size,
            block_size4: (n4w as u16, n4h as u16),
            qindex,
            lossless,
            marks,
        });
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
        for leaf in &self.inter_leaves {
            self.copy_parsed(
                published..leaf.marks,
                deblock_blocks,
                chroma_deblock_blocks,
                tx_skip_records,
            );
            published = leaf.marks;
            record_inter_deblock_geometry(
                deblock_blocks,
                chroma_deblock_blocks,
                tx_skip_records,
                &leaf.frontier,
                (
                    usize::from(leaf.block_size4.0),
                    usize::from(leaf.block_size4.1),
                ),
                chroma_format,
                leaf.residual.as_ref(),
                residual_blocks,
                leaf.sub_pu_size,
                leaf.qindex,
                leaf.lossless,
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

#[allow(clippy::too_many_arguments)]
pub(crate) fn record_inter_deblock_geometry(
    deblock_blocks: &mut Vec<crate::filters::deblock::DeblockBlock>,
    chroma_deblock_blocks: &mut crate::filters::deblock::ChromaDeblockRecords,
    tx_skip_records: &mut Vec<crate::filters::wienerns_lr::WienerNsLrTxSkipTransformRecord>,
    frontier: &DecodeBlockFrontier,
    block_size4: (usize, usize),
    chroma_format: ChromaFormatIdc,
    residual: Option<&InterResidual>,
    residual_blocks: &[InterResidualBlock],
    sub_pu_size: Option<crate::filters::deblock::DeblockSubPuSize>,
    qindex: u32,
    lossless: bool,
) -> Result<()> {
    let qindex = qindex as u16;
    let (n4w, n4h) = block_size4;
    let (sub_x, sub_y) = chroma_subsampling(chroma_format);
    let chroma_subsampling = (u32::from(sub_x), u32::from(sub_y));
    let chroma_ref = frontier.chroma_ref_geometry();
    let luma_prediction = crate::filters::deblock::DeblockPredictionUnit::new(
        frontier.r,
        frontier.c,
        super::residual::max_tx_size(frontier.b_size.index())?,
    );
    let chroma_prediction = crate::filters::deblock::DeblockPredictionUnit::new(
        chroma_ref.row(),
        chroma_ref.col(),
        super::residual::max_tx_size(chroma_ref.size().index())?,
    );
    let inherited_chroma = chroma_ref.row() != frontier.r
        || chroma_ref.col() != frontier.c
        || chroma_ref.size() != frontier.b_size;
    let inherited_chroma_metadata = frontier.has_chroma && inherited_chroma;
    if inherited_chroma_metadata {
        let chroma_plane_size = get_plane_residual_size(chroma_ref.size(), 1, sub_x, sub_y)
            .map_err(|_| super::residual::residual_geometry_error())?
            .ok_or_else(super::residual::residual_geometry_error)?;
        let chroma_tx = super::residual::max_tx_size(chroma_plane_size.index())?;
        let chroma_n4w = chroma_ref
            .size()
            .num_4x4_wide()
            .map_err(|_| super::residual::residual_geometry_error())?;
        let chroma_n4h = chroma_ref
            .size()
            .num_4x4_high()
            .map_err(|_| super::residual::residual_geometry_error())?;
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
        let tx_size = super::residual::max_tx_size(frontier.b_size.index())?;
        let tx_w4 = super::residual::tx_size_dimension(&TX_WIDTH, tx_size)? / MI_SIZE;
        let tx_h4 = super::residual::tx_size_dimension(&TX_HEIGHT, tx_size)? / MI_SIZE;
        for row4 in crate::support::step_range(0..n4h, tx_h4) {
            for col4 in crate::support::step_range(0..n4w, tx_w4) {
                deblock_blocks.push(crate::filters::deblock::DeblockBlock {
                    r: (frontier.r + row4) as u16,
                    c: (frontier.c + col4) as u16,
                    n4w: tx_w4 as u8,
                    n4h: tx_h4 as u8,
                    tx: tx_size as u8,
                    skip: true,
                    lossless,
                    qindex,
                    prediction: luma_prediction,
                    sub_pu_size,
                });
                tx_skip_records.push(
                    crate::filters::wienerns_lr::WienerNsLrTxSkipTransformRecord {
                        row: frontier.r + row4,
                        col: frontier.c + col4,
                        rows: tx_h4,
                        cols: tx_w4,
                        skip_flag: true,
                        eob: 0,
                    },
                );
            }
        }
        if frontier.has_chroma {
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
                deblock_blocks.push(crate::filters::deblock::DeblockBlock {
                    r: (block.y / MI_SIZE) as u16,
                    c: (block.x / MI_SIZE) as u16,
                    n4w: tx_w4 as u8,
                    n4h: tx_h4 as u8,
                    tx: block.tx_size as u8,
                    skip: false,
                    lossless,
                    qindex,
                    prediction: luma_prediction,
                    sub_pu_size,
                });
                tx_skip_records.push(
                    crate::filters::wienerns_lr::WienerNsLrTxSkipTransformRecord {
                        row: block.y / MI_SIZE,
                        col: block.x / MI_SIZE,
                        rows: tx_h4,
                        cols: tx_w4,
                        skip_flag: false,
                        eob: block.coeffs.eob(),
                    },
                );
            }
            ReconPlaneId::V => {}
            ReconPlaneId::U => {
                if let Some(mut record) =
                    crate::filters::wienerns_lr::chroma_transform_deblock_block(
                        block.x,
                        block.y,
                        block.tx_size,
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
        }
    }
    Ok(())
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
    let chroma_tx = super::residual::max_tx_size(plane_size.index())?;
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
                    row: r as usize,
                    col: 0,
                    rows: 1,
                    cols: 1,
                    skip_flag: false,
                    eob: 1,
                },
            );
        };
        let mut unit = TileFilterRecords::default();
        let mut expected = TileFilterRecords::default();
        for (index, leaf) in leaves.iter().enumerate() {
            parsed(&mut unit, 90 + index as u16);
            parsed(&mut expected, 90 + index as u16);
            unit.defer_inter_leaf(leaf, (2, 2), None, None, 40, false);
            record_inter_deblock_geometry(
                &mut expected.deblock_blocks,
                &mut expected.chroma_deblock_blocks,
                &mut expected.tx_skip_records,
                leaf,
                (2, 2),
                chroma,
                None,
                &[],
                None,
                40,
                false,
            )
            .unwrap();
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

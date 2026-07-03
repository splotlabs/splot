// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::span::ByteOffset;
use splot_core::tables::conversion::{TX_HEIGHT, TX_WIDTH};

use crate::error::Result;
use crate::runtime_minimal::wienerns_lr::recon::WienerNsLrReconSink;
use crate::tile_payload::TileBlockDecodedState;

use super::{Block4x4Extent, MI_SIZE, SelectableLumaTxRecord, tx_dimension};

#[derive(Clone, Copy)]
pub(super) struct LumaCodingBlockExtent {
    pub(super) block_row: usize,
    pub(super) block_col: usize,
    pub(super) n4w: usize,
    pub(super) n4h: usize,
}

impl LumaCodingBlockExtent {
    pub(super) const fn new(block_row: usize, block_col: usize, extent: Block4x4Extent) -> Self {
        Self {
            block_row,
            block_col,
            n4w: extent.cols,
            n4h: extent.rows,
        }
    }
}

pub(super) fn record_per_transform_far_edge(
    sink: &mut WienerNsLrReconSink<u16>,
    block_decoded: &TileBlockDecodedState,
    luma_records: &[SelectableLumaTxRecord],
    block: LumaCodingBlockExtent,
    tile_offset: ByteOffset,
) -> Result<()> {
    for record in luma_records {
        let tx_w4 = tx_dimension("Tx_Width", &TX_WIDTH, record.tx_size, tile_offset)? / MI_SIZE;
        let tx_h4 = tx_dimension("Tx_Height", &TX_HEIGHT, record.tx_size, tile_offset)? / MI_SIZE;
        let (above_right, below_left) = transform_luma_far_edge_avail(
            block_decoded,
            record.row,
            record.col,
            tx_w4,
            tx_h4,
            record.middle,
            block,
        );
        sink.record_block_decoded_far_edge(
            record.col,
            record.row,
            record.tx_size,
            above_right,
            below_left,
        );
    }
    Ok(())
}

pub(super) fn transform_luma_far_edge_avail(
    block_decoded: &TileBlockDecodedState,
    tx_row: usize,
    tx_col: usize,
    tx_w4: usize,
    tx_h4: usize,
    suppress_uneven5_far_edges: bool,
    block: LumaCodingBlockExtent,
) -> (usize, usize) {
    if suppress_uneven5_far_edges {
        return (0, 0);
    }

    let sb_mask = block_decoded.sb_size4().saturating_sub(1);
    let x4 = tx_col & sb_mask;
    let y4 = tx_row & sb_mask;
    let col_off = tx_col.saturating_sub(block.block_col);
    let row_off = tx_row.saturating_sub(block.block_row);
    let above_right = if col_off + tx_w4 < block.n4w {
        tx_w4
    } else if row_off == 0 {
        block_decoded.count_top_right_avail(0, x4, y4, tx_w4)
    } else {
        0
    };
    let below_left = if col_off > 0 {
        0
    } else if row_off + tx_h4 < block.n4h {
        tx_h4
    } else {
        block_decoded.count_bottom_left_avail(0, x4, y4, tx_h4)
    };
    (above_right, below_left)
}

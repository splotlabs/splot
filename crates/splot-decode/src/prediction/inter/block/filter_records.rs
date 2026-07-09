// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Inter block filter-record handoff.

use splot_core::headers::sequence::ChromaFormatIdc;
use splot_core::tables::conversion::{TX_HEIGHT, TX_WIDTH};

#[allow(clippy::wildcard_imports)]
use super::*;

#[allow(clippy::too_many_arguments)]
pub(crate) fn record_inter_deblock_geometry(
    deblock_blocks: &mut Vec<crate::filters::deblock::DeblockBlock>,
    chroma_deblock_blocks: &mut [Vec<crate::filters::deblock::DeblockBlock>; 2],
    tx_skip_records: &mut Vec<crate::filters::wienerns_lr::WienerNsLrTxSkipTransformRecord>,
    frontier: &DecodeBlockFrontier,
    block_size4: (usize, usize),
    chroma_format: ChromaFormatIdc,
    residual: Option<&InterResidual>,
    qindex: u32,
    lossless: bool,
    tile_offset: ByteOffset,
) -> Result<()> {
    let (n4w, n4h) = block_size4;
    let (sub_x, sub_y) = chroma_subsampling(chroma_format);
    let chroma_subsampling = (u32::from(sub_x), u32::from(sub_y));
    let Some(residual) = residual else {
        let tx_size = super::residual::max_tx_size(frontier.b_size.index(), tile_offset)?;
        let tx_w4 =
            super::residual::tx_size_dimension("Tx_Width", &TX_WIDTH, tx_size, tile_offset)?
                / MI_SIZE;
        let tx_h4 =
            super::residual::tx_size_dimension("Tx_Height", &TX_HEIGHT, tx_size, tile_offset)?
                / MI_SIZE;
        for row4 in (0..n4h).step_by(tx_h4.max(1)) {
            for col4 in (0..n4w).step_by(tx_w4.max(1)) {
                deblock_blocks.push(crate::filters::deblock::DeblockBlock {
                    r: frontier.r + row4,
                    c: frontier.c + col4,
                    block_r: frontier.r,
                    block_c: frontier.c,
                    chroma_base_r: frontier.r + row4,
                    chroma_base_c: frontier.c + col4,
                    n4w: tx_w4,
                    n4h: tx_h4,
                    luma_tx: tx_size,
                    chroma_tx:
                        crate::filters::wienerns_lr::fixed_largest_420_chroma_tx_size_from_luma_4x4(
                            tx_w4, tx_h4,
                        ),
                    qindex,
                    skip: true,
                    lossless,
                });
                tx_skip_records.push(
                    crate::filters::wienerns_lr::WienerNsLrTxSkipTransformRecord {
                        row: frontier.r + row4,
                        col: frontier.c + col4,
                        rows: tx_h4,
                        cols: tx_w4,
                        skip_flag: true,
                        eob: 0,
                        intra_ist: None,
                    },
                );
            }
        }
        return Ok(());
    };
    for block in &residual.blocks {
        match block.plane {
            ReconPlaneId::Y => {
                let tx_w4 = (1usize << block.log2_width) / MI_SIZE;
                let tx_h4 = (1usize << block.log2_height) / MI_SIZE;
                deblock_blocks.push(crate::filters::deblock::DeblockBlock {
                    r: block.y / MI_SIZE,
                    c: block.x / MI_SIZE,
                    block_r: frontier.r,
                    block_c: frontier.c,
                    chroma_base_r: block.y / MI_SIZE,
                    chroma_base_c: block.x / MI_SIZE,
                    n4w: tx_w4,
                    n4h: tx_h4,
                    luma_tx: block.tx_size,
                    chroma_tx:
                        crate::filters::wienerns_lr::fixed_largest_420_chroma_tx_size_from_luma_4x4(
                            tx_w4, tx_h4,
                        ),
                    qindex,
                    skip: false,
                    lossless,
                });
                tx_skip_records.push(
                    crate::filters::wienerns_lr::WienerNsLrTxSkipTransformRecord {
                        row: block.y / MI_SIZE,
                        col: block.x / MI_SIZE,
                        rows: tx_h4,
                        cols: tx_w4,
                        skip_flag: false,
                        eob: block.coeffs.eob,
                        intra_ist: None,
                    },
                );
            }
            ReconPlaneId::U | ReconPlaneId::V => {
                if let Some((plane_index, record)) =
                    crate::filters::wienerns_lr::chroma_transform_deblock_block(
                        block.plane,
                        block.x,
                        block.y,
                        block.tx_size,
                        chroma_subsampling,
                        qindex,
                        lossless,
                    )
                {
                    chroma_deblock_blocks[plane_index].push(record);
                }
            }
        }
    }
    Ok(())
}

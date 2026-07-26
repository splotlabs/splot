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
    residual_blocks: &[InterResidualBlock],
    sub_pu_size: Option<crate::filters::deblock::DeblockSubPuSize>,
    qindex: u32,
    lossless: bool,
    tile_offset: ByteOffset,
) -> Result<()> {
    let _phase = crate::timing::WalkPhaseScope::new(crate::timing::WalkPhase::Records);
    let (n4w, n4h) = block_size4;
    let (sub_x, sub_y) = chroma_subsampling(chroma_format);
    let chroma_subsampling = (u32::from(sub_x), u32::from(sub_y));
    let chroma_ref = frontier.chroma_ref_geometry();
    let luma_prediction = crate::filters::deblock::DeblockPredictionUnit {
        base_r: frontier.r,
        base_c: frontier.c,
        default_sub_pu_tx: super::residual::max_tx_size(frontier.b_size.index(), tile_offset)?,
    };
    let chroma_prediction = crate::filters::deblock::DeblockPredictionUnit {
        base_r: chroma_ref.row(),
        base_c: chroma_ref.col(),
        default_sub_pu_tx: super::residual::max_tx_size(chroma_ref.size().index(), tile_offset)?,
    };
    let inherited_chroma = chroma_ref.row() != frontier.r
        || chroma_ref.col() != frontier.c
        || chroma_ref.size() != frontier.b_size;
    let inherited_chroma_metadata = frontier.has_chroma && inherited_chroma;
    if inherited_chroma_metadata {
        let chroma_plane_size = get_plane_residual_size(chroma_ref.size(), 1, sub_x, sub_y)
            .map_err(|_| super::residual::residual_geometry_error(tile_offset))?
            .valid()
            .ok_or_else(|| super::residual::residual_geometry_error(tile_offset))?;
        let chroma_tx = super::residual::max_tx_size(chroma_plane_size.index(), tile_offset)?;
        let chroma_n4w = chroma_ref
            .size()
            .num_4x4_wide()
            .map_err(|_| super::residual::residual_geometry_error(tile_offset))?;
        let chroma_n4h = chroma_ref
            .size()
            .num_4x4_high()
            .map_err(|_| super::residual::residual_geometry_error(tile_offset))?;
        let block = crate::filters::deblock::DeblockBlock {
            r: chroma_ref.row(),
            c: chroma_ref.col(),
            luma_prediction,
            chroma_prediction,
            chroma_base_r: chroma_ref.row(),
            chroma_base_c: chroma_ref.col(),
            n4w: chroma_n4w,
            n4h: chroma_n4h,
            luma_tx: chroma_tx,
            chroma_tx: Some(chroma_tx),
            sub_pu_size,
            chroma_transform_only: false,
            qindex,
            skip: residual.is_none(),
            lossless,
        };
        chroma_deblock_blocks[0].push(block);
        chroma_deblock_blocks[1].push(block);
    }
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
                    luma_prediction,
                    chroma_prediction,
                    chroma_base_r: frontier.r + row4,
                    chroma_base_c: frontier.c + col4,
                    n4w: tx_w4,
                    n4h: tx_h4,
                    luma_tx: tx_size,
                    chroma_tx:
                        crate::filters::wienerns_lr::fixed_largest_420_chroma_tx_size_from_luma_4x4(
                            tx_w4, tx_h4,
                        ),
                    sub_pu_size,
                    chroma_transform_only: false,
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
    let blocks = residual
        .blocks(residual_blocks)
        .ok_or_else(|| super::residual::residual_geometry_error(tile_offset))?;
    for block in blocks {
        match block.plane {
            ReconPlaneId::Y => {
                let tx_w4 = (1usize << block.log2_width) / MI_SIZE;
                let tx_h4 = (1usize << block.log2_height) / MI_SIZE;
                deblock_blocks.push(crate::filters::deblock::DeblockBlock {
                    r: block.y / MI_SIZE,
                    c: block.x / MI_SIZE,
                    luma_prediction,
                    chroma_prediction,
                    chroma_base_r: block.y / MI_SIZE,
                    chroma_base_c: block.x / MI_SIZE,
                    n4w: tx_w4,
                    n4h: tx_h4,
                    luma_tx: block.tx_size,
                    chroma_tx:
                        crate::filters::wienerns_lr::fixed_largest_420_chroma_tx_size_from_luma_4x4(
                            tx_w4, tx_h4,
                        ),
                    sub_pu_size,
                    chroma_transform_only: false,
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
                if let Some((plane_index, mut record)) =
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
                    retain_inter_prediction_metadata(
                        &mut record,
                        luma_prediction,
                        chroma_prediction,
                        sub_pu_size,
                        inherited_chroma_metadata,
                    );
                    chroma_deblock_blocks[plane_index].push(record);
                }
            }
        }
    }
    Ok(())
}

fn retain_inter_prediction_metadata(
    record: &mut crate::filters::deblock::DeblockBlock,
    luma_prediction: crate::filters::deblock::DeblockPredictionUnit,
    chroma_prediction: crate::filters::deblock::DeblockPredictionUnit,
    sub_pu_size: Option<crate::filters::deblock::DeblockSubPuSize>,
    chroma_transform_only: bool,
) {
    record.luma_prediction = luma_prediction;
    record.chroma_prediction = chroma_prediction;
    record.sub_pu_size = sub_pu_size;
    record.chroma_transform_only = chroma_transform_only;
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use splot_recon::PlaneId;

    use super::*;

    #[test]
    fn ordinary_chroma_residual_retains_parent_prediction_and_rectangular_subpu() {
        let (_, mut record) = crate::filters::wienerns_lr::chroma_transform_deblock_block(
            PlaneId::U,
            8,
            12,
            3,
            (1, 1),
            77,
            false,
        )
        .unwrap();
        let luma_prediction = crate::filters::deblock::DeblockPredictionUnit {
            base_r: 2,
            base_c: 3,
            default_sub_pu_tx: 4,
        };
        let chroma_prediction = crate::filters::deblock::DeblockPredictionUnit {
            base_r: 4,
            base_c: 5,
            default_sub_pu_tx: 6,
        };
        let sub_pu_size = crate::filters::deblock::DeblockSubPuSize::new(8, 16);

        retain_inter_prediction_metadata(
            &mut record,
            luma_prediction,
            chroma_prediction,
            Some(sub_pu_size),
            false,
        );

        assert_eq!(record.luma_prediction, luma_prediction);
        assert_eq!(record.chroma_prediction, chroma_prediction);
        assert_eq!(record.sub_pu_size, Some(sub_pu_size));
        assert!(!record.chroma_transform_only);
    }
}

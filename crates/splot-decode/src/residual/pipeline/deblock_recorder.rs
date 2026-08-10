// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Deblock metadata recording for residual reconstruction.

pub(crate) struct DeblockRecorder<'a> {
    pub(crate) blocks: &'a mut Vec<crate::filters::deblock::DeblockBlock>,
    pub(crate) chroma_blocks: &'a mut crate::filters::deblock::ChromaDeblockRecords,
    pub(crate) tx_skip_records:
        &'a mut Vec<crate::filters::wienerns_lr::WienerNsLrTxSkipTransformRecord>,
    pub(crate) block_r: usize,
    pub(crate) block_c: usize,
    pub(crate) chroma_tx: Option<usize>,
    pub(crate) chroma_subsampling: (u32, u32),
    pub(crate) qindex: u32,
    pub(crate) lossless: bool,
}

impl DeblockRecorder<'_> {
    pub(super) fn record_chroma_unit(
        &mut self,
        plane_id: splot_recon::PlaneId,
        x: usize,
        y: usize,
        tx_size: usize,
    ) {
        if let Some((plane_index, record)) =
            crate::filters::wienerns_lr::chroma_transform_deblock_block(
                plane_id,
                x,
                y,
                tx_size,
                self.chroma_subsampling,
                self.qindex,
                self.lossless,
            )
        {
            self.chroma_blocks.push(plane_index, record);
        }
    }

    pub(super) fn record_luma_unit(
        &mut self,
        r: usize,
        c: usize,
        n4w: usize,
        n4h: usize,
        luma_tx: usize,
        eob: usize,
    ) {
        self.blocks.push(crate::filters::deblock::DeblockBlock {
            r,
            c,
            luma_prediction: crate::filters::deblock::DeblockPredictionUnit {
                base_r: self.block_r,
                base_c: self.block_c,
                default_sub_pu_tx: luma_tx,
            },
            chroma_prediction: crate::filters::deblock::DeblockPredictionUnit {
                base_r: self.block_r,
                base_c: self.block_c,
                default_sub_pu_tx: self.chroma_tx.unwrap_or(luma_tx),
            },
            chroma_base_r: self.block_r,
            chroma_base_c: self.block_c,
            n4w,
            n4h,
            luma_tx,
            chroma_tx: self.chroma_tx,
            sub_pu_size: None,
            chroma_transform_only: false,
            qindex: self.qindex,
            skip: false,
            lossless: self.lossless,
        });
        self.tx_skip_records.push(
            crate::filters::wienerns_lr::WienerNsLrTxSkipTransformRecord {
                row: r,
                col: c,
                rows: n4h,
                cols: n4w,
                skip_flag: false,
                eob,
            },
        );
    }
}

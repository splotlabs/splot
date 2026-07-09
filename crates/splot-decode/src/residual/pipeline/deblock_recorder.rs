// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Deblock metadata recording for residual reconstruction.

pub(crate) struct DeblockRecorder<'a> {
    pub(crate) blocks: &'a mut Vec<crate::filters::deblock::DeblockBlock>,
    pub(crate) chroma_blocks: &'a mut [Vec<crate::filters::deblock::DeblockBlock>; 2],
    pub(crate) tx_skip_records:
        &'a mut Vec<crate::filters::wienerns_lr::WienerNsLrTxSkipTransformRecord>,
    pub(crate) block_r: usize,
    pub(crate) block_c: usize,
    pub(crate) block_w4: usize,
    pub(crate) block_h4: usize,
    pub(crate) luma_tx: usize,
    pub(crate) chroma_tx: Option<usize>,
    pub(crate) qindex: u32,
    pub(crate) lossless: bool,
}

impl DeblockRecorder<'_> {
    pub(super) fn record_chroma_part_block(&mut self) {
        for chroma in self.chroma_blocks.iter_mut() {
            chroma.push(crate::filters::deblock::DeblockBlock {
                r: self.block_r,
                c: self.block_c,
                block_r: self.block_r,
                block_c: self.block_c,
                chroma_base_r: self.block_r,
                chroma_base_c: self.block_c,
                n4w: self.block_w4,
                n4h: self.block_h4,
                luma_tx: self.luma_tx,
                chroma_tx: self.chroma_tx,
                qindex: self.qindex,
                skip: false,
                lossless: self.lossless,
            });
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
            block_r: self.block_r,
            block_c: self.block_c,
            chroma_base_r: self.block_r,
            chroma_base_c: self.block_c,
            n4w,
            n4h,
            luma_tx,
            chroma_tx: self.chroma_tx,
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
                intra_ist: None,
            },
        );
    }
}

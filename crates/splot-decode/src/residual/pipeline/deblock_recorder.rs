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
    pub(crate) chroma_subsampling: (u32, u32),
    pub(crate) qindex: u16,
    pub(crate) lossless: bool,
}

impl DeblockRecorder<'_> {
    /// Records a chroma transform unit once for both planes: from its U
    /// unit, as V's has the same geometry ([`ChromaDeblockRecords`]).
    ///
    /// [`ChromaDeblockRecords`]: crate::filters::deblock::ChromaDeblockRecords
    pub(crate) fn record_chroma_unit(
        &mut self,
        plane_id: splot_recon::PlaneId,
        x: usize,
        y: usize,
        tx_size: usize,
    ) {
        if plane_id != splot_recon::PlaneId::U {
            return;
        }
        if let Some(record) = crate::filters::wienerns_lr::chroma_transform_deblock_block(
            x,
            y,
            tx_size,
            None,
            self.chroma_subsampling,
            self.qindex,
            self.lossless,
        ) {
            self.chroma_blocks.push(record);
        }
    }

    pub(crate) fn record_luma_unit(
        &mut self,
        r: usize,
        c: usize,
        n4w: usize,
        n4h: usize,
        luma_tx: usize,
        eob: usize,
    ) {
        self.blocks.push(crate::filters::deblock::DeblockBlock {
            r: r as u16,
            c: c as u16,
            n4w: n4w as u8,
            n4h: n4h as u8,
            tx: luma_tx as u8,
            skip: false,
            lossless: self.lossless,
            qindex: self.qindex,
            prediction: crate::filters::deblock::DeblockPredictionUnit::new(
                self.block_r,
                self.block_c,
                luma_tx,
            ),
            sub_pu_size: None,
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

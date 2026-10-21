// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Encoder-side reconstruction of `DC_PRED` transform blocks into a `u16` frame.
//!
//! The frame has the decoder's geometry: coded size `FrameWidth x FrameHeight` and
//! storage `MiCols * 4 x MiRows * 4`. Prediction reads the in-storage edges of
//! § 7.13.2 with single-tile availability (`x > 0` left, `y > 0` above), and the
//! residual uses the decoder's own dequantization and inverse-transform parameters.

use splot_decode::encoder_syntax::{TransformBlock, dct_recon_params, tx_log2};
use splot_recon::{
    BitDepth, CurrentFrameWorkspace, DecodedFrame, DecodedFrameInfo, IntraDcEdges,
    IntraRectBlockSize, OutputIndex, PixelFormat, PlaneId, PlaneRect, PlaneSize,
    predict_intra_dc_rect_value, reconstruct_transform_block_residual,
};

use crate::tile_writer::{Result, TileWriterError, coeffs::adjusted_log2};

/// The reconstructed samples of one frame.
pub(crate) struct ReconFrame {
    workspace: CurrentFrameWorkspace<u16>,
    bit_depth: BitDepth,
    qindex: u32,
}

impl ReconFrame {
    /// A frame of `width x height` luma samples over `mi_cols x mi_rows` 4x4 storage.
    pub(crate) fn new(
        (width, height): (u32, u32),
        (mi_cols, mi_rows): (usize, usize),
        bit_depth: BitDepth,
        pixel_format: PixelFormat,
        qindex: u32,
    ) -> Result<Self> {
        let size = PlaneSize::new(width as usize, height as usize)?;
        let info = DecodedFrameInfo::new(
            OutputIndex::new(0),
            bit_depth,
            pixel_format,
            size,
            PlaneRect::new(0, 0, width as usize, height as usize)?,
        )?
        .with_storage_luma_size(PlaneSize::new(mi_cols * 4, mi_rows * 4)?)?;
        Ok(Self {
            workspace: CurrentFrameWorkspace::new(info, 0)?,
            bit_depth,
            qindex,
        })
    }

    /// The row-major `DC_PRED` prediction of `tx`.
    pub(crate) fn predict_dc(&self, tx: &TransformBlock) -> Result<Vec<u16>> {
        let size = rect_size(tx)?;
        let edges =
            self.workspace
                .intra_dc_edges_for_rect(plane_id(tx.plane)?, tx.x, tx.y, size)?;
        let dc_edges = IntraDcEdges::new(
            (tx.x > 0).then(|| edges.left_samples()).flatten(),
            (tx.y > 0).then(|| edges.above_samples()).flatten(),
        );
        let dc = predict_intra_dc_rect_value(self.bit_depth, size, dc_edges)?;
        Ok(vec![dc; size.sample_count()])
    }

    /// Writes `prediction` plus the residual of the adjusted `levels` of `tx`.
    pub(crate) fn reconstruct(
        &mut self,
        tx: &TransformBlock,
        prediction: &[u16],
        levels: &[i32],
        eob: usize,
    ) -> Result<()> {
        let plane = plane_id(tx.plane)?;
        let size = rect_size(tx)?;
        if eob == 0 {
            self.workspace
                .write_rect_block(plane, tx.x, tx.y, size, prediction)?;
            return Ok(());
        }
        let (log2_w, log2_h) = tx_log2(tx.tx_size)?;
        let (params, transform) =
            dct_recon_params(self.qindex, plane, log2_w, log2_h, self.bit_depth)?;
        let (adj_w, adj_h) = adjusted_log2(tx.tx_size)?;
        let samples = size.sample_count();
        let mut dequant = vec![0; 1 << (adj_w + adj_h)];
        let mut residual = vec![0; samples];
        let mut out = vec![0; samples];
        reconstruct_transform_block_residual(
            prediction,
            levels,
            &params,
            &transform,
            &mut dequant,
            &mut residual,
            &mut out,
        )?;
        self.workspace
            .write_rect_block(plane, tx.x, tx.y, size, &out)?;
        Ok(())
    }

    /// The finished frame.
    pub(crate) fn into_frame(self) -> Result<DecodedFrame<u16>> {
        Ok(self.workspace.freeze()?)
    }
}

fn rect_size(tx: &TransformBlock) -> Result<IntraRectBlockSize> {
    let (log2_w, log2_h) = tx_log2(tx.tx_size)?;
    Ok(IntraRectBlockSize::new(log2_w as u8, log2_h as u8)?)
}

pub(crate) fn plane_id(plane: usize) -> Result<PlaneId> {
    match plane {
        0 => Ok(PlaneId::Y),
        1 => Ok(PlaneId::U),
        2 => Ok(PlaneId::V),
        _ => Err(TileWriterError::Unsupported("plane index")),
    }
}

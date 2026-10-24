// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Cross-chroma transforms (CCTX, § 5.20.7.27, § 7.14.6) of the encoder facade.

use splot_recon::PlaneId;

use super::super::cdf::TileCdfSelector;
use super::super::general_intra_residual::cctx::{
    CCTX_MTX, reconstruct_general_intra_chroma_cctx_pair_into,
};
use super::super::general_intra_residual::{CoeffBlock, LumaCoeffBlock, is_cctx_geometry_allowed};
use super::super::tile_frontier::chroma_subsampling;
use super::{
    EncoderSyntaxError, IntraTileSyntax, LeafBlock, Result, TransformBlock, coefficient_scan, rule,
    tx_log2,
};

/// `Cctx_Mtx` (§ 7.14.6): `(cos, sin) << CCTX_PREC_BITS` of `cctx_type`
/// `1..=6` at index `cctx_type - 1`.
pub const CCTX_ROTATIONS: [[i32; 2]; 6] = CCTX_MTX;

impl IntraTileSyntax {
    /// Whether U block `tx` with `eob` codes `cctx_type` (§ 5.20.7.27).
    #[must_use]
    pub fn cctx_coded(&self, tx: &TransformBlock, eob: usize) -> bool {
        self.tools.enable_cctx
            && tx.plane == 1
            && self.cctx_leaf
            && eob > 0
            && (self.leaf_is_inter() || eob != 1)
    }

    /// Sends the `cctx_type` of U block `tx` with `eob` to `put` as `(CDF row,
    /// value)`; nothing when the decoder does not read it.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Unsupported`] for a `cctx_type` the block
    /// cannot code, and the first error of `put`.
    pub fn code_cctx_type<E: From<EncoderSyntaxError>>(
        &mut self,
        tx: &TransformBlock,
        eob: usize,
        put: impl FnOnce(&mut [u16], usize) -> core::result::Result<(), E>,
    ) -> core::result::Result<(), E> {
        if !self.cctx_coded(tx, eob) {
            if tx.cctx != 0 {
                return Err(EncoderSyntaxError::Unsupported("CCTX for this block and eob").into());
            }
            return Ok(());
        }
        if usize::from(tx.cctx) > CCTX_ROTATIONS.len() {
            return Err(EncoderSyntaxError::Unsupported("CCTX type").into());
        }
        put(self.tile_row(TileCdfSelector::CctxType)?, tx.cctx.into())
    }

    /// The reconstruction of chroma blocks `u` (with its `cctx`) and `v` of one
    /// position over their row-major `predictions`, coded as `levels`, with the
    /// decoder's dequantization, inverse cross-chroma transform and inverse
    /// transform.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for a `cctx` of `0` or blocks the decoder
    /// cannot pair.
    pub fn cctx_reconstruction(
        &self,
        [u, v]: [&TransformBlock; 2],
        predictions: [&[u16]; 2],
        levels: [&[i32]; 2],
    ) -> Result<[Vec<u16>; 2]> {
        let block = |tx: &TransformBlock, levels: &[i32], cctx_type| -> Result<LumaCoeffBlock> {
            let nonzero = levels
                .iter()
                .rposition(|&q| q != 0)
                .map_or(0, |last| last + 1);
            Ok(LumaCoeffBlock {
                eob: block_eob(tx, levels)?,
                quant_range: 0..nonzero,
                zero_tail: levels.len() - nonzero,
                cctx_type,
                ..LumaCoeffBlock::empty(tx.tx_type as usize, false)
            })
        };
        let u_block = block(u, levels[0], Some(u.cctx.into()))?;
        let v_block = block(v, levels[1], None)?;
        let (log2_w, log2_h) = tx_log2(u.tx_size)?;
        let (mut u_out, mut v_out) = (Vec::new(), Vec::new());
        reconstruct_general_intra_chroma_cctx_pair_into(
            CoeffBlock::new(&u_block, levels[0]).map_err(rule("CCTX block"))?,
            predictions[0],
            CoeffBlock::new(&v_block, levels[1]).map_err(rule("CCTX block"))?,
            predictions[1],
            self.tools.qindex,
            log2_w,
            log2_h,
            u.cctx.into(),
            false,
            self.tools.bit_depth,
            &mut u_out,
            &mut v_out,
        )
        .map_err(rule("CCTX reconstruction"))?;
        Ok([u_out, v_out])
    }

    /// Records whether the chroma of `leaf` may code CCTX (§ 5.20.7.27
    /// `is_cctx_allowed` geometry), for the leaf just planned.
    pub(super) fn plan_cctx(&mut self, leaf: &LeafBlock) {
        let (block_ctx, _) = self.block_ctx(leaf);
        let (sub_x, sub_y) = chroma_subsampling(self.chroma);
        let plane = block_ctx.plane_block(PlaneId::U);
        self.cctx_leaf = leaf.has_chroma()
            && is_cctx_geometry_allowed(sub_x && sub_y, plane.width4() * 4, plane.height4() * 4);
    }
}

/// The `eob` of row-major adjusted `levels` in the scan of `tx`.
pub(super) fn block_eob(tx: &TransformBlock, levels: &[i32]) -> Result<usize> {
    Ok(coefficient_scan(tx)?
        .iter()
        .rposition(|&pos| levels.get(usize::from(pos)).is_some_and(|&l| l != 0))
        .map_or(0, |last| last + 1))
}

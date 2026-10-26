// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Intra block copy of the encoder facade (§ 5.20.5.4): the block vectors
//! a leaf may code (§ 6.17.3.3 `is_mv_valid`), their syntax, the copy
//! prediction and the publication, through the decoder's intra-frame state.

use core::ops::Range;

use splot_core::symbol_encoder::SymbolSink;

use super::super::cdf::TileCdfSelector;
use super::{
    EncoderSyntaxError, IntraTileSyntax, LeafBlock, MotionVector, Result, TransformBlock,
    TxPartition, rule,
};
use crate::prediction::inter::read_mv::{MV_PRECISION_ONE_PEL, MvReadConfig};
use crate::prediction::inter::{MvdCoding, non_intra_leaf_mode};

/// An intra block copy of a leaf (§ 5.20.5.4).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IntrabcModes {
    /// The whole-sample block vector in 1/8 samples.
    pub bv: MotionVector,
    /// `RefMvIdx`, below the length of [`IntraTileSyntax::intrabc_ref_bvs`].
    pub drl: usize,
}

impl IntraTileSyntax {
    /// Whether `leaf` codes `use_intrabc`: an intra frame with
    /// `allow_intrabc` and a leaf of at most 64x64 other than 64x64.
    #[must_use]
    pub fn intrabc_allowed(&self, leaf: &LeafBlock) -> bool {
        self.inter.as_ref().is_some_and(|inter| {
            inter.tile.is_intra_frame()
                && matches!(inter.tile.intrabc_contexts(&leaf.frontier), Ok(Some(_)))
        })
    }

    /// The reference block vector of each `RefMvIdx` of `leaf` (§ 7.12).
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] when `leaf` codes no `use_intrabc`.
    pub fn intrabc_ref_bvs(&mut self, leaf: &LeafBlock) -> Result<Vec<MotionVector>> {
        if !self.intrabc_allowed(leaf) {
            return Err(NO_INTRABC);
        }
        let mvs = self
            .inter_state()?
            .tile
            .intrabc_ref_mvs(&leaf.frontier)
            .map_err(rule("IntraBC vector stack"))?;
        Ok(mvs.into_iter().map(MotionVector::from).collect())
    }

    /// Whether `leaf` may code the block vector `bv`: a whole-sample vector
    /// of a leaf at least 8x8 that § 6.17.3.3 `is_mv_valid` accepts for the
    /// global area, with the source in the current or the previous
    /// superblock row.
    #[must_use]
    pub fn intrabc_bv_valid(&self, leaf: &LeafBlock, bv: MotionVector) -> bool {
        let Ok(size) = leaf.dimensions() else {
            return false;
        };
        self.intrabc_allowed(leaf)
            && global_bv_valid(
                &BvFrame {
                    mi: (leaf.row4(), leaf.col4()),
                    size,
                    tile: self.tile.clone(),
                    sb_size: self.sb_size4 * 4,
                },
                [bv.row, bv.col],
            )
    }

    /// Plans the IntraBC leaf `leaf` of `modes` with the luma transform split
    /// by `partition`: predicts it by the copy and returns its transform
    /// blocks as [`Self::plan_inter_leaf_partitioned`] does.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for a vector or partition `leaf`
    /// cannot code.
    pub fn plan_intrabc_leaf(
        &mut self,
        leaf: &LeafBlock,
        modes: IntrabcModes,
        partition: TxPartition,
    ) -> Result<Vec<TransformBlock>> {
        let split = self.partitioned_tx_size(leaf, partition)?;
        if !self.intrabc_bv_valid(leaf, modes.bv) {
            return Err(EncoderSyntaxError::Unsupported("this IntraBC vector"));
        }
        let motion = self.inter_ref()?.tile.intrabc_motion(modes.drl);
        self.plan_motion_leaf(leaf, (motion, Some(modes.bv.into())), (partition, split))
    }

    /// Writes `use_intrabc` of `leaf` and, for an IntraBC leaf (`skip` is
    /// `Some`), its `skip_txfm` (§ 5.20.5.3); nothing for a leaf that codes
    /// no `use_intrabc`.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for an IntraBC leaf where `leaf` codes
    /// none, or for a write failure.
    pub fn write_use_intrabc(
        &mut self,
        leaf: &LeafBlock,
        skip: Option<bool>,
        symbols: &mut impl SymbolSink,
    ) -> Result<()> {
        let contexts = match &self.inter {
            Some(inter) if inter.tile.is_intra_frame() => inter
                .tile
                .intrabc_contexts(&leaf.frontier)
                .map_err(rule("IntraBC contexts"))?,
            _ => None,
        };
        let Some(contexts) = contexts else {
            return match skip {
                Some(_) => Err(NO_INTRABC),
                None => Ok(()),
            };
        };
        let ctx = contexts.use_intrabc;
        self.put(
            symbols,
            TileCdfSelector::Intrabc { ctx },
            usize::from(skip.is_some()),
        )?;
        match skip {
            Some(skip) => {
                let ctx = contexts.skip;
                self.put(symbols, TileCdfSelector::Skip { ctx }, usize::from(skip))
            }
            None => Ok(()),
        }
    }

    /// Writes the § 5.20.5.4 IntraBC syntax of `leaf` under `modes`:
    /// `intrabc_mode`, the `intrabc_drl_mode` bits, `intrabc_precision`
    /// (whole samples), the vector difference and `morph_pred = 0`.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for an index or vector `leaf` cannot
    /// code, or for a write failure.
    pub fn write_intrabc_modes(
        &mut self,
        leaf: &LeafBlock,
        modes: IntrabcModes,
        symbols: &mut impl SymbolSink,
    ) -> Result<()> {
        let refs = self.intrabc_ref_bvs(leaf)?;
        let reference = *refs.get(modes.drl).ok_or(NO_INTRABC)?;
        let tile = &self.inter_ref()?.tile;
        let contexts = tile
            .intrabc_contexts(&leaf.frontier)
            .map_err(rule("IntraBC contexts"))?
            .ok_or(NO_INTRABC)?;
        let core = tile.frame_header();
        let force_integer_mv = core.force_integer_mv == Some(true);
        let morph_coded = core.allow_screen_content_tools == Some(true)
            && tile
                .sequence_header()
                .inter
                .as_ref()
                .is_some_and(|inter| inter.enable_bawp);
        let mode = usize::from(modes.bv == reference);
        self.put(symbols, TileCdfSelector::IntrabcMode, mode)?;
        let max = refs.len() - 1;
        for idx in 0..max {
            literal(symbols, u32::from(modes.drl > idx), 1)?;
            if modes.drl <= idx {
                break;
            }
        }
        if mode == 0 {
            if !force_integer_mv {
                self.put(symbols, TileCdfSelector::IntrabcPrecision, 0)?;
            }
            let base = lower_to_whole(reference);
            let mvd = MotionVector {
                row: modes.bv.row - base.row,
                col: modes.bv.col - base.col,
            };
            let coding = MvdCoding {
                precision: MV_PRECISION_ONE_PEL,
                use_amvd: false,
                derive_sign: false,
                derive_threshold: 1,
            };
            let config = MvReadConfig::intrabc(MV_PRECISION_ONE_PEL);
            super::mv::write_mvds_with(
                (&mut self.cdfs, &mut self.journal),
                (coding, config),
                &[mvd],
                symbols,
            )?;
        }
        if morph_coded {
            let ctx = contexts.morph_pred;
            self.put(symbols, TileCdfSelector::MorphPred { ctx }, 0)?;
        }
        Ok(())
    }

    /// Publishes the coded IntraBC leaf `leaf` with `skip_txfm = skip` into
    /// the neighbour, IntraBC, motion and `BlockDecoded` state.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] without a planned IntraBC leaf.
    pub fn publish_intrabc_leaf(&mut self, leaf: &LeafBlock, skip: bool) -> Result<()> {
        let mv = self.take_planned_intrabc(leaf)?;
        let state = self.inter.as_mut().ok_or(NO_INTRABC)?;
        if skip {
            state
                .tile
                .reset_skip_contexts(&leaf.frontier, &mut self.coeff)
                .map_err(rule("coefficient contexts"))?;
        }
        let tile = &mut self.inter_state()?.tile;
        tile.publish_intrabc(&leaf.frontier, mv, skip)
            .map_err(rule("IntraBC publication"))?;
        self.publish_leaf_modes(leaf, non_intra_leaf_mode(&leaf.frontier).mark_intrabc())
    }
}

/// `pred` with each component rounded to whole samples as § 7.10.2.11
/// `lower_mv_precision` does for `MV_PRECISION_ONE_PEL`.
fn lower_to_whole(pred: MotionVector) -> MotionVector {
    crate::prediction::inter::read_mv::lower_mv_precision(MV_PRECISION_ONE_PEL, pred.into()).into()
}

fn literal(symbols: &mut impl SymbolSink, value: u32, bits: u32) -> Result<()> {
    symbols.literal(value, bits).map_err(rule("symbol write"))
}

const NO_INTRABC: EncoderSyntaxError = EncoderSyntaxError::Unsupported("IntraBC in this block");

/// `INTRABC_DELAY_SB64` (§ 3).
const INTRABC_DELAY_SB64: i64 = 4;
/// `MI_SIZE` (§ 3).
const MI_SIZE: i64 = 4;

/// A leaf and the tile it lies in, in luma 4x4 units.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct BvFrame {
    /// `MiRow`, `MiCol` of the leaf.
    pub(super) mi: (usize, usize),
    /// `Block_Width[MiSize]`, `Block_Height[MiSize]` in samples.
    pub(super) size: (usize, usize),
    /// `MiRowStart..MiRowEnd` and `MiColStart..MiColEnd`.
    pub(super) tile: (Range<usize>, Range<usize>),
    /// `Block_Width[SbSize]` (= `Block_Height[SbSize]`) in samples.
    pub(super) sb_size: usize,
}

/// Whether the whole-sample block vector `mv` (`[row, col]` in 1/8 samples)
/// passes § 6.17.3.3 `is_mv_valid` for a frame with `allow_local_intrabc =
/// 0` and `allow_global_intrabc = 1`, taking `sb64Residual = 0` (a subset of
/// the vectors `AllowExtraIBCRange` allows), for a leaf at least 8x8 whose
/// chroma is its own (so `ChromaMiRow/Col` equal `MiRow/Col`). The source
/// must also lie in the current or the previous superblock row, the global
/// area of the frame header semantics.
pub(super) fn global_bv_valid(frame: &BvFrame, mv: [i32; 2]) -> bool {
    if mv[0] & 7 != 0 || mv[1] & 7 != 0 || frame.size.0 < 8 || frame.size.1 < 8 {
        return false;
    }
    let (mi_row, mi_col) = (frame.mi.0 as i64, frame.mi.1 as i64);
    let (bw, bh) = (frame.size.0 as i64, frame.size.1 as i64);
    let (delta_row, delta_col) = (i64::from(mv[0] >> 3), i64::from(mv[1] >> 3));
    let src_top = mi_row * MI_SIZE + delta_row;
    let src_left = mi_col * MI_SIZE + delta_col;
    let src_bottom = src_top + bh;
    let src_right = src_left + bw;
    let (rows, cols) = &frame.tile;
    if src_top < rows.start as i64 * MI_SIZE
        || src_left < cols.start as i64 * MI_SIZE
        || src_bottom > rows.end as i64 * MI_SIZE
        || src_right > cols.end as i64 * MI_SIZE
    {
        return false;
    }
    let sb_h = frame.sb_size as i64;
    let active_sb_row = mi_row * MI_SIZE / sb_h;
    let active_sb64_col = (mi_col * MI_SIZE) >> 6;
    let src_sb_row = (src_bottom - 1) / sb_h;
    let src_sb64_col = (src_right - 1) >> 6;
    let total_sb64_per_row = ((cols.len() as i64 - 1) >> 4) + 1;
    let active_sb64 = active_sb_row * total_sb64_per_row + active_sb64_col;
    let src_sb64 = src_sb_row * total_sb64_per_row + src_sb64_col;
    if src_sb64 >= active_sb64 - INTRABC_DELAY_SB64 {
        return false;
    }
    let gradient = INTRABC_DELAY_SB64 + sb_h / 64;
    let wf_offset = gradient * (active_sb_row - src_sb_row);
    if src_sb_row > active_sb_row
        || src_sb64_col >= active_sb64_col - INTRABC_DELAY_SB64 + wf_offset
    {
        return false;
    }
    src_top / sb_h >= active_sb_row - 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(mi: (usize, usize), size: (usize, usize)) -> BvFrame {
        BvFrame {
            mi,
            size,
            tile: (0..64, 0..160),
            sb_size: 64,
        }
    }

    /// § 6.17.3.3 for a 16x8 leaf at (64, 384) of a 640-wide tile of 64x64
    /// superblocks: in its own row the source must end four 64-wide columns
    /// before the current one; in the row above, before the wavefront.
    #[test]
    fn global_vectors_follow_the_superblock_delay() {
        let leaf = frame((16, 96), (16, 8));
        let sample = |dy: i32, dx: i32| [dy * 8, dx * 8];
        assert!(global_bv_valid(&leaf, sample(0, -272)));
        assert!(!global_bv_valid(&leaf, sample(0, -264)));
        assert!(global_bv_valid(&leaf, sample(-64, 0)));
        assert!(global_bv_valid(&leaf, sample(-64, 48)));
        assert!(!global_bv_valid(&leaf, sample(-64, 56)));
        assert!(!global_bv_valid(&leaf, sample(-128, 0)));
        assert!(!global_bv_valid(&leaf, sample(0, -400)));
        assert!(!global_bv_valid(&leaf, [4, -64 * 8 * 5]));
        assert!(!global_bv_valid(&frame((16, 96), (4, 8)), sample(-64, 0)));
    }

    #[test]
    fn vectors_leave_neither_the_tile_nor_the_frame_top() {
        let leaf = frame((0, 80), (8, 8));
        assert!(!global_bv_valid(&leaf, [-8, -64 * 8 * 5]));
        let mut tiled = frame((16, 96), (8, 8));
        tiled.tile.1 = 64..160;
        assert!(!global_bv_valid(&tiled, [0, -(96 - 60) * 4 * 8]));
        assert!(global_bv_valid(&tiled, [-64 * 8, -(96 - 64) * 4 * 8]));
    }
}

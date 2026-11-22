// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Intra block copy of the encoder facade (§ 5.20.5.4): the block vectors
//! a leaf may code (§ 6.19.7.12 `is_mv_valid`), their syntax, the copy
//! prediction and the publication, through the decoder's intra-frame state.

use core::ops::Range;

use splot_core::symbol_encoder::SymbolSink;

use super::super::cdf::TileCdfSelector;
use super::super::partition_traversal::GeneralIntraLeafMode;
use super::{
    EncoderSyntaxError, IntraTileSyntax, LeafBlock, MotionVector, Result, TransformBlock,
    TxPartition, rule,
};
use crate::prediction::inter::MvdCoding;
use crate::prediction::inter::read_mv::{
    MV_PRECISION_ONE_PEL, MV_PRECISION_QUARTER_PEL, MvReadConfig,
};

/// The block vectors one leaf may code, see
/// [`IntraTileSyntax::intrabc_vectors`].
#[derive(Clone, Debug)]
pub struct IntrabcVectors(BvFrame, Option<LocalArea>, bool, bool);

impl IntrabcVectors {
    /// Whether the leaf may code the block vector `bv`.
    #[must_use]
    pub fn valid(&self, bv: MotionVector) -> bool {
        let mv = [bv.row, bv.col];
        if (bv.row | bv.col) & 7 != 0 && (!self.2 || (bv.row | bv.col) & 1 != 0) {
            return false;
        }
        self.1
            .as_ref()
            .is_some_and(|area| local_bv_valid(&self.0, area, mv))
            || self.3 && global_bv_valid(&self.0, mv)
    }

    /// Whether `bv` may code `morph_pred = 1`: § 6.19.5.4 requires
    /// `is_offset_mv_valid( -1, -1 )`, the vector one sample up and left,
    /// whose source holds the reference template.
    #[must_use]
    pub fn morph_valid(&self, bv: MotionVector) -> bool {
        self.valid(MotionVector {
            row: bv.row - 8,
            col: bv.col - 8,
        })
    }
}

/// An intra block copy of a leaf (§ 5.20.5.4).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IntrabcModes {
    /// The block vector in 1/8 samples: whole or quarter samples.
    pub bv: MotionVector,
    /// `RefMvIdx`, below the length of [`IntraTileSyntax::intrabc_ref_bvs`].
    pub drl: usize,
    /// `morph_pred`: the § 7.13.3.26 morphological luma prediction, only
    /// where [`IntraTileSyntax::intrabc_morph_coded`] says it is coded.
    pub morph: bool,
}

impl IntraTileSyntax {
    /// Whether `leaf` codes `use_intrabc` (§ 5.20.5.3, § 5.20.7.3): a frame
    /// with `allow_intrabc`, a leaf of at most 64x64 other than 64x64 that
    /// may code `is_inter = 0`, in a `MIXED_REGION` of an inter frame.
    #[must_use]
    pub fn intrabc_allowed(&self, leaf: &LeafBlock) -> bool {
        self.intra_allowed(leaf)
            && self.inter.as_ref().is_some_and(|inter| {
                matches!(inter.tile.intrabc_contexts(&leaf.frontier), Ok(Some(_)))
            })
    }

    /// Whether the frame codes `allow_intrabc` with the local area, whose
    /// vectors depend on the leaves coded before.
    #[must_use]
    pub fn intrabc_local_area(&self) -> bool {
        self.inter
            .as_ref()
            .is_some_and(|inter| inter.tile.intrabc_local_area())
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

    /// Whether IntraBC leaves code `morph_pred` (§ 5.20.5.4): an intra
    /// frame with `allow_screen_content_tools` and `enable_bawp`.
    #[must_use]
    pub fn intrabc_morph_coded(&self) -> bool {
        self.inter.as_ref().is_some_and(|inter| {
            inter.tile.is_intra_frame()
                && inter.tile.frame_header().allow_screen_content_tools == Some(true)
                && inter
                    .tile
                    .sequence_header()
                    .inter
                    .as_ref()
                    .is_some_and(|inter| inter.enable_bawp)
        })
    }

    /// The block vectors `leaf` may code, `None` when it codes no
    /// `use_intrabc`: vectors that § 6.19.7.12 `is_mv_valid` accepts, with
    /// `allow_global_intrabc` for the global area with the source in the
    /// current or the previous superblock row, with `allow_local_intrabc`
    /// (the only area of an inter frame) for the local area in a strict
    /// subset of § 6.19.7.12 `check_valid_local_ibc`.
    #[must_use]
    pub fn intrabc_vectors(&self, leaf: &LeafBlock) -> Option<IntrabcVectors> {
        let size = leaf.dimensions().ok()?;
        if !self.intrabc_allowed(leaf) {
            return None;
        }
        let tile = &self.inter.as_ref()?.tile;
        let geometry = leaf.frontier.chroma_ref_geometry();
        let chroma = leaf.frontier.has_chroma.then(|| {
            let chroma_size = geometry.size();
            let sdp = tile
                .sequence_header()
                .partition
                .is_some_and(|p| p.enable_sdp);
            ChromaRef {
                mi: (geometry.row(), geometry.col()),
                size: (
                    chroma_size.width_samples().unwrap_or(size.0),
                    chroma_size.height_samples().unwrap_or(size.1),
                ),
                local: !(sdp && tile.is_intra_frame()),
            }
        });
        let frame = BvFrame {
            mi: (leaf.row4(), leaf.col4()),
            size,
            tile: self.tile.clone(),
            sb_size: self.sb_size4 * 4,
            chroma,
        };
        let (global, local) = tile.intrabc_areas();
        let area = local.then(|| {
            LocalArea::new(frame.mi, frame.sb_size, |row, col| {
                tile.intrabc_coded_base_col(row, col)
            })
        });
        Some(IntrabcVectors(
            frame,
            area,
            !tile.force_integer_mv(),
            global,
        ))
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
        let vectors = self.intrabc_vectors(leaf);
        let valid = vectors.as_ref().is_some_and(|vectors| {
            vectors.valid(modes.bv) && (!modes.morph || vectors.morph_valid(modes.bv))
        });
        if !valid {
            return Err(EncoderSyntaxError::Unsupported("this IntraBC vector"));
        }
        let motion = self.inter_ref()?.tile.intrabc_motion(modes.drl);
        let intrabc = Some((modes.bv.into(), modes.morph));
        self.plan_motion_leaf(leaf, (motion, intrabc), (partition, split))
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
            Some(inter) => inter
                .tile
                .intrabc_contexts(&leaf.frontier)
                .map_err(rule("IntraBC contexts"))?,
            None => None,
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
    /// (quarter samples for a fractional vector), the vector difference and
    /// `morph_pred`.
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
        let force_integer_mv = tile.force_integer_mv();
        let morph_coded = self.intrabc_morph_coded();
        if modes.morph && !morph_coded {
            return Err(EncoderSyntaxError::Unsupported("morph_pred in this frame"));
        }
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
            let [row, col] = [modes.bv.row, modes.bv.col];
            let quarter = row & 7 != 0 || col & 7 != 0;
            if quarter && (force_integer_mv || row & 1 != 0 || col & 1 != 0) {
                return Err(EncoderSyntaxError::Unsupported("this IntraBC vector"));
            }
            if !force_integer_mv {
                self.put(
                    symbols,
                    TileCdfSelector::IntrabcPrecision,
                    usize::from(quarter),
                )?;
            }
            let (precision, base) = if quarter {
                (MV_PRECISION_QUARTER_PEL, reference)
            } else {
                (MV_PRECISION_ONE_PEL, lower_to_whole(reference))
            };
            let mvd = MotionVector {
                row: row - base.row,
                col: col - base.col,
            };
            let coding = MvdCoding {
                precision,
                use_amvd: false,
                derive_sign: false,
                derive_threshold: 1,
            };
            let config = MvReadConfig::intrabc(precision);
            super::mv::write_mvds_with(
                (&mut self.cdfs, &mut self.journal),
                (coding, config),
                &[mvd],
                symbols,
            )?;
        }
        if morph_coded {
            let ctx = contexts.morph_pred;
            self.put(
                symbols,
                TileCdfSelector::MorphPred { ctx },
                usize::from(modes.morph),
            )?;
        }
        Ok(())
    }

    /// Publishes the coded IntraBC leaf `leaf` with `skip_txfm = skip` into
    /// the neighbour, IntraBC, motion and `BlockDecoded` state.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] without a planned IntraBC leaf.
    pub fn publish_intrabc_leaf(&mut self, leaf: &LeafBlock, skip: bool) -> Result<()> {
        let (mv, morph) = self.take_planned_intrabc(leaf)?;
        let state = self.inter.as_mut().ok_or(NO_INTRABC)?;
        if skip {
            state
                .tile
                .reset_skip_contexts(&leaf.frontier, &mut self.coeff)
                .map_err(rule("coefficient contexts"))?;
        }
        let tile = &mut self.inter_state()?.tile;
        tile.publish_intrabc(&leaf.frontier, mv, (morph, skip))
            .map_err(rule("IntraBC publication"))?;
        self.publish_leaf_modes(leaf, GeneralIntraLeafMode::no_luma_mode().mark_intrabc())?;
        self.end_leaf_segment();
        Ok(())
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
    /// The chroma reference block of a leaf with `HasChroma`.
    pub(super) chroma: Option<ChromaRef>,
}

/// The block whose chroma a leaf with `HasChroma` codes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ChromaRef {
    /// `ChromaMiRow`, `ChromaMiCol`.
    pub(super) mi: (usize, usize),
    /// `Block_Width[ChromaMiSize]`, `Block_Height[ChromaMiSize]` in samples.
    pub(super) size: (usize, usize),
    /// `!enable_sdp || !FrameIsIntra`: the local area checks this block.
    pub(super) local: bool,
}

/// The 64x64 local IntraBC buffers (§ 6.19.7.12 `check_valid_local_ibc`)
/// a leaf copies from, each with the coded flags of its 16x16 4x4 cells.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct LocalArea {
    /// `(bufRow, bufCol)` and one bit per coded 4x4 cell, a `u16` per row.
    chunks: Vec<((usize, usize), [u16; 16])>,
}

/// 4x4 cells per side of an `IBC_BUFFER_SIZE` (§ 3) region.
const CHUNK4: usize = 16;

impl LocalArea {
    /// The buffers of the leaf at `mi` in superblocks of `sb_size` samples:
    /// its own 64x64 region; with 64x64 superblocks the three to its left;
    /// with 128x128 superblocks the top regions of the current and the
    /// previous superblock that a leaf entered (`update_ibc_buffers`) and no
    /// later region of their `ibc_buffer_index` replaced. `base_col` gives
    /// the base column of the coded block of a 4x4 cell, `None` uncoded.
    pub(super) fn new(
        mi: (usize, usize),
        sb_size: usize,
        base_col: impl Fn(usize, usize) -> Option<usize>,
    ) -> Self {
        let cur = (mi.0 / CHUNK4, mi.1 / CHUNK4);
        let entered = |(r, c): (usize, usize)| base_col(r * CHUNK4, c * CHUNK4) == Some(c * CHUNK4);
        let untouched = |(r, c): (usize, usize)| {
            (0..CHUNK4 * CHUNK4)
                .all(|i| base_col(r * CHUNK4 + i / CHUNK4, c * CHUNK4 + i % CHUNK4).is_none())
        };
        let index = |(r, c): (usize, usize)| (c & 1) | ((r & 1) << 1);
        let mut regions = vec![cur];
        match sb_size {
            64 => regions.extend((1..=3).filter_map(|k| Some((cur.0, cur.1.checked_sub(k)?)))),
            128 => {
                let (top, col0) = (cur.0 & !1, cur.1 & !1);
                let own = [col0, col0 + 1].map(|col| ((top, col), true));
                let previous = [2, 1].map(|k| {
                    let newer = (top, col0 + (k & 1));
                    ((top, col0.wrapping_sub(k)), col0 >= k && untouched(newer))
                });
                regions.extend(own.into_iter().chain(previous).filter_map(|(at, ok)| {
                    (ok && index(at) != index(cur) && entered(at)).then_some(at)
                }));
            }
            _ => {}
        }
        let chunks = regions
            .into_iter()
            .map(|(r, c)| {
                let rows = core::array::from_fn(|y| {
                    (0..CHUNK4).fold(0u16, |bits, x| {
                        let coded = base_col(r * CHUNK4 + y, c * CHUNK4 + x).is_some();
                        bits | (u16::from(coded) << x)
                    })
                });
                ((r, c), rows)
            })
            .collect();
        Self { chunks }
    }

    fn coded(&self, row: usize, col: usize) -> bool {
        let at = (row / CHUNK4, col / CHUNK4);
        self.chunks
            .iter()
            .find(|(chunk, _)| *chunk == at)
            .is_some_and(|(_, rows)| rows[row % CHUNK4] >> (col % CHUNK4) & 1 == 1)
    }
}

/// Whether the block vector `mv` passes § 6.19.7.12 `is_mv_valid`
/// through `check_valid_local_ibc` under a stricter rule: every 4x4 cell of
/// the checked block (the chroma reference block of a leaf with `HasChroma`
/// unless SDP codes the intra frame, else the leaf) moved by `mv`, with its
/// borders, is coded and lies in a buffer of `area`, and neither of
/// its corners lies at or right of and at or below that block's top-left
/// sample. Not used: the `coloY`/`coloX` case (a buffer that shares its
/// `ibc_buffer_index` with the current one), the lower 64x64 regions of a
/// previous 128x128 superblock, other superblock sizes' older regions, and
/// sources with an uncoded inner 4x4 cell.
pub(super) fn local_bv_valid(frame: &BvFrame, area: &LocalArea, mv: [i32; 2]) -> bool {
    if source_edges(frame, mv).is_none() {
        return false;
    }
    let (mi, (w, h)) = match frame.chroma {
        Some(chroma) if chroma.local => (chroma.mi, chroma.size),
        _ => (frame.mi, frame.size),
    };
    let (row0, col0) = (mi.0 as i64 * MI_SIZE, mi.1 as i64 * MI_SIZE);
    let (top, left) = (row0 + i64::from(mv[0] >> 3), col0 + i64::from(mv[1] >> 3));
    let [bottom, right] = borders(mv);
    let (bottom, right) = (top + h as i64 + bottom, left + w as i64 + right);
    if top < 0
        || left < 0
        || [(left, top), (right - 1, bottom - 1)]
            .iter()
            .any(|&(x, y)| x >= col0 && y >= row0)
    {
        return false;
    }
    let cells = |a: i64, b: i64| (a / MI_SIZE) as usize..=((b - 1) / MI_SIZE) as usize;
    cells(top, bottom).all(|row| cells(left, right).all(|col| area.coded(row, col)))
}

/// The § 6.19.7.12 `[bottomBorder, rightBorder]` of `mv`: one sample below
/// or right of the block for a fractional row or column.
fn borders(mv: [i32; 2]) -> [i64; 2] {
    mv.map(|c| i64::from(c & 7 != 0))
}

/// The § 6.19.7.12 `[srcTopEdge, srcLeftEdge, srcBottomEdge, srcRightEdge]`
/// of the block vector `mv`: the bottom and right edges of the moved leaf
/// plus their borders, the top and left edges of its moved chroma reference
/// block with `HasChroma`; `None` when they leave the tile.
fn source_edges(frame: &BvFrame, mv: [i32; 2]) -> Option<[i64; 4]> {
    let (delta_row, delta_col) = (i64::from(mv[0] >> 3), i64::from(mv[1] >> 3));
    let top = frame.mi.0 as i64 * MI_SIZE + delta_row;
    let left = frame.mi.1 as i64 * MI_SIZE + delta_col;
    let [bottom, right] = borders(mv);
    let (bottom, right) = (
        top + frame.size.1 as i64 + bottom,
        left + frame.size.0 as i64 + right,
    );
    let (top, left) = match frame.chroma {
        Some(chroma) => (
            chroma.mi.0 as i64 * MI_SIZE + delta_row,
            chroma.mi.1 as i64 * MI_SIZE + delta_col,
        ),
        None => (top, left),
    };
    let (rows, cols) = &frame.tile;
    let inside = top >= rows.start as i64 * MI_SIZE
        && left >= cols.start as i64 * MI_SIZE
        && bottom <= rows.end as i64 * MI_SIZE
        && right <= cols.end as i64 * MI_SIZE;
    inside.then_some([top, left, bottom, right])
}

/// Whether the block vector `mv` (`[row, col]` in 1/8 samples)
/// passes § 6.19.7.12 `is_mv_valid` for the global area (`allow_global_intrabc
/// = 1`), taking `sb64Residual = 0` (a subset of the vectors
/// `AllowExtraIBCRange` allows). The source must also lie in the current or
/// the previous superblock row, the global area of the frame header
/// semantics.
pub(super) fn global_bv_valid(frame: &BvFrame, mv: [i32; 2]) -> bool {
    let Some([src_top, _, src_bottom, src_right]) = source_edges(frame, mv) else {
        return false;
    };
    let (mi_row, mi_col) = (frame.mi.0 as i64, frame.mi.1 as i64);
    let cols = &frame.tile.1;
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
            chroma: Some(ChromaRef {
                mi,
                size,
                local: true,
            }),
        }
    }

    /// A 4x4 leaf at `mi` that codes the chroma of the 8x8 block at `chroma`.
    fn offset_leaf(mi: (usize, usize), chroma: (usize, usize)) -> BvFrame {
        BvFrame {
            chroma: Some(ChromaRef {
                mi: chroma,
                size: (8, 8),
                local: true,
            }),
            ..frame(mi, (4, 4))
        }
    }

    /// § 6.19.7.12 for a 16x8 leaf at (64, 384) of a 640-wide tile of 64x64
    /// superblocks: in its own row the source must end four 64-wide columns
    /// before the current one; in the row above, before the wavefront. The
    /// right border of a fractional column counts.
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
        assert!(global_bv_valid(&leaf, [0, -272 * 8 - 6]));
        assert!(!global_bv_valid(&leaf, [0, -272 * 8 + 2]));
    }

    /// A fractional vector needs `force_integer_mv = 0` and quarter samples.
    #[test]
    fn fractional_vectors_need_quarter_samples() {
        let vectors = |quarter| IntrabcVectors(frame((16, 96), (16, 8)), None, quarter, true);
        let col = |col| MotionVector { row: 0, col };
        assert!(vectors(true).valid(col(-272 * 8 - 6)));
        assert!(!vectors(false).valid(col(-272 * 8 - 6)));
        assert!(!vectors(true).valid(col(-272 * 8 - 5)));
        assert!(vectors(false).valid(col(-272 * 8)));
    }

    /// § 6.19.7.12 `HasChroma`: a chroma-offset leaf takes the top and left
    /// source edges from its chroma reference block, a leaf without chroma
    /// from itself.
    #[test]
    fn small_leaves_take_the_source_corner_of_their_chroma_block() {
        let sample = |dy: i32, dx: i32| [dy * 8, dx * 8];
        let no_chroma = BvFrame {
            chroma: None,
            ..frame((17, 97), (4, 4))
        };
        assert!(global_bv_valid(&no_chroma, sample(-68, -4)));
        assert!(!global_bv_valid(
            &offset_leaf((17, 97), (16, 96)),
            sample(-68, -4)
        ));
        assert!(global_bv_valid(
            &offset_leaf((17, 97), (16, 96)),
            sample(-64, -4)
        ));
    }

    /// The buffers of a [`LocalArea`] whose coded 4x4 cells `coded` gives,
    /// each cell in a block that starts at the left column of its 64x64
    /// region, or at `wide` for a cell of the 128x64 block there.
    fn regions(
        mi: (usize, usize),
        sb_size: usize,
        coded: impl Fn(usize, usize) -> bool,
        wide: Option<(usize, usize)>,
    ) -> (LocalArea, Vec<(usize, usize)>) {
        let area = LocalArea::new(mi, sb_size, |row, col| {
            let base = match wide {
                Some((r, c)) if row / 16 == r && col / 32 == c / 2 => c * 16,
                _ => col / 16 * 16,
            };
            coded(row, col).then_some(base)
        });
        let chunks = area.chunks.iter().map(|&(at, _)| at).collect();
        (area, chunks)
    }

    /// § 6.19.7.12 with 64x64 superblocks: a 16x16 leaf in the lower half of
    /// the region at (64, 256) copies from its own coded top half and from
    /// the three regions to its left; the fourth region to the left shares
    /// the current `ibc_buffer_index` (the `coloY` case) and is refused, as
    /// is a source with an uncoded 4x4 cell, also in the bottom border of a
    /// fractional row.
    #[test]
    fn local_vectors_use_the_last_four_regions_of_the_row() {
        let leaf = frame((24, 64), (16, 16));
        let coded = |row: usize, col: usize| row < 16 || col < 64 || row < 24;
        let (area, chunks) = regions(leaf.mi, 64, coded, None);
        assert_eq!(chunks, [(1, 4), (1, 3), (1, 2), (1, 1)]);
        let sample = |dy: i32, dx: i32| [dy * 8, dx * 8];
        assert!(local_bv_valid(&leaf, &area, sample(-32, 0)));
        assert!(local_bv_valid(&leaf, &area, sample(-16, -64)));
        assert!(local_bv_valid(&leaf, &area, sample(0, -192)));
        assert!(!local_bv_valid(&leaf, &area, sample(0, -256)));
        assert!(!global_bv_valid(&leaf, sample(0, -256)));
        assert!(!local_bv_valid(&leaf, &area, sample(-8, 0)));
        assert!(local_bv_valid(&leaf, &area, sample(-32, 8)));
        assert!(local_bv_valid(&leaf, &area, [-32 * 8 + 4, 0]));
        assert!(local_bv_valid(&leaf, &area, sample(-16, 0)));
        assert!(!local_bv_valid(&leaf, &area, [-16 * 8 + 2, 0]));
    }

    /// The local area checks the moved chroma reference block of a
    /// chroma-offset leaf: its luma copy at (100, 252) is coded, but the
    /// 8x8 chroma block at (96, 248) is not.
    #[test]
    fn local_vectors_of_chroma_offset_leaves_check_the_chroma_block() {
        let coded = |row: usize, col: usize| row < 24 || (col < 64 && row != 24);
        let (area, _) = regions((25, 65), 64, coded, None);
        let sample = |dy: i32, dx: i32| [dy * 8, dx * 8];
        let luma_only = BvFrame {
            chroma: None,
            ..frame((25, 65), (4, 4))
        };
        assert!(local_bv_valid(&luma_only, &area, sample(0, -8)));
        assert!(!local_bv_valid(
            &offset_leaf((25, 65), (24, 64)),
            &area,
            sample(0, -8)
        ));
        assert!(local_bv_valid(
            &offset_leaf((25, 65), (24, 64)),
            &area,
            sample(-8, -8)
        ));
    }

    /// § 6.19.7.12 with 128x128 superblocks: a leaf in the top-left region of
    /// a superblock copies from the top-right region of the previous one,
    /// whose buffer no newer region replaced; in the lower-left region after
    /// a vertical split it also copies from its own top-left region, not
    /// from the previous top-left one that region replaced. A region that a
    /// 128x64 block covers is not entered and stays out.
    #[test]
    fn local_regions_of_128_superblocks_follow_the_buffer_index() {
        let before = |row: usize, col: usize| row < 32 || col < 64;
        let (_, chunks) = regions((32, 64), 128, before, None);
        assert_eq!(chunks, [(2, 4), (2, 3)]);
        let vertical = |row: usize, col: usize| before(row, col) || (row < 64 && col < 80);
        let (_, chunks) = regions((48, 64), 128, vertical, None);
        assert_eq!(chunks, [(3, 4), (2, 4), (2, 3)]);
        let wide = |row: usize, col: usize| before(row, col) || row < 48;
        let (_, chunks) = regions((48, 64), 128, wide, Some((2, 4)));
        assert_eq!(chunks, [(3, 4), (2, 4)]);
    }

    /// `morph_pred = 1` needs the vector one sample up and left to be valid
    /// too: a copy from the frame's top row cannot code it.
    #[test]
    fn morph_needs_the_template_source() {
        let vectors = IntrabcVectors(frame((0, 80), (8, 8)), None, true, true);
        let bv = |row, col| MotionVector { row, col };
        assert!(vectors.valid(bv(0, -64 * 8 * 5)));
        let local_only = IntrabcVectors(frame((0, 80), (8, 8)), None, true, false);
        assert!(!local_only.valid(bv(0, -64 * 8 * 5)));
        assert!(!vectors.morph_valid(bv(0, -64 * 8 * 5)));
        let lower = IntrabcVectors(frame((16, 96), (16, 8)), None, true, true);
        assert!(lower.morph_valid(bv(0, -272 * 8 - 8)));
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

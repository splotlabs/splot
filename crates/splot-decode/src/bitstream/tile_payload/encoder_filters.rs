// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Encoder-facing loop-filter facade (deblocking, CDEF, CCSO, loop
//! restoration); not a stable API.
//!
//! - [`FilterRecorder`] keeps the per-transform records the decoder's walk keeps.
//!   Call [`FilterRecorder::record_leaf`] once per coded leaf, after the leaf's
//!   residual is coded, with every transform block of the leaf in § 5.20.7.23
//!   order and the `eob` of each.
//! - [`CdefIndexMap`] is the tile's `cdef_idx` state of § 5.20.10.1: it names the
//!   leaf that codes each 64x64 index and the `cdef_index0` context.
//! - [`LrLayout`] lists the § 5.20.10.4 restoration units of each superblock.
//! - [`CcsoBlockMap`] is the tile's `ccso_blk` state of § 5.20.10.2, and
//!   [`CcsoGrid`] the frame's CCSO unit grid.
//! - [`apply_loop_filters`] runs the decoder's own § 7.2 filter chain on a frame.
//! - [`merge_tiles`] puts the tiles of a frame, each coded by its own facade,
//!   into one reconstruction.

use std::sync::Arc;

use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::sequence::{ChromaFormatIdc, SequenceHeader};
use splot_recon::{CurrentFrameWorkspace, DecodedFrame, PlaneId, PlaneRect};

use splot_core::span::ByteOffset;

use super::super::cdf::TileCdfSelector;
use super::super::partition_traversal::{
    LrUnitRestorationType, ROOT_HAS_CHROMA, TilePartitionBounds, TilePartitionCall,
    TilePartitionFrameFacts, pc_wiener_unit_source_blocks, superblock_lr_units,
};
use super::super::tile_frontier::minimal_partition_frame_facts;
use super::{
    EncoderSyntaxError, FrameStart, IntraTileSyntax, LeafBlock, PartitionNode, Result,
    TransformBlock, rule, tile_count, tile_ranges, tx_log2,
};
use crate::filters::ccso::{CCSO_OFFSET, CcsoUnitGrid, ccso_sample_offsets};
use crate::filters::deblock::{ChromaDeblockRecords, DeblockBlock};
use crate::filters::wienerns_lr::tx_records::CdefState;
use crate::filters::wienerns_lr::tx_records::ccso::{CcsoState, ccso_mi_width_log2};
use crate::filters::wienerns_lr::{FrameFilterRecords, WienerNsLrTxSkipTransformRecord};

/// `cdef_idx` units are 64x64 luma samples, 16 4x4 units a side (§ 5.20.10.1).
const CDEF_UNIT_MI: usize = 16;

impl IntraTileSyntax {
    /// The `cdef_index0` CDF row for context `ctx` (§ 5.20.10.1).
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] for an invalid context.
    pub fn cdef_index0_cdf(&mut self, ctx: usize) -> Result<&mut [u16]> {
        self.tile_row(TileCdfSelector::CdefIndex0 { ctx })
    }

    /// The `cdef_index_minus_1` CDF row for `CdefStrengths` sets (§ 5.20.10.1).
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] for a count outside `3..=8`.
    pub fn cdef_index_minus1_cdf(&mut self, strengths: usize) -> Result<&mut [u16]> {
        self.tile_row(TileCdfSelector::CdefIndexMinus1 { strengths })
    }

    /// The CDF row of the `read_lr_unit` symbol `symbol` (§ 5.20.10.5).
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] for an invalid tool or plane.
    pub fn lr_symbol_cdf(&mut self, symbol: LrSymbol) -> Result<&mut [u16]> {
        self.tile_row(symbol.selector())
    }

    /// The `ccso_blk` CDF row of `plane` for context `ctx` (§ 5.20.10.2).
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] for an invalid plane or context.
    pub fn ccso_blk_cdf(&mut self, plane: usize, ctx: usize) -> Result<&mut [u16]> {
        self.tile_row(TileCdfSelector::CcsoBlk { plane, ctx })
    }
}

impl FrameStart {
    /// A copy of the `ccso_blk` CDF row of `plane` for context `ctx` that
    /// every tile of the frame starts from (§ 5.20.10.2).
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] for an invalid plane or context.
    pub fn ccso_blk_cdf(&self, plane: usize, ctx: usize) -> Result<Vec<u16>> {
        let mut cdfs = self.cdfs.tile_copy();
        let row = cdfs
            .row_mut(TileCdfSelector::CcsoBlk { plane, ctx })
            .map_err(rule("CDF row"))?;
        Ok(row.to_vec())
    }
}

impl FrameStart {
    /// A copy of the CDF row of `symbol` that every tile of the frame starts
    /// from (§ 5.20.10.5).
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] for an invalid tool or plane.
    pub fn lr_symbol_cdf(&self, symbol: LrSymbol) -> Result<Vec<u16>> {
        let mut cdfs = self.cdfs.tile_copy();
        let row = cdfs.row_mut(symbol.selector()).map_err(rule("CDF row"))?;
        Ok(row.to_vec())
    }
}

/// A `read_lr_unit` symbol (§ 5.20.10.5).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum LrSymbol {
    /// `use_pc_wiener` of a `RESTORE_PC_WIENER` plane.
    UsePcWiener,
    /// `use_wiener_ns` of a `RESTORE_WIENER_NONSEP` plane.
    UseWienerNs,
    /// `flex_restoration_type` of `tool` (`0` or `1`) of a `RESTORE_SWITCHABLE` plane.
    Flex {
        /// The tool the symbol selects when it is `1`.
        tool: usize,
        /// The plane.
        plane: usize,
    },
}

impl LrSymbol {
    const fn selector(self) -> TileCdfSelector {
        match self {
            Self::UsePcWiener => TileCdfSelector::UsePcWiener,
            Self::UseWienerNs => TileCdfSelector::UseWienerNs,
            Self::Flex { tool, plane } => TileCdfSelector::FlexRestorationType { tool, plane },
        }
    }
}

/// The restoration a unit applies (§ 5.20.10.5 `LrType`), other than `RESTORE_NONE`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum LrUnitType {
    /// `RESTORE_PC_WIENER`.
    PcWiener,
    /// `RESTORE_WIENER_NONSEP` with the frame filters.
    WienerNs,
}

/// The CCSO unit grid of a frame (§ 5.18.7.12): units of
/// `1 << luma_size_log2` luma samples in raster order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CcsoGrid {
    /// `CcsoLumaSizeLog2`.
    pub luma_size_log2: u32,
    /// Unit rows.
    pub rows: usize,
    /// Unit columns.
    pub cols: usize,
}

impl CcsoGrid {
    /// The grid of the frame `core` coded with `seq`.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] for missing frame facts.
    pub fn new(seq: &SequenceHeader, core: &FrameHeaderCore) -> Result<Self> {
        let shift = ccso_mi_width_log2(seq, core).map_err(rule("CCSO unit size"))?;
        let frame = minimal_partition_frame_facts(seq, core).map_err(rule("frame facts"))?;
        Ok(Self {
            luma_size_log2: shift + 2,
            rows: frame.mi_rows().div_ceil(1 << shift),
            cols: frame.mi_cols().div_ceil(1 << shift),
        })
    }
}

/// `CcsoFilterOffset` of `ccso_offset_idx` at `ccso_scale_idx` (§ 5.18.7.12),
/// or `None` outside `0..8`.
#[must_use]
pub fn ccso_offset(offset_idx: u8, scale_idx: u8) -> Option<i32> {
    CCSO_OFFSET
        .get(usize::from(offset_idx))
        .map(|&offset| offset * (i32::from(scale_idx) + 1))
}

/// The luma taps `(dx, dy)` of `cls0` and `cls1` for `ccso_ext_filter` (§ 7.19.1).
///
/// # Errors
/// Returns [`EncoderSyntaxError::Rule`] for a filter outside `0..7`.
pub fn ccso_taps(ext_filter: u8) -> Result<[(isize, isize); 2]> {
    ccso_sample_offsets(ext_filter).map_err(rule("ccso_ext_filter"))
}

/// The `ccso_blk` state of one tile of a frame (§ 5.20.10.2), filled in
/// coding order.
pub struct CcsoBlockMap {
    state: CcsoState,
    tile_col_start: usize,
}

impl CcsoBlockMap {
    /// An empty map for tile `tile` of the frame `core` coded with `seq`.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for missing frame facts, a tile the
    /// frame does not have, or CCSO block reuse.
    pub fn new(seq: &SequenceHeader, core: &FrameHeaderCore, tile: usize) -> Result<Self> {
        let frame = minimal_partition_frame_facts(seq, core).map_err(rule("frame facts"))?;
        let (rows, cols) = tile_ranges(core, tile)?;
        let tile_col_start = cols.start;
        let state = CcsoState::new(
            frame.mi_rows(),
            frame.mi_cols(),
            seq,
            core,
            &[],
            &[],
            ByteOffset::new(0),
            Default::default(),
        )
        .and_then(|state| state.try_for_tile(rows, cols, None))
        .map_err(rule("CCSO state"))?;
        Ok(Self {
            state,
            tile_col_start,
        })
    }

    /// The `(unitRow, unitCol)` whose `ccso_blk` `leaf` codes, if any.
    #[must_use]
    pub fn unit(&self, leaf: &LeafBlock) -> Option<(usize, usize)> {
        self.state.unit_at(leaf.row4(), leaf.col4())
    }

    /// The units of the tile in raster order, which is their coding order:
    /// a unit is never smaller than a superblock.
    #[must_use]
    pub fn units(&self) -> Vec<(usize, usize)> {
        let state = &self.state;
        (state.row_start..state.row_start + state.grid_rows)
            .flat_map(|row| {
                (state.col_start..state.col_start + state.grid_cols).map(move |col| (row, col))
            })
            .collect()
    }

    /// The `ccso_blk` context of `plane` at `unit`, or `None` when the plane
    /// codes no flag.
    #[must_use]
    pub fn ctx(&self, plane: usize, unit: (usize, usize)) -> Option<usize> {
        self.state.blk_ctx(plane, unit, self.tile_col_start)
    }

    /// Stores `ccso_blk` of `plane` at `unit` after it is coded.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] for a unit outside the tile.
    pub fn set(&mut self, plane: usize, unit: (usize, usize), on: bool) -> Result<()> {
        self.state
            .set_block_value(plane, unit.0, unit.1, u8::from(on))
            .map_err(rule("ccso_blk"))
    }
}

/// The deblocking and loop-restoration records of every coded transform block.
#[derive(Clone)]
pub struct FilterRecorder {
    qindex: u32,
    subsampling: (u32, u32),
    deblock_blocks: Vec<DeblockBlock>,
    chroma_blocks: ChromaDeblockRecords,
    tx_skip: Vec<WienerNsLrTxSkipTransformRecord>,
}

impl FilterRecorder {
    /// An empty recorder for the frame `core` coded with `seq`.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Unsupported`] without quantization params.
    pub fn new(seq: &SequenceHeader, core: &FrameHeaderCore) -> Result<Self> {
        let qindex = core
            .quantization_params
            .ok_or(EncoderSyntaxError::Unsupported(
                "missing quantization params",
            ))?
            .base_q_idx;
        let subsampling = match seq.general.chroma_format_idc {
            ChromaFormatIdc::Yuv420 | ChromaFormatIdc::Monochrome => (1, 1),
            ChromaFormatIdc::Yuv422 => (1, 0),
            ChromaFormatIdc::Yuv444 => (0, 0),
        };
        Ok(Self {
            qindex,
            subsampling,
            deblock_blocks: Vec::new(),
            chroma_blocks: ChromaDeblockRecords::new(),
            tx_skip: Vec::new(),
        })
    }

    /// Records one coded leaf: `transforms` are all its transform blocks in
    /// § 5.20.7.23 order and `eobs[i]` is the `eob` of `transforms[i]`.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Unsupported`] when the slices differ in length
    /// or a transform size is unknown.
    pub fn record_leaf(
        &mut self,
        leaf: &LeafBlock,
        transforms: &[TransformBlock],
        eobs: &[usize],
    ) -> Result<()> {
        if transforms.len() != eobs.len() {
            return Err(EncoderSyntaxError::Unsupported(
                "one eob per transform block",
            ));
        }
        let mut recorder = crate::residual::pipeline::DeblockRecorder {
            blocks: &mut self.deblock_blocks,
            chroma_blocks: &mut self.chroma_blocks,
            tx_skip_records: &mut self.tx_skip,
            block_r: leaf.row4(),
            block_c: leaf.col4(),
            chroma_tx: transforms
                .iter()
                .find(|tx| tx.plane == 1)
                .map(|tx| tx.tx_size),
            chroma_subsampling: self.subsampling,
            qindex: self.qindex,
            lossless: false,
        };
        for (tx, &eob) in transforms.iter().zip(eobs) {
            if tx.plane == 0 {
                let (log2_w, log2_h) = tx_log2(tx.tx_size)?;
                recorder.record_luma_unit(
                    tx.y / 4,
                    tx.x / 4,
                    (1 << log2_w) / 4,
                    (1 << log2_h) / 4,
                    tx.tx_size,
                    eob,
                );
            } else {
                let plane = if tx.plane == 1 {
                    PlaneId::U
                } else {
                    PlaneId::V
                };
                recorder.record_chroma_unit(plane, tx.x, tx.y, tx.tx_size);
            }
        }
        Ok(())
    }

    /// Adds the records of `other`, a later tile of the same frame.
    pub fn append(&mut self, mut other: Self) {
        self.deblock_blocks.append(&mut other.deblock_blocks);
        self.chroma_blocks.append(&mut other.chroma_blocks);
        self.tx_skip.append(&mut other.tx_skip);
    }

    /// Records the planned inter leaf `leaf` of `syntax`, coded with `skip_txfm
    /// = skip`; call it before [`IntraTileSyntax::publish_inter_leaf`].
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] without a planned inter leaf.
    pub fn record_inter_leaf(
        &mut self,
        syntax: &IntraTileSyntax,
        leaf: &LeafBlock,
        skip: bool,
    ) -> Result<()> {
        syntax.record_inter_filters(
            leaf,
            skip,
            (
                &mut self.deblock_blocks,
                &mut self.chroma_blocks,
                &mut self.tx_skip,
            ),
        )
    }
}

/// A leaf that codes `cdef_idx` (§ 5.20.10.1).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CdefSlot {
    /// `CdefStrengths`; `1` codes no symbol.
    pub strengths: usize,
    /// The `cdef_index0` context.
    pub ctx: usize,
    /// Row of the 64x64 unit.
    pub unit_row: usize,
    /// Column of the 64x64 unit.
    pub unit_col: usize,
}

/// The `cdef_idx[][]` state of one tile of a frame, filled in coding order.
pub struct CdefIndexMap {
    state: CdefState,
    strengths: usize,
    skip_codes_index: bool,
    rows: usize,
    cols: usize,
    origin: (usize, usize),
}

impl CdefIndexMap {
    /// An empty map for tile `tile` of the frame `core` coded with `seq`.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for missing frame facts or a tile the
    /// frame does not have.
    pub fn new(seq: &SequenceHeader, core: &FrameHeaderCore, tile: usize) -> Result<Self> {
        let (tile_rows, tile_cols) = tile_ranges(core, tile)?;
        let frame = minimal_partition_frame_facts(seq, core).map_err(rule("frame facts"))?;
        let strengths = core
            .cdef_params
            .as_ref()
            .filter(|cdef| cdef.cdef_frame_enable)
            .and_then(|cdef| cdef.cdef_strengths)
            .map_or(0, usize::from);
        let (rows, cols) = cdef_unit_grid((frame.mi_rows(), frame.mi_cols()));
        Ok(Self {
            state: CdefState::new_reusing(
                frame.mi_rows(),
                frame.mi_cols(),
                seq,
                core.frame_is_intra == Some(true),
                Vec::new(),
            )
            .map_err(rule("cdef state"))?,
            strengths,
            skip_codes_index: core
                .cdef_params
                .as_ref()
                .is_none_or(|cdef| cdef.cdef_on_skip_txfm_frame_enable != Some(false)),
            rows,
            cols,
            origin: (tile_rows.start, tile_cols.start),
        })
    }

    /// The slot `leaf` codes with `skip_txfm = skip`, or `None` when CDEF is
    /// off, the frame codes no index for a skipped block, or its unit has an index.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] for a leaf outside the frame.
    pub fn slot(&self, leaf: &LeafBlock, skip: bool) -> Result<Option<CdefSlot>> {
        if self.strengths == 0 || (skip && !self.skip_codes_index) {
            return Ok(None);
        }
        let (r, c) = (leaf.row4(), leaf.col4());
        let (unit_row, unit_col) = (r / CDEF_UNIT_MI, c / CDEF_UNIT_MI);
        if self
            .state
            .value(unit_row, unit_col)
            .map_err(rule("cdef_idx"))?
            .is_some()
        {
            return Ok(None);
        }
        let ctx = self
            .state
            .cdef_index0_ctx_at(r, c, self.origin.0, self.origin.1)
            .map_err(rule("cdef_index0 context"))?;
        Ok(Some(CdefSlot {
            strengths: self.strengths,
            ctx,
            unit_row,
            unit_col,
        }))
    }

    /// Stores `index` for every unit `leaf` covers, after its slot is coded.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Unsupported`] for an index outside the
    /// strength sets, and [`EncoderSyntaxError::Rule`] for an invalid leaf.
    pub fn set(&mut self, leaf: &LeafBlock, index: usize) -> Result<()> {
        if index >= self.strengths {
            return Err(EncoderSyntaxError::Unsupported("cdef_idx out of range"));
        }
        let size = leaf.frontier.b_size;
        let n4w = size.num_4x4_wide().map_err(rule("block size"))?;
        let n4h = size.num_4x4_high().map_err(rule("block size"))?;
        self.state
            .fill_units(leaf.row4(), leaf.col4(), n4w, n4h, index)
            .map_err(rule("cdef_idx"))
    }

    /// The `(rows, cols)` of the 64x64 unit grid.
    #[must_use]
    pub const fn grid(&self) -> (usize, usize) {
        (self.rows, self.cols)
    }

    /// The coded `cdef_idx` per unit in raster order, `None` where none is coded.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] for a corrupt state.
    pub fn into_indices(self) -> Result<Vec<Option<usize>>> {
        Ok(self
            .state
            .into_grid()
            .map_err(rule("cdef_idx"))?
            .into_values())
    }
}

/// The `(rows, cols)` of the 64x64 `cdef_idx` grid of `(MiRows, MiCols)`.
#[must_use]
pub const fn cdef_unit_grid((mi_rows, mi_cols): (usize, usize)) -> (usize, usize) {
    (
        mi_rows.div_ceil(CDEF_UNIT_MI),
        mi_cols.div_ceil(CDEF_UNIT_MI),
    )
}

/// One `read_lr_unit` call (§ 5.20.10.5).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct LrUnit {
    /// The tile that codes the unit, in raster order.
    pub tile: usize,
    /// Plane index.
    pub plane: usize,
    /// `unitRow`.
    pub row: usize,
    /// `unitCol`.
    pub col: usize,
}

/// The restoration-unit layout of one tile of a frame. A `RESTORE_WIENER_NONSEP`
/// or `RESTORE_SWITCHABLE` plane must code `frame_filters_on = 1`, so a unit codes
/// its type only.
pub struct LrLayout {
    frame: TilePartitionFrameFacts,
    bounds: TilePartitionBounds,
    tile: usize,
    rows: core::ops::Range<usize>,
    cols: core::ops::Range<usize>,
}

impl LrLayout {
    /// The layout of tile `tile` of the frame `core` coded with `seq`.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for missing frame facts or a tile the
    /// frame does not have.
    pub fn new(seq: &SequenceHeader, core: &FrameHeaderCore, tile: usize) -> Result<Self> {
        let frame = minimal_partition_frame_facts(seq, core).map_err(rule("frame facts"))?;
        let (rows, cols) = tile_ranges(core, tile)?;
        let bounds = TilePartitionBounds::new(rows.clone(), cols.clone());
        Ok(Self {
            frame,
            bounds,
            tile,
            rows,
            cols,
        })
    }

    /// The layouts of every tile of the frame `core`, in tile order.
    ///
    /// # Errors
    /// As [`Self::new`].
    pub fn tiles(seq: &SequenceHeader, core: &FrameHeaderCore) -> Result<Vec<Self>> {
        (0..tile_count(core))
            .map(|tile| Self::new(seq, core, tile))
            .collect()
    }

    /// The units `read_lr` codes at superblock `root`, in syntax order.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] for an invalid root.
    pub fn units(&self, root: PartitionNode) -> Result<Vec<LrUnit>> {
        Ok(superblock_lr_units(self.frame, root.0, self.bounds)
            .map_err(rule("read_lr"))?
            .into_iter()
            .map(|(plane, row, col, _)| LrUnit {
                tile: self.tile,
                plane,
                row,
                col,
            })
            .collect())
    }

    /// The units of every superblock of the tile, in coding order.
    ///
    /// # Errors
    /// As [`Self::units`].
    pub fn all_units(&self) -> Result<Vec<LrUnit>> {
        let sb4 = self
            .frame
            .sb_size()
            .num_4x4_wide()
            .map_err(rule("superblock size"))?;
        let mut units = Vec::new();
        for row4 in self.rows.clone().step_by(sb4) {
            for col4 in self.cols.clone().step_by(sb4) {
                let root =
                    TilePartitionCall::root(row4, col4, self.frame.sb_size(), ROOT_HAS_CHROMA);
                units.extend(self.units(PartitionNode(root))?);
            }
        }
        Ok(units)
    }

    /// The plane rectangles a unit of the tile filters, which can reach past
    /// the visible frame.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for a unit outside the tile.
    pub fn unit_rects(&self, unit: LrUnit) -> Result<Vec<PlaneRect>> {
        self.source_blocks(unit, LrUnitType::PcWiener)?
            .into_iter()
            .map(|block| {
                PlaneRect::new(block.x, block.y, block.width, block.height)
                    .map_err(rule("restoration unit"))
            })
            .collect()
    }

    fn source_blocks(
        &self,
        unit: LrUnit,
        kind: LrUnitType,
    ) -> Result<Vec<crate::bitstream::tile_payload::WienerNsLrSourceBlock>> {
        if unit.tile != self.tile {
            return Err(EncoderSyntaxError::Unsupported(
                "a restoration unit of another tile",
            ));
        }
        let mut blocks =
            pc_wiener_unit_source_blocks(self.frame, self.bounds, unit.plane, unit.row, unit.col)
                .map_err(rule("restoration unit"))?;
        if kind == LrUnitType::WienerNs {
            for block in &mut blocks {
                block.restoration_type = LrUnitRestorationType::WienerNonsep;
            }
        }
        Ok(blocks)
    }
}

/// Runs the decoder's deblocking, CDEF, and loop-restoration chain (§ 7.2) of
/// the frame `core` on `frame`, as a decoder of the same tile would.
///
/// `cdef_indices` holds `cdef_idx` per 64x64 unit in raster order (see
/// [`cdef_unit_grid`]), or is empty when no unit codes one; `ccso_blocks[p]`
/// holds `ccso_blk` of plane `p` per [`CcsoGrid`] unit in raster order, or is
/// empty when every unit is off; `lr_on` lists the restoring units and their type.
///
/// # Errors
/// Returns [`EncoderSyntaxError::Rule`] when a filter stage rejects its inputs.
pub fn apply_loop_filters(
    seq: &SequenceHeader,
    core: &FrameHeaderCore,
    recorder: &FilterRecorder,
    frame: &DecodedFrame<u16>,
    (cdef_indices, ccso_blocks): (&[Option<usize>], &[Vec<u8>]),
    lr_on: &[(LrUnit, LrUnitType)],
) -> Result<DecodedFrame<u16>> {
    let layouts = LrLayout::tiles(seq, core)?;
    let mut lr_source_blocks = Vec::new();
    for &(unit, kind) in lr_on {
        let layout = layouts
            .get(unit.tile)
            .ok_or(EncoderSyntaxError::Unsupported(
                "a restoration unit of another tile",
            ))?;
        lr_source_blocks.extend(layout.source_blocks(unit, kind)?);
    }
    let frame_facts = minimal_partition_frame_facts(seq, core).map_err(rule("frame facts"))?;
    let (rows, cols) = cdef_unit_grid((frame_facts.mi_rows(), frame_facts.mi_cols()));
    let cdef_indices = if cdef_indices.is_empty() {
        vec![None; rows * cols]
    } else {
        cdef_indices.to_vec()
    };
    let cdef_grid = crate::filters::cdef::CdefUnitGrid::new(rows, cols, cdef_indices)
        .map_err(rule("cdef_idx grid"))?;
    let mut workspace = CurrentFrameWorkspace::new(frame.info(), 0).map_err(rule("workspace"))?;
    copy_luma_rect(&mut workspace, frame, (0, 0, usize::MAX, usize::MAX))?;
    let luma = frame.coded_luma_size();
    let mut sink = crate::filters::wienerns_lr::recon_final_filter_sink(
        workspace,
        luma.width(),
        luma.height(),
        frame.bit_depth(),
    );
    sink.set_filter_records(FrameFilterRecords {
        deblock_blocks: recorder.deblock_blocks.clone(),
        chroma_deblock_blocks: recorder.chroma_blocks.clone(),
        tx_skip_records: recorder.tx_skip.clone(),
        lr_source_blocks,
        ..FrameFilterRecords::default()
    });
    sink.set_cdef_grid(Some(cdef_grid));
    sink.set_ccso_grid(ccso_unit_grid(seq, core, ccso_blocks)?.map(Arc::new));
    sink.set_cfl_ds_filter_index(seq.intra.as_ref().map_or(0, |i| i.cfl_ds_filter_index));
    let (filtered, _) = sink
        .into_filtered_frame(
            Arc::new(core.clone()),
            seq.filter
                .is_some_and(|filter| filter.disable_loopfilters_across_tiles),
            crate::pipeline::deblock_quant_deltas(seq, core),
            None,
            core::convert::identity,
        )
        .map_err(rule("loop filters"))?;
    Ok(filtered)
}

/// The decoder's CCSO grid of the frame `core` with `blocks`, or `None` when
/// the frame codes no CCSO.
fn ccso_unit_grid(
    seq: &SequenceHeader,
    core: &FrameHeaderCore,
    blocks: &[Vec<u8>],
) -> Result<Option<CcsoUnitGrid>> {
    let Some(params) = core
        .ccso_params
        .as_ref()
        .filter(|params| params.ccso_frame_flag == Some(true))
    else {
        return Ok(None);
    };
    let grid = CcsoGrid::new(seq, core)?;
    let cells = grid.rows * grid.cols;
    let mut planes: [Vec<u8>; 3] = Default::default();
    for (plane, target) in planes.iter_mut().enumerate() {
        *target = match blocks.get(plane) {
            Some(values) if values.len() == cells => values.clone(),
            Some(values) if !values.is_empty() => {
                return Err(EncoderSyntaxError::Unsupported(
                    "ccso_blk grid of another size",
                ));
            }
            _ => vec![0; cells],
        };
    }
    let enabled =
        core::array::from_fn(|plane| params.planes.get(plane).is_some_and(|p| p.ccso_planes));
    CcsoUnitGrid::new(
        true,
        grid.luma_size_log2 - 2,
        enabled,
        planes,
        grid.rows,
        grid.cols,
    )
    .map(Some)
    .map_err(rule("CCSO grid"))
}

/// The § 7.20.4 PC-Wiener class `cls` of every 4x4 luma cell of the frame
/// `core`, row by row over `(MiRows, MiCols)`, as its loop restoration
/// classifies the luma of `frame`. `frame` is read for every § 7.20.2 source
/// sample, also for the two rows past a stripe that the decoder takes before
/// CDEF.
///
/// # Errors
/// Returns [`EncoderSyntaxError::Rule`] when the transform records do not
/// cover the frame or a classification step rejects its inputs.
pub fn pc_wiener_cell_classes(
    seq: &SequenceHeader,
    core: &FrameHeaderCore,
    recorder: &FilterRecorder,
    frame: &DecodedFrame<u16>,
) -> Result<Vec<u8>> {
    let facts = minimal_partition_frame_facts(seq, core).map_err(rule("frame facts"))?;
    let (mi_rows, mi_cols) = (facts.mi_rows(), facts.mi_cols());
    let grid = crate::filters::wienerns_lr::derive_wienerns_lr_tx_skip_grid_reusing(
        mi_rows,
        mi_cols,
        &recorder.tx_skip,
        Vec::new(),
    )
    .map_err(rule("LrTxSkip grid"))?;
    let luma = frame
        .plane(PlaneId::Y)
        .ok_or(EncoderSyntaxError::Unsupported("a frame without luma"))?;
    let stride = luma.stride_samples();
    let storage = luma.storage_size();
    let across = seq
        .filter
        .is_some_and(|filter| filter.disable_loopfilters_across_tiles);
    let mut classes = vec![0u8; mi_rows * mi_cols];
    for tile in 0..tile_count(core) {
        let (rows, cols) = tile_ranges(core, tile)?;
        let (start_x, end_x, start_y, end_y) = if across {
            (
                cols.start * 4,
                cols.end * 4 - 1,
                rows.start * 4,
                rows.end * 4 - 1,
            )
        } else {
            (0, mi_cols * 4 - 1, 0, mi_rows * 4 - 1)
        };
        let mut base = rows.start * 4;
        while base < rows.end * 4 + 8 {
            let stripe_start = base.checked_sub(8).map_or(start_y, |y| y.max(start_y));
            let stripe_end = end_y.min(base + 55);
            let top = stripe_start.max(rows.start * 4);
            let bottom = (stripe_end + 1).min(rows.end * 4);
            base += 64;
            if bottom <= top {
                continue;
            }
            for block_x in (cols.start * 4..cols.end * 4).step_by(64) {
                let block_end_x = (cols.end * 4 - 1).min(block_x + 63);
                let params = splot_recon::PcWienerClassifyParams {
                    x: block_x as isize,
                    y: top as isize,
                    bit_depth: frame.bit_depth(),
                    base_q_idx: recorder.qindex,
                    block_start_x: block_x,
                    block_end_x,
                    luma_stripe_start_y: stripe_start,
                    luma_stripe_end_y: stripe_end,
                    tile_start_y: rows.start * 4,
                    tile_end_y: rows.end * 4 - 1,
                };
                let source = |x: isize, y: isize| -> splot_recon::Result<u16> {
                    let x = (x.max(start_x as isize) as usize).min(end_x);
                    let y = (y.max(start_y as isize) as usize).min(end_y);
                    let y = y.clamp(stripe_start.saturating_sub(2), stripe_end + 2);
                    let (x, y) = (x.min(storage.width() - 1), y.min(storage.height() - 1));
                    Ok(luma.samples()[y * stride + x])
                };
                let skip = |lookup: splot_recon::PcWienerTxSkipLookup| {
                    grid.run(lookup.row, lookup.col, 1)
                        .map(|run| i32::from(run[0]))
                };
                let cells = splot_recon::pc_wiener_classify_grid(
                    &params,
                    (block_end_x + 1 - block_x) / 4,
                    (bottom - top) / 4,
                    source,
                    skip,
                )
                .map_err(rule("PC-Wiener classes"))?;
                let width = (block_end_x + 1 - block_x) / 4;
                for (index, cell) in cells.iter().enumerate() {
                    let (row, col) = (top / 4 + index / width, block_x / 4 + index % width);
                    classes[row * mi_cols + col] = cell.class;
                }
            }
        }
    }
    Ok(classes)
}

/// The reconstruction of the frame `core` whose tile `t` holds the samples of
/// `tiles[t]`, each tile coded by its own facade.
///
/// # Errors
/// Returns [`EncoderSyntaxError`] when the frame has another tile count or the
/// frames differ in geometry.
pub fn merge_tiles(
    core: &FrameHeaderCore,
    tiles: &[DecodedFrame<u16>],
) -> Result<DecodedFrame<u16>> {
    let first = tiles
        .first()
        .filter(|_| tiles.len() == tile_count(core))
        .ok_or(EncoderSyntaxError::Unsupported("one frame per tile"))?;
    let mut workspace = CurrentFrameWorkspace::new(first.info(), 0).map_err(rule("workspace"))?;
    for (tile, frame) in tiles.iter().enumerate() {
        let (rows, cols) = tile_ranges(core, tile)?;
        copy_luma_rect(
            &mut workspace,
            frame,
            (
                cols.start * 4,
                rows.start * 4,
                cols.len() * 4,
                rows.len() * 4,
            ),
        )?;
    }
    workspace.freeze().map_err(rule("frame"))
}

/// Copies the samples of `frame` over the luma rectangle `(x, y, width,
/// height)`, and the chroma rectangles under it, into `workspace`.
fn copy_luma_rect(
    workspace: &mut CurrentFrameWorkspace<u16>,
    frame: &DecodedFrame<u16>,
    (x, y, width, height): (usize, usize, usize, usize),
) -> Result<()> {
    let format = frame.info().pixel_format();
    let chroma = (
        u32::from(format.subsampling_x()),
        u32::from(format.subsampling_y()),
    );
    for (id, (shift_x, shift_y)) in [
        (PlaneId::Y, (0, 0)),
        (PlaneId::U, chroma),
        (PlaneId::V, chroma),
    ] {
        let Some(plane) = frame.plane(id) else {
            continue;
        };
        let size = plane.storage_size();
        let (left, top) = (x >> shift_x, y >> shift_y);
        let right = x
            .saturating_add(width)
            .div_ceil(1 << shift_x)
            .min(size.width());
        let bottom = y
            .saturating_add(height)
            .div_ceil(1 << shift_y)
            .min(size.height());
        if right <= left || bottom <= top {
            continue;
        }
        let rect = PlaneRect::new(left, top, right - left, bottom - top).map_err(rule("plane"))?;
        let stride = plane.stride_samples();
        workspace
            .write_rect(id, rect, &plane.samples()[top * stride + left..], stride)
            .map_err(rule("plane"))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "encoder_filters_tests.rs"]
mod tests;

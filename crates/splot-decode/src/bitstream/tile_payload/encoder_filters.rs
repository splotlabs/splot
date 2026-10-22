// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Encoder-facing loop-filter facade (deblocking, CDEF, loop restoration); not a
//! stable API.
//!
//! - [`FilterRecorder`] keeps the per-transform records the decoder's walk keeps.
//!   Call [`FilterRecorder::record_leaf`] once per coded leaf, after the leaf's
//!   residual is coded, with every transform block of the leaf in § 5.20.7.23
//!   order and the `eob` of each.
//! - [`CdefIndexMap`] is the tile's `cdef_idx` state of § 5.20.10.1: it names the
//!   leaf that codes each 64x64 index and the `cdef_index0` context.
//! - [`LrLayout`] lists the § 5.20.10.4 restoration units of each superblock.
//! - [`apply_loop_filters`] runs the decoder's own § 7.2 filter chain on a frame.

use std::sync::Arc;

use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::sequence::{ChromaFormatIdc, SequenceHeader};
use splot_recon::{CurrentFrameWorkspace, DecodedFrame, PlaneId, PlaneRect};

use super::super::cdf::TileCdfSelector;
use super::super::partition_traversal::{
    ROOT_HAS_CHROMA, TilePartitionBounds, TilePartitionCall, TilePartitionFrameFacts,
    TilePartitionLoopRestorationPlaneTool, pc_wiener_unit_source_blocks, superblock_lr_units,
};
use super::super::tile_frontier::minimal_partition_frame_facts;
use super::{
    EncoderSyntaxError, IntraTileSyntax, LeafBlock, PartitionNode, Result, TransformBlock, rule,
    tx_log2,
};
use crate::filters::deblock::{ChromaDeblockRecords, DeblockBlock};
use crate::filters::wienerns_lr::tx_records::CdefState;
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

    /// The `use_pc_wiener` CDF row (§ 5.20.10.5).
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] for a CDF table defect.
    pub fn use_pc_wiener_cdf(&mut self) -> Result<&mut [u16]> {
        self.tile_row(TileCdfSelector::UsePcWiener)
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

/// The `cdef_idx[][]` state of a single-tile frame, filled in coding order.
pub struct CdefIndexMap {
    state: CdefState,
    strengths: usize,
    skip_codes_index: bool,
    rows: usize,
    cols: usize,
}

impl CdefIndexMap {
    /// An empty map for the frame `core` coded with `seq`.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] for missing frame facts.
    pub fn new(seq: &SequenceHeader, core: &FrameHeaderCore) -> Result<Self> {
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
            .cdef_index0_ctx_at(r, c, 0, 0)
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
    /// Plane index.
    pub plane: usize,
    /// `unitRow`.
    pub row: usize,
    /// `unitCol`.
    pub col: usize,
}

/// The restoration-unit layout of a frame whose planes use `RESTORE_PC_WIENER`
/// or `RESTORE_NONE`.
pub struct LrLayout {
    frame: TilePartitionFrameFacts,
    bounds: TilePartitionBounds,
}

impl LrLayout {
    /// The layout of the frame `core` coded with `seq`.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] for missing frame facts.
    pub fn new(seq: &SequenceHeader, core: &FrameHeaderCore) -> Result<Self> {
        let frame = minimal_partition_frame_facts(seq, core).map_err(rule("frame facts"))?;
        let bounds = TilePartitionBounds::new(0..frame.mi_rows(), 0..frame.mi_cols());
        Ok(Self { frame, bounds })
    }

    /// The units `read_lr` codes at superblock `root`, in syntax order. Each
    /// codes one `use_pc_wiener` symbol.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Unsupported`] for a plane tool other than
    /// `RESTORE_PC_WIENER`, and [`EncoderSyntaxError::Rule`] for an invalid root.
    pub fn units(&self, root: PartitionNode) -> Result<Vec<LrUnit>> {
        superblock_lr_units(self.frame, root.0, self.bounds)
            .map_err(rule("read_lr"))?
            .into_iter()
            .map(|(plane, row, col, tool)| {
                if tool == TilePartitionLoopRestorationPlaneTool::PcWiener {
                    Ok(LrUnit { plane, row, col })
                } else {
                    Err(EncoderSyntaxError::Unsupported(
                        "restoration tool other than RESTORE_PC_WIENER",
                    ))
                }
            })
            .collect()
    }

    /// The units of every superblock, in coding order.
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
        for row4 in (0..self.frame.mi_rows()).step_by(sb4) {
            for col4 in (0..self.frame.mi_cols()).step_by(sb4) {
                let root =
                    TilePartitionCall::root(row4, col4, self.frame.sb_size(), ROOT_HAS_CHROMA);
                units.extend(self.units(PartitionNode(root))?);
            }
        }
        Ok(units)
    }

    /// The plane rectangles a `RESTORE_PC_WIENER` unit filters, which can reach
    /// past the visible frame.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] for a unit outside the frame.
    pub fn unit_rects(&self, unit: LrUnit) -> Result<Vec<PlaneRect>> {
        pc_wiener_unit_source_blocks(self.frame, self.bounds, unit.plane, unit.row, unit.col)
            .map_err(rule("restoration unit"))?
            .into_iter()
            .map(|block| {
                PlaneRect::new(block.x, block.y, block.width, block.height)
                    .map_err(rule("restoration unit"))
            })
            .collect()
    }
}

/// Runs the decoder's deblocking, CDEF, and loop-restoration chain (§ 7.2) of
/// the frame `core` on `frame`, as a decoder of the same tile would.
///
/// `cdef_indices` holds `cdef_idx` per 64x64 unit in raster order (see
/// [`cdef_unit_grid`]), or is empty when no unit codes one; `lr_on` lists the
/// units whose `use_pc_wiener` is `1`.
///
/// # Errors
/// Returns [`EncoderSyntaxError::Rule`] when a filter stage rejects its inputs.
pub fn apply_loop_filters(
    seq: &SequenceHeader,
    core: &FrameHeaderCore,
    recorder: &FilterRecorder,
    frame: &DecodedFrame<u16>,
    cdef_indices: &[Option<usize>],
    lr_on: &[LrUnit],
) -> Result<DecodedFrame<u16>> {
    let layout = LrLayout::new(seq, core)?;
    let mut lr_source_blocks = Vec::new();
    for unit in lr_on {
        lr_source_blocks.extend(
            pc_wiener_unit_source_blocks(
                layout.frame,
                layout.bounds,
                unit.plane,
                unit.row,
                unit.col,
            )
            .map_err(rule("restoration unit"))?,
        );
    }
    let (rows, cols) = cdef_unit_grid((layout.frame.mi_rows(), layout.frame.mi_cols()));
    let cdef_indices = if cdef_indices.is_empty() {
        vec![None; rows * cols]
    } else {
        cdef_indices.to_vec()
    };
    let cdef_grid = crate::filters::cdef::CdefUnitGrid::new(rows, cols, cdef_indices)
        .map_err(rule("cdef_idx grid"))?;
    let mut workspace = CurrentFrameWorkspace::new(frame.info(), 0).map_err(rule("workspace"))?;
    for id in [PlaneId::Y, PlaneId::U, PlaneId::V] {
        if let Some(plane) = frame.plane(id) {
            let size = plane.storage_size();
            let rect = PlaneRect::new(0, 0, size.width(), size.height()).map_err(rule("plane"))?;
            workspace
                .write_rect(id, rect, plane.samples(), plane.stride_samples())
                .map_err(rule("plane"))?;
        }
    }
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

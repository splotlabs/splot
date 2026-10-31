// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Tile writer of key and inter frames.
//!
//! [`KeyFrameTileWriter`] walks each [`SuperblockDecision`] in § 5.20.3.1 syntax
//! order, writes every symbol through [`SymbolEncoder`] with the CDF rows and
//! contexts of the decoder's `encoder_syntax` facade, reconstructs each transform
//! block through the facade, and publishes the coded facts back to it.

pub(crate) mod coeffs;
mod decision;
pub(crate) mod filters;

use core::ops::Range;

use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::sequence::SequenceHeader;
use splot_core::symbol_encoder::{SymbolEncoder, SymbolEncoderConfig, SymbolSink};
use splot_core::write::WriteError;
use splot_decode::encoder_syntax::{
    AMVD_STEPS, EncoderReferences, EncoderSyntaxError, FrameStart, IntraTileSyntax, LeafBlock,
    PartitionNode, TileEnd, TransformBlock, TxPartition, WarpModel,
};
use splot_recon::{DecodedFrame, ReconError};

pub(crate) use decision::{
    CompoundMode, CompoundModes, CompoundType, INTRA_MODES, InterMode, InterModes, InterPrediction,
    IntrabcModes, Ist, LeafChoice, MotionVector, Partition, PartitionTree, ResidualCoder,
    SuperblockDecision, TransformDecision, TxType,
};

/// Output byte cap of one tile; larger than any 4096x2160 tile of 10-bit levels needs.
const MAX_TILE_BYTES: usize = 1 << 28;
/// Symbol-operation cap of one tile.
const MAX_TILE_OPERATIONS: usize = 1 << 30;

/// Every partition type, in `PartitionType` index order.
pub(crate) const PARTITIONS: [Partition; 10] = [
    Partition::None,
    Partition::Horz,
    Partition::Vert,
    Partition::Horz3,
    Partition::Vert3,
    Partition::Horz4A,
    Partition::Horz4B,
    Partition::Vert4A,
    Partition::Vert4B,
    Partition::Split,
];

/// Error from the key-frame tile writer.
#[derive(Debug, thiserror::Error)]
pub(crate) enum TileWriterError {
    #[error("unsupported key-frame tile feature: {0}")]
    Unsupported(&'static str),
    #[error("partition {partition:?} is not codable at ({row4}, {col4}): {reason}")]
    Partition {
        partition: Partition,
        row4: usize,
        col4: usize,
        reason: &'static str,
    },
    #[error("decision shape mismatch: {0}")]
    Shape(&'static str),
    #[error("transform block expects {expected} levels, got {actual}")]
    LevelCount { expected: usize, actual: usize },
    #[error("quantized level {0} exceeds the conformance bound")]
    LevelOutOfRange(i32),
    #[error("all superblocks are already written")]
    TileComplete,
    #[error("{written} of {total} superblocks written")]
    TileIncomplete { written: usize, total: usize },
    #[error(transparent)]
    Syntax(#[from] EncoderSyntaxError),
    #[error(transparent)]
    Symbol(#[from] WriteError),
    #[error(transparent)]
    Recon(#[from] ReconError),
    #[error(transparent)]
    Kernel(#[from] crate::kernels::KernelError),
}

pub(crate) type Result<T> = core::result::Result<T, TileWriterError>;

/// Writes one tile of a frame, one superblock at a time.
pub(crate) struct KeyFrameTileWriter {
    syntax: IntraTileSyntax,
    symbols: SymbolEncoder,
    origin: (usize, usize),
    sb_cols: usize,
    superblocks: usize,
    written: usize,
    filters: filters::TileFilters,
}

impl KeyFrameTileWriter {
    /// A writer for tile `tile`, in raster order, of the frame `core` coded
    /// with `seq`, predicting from `refs`.
    #[cfg(test)]
    pub(crate) fn new(
        seq: &SequenceHeader,
        core: &FrameHeaderCore,
        refs: &EncoderReferences,
        tile: usize,
    ) -> Result<Self> {
        Self::with_start(seq, core, refs, &refs.frame_start(core)?, tile)
    }

    /// [`Self::new`] starting from the frame's initial `cdfs`.
    pub(crate) fn with_start(
        seq: &SequenceHeader,
        core: &FrameHeaderCore,
        refs: &EncoderReferences,
        cdfs: &FrameStart,
        tile: usize,
    ) -> Result<Self> {
        let syntax = IntraTileSyntax::with_start(seq, core, refs, cdfs, tile)?;
        let (rows, cols) = syntax.tile_ranges();
        let sb4 = syntax.superblock_size4();
        let config = SymbolEncoderConfig::new()
            .with_max_output_bytes(MAX_TILE_BYTES)
            .with_max_operations(MAX_TILE_OPERATIONS);
        Ok(Self {
            syntax,
            symbols: SymbolEncoder::with_config(config),
            origin: (rows.start, cols.start),
            sb_cols: cols.len().div_ceil(sb4),
            superblocks: cols.len().div_ceil(sb4) * rows.len().div_ceil(sb4),
            written: 0,
            filters: filters::TileFilters::new(seq, core, tile)?,
        })
    }

    /// The facade, for planning decisions.
    pub(crate) const fn syntax(&self) -> &IntraTileSyntax {
        &self.syntax
    }

    /// The root node of the next superblock of the tile in raster order.
    pub(crate) fn next_superblock_root(&self) -> Option<PartitionNode> {
        (self.written < self.superblocks).then(|| {
            let sb4 = self.syntax.superblock_size4();
            self.syntax.superblock_root(
                self.origin.0 + self.written / self.sb_cols * sb4,
                self.origin.1 + self.written % self.sb_cols * sb4,
            )
        })
    }

    /// Writes the next superblock. After an error the writer state is partial; drop it.
    pub(crate) fn write_superblock(
        &mut self,
        decision: &SuperblockDecision,
        coder: &mut impl ResidualCoder,
    ) -> Result<()> {
        let root = self
            .next_superblock_root()
            .ok_or(TileWriterError::TileComplete)?;
        self.syntax.begin_superblock(root);
        if self.syntax.sdp_roots(root).is_none() {
            self.filters
                .write_lr_units(&mut self.syntax, &mut self.symbols, root)?;
        }
        Walk {
            syntax: &mut self.syntax,
            sink: &mut self.symbols,
            filters: Some(&mut self.filters),
        }
        .write_node(root, &decision.tree, coder)?;
        self.written += 1;
        Ok(())
    }

    /// Runs `probe` on the facade at the root of every superblock of the tile
    /// in raster order, before any superblock is written.
    pub(crate) fn probe_superblocks(
        &mut self,
        mut probe: impl FnMut(&mut IntraTileSyntax, PartitionNode) -> Result<()>,
    ) -> Result<()> {
        let sb4 = self.syntax.superblock_size4();
        for index in 0..self.superblocks {
            let root = self.syntax.superblock_root(
                self.origin.0 + index / self.sb_cols * sb4,
                self.origin.1 + index % self.sb_cols * sb4,
            );
            self.syntax.begin_superblock(root);
            probe(&mut self.syntax, root)?;
        }
        Ok(())
    }

    /// Runs `search` on the facade at the start of the next superblock, then
    /// rolls the facade back, so the superblock is still to be written.
    pub(crate) fn search_superblock<T>(
        &mut self,
        search: impl FnOnce(&mut IntraTileSyntax, PartitionNode) -> Result<T>,
    ) -> Result<T> {
        let root = self
            .next_superblock_root()
            .ok_or(TileWriterError::TileComplete)?;
        self.syntax.begin_superblock(root);
        let saved = self.syntax.snapshot(root)?;
        let found = search(&mut self.syntax, root);
        self.syntax.restore(&saved)?;
        found
    }

    /// Plans the following superblocks at the codable qindex nearest
    /// `target`; returns it.
    pub(crate) fn set_superblock_qindex(&mut self, target: u32) -> Result<u32> {
        Ok(self.syntax.set_superblock_qindex(target)?)
    }

    /// Sets the loop-filter choices the following superblocks code.
    pub(crate) fn set_filter_syntax(&mut self, syntax: filters::FilterSyntax) {
        self.filters.set_syntax(syntax);
    }

    /// The `tile_data` bytes and the reconstructed frame.
    #[cfg(test)]
    pub(crate) fn finish(self) -> Result<(Vec<u8>, DecodedFrame<u16>)> {
        let (tile, recon, ..) = self.finish_with_filters()?;
        Ok((tile, recon))
    }

    /// [`Self::finish`] plus the tile's final CDFs, the filter records and the
    /// coded filter choices.
    pub(crate) fn finish_with_filters(
        self,
    ) -> Result<(Vec<u8>, DecodedFrame<u16>, TileEnd, filters::CodedFilters)> {
        if self.written != self.superblocks {
            return Err(TileWriterError::TileIncomplete {
                written: self.written,
                total: self.superblocks,
            });
        }
        let (recon, cdfs) = self.syntax.finish()?;
        Ok((
            self.symbols.finish()?.into_bytes(),
            recon,
            cdfs,
            self.filters.finish()?,
        ))
    }
}

#[cfg(test)]
thread_local! {
    /// Compound leaves of the final passes on this thread: weighted
    /// `COMPOUND_AVERAGE`, wedge, difference-weighted, and masked
    /// `NEAR_NEARMV`; then inter-intra leaves, those of them at the left
    /// edge of a tile that does not start the frame, and BAWP leaves.
    pub(crate) static CODED_TYPES: core::cell::Cell<[usize; 7]> =
        const { core::cell::Cell::new([0; 7]) };
}

#[cfg(test)]
fn count_coded_type(prediction: InterPrediction, tile_edge: bool) {
    let mut counts = CODED_TYPES.get();
    if let InterPrediction::Single(single) = prediction {
        counts[4] += usize::from(single.interintra.is_some());
        counts[5] += usize::from(single.interintra.is_some() && tile_edge);
        counts[6] += usize::from(single.bawp.is_some());
        CODED_TYPES.set(counts);
    }
    let InterPrediction::Compound(modes) = prediction else {
        return;
    };
    match modes.blend {
        CompoundType::Average { cwp } => counts[0] += usize::from(cwp > 0),
        CompoundType::Wedge { .. } => counts[1] += 1,
        CompoundType::DiffWeighted { .. } => counts[2] += 1,
    }
    counts[3] += usize::from(
        modes.mode == CompoundMode::NearNear
            && !matches!(modes.blend, CompoundType::Average { .. }),
    );
    CODED_TYPES.set(counts);
}

std::thread_local! {
    /// The prediction buffers of the leaf writers, kept across leaves and
    /// trials (a [`Walk`] lives for one trial); a nested walk finds them empty.
    static PREDICTIONS: core::cell::Cell<[Vec<u16>; 2]> = core::cell::Cell::default();
}

/// One syntax pass over a tile: the facade, a symbol sink, and the filter
/// state. The final pass codes into the [`SymbolEncoder`] with the filters; an
/// RD trial prices into a rate sink without them, because the filter syntax
/// does not depend on the trial and the filter state does not roll back.
pub(crate) struct Walk<'a, S> {
    pub(crate) syntax: &'a mut IntraTileSyntax,
    pub(crate) sink: &'a mut S,
    filters: Option<&'a mut filters::TileFilters>,
}

impl<'a, S: SymbolSink> Walk<'a, S> {
    /// A trial pass that skips the filter syntax.
    pub(crate) const fn trial(syntax: &'a mut IntraTileSyntax, sink: &'a mut S) -> Self {
        Self {
            syntax,
            sink,
            filters: None,
        }
    }

    fn write_node(
        &mut self,
        node: PartitionNode,
        tree: &PartitionTree,
        coder: &mut impl ResidualCoder,
    ) -> Result<()> {
        if !self.syntax.in_frame(node) {
            return Ok(());
        }
        let sdp = self.syntax.sdp_roots(node);
        let partition = match (tree, sdp) {
            (PartitionTree::Sdp { luma, chroma }, Some((luma_root, chroma_root))) => {
                let superblock = node.dimensions()?.0 == self.syntax.superblock_size4() * 4;
                for (root, tree) in [(luma_root, luma), (chroma_root, chroma)] {
                    if superblock && let Some(filters) = self.filters.as_deref_mut() {
                        filters.write_lr_units(self.syntax, self.sink, root)?;
                    }
                    self.write_node(root, tree, coder)?;
                }
                return Ok(());
            }
            (PartitionTree::Leaf(_), None) => Partition::None,
            (PartitionTree::Split { kind, .. }, None) => *kind,
            _ => return Err(TileWriterError::Shape("SDP tree at a shared node or none")),
        };
        self.write_partition(node, partition)?;
        match tree {
            PartitionTree::Leaf(choice) => {
                let leaf = self.syntax.leaf(node)?;
                self.write_leaf(&leaf, *choice, coder, 0..3).map(drop)
            }
            PartitionTree::Split { kind, children } => {
                let calls = self.syntax.children(node, *kind)?;
                if calls.len() != children.len() {
                    return Err(TileWriterError::Shape("child count"));
                }
                for (call, child) in calls.into_iter().zip(children) {
                    self.write_node(call, child, coder)?;
                }
                Ok(())
            }
            PartitionTree::Sdp { .. } => Err(TileWriterError::Shape("SDP tree")),
        }
    }

    /// The § 5.20.3.2 `read_partition` symbols that select `partition`.
    pub(crate) fn write_partition(
        &mut self,
        node: PartitionNode,
        partition: Partition,
    ) -> Result<()> {
        self.syntax
            .write_partition(node, partition, self.sink)
            .map_err(|error| match error {
                EncoderSyntaxError::Unsupported(reason) => TileWriterError::Partition {
                    partition,
                    row4: node.row4(),
                    col4: node.col4(),
                    reason,
                },
                error => error.into(),
            })
    }

    /// Codes the blocks of `planes` of `leaf` under `choice`, then writes the
    /// leaf syntax in order: `is_inter`, `cdef_idx`, the modes, the palette
    /// color map, the luma transform partition, and the coefficients of
    /// `planes`. Only a pass over all planes publishes the leaf. An inter leaf codes every plane. Returns
    /// the leaf's transform blocks.
    pub(crate) fn write_leaf(
        &mut self,
        leaf: &LeafBlock,
        choice: LeafChoice,
        coder: &mut impl ResidualCoder,
        planes: Range<usize>,
    ) -> Result<Vec<TransformBlock>> {
        if let Some(modes) = choice.inter {
            #[cfg(test)]
            if self.filters.is_some() {
                let cols = self.syntax.tile_ranges().1;
                count_coded_type(modes, cols.start > 0 && leaf.col4() == cols.start);
            }
            return self.write_inter_leaf(leaf, (modes, choice.tx_partition), coder);
        }
        if let Some(modes) = choice.intrabc {
            return self.write_intrabc_leaf(leaf, (modes, choice.tx_partition), coder);
        }
        let LeafChoice {
            modes,
            tx_partition,
            ..
        } = choice;
        if let Some(palette) = &modes.palette {
            self.syntax
                .set_palette_map(coder.palette_map(leaf, palette)?);
        }
        let mut blocks = self
            .syntax
            .plan_leaf_partitioned(leaf, modes, tx_partition)?;
        let mut levels = vec![Vec::new(); blocks.len()];
        let [mut prediction, mut v_prediction] = PREDICTIONS.take();
        // Luma first: the decoder reconstructs CFL chroma after the whole luma.
        for luma in [true, false] {
            for index in 0..blocks.len() {
                let tx = blocks[index];
                if !planes.contains(&tx.plane)
                    || (tx.plane == 0) != luma
                    || !levels[index].is_empty()
                {
                    continue;
                }
                self.syntax.predict_into(index, &mut prediction)?;
                if let Some(v_index) = chroma_partner(&blocks, &tx) {
                    let v = blocks[v_index];
                    self.syntax.predict_into(v_index, &mut v_prediction)?;
                    let [u_decision, v_decision] =
                        self.code_chroma_pair(coder, [&tx, &v], [&prediction, &v_prediction])?;
                    blocks[index].cctx = u_decision.cctx;
                    if u_decision.cctx == 0 {
                        self.syntax.reconstruct_as(index, &tx, &u_decision.levels)?;
                        self.syntax
                            .reconstruct_as(v_index, &v, &v_decision.levels)?;
                    } else {
                        let levels = [u_decision.levels.as_slice(), &v_decision.levels];
                        let u = &blocks[index];
                        self.syntax
                            .reconstruct_cctx_pair([index, v_index], u, levels)?;
                    }
                    levels[index] = u_decision.levels;
                    levels[v_index] = v_decision.levels;
                    continue;
                }
                let decision = if tx.plane == 0 {
                    self.code_luma(coder, &tx, &prediction)?
                } else {
                    code_as(self.syntax, coder, &tx, &prediction)?
                };
                let tx = &mut blocks[index];
                *tx = TransformBlock {
                    tx_type: decision.tx_type,
                    ist: decision.ist,
                    ..*tx
                };
                self.syntax.reconstruct_as(index, tx, &decision.levels)?;
                levels[index] = decision.levels;
            }
        }
        PREDICTIONS.set([prediction, v_prediction]);
        self.syntax.write_is_inter(leaf, false, self.sink)?;
        self.syntax.write_use_intrabc(leaf, None, self.sink)?;
        if let Some(filters) = self.filters.as_deref_mut() {
            filters.write_leaf_syntax(self.syntax, self.sink, leaf, false)?;
        }
        self.syntax.write_intra_modes(leaf, modes, self.sink)?;
        self.syntax.write_palette_tokens(self.sink)?;
        let sink = &mut *self.sink;
        self.syntax
            .code_luma_tx_partition(leaf, tx_partition, |cdf, value| {
                coeffs::write_symbol(sink, cdf, value)
            })?;
        let eobs = self.write_residual(&blocks, &levels, &planes)?;
        if planes == (0..3) {
            self.syntax.publish_intra_leaf(leaf, modes)?;
            if let Some(filters) = self.filters.as_deref_mut() {
                filters.record_leaf(self.syntax, leaf, &blocks, &eobs)?;
            }
        }
        Ok(blocks)
    }

    /// Codes the inter leaf `leaf` under `prediction`: its residual over the
    /// motion compensated prediction, then `skip_mode` and `is_inter`,
    /// `skip_txfm`, `cdef_idx`, the reference and mode syntax, the motion
    /// vector differences, `interp_filter`, and the residual. A leaf whose levels are all zero
    /// codes `skip_txfm = 1`.
    fn write_inter_leaf(
        &mut self,
        leaf: &LeafBlock,
        (prediction, tx_partition): (InterPrediction, TxPartition),
        coder: &mut impl ResidualCoder,
    ) -> Result<Vec<TransformBlock>> {
        let prediction = snap_prediction(self.syntax, leaf, prediction)?;
        let mut blocks = self
            .syntax
            .plan_inter_leaf_partitioned(leaf, prediction, tx_partition)?;
        let (levels, skip) = self.code_inter_residual(leaf, &mut blocks, coder)?;
        self.syntax.write_is_inter(leaf, true, self.sink)?;
        self.syntax.write_skip_txfm(leaf, skip, self.sink)?;
        if let Some(filters) = self.filters.as_deref_mut() {
            filters.write_leaf_syntax(self.syntax, self.sink, leaf, skip)?;
        }
        let mvds = self.syntax.write_inter_modes(leaf, self.sink)?;
        self.syntax.write_mvds(&mvds, self.sink)?;
        self.syntax.write_interp_filter(leaf, self.sink)?;
        if !skip {
            let sink = &mut *self.sink;
            self.syntax
                .code_luma_tx_partition(leaf, tx_partition, |cdf, value| {
                    coeffs::write_symbol(sink, cdf, value)
                })?;
            self.write_residual(&blocks, &levels, &(0..3))?;
        }
        if let Some(filters) = self.filters.as_deref_mut() {
            filters.record_inter_leaf(self.syntax, leaf, skip)?;
        }
        self.syntax.publish_inter_leaf(leaf, skip)?;
        Ok(blocks)
    }

    /// Codes the IntraBC leaf `leaf` of `modes`: its residual over the copy,
    /// then `use_intrabc` and `skip_txfm`, `cdef_idx`, the IntraBC syntax and
    /// the residual. A leaf whose levels are all zero codes `skip_txfm = 1`.
    fn write_intrabc_leaf(
        &mut self,
        leaf: &LeafBlock,
        (modes, tx_partition): (IntrabcModes, TxPartition),
        coder: &mut impl ResidualCoder,
    ) -> Result<Vec<TransformBlock>> {
        let mut blocks = self.syntax.plan_intrabc_leaf(leaf, modes, tx_partition)?;
        let (levels, skip) = self.code_inter_residual(leaf, &mut blocks, coder)?;
        self.syntax.write_use_intrabc(leaf, Some(skip), self.sink)?;
        if let Some(filters) = self.filters.as_deref_mut() {
            filters.write_leaf_syntax(self.syntax, self.sink, leaf, skip)?;
        }
        self.syntax.write_intrabc_modes(leaf, modes, self.sink)?;
        if !skip {
            let sink = &mut *self.sink;
            self.syntax
                .code_luma_tx_partition(leaf, tx_partition, |cdf, value| {
                    coeffs::write_symbol(sink, cdf, value)
                })?;
            self.write_residual(&blocks, &levels, &(0..3))?;
        }
        if let Some(filters) = self.filters.as_deref_mut() {
            filters.record_inter_leaf(self.syntax, leaf, skip)?;
        }
        self.syntax.publish_intrabc_leaf(leaf, skip)?;
        Ok(blocks)
    }

    /// Codes each planned block of the inter or IntraBC leaf `leaf` over its
    /// prediction and adds the residual; returns the levels and whether all
    /// are zero.
    fn code_inter_residual(
        &mut self,
        leaf: &LeafBlock,
        blocks: &mut [TransformBlock],
        coder: &mut impl ResidualCoder,
    ) -> Result<(Vec<Vec<i32>>, bool)> {
        let mut levels = vec![Vec::new(); blocks.len()];
        let [mut prediction, v_prediction] = PREDICTIONS.take();
        for index in 0..blocks.len() {
            let mut tx = blocks[index];
            if tx.plane > 0 {
                tx.tx_type = self
                    .syntax
                    .inter_chroma_tx_type(leaf, &blocks[..index], &tx)?;
            }
            self.syntax.reconstruction_into(&tx, &mut prediction)?;
            let decision = if tx.plane == 0 {
                self.code_luma(coder, &tx, &prediction)?
            } else {
                code_as(self.syntax, coder, &tx, &prediction)?
            };
            tx.tx_type = decision.tx_type;
            blocks[index] = tx;
            levels[index] = decision.levels;
        }
        PREDICTIONS.set([prediction, v_prediction]);
        let skip = levels.iter().flatten().all(|&level| level == 0);
        if !skip {
            self.syntax.reconstruct_inter(blocks, &levels)?;
        }
        Ok((levels, skip))
    }

    /// Writes the coefficients of the blocks of `planes`; returns each `eob`.
    fn write_residual(
        &mut self,
        blocks: &[TransformBlock],
        levels: &[Vec<i32>],
        planes: &Range<usize>,
    ) -> Result<Vec<usize>> {
        let mut eob_u = 0;
        let mut eobs = vec![0; blocks.len()];
        for ((tx, levels), eob) in blocks.iter().zip(levels).zip(&mut eobs) {
            if !planes.contains(&tx.plane) {
                continue;
            }
            *eob = coeffs::write_coeffs(
                self.syntax,
                self.sink,
                tx,
                levels,
                tx.plane == 2 && eob_u != 0,
            )?;
            if tx.plane == 1 {
                eob_u = *eob;
            }
        }
        Ok(eobs)
    }

    /// The decisions of U block `u` and V block `v` of one position; a CCTX
    /// the decoder would not read for the final U levels is recoded without.
    fn code_chroma_pair(
        &mut self,
        coder: &mut impl ResidualCoder,
        [u, v]: [&TransformBlock; 2],
        predictions: [&[u16]; 2],
    ) -> Result<[TransformDecision; 2]> {
        let decisions = coder.code_chroma_pair(self.syntax, [u, v], predictions)?;
        let [u_decision, v_decision] = &decisions;
        if u_decision.tx_type != u.tx_type || v_decision.tx_type != v.tx_type {
            return Err(TileWriterError::Unsupported(
                "chroma transform type other than the planned one",
            ));
        }
        let coded = TransformBlock {
            cctx: u_decision.cctx,
            ..*u
        };
        if coded.cctx != 0
            && !self
                .syntax
                .cctx_coded(&coded, coeffs::eob(&coded, &u_decision.levels)?)
        {
            return Ok([
                code_as(self.syntax, coder, u, predictions[0])?,
                code_as(self.syntax, coder, v, predictions[1])?,
            ]);
        }
        Ok(decisions)
    }

    /// The decision of luma block `tx` in a type the decoder reads or infers
    /// for its levels.
    fn code_luma(
        &mut self,
        coder: &mut impl ResidualCoder,
        tx: &TransformBlock,
        prediction: &[u16],
    ) -> Result<TransformDecision> {
        let tx_types = self.syntax.allowed_luma_tx_types(tx)?;
        let mut decision = coder.code_luma(self.syntax, tx, prediction, &tx_types)?;
        if !tx_types.contains(&decision.tx_type) {
            return Err(TileWriterError::Unsupported(
                "luma transform type outside the offered types",
            ));
        }
        let coded = TransformBlock {
            tx_type: decision.tx_type,
            ist: decision.ist,
            ..*tx
        };
        if coded.ist != Ist::OFF
            && !self
                .syntax
                .ist_coded(&coded, coeffs::eob(&coded, &decision.levels)?)?
        {
            let tx = TransformBlock {
                ist: Ist::OFF,
                ..coded
            };
            decision = code_as(self.syntax, coder, &tx, prediction)?;
        }
        let coded = TransformBlock {
            tx_type: decision.tx_type,
            ..*tx
        };
        let eob = coeffs::eob(&coded, &decision.levels)?;
        let inferred = if eob == 0 {
            Some(tx.tx_type)
        } else {
            self.syntax.inferred_luma_tx_type(&coded, eob)?
        };
        match inferred {
            Some(tx_type) if tx_type != decision.tx_type && eob == 0 => {
                decision.tx_type = tx_type;
            }
            Some(tx_type) if tx_type != decision.tx_type => {
                let tx = TransformBlock { tx_type, ..*tx };
                decision = code_as(self.syntax, coder, &tx, prediction)?;
            }
            _ => {}
        }
        Ok(decision)
    }
}

/// The index of the V block that pairs with U block `u` in `blocks`.
fn chroma_partner(blocks: &[TransformBlock], u: &TransformBlock) -> Option<usize> {
    if u.plane != 1 {
        return None;
    }
    blocks
        .iter()
        .position(|v| v.plane == 2 && (v.x, v.y, v.tx_size) == (u.x, u.y, u.tx_size))
}

/// `prediction` with each coded vector moved onto the lattice of its base. A
/// `LOCALWARP` or `EXTENDWARP` model the leaf cannot code, which a memoized
/// subtree recoded under other neighbours meets, codes the zero `DELTAWARP`;
/// a BAWP whose § 7.13.3.25 template no longer fits is dropped.
pub(crate) fn snap_prediction(
    syntax: &mut IntraTileSyntax,
    leaf: &LeafBlock,
    prediction: InterPrediction,
) -> Result<InterPrediction> {
    let step = syntax.mv_step();
    let snapped = match prediction {
        InterPrediction::Single(mut modes) if modes.mode == InterMode::New => {
            let base = syntax.newmv_base(leaf, modes)?;
            let target = modes.mv;
            if modes.amvd {
                modes.mv = snap_amvd(target, base);
            } else {
                let step = syntax.newmv_step(&modes);
                modes.mv = snap_mv(target, base, step);
                if modes.precision.is_some() {
                    modes = derivable_sign(syntax, leaf, modes, step)?;
                }
            }
            InterPrediction::Single(modes)
        }
        InterPrediction::Compound(mut modes) => {
            let bases = syntax.compound_bases(leaf, &modes)?;
            for (mv, base) in modes.mvs.iter_mut().zip(bases) {
                if let Some(base) = base {
                    *mv = snap_mv(*mv, base, step);
                }
            }
            InterPrediction::Compound(modes)
        }
        InterPrediction::Warp(mut modes) => {
            let options = syntax.warp_options(leaf, modes.ref_frame)?;
            let reachable = match modes.model {
                WarpModel::Local => options.is_some_and(|options| options.local),
                WarpModel::Extend => options.is_some_and(|options| options.extend),
                _ => true,
            };
            if !reachable {
                modes.model = WarpModel::Delta {
                    high_precision: false,
                    values: [0; 4],
                };
            }
            if let Some(target) = modes.mv {
                let base = syntax.warp_base(leaf, modes)?;
                modes.mv = Some(snap_mv(target, base, syntax.warp_step(&modes)));
            }
            InterPrediction::Warp(modes)
        }
        other => other,
    };
    Ok(match snapped {
        InterPrediction::Single(modes) if !syntax.bawp_fits(leaf, modes)? => {
            InterPrediction::Single(InterModes {
                bawp: None,
                ..modes
            })
        }
        other => other,
    })
}

/// `modes`, whose coarse difference may have its last sign derived from the
/// parity of its magnitudes (§ 5.20.7.13), moved by one `step` to the next
/// lattice vector the decoder reads as coded when the derived sign differs.
fn derivable_sign(
    syntax: &mut IntraTileSyntax,
    leaf: &LeafBlock,
    modes: InterModes,
    step: i32,
) -> Result<InterModes> {
    for (dr, dc) in [(0, 0), (0, 1), (0, -1), (1, 0), (-1, 0)] {
        let moved = InterModes {
            mv: MotionVector {
                row: modes.mv.row + dr * step,
                col: modes.mv.col + dc * step,
            },
            ..modes
        };
        let mut sink = crate::rd::coeff_rate::RateSink::adapting();
        match syntax.price_inter_modes(leaf, InterPrediction::Single(moved), &mut sink) {
            Ok(()) => return Ok(moved),
            Err(EncoderSyntaxError::Unsupported(_)) => {}
            Err(error) => return Err(error.into()),
        }
    }
    Err(TileWriterError::Unsupported(
        "a NEWMV without a codable derived sign",
    ))
}

/// `mv` moved to the nearest vector `base` plus a multiple of `step` per
/// component, which a `NEWMV` over `base` can code.
pub(crate) fn snap_mv(mv: MotionVector, base: MotionVector, step: i32) -> MotionVector {
    let round = |v: i32, b: i32| b + (v - b + step / 2).div_euclid(step) * step;
    MotionVector {
        row: round(mv.row, base.row),
        col: round(mv.col, base.col),
    }
}

/// `mv` moved to the nearest vector `base` plus, per component, zero or `±`
/// an [`AMVD_STEPS`] entry, which an adaptive-MVD `NEWMV` over `base` codes.
pub(crate) fn snap_amvd(mv: MotionVector, base: MotionVector) -> MotionVector {
    let round = |v: i32, b: i32| {
        let d = v - b;
        let nearest = AMVD_STEPS
            .iter()
            .map(|&step| step * d.signum())
            .chain([0])
            .min_by_key(|&step| ((d - step).abs(), step.abs()))
            .unwrap_or(0);
        b + nearest
    };
    MotionVector {
        row: round(mv.row, base.row),
        col: round(mv.col, base.col),
    }
}

/// `coder.code(tx)`, checked to use `tx.tx_type`.
fn code_as(
    syntax: &mut IntraTileSyntax,
    coder: &mut impl ResidualCoder,
    tx: &TransformBlock,
    prediction: &[u16],
) -> Result<TransformDecision> {
    let decision = coder.code(syntax, tx, prediction)?;
    if decision.tx_type != tx.tx_type || decision.ist != tx.ist || decision.cctx != tx.cctx {
        return Err(TileWriterError::Unsupported(
            "transform type other than the requested one",
        ));
    }
    Ok(decision)
}

#[cfg(test)]
mod tests;

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
mod mv;

use core::ops::Range;

use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::sequence::SequenceHeader;
use splot_core::symbol_encoder::{SymbolEncoder, SymbolEncoderConfig, SymbolSink};
use splot_core::write::WriteError;
use splot_decode::encoder_syntax::{
    EncoderReferences, EncoderSyntaxError, IntraTileSyntax, LeafBlock, PartitionNode,
    TransformBlock,
};
use splot_recon::{DecodedFrame, ReconError};

pub(crate) use decision::{
    INTRA_MODES, InterMode, InterModes, LeafChoice, MotionVector, Partition, PartitionTree,
    ResidualCoder, SuperblockDecision, TransformDecision, TxType,
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

/// Writes the single tile of a key frame, one superblock at a time.
pub(crate) struct KeyFrameTileWriter {
    syntax: IntraTileSyntax,
    symbols: SymbolEncoder,
    sb_cols: usize,
    superblocks: usize,
    written: usize,
    filters: filters::TileFilters,
}

impl KeyFrameTileWriter {
    /// A writer for the frame `core` coded with `seq`, predicting from `refs`.
    pub(crate) fn new(
        seq: &SequenceHeader,
        core: &FrameHeaderCore,
        refs: &EncoderReferences,
    ) -> Result<Self> {
        let syntax = IntraTileSyntax::new(seq, core, refs)?;
        let (mi_rows, mi_cols) = syntax.mi_dimensions();
        let sb4 = syntax.superblock_size4();
        let config = SymbolEncoderConfig::new()
            .with_max_output_bytes(MAX_TILE_BYTES)
            .with_max_operations(MAX_TILE_OPERATIONS);
        Ok(Self {
            syntax,
            symbols: SymbolEncoder::with_config(config),
            sb_cols: mi_cols.div_ceil(sb4),
            superblocks: mi_cols.div_ceil(sb4) * mi_rows.div_ceil(sb4),
            written: 0,
            filters: filters::TileFilters::new(seq, core)?,
        })
    }

    /// The facade, for planning decisions.
    pub(crate) const fn syntax(&self) -> &IntraTileSyntax {
        &self.syntax
    }

    /// The root node of the next superblock in raster order.
    pub(crate) fn next_superblock_root(&self) -> Option<PartitionNode> {
        (self.written < self.superblocks).then(|| {
            let sb4 = self.syntax.superblock_size4();
            self.syntax.superblock_root(
                self.written / self.sb_cols * sb4,
                self.written % self.sb_cols * sb4,
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
        self.filters
            .write_lr_units(&mut self.syntax, &mut self.symbols, root)?;
        Walk {
            syntax: &mut self.syntax,
            sink: &mut self.symbols,
            filters: Some(&mut self.filters),
        }
        .write_node(root, &decision.tree, coder)?;
        self.written += 1;
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

    /// Sets the loop-filter choices the following superblocks code.
    pub(crate) fn set_filter_syntax(&mut self, syntax: filters::FilterSyntax) {
        self.filters.set_syntax(syntax);
    }

    /// The `tile_data` bytes and the reconstructed frame.
    #[cfg(test)]
    pub(crate) fn finish(self) -> Result<(Vec<u8>, DecodedFrame<u16>)> {
        let (tile, recon, _) = self.finish_with_filters()?;
        Ok((tile, recon))
    }

    /// [`Self::finish`] plus the filter records and the coded filter choices.
    pub(crate) fn finish_with_filters(
        self,
    ) -> Result<(Vec<u8>, DecodedFrame<u16>, filters::CodedFilters)> {
        if self.written != self.superblocks {
            return Err(TileWriterError::TileIncomplete {
                written: self.written,
                total: self.superblocks,
            });
        }
        Ok((
            self.symbols.finish()?.into_bytes(),
            self.syntax.finish()?,
            self.filters.finish()?,
        ))
    }
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
        let partition = match tree {
            PartitionTree::Leaf(_) => Partition::None,
            PartitionTree::Split { kind, .. } => *kind,
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
    /// leaf syntax in order: `is_inter`, `cdef_idx`, the modes, the luma
    /// transform partition, and the coefficients of `planes`. Only a pass over
    /// all planes publishes the leaf. An inter leaf codes every plane. Returns
    /// the leaf's transform blocks.
    pub(crate) fn write_leaf(
        &mut self,
        leaf: &LeafBlock,
        choice: LeafChoice,
        coder: &mut impl ResidualCoder,
        planes: Range<usize>,
    ) -> Result<Vec<TransformBlock>> {
        if let Some(modes) = choice.inter {
            return self.write_inter_leaf(leaf, modes, coder);
        }
        let LeafChoice {
            modes,
            tx_partition,
            ..
        } = choice;
        let mut blocks = self
            .syntax
            .plan_leaf_partitioned(leaf, modes, tx_partition)?;
        let mut levels = vec![Vec::new(); blocks.len()];
        for (index, tx) in blocks.iter_mut().enumerate() {
            if !planes.contains(&tx.plane) {
                continue;
            }
            let prediction = self.syntax.predict(index)?;
            let decision = if tx.plane == 0 {
                self.code_luma(coder, tx, &prediction)?
            } else {
                code_as(self.syntax, coder, tx, &prediction)?
            };
            self.syntax
                .reconstruct_as(index, decision.tx_type, &decision.levels)?;
            tx.tx_type = decision.tx_type;
            levels[index] = decision.levels;
        }
        self.syntax.write_is_inter(leaf, false, self.sink)?;
        if let Some(filters) = self.filters.as_deref_mut() {
            filters.write_cdef_index(self.syntax, self.sink, leaf, false)?;
        }
        self.syntax.write_intra_modes(leaf, modes, self.sink)?;
        let sink = &mut *self.sink;
        self.syntax
            .code_luma_tx_partition(leaf, tx_partition, |cdf, value| {
                coeffs::write_symbol(sink, cdf, value)
            })?;
        let eobs = self.write_residual(&blocks, &levels, &planes)?;
        if planes == (0..3) {
            self.syntax.publish_intra_leaf(leaf, modes)?;
            if let Some(filters) = self.filters.as_deref_mut() {
                filters.record_leaf(leaf, &blocks, &eobs)?;
            }
        }
        Ok(blocks)
    }

    /// Codes the inter leaf `leaf` under `modes`: its residual over the motion
    /// compensated prediction, then `is_inter`, `skip_txfm`, `cdef_idx`, the
    /// reference and mode syntax, the motion vector difference, and the
    /// residual. A leaf whose levels are all zero codes `skip_txfm = 1`.
    fn write_inter_leaf(
        &mut self,
        leaf: &LeafBlock,
        mut modes: InterModes,
        coder: &mut impl ResidualCoder,
    ) -> Result<Vec<TransformBlock>> {
        if modes.mode == InterMode::New {
            let base = self.syntax.newmv_base(leaf, modes.ref_frame, modes.drl)?;
            modes.mv = snap_mv(modes.mv, base, self.syntax.mv_step());
        }
        let mut blocks = self.syntax.plan_inter_leaf(leaf, modes)?;
        let mut levels = vec![Vec::new(); blocks.len()];
        let mut luma = None;
        for (index, tx) in blocks.iter_mut().enumerate() {
            if let (true, Some(luma)) = (tx.plane > 0, luma.as_ref()) {
                tx.tx_type = self.syntax.inter_chroma_tx_type(leaf, luma, tx)?;
            }
            let prediction = self.syntax.reconstruction(tx)?;
            let decision = if tx.plane == 0 {
                self.code_luma(coder, tx, &prediction)?
            } else {
                code_as(self.syntax, coder, tx, &prediction)?
            };
            tx.tx_type = decision.tx_type;
            levels[index] = decision.levels;
            if tx.plane == 0 {
                luma = Some(*tx);
            }
        }
        let skip = levels.iter().flatten().all(|&level| level == 0);
        if !skip {
            self.syntax.reconstruct_inter(&blocks, &levels)?;
        }
        self.syntax.write_is_inter(leaf, true, self.sink)?;
        self.syntax.write_skip_txfm(leaf, skip, self.sink)?;
        if let Some(filters) = self.filters.as_deref_mut() {
            filters.write_cdef_index(self.syntax, self.sink, leaf, skip)?;
        }
        if let Some(mvd) = self.syntax.write_inter_modes(leaf, self.sink)? {
            mv::write_mvd(self.syntax, self.sink, mvd)?;
        }
        if !skip {
            self.syntax.write_inter_tx_partition(leaf, self.sink)?;
            self.write_residual(&blocks, &levels, &(0..3))?;
        }
        if let Some(filters) = self.filters.as_deref_mut() {
            filters.record_inter_leaf(self.syntax, leaf, skip)?;
        }
        self.syntax.publish_inter_leaf(leaf, skip)?;
        Ok(blocks)
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

/// `mv` moved to the nearest vector `base` plus a multiple of `step` per
/// component, which a `NEWMV` over `base` can code.
pub(crate) fn snap_mv(mv: MotionVector, base: MotionVector, step: i32) -> MotionVector {
    let round = |v: i32, b: i32| b + (v - b + step / 2).div_euclid(step) * step;
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
    if decision.tx_type != tx.tx_type {
        return Err(TileWriterError::Unsupported(
            "transform type other than the requested one",
        ));
    }
    Ok(decision)
}

#[cfg(test)]
mod tests;

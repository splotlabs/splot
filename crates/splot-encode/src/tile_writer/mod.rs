// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Key-frame tile writer.
//!
//! [`KeyFrameTileWriter`] walks each [`SuperblockDecision`] in § 5.20.3.1 syntax
//! order, writes every symbol through [`SymbolEncoder`] with the CDF rows and
//! contexts of the decoder's `encoder_syntax` facade, publishes the coded facts
//! back to the facade, and reconstructs each transform block.

pub(crate) mod coeffs;
mod decision;
pub(crate) mod filters;

use splot_core::headers::frame::{FrameHeaderCore, KEY_FRAME_REDUCED_TX_SET, TxMode};
use splot_core::headers::sequence::{ChromaFormatIdc, SequenceHeader};
use splot_core::symbol::Symbol;
use splot_core::symbol_encoder::{SymbolEncoder, SymbolEncoderConfig};
use splot_core::write::WriteError;
use splot_decode::encoder_syntax::{
    EncoderSyntaxError, IntraTileSyntax, LeafBlock, PartitionCdf, PartitionNode, RectKind,
};
use splot_recon::{BitDepth, DecodedFrame, PixelFormat, ReconError};

pub(crate) use decision::{
    BlockDecision, ChromaMode, IntraMode, LumaMode, Partition, PartitionTree, ResidualCoder,
    SuperblockDecision, TransformDecision, TxType,
};

use crate::recon::ReconFrame;

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
    recon: ReconFrame,
    bit_depth: BitDepth,
    sb_cols: usize,
    superblocks: usize,
    written: usize,
    filters: filters::TileFilters,
}

impl KeyFrameTileWriter {
    /// A writer for the frame `core` coded with `seq`.
    pub(crate) fn new(seq: &SequenceHeader, core: &FrameHeaderCore) -> Result<Self> {
        check_supported(seq, core)?;
        let base_q_idx = core
            .quantization_params
            .ok_or(TileWriterError::Unsupported("missing quantization params"))?
            .base_q_idx;
        let size = core
            .frame_size
            .ok_or(TileWriterError::Unsupported("missing frame size"))?;
        let syntax = IntraTileSyntax::new(seq, core)?;
        let (mi_rows, mi_cols) = syntax.mi_dimensions();
        let sb4 = syntax.superblock_size4();
        let bit_depth = if seq.general.bit_depth_idc.get() == 1 {
            BitDepth::Eight
        } else {
            BitDepth::Ten
        };
        let recon = ReconFrame::new(
            (size.width, size.height),
            (mi_cols, mi_rows),
            bit_depth,
            PixelFormat::Yuv420,
            base_q_idx,
        )?;
        let config = SymbolEncoderConfig::new()
            .with_max_output_bytes(MAX_TILE_BYTES)
            .with_max_operations(MAX_TILE_OPERATIONS);
        Ok(Self {
            syntax,
            symbols: SymbolEncoder::with_config(config),
            recon,
            bit_depth,
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
        self.write_node(root, &decision.tree, coder)?;
        self.written += 1;
        Ok(())
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
            self.recon.into_frame()?,
            self.filters.finish()?,
        ))
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
            PartitionTree::Leaf(block) => self.write_leaf(&self.syntax.leaf(node)?, *block, coder),
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
    fn write_partition(&mut self, node: PartitionNode, partition: Partition) -> Result<()> {
        let options = self.syntax.partition_options(node)?;
        let refuse = |reason| TileWriterError::Partition {
            partition,
            row4: node.row4(),
            col4: node.col4(),
            reason,
        };
        if !options.allows(partition) {
            return Err(refuse("not allowed"));
        }
        if let Some(implied) = options.implied().filter(|&p| options.allows(p)) {
            return if implied == partition {
                Ok(())
            } else {
                Err(refuse("position implies another partition"))
            };
        }
        if PARTITIONS.iter().filter(|&&p| options.allows(p)).count() == 1 {
            return Ok(());
        }
        if options.allows(Partition::None) {
            self.partition_bool(node, PartitionCdf::DoSplit, partition != Partition::None)?;
            if partition == Partition::None {
                return Ok(());
            }
        }
        if options.allows(Partition::Split) {
            self.partition_bool(
                node,
                PartitionCdf::DoSquareSplit,
                partition == Partition::Split,
            )?;
            if partition == Partition::Split {
                return Ok(());
            }
        }
        let (kind, family) = rect_family(partition).ok_or_else(|| refuse("not a rectangle"))?;
        let allow = |kind| family_of(kind).iter().any(|&p| options.allows(p));
        match options.rect_type() {
            Some(implied) if implied != kind => {
                return Err(refuse("block size implies the other rectangle"));
            }
            Some(_) => {}
            None if !allow(RectKind::Horz) || !allow(RectKind::Vert) => {}
            None => self.partition_bool(node, PartitionCdf::RectType, kind == RectKind::Vert)?,
        }
        let [non_ext, three, four_a, four_b] = family;
        let ext3 = options.allows(three);
        let ext4 = options.allows(four_a) || options.allows(four_b);
        let do_ext = partition != non_ext;
        if options.allows(non_ext) && (ext3 || ext4) {
            self.partition_bool(node, PartitionCdf::DoExtPartition(kind), do_ext)?;
        }
        if !do_ext {
            return Ok(());
        }
        let uneven = partition == four_a || partition == four_b;
        if ext3 && ext4 {
            self.partition_bool(node, PartitionCdf::DoUneven4Way(kind), uneven)?;
        }
        if uneven {
            self.symbols
                .write_literal(u32::from(partition == four_b), 1)?;
        }
        Ok(())
    }

    fn partition_bool(
        &mut self,
        node: PartitionNode,
        cdf: PartitionCdf,
        value: bool,
    ) -> Result<()> {
        let row = self.syntax.partition_cdf(node, cdf)?;
        self.symbols
            .write_symbol_u16(row, Symbol::new(u8::from(value)))?;
        Ok(())
    }

    fn write_leaf(
        &mut self,
        leaf: &LeafBlock,
        block: BlockDecision,
        coder: &mut impl ResidualCoder,
    ) -> Result<()> {
        self.filters
            .write_cdef_index(&mut self.syntax, &mut self.symbols, leaf)?;
        if block.y_mode
            != (LumaMode {
                mode: IntraMode::Dc,
                angle_delta: 0,
            })
        {
            return Err(TileWriterError::Unsupported("luma mode other than DC_PRED"));
        }
        let symbol = Symbol::new(0);
        self.symbols
            .write_symbol_u16(self.syntax.y_mode_set_cdf()?, symbol)?;
        self.symbols
            .write_symbol_u16(self.syntax.y_mode_index_cdf(leaf)?, symbol)?;
        if leaf.has_chroma() {
            if block.uv_mode
                != (ChromaMode::Intra {
                    mode: IntraMode::Dc,
                    angle_delta: 0,
                })
            {
                return Err(TileWriterError::Unsupported(
                    "chroma mode other than DC_PRED",
                ));
            }
            self.symbols
                .write_symbol_u16(self.syntax.uv_mode_cdf(false)?, symbol)?;
        }
        self.write_residual(leaf, coder)?;
        self.syntax.publish_dc_leaf(leaf)?;
        Ok(())
    }

    fn write_residual(&mut self, leaf: &LeafBlock, coder: &mut impl ResidualCoder) -> Result<()> {
        let mut eob_u = 0;
        let transforms = self.syntax.transform_blocks(leaf, self.bit_depth)?;
        let mut eobs = Vec::with_capacity(transforms.len());
        for tx in &transforms {
            let prediction = self.recon.predict_dc(tx)?;
            let decision = coder.code(tx, &prediction)?;
            if decision.tx_type != TxType::DctDct {
                return Err(TileWriterError::Unsupported(
                    "transform type other than DCT_DCT",
                ));
            }
            let eob = coeffs::write_coeffs(
                &mut self.syntax,
                &mut self.symbols,
                tx,
                &decision.levels,
                tx.plane == 2 && eob_u != 0,
            )?;
            if tx.plane == 1 {
                eob_u = eob;
            }
            self.recon
                .reconstruct(tx, &prediction, &decision.levels, eob)?;
            eobs.push(eob);
        }
        self.filters.record_leaf(leaf, &transforms, &eobs)
    }
}

/// The rectangle direction of `partition` and its family `[non-ext, 3-way, 4A, 4B]`.
fn rect_family(partition: Partition) -> Option<(RectKind, [Partition; 4])> {
    let kind = match partition {
        Partition::Horz | Partition::Horz3 | Partition::Horz4A | Partition::Horz4B => {
            RectKind::Horz
        }
        Partition::Vert | Partition::Vert3 | Partition::Vert4A | Partition::Vert4B => {
            RectKind::Vert
        }
        Partition::None | Partition::Split => return None,
    };
    Some((kind, family_of(kind)))
}

const fn family_of(kind: RectKind) -> [Partition; 4] {
    match kind {
        RectKind::Horz => [
            Partition::Horz,
            Partition::Horz3,
            Partition::Horz4A,
            Partition::Horz4B,
        ],
        RectKind::Vert => [
            Partition::Vert,
            Partition::Vert3,
            Partition::Vert4A,
            Partition::Vert4B,
        ],
    }
}

/// Rejects every sequence or frame tool the writer does not code.
fn check_supported(seq: &SequenceHeader, core: &FrameHeaderCore) -> Result<()> {
    let unsupported = |what| Err(TileWriterError::Unsupported(what));
    if seq.general.chroma_format_idc != ChromaFormatIdc::Yuv420 {
        return unsupported("chroma format other than 4:2:0");
    }
    let (Some(intra), Some(tq), Some(partition)) =
        (seq.intra, seq.transform_quant_entropy, seq.partition)
    else {
        return unsupported("incomplete sequence header");
    };
    if partition.enable_sdp {
        return unsupported("semi-decoupled partitioning");
    }
    if intra.enable_cfl_intra || intra.enable_mhccp || intra.enable_dip || intra.enable_ibp {
        return unsupported("CFL, MHCCP, DIP, or IBP");
    }
    if tq.enable_idtx_intra || tq.enable_intra_ist || tq.enable_cctx || tq.enable_tcq {
        return unsupported("IDTX/FSC, intra IST, CCTX, or TCQ");
    }
    let tail = core
        .intra_tail
        .as_ref()
        .ok_or(TileWriterError::Unsupported("incomplete key-frame header"))?;
    if tail.tx_mode != TxMode::Largest || u32::from(tail.reduced_tx_set) != KEY_FRAME_REDUCED_TX_SET
    {
        return unsupported("transform mode other than TX_MODE_LARGEST with reduced_tx_set 2");
    }
    if core.allow_screen_content_tools == Some(true) || core.allow_intrabc == Some(true) {
        return unsupported("screen content tools");
    }
    let lossless = core.lossless_info.as_ref();
    if lossless.is_some_and(|info| info.allow_tcq || info.allow_parity_hiding) {
        return unsupported("TCQ or parity hiding");
    }
    if core
        .segmentation_params
        .as_ref()
        .is_some_and(|seg| seg.segmentation_enabled)
        || core
            .delta_q_params
            .as_ref()
            .is_some_and(|dq| dq.delta_q_present)
        || core
            .setup_qm_params
            .as_ref()
            .is_some_and(|qm| qm.using_qmatrix)
    {
        return unsupported("segmentation, delta-q, or quantizer matrices");
    }
    Ok(())
}

#[cfg(test)]
mod tests;

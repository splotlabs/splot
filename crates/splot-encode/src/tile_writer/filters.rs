// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Loop-filter syntax of the key-frame tile writer: `cdef_idx` (§ 5.20.10.1) and
//! `use_pc_wiener` (§ 5.20.10.5), plus the per-leaf records of the decoder's
//! filter chain.
//!
//! The writer calls three hooks: [`TileFilters::write_lr_units`] at each
//! superblock root before its partition symbols, [`TileFilters::write_cdef_index`]
//! at the start of each leaf (after `skip_txfm` of an inter leaf), and
//! [`TileFilters::record_leaf`] or [`TileFilters::record_inter_leaf`] after
//! each leaf's residual.

use std::collections::BTreeSet;

use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::sequence::SequenceHeader;
use splot_core::symbol::Symbol;
use splot_core::symbol_encoder::SymbolSink;
use splot_decode::encoder_syntax::{
    CdefIndexMap, FilterRecorder, IntraTileSyntax, LeafBlock, LrLayout, LrUnit, PartitionNode,
    TransformBlock,
};

use super::{Result, TileWriterError};

/// The tile-level filter choices a writer codes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct FilterSyntax {
    /// `cdef_idx` per 64x64 unit in raster order; a missing entry codes `0`.
    pub(crate) cdef_indices: Vec<Option<usize>>,
    /// The restoration units whose `use_pc_wiener` is `1`.
    pub(crate) lr_on: BTreeSet<LrUnit>,
}

/// The filter records of a finished tile and the filter choices it coded.
pub(crate) type CodedFilters = (FilterRecorder, FilterSyntax);

/// The filter state of one tile.
pub(crate) struct TileFilters {
    recorder: FilterRecorder,
    cdef: CdefIndexMap,
    lr: LrLayout,
    syntax: FilterSyntax,
}

impl TileFilters {
    /// The state of tile `tile` of the frame `core` coded with `seq`; it codes
    /// every `cdef_idx` as `0` and every restoration unit off until
    /// [`Self::set_syntax`].
    pub(crate) fn new(seq: &SequenceHeader, core: &FrameHeaderCore, tile: usize) -> Result<Self> {
        Ok(Self {
            recorder: FilterRecorder::new(seq, core)?,
            cdef: CdefIndexMap::new(seq, core, tile)?,
            lr: LrLayout::new(seq, core, tile)?,
            syntax: FilterSyntax::default(),
        })
    }

    /// Sets the choices the following superblocks code.
    pub(crate) fn set_syntax(&mut self, syntax: FilterSyntax) {
        self.syntax = syntax;
    }

    /// Writes `use_pc_wiener` of every unit `read_lr` codes at superblock `root`.
    pub(crate) fn write_lr_units(
        &self,
        syntax: &mut IntraTileSyntax,
        symbols: &mut impl SymbolSink,
        root: PartitionNode,
    ) -> Result<()> {
        for unit in self.lr.units(root)? {
            let on = self.syntax.lr_on.contains(&unit);
            symbols.symbol(syntax.use_pc_wiener_cdf()?, Symbol::new(u8::from(on)))?;
        }
        Ok(())
    }

    /// Writes `cdef_idx` when `leaf`, coded with `skip_txfm = skip`, is the
    /// first leaf of its 64x64 unit that codes one.
    pub(crate) fn write_cdef_index(
        &mut self,
        syntax: &mut IntraTileSyntax,
        symbols: &mut impl SymbolSink,
        leaf: &LeafBlock,
        skip: bool,
    ) -> Result<()> {
        let Some(slot) = self.cdef.slot(leaf, skip)? else {
            return Ok(());
        };
        let cols = self.cdef.grid().1;
        let index = self
            .syntax
            .cdef_indices
            .get(slot.unit_row * cols + slot.unit_col)
            .copied()
            .flatten()
            .unwrap_or(0);
        if index >= slot.strengths {
            return Err(TileWriterError::Shape("cdef_idx outside the strength sets"));
        }
        if slot.strengths > 1 {
            symbols.symbol(
                syntax.cdef_index0_cdf(slot.ctx)?,
                Symbol::new(u8::from(index == 0)),
            )?;
            if index > 0 && slot.strengths > 2 {
                symbols.symbol(
                    syntax.cdef_index_minus1_cdf(slot.strengths)?,
                    Symbol::new((index - 1) as u8),
                )?;
            }
        }
        self.cdef.set(leaf, index)?;
        Ok(())
    }

    /// Records the transform blocks of a coded leaf and their `eob`s.
    pub(crate) fn record_leaf(
        &mut self,
        leaf: &LeafBlock,
        transforms: &[TransformBlock],
        eobs: &[usize],
    ) -> Result<()> {
        Ok(self.recorder.record_leaf(leaf, transforms, eobs)?)
    }

    /// Records the planned inter leaf of `syntax` before it is published.
    pub(crate) fn record_inter_leaf(
        &mut self,
        syntax: &IntraTileSyntax,
        leaf: &LeafBlock,
        skip: bool,
    ) -> Result<()> {
        Ok(self.recorder.record_inter_leaf(syntax, leaf, skip)?)
    }

    /// The records and the coded choices.
    pub(crate) fn finish(self) -> Result<CodedFilters> {
        let cdef_indices = self.cdef.into_indices()?;
        Ok((
            self.recorder,
            FilterSyntax {
                cdef_indices,
                lr_on: self.syntax.lr_on,
            },
        ))
    }
}

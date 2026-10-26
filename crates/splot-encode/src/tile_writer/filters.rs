// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Loop-filter syntax of the key-frame tile writer: `use_gdf` (§ 5.20.10.3),
//! `cdef_idx` (§ 5.20.10.1), `ccso_blk` (§ 5.20.10.2) and the `read_lr_unit`
//! type (§ 5.20.10.5), plus the per-leaf records of the decoder's filter chain.
//!
//! The writer calls three hooks: [`TileFilters::write_lr_units`] at each
//! superblock root before its partition symbols, [`TileFilters::write_leaf_syntax`]
//! at the start of each leaf (after `skip_txfm` of an inter leaf), and
//! [`TileFilters::record_leaf`] or [`TileFilters::record_inter_leaf`] after
//! each leaf's residual.

use std::collections::BTreeMap;

use splot_core::headers::frame::{FrameHeaderCore, FrameRestorationType};
use splot_core::headers::sequence::SequenceHeader;
use splot_core::symbol::Symbol;
use splot_core::symbol_encoder::SymbolSink;
use splot_decode::encoder_syntax::{
    CcsoBlockMap, CcsoGrid, CdefIndexMap, FilterRecorder, GdfBlockMap, GdfGrid, IntraTileSyntax,
    LeafBlock, LrLayout, LrSymbol, LrUnit, LrUnitType, PartitionNode, TransformBlock,
};

use super::{Result, TileWriterError};

/// The tile-level filter choices a writer codes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct FilterSyntax {
    /// `cdef_idx` per 64x64 unit in raster order; a missing entry codes `0`.
    pub(crate) cdef_indices: Vec<Option<usize>>,
    /// The restoring units and their type; a missing unit codes `RESTORE_NONE`.
    pub(crate) lr_on: BTreeMap<LrUnit, LrUnitType>,
    /// `ccso_blk` per plane and [`CcsoGrid`] unit in raster order; a missing
    /// entry codes `0`.
    pub(crate) ccso_blocks: Vec<Vec<u8>>,
    /// `use_gdf` per [`GdfGrid`] unit in raster order; a missing entry codes `0`.
    pub(crate) gdf_blocks: Vec<u8>,
}

/// The filter records of a finished tile and the filter choices it coded.
pub(crate) type CodedFilters = (FilterRecorder, FilterSyntax);

/// The symbols that code `kind` (`None` is `RESTORE_NONE`) on a `plane` coded
/// with `tool`, in order (§ 5.20.10.5).
pub(crate) fn lr_unit_symbols(
    tool: FrameRestorationType,
    plane: usize,
    kind: Option<LrUnitType>,
) -> Result<Vec<(LrSymbol, bool)>> {
    let flex = |tool| LrSymbol::Flex { tool, plane };
    Ok(match (tool, kind) {
        (FrameRestorationType::PcWiener, None | Some(LrUnitType::PcWiener)) => {
            vec![(LrSymbol::UsePcWiener, kind.is_some())]
        }
        (FrameRestorationType::WienerNonsep, None | Some(LrUnitType::WienerNs)) => {
            vec![(LrSymbol::UseWienerNs, kind.is_some())]
        }
        (FrameRestorationType::Switchable, None) => vec![(flex(0), true)],
        (FrameRestorationType::Switchable, Some(LrUnitType::PcWiener)) => {
            vec![(flex(0), false), (flex(1), true)]
        }
        (FrameRestorationType::Switchable, Some(LrUnitType::WienerNs)) => {
            vec![(flex(0), false), (flex(1), false)]
        }
        _ => {
            return Err(TileWriterError::Shape(
                "a restoration unit type the plane does not code",
            ));
        }
    })
}

/// The filter state of one tile.
pub(crate) struct TileFilters {
    recorder: FilterRecorder,
    cdef: CdefIndexMap,
    ccso: CcsoBlockMap,
    ccso_cols: usize,
    gdf: GdfBlockMap,
    gdf_cols: usize,
    lr: LrLayout,
    lr_tools: [FrameRestorationType; 3],
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
            ccso: CcsoBlockMap::new(seq, core, tile)?,
            ccso_cols: CcsoGrid::new(seq, core)?.cols,
            gdf: GdfBlockMap::new(seq, core, tile)?,
            gdf_cols: GdfGrid::new(seq, core)?.map_or(0, |grid| grid.cols),
            lr: LrLayout::new(seq, core, tile)?,
            lr_tools: core::array::from_fn(|plane| {
                core.lr_params
                    .as_ref()
                    .and_then(|lr| lr.planes.get(plane))
                    .map_or(FrameRestorationType::None, |p| p.restoration_type)
            }),
            syntax: FilterSyntax::default(),
        })
    }

    /// Sets the choices the following superblocks code.
    pub(crate) fn set_syntax(&mut self, syntax: FilterSyntax) {
        self.syntax = syntax;
    }

    /// Writes the type of every unit `read_lr` codes at superblock `root`.
    pub(crate) fn write_lr_units(
        &self,
        syntax: &mut IntraTileSyntax,
        symbols: &mut impl SymbolSink,
        root: PartitionNode,
    ) -> Result<()> {
        for unit in self.lr.units(root)? {
            let kind = self.syntax.lr_on.get(&unit).copied();
            for (symbol, value) in lr_unit_symbols(self.lr_tools[unit.plane], unit.plane, kind)? {
                symbols.symbol(syntax.lr_symbol_cdf(symbol)?, Symbol::new(u8::from(value)))?;
            }
        }
        Ok(())
    }

    /// Writes `use_gdf`, `cdef_idx`, `ccso_blk` and `delta_q_abs` where
    /// `leaf`, coded with `skip_txfm = skip`, codes them.
    pub(crate) fn write_leaf_syntax(
        &mut self,
        syntax: &mut IntraTileSyntax,
        symbols: &mut impl SymbolSink,
        leaf: &LeafBlock,
        skip: bool,
    ) -> Result<()> {
        if leaf.is_chroma_part() {
            return Ok(()); // § 5.20.5.3: a CHROMA_PART leaf codes none of them
        }
        if let Some((row, col)) = self.gdf.unit(leaf) {
            let on = self
                .syntax
                .gdf_blocks
                .get(row * self.gdf_cols + col)
                .is_some_and(|&value| value != 0);
            symbols.symbol(syntax.use_gdf_cdf()?, Symbol::new(u8::from(on)))?;
        }
        self.write_cdef_index(syntax, symbols, leaf, skip)?;
        self.write_ccso_blocks(syntax, symbols, leaf)?;
        Ok(syntax.write_delta_qindex(leaf, skip, symbols)?)
    }

    /// Writes `cdef_idx` when `leaf`, coded with `skip_txfm = skip`, is the
    /// first leaf of its 64x64 unit that codes one.
    fn write_cdef_index(
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

    /// Writes `ccso_blk` of every coded plane when `leaf` starts a CCSO unit.
    fn write_ccso_blocks(
        &mut self,
        syntax: &mut IntraTileSyntax,
        symbols: &mut impl SymbolSink,
        leaf: &LeafBlock,
    ) -> Result<()> {
        let Some(unit) = self.ccso.unit(leaf) else {
            return Ok(());
        };
        for plane in 0..3 {
            let Some(ctx) = self.ccso.ctx(plane, unit) else {
                continue;
            };
            let on = self
                .syntax
                .ccso_blocks
                .get(plane)
                .and_then(|blocks| blocks.get(unit.0 * self.ccso_cols + unit.1))
                .is_some_and(|&value| value != 0);
            symbols.symbol(syntax.ccso_blk_cdf(plane, ctx)?, Symbol::new(u8::from(on)))?;
            self.ccso.set(plane, unit, on)?;
        }
        Ok(())
    }

    /// Records the transform blocks of a coded leaf and their `eob`s.
    pub(crate) fn record_leaf(
        &mut self,
        syntax: &IntraTileSyntax,
        leaf: &LeafBlock,
        transforms: &[TransformBlock],
        eobs: &[usize],
    ) -> Result<()> {
        Ok(self.recorder.record_leaf(syntax, leaf, transforms, eobs)?)
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
                ccso_blocks: self.syntax.ccso_blocks,
                gdf_blocks: self.syntax.gdf_blocks,
            },
        ))
    }
}

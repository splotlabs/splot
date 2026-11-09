// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 5.20.3.1 partition traversal.

use splot_core::symbol::{CdfValidationMode, SymbolDecoder, SymbolDecoderConfig};

use super::DecodeTileWorkUnit;
use super::cdf::block_context::{IntraJointMode, IntraYMode, MrlSelection};
use super::cdf::context::{PartitionContextInput, SquareSplitContextInput};
use super::cdf::{self, TileCdfError};
use super::intra_joint_modes::{
    IntraFacts, LumaPalette, TileIntraFactsState, TileIntraFactsStateError,
};
use super::mi_size_state::{TileMiSizeState, TileMiSizeStateError};
use super::partition::{self, PartitionDecisionError, PartitionType};
use super::partition_allowed::{
    PartitionAllowedError, PartitionAllowedInput, PartitionDecisionMemo, PartitionFeatureFlags,
    PartitionTreeType, decision_memo_key,
};
use super::partition_size::{
    BlockSize, PartitionSizeError, h_partition_midsize, partition_subsize,
};
use crate::{DecodeLimitError, DecodeLimitName, DecodeLimits};

mod lr_records;
mod lr_syntax;
mod partition_children;
mod state_publication;
mod tree_walk;

pub(crate) use lr_records::LrUnitRestorationType;
pub(crate) use lr_records::{WienerNsLrSourceBlock, WienerNsLrUnitFilter};
#[cfg(test)]
use lr_syntax::{WienerNsUnitFilterState, read_wiener_ns_unit_filter};
pub(crate) use lr_syntax::{pc_wiener_unit_source_blocks, superblock_lr_units};
pub(crate) use partition_children::{ChildCallSink, child_calls, child_calls_into};
pub(crate) use state_publication::DecodedLeafPublication;
pub(crate) use state_publication::publish_intra_leaf_state;
use tree_walk::extended_sdp_allowed_for_child;
pub(crate) use tree_walk::valid_subsize;
pub(crate) use tree_walk::{
    GeneralIntraPartitionTreeCursor, GeneralIntraPartitionTreeOutput, GeneralIntraTreeWalkError,
    LeafNeighbours, SdpPartitionState, TileTraversalStorage, decode_block_frontier,
    decode_block_part, is_intra_sdp_shared_root, partition_allowed_input, partition_context_inputs,
    updated_chroma_offset,
};
#[cfg(test)]
use tree_walk::{
    MIXED_REGION, partition_cdf_plane, read_extended_sdp_region_type,
    read_frontier_partition_decision, should_read_extended_sdp_region_type,
};

pub(crate) const BLOCK_8X32: usize = 21;
pub(crate) const BLOCK_32X8: usize = 22;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TilePartitionLoopRestorationState {
    NoSyntax,
    Frame(TilePartitionLoopRestorationFrameState),
    UnsupportedReadLrSyntax,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TilePartitionLoopRestorationPlaneTool {
    None,
    WienerNs,
    PcWiener,
    Switchable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TilePartitionLoopRestorationFrameState {
    plane_tool: [TilePartitionLoopRestorationPlaneTool; 3],
    frame_filters_on: [bool; 3],
    unit_size: [usize; 3],
}

impl TilePartitionLoopRestorationFrameState {
    #[must_use]
    pub(crate) const fn new(
        plane_tool: [TilePartitionLoopRestorationPlaneTool; 3],
        frame_filters_on: [bool; 3],
        unit_size: [usize; 3],
    ) -> Self {
        Self {
            plane_tool,
            frame_filters_on,
            unit_size,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TilePartitionFrameFacts {
    mi_rows: usize,
    mi_cols: usize,
    sb_size: BlockSize,
    num_planes: usize,
    pub(crate) subsampling_x: bool,
    pub(crate) subsampling_y: bool,
    frame_is_intra: bool,
    enable_sdp: bool,
    enable_extended_sdp: bool,
    disable_loopfilters_across_tiles: bool,
    loop_restoration: TilePartitionLoopRestorationState,
    features: PartitionFeatureFlags,
    max_pb_aspect_ratio: usize,
}

impl TilePartitionFrameFacts {
    #[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
    pub(crate) fn new(
        mi_rows: usize,
        mi_cols: usize,
        sb_size: usize,
        num_planes: usize,
        subsampling_x: bool,
        subsampling_y: bool,
        frame_is_intra: bool,
        enable_sdp: bool,
        enable_extended_sdp: bool,
        disable_loopfilters_across_tiles: bool,
        loop_restoration: TilePartitionLoopRestorationState,
        features: PartitionFeatureFlags,
        max_pb_aspect_ratio: usize,
    ) -> Result<Self, TilePartitionTraversalError> {
        Ok(Self {
            mi_rows,
            mi_cols,
            sb_size: BlockSize::new(sb_size)?,
            num_planes,
            subsampling_x,
            subsampling_y,
            frame_is_intra,
            enable_sdp,
            enable_extended_sdp,
            disable_loopfilters_across_tiles,
            loop_restoration,
            features,
            max_pb_aspect_ratio,
        })
    }

    #[must_use]
    pub(crate) const fn sb_size(&self) -> BlockSize {
        self.sb_size
    }

    #[must_use]
    pub(crate) const fn mi_rows(&self) -> usize {
        self.mi_rows
    }

    #[must_use]
    pub(crate) const fn mi_cols(&self) -> usize {
        self.mi_cols
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ChromaRefGeometry {
    row: u32,
    col: u32,
    size: BlockSize,
}

impl ChromaRefGeometry {
    pub(crate) const fn new(row: usize, col: usize, size: BlockSize) -> Self {
        Self {
            row: row as u32,
            col: col as u32,
            size,
        }
    }

    pub(crate) const fn row(self) -> usize {
        self.row as usize
    }

    pub(crate) const fn col(self) -> usize {
        self.col as usize
    }

    pub(crate) const fn size(self) -> BlockSize {
        self.size
    }
}

const CALL_CHROMA_OFFSET: u8 = 1;
const CALL_HAS_CHROMA: u8 = 1 << 1;
const CALL_CFL_ALLOWED_IN_SDP: u8 = 1 << 2;
const CALL_EXTENDED_SDP_ALLOWED: u8 = 1 << 3;
const CALL_INTRA_REGION: u8 = 1 << 4;
const CALL_EXTENDED_SDP_CHROMA_BLOCK: u8 = 1 << 5;

/// One § 5.20.3.1 `decode_partition` call with its propagated state, packed
/// into 24 bytes: the walk copies one per node and per child.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TilePartitionCall {
    r: u32,
    c: u32,
    chroma_row: u32,
    chroma_col: u32,
    pub(crate) b_size: BlockSize,
    chroma_size: BlockSize,
    pub(crate) parent_size: Option<BlockSize>,
    tree_type: PartitionTreeType,
    flags: u8,
}

const fn call_flag(value: bool, flag: u8) -> u8 {
    if value { flag } else { 0 }
}

impl TilePartitionCall {
    pub(crate) const fn root(r: usize, c: usize, b_size: BlockSize, has_chroma: bool) -> Self {
        Self {
            r: r as u32,
            c: c as u32,
            chroma_row: r as u32,
            chroma_col: c as u32,
            b_size,
            chroma_size: b_size,
            parent_size: None,
            tree_type: PartitionTreeType::Shared,
            flags: call_flag(has_chroma, CALL_HAS_CHROMA)
                | CALL_CFL_ALLOWED_IN_SDP
                | CALL_EXTENDED_SDP_ALLOWED,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) const fn child(
        r: usize,
        c: usize,
        b_size: BlockSize,
        parent_size: Option<BlockSize>,
        chroma_offset: bool,
        has_chroma: bool,
        tree_type: PartitionTreeType,
        chroma_ref: Option<ChromaRefGeometry>,
        extended_sdp_allowed: bool,
        intra_region: bool,
    ) -> Self {
        let chroma_ref = match chroma_ref {
            Some(chroma_ref) => chroma_ref,
            None => ChromaRefGeometry::new(r, c, b_size),
        };
        Self {
            r: r as u32,
            c: c as u32,
            chroma_row: chroma_ref.row,
            chroma_col: chroma_ref.col,
            b_size,
            chroma_size: chroma_ref.size,
            parent_size,
            tree_type,
            flags: call_flag(chroma_offset, CALL_CHROMA_OFFSET)
                | call_flag(has_chroma, CALL_HAS_CHROMA)
                | CALL_CFL_ALLOWED_IN_SDP
                | call_flag(extended_sdp_allowed, CALL_EXTENDED_SDP_ALLOWED)
                | call_flag(intra_region, CALL_INTRA_REGION),
        }
    }

    pub(crate) const fn r(self) -> usize {
        self.r as usize
    }

    pub(crate) const fn c(self) -> usize {
        self.c as usize
    }

    const fn flag(self, flag: u8) -> bool {
        self.flags & flag != 0
    }

    const fn with_flag(self, flag: u8, value: bool) -> Self {
        Self {
            flags: self.flags & !flag | call_flag(value, flag),
            ..self
        }
    }

    pub(crate) const fn chroma_offset(self) -> bool {
        self.flag(CALL_CHROMA_OFFSET)
    }

    pub(crate) const fn has_chroma(self) -> bool {
        self.flag(CALL_HAS_CHROMA)
    }

    const fn extended_sdp_allowed(self) -> bool {
        self.flag(CALL_EXTENDED_SDP_ALLOWED)
    }

    const fn intra_region(self) -> bool {
        self.flag(CALL_INTRA_REGION)
    }

    /// Whether this call is the collocated extended-SDP chroma block, which
    /// § 5.20.3.1 decodes as one block after its luma tree.
    const fn is_extended_sdp_chroma_block(self) -> bool {
        self.flag(CALL_EXTENDED_SDP_CHROMA_BLOCK)
    }

    pub(crate) const fn tree_type(self) -> PartitionTreeType {
        self.tree_type
    }

    pub(crate) const fn cfl_allowed_in_sdp(self) -> bool {
        self.flag(CALL_CFL_ALLOWED_IN_SDP)
    }

    pub(crate) const fn chroma_ref_geometry(self) -> ChromaRefGeometry {
        if !self.chroma_offset() && self.has_chroma() {
            return ChromaRefGeometry {
                row: self.r,
                col: self.c,
                size: self.b_size,
            };
        }
        ChromaRefGeometry {
            row: self.chroma_row,
            col: self.chroma_col,
            size: self.chroma_size,
        }
    }

    pub(crate) const fn with_tree_type(self, tree_type: PartitionTreeType) -> Self {
        Self { tree_type, ..self }
    }

    pub(crate) const fn with_cfl_allowed_in_sdp(self, cfl_allowed_in_sdp: bool) -> Self {
        self.with_flag(CALL_CFL_ALLOWED_IN_SDP, cfl_allowed_in_sdp)
    }

    const fn with_intra_region(self, intra_region: bool) -> Self {
        self.with_flag(CALL_INTRA_REGION, intra_region)
    }

    /// The child of a template call at (`r`, `c`) with its own chroma state.
    #[inline]
    const fn at_child(
        self,
        r: usize,
        c: usize,
        b_size: BlockSize,
        chroma_offset: bool,
        has_chroma: bool,
        chroma_ref: ChromaRefGeometry,
    ) -> Self {
        Self {
            r: r as u32,
            c: c as u32,
            chroma_row: chroma_ref.row,
            chroma_col: chroma_ref.col,
            b_size,
            chroma_size: chroma_ref.size,
            flags: self.flags & !(CALL_CHROMA_OFFSET | CALL_HAS_CHROMA)
                | call_flag(chroma_offset, CALL_CHROMA_OFFSET)
                | call_flag(has_chroma, CALL_HAS_CHROMA),
            ..self
        }
    }

    const fn as_extended_sdp_chroma_block(self) -> Self {
        self.with_flag(CALL_EXTENDED_SDP_CHROMA_BLOCK, true)
    }
}

#[allow(clippy::struct_field_names)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TilePartitionBounds {
    mi_row_start: usize,
    mi_row_end: usize,
    mi_col_start: usize,
    mi_col_end: usize,
}

impl TilePartitionBounds {
    fn from_work_unit(work_unit: &DecodeTileWorkUnit) -> Self {
        let row_range = work_unit.mi_row_range();
        let col_range = work_unit.mi_col_range();
        Self::new(
            row_range.start as usize..row_range.end as usize,
            col_range.start as usize..col_range.end as usize,
        )
    }

    pub(crate) const fn new(rows: core::ops::Range<usize>, cols: core::ops::Range<usize>) -> Self {
        Self {
            mi_row_start: rows.start,
            mi_row_end: rows.end,
            mi_col_start: cols.start,
            mi_col_end: cols.end,
        }
    }

    /// `AvailU` of a call inside the tile, whose row above is in the tile
    /// exactly when the call is below the tile's first row.
    const fn avail_u(self, call: TilePartitionCall) -> bool {
        call.r() > self.mi_row_start
    }

    /// `AvailL` of a call inside the tile.
    const fn avail_l(self, call: TilePartitionCall) -> bool {
        call.c() > self.mi_col_start
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DecodeBlockFrontier {
    pub(crate) r: usize,
    pub(crate) c: usize,
    pub(crate) b_size: BlockSize,
    pub(crate) has_chroma: bool,
    pub(crate) chroma_offset: bool,
    chroma_ref: ChromaRefGeometry,
    part: DecodeBlockPart,
    intra_region: bool,
    cfl_allowed_in_sdp: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DecodeBlockPart {
    Shared,
    LumaPart,
    ChromaPart {
        y_mode: IntraYMode,
        angle_delta_y: i8,
    },
}

impl DecodeBlockFrontier {
    pub(crate) const fn is_luma_part(&self) -> bool {
        matches!(self.part, DecodeBlockPart::LumaPart)
    }

    pub(crate) const fn is_chroma_part(&self) -> bool {
        matches!(self.part, DecodeBlockPart::ChromaPart { .. })
    }

    pub(crate) const fn is_mixed_region(&self) -> bool {
        !self.intra_region
    }

    pub(crate) const fn shared_mixed_chroma_ref_forces_inter(&self) -> bool {
        !self.is_luma_part()
            && !self.is_chroma_part()
            && self.is_mixed_region()
            && self.b_size.index() != self.chroma_ref.size.index()
    }

    pub(crate) const fn part(&self) -> DecodeBlockPart {
        self.part
    }

    pub(super) const fn tree_type(&self) -> PartitionTreeType {
        match self.part {
            DecodeBlockPart::Shared => PartitionTreeType::Shared,
            DecodeBlockPart::LumaPart => PartitionTreeType::LumaPart,
            DecodeBlockPart::ChromaPart { .. } => PartitionTreeType::ChromaPart,
        }
    }

    pub(crate) const fn cfl_allowed_in_sdp(&self) -> bool {
        self.cfl_allowed_in_sdp
    }

    pub(crate) const fn chroma_ref_geometry(&self) -> ChromaRefGeometry {
        self.chroma_ref
    }

    /// A shared-tree leaf at `(r, c)` that carries its own chroma.
    #[cfg(test)]
    pub(crate) const fn shared_leaf(r: usize, c: usize, b_size: BlockSize) -> Self {
        Self {
            r,
            c,
            b_size,
            has_chroma: true,
            chroma_offset: false,
            chroma_ref: ChromaRefGeometry::new(r, c, b_size),
            part: DecodeBlockPart::Shared,
            intra_region: false,
            cfl_allowed_in_sdp: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GeneralIntraLeafMode {
    luma: Option<IntraFacts>,
    palette_y: Option<LumaPalette>,
    /// The `UVCfls` / `UVSmooth` bits of a leaf with chroma.
    uv: u8,
    intrabc: bool,
}

impl GeneralIntraLeafMode {
    #[must_use]
    pub(crate) fn luma(
        intra_joint_mode: IntraJointMode,
        y_mode: IntraYMode,
        angle_delta_y: i8,
        fsc_mode: u8,
        mrl: MrlSelection,
    ) -> Self {
        Self {
            luma: Some(IntraFacts::luma(
                intra_joint_mode,
                y_mode,
                angle_delta_y,
                fsc_mode,
                mrl,
            )),
            palette_y: None,
            uv: 0,
            intrabc: false,
        }
    }

    #[must_use]
    pub(crate) const fn no_luma_mode() -> Self {
        Self {
            luma: None,
            palette_y: None,
            uv: 0,
            intrabc: false,
        }
    }

    #[must_use]
    pub(crate) const fn mark_intrabc(mut self) -> Self {
        self.intrabc = true;
        self
    }

    #[must_use]
    pub(crate) const fn is_intrabc(&self) -> bool {
        self.intrabc
    }

    #[must_use]
    pub(crate) const fn chroma(uv: u8) -> Self {
        Self::no_luma_mode().with_uv(uv)
    }

    #[must_use]
    pub(crate) const fn with_palette_y(mut self, palette_y: Option<LumaPalette>) -> Self {
        self.palette_y = palette_y;
        self
    }

    #[must_use]
    pub(crate) const fn with_use_dip(mut self, use_dip: u8) -> Self {
        if let Some(luma) = self.luma {
            self.luma = Some(luma.with_dip(use_dip != 0));
        }
        self
    }

    #[must_use]
    pub(crate) const fn with_uv(mut self, uv: u8) -> Self {
        self.uv = uv;
        self
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum TilePartitionTraversalError {
    #[error("partition traversal allocation failed: {0}")]
    Allocation(#[from] std::collections::TryReserveError),
    #[error("partition traversal rejected by resource limit: {0}")]
    Limit(#[from] DecodeLimitError),
    #[error("partition traversal block-decoded state failed: {0}")]
    BlockDecoded(#[from] super::block_decoded_state::TileBlockDecodedStateError),
    #[error("partition traversal intra mode facts state failed: {0}")]
    IntraFactsState(#[from] TileIntraFactsStateError),
    #[error("partition traversal size lookup failed: {0}")]
    Size(#[from] PartitionSizeError),
    #[error("partition traversal allowed-set derivation failed: {0}")]
    Allowed(#[from] PartitionAllowedError),
    #[error("partition traversal decision failed: {0}")]
    Decision(#[from] PartitionDecisionError),
    #[error("partition traversal symbol initialization failed: {0}")]
    Symbol(#[from] splot_core::Error),
    #[error("loop-restoration syntax symbol read failed: {0}")]
    LoopRestorationSymbol(#[source] splot_core::Error),
    #[error("partition traversal CDF context failed: {0}")]
    Cdf(#[from] TileCdfError),
    #[error("partition traversal unsupported path: {0:?}")]
    Unsupported(TilePartitionTraversalUnsupported),
    #[error("{coordinate} coordinate underflow: {base} - {offset}")]
    CoordinateUnderflow {
        coordinate: &'static str,
        base: usize,
        offset: usize,
    },
    #[error("{coordinate} coordinate overflow: {base} + {offset}")]
    CoordinateOverflow {
        coordinate: &'static str,
        base: usize,
        offset: usize,
    },
    #[error("{coordinate} coordinate offset overflow: {left} * {right}")]
    CoordinateOffsetOverflow {
        coordinate: &'static str,
        left: usize,
        right: usize,
    },
    #[error("loop restoration plane {plane} has invalid unit size {unit_size}")]
    InvalidLoopRestorationUnitSize { plane: usize, unit_size: usize },
    #[error("partition traversal selected invalid child size for {partition:?} at bSize {b_size}")]
    InvalidPartitionSubsize {
        partition: PartitionType,
        b_size: usize,
    },
    #[error("partition traversal decoded invalid extended SDP region type {value}")]
    InvalidRegionType { value: u8 },
    #[error("partition traversal produced more than four child calls")]
    TooManyChildCalls,
    #[error("partition traversal missing intra luma mode state at ({r}, {c})")]
    MissingIntraLumaModeState { r: usize, c: usize },
    #[error("partition traversal missing collocated SDP luma mode state at ({r}, {c})")]
    MissingSdpLumaModeState { r: usize, c: usize },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TilePartitionTraversalUnsupported {
    ReadLoopRestoration,
}

fn ensure_supported_traversal_frame(
    frame: TilePartitionFrameFacts,
) -> Result<(), TilePartitionTraversalError> {
    if frame.loop_restoration == TilePartitionLoopRestorationState::UnsupportedReadLrSyntax {
        return Err(TilePartitionTraversalError::Unsupported(
            TilePartitionTraversalUnsupported::ReadLoopRestoration,
        ));
    }
    if let TilePartitionLoopRestorationState::Frame(lr) = frame.loop_restoration {
        for plane in 0..frame.num_planes.min(3) {
            if lr.plane_tool[plane] == TilePartitionLoopRestorationPlaneTool::Switchable
                && plane != 0
            {
                return Err(TilePartitionTraversalError::Unsupported(
                    TilePartitionTraversalUnsupported::ReadLoopRestoration,
                ));
            }
        }
    }
    Ok(())
}

fn symbol_decoder_for_work_unit<'payload>(
    work_unit: &DecodeTileWorkUnit,
    tile_bytes: &'payload [u8],
) -> Result<SymbolDecoder<'payload>, TilePartitionTraversalError> {
    let config = SymbolDecoderConfig::new()
        .with_cdf_update_mode(work_unit.cdf().update_mode())
        .with_cdf_validation_mode(CdfValidationMode::Trusted);
    SymbolDecoder::with_base_and_config(tile_bytes, work_unit.tile_byte_span().start, config)
        .map_err(TilePartitionTraversalError::from)
}

/// § 5.20.4 seeds `decode_partition`'s propagated `hasChroma` with 1. The
/// `NumPlanes > 1` term belongs to `HasChroma` alone, so a monochrome sequence
/// still derives `ChromaMiRow` / `ChromaMiCol` / `ChromaMiSize`.
pub(super) const ROOT_HAS_CHROMA: bool = true;

pub(crate) const fn call_in_frame(
    frame: &TilePartitionFrameFacts,
    call: TilePartitionCall,
) -> bool {
    call.r() < frame.mi_rows && call.c() < frame.mi_cols
}

fn checked_mul_shifted(
    coordinate: &'static str,
    value: usize,
    scale: usize,
    shift: usize,
) -> Result<usize, TilePartitionTraversalError> {
    Ok(checked_mul(coordinate, value, scale)? >> shift)
}

fn plane_range_for_tree_type(tree_type: PartitionTreeType, num_planes: usize) -> (usize, usize) {
    match tree_type {
        PartitionTreeType::Shared => (0, num_planes.min(3)),
        PartitionTreeType::LumaPart => (0, num_planes.min(1)),
        PartitionTreeType::ChromaPart => (1, num_planes.min(3)),
    }
}

fn plane_subsampling(frame: TilePartitionFrameFacts, plane: usize) -> (usize, usize) {
    if plane == 0 {
        (0, 0)
    } else {
        (
            usize::from(frame.subsampling_x),
            usize::from(frame.subsampling_y),
        )
    }
}

/// Adds tile coordinates. These hot helpers use `let ... else` rather than
/// `ok_or`, whose eager error value runs its drop glue even on success.
pub(crate) fn checked_add(
    coordinate: &'static str,
    base: usize,
    offset: usize,
) -> Result<usize, TilePartitionTraversalError> {
    let Some(sum) = base.checked_add(offset) else {
        return Err(TilePartitionTraversalError::CoordinateOverflow {
            coordinate,
            base,
            offset,
        });
    };
    Ok(sum)
}

fn checked_sub(
    coordinate: &'static str,
    base: usize,
    offset: usize,
) -> Result<usize, TilePartitionTraversalError> {
    let Some(difference) = base.checked_sub(offset) else {
        return Err(TilePartitionTraversalError::CoordinateUnderflow {
            coordinate,
            base,
            offset,
        });
    };
    Ok(difference)
}

fn checked_mul(
    coordinate: &'static str,
    left: usize,
    right: usize,
) -> Result<usize, TilePartitionTraversalError> {
    let Some(product) = left.checked_mul(right) else {
        return Err(TilePartitionTraversalError::CoordinateOffsetOverflow {
            coordinate,
            left,
            right,
        });
    };
    Ok(product)
}

fn checked_shl(
    coordinate: &'static str,
    value: usize,
    shift: usize,
) -> Result<usize, TilePartitionTraversalError> {
    let shift =
        u32::try_from(shift).map_err(|_| TilePartitionTraversalError::CoordinateOverflow {
            coordinate,
            base: value,
            offset: shift,
        })?;
    let Some(shifted) = value.checked_shl(shift) else {
        return Err(TilePartitionTraversalError::CoordinateOverflow {
            coordinate,
            base: value,
            offset: shift as usize,
        });
    };
    Ok(shifted)
}

#[cfg(test)]
pub(crate) fn checked_scaled_add(
    coordinate: &'static str,
    base: usize,
    scale: usize,
    value: usize,
) -> Result<usize, TilePartitionTraversalError> {
    checked_add(coordinate, base, checked_mul(coordinate, scale, value)?)
}

#[cfg(test)]
#[path = "partition_traversal_tests.rs"]
pub(crate) mod tests;

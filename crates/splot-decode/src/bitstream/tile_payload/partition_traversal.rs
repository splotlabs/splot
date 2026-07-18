// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 5.20.3.1 partition traversal.

use splot_core::symbol::{
    CdfValidationMode, SymbolDecoder, SymbolDecoderCheckpoint, SymbolDecoderConfig,
};

use super::DecodeTileWorkUnit;
use super::cdf::block_context::IntraYMode;
use super::cdf::context::{PartitionContextInput, SquareSplitContextInput};
use super::cdf::{self, TileCdfError};
use super::intra_joint_modes::{
    IsCflContext, LumaPalette, TileFscModeState, TileFscModeStateError, TileIntraJointModeState,
    TileIntraYModeFacts, TileIntraYModeState, TileIntraYModeStateError, TileLumaPaletteState,
    TileLumaPaletteStateError, TileUseDipState, TileUseDipStateError, TileUsesMrlsState,
    TileUsesMrlsStateError, TileUvCflState,
};
use super::mi_size_state::{TileMiSizeState, TileMiSizeStateError};
use super::partition::{self, PartitionDecisionError, PartitionType, ReadPartitionDecision};
use super::partition_allowed::{
    PartitionAllowedError, PartitionAllowedInput, PartitionFeatureFlags, PartitionTreeType,
    partition_decision_facts,
};
use super::partition_size::{
    BlockSize, PartitionSizeError, PartitionSubsize, h_partition_midsize, partition_subsize,
};
use crate::{DecodeLimitError, DecodeLimitName, DecodeLimits};

mod lr_records;
mod lr_syntax;
mod partition_children;
mod state_publication;
mod tree_walk;

pub(crate) use lr_records::LrUnitRestorationType;
#[cfg(test)]
use lr_records::WienerNsLrUnitSelection;
pub(crate) use lr_records::{
    TileLoopRestorationRootFrontier, WienerNsLrSourceBlock, WienerNsLrUnitFilter,
};
pub(crate) use lr_syntax::consume_tile_loop_restoration_root_frontier;
#[cfg(test)]
use lr_syntax::{WienerNsUnitFilterState, read_wiener_ns_unit_filter};
use partition_children::child_calls;
pub(crate) use state_publication::DecodedLeafPublication;
pub(crate) use tree_walk::{
    GeneralIntraPartitionTreeCursor, GeneralIntraPartitionTreeOutput, GeneralIntraTreeWalkError,
    plan_tile_partition_traversal_cursor,
};
#[cfg(test)]
use tree_walk::{
    MIXED_REGION, SdpPartitionState, decode_block_frontier, is_cfl_context_for_chroma_ref,
    is_intra_sdp_shared_root, partition_cdf_plane, plan_tile_partition_traversal_frontier,
    read_extended_sdp_region_type, read_frontier_partition_decision,
    should_read_extended_sdp_region_type,
};
use tree_walk::{extended_sdp_allowed_for_child, valid_subsize};

pub(crate) const BLOCK_8X32: usize = 21;
pub(crate) const BLOCK_32X8: usize = 22;

#[allow(clippy::struct_field_names)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TilePartitionContextState<'a> {
    mi_sizes: &'a [usize],
    mi_size_stride: usize,
    left_mi_sizes: [&'a [usize]; 2],
    above_mi_sizes: [&'a [usize]; 2],
    origin_row: usize,
    origin_col: usize,
}

impl<'a> TilePartitionContextState<'a> {
    #[must_use]
    pub(crate) const fn new(
        mi_sizes: &'a [usize],
        mi_size_stride: usize,
        left_mi_sizes: [&'a [usize]; 2],
        above_mi_sizes: [&'a [usize]; 2],
    ) -> Self {
        Self::new_at(
            mi_sizes,
            mi_size_stride,
            left_mi_sizes,
            above_mi_sizes,
            0,
            0,
        )
    }

    #[must_use]
    pub(crate) const fn new_at(
        mi_sizes: &'a [usize],
        mi_size_stride: usize,
        left_mi_sizes: [&'a [usize]; 2],
        above_mi_sizes: [&'a [usize]; 2],
        origin_row: usize,
        origin_col: usize,
    ) -> Self {
        Self {
            mi_sizes,
            mi_size_stride,
            left_mi_sizes,
            above_mi_sizes,
            origin_row,
            origin_col,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TilePartitionBruState {
    Active,
    Unsupported,
}

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
    has_chroma: bool,
    bru_state: TilePartitionBruState,
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
        has_chroma: bool,
        bru_state: TilePartitionBruState,
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
            has_chroma,
            bru_state,
        })
    }

    #[must_use]
    pub(crate) const fn sb_size(&self) -> BlockSize {
        self.sb_size
    }
}

#[derive(Debug)]
pub(crate) struct TilePartitionTraversalInput<'work, 'payload, 'ctx> {
    work_unit: &'work mut DecodeTileWorkUnit<'payload>,
    frame: TilePartitionFrameFacts,
    context: TilePartitionContextState<'ctx>,
    limits: DecodeLimits,
}

impl<'work, 'payload, 'ctx> TilePartitionTraversalInput<'work, 'payload, 'ctx> {
    #[must_use]
    pub(crate) const fn new(
        work_unit: &'work mut DecodeTileWorkUnit<'payload>,
        frame: TilePartitionFrameFacts,
        context: TilePartitionContextState<'ctx>,
        limits: DecodeLimits,
    ) -> Self {
        Self {
            work_unit,
            frame,
            context,
            limits,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ChromaRefGeometry {
    row: usize,
    col: usize,
    size: BlockSize,
}

impl ChromaRefGeometry {
    pub(crate) const fn new(row: usize, col: usize, size: BlockSize) -> Self {
        Self { row, col, size }
    }

    #[allow(dead_code)]
    pub(crate) const fn row(self) -> usize {
        self.row
    }

    #[allow(dead_code)]
    pub(crate) const fn col(self) -> usize {
        self.col
    }

    #[allow(dead_code)]
    pub(crate) const fn size(self) -> BlockSize {
        self.size
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TilePartitionCall {
    pub(crate) r: usize,
    pub(crate) c: usize,
    pub(crate) b_size: BlockSize,
    pub(crate) parent_size: Option<BlockSize>,
    pub(crate) chroma_offset: bool,
    pub(crate) has_chroma: bool,
    tree_type: PartitionTreeType,
    cfl_allowed_in_sdp: bool,
    extended_sdp_allowed: bool,
    intra_region: bool,
    chroma_ref: Option<ChromaRefGeometry>,
}

impl TilePartitionCall {
    const fn root(r: usize, c: usize, b_size: BlockSize, has_chroma: bool) -> Self {
        Self {
            r,
            c,
            b_size,
            parent_size: None,
            chroma_offset: false,
            has_chroma,
            tree_type: PartitionTreeType::Shared,
            cfl_allowed_in_sdp: true,
            extended_sdp_allowed: true,
            intra_region: false,
            chroma_ref: None,
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
        Self {
            r,
            c,
            b_size,
            parent_size,
            chroma_offset,
            has_chroma,
            tree_type,
            cfl_allowed_in_sdp: true,
            extended_sdp_allowed,
            intra_region,
            chroma_ref,
        }
    }

    pub(crate) const fn tree_type(self) -> PartitionTreeType {
        self.tree_type
    }

    pub(crate) const fn cfl_allowed_in_sdp(self) -> bool {
        self.cfl_allowed_in_sdp
    }

    pub(crate) const fn set_cfl_allowed_in_sdp(&mut self, value: bool) {
        self.cfl_allowed_in_sdp = value;
    }

    pub(crate) fn chroma_ref_geometry(self) -> ChromaRefGeometry {
        if !self.chroma_offset && self.has_chroma {
            return ChromaRefGeometry {
                row: self.r,
                col: self.c,
                size: self.b_size,
            };
        }
        self.chroma_ref.unwrap_or(ChromaRefGeometry {
            row: self.r,
            col: self.c,
            size: self.b_size,
        })
    }

    const fn with_tree_type(self, tree_type: PartitionTreeType) -> Self {
        Self { tree_type, ..self }
    }

    const fn with_cfl_allowed_in_sdp(self, cfl_allowed_in_sdp: bool) -> Self {
        Self {
            cfl_allowed_in_sdp,
            ..self
        }
    }

    #[allow(dead_code)]
    const fn with_extended_sdp_allowed(self, extended_sdp_allowed: bool) -> Self {
        Self {
            extended_sdp_allowed,
            ..self
        }
    }

    const fn with_intra_region(self, intra_region: bool) -> Self {
        Self {
            intra_region,
            ..self
        }
    }
}

#[allow(clippy::struct_field_names)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TilePartitionBounds {
    mi_row_start: usize,
    mi_row_end: usize,
    mi_col_start: usize,
    mi_col_end: usize,
}

impl TilePartitionBounds {
    fn from_work_unit(work_unit: &DecodeTileWorkUnit<'_>) -> Self {
        let row_range = work_unit.mi_row_range();
        let col_range = work_unit.mi_col_range();
        Self {
            mi_row_start: row_range.start as usize,
            mi_row_end: row_range.end as usize,
            mi_col_start: col_range.start as usize,
            mi_col_end: col_range.end as usize,
        }
    }

    const fn is_inside(self, r: usize, c: usize) -> bool {
        self.mi_col_start <= c
            && c < self.mi_col_end
            && self.mi_row_start <= r
            && r < self.mi_row_end
    }

    fn avail_u_at(self, r: usize, c: usize) -> bool {
        r.checked_sub(1)
            .is_some_and(|candidate_r| self.is_inside(candidate_r, c))
    }

    fn avail_l_at(self, r: usize, c: usize) -> bool {
        c.checked_sub(1)
            .is_some_and(|candidate_c| self.is_inside(r, candidate_c))
    }

    fn avail_u(self, call: TilePartitionCall) -> bool {
        self.avail_u_at(call.r, call.c)
    }

    fn avail_l(self, call: TilePartitionCall) -> bool {
        self.avail_l_at(call.r, call.c)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TilePartitionFrontierStep {
    pub(crate) call: TilePartitionCall,
    pub(crate) decision: ReadPartitionDecision,
    pub(crate) symbol_count_before: u64,
    pub(crate) symbol_count_after: u64,
    using_extended_sdp: bool,
}

impl TilePartitionFrontierStep {
    const fn partition(self) -> PartitionType {
        self.decision.partition
    }

    const fn using_extended_sdp(self) -> bool {
        self.using_extended_sdp
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
    tree_type: PartitionTreeType,
    intra_region: bool,
    stored_luma_y_mode: Option<TileIntraYModeFacts>,
    cfl_allowed_in_sdp: bool,
    pub(crate) symbol_count_before_block: u64,
    pub(crate) symbol_checkpoint_before_block: SymbolDecoderCheckpoint,
}

impl DecodeBlockFrontier {
    pub(crate) const fn is_luma_part(&self) -> bool {
        matches!(self.tree_type, PartitionTreeType::LumaPart)
    }

    pub(crate) const fn is_chroma_part(&self) -> bool {
        matches!(self.tree_type, PartitionTreeType::ChromaPart)
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

    pub(crate) const fn stored_luma_y_mode(&self) -> Option<IntraYMode> {
        match self.stored_luma_y_mode {
            Some(facts) => Some(facts.y_mode),
            None => None,
        }
    }

    pub(crate) const fn stored_luma_angle_delta_y(&self) -> Option<i8> {
        match self.stored_luma_y_mode {
            Some(facts) => Some(facts.angle_delta_y),
            None => None,
        }
    }

    pub(crate) const fn cfl_allowed_in_sdp(&self) -> bool {
        self.cfl_allowed_in_sdp
    }

    #[allow(dead_code)]
    pub(crate) const fn chroma_ref_geometry(&self) -> ChromaRefGeometry {
        self.chroma_ref
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TilePartitionTraversalPlan {
    pub(crate) tile_num: u32,
    steps: Vec<TilePartitionFrontierStep>,
    skipped_out_of_frame: Vec<TilePartitionCall>,
    pending_children: Vec<TilePartitionCall>,
    frontier: DecodeBlockFrontier,
    pub(crate) consumed_bits_before: u64,
    pub(crate) consumed_bits_after: u64,
    symbol_count_after: u64,
}

impl TilePartitionTraversalPlan {
    #[must_use]
    pub(crate) fn steps(&self) -> &[TilePartitionFrontierStep] {
        &self.steps
    }

    #[must_use]
    pub(crate) fn pending_children(&self) -> &[TilePartitionCall] {
        &self.pending_children
    }

    #[must_use]
    pub(crate) const fn frontier(&self) -> DecodeBlockFrontier {
        self.frontier
    }

    #[must_use]
    pub(crate) const fn symbol_count_after(&self) -> u64 {
        self.symbol_count_after
    }
}

pub(crate) struct TilePartitionTraversalCursor<'payload> {
    plan: TilePartitionTraversalPlan,
    symbols: SymbolDecoder<'payload>,
}

impl<'payload> TilePartitionTraversalCursor<'payload> {
    #[must_use]
    pub(crate) fn into_parts(self) -> (TilePartitionTraversalPlan, SymbolDecoder<'payload>) {
        (self.plan, self.symbols)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GeneralIntraLeafMode {
    intra_joint_mode: Option<u8>,
    y_mode: Option<IntraYMode>,
    angle_delta_y: Option<i8>,
    fsc_mode: Option<u8>,
    uses_mrls: Option<u8>,
    use_dip: Option<u8>,
    palette_y: Option<LumaPalette>,
    uv_cfl: Option<bool>,
    intrabc: bool,
}

impl GeneralIntraLeafMode {
    pub(crate) fn y_mode_is_smooth(&self) -> bool {
        self.y_mode.is_some_and(IntraYMode::is_smooth)
    }

    #[must_use]
    pub(crate) const fn luma(
        intra_joint_mode: u8,
        y_mode: IntraYMode,
        angle_delta_y: i8,
        fsc_mode: u8,
        uses_mrls: u8,
    ) -> Self {
        Self {
            intra_joint_mode: Some(intra_joint_mode),
            y_mode: Some(y_mode),
            angle_delta_y: Some(angle_delta_y),
            fsc_mode: Some(fsc_mode),
            uses_mrls: Some(uses_mrls),
            use_dip: Some(0),
            palette_y: None,
            uv_cfl: None,
            intrabc: false,
        }
    }

    #[must_use]
    pub(crate) const fn no_luma_mode() -> Self {
        Self {
            intra_joint_mode: None,
            y_mode: None,
            angle_delta_y: None,
            fsc_mode: None,
            uses_mrls: None,
            use_dip: None,
            palette_y: None,
            uv_cfl: None,
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
    pub(crate) const fn chroma(uv_cfl: bool) -> Self {
        Self {
            intra_joint_mode: None,
            y_mode: None,
            angle_delta_y: None,
            fsc_mode: None,
            uses_mrls: None,
            use_dip: None,
            palette_y: None,
            uv_cfl: Some(uv_cfl),
            intrabc: false,
        }
    }

    #[must_use]
    pub(crate) const fn with_palette_y(mut self, palette_y: Option<LumaPalette>) -> Self {
        self.palette_y = palette_y;
        self
    }

    #[must_use]
    pub(crate) const fn with_use_dip(mut self, use_dip: u8) -> Self {
        self.use_dip = Some(use_dip);
        self
    }

    #[must_use]
    pub(crate) const fn with_uv_cfl(mut self, uv_cfl: bool) -> Self {
        self.uv_cfl = Some(uv_cfl);
        self
    }

    #[allow(dead_code)]
    #[must_use]
    pub(crate) const fn luma_y_mode(self) -> Option<IntraYMode> {
        self.y_mode
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum TilePartitionTraversalError {
    #[error("partition traversal rejected by resource limit: {0}")]
    Limit(#[from] DecodeLimitError),
    #[error("partition traversal block-decoded state failed: {0}")]
    BlockDecoded(#[from] super::block_decoded_state::TileBlockDecodedStateError),
    #[error("partition traversal intra YMode state failed: {0}")]
    IntraYModeState(#[from] TileIntraYModeStateError),
    #[error("partition traversal intra UsesMrls state failed: {0}")]
    UsesMrlsState(#[from] TileUsesMrlsStateError),
    #[error("partition traversal intra UseDip state failed: {0}")]
    UseDipState(#[from] TileUseDipStateError),
    #[error("partition traversal intra FscModes state failed: {0}")]
    FscModeState(#[from] TileFscModeStateError),
    #[error("partition traversal luma palette state failed: {0}")]
    LumaPaletteState(#[from] TileLumaPaletteStateError),
    #[error("partition traversal size lookup failed: {0}")]
    Size(#[from] PartitionSizeError),
    #[error("partition traversal allowed-set derivation failed: {0}")]
    Allowed(#[from] PartitionAllowedError),
    #[error("partition traversal decision failed: {0}")]
    Decision(#[from] PartitionDecisionError),
    #[error("partition traversal symbol initialization failed: {0}")]
    Symbol(#[from] splot_core::Error),
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
    #[error("partition traversal reached no in-frame decode_block frontier")]
    NoBlockFrontier,
    #[error("partition traversal missing intra luma mode state at ({r}, {c})")]
    MissingIntraLumaModeState { r: usize, c: usize },
    #[error("partition traversal missing intra UsesMrls state at ({r}, {c})")]
    MissingIntraUsesMrlsState { r: usize, c: usize },
    #[error("partition traversal missing intra FscModes state at ({r}, {c})")]
    MissingIntraFscModeState { r: usize, c: usize },
    #[error("partition traversal missing intra UseDip state at ({r}, {c})")]
    MissingIntraUseDipState { r: usize, c: usize },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TilePartitionTraversalUnsupported {
    ExtendedSdp,
    ReadLoopRestoration,
    BruOrBridge,
}

fn ensure_supported_traversal_frame(
    frame: TilePartitionFrameFacts,
    reject_extended_sdp: bool,
) -> Result<(), TilePartitionTraversalError> {
    if reject_extended_sdp && frame.enable_extended_sdp && !frame.frame_is_intra {
        return Err(TilePartitionTraversalError::Unsupported(
            TilePartitionTraversalUnsupported::ExtendedSdp,
        ));
    }
    if frame.loop_restoration == TilePartitionLoopRestorationState::UnsupportedReadLrSyntax {
        return Err(TilePartitionTraversalError::Unsupported(
            TilePartitionTraversalUnsupported::ReadLoopRestoration,
        ));
    }
    if let TilePartitionLoopRestorationState::Frame(lr) = frame.loop_restoration {
        for plane in 0..frame.num_planes.min(3) {
            if lr.plane_tool[plane] == TilePartitionLoopRestorationPlaneTool::Switchable
                && (plane != 0 || !lr.frame_filters_on[plane])
            {
                return Err(TilePartitionTraversalError::Unsupported(
                    TilePartitionTraversalUnsupported::ReadLoopRestoration,
                ));
            }
        }
    }
    if frame.bru_state != TilePartitionBruState::Active {
        return Err(TilePartitionTraversalError::Unsupported(
            TilePartitionTraversalUnsupported::BruOrBridge,
        ));
    }
    Ok(())
}

fn symbol_decoder_for_work_unit<'payload>(
    work_unit: &DecodeTileWorkUnit<'payload>,
) -> Result<SymbolDecoder<'payload>, TilePartitionTraversalError> {
    let config = SymbolDecoderConfig::new()
        .with_cdf_update_mode(work_unit.cdf().update_mode())
        .with_cdf_validation_mode(CdfValidationMode::Trusted);
    SymbolDecoder::with_base_and_config(
        work_unit.tile_bytes(),
        work_unit.tile_byte_span().start,
        config,
    )
    .map_err(TilePartitionTraversalError::from)
}

fn root_partition_call(
    work_unit: &DecodeTileWorkUnit<'_>,
    frame: TilePartitionFrameFacts,
) -> TilePartitionCall {
    TilePartitionCall::root(
        work_unit.mi_row_range().start as usize,
        work_unit.mi_col_range().start as usize,
        frame.sb_size,
        frame.has_chroma,
    )
}

const fn call_in_frame(frame: TilePartitionFrameFacts, call: TilePartitionCall) -> bool {
    call.r < frame.mi_rows && call.c < frame.mi_cols
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

pub(crate) fn checked_add(
    coordinate: &'static str,
    base: usize,
    offset: usize,
) -> Result<usize, TilePartitionTraversalError> {
    base.checked_add(offset)
        .ok_or(TilePartitionTraversalError::CoordinateOverflow {
            coordinate,
            base,
            offset,
        })
}

fn checked_sub(
    coordinate: &'static str,
    base: usize,
    offset: usize,
) -> Result<usize, TilePartitionTraversalError> {
    base.checked_sub(offset)
        .ok_or(TilePartitionTraversalError::CoordinateUnderflow {
            coordinate,
            base,
            offset,
        })
}

fn checked_mul(
    coordinate: &'static str,
    left: usize,
    right: usize,
) -> Result<usize, TilePartitionTraversalError> {
    left.checked_mul(right)
        .ok_or(TilePartitionTraversalError::CoordinateOffsetOverflow {
            coordinate,
            left,
            right,
        })
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
    value
        .checked_shl(shift)
        .ok_or(TilePartitionTraversalError::CoordinateOverflow {
            coordinate,
            base: value,
            offset: shift as usize,
        })
}

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

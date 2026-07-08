// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 5.20.3.1 partition traversal.

use splot_core::symbol::{SymbolDecoder, SymbolDecoderCheckpoint, SymbolDecoderConfig};

use super::DecodeTileWorkUnit;
use super::block_decoded_state::TileBlockDecodedState;
use super::cdf::TileCdfError;
use super::cdf::block_context::IntraYMode;
use super::cdf::context::{PartitionContextInput, SquareSplitContextInput};
use super::intra_joint_modes::{
    IsCflContext, LumaPalette, TileFscModeState, TileFscModeStateError, TileIntraJointModeState,
    TileIntraYModeFacts, TileIntraYModeState, TileIntraYModeStateError, TileLumaPaletteState,
    TileLumaPaletteStateError, TileUseDipState, TileUseDipStateError, TileUsesMrlsState,
    TileUsesMrlsStateError, TileUvCflState,
};
use super::mi_size_state::{TileMiSizeState, TileMiSizeStateError};
use super::partition::{PartitionDecisionError, PartitionType, ReadPartitionDecision};
use super::partition_allowed::{
    PartitionAllowedError, PartitionAllowedInput, PartitionFeatureFlags, PartitionTreeType,
    partition_decision_facts,
};
use super::partition_size::{
    BlockSize, PartitionSizeError, PartitionSubsize, h_partition_midsize, partition_subsize,
};
use crate::{DecodeLimitError, DecodeLimitName, DecodeLimits};

mod partition_children;

use partition_children::child_calls;

pub(crate) const BLOCK_8X32: usize = 21;
pub(crate) const BLOCK_32X8: usize = 22;
const BLOCK_64X64: usize = 12;
const MI_SIZE: usize = 4;
const LR_BANK_SIZE: usize = 4;
const WIENER_NS_LUMA_COEFFS: usize = 16;
const WIENER_NS_CHROMA_COEFFS: usize = 18;
const WIENER_NS_SHORT_COEFFS: usize = 6;
const WIENER_NS_LUMA_SUBSETS: usize = 4;
const WIENER_NS_CHROMA_SUBSETS: usize = 3;
const INTER_SDP_MAX_BLOCK_SIZE: usize = 64;
const INTRA_REGION: u8 = 0;
const MIXED_REGION: u8 = 1;
const WIENER_NS_TAPS_K: [[u8; WIENER_NS_CHROMA_COEFFS]; 2] = [
    [6, 6, 5, 5, 5, 5, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4],
    [6, 6, 5, 5, 5, 5, 5, 5, 5, 5, 4, 4, 4, 4, 4, 4, 4, 4],
];
const WIENER_NS_TAPS_MIN: [[i16; WIENER_NS_CHROMA_COEFFS]; 2] = [
    [
        -24, -24, -14, -14, -16, -16, -8, -8, -8, -8, -8, -8, -8, -8, -8, -8, -8, -8,
    ],
    [
        -24, -24, -14, -14, -16, -16, -16, -16, -16, -16, -8, -8, -8, -8, -8, -8, -8, -8,
    ],
];
const WIENER_NS_TAPS_PRESENT: [[[bool; WIENER_NS_CHROMA_COEFFS]; WIENER_NS_LUMA_SUBSETS]; 2] = [
    [
        [
            true, true, true, true, true, true, false, false, false, false, false, false, false,
            false, false, false, false, false,
        ],
        [
            true, true, false, false, false, false, true, true, true, true, true, true, false,
            false, false, false, false, false,
        ],
        [
            true, true, true, true, true, true, true, true, true, true, true, true, false, false,
            false, false, false, false,
        ],
        [
            true, true, true, true, true, true, true, true, true, true, true, true, true, true,
            true, true, false, false,
        ],
    ],
    [
        [
            true, true, true, true, true, true, false, false, false, false, false, false, false,
            false, false, false, false, false,
        ],
        [
            true, true, true, true, true, true, true, true, true, true, false, false, false, false,
            false, false, false, false,
        ],
        [
            true, true, true, true, true, true, true, true, true, true, true, true, true, true,
            true, true, true, true,
        ],
        [false; WIENER_NS_CHROMA_COEFFS],
    ],
];

#[allow(clippy::struct_field_names)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TilePartitionContextState<'a> {
    mi_sizes: [&'a [&'a [usize]]; 2],
    left_mi_sizes: [&'a [usize]; 2],
    above_mi_sizes: [&'a [usize]; 2],
}

impl<'a> TilePartitionContextState<'a> {
    #[must_use]
    pub(crate) const fn new(
        mi_sizes: [&'a [&'a [usize]]; 2],
        left_mi_sizes: [&'a [usize]; 2],
        above_mi_sizes: [&'a [usize]; 2],
    ) -> Self {
        Self {
            mi_sizes,
            left_mi_sizes,
            above_mi_sizes,
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
    FrameWienerNs(TilePartitionWienerNsLoopRestorationState),
    UnsupportedReadLrSyntax,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TilePartitionWienerNsLoopRestorationState {
    plane_enabled: [bool; 3],
    frame_filters_on: [bool; 3],
    unit_size: [usize; 3],
}

impl TilePartitionWienerNsLoopRestorationState {
    #[must_use]
    pub(crate) const fn new(
        plane_enabled: [bool; 3],
        frame_filters_on: [bool; 3],
        unit_size: [usize; 3],
    ) -> Self {
        Self {
            plane_enabled,
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

fn is_cfl_context_for_chroma_ref(
    uv_cfls: &TileUvCflState,
    tile_bounds: TilePartitionBounds,
    chroma_ref: ChromaRefGeometry,
) -> IsCflContext {
    let row = chroma_ref.row();
    let col = chroma_ref.col();
    IsCflContext::new(uv_cfls.is_cfl_ctx(
        row,
        col,
        tile_bounds.avail_u_at(row, col),
        tile_bounds.avail_l_at(row, col),
    ))
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TileLoopRestorationRootFrontier {
    symbol_count_after: u64,
    consumed_bits_after: u64,
    lr_units_consumed: usize,
    active_wiener_ns_units: usize,
    selections: Vec<WienerNsLrUnitSelection>,
    active_source_blocks: Vec<WienerNsLrSourceBlock>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WienerNsLrUnitSelection {
    pub(crate) plane: usize,
    pub(crate) unit_row: usize,
    pub(crate) unit_col: usize,
    pub(crate) active: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WienerNsLrSourceBlock {
    pub(crate) plane: usize,
    pub(crate) row: usize,
    pub(crate) col: usize,
    pub(crate) unit_row: usize,
    pub(crate) unit_col: usize,
    pub(crate) tile_mi_row_start: usize,
    pub(crate) tile_mi_row_end: usize,
    pub(crate) tile_mi_col_start: usize,
    pub(crate) tile_mi_col_end: usize,
    pub(crate) x: usize,
    pub(crate) y: usize,
    pub(crate) width: usize,
    pub(crate) height: usize,
    pub(crate) luma_start_x: usize,
    pub(crate) luma_end_x: usize,
    pub(crate) luma_start_y: usize,
    pub(crate) luma_end_y: usize,
    pub(crate) frame_luma_end_y: usize,
    pub(crate) luma_stripe_start_y: usize,
    pub(crate) luma_stripe_end_y: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WienerNsLrUnitFilter {
    pub(crate) plane: usize,
    pub(crate) unit_row: usize,
    pub(crate) unit_col: usize,
    pub(crate) coeff_count: usize,
    pub(crate) coeffs: [i16; WIENER_NS_CHROMA_COEFFS],
}

impl TileLoopRestorationRootFrontier {
    #[must_use]
    pub(crate) const fn symbol_count_after(&self) -> u64 {
        self.symbol_count_after
    }

    #[must_use]
    pub(crate) const fn consumed_bits_after(&self) -> u64 {
        self.consumed_bits_after
    }

    #[must_use]
    pub(crate) const fn lr_units_consumed(&self) -> usize {
        self.lr_units_consumed
    }

    #[must_use]
    pub(crate) const fn active_wiener_ns_units(&self) -> usize {
        self.active_wiener_ns_units
    }

    #[must_use]
    pub(crate) fn selections(&self) -> &[WienerNsLrUnitSelection] {
        &self.selections
    }

    #[must_use]
    pub(crate) fn active_source_blocks(&self) -> &[WienerNsLrSourceBlock] {
        &self.active_source_blocks
    }

    #[must_use]
    pub(crate) const fn all_lr_units_inactive(&self) -> bool {
        self.active_wiener_ns_units == 0
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct WienerNsLrUnitActivity {
    units_consumed: usize,
    active_units: usize,
    selections: Vec<WienerNsLrUnitSelection>,
    active_source_blocks: Vec<WienerNsLrSourceBlock>,
    unit_filters: Vec<WienerNsLrUnitFilter>,
    unit_filter_state: WienerNsUnitFilterState,
    retain_source_blocks: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct WienerNsUnitFilterState {
    bank_size: [usize; 3],
    bank_ptr: [usize; 3],
    bank: [[[i16; WIENER_NS_CHROMA_COEFFS]; LR_BANK_SIZE]; 3],
}

impl Default for WienerNsUnitFilterState {
    fn default() -> Self {
        let mut bank = [[[0i16; WIENER_NS_CHROMA_COEFFS]; LR_BANK_SIZE]; 3];
        for (plane, plane_bank) in bank.iter_mut().enumerate() {
            let plane_index = usize::from(plane > 0);
            for slot in plane_bank {
                for (j, coeff) in slot.iter_mut().enumerate() {
                    *coeff = wiener_ns_initial_tap_value(plane_index, j);
                }
            }
        }
        Self {
            bank_size: [0; 3],
            bank_ptr: [0; 3],
            bank,
        }
    }
}

impl WienerNsLrUnitActivity {
    fn retaining_source_blocks() -> Self {
        Self {
            retain_source_blocks: true,
            ..Self::default()
        }
    }

    fn record(
        &mut self,
        plane: usize,
        unit_row: usize,
        unit_col: usize,
        active: bool,
    ) -> Result<(), TilePartitionTraversalError> {
        self.units_consumed = checked_add("lr_units_consumed", self.units_consumed, 1)?;
        if active {
            self.active_units = checked_add("lr_active_wiener_ns_units", self.active_units, 1)?;
        }
        self.selections.push(WienerNsLrUnitSelection {
            plane,
            unit_row,
            unit_col,
            active,
        });
        Ok(())
    }

    fn record_source_block(
        &mut self,
        block: WienerNsLrSourceBlock,
        limits: DecodeLimits,
    ) -> Result<(), TilePartitionTraversalError> {
        if !self.retain_source_blocks {
            return Ok(());
        }
        let next_len = checked_add(
            "lr_active_source_blocks",
            self.active_source_blocks.len(),
            1,
        )?;
        limits.ensure_allocation_len(DecodeLimitName::MaxLumaSamplesPerFrame, next_len as u64)?;
        self.active_source_blocks.push(block);
        Ok(())
    }

    fn record_unit_filter(
        &mut self,
        filter: WienerNsLrUnitFilter,
        limits: DecodeLimits,
    ) -> Result<(), TilePartitionTraversalError> {
        if !self.retain_source_blocks {
            return Ok(());
        }
        let next_len = checked_add("lr_unit_filters", self.unit_filters.len(), 1)?;
        limits.ensure_allocation_len(DecodeLimitName::MaxLumaSamplesPerFrame, next_len as u64)?;
        self.unit_filters.push(filter);
        Ok(())
    }
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

pub(crate) fn plan_tile_partition_traversal_frontier(
    input: TilePartitionTraversalInput<'_, '_, '_>,
) -> Result<TilePartitionTraversalPlan, TilePartitionTraversalError> {
    Ok(plan_tile_partition_traversal_cursor(input)?.plan)
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
    let config = SymbolDecoderConfig::new().with_cdf_update_mode(work_unit.cdf().update_mode());
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

fn decode_block_frontier(
    call: TilePartitionCall,
    frame: TilePartitionFrameFacts,
    sub_size: BlockSize,
    chroma_offset: bool,
    stored_luma_y_mode: Option<TileIntraYModeFacts>,
    symbols: &SymbolDecoder<'_>,
) -> DecodeBlockFrontier {
    let tree_type = call.tree_type;
    DecodeBlockFrontier {
        r: call.r,
        c: call.c,
        b_size: sub_size,
        has_chroma: call.has_chroma
            && frame.num_planes > 1
            && tree_type != PartitionTreeType::LumaPart,
        chroma_offset,
        chroma_ref: call.chroma_ref_geometry(),
        tree_type,
        intra_region: call.intra_region,
        stored_luma_y_mode,
        cfl_allowed_in_sdp: call.cfl_allowed_in_sdp,
        symbol_count_before_block: symbols.symbol_count(),
        symbol_checkpoint_before_block: symbols.checkpoint(),
    }
}

pub(crate) fn consume_tile_loop_restoration_root_frontier(
    input: TilePartitionTraversalInput<'_, '_, '_>,
) -> Result<TileLoopRestorationRootFrontier, TilePartitionTraversalError> {
    let TilePartitionTraversalInput {
        work_unit,
        frame,
        context: _,
        limits,
    } = input;
    ensure_supported_traversal_frame(frame, true)?;

    let mut cdfs = work_unit.cdf().tile_cdfs().clone();
    let mut lr_activity = WienerNsLrUnitActivity::retaining_source_blocks();
    let mut symbols = symbol_decoder_for_work_unit(work_unit)?;
    let tile_bounds = TilePartitionBounds::from_work_unit(work_unit);
    let root = root_partition_call(work_unit, frame);
    limits.ensure(DecodeLimitName::MaxTilePartitionSteps, 1)?;
    if call_in_frame(frame, root) {
        read_loop_restoration_for_call(
            frame,
            root,
            tile_bounds,
            &mut cdfs,
            &mut symbols,
            &mut lr_activity,
            limits,
        )?;
    }
    *work_unit.cdf_mut().tile_cdfs_mut() = cdfs;
    Ok(TileLoopRestorationRootFrontier {
        symbol_count_after: symbols.symbol_count(),
        consumed_bits_after: symbols.consumed_bits().get(),
        lr_units_consumed: lr_activity.units_consumed,
        active_wiener_ns_units: lr_activity.active_units,
        selections: lr_activity.selections,
        active_source_blocks: lr_activity.active_source_blocks,
    })
}

pub(crate) fn plan_tile_partition_traversal_cursor<'payload>(
    input: TilePartitionTraversalInput<'_, 'payload, '_>,
) -> Result<TilePartitionTraversalCursor<'payload>, TilePartitionTraversalError> {
    let TilePartitionTraversalInput {
        work_unit,
        frame,
        context,
        limits,
    } = input;
    ensure_supported_traversal_frame(frame, false)?;

    let mut cdfs = work_unit.cdf().tile_cdfs().clone();
    let mut symbols = symbol_decoder_for_work_unit(work_unit)?;
    let mut lr_activity = WienerNsLrUnitActivity::default();
    let mut sdp_state = SdpPartitionState::default();
    let consumed_bits_before = symbols.consumed_bits().get();
    let tile_bounds = TilePartitionBounds::from_work_unit(work_unit);
    let root = root_partition_call(work_unit, frame);
    let mut stack = vec![root];
    let mut steps = Vec::new();
    let mut skipped_out_of_frame = Vec::new();

    while let Some(call) = stack.pop() {
        limits.ensure(
            DecodeLimitName::MaxTilePartitionSteps,
            (steps.len() + 1) as u64,
        )?;
        if !call_in_frame(frame, call) {
            skipped_out_of_frame.push(call);
            continue;
        }
        if is_intra_sdp_shared_root(frame, call) {
            stack.push(call.with_tree_type(PartitionTreeType::ChromaPart));
            stack.push(call.with_tree_type(PartitionTreeType::LumaPart));
            continue;
        }
        read_loop_restoration_for_call(
            frame,
            call,
            tile_bounds,
            &mut cdfs,
            &mut symbols,
            &mut lr_activity,
            limits,
        )?;

        let step = read_frontier_partition_step(
            call,
            frame,
            tile_bounds,
            context,
            &mut sdp_state,
            &mut cdfs,
            &mut symbols,
        )?;
        if step.using_extended_sdp() {
            return Err(TilePartitionTraversalError::Unsupported(
                TilePartitionTraversalUnsupported::ExtendedSdp,
            ));
        }

        let call = step.call;
        let partition = step.partition();
        steps.push(step);
        let sub_size = valid_subsize(partition, call.b_size)?;
        let chroma_offset = updated_chroma_offset(call, partition, sub_size, frame)?;
        if partition == PartitionType::None {
            stack.reverse();
            let plan = TilePartitionTraversalPlan {
                tile_num: work_unit.tile_num(),
                steps,
                skipped_out_of_frame,
                pending_children: stack,
                frontier: decode_block_frontier(
                    call,
                    frame,
                    sub_size,
                    chroma_offset,
                    None,
                    &symbols,
                ),
                consumed_bits_before,
                consumed_bits_after: symbols.consumed_bits().get(),
                symbol_count_after: symbols.symbol_count(),
            };
            *work_unit.cdf_mut().tile_cdfs_mut() = cdfs;
            return Ok(TilePartitionTraversalCursor { plan, symbols });
        }

        let children = child_calls(call, partition, sub_size, frame, chroma_offset)?;
        stack.extend(children.as_slice().iter().rev().copied());
    }

    Err(TilePartitionTraversalError::NoBlockFrontier)
}

pub(crate) struct GeneralIntraPartitionTreeOutput<'payload> {
    pub(crate) symbols: SymbolDecoder<'payload>,
    pub(crate) active_source_blocks: Vec<WienerNsLrSourceBlock>,
    pub(crate) unit_filters: Vec<WienerNsLrUnitFilter>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TilePartitionStackEntry {
    Partition(TilePartitionCall),
    ExtendedSdpChromaBlock(TilePartitionCall),
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum GeneralIntraTreeWalkError<E> {
    #[error("partition tree walk traversal failed: {0}")]
    Traversal(#[from] TilePartitionTraversalError),
    #[error("partition tree walk MI-size update failed: {0}")]
    MiSize(TileMiSizeStateError),
    #[error("partition tree walk leaf-block decode failed")]
    Leaf(E),
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn decode_general_intra_partition_tree<'payload, E, F>(
    work_unit: &mut DecodeTileWorkUnit<'payload>,
    frame: TilePartitionFrameFacts,
    mi_size_state: &mut TileMiSizeState,
    joint_modes: &mut TileIntraJointModeState,
    uses_mrls: &mut TileUsesMrlsState,
    use_dip: &mut TileUseDipState,
    fsc_modes: &mut TileFscModeState,
    palette_y: &mut TileLumaPaletteState,
    uv_cfls: &mut TileUvCflState,
    limits: DecodeLimits,
    retain_lr_source_blocks: bool,
    mut on_leaf: F,
) -> Result<GeneralIntraPartitionTreeOutput<'payload>, GeneralIntraTreeWalkError<E>>
where
    F: FnMut(
        &mut DecodeTileWorkUnit<'payload>,
        &mut SymbolDecoder<'payload>,
        &DecodeBlockFrontier,
        &TileIntraJointModeState,
        &TileUsesMrlsState,
        &TileUseDipState,
        &TileFscModeState,
        &TileLumaPaletteState,
        IsCflContext,
        &mut TileBlockDecodedState,
    ) -> Result<GeneralIntraLeafMode, E>,
{
    ensure_supported_traversal_frame(frame, false)?;

    let mut symbols = symbol_decoder_for_work_unit(work_unit)?;
    let mut lr_activity = if retain_lr_source_blocks {
        WienerNsLrUnitActivity::retaining_source_blocks()
    } else {
        WienerNsLrUnitActivity::default()
    };
    let tile_bounds = TilePartitionBounds::from_work_unit(work_unit);
    let mut y_modes = TileIntraYModeState::new(frame.mi_rows, frame.mi_cols)
        .map_err(TilePartitionTraversalError::from)?;
    let sb_size4 = frame
        .sb_size
        .num_4x4_wide()
        .map_err(TilePartitionTraversalError::from)?
        .max(1);
    let mi_row_start = work_unit.mi_row_range().start as usize;
    let mi_row_end = (work_unit.mi_row_range().end as usize).min(frame.mi_rows);
    let mi_col_start = work_unit.mi_col_range().start as usize;
    let mi_col_end = (work_unit.mi_col_range().end as usize).min(frame.mi_cols);
    let mut block_decoded = TileBlockDecodedState::new(
        frame.num_planes,
        usize::from(frame.subsampling_x),
        usize::from(frame.subsampling_y),
        sb_size4,
        mi_col_end,
        mi_row_end,
    )
    .map_err(TilePartitionTraversalError::from)?;
    let sb_mask = sb_size4.saturating_sub(1);
    let mut step_count: u64 = 0;
    let mut sdp_state = SdpPartitionState::default();

    let mut sb_row = mi_row_start;
    while sb_row < mi_row_end {
        mi_size_state.clear_left_context();
        let mut sb_col = mi_col_start;
        while sb_col < mi_col_end {
            block_decoded.clear_superblock(sb_row, sb_col);
            let root = TilePartitionCall::root(sb_row, sb_col, frame.sb_size, frame.has_chroma);
            let mut stack = vec![TilePartitionStackEntry::Partition(root)];
            while let Some(entry) = stack.pop() {
                let (call, forced_extended_sdp_chroma_block) = match entry {
                    TilePartitionStackEntry::Partition(call) => (call, false),
                    TilePartitionStackEntry::ExtendedSdpChromaBlock(call) => (call, true),
                };
                step_count += 1;
                limits
                    .ensure(DecodeLimitName::MaxTilePartitionSteps, step_count)
                    .map_err(TilePartitionTraversalError::from)?;
                if !call_in_frame(frame, call) {
                    continue;
                }
                if !forced_extended_sdp_chroma_block && is_intra_sdp_shared_root(frame, call) {
                    stack.push(TilePartitionStackEntry::Partition(
                        call.with_tree_type(PartitionTreeType::ChromaPart),
                    ));
                    stack.push(TilePartitionStackEntry::Partition(
                        call.with_tree_type(PartitionTreeType::LumaPart),
                    ));
                    continue;
                }
                let (call, sub_size, chroma_offset, partition_is_none) =
                    if forced_extended_sdp_chroma_block {
                        (call, call.b_size, false, true)
                    } else {
                        read_loop_restoration_for_call(
                            frame,
                            call,
                            tile_bounds,
                            work_unit.cdf_mut().tile_cdfs_mut(),
                            &mut symbols,
                            &mut lr_activity,
                            limits,
                        )?;

                        let step = mi_size_state
                            .with_context_state(|context| {
                                read_frontier_partition_step(
                                    call,
                                    frame,
                                    tile_bounds,
                                    context,
                                    &mut sdp_state,
                                    work_unit.cdf_mut().tile_cdfs_mut(),
                                    &mut symbols,
                                )
                            })
                            .map_err(GeneralIntraTreeWalkError::MiSize)??;
                        let call = step.call;
                        let partition = step.partition();

                        let sub_size = valid_subsize(partition, call.b_size)?;
                        let chroma_offset =
                            updated_chroma_offset(call, partition, sub_size, frame)?;
                        if partition != PartitionType::None {
                            let children =
                                child_calls(call, partition, sub_size, frame, chroma_offset)?;
                            if step.using_extended_sdp() {
                                stack.push(TilePartitionStackEntry::ExtendedSdpChromaBlock(
                                    extended_sdp_chroma_call(frame, call),
                                ));
                            }
                            stack.extend(
                                children
                                    .as_slice()
                                    .iter()
                                    .rev()
                                    .copied()
                                    .map(TilePartitionStackEntry::Partition),
                            );
                        }
                        (
                            call,
                            sub_size,
                            chroma_offset,
                            partition == PartitionType::None,
                        )
                    };
                if partition_is_none {
                    let stored_luma_y_mode = if call.tree_type == PartitionTreeType::ChromaPart {
                        y_modes.y_mode_facts_at(call.r, call.c)
                    } else {
                        None
                    };
                    let frontier = decode_block_frontier(
                        call,
                        frame,
                        sub_size,
                        chroma_offset,
                        stored_luma_y_mode,
                        &symbols,
                    );
                    let tree_type = frontier.tree_type;
                    let chroma_ref = frontier.chroma_ref_geometry();
                    let is_cfl_ctx =
                        is_cfl_context_for_chroma_ref(uv_cfls, tile_bounds, chroma_ref);
                    let leaf_mode = on_leaf(
                        work_unit,
                        &mut symbols,
                        &frontier,
                        joint_modes,
                        uses_mrls,
                        use_dip,
                        fsc_modes,
                        palette_y,
                        is_cfl_ctx,
                        &mut block_decoded,
                    )
                    .map_err(GeneralIntraTreeWalkError::Leaf)?;
                    let block_n4w = sub_size
                        .num_4x4_wide()
                        .map_err(TilePartitionTraversalError::from)?;
                    let block_n4h = sub_size
                        .num_4x4_high()
                        .map_err(TilePartitionTraversalError::from)?;
                    if let Some(uv_cfl) = leaf_mode.uv_cfl {
                        let chroma_ref = frontier.chroma_ref_geometry();
                        let chroma_n4w = chroma_ref
                            .size()
                            .num_4x4_wide()
                            .map_err(TilePartitionTraversalError::from)?;
                        let chroma_n4h = chroma_ref
                            .size()
                            .num_4x4_high()
                            .map_err(TilePartitionTraversalError::from)?;
                        uv_cfls.record_block(
                            chroma_ref.row(),
                            chroma_ref.col(),
                            chroma_n4w,
                            chroma_n4h,
                            uv_cfl,
                        );
                    }
                    if tree_type != PartitionTreeType::ChromaPart {
                        if let Some(joint_mode) = leaf_mode.intra_joint_mode {
                            let y_mode = leaf_mode.y_mode.ok_or(
                                TilePartitionTraversalError::MissingIntraLumaModeState {
                                    r: call.r,
                                    c: call.c,
                                },
                            )?;
                            let angle_delta_y = leaf_mode.angle_delta_y.ok_or(
                                TilePartitionTraversalError::MissingIntraLumaModeState {
                                    r: call.r,
                                    c: call.c,
                                },
                            )?;
                            let uses_mrls_value = leaf_mode.uses_mrls.ok_or(
                                TilePartitionTraversalError::MissingIntraUsesMrlsState {
                                    r: call.r,
                                    c: call.c,
                                },
                            )?;
                            let fsc_mode = leaf_mode.fsc_mode.ok_or(
                                TilePartitionTraversalError::MissingIntraFscModeState {
                                    r: call.r,
                                    c: call.c,
                                },
                            )?;
                            let use_dip_value = leaf_mode.use_dip.ok_or(
                                TilePartitionTraversalError::MissingIntraUseDipState {
                                    r: call.r,
                                    c: call.c,
                                },
                            )?;
                            joint_modes
                                .record_block(call.r, call.c, block_n4w, block_n4h, joint_mode);
                            fsc_modes.record_block(call.r, call.c, block_n4w, block_n4h, fsc_mode);
                            use_dip.record_block(
                                call.r,
                                call.c,
                                block_n4w,
                                block_n4h,
                                use_dip_value,
                            );
                            uses_mrls.record_block(
                                call.r,
                                call.c,
                                block_n4w,
                                block_n4h,
                                uses_mrls_value,
                            );
                            palette_y.record_block(
                                call.r,
                                call.c,
                                block_n4w,
                                block_n4h,
                                leaf_mode.palette_y,
                            );
                            y_modes.record_block(
                                call.r,
                                call.c,
                                block_n4w,
                                block_n4h,
                                y_mode,
                                angle_delta_y,
                            );
                        } else {
                            if frame.frame_is_intra && !leaf_mode.is_intrabc() {
                                return Err(GeneralIntraTreeWalkError::Traversal(
                                    TilePartitionTraversalError::MissingIntraLumaModeState {
                                        r: call.r,
                                        c: call.c,
                                    },
                                ));
                            }
                            joint_modes
                                .record_non_intra_block(call.r, call.c, block_n4w, block_n4h);
                            fsc_modes.record_non_intra_block(call.r, call.c, block_n4w, block_n4h);
                            use_dip.record_non_intra_block(call.r, call.c, block_n4w, block_n4h);
                            uses_mrls.record_non_intra_block(call.r, call.c, block_n4w, block_n4h);
                            palette_y.record_non_intra_block(call.r, call.c, block_n4w, block_n4h);
                            if leaf_mode.is_intrabc() {
                                y_modes.record_block(
                                    call.r,
                                    call.c,
                                    block_n4w,
                                    block_n4h,
                                    IntraYMode::DC_PRED, // § 5.20.5.3 AV2 intraBC mode = DC_PRED (decodemv.c); an SDP chroma-part reads this collocated luma mode
                                    0,
                                );
                            } else {
                                y_modes
                                    .record_non_intra_block(call.r, call.c, block_n4w, block_n4h);
                            }
                        }
                    }
                    let sub_block_mi_row = call.r & sb_mask;
                    let sub_block_mi_col = call.c & sb_mask;
                    let (plane_start, plane_end) =
                        plane_range_for_tree_type(tree_type, frame.num_planes);
                    for plane in plane_start..plane_end {
                        let (sub_x, sub_y) = plane_subsampling(frame, plane);
                        block_decoded.set_block(
                            plane,
                            sub_block_mi_row,
                            sub_block_mi_col,
                            (block_n4w >> sub_x).max(1),
                            (block_n4h >> sub_y).max(1),
                        );
                    }
                    if tree_type != PartitionTreeType::ChromaPart {
                        mi_size_state
                            .update_luma_block(call.r, call.c, sub_size)
                            .map_err(GeneralIntraTreeWalkError::MiSize)?;
                    }
                    if frontier.has_chroma || tree_type == PartitionTreeType::ChromaPart {
                        let chroma_ref = call.chroma_ref_geometry();
                        mi_size_state
                            .update_chroma_block(chroma_ref.row, chroma_ref.col, chroma_ref.size)
                            .map_err(GeneralIntraTreeWalkError::MiSize)?;
                    }
                }
            }
            sb_col += sb_size4;
        }
        sb_row += sb_size4;
    }

    Ok(GeneralIntraPartitionTreeOutput {
        symbols,
        active_source_blocks: lr_activity.active_source_blocks,
        unit_filters: lr_activity.unit_filters,
    })
}

fn read_loop_restoration_for_call(
    frame: TilePartitionFrameFacts,
    call: TilePartitionCall,
    tile_bounds: TilePartitionBounds,
    cdfs: &mut super::cdf::TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    lr_activity: &mut WienerNsLrUnitActivity,
    limits: DecodeLimits,
) -> Result<(), TilePartitionTraversalError> {
    if call.b_size != frame.sb_size {
        return Ok(());
    }
    let TilePartitionLoopRestorationState::FrameWienerNs(lr) = frame.loop_restoration else {
        return Ok(());
    };
    let w = call.b_size.num_4x4_wide()?;
    let h = call.b_size.num_4x4_high()?;
    let (plane_start, plane_end) = plane_range_for_tree_type(call.tree_type, frame.num_planes);
    for plane in plane_start..plane_end.min(3) {
        if !lr.plane_enabled[plane] {
            continue;
        }
        read_wiener_ns_lr_units_for_plane(
            plane,
            lr.unit_size[plane],
            lr.frame_filters_on[plane],
            frame,
            call,
            tile_bounds,
            w,
            h,
            cdfs,
            symbols,
            lr_activity,
            limits,
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn read_wiener_ns_lr_units_for_plane(
    plane: usize,
    unit_size: usize,
    frame_filters_on: bool,
    frame: TilePartitionFrameFacts,
    call: TilePartitionCall,
    tile_bounds: TilePartitionBounds,
    w: usize,
    h: usize,
    cdfs: &mut super::cdf::TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    lr_activity: &mut WienerNsLrUnitActivity,
    limits: DecodeLimits,
) -> Result<(), TilePartitionTraversalError> {
    if unit_size == 0 {
        return Err(
            TilePartitionTraversalError::InvalidLoopRestorationUnitSize { plane, unit_size },
        );
    }
    let (sub_x, sub_y) = plane_subsampling(frame, plane);
    let sample_step_x = MI_SIZE >> sub_x;
    let sample_step_y = MI_SIZE >> sub_y;

    let mi_cols = checked_sub(
        "lr_mi_cols",
        tile_bounds.mi_col_end,
        tile_bounds.mi_col_start,
    )?;
    let mi_rows = checked_sub(
        "lr_mi_rows",
        tile_bounds.mi_row_end,
        tile_bounds.mi_row_start,
    )?;
    let frame_cols = checked_mul_shifted("lr_frame_cols", mi_cols, MI_SIZE, sub_x)?;
    let frame_rows = checked_mul_shifted("lr_frame_rows", mi_rows, MI_SIZE, sub_y)?;
    let lr_row_offset =
        checked_mul_shifted("lr_row_offset", tile_bounds.mi_row_start, MI_SIZE, sub_y)? / unit_size;
    let lr_col_offset =
        checked_mul_shifted("lr_col_offset", tile_bounds.mi_col_start, MI_SIZE, sub_x)? / unit_size;
    let c = checked_sub("lr_c", call.c, tile_bounds.mi_col_start)?;
    let r = checked_sub("lr_r", call.r, tile_bounds.mi_row_start)?;

    let unit_rows = count_units_in_frame(unit_size, frame_rows)?;
    let unit_cols = count_units_in_frame(unit_size, frame_cols)?;
    let unit_row_start = ceil_unit_index(
        checked_mul("lr_unit_row_start", r, sample_step_y)?,
        unit_size,
    )?;
    let unit_col_start = ceil_unit_index(
        checked_mul("lr_unit_col_start", c, sample_step_x)?,
        unit_size,
    )?;
    let unit_row_end = unit_rows.min(ceil_unit_index(
        checked_mul(
            "lr_unit_row_end",
            checked_add("lr_r_end", r, h)?,
            sample_step_y,
        )?,
        unit_size,
    )?);
    let unit_col_end = unit_cols.min(ceil_unit_index(
        checked_mul(
            "lr_unit_col_end",
            checked_add("lr_c_end", c, w)?,
            sample_step_x,
        )?,
        unit_size,
    )?);

    for unit_row in unit_row_start..unit_row_end {
        for unit_col in unit_col_start..unit_col_end {
            let unit_row = checked_add("lr_unit_row", unit_row, lr_row_offset)?;
            let unit_col = checked_add("lr_unit_col", unit_col, lr_col_offset)?;
            let active = read_wiener_ns_lr_unit(
                plane,
                frame_filters_on,
                unit_row,
                unit_col,
                cdfs,
                symbols,
                lr_activity,
                limits,
            )?;
            if active {
                record_active_wiener_ns_source_blocks_for_unit(
                    LrSourceBlockDerivation {
                        plane,
                        unit_size,
                        unit_row,
                        unit_col,
                        frame,
                        tile_bounds,
                        sub_x,
                        sub_y,
                    },
                    limits,
                    lr_activity,
                )?;
            }
        }
    }
    Ok(())
}
#[allow(clippy::too_many_arguments)]
fn read_wiener_ns_lr_unit(
    plane: usize,
    frame_filters_on: bool,
    unit_row: usize,
    unit_col: usize,
    cdfs: &mut super::cdf::TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    lr_activity: &mut WienerNsLrUnitActivity,
    limits: DecodeLimits,
) -> Result<bool, TilePartitionTraversalError> {
    let use_wiener_ns = cdfs
        .with_row_mut(super::cdf::TileCdfSelector::UseWienerNs, |row| {
            symbols.read_symbol(row)
        })??
        .get()
        != 0;
    lr_activity.record(plane, unit_row, unit_col, use_wiener_ns)?;
    if use_wiener_ns && !frame_filters_on {
        let filter =
            read_wiener_ns_unit_filter(plane, cdfs, symbols, &mut lr_activity.unit_filter_state)?;
        lr_activity.record_unit_filter(
            WienerNsLrUnitFilter {
                plane,
                unit_row,
                unit_col,
                coeff_count: wiener_ns_coeff_count(plane),
                coeffs: filter,
            },
            limits,
        )?;
    }
    Ok(use_wiener_ns)
}
fn read_wiener_ns_unit_filter(
    plane: usize,
    cdfs: &mut super::cdf::TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    state: &mut WienerNsUnitFilterState,
) -> Result<[i16; WIENER_NS_CHROMA_COEFFS], TilePartitionTraversalError> {
    let merged = read_wiener_ns_raw_literal(symbols, 1)? != 0;
    let previous_bank_size = state.bank_size[plane];
    let mut ref_from_last = 0usize;
    while ref_from_last < previous_bank_size.saturating_sub(1) {
        let use_bank = read_wiener_ns_raw_literal(symbols, 1)? != 0;
        if use_bank {
            break;
        }
        ref_from_last = checked_add("wiener_ns_bank_ref", ref_from_last, 1)?;
    }
    if merged {
        if state.bank_size[plane] == 0 {
            let coeffs = state.bank[plane][0];
            add_wiener_ns_unit_filter_to_bank(state, plane, coeffs)?;
            return Ok(coeffs);
        }
        let ref_index = wiener_ns_bank_ref_index(state, plane, ref_from_last)?;
        return Ok(state.bank[plane][ref_index]);
    }

    let ref_index = wiener_ns_bank_ref_index(state, plane, ref_from_last)?;
    let ref_coeffs = state.bank[plane][ref_index];
    let subset = read_wiener_ns_subset_symbol(plane, cdfs, symbols)?;
    let wiener_ns_uv_sym = if plane > 0 && subset > 0 {
        cdfs.with_row_mut(super::cdf::TileCdfSelector::WienerNsUvSym, |row| {
            symbols.read_symbol(row)
        })??
        .get()
            != 0
    } else {
        false
    };

    let plane_index = usize::from(plane > 0);
    let n_coeffs = wiener_ns_coeff_count(plane);
    let mut coeffs = [0i16; WIENER_NS_CHROMA_COEFFS];
    let mut j = 0usize;
    while j < n_coeffs {
        if WIENER_NS_TAPS_PRESENT[plane_index][subset][j] {
            let min = WIENER_NS_TAPS_MIN[plane_index][j];
            let ref_symb = ref_coeffs[j].checked_sub(min).ok_or(
                TilePartitionTraversalError::CoordinateUnderflow {
                    coordinate: "wiener_ns_ref_symb",
                    base: ref_coeffs[j] as usize,
                    offset: min.unsigned_abs() as usize,
                },
            )?;
            let decoded = read_wiener_ns_4part_wref(
                WIENER_NS_TAPS_K[plane_index][j],
                usize::try_from(ref_symb).map_err(|_| {
                    TilePartitionTraversalError::CoordinateOverflow {
                        coordinate: "wiener_ns_ref_symb",
                        base: ref_symb as usize,
                        offset: 0,
                    }
                })?,
                cdfs,
                symbols,
            )?;
            let value = i32::try_from(decoded).map_err(|_| {
                TilePartitionTraversalError::CoordinateOverflow {
                    coordinate: "wiener_ns_coeff",
                    base: decoded,
                    offset: 0,
                }
            })? + i32::from(min);
            coeffs[j] = i16::try_from(value).map_err(|_| {
                TilePartitionTraversalError::CoordinateOverflow {
                    coordinate: "wiener_ns_coeff",
                    base: decoded,
                    offset: min.unsigned_abs() as usize,
                }
            })?;
        }
        if plane > 0 && j >= WIENER_NS_SHORT_COEFFS && wiener_ns_uv_sym {
            let next_j = checked_add("wiener_ns_coeff_index", j, 1)?;
            if next_j < n_coeffs {
                coeffs[next_j] = coeffs[j];
            }
            j = checked_add("wiener_ns_coeff_index", j, 2)?;
        } else {
            j = checked_add("wiener_ns_coeff_index", j, 1)?;
        }
    }
    add_wiener_ns_unit_filter_to_bank(state, plane, coeffs)?;
    Ok(coeffs)
}

fn read_wiener_ns_subset_symbol(
    plane: usize,
    cdfs: &mut super::cdf::TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
) -> Result<usize, TilePartitionTraversalError> {
    let num_subsets = if plane > 0 {
        WIENER_NS_CHROMA_SUBSETS
    } else {
        WIENER_NS_LUMA_SUBSETS
    };
    let mut subset = 0usize;
    while subset < num_subsets.saturating_sub(1) {
        let wiener_ns_length = cdfs.with_row_mut(
            super::cdf::TileCdfSelector::WienerNsLength {
                plane_ctx: plane.min(1),
            },
            |row| symbols.read_symbol(row),
        )??;
        if wiener_ns_length.get() == 0 {
            break;
        }
        subset = checked_add("wiener_ns_subset", subset, 1)?;
    }
    Ok(subset)
}

const fn wiener_ns_coeff_count(plane: usize) -> usize {
    if plane > 0 {
        WIENER_NS_CHROMA_COEFFS
    } else {
        WIENER_NS_LUMA_COEFFS
    }
}

const fn wiener_ns_initial_tap_value(plane_index: usize, j: usize) -> i16 {
    WIENER_NS_TAPS_MIN[plane_index][j] + ((1i16 << WIENER_NS_TAPS_K[plane_index][j]) >> 1)
}

fn wiener_ns_bank_ref_index(
    state: &WienerNsUnitFilterState,
    plane: usize,
    ref_from_last: usize,
) -> Result<usize, TilePartitionTraversalError> {
    let bank_size = state.bank_size[plane];
    if bank_size == 0 {
        return Ok(0);
    }
    if ref_from_last >= bank_size {
        return Err(TilePartitionTraversalError::CoordinateOverflow {
            coordinate: "wiener_ns_bank_ref",
            base: ref_from_last,
            offset: bank_size,
        });
    }
    let ptr = state.bank_ptr[plane];
    if ptr < ref_from_last {
        checked_add("wiener_ns_bank_ref_index", ptr, LR_BANK_SIZE)
            .and_then(|base| checked_sub("wiener_ns_bank_ref_index", base, ref_from_last))
    } else {
        checked_sub("wiener_ns_bank_ref_index", ptr, ref_from_last)
    }
}

fn add_wiener_ns_unit_filter_to_bank(
    state: &mut WienerNsUnitFilterState,
    plane: usize,
    coeffs: [i16; WIENER_NS_CHROMA_COEFFS],
) -> Result<(), TilePartitionTraversalError> {
    if state.bank_size[plane] < LR_BANK_SIZE {
        state.bank_ptr[plane] = state.bank_size[plane];
        state.bank_size[plane] = checked_add("wiener_ns_bank_size", state.bank_size[plane], 1)?;
    } else {
        state.bank_ptr[plane] =
            checked_add("wiener_ns_bank_ptr", state.bank_ptr[plane], 1)? % LR_BANK_SIZE;
    }
    state.bank[plane][state.bank_ptr[plane]] = coeffs;
    Ok(())
}

fn read_wiener_ns_4part_wref(
    k: u8,
    ref_symb: usize,
    cdfs: &mut super::cdf::TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
) -> Result<usize, TilePartitionTraversalError> {
    let wiener_ns_base = cdfs
        .with_row_mut(super::cdf::TileCdfSelector::WienerNsBase, |row| {
            symbols.read_symbol(row)
        })??
        .get() as usize;
    let nsymb_bits = usize::from(k);
    let part_bits = [
        checked_sub("wiener_ns_4part_bits", nsymb_bits, 3)?,
        checked_sub("wiener_ns_4part_bits", nsymb_bits, 3)?,
        checked_sub("wiener_ns_4part_bits", nsymb_bits, 2)?,
        checked_sub("wiener_ns_4part_bits", nsymb_bits, 1)?,
    ];
    let part_offsets = [
        0usize,
        checked_shl("wiener_ns_4part_offset", 1, part_bits[0])?,
        checked_shl("wiener_ns_4part_offset", 1, part_bits[2])?,
        checked_shl("wiener_ns_4part_offset", 1, part_bits[3])?,
    ];
    let bits =
        *part_bits
            .get(wiener_ns_base)
            .ok_or(TilePartitionTraversalError::CoordinateOverflow {
                coordinate: "wiener_ns_4part_part",
                base: wiener_ns_base,
                offset: 0,
            })?;
    let bits =
        u32::try_from(bits).map_err(|_| TilePartitionTraversalError::CoordinateOverflow {
            coordinate: "wiener_ns_4part_bits",
            base: usize::from(k),
            offset: 0,
        })?;
    let literal = usize::try_from(read_wiener_ns_raw_literal(symbols, bits)?).map_err(|_| {
        TilePartitionTraversalError::CoordinateOverflow {
            coordinate: "wiener_ns_4part_literal",
            base: usize::from(k),
            offset: 0,
        }
    })?;
    let offset = *part_offsets.get(wiener_ns_base).ok_or(
        TilePartitionTraversalError::CoordinateOverflow {
            coordinate: "wiener_ns_4part_part",
            base: wiener_ns_base,
            offset: 0,
        },
    )?;
    let symbol = checked_add("wiener_ns_4part_symbol", literal, offset)?;
    let n = checked_shl("wiener_ns_4part_range", 1, nsymb_bits)?;
    inverse_recenter_finite_nonneg(n, ref_symb, symbol)
}

fn read_wiener_ns_raw_literal(
    symbols: &mut SymbolDecoder<'_>,
    bits: u32,
) -> Result<u32, TilePartitionTraversalError> {
    let value = symbols.read_literal(bits)?;
    Ok(value)
}

fn inverse_recenter_finite_nonneg(
    n: usize,
    r: usize,
    v: usize,
) -> Result<usize, TilePartitionTraversalError> {
    if n == 0 || r >= n || v >= n {
        return Err(TilePartitionTraversalError::CoordinateOverflow {
            coordinate: "wiener_ns_recenter",
            base: r,
            offset: v,
        });
    }
    if checked_mul("wiener_ns_recenter", r, 2)? <= n {
        inverse_recenter_nonneg(r, v)
    } else {
        let mirrored_r = checked_sub("wiener_ns_recenter", n - 1, r)?;
        let mirrored = inverse_recenter_nonneg(mirrored_r, v)?;
        checked_sub("wiener_ns_recenter", n - 1, mirrored)
    }
}

fn inverse_recenter_nonneg(r: usize, v: usize) -> Result<usize, TilePartitionTraversalError> {
    if v > checked_mul("wiener_ns_recenter", r, 2)? {
        return Ok(v);
    }
    if v & 1 == 0 {
        checked_add("wiener_ns_recenter", v >> 1, r)
    } else {
        checked_sub(
            "wiener_ns_recenter",
            r,
            checked_add("wiener_ns_recenter", v, 1)? >> 1,
        )
    }
}

#[derive(Clone, Copy)]
struct LrSourceBlockDerivation {
    plane: usize,
    unit_size: usize,
    unit_row: usize,
    unit_col: usize,
    frame: TilePartitionFrameFacts,
    tile_bounds: TilePartitionBounds,
    sub_x: usize,
    sub_y: usize,
}

fn record_active_wiener_ns_source_blocks_for_unit(
    input: LrSourceBlockDerivation,
    limits: DecodeLimits,
    lr_activity: &mut WienerNsLrUnitActivity,
) -> Result<(), TilePartitionTraversalError> {
    if !lr_activity.retain_source_blocks {
        return Ok(());
    }
    let geometry = lr_unit_geometry(input)?;
    let mut rows = Vec::new();
    for row in input.tile_bounds.mi_row_start..input.tile_bounds.mi_row_end {
        if lr_unit_row_for_mi(input, geometry, row)? == input.unit_row {
            rows.push(row);
        }
    }
    let mut cols = Vec::new();
    for col in input.tile_bounds.mi_col_start..input.tile_bounds.mi_col_end {
        if lr_unit_col_for_mi(input, geometry, col)? == input.unit_col {
            cols.push(col);
        }
    }
    for &row in &rows {
        for &col in &cols {
            let block = lr_source_block_for(input, row, col)?;
            lr_activity.record_source_block(block, limits)?;
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct LrUnitGeometry {
    unit_rows: usize,
    unit_cols: usize,
    lr_row_offset: usize,
    lr_col_offset: usize,
}

fn lr_unit_geometry(
    input: LrSourceBlockDerivation,
) -> Result<LrUnitGeometry, TilePartitionTraversalError> {
    let mi_cols = checked_sub(
        "lr_source_mi_cols",
        input.tile_bounds.mi_col_end,
        input.tile_bounds.mi_col_start,
    )?;
    let mi_rows = checked_sub(
        "lr_source_mi_rows",
        input.tile_bounds.mi_row_end,
        input.tile_bounds.mi_row_start,
    )?;
    let frame_cols = checked_mul_shifted("lr_source_frame_cols", mi_cols, MI_SIZE, input.sub_x)?;
    let frame_rows = checked_mul_shifted("lr_source_frame_rows", mi_rows, MI_SIZE, input.sub_y)?;
    let unit_rows = count_units_in_frame(input.unit_size, frame_rows)?;
    let unit_cols = count_units_in_frame(input.unit_size, frame_cols)?;
    let lr_row_offset = checked_mul_shifted(
        "lr_source_row_offset",
        input.tile_bounds.mi_row_start,
        MI_SIZE,
        input.sub_y,
    )? / input.unit_size;
    let lr_col_offset = checked_mul_shifted(
        "lr_source_col_offset",
        input.tile_bounds.mi_col_start,
        MI_SIZE,
        input.sub_x,
    )? / input.unit_size;
    Ok(LrUnitGeometry {
        unit_rows,
        unit_cols,
        lr_row_offset,
        lr_col_offset,
    })
}

fn lr_unit_row_for_mi(
    input: LrSourceBlockDerivation,
    geometry: LrUnitGeometry,
    row: usize,
) -> Result<usize, TilePartitionTraversalError> {
    let local_row = checked_sub("lr_source_row", row, input.tile_bounds.mi_row_start)?;
    let row_sample = checked_mul("lr_source_unit_row_sample", local_row, MI_SIZE)?;
    let row_sample = checked_add("lr_source_unit_row_sample", row_sample, 8)?;
    let row_sample = row_sample >> input.sub_y;
    checked_add(
        "lr_source_unit_row",
        geometry.lr_row_offset,
        (row_sample / input.unit_size).min(geometry.unit_rows.saturating_sub(1)),
    )
}

fn lr_unit_col_for_mi(
    input: LrSourceBlockDerivation,
    geometry: LrUnitGeometry,
    col: usize,
) -> Result<usize, TilePartitionTraversalError> {
    let local_col = checked_sub("lr_source_col", col, input.tile_bounds.mi_col_start)?;
    let col_sample =
        checked_mul_shifted("lr_source_unit_col_sample", local_col, MI_SIZE, input.sub_x)?;
    checked_add(
        "lr_source_unit_col",
        geometry.lr_col_offset,
        (col_sample / input.unit_size).min(geometry.unit_cols.saturating_sub(1)),
    )
}

fn lr_source_block_for(
    input: LrSourceBlockDerivation,
    row: usize,
    col: usize,
) -> Result<WienerNsLrSourceBlock, TilePartitionTraversalError> {
    let x = checked_mul_shifted("lr_source_x", col, MI_SIZE, input.sub_x)?;
    let y = checked_mul_shifted("lr_source_y", row, MI_SIZE, input.sub_y)?;
    let width = MI_SIZE >> input.sub_x;
    let height = MI_SIZE >> input.sub_y;
    let (luma_start_x_mi, luma_end_x_mi, luma_start_y_mi, luma_end_y_mi) =
        if input.frame.disable_loopfilters_across_tiles {
            (
                input.tile_bounds.mi_col_start,
                input.tile_bounds.mi_col_end,
                input.tile_bounds.mi_row_start,
                input.tile_bounds.mi_row_end,
            )
        } else {
            (0, input.frame.mi_cols, 0, input.frame.mi_rows)
        };
    let luma_start_x = checked_mul("lr_luma_start_x", luma_start_x_mi, MI_SIZE)?;
    let luma_start_y = checked_mul("lr_luma_start_y", luma_start_y_mi, MI_SIZE)?;
    let luma_end_x = checked_sub(
        "lr_luma_end_x",
        checked_mul("lr_luma_end_x", luma_end_x_mi, MI_SIZE)?,
        1,
    )?;
    let luma_end_y = checked_sub(
        "lr_luma_end_y",
        checked_mul("lr_luma_end_y", luma_end_y_mi, MI_SIZE)?,
        1,
    )?;
    let frame_luma_end_y = checked_sub(
        "lr_frame_luma_end_y",
        checked_mul("lr_frame_luma_end_y", input.frame.mi_rows, MI_SIZE)?,
        1,
    )?;
    let local_row = checked_sub("lr_source_local_row", row, input.tile_bounds.mi_row_start)?;
    let luma_y = checked_mul("lr_source_luma_y", local_row, MI_SIZE)?;
    let stripe_num = checked_add("lr_source_stripe_num", luma_y, 8)? / 64;
    let stripe_base = checked_add(
        "lr_source_stripe_base",
        checked_mul(
            "lr_source_stripe_base",
            input.tile_bounds.mi_row_start,
            MI_SIZE,
        )?,
        checked_mul("lr_source_stripe_base", stripe_num, 64)?,
    )?;
    let luma_stripe_start_y = stripe_base
        .checked_sub(8)
        .map_or(luma_start_y, |start| luma_start_y.max(start));
    let luma_stripe_end_y = luma_end_y.min(checked_add("lr_source_stripe_end_y", stripe_base, 55)?);

    Ok(WienerNsLrSourceBlock {
        plane: input.plane,
        row,
        col,
        unit_row: input.unit_row,
        unit_col: input.unit_col,
        tile_mi_row_start: input.tile_bounds.mi_row_start,
        tile_mi_row_end: input.tile_bounds.mi_row_end,
        tile_mi_col_start: input.tile_bounds.mi_col_start,
        tile_mi_col_end: input.tile_bounds.mi_col_end,
        x,
        y,
        width,
        height,
        luma_start_x,
        luma_end_x,
        luma_start_y,
        luma_end_y,
        frame_luma_end_y,
        luma_stripe_start_y,
        luma_stripe_end_y,
    })
}

fn count_units_in_frame(
    unit_size: usize,
    frame_size: usize,
) -> Result<usize, TilePartitionTraversalError> {
    Ok(checked_add("lr_count_units", frame_size, unit_size >> 1)? / unit_size)
        .map(|count| count.max(1))
}

fn ceil_unit_index(value: usize, unit_size: usize) -> Result<usize, TilePartitionTraversalError> {
    let adjusted = checked_add("lr_unit_ceil", value, unit_size.saturating_sub(1))?;
    Ok(adjusted / unit_size)
}

fn checked_mul_shifted(
    coordinate: &'static str,
    value: usize,
    scale: usize,
    shift: usize,
) -> Result<usize, TilePartitionTraversalError> {
    Ok(checked_mul(coordinate, value, scale)? >> shift)
}

fn read_frontier_partition_step(
    call: TilePartitionCall,
    frame: TilePartitionFrameFacts,
    tile_bounds: TilePartitionBounds,
    context: TilePartitionContextState<'_>,
    sdp_state: &mut SdpPartitionState,
    cdfs: &mut super::cdf::TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
) -> Result<TilePartitionFrontierStep, TilePartitionTraversalError> {
    let symbol_count_before = symbols.symbol_count();
    let forced_chroma_partition = sdp_state.forced_chroma_partition(frame, call);
    let decision = read_frontier_partition_decision(
        call,
        frame,
        tile_bounds,
        context,
        forced_chroma_partition,
        cdfs,
        symbols,
    )?;
    let symbol_count_after = symbols.symbol_count();
    let partition = decision.partition;
    let call = call.with_cfl_allowed_in_sdp(sdp_state.record_partition(frame, call, partition));
    let (call, using_extended_sdp) =
        read_extended_sdp_region_type(frame, call, partition, cdfs, symbols)?;
    Ok(TilePartitionFrontierStep {
        call,
        decision,
        symbol_count_before,
        symbol_count_after,
        using_extended_sdp,
    })
}

fn read_frontier_partition_decision(
    call: TilePartitionCall,
    frame: TilePartitionFrameFacts,
    tile_bounds: TilePartitionBounds,
    context: TilePartitionContextState<'_>,
    forced_chroma_partition: Option<PartitionType>,
    cdfs: &mut super::cdf::TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
) -> Result<ReadPartitionDecision, TilePartitionTraversalError> {
    let mixed_region = !frame.frame_is_intra && call.parent_size.is_some() && !call.intra_region;
    let allowed = PartitionAllowedInput::new(
        call.r,
        call.c,
        frame.mi_rows,
        frame.mi_cols,
        call.b_size.index(),
        call.tree_type,
        frame.subsampling_x,
        frame.subsampling_y,
        frame.features,
        frame.frame_is_intra,
        mixed_region,
        frame.max_pb_aspect_ratio,
        call.has_chroma,
        call.chroma_offset,
        frame.num_planes,
        forced_chroma_partition,
    )?;
    let facts = partition_decision_facts(allowed)?;
    let partition_plane = partition_cdf_plane(call.tree_type);
    let partition_context = PartitionContextInput::new(
        call.b_size.index(),
        partition_plane,
        call.r,
        call.c,
        context.left_mi_sizes,
        context.above_mi_sizes,
    )?;
    let avail_u = tile_bounds.avail_u(call);
    let avail_l = tile_bounds.avail_l(call);
    let square_context = SquareSplitContextInput::new(
        call.b_size.index(),
        0,
        call.r,
        call.c,
        avail_u,
        avail_l,
        context.mi_sizes,
    )?;
    let decision_input =
        facts.read_partition_decision_input(true, partition_context, square_context);
    let decision = super::partition::read_partition_decision(decision_input, cdfs, symbols)?;
    Ok(decision)
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

const fn partition_cdf_plane(tree_type: PartitionTreeType) -> usize {
    matches!(tree_type, PartitionTreeType::ChromaPart) as usize
}

fn is_intra_sdp_shared_root(frame: TilePartitionFrameFacts, call: TilePartitionCall) -> bool {
    frame.enable_sdp
        && frame.frame_is_intra
        && call.tree_type == PartitionTreeType::Shared
        && call.b_size.index() == BLOCK_64X64
}

pub(crate) fn extended_sdp_allowed_for_child(
    frame: TilePartitionFrameFacts,
    call: TilePartitionCall,
    partition: PartitionType,
    sub_size: BlockSize,
) -> bool {
    if !frame.enable_extended_sdp || !call.extended_sdp_allowed {
        return false;
    }
    let Ok(width) = sub_size.width_samples() else {
        return false;
    };
    let Ok(height) = sub_size.height_samples() else {
        return false;
    };
    if width <= 4 || height <= 4 {
        return false;
    }
    if !matches!(partition, PartitionType::Horz3 | PartitionType::Vert3) {
        return true;
    }
    let Ok(middle_size) = h_partition_midsize(call.b_size) else {
        return false;
    };
    let Some(middle_size) = middle_size.valid() else {
        return false;
    };
    let Ok(middle_width) = middle_size.width_samples() else {
        return false;
    };
    let Ok(middle_height) = middle_size.height_samples() else {
        return false;
    };
    middle_width > 4 && middle_height > 4
}

fn read_extended_sdp_region_type(
    frame: TilePartitionFrameFacts,
    call: TilePartitionCall,
    partition: PartitionType,
    cdfs: &mut super::cdf::TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
) -> Result<(TilePartitionCall, bool), TilePartitionTraversalError> {
    if !should_read_extended_sdp_region_type(frame, call, partition)? {
        return Ok((call, false));
    }
    let ctx = intra_region_context(call.b_size)?;
    let selector = super::cdf::TileCdfSelector::RegionType { ctx };
    let region_type = cdfs
        .with_row_mut(selector, |row| symbols.read_symbol(row))??
        .get();
    match region_type {
        INTRA_REGION => Ok((
            call.with_tree_type(PartitionTreeType::LumaPart)
                .with_intra_region(true),
            true,
        )),
        MIXED_REGION => Ok((call.with_intra_region(false), false)),
        value => Err(TilePartitionTraversalError::InvalidRegionType { value }),
    }
}

fn should_read_extended_sdp_region_type(
    frame: TilePartitionFrameFacts,
    call: TilePartitionCall,
    partition: PartitionType,
) -> Result<bool, TilePartitionTraversalError> {
    if frame.frame_is_intra
        || !frame.enable_extended_sdp
        || call.tree_type != PartitionTreeType::Shared
        || call.intra_region
        || !call.extended_sdp_allowed
        || call.b_size == frame.sb_size
        || frame.bru_state != TilePartitionBruState::Active
        || partition == PartitionType::None
    {
        return Ok(false);
    }
    is_bsize_allowed_for_extended_sdp(call.b_size, partition)
}
fn is_bsize_allowed_for_extended_sdp(
    b_size: BlockSize,
    partition: PartitionType,
) -> Result<bool, TilePartitionTraversalError> {
    let width = b_size.width_samples()?;
    let height = b_size.height_samples()?;
    Ok(width <= INTER_SDP_MAX_BLOCK_SIZE
        && height <= INTER_SDP_MAX_BLOCK_SIZE
        && width >= 8
        && height >= 8
        && matches!(
            partition,
            PartitionType::Horz | PartitionType::Vert | PartitionType::Horz3 | PartitionType::Vert3
        ))
}

fn intra_region_context(b_size: BlockSize) -> Result<usize, TilePartitionTraversalError> {
    let samples = checked_mul(
        "region_type_area",
        b_size.width_samples()?,
        b_size.height_samples()?,
    )?;
    Ok(match samples {
        0..=128 => 0,
        129..=512 => 1,
        513..=1024 => 2,
        _ => 3,
    })
}

fn extended_sdp_chroma_call(
    frame: TilePartitionFrameFacts,
    call: TilePartitionCall,
) -> TilePartitionCall {
    TilePartitionCall::child(
        call.r,
        call.c,
        call.b_size,
        call.parent_size,
        false,
        frame.has_chroma,
        PartitionTreeType::ChromaPart,
        Some(ChromaRefGeometry::new(call.r, call.c, call.b_size)),
        false,
        true,
    )
    .with_cfl_allowed_in_sdp(true)
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct SdpPartitionState {
    top_luma_horz: bool,
    top_luma_vert: bool,
    top_luma_uneven_horz: bool,
    top_luma_uneven_vert: bool,
    chroma_follows_luma: bool,
    luma_64x64_partition: Option<PartitionType>,
}

impl SdpPartitionState {
    fn record_partition(
        &mut self,
        frame: TilePartitionFrameFacts,
        call: TilePartitionCall,
        partition: PartitionType,
    ) -> bool {
        if !frame.frame_is_intra {
            return call.cfl_allowed_in_sdp;
        }

        if call.tree_type == PartitionTreeType::LumaPart && call.b_size.index() == BLOCK_64X64 {
            self.top_luma_horz = matches!(partition, PartitionType::Horz | PartitionType::Horz3);
            self.top_luma_vert = matches!(partition, PartitionType::Vert | PartitionType::Vert3);
            self.top_luma_uneven_horz =
                matches!(partition, PartitionType::Horz4A | PartitionType::Horz4B);
            self.top_luma_uneven_vert =
                matches!(partition, PartitionType::Vert4A | PartitionType::Vert4B);
            self.chroma_follows_luma =
                partition == PartitionType::None || self.top_luma_horz || self.top_luma_vert;
            self.luma_64x64_partition = Some(partition);
        }

        let this_horz = matches!(
            partition,
            PartitionType::Horz
                | PartitionType::Horz3
                | PartitionType::Horz4A
                | PartitionType::Horz4B
        );
        let this_vert = matches!(
            partition,
            PartitionType::Vert
                | PartitionType::Vert3
                | PartitionType::Vert4A
                | PartitionType::Vert4B
        );

        let mut cfl_allowed_in_sdp = call.cfl_allowed_in_sdp;
        if call.tree_type == PartitionTreeType::ChromaPart && call.b_size.index() == BLOCK_64X64 {
            cfl_allowed_in_sdp = self.chroma_follows_luma
                || partition == PartitionType::None
                || ((self.top_luma_horz || self.top_luma_uneven_horz) && this_horz)
                || ((self.top_luma_vert || self.top_luma_uneven_vert) && this_vert);
        }

        if call.tree_type == PartitionTreeType::LumaPart
            && call
                .parent_size
                .is_some_and(|parent| parent.index() == BLOCK_64X64)
            && (partition == PartitionType::None
                || (self.top_luma_horz && this_horz)
                || (self.top_luma_vert && this_vert))
        {
            self.chroma_follows_luma = false;
        }

        cfl_allowed_in_sdp
    }

    fn forced_chroma_partition(
        &self,
        frame: TilePartitionFrameFacts,
        call: TilePartitionCall,
    ) -> Option<PartitionType> {
        if !frame.frame_is_intra
            || call.tree_type != PartitionTreeType::ChromaPart
            || call.b_size.index() != BLOCK_64X64
            || !self.chroma_follows_luma
        {
            return None;
        }
        self.luma_64x64_partition
    }
}

fn updated_chroma_offset(
    call: TilePartitionCall,
    partition: PartitionType,
    sub_size: BlockSize,
    frame: TilePartitionFrameFacts,
) -> Result<bool, TilePartitionTraversalError> {
    if call.chroma_offset || !call.has_chroma {
        return Ok(call.chroma_offset);
    }
    if is_chroma_offset_for_subsize(sub_size, frame)? {
        return Ok(true);
    }
    if partition == PartitionType::Horz3 {
        let middle_chroma = call.b_size.index() == BLOCK_8X32 && frame.subsampling_x;
        if !middle_chroma
            && let Some(midsize) = h_partition_midsize(call.b_size)?.valid()
            && is_chroma_offset_for_subsize(midsize, frame)?
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn is_chroma_offset_for_subsize(
    sub_size: BlockSize,
    frame: TilePartitionFrameFacts,
) -> Result<bool, TilePartitionTraversalError> {
    Ok((frame.subsampling_y && sub_size.mi_height_log2()? == 0)
        || (frame.subsampling_x && sub_size.mi_width_log2()? == 0))
}

pub(crate) fn valid_subsize(
    partition: PartitionType,
    b_size: BlockSize,
) -> Result<BlockSize, TilePartitionTraversalError> {
    match partition_subsize(partition, b_size)? {
        PartitionSubsize::Valid(sub_size) => Ok(sub_size),
        PartitionSubsize::Invalid => Err(TilePartitionTraversalError::InvalidPartitionSubsize {
            partition,
            b_size: b_size.index(),
        }),
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

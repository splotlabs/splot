// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Crate-private AV2 tile-payload decode boundary planning.
//!
//! Feature tracking: `DECODE-TILE-PAYLOAD-BOUNDARY`.

#![cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "crate-private tile boundary is tested before runtime decode derives tile facts"
    )
)]

mod block_decoded_state;
mod block_symbol;
mod cdf;
mod coeff_loop;
mod coeff_state;
mod general_intra_block;
mod general_intra_residual;
mod input;
mod intra_joint_modes;
mod mi_size_state;
mod partition;
mod partition_allowed;
mod partition_size;
mod partition_traversal;
mod runtime_frontier;

use core::fmt;

use cdf::{TileCdfError, TileCdfPolicyInput, TileCdfWorkUnitBoundary, tile_cdf_save_policy};
use splot_core::headers::tile_group::{TileFramingDefect, TileGroupFraming};
use splot_core::segment::MAX_SEGMENTS;
use splot_core::span::{ByteOffset, ByteSpan};
use splot_core::symbol::{CdfUpdateMode, SymbolDecoder, SymbolDecoderConfig};
use splot_core::types::ObuType;

use crate::{
    DecodeIvfFrameContext, DecodeLayerSelection, DecodeLimitError, DecodeLimitName, DecodeLimitOp,
    DecodeLimits, DecodeObuSourceKind, UNSUPPORTED_FEATURE_RULE_ID,
};

pub(crate) use block_decoded_state::TileBlockDecodedState;
pub(crate) use block_symbol::MinimalBlockSymbolTraceError;
pub(crate) use cdf::block_context::supported_chroma_mode;
pub(crate) use cdf::block_context::{
    IntraYMode, SupportedChromaMode, SupportedDirectionalLumaMode, SupportedNonDcLumaMode,
};
pub(crate) use cdf::{FrameCdfSubset, MvCdfSelector, TileCdfSelector, TileCdfSubset};
pub(crate) use coeff_state::{CoeffContextReset, TileCoeffContextState};
pub(crate) use general_intra_block::{
    GeneralIntraBlockModeError, GeneralIntraBlockModes, GeneralIntraChromaModeContext,
    GeneralIntraChromaToolConfig, decode_general_intra_block_modes,
    decode_general_intra_chroma_block_mode, decode_general_intra_luma_block_mode,
};
pub(crate) use general_intra_residual::{
    ActiveChromaResidualPolicy, ActiveIntraIstResidualPolicy, GeneralIntraResidualError,
    IntraIstSyntax, LumaCoeffBlock, LumaTransformTypeContext, TransformToolResidualPolicy,
    decode_general_intra_plane_coeffs, reconstruct_general_intra_block,
    reconstruct_general_intra_block_rect_with_prediction,
    reconstruct_general_intra_block_with_prediction,
};
pub(crate) use input::{
    FrameCandidateCdfFacts, FrameCandidateCoeffFacts, FrameCandidateTileBoundaryError,
    FrameCandidateTileBoundaryInput, FrameCandidateTileFacts, FrameCandidateTileMalformed,
    TileGroupPositionFacts, plan_derived_tile_payload_boundary,
};
pub(crate) use intra_joint_modes::{TileFscModeState, TileIntraJointModeState, TileUsesMrlsState};
pub(crate) use partition_traversal::WienerNsLrSourceBlock;
pub(crate) use partition_traversal::{
    DecodeBlockFrontier, GeneralIntraLeafMode, GeneralIntraTreeWalkError,
};
pub(crate) use partition_traversal::{
    TilePartitionTraversalError, TilePartitionTraversalUnsupported,
};
pub(crate) use runtime_frontier::{
    GeneralIntraMultiblockError, MinimalRuntimeBlockSymbolFrontierError,
    MinimalRuntimePartitionFrontierError, MinimalRuntimeReconstructionTrace,
    consume_minimal_runtime_lr_unit_frontier, decode_general_intra_multiblock_tree,
    frame_mi_dimensions, plan_minimal_runtime_block_symbol_frontier,
};

pub(crate) const TILE_PAYLOAD_DECODE_MATRIX_ROW: &str = "tile-payload-decode";
pub(crate) const TILE_PAYLOAD_DECODE_FEATURE_ID: &str = "DECODE-TILE-PAYLOAD-BOUNDARY";

#[derive(Clone, Copy, Debug)]
pub(crate) struct TilePayloadBoundaryInput<'payload, 'facts> {
    payload: &'payload [u8],
    payload_base: ByteOffset,
    framing: &'facts TileGroupFraming,
    source: TilePayloadSource,
    selected_layer: DecodeLayerSelection,
    grid: TileGridFacts<'facts>,
    frame: TileFrameFacts,
    limits: DecodeLimits,
}

impl<'payload, 'facts> TilePayloadBoundaryInput<'payload, 'facts> {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub(crate) const fn new(
        payload: &'payload [u8],
        payload_base: ByteOffset,
        framing: &'facts TileGroupFraming,
        source: TilePayloadSource,
        selected_layer: DecodeLayerSelection,
        grid: TileGridFacts<'facts>,
        frame: TileFrameFacts,
        limits: DecodeLimits,
    ) -> Self {
        Self {
            payload,
            payload_base,
            framing,
            source,
            selected_layer,
            grid,
            frame,
            limits,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TilePayloadSource {
    source_kind: DecodeObuSourceKind,
    ivf_frame: Option<DecodeIvfFrameContext>,
    obu_index: u64,
    obu_offset: ByteOffset,
}

impl TilePayloadSource {
    #[must_use]
    pub(crate) const fn new(
        source_kind: DecodeObuSourceKind,
        ivf_frame: Option<DecodeIvfFrameContext>,
        obu_index: u64,
        obu_offset: ByteOffset,
    ) -> Self {
        Self {
            source_kind,
            ivf_frame,
            obu_index,
            obu_offset,
        }
    }

    #[must_use]
    pub(crate) const fn source_kind(self) -> DecodeObuSourceKind {
        self.source_kind
    }

    #[must_use]
    pub(crate) const fn ivf_frame(self) -> Option<DecodeIvfFrameContext> {
        self.ivf_frame
    }

    #[must_use]
    pub(crate) const fn obu_index(self) -> u64 {
        self.obu_index
    }

    #[must_use]
    pub(crate) const fn obu_offset(self) -> ByteOffset {
        self.obu_offset
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TileGridFacts<'a> {
    tile_cols: u32,
    tile_rows: u32,
    mi_col_starts: &'a [u32],
    mi_row_starts: &'a [u32],
}

impl<'a> TileGridFacts<'a> {
    #[must_use]
    pub(crate) const fn new(
        tile_cols: u32,
        tile_rows: u32,
        mi_col_starts: &'a [u32],
        mi_row_starts: &'a [u32],
    ) -> Self {
        Self {
            tile_cols,
            tile_rows,
            mi_col_starts,
            mi_row_starts,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TileFrameFacts {
    obu_type: ObuType,
    is_frame_intra: bool,
    is_complete_intra_first_tile_group: bool,
    is_last_tile_group: bool,
    is_bridge: bool,
    bru_path: TileBruPath,
    base_q_idx: u32,
    coeff_frame_facts: TileCoeffFrameFacts,
    disable_cdf_update: bool,
    cdf_policy: TileCdfPolicyInput,
}

impl TileFrameFacts {
    #[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
    #[must_use]
    pub(crate) const fn new(
        obu_type: ObuType,
        is_frame_intra: bool,
        is_complete_intra_first_tile_group: bool,
        is_last_tile_group: bool,
        is_bridge: bool,
        bru_path: TileBruPath,
        base_q_idx: u32,
        disable_cdf_update: bool,
    ) -> Self {
        Self {
            obu_type,
            is_frame_intra,
            is_complete_intra_first_tile_group,
            is_last_tile_group,
            is_bridge,
            bru_path,
            base_q_idx,
            coeff_frame_facts: TileCoeffFrameFacts::default_for_base_q(base_q_idx),
            disable_cdf_update,
            cdf_policy: TileCdfPolicyInput::single_tile_default(),
        }
    }

    #[must_use]
    pub(crate) const fn with_coeff_frame_facts(
        mut self,
        coeff_frame_facts: TileCoeffFrameFacts,
    ) -> Self {
        self.coeff_frame_facts = coeff_frame_facts;
        self
    }

    #[must_use]
    pub(crate) const fn with_cdf_policy(mut self, cdf_policy: TileCdfPolicyInput) -> Self {
        self.cdf_policy = cdf_policy;
        self
    }
}

/// Parsed frame/sequence facts used by coefficient decoding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TileCoeffFrameFacts {
    enable_fsc: bool,
    enable_idtx_intra: bool,
    enable_intra_ist: bool,
    enable_inter_ist: bool,
    enable_chroma_dctonly: bool,
    enable_cctx: bool,
    reduced_tx_set: usize,
    lossless_array: [bool; MAX_SEGMENTS],
    allow_tcq: bool,
    allow_parity_hiding: bool,
    base_q_idx: u32,
}

/// Named input for parsed frame/sequence facts used by coefficient decoding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TileCoeffFrameFactsInput {
    pub(crate) enable_fsc: bool,
    pub(crate) enable_idtx_intra: bool,
    pub(crate) enable_intra_ist: bool,
    pub(crate) enable_inter_ist: bool,
    pub(crate) enable_chroma_dctonly: bool,
    pub(crate) enable_cctx: bool,
    pub(crate) reduced_tx_set: usize,
    pub(crate) lossless_array: [bool; MAX_SEGMENTS],
    pub(crate) allow_tcq: bool,
    pub(crate) allow_parity_hiding: bool,
    pub(crate) base_q_idx: u32,
}

impl TileCoeffFrameFacts {
    #[must_use]
    pub(crate) const fn new(input: TileCoeffFrameFactsInput) -> Self {
        Self {
            enable_fsc: input.enable_fsc,
            enable_idtx_intra: input.enable_idtx_intra,
            enable_intra_ist: input.enable_intra_ist,
            enable_inter_ist: input.enable_inter_ist,
            enable_chroma_dctonly: input.enable_chroma_dctonly,
            enable_cctx: input.enable_cctx,
            reduced_tx_set: input.reduced_tx_set,
            lossless_array: input.lossless_array,
            allow_tcq: input.allow_tcq,
            allow_parity_hiding: input.allow_parity_hiding,
            base_q_idx: input.base_q_idx,
        }
    }

    const fn default_for_base_q(base_q_idx: u32) -> Self {
        Self {
            enable_fsc: false,
            enable_idtx_intra: false,
            enable_intra_ist: false,
            enable_inter_ist: false,
            enable_chroma_dctonly: false,
            enable_cctx: false,
            reduced_tx_set: 0,
            lossless_array: [false; MAX_SEGMENTS],
            allow_tcq: false,
            allow_parity_hiding: false,
            base_q_idx,
        }
    }

    #[must_use]
    pub(crate) const fn enable_fsc(self) -> bool {
        self.enable_fsc
    }

    #[must_use]
    pub(crate) const fn enable_idtx_intra(self) -> bool {
        self.enable_idtx_intra
    }

    #[must_use]
    pub(crate) const fn enable_intra_ist(self) -> bool {
        self.enable_intra_ist
    }

    #[must_use]
    pub(crate) const fn enable_inter_ist(self) -> bool {
        self.enable_inter_ist
    }

    #[must_use]
    pub(crate) const fn enable_chroma_dctonly(self) -> bool {
        self.enable_chroma_dctonly
    }

    #[must_use]
    pub(crate) const fn enable_cctx(self) -> bool {
        self.enable_cctx
    }

    #[must_use]
    pub(crate) const fn reduced_tx_set(self) -> usize {
        self.reduced_tx_set
    }

    #[must_use]
    pub(crate) const fn base_q_idx(self) -> u32 {
        self.base_q_idx
    }

    #[must_use]
    pub(crate) const fn allow_tcq(self) -> bool {
        self.allow_tcq
    }

    #[must_use]
    pub(crate) const fn allow_parity_hiding(self) -> bool {
        self.allow_parity_hiding
    }

    #[must_use]
    pub(crate) const fn lossless_for_segment(self, segment_id: usize) -> Option<bool> {
        if segment_id < self.lossless_array.len() {
            Some(self.lossless_array[segment_id])
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TileBruPath {
    NotUsed,
    Active,
    Inactive,
}

/// Deterministic plan for framed tile payload work.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DecodeTilePayloadPlan<'a> {
    source: TilePayloadSource,
    selected_layer: DecodeLayerSelection,
    work_units: Vec<DecodeTileWorkUnit<'a>>,
    unsupported: TilePayloadUnsupported,
    frame_end: FrameEndBoundary,
}

impl<'a> DecodeTilePayloadPlan<'a> {
    #[must_use]
    pub(crate) const fn source(&self) -> TilePayloadSource {
        self.source
    }

    #[must_use]
    pub(crate) const fn selected_layer(&self) -> DecodeLayerSelection {
        self.selected_layer
    }

    #[must_use]
    pub(crate) fn work_units(&self) -> &[DecodeTileWorkUnit<'a>] {
        &self.work_units
    }

    pub(crate) fn work_units_mut(&mut self) -> &mut [DecodeTileWorkUnit<'a>] {
        &mut self.work_units
    }

    #[must_use]
    pub(crate) const fn unsupported(&self) -> TilePayloadUnsupported {
        self.unsupported
    }

    #[must_use]
    pub(crate) const fn frame_end(&self) -> FrameEndBoundary {
        self.frame_end
    }
}

/// One deterministic tile work unit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DecodeTileWorkUnit<'a> {
    source: TilePayloadSource,
    selected_layer: DecodeLayerSelection,
    tile_num: u32,
    tile_row: u32,
    tile_col: u32,
    mi_row_range: core::ops::Range<u32>,
    mi_col_range: core::ops::Range<u32>,
    tile_bytes: &'a [u8],
    tile_byte_span: ByteSpan,
    tile_size: u64,
    current_q_index_at_entry: u32,
    coeff_frame_facts: TileCoeffFrameFacts,
    bru_path: TileBruPath,
    symbol: SymbolInitBoundary,
    cdf: TileCdfWorkUnitBoundary,
}

impl<'a> DecodeTileWorkUnit<'a> {
    #[must_use]
    pub(crate) const fn source(&self) -> TilePayloadSource {
        self.source
    }

    #[must_use]
    pub(crate) const fn selected_layer(&self) -> DecodeLayerSelection {
        self.selected_layer
    }

    #[must_use]
    pub(crate) const fn tile_num(&self) -> u32 {
        self.tile_num
    }

    #[must_use]
    pub(crate) const fn tile_row(&self) -> u32 {
        self.tile_row
    }

    #[must_use]
    pub(crate) const fn tile_col(&self) -> u32 {
        self.tile_col
    }

    #[must_use]
    pub(crate) fn mi_row_range(&self) -> core::ops::Range<u32> {
        self.mi_row_range.clone()
    }

    #[must_use]
    pub(crate) fn mi_col_range(&self) -> core::ops::Range<u32> {
        self.mi_col_range.clone()
    }

    #[must_use]
    pub(crate) const fn tile_bytes(&self) -> &'a [u8] {
        self.tile_bytes
    }

    #[must_use]
    pub(crate) const fn tile_byte_span(&self) -> ByteSpan {
        self.tile_byte_span
    }

    #[must_use]
    pub(crate) const fn tile_size(&self) -> u64 {
        self.tile_size
    }

    #[must_use]
    pub(crate) const fn current_q_index_at_entry(&self) -> u32 {
        self.current_q_index_at_entry
    }

    #[must_use]
    pub(crate) const fn coeff_frame_facts(&self) -> TileCoeffFrameFacts {
        self.coeff_frame_facts
    }

    #[must_use]
    pub(crate) const fn symbol(&self) -> SymbolInitBoundary {
        self.symbol
    }

    #[must_use]
    pub(crate) const fn cdf(&self) -> &TileCdfWorkUnitBoundary {
        &self.cdf
    }

    pub(crate) fn cdf_mut(&mut self) -> &mut TileCdfWorkUnitBoundary {
        &mut self.cdf
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SymbolInitBoundary {
    consumed_bits: u64,
    symbol_max_bits: i64,
    cdf_update_mode: CdfUpdateMode,
}

impl SymbolInitBoundary {
    #[must_use]
    pub(crate) const fn consumed_bits(self) -> u64 {
        self.consumed_bits
    }

    #[must_use]
    pub(crate) const fn symbol_max_bits(self) -> i64 {
        self.symbol_max_bits
    }

    #[must_use]
    pub(crate) const fn cdf_update_mode(self) -> CdfUpdateMode {
        self.cdf_update_mode
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FrameEndBoundary {
    reaches_last_tile_group: bool,
    frame_end_update_cdf_deferred: bool,
    decode_frame_wrapup_deferred: bool,
}

impl FrameEndBoundary {
    const fn deferred(reaches_last_tile_group: bool) -> Self {
        Self {
            reaches_last_tile_group,
            frame_end_update_cdf_deferred: reaches_last_tile_group,
            decode_frame_wrapup_deferred: reaches_last_tile_group,
        }
    }

    #[must_use]
    pub(crate) const fn reaches_last_tile_group(self) -> bool {
        self.reaches_last_tile_group
    }

    #[must_use]
    pub(crate) const fn frame_end_update_cdf_deferred(self) -> bool {
        self.frame_end_update_cdf_deferred
    }

    #[must_use]
    pub(crate) const fn decode_frame_wrapup_deferred(self) -> bool {
        self.decode_frame_wrapup_deferred
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TilePayloadUnsupportedReason {
    DecodeTileSyntax,
    MissingCompleteIntraFirstTileGroup,
    NonSingleTile,
    MultipleTiles,
    MultipleTileGroups,
    NonClosedLoopKey,
    NonIntraFrame,
    BridgeTile,
    BruTileActivity,
    InvalidTileGrid,
}

crate::impl_reason_labels!(pub(crate) TilePayloadUnsupportedReason {
    DecodeTileSyntax => "decode_tile_syntax",
    MissingCompleteIntraFirstTileGroup => "missing_complete_intra_first_tile_group",
    NonSingleTile => "non_single_tile",
    MultipleTiles => "multiple_tiles",
    MultipleTileGroups => "multiple_tile_groups",
    NonClosedLoopKey => "non_closed_loop_key",
    NonIntraFrame => "non_intra_frame",
    BridgeTile => "bridge_tile",
    BruTileActivity => "bru_tile_activity",
    InvalidTileGrid => "invalid_tile_grid",
});

impl TilePayloadUnsupportedReason {
    #[must_use]
    pub(crate) const fn spec_section(self) -> &'static str {
        match self {
            Self::DecodeTileSyntax => "5.20.2.1",
            Self::MissingCompleteIntraFirstTileGroup
            | Self::NonSingleTile
            | Self::MultipleTiles
            | Self::MultipleTileGroups
            | Self::BridgeTile
            | Self::BruTileActivity => "5.20.1",
            Self::NonClosedLoopKey | Self::NonIntraFrame => "7.1",
            Self::InvalidTileGrid => "6.19.1",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TilePayloadUnsupported {
    reason: TilePayloadUnsupportedReason,
    tile_num: Option<u32>,
    byte_offset: ByteOffset,
    spec_section: &'static str,
    matrix_row: &'static str,
    feature_id: &'static str,
    message: &'static str,
}

impl TilePayloadUnsupported {
    fn new(
        reason: TilePayloadUnsupportedReason,
        tile_num: Option<u32>,
        byte_offset: ByteOffset,
        message: &'static str,
    ) -> Self {
        Self {
            reason,
            tile_num,
            byte_offset,
            spec_section: reason.spec_section(),
            matrix_row: TILE_PAYLOAD_DECODE_MATRIX_ROW,
            feature_id: TILE_PAYLOAD_DECODE_FEATURE_ID,
            message,
        }
    }

    #[allow(clippy::unused_self)]
    #[must_use]
    pub(crate) const fn rule_id(self) -> &'static str {
        UNSUPPORTED_FEATURE_RULE_ID
    }

    #[must_use]
    pub(crate) const fn matrix_row(self) -> &'static str {
        self.matrix_row
    }

    #[must_use]
    pub(crate) const fn feature_id(self) -> &'static str {
        self.feature_id
    }

    #[must_use]
    pub(crate) const fn spec_section(self) -> &'static str {
        self.spec_section
    }

    #[must_use]
    pub(crate) const fn reason(self) -> TilePayloadUnsupportedReason {
        self.reason
    }

    #[must_use]
    pub(crate) const fn tile_num(self) -> Option<u32> {
        self.tile_num
    }

    #[must_use]
    pub(crate) const fn byte_offset(self) -> ByteOffset {
        self.byte_offset
    }

    #[must_use]
    pub(crate) const fn message(self) -> &'static str {
        self.message
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TilePayloadMalformed {
    FramingDefect(TileFramingDefect),
    TileRangeOutOfBounds {
        tile_num: u32,
        tile_data_offset: u64,
        tile_size: u64,
        payload_len: u64,
    },
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum TilePayloadBoundaryError {
    #[error("tile payload boundary rejected by resource limit: {0}")]
    Limit(#[from] DecodeLimitError),
    #[error("malformed tile payload boundary: {0}")]
    Malformed(TilePayloadMalformed),
    #[error("unsupported tile payload boundary: {0}")]
    Unsupported(TilePayloadUnsupported),
    #[error("tile CDF boundary failed: {0}")]
    Cdf(#[from] TileCdfError),
    #[error("tile payload symbol initialization failed: {0}")]
    Symbol(#[from] splot_core::Error),
}

impl fmt::Display for TilePayloadMalformed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FramingDefect(defect) => {
                write!(f, "tile framing defect {}", defect.label())
            }
            Self::TileRangeOutOfBounds {
                tile_num,
                tile_data_offset,
                tile_size,
                payload_len,
            } => write!(
                f,
                "tile {tile_num} byte range [{tile_data_offset}, +{tile_size}) exceeds payload length {payload_len}"
            ),
        }
    }
}

impl fmt::Display for TilePayloadUnsupported {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.tile_num {
            Some(tile_num) => write!(
                f,
                "{} at tile {tile_num} byte {}: {}",
                self.reason.as_str(),
                self.byte_offset,
                self.message
            ),
            None => write!(
                f,
                "{} at byte {}: {}",
                self.reason.as_str(),
                self.byte_offset,
                self.message
            ),
        }
    }
}

/// Plans the minimal tile-payload boundary and stops at `decode_tile()`.
pub(crate) fn plan_tile_payload_boundary<'a>(
    input: &TilePayloadBoundaryInput<'a, '_>,
) -> Result<DecodeTilePayloadPlan<'a>, TilePayloadBoundaryError> {
    input.limits.ensure_mul(
        DecodeLimitName::MaxTileCount,
        u64::from(input.grid.tile_cols),
        u64::from(input.grid.tile_rows),
    )?;
    for tile in &input.framing.tiles {
        input
            .limits
            .ensure(DecodeLimitName::MaxTilePayloadBytes, tile.tile_size)?;
    }

    if let Some(defect) = input.framing.defect {
        return Err(TilePayloadBoundaryError::Malformed(
            TilePayloadMalformed::FramingDefect(defect),
        ));
    }
    if input.frame.is_bridge {
        return Err(unsupported_boundary_without_tile(
            TilePayloadUnsupportedReason::BridgeTile,
            input.payload_base,
            "bridge tile payload behavior is outside the current tile payload boundary tier.",
        ));
    }
    match (input.frame.obu_type, input.frame.is_frame_intra) {
        (ObuType::ClosedLoopKey, true) | (ObuType::RegularTileGroup, false) => {}
        (ObuType::RegularTileGroup, true) | (_, false) => {
            return Err(unsupported_boundary_without_tile(
                TilePayloadUnsupportedReason::NonIntraFrame,
                input.payload_base,
                "inter-only tile payload behavior is outside the current tile payload boundary tier.",
            ));
        }
        _ => {
            return Err(unsupported_boundary_without_tile(
                TilePayloadUnsupportedReason::NonClosedLoopKey,
                input.payload_base,
                "only OBU_CLOSED_LOOP_KEY (intra) or OBU_REGULAR_TILE_GROUP (inter) is inside the current tile payload boundary tier.",
            ));
        }
    }
    if !input.frame.is_complete_intra_first_tile_group {
        return Err(unsupported_boundary_without_tile(
            TilePayloadUnsupportedReason::MissingCompleteIntraFirstTileGroup,
            input.payload_base,
            "tile payload planning requires a complete first tile group in the minimal tier.",
        ));
    }
    if !input.frame.is_last_tile_group {
        return Err(unsupported_boundary_without_tile(
            TilePayloadUnsupportedReason::MultipleTileGroups,
            input.payload_base,
            "multiple tile groups are outside the current tile payload boundary tier.",
        ));
    }
    if input.frame.bru_path != TileBruPath::NotUsed {
        return Err(unsupported_boundary_without_tile(
            TilePayloadUnsupportedReason::BruTileActivity,
            input.payload_base,
            "BRU tile activity is outside the current tile payload boundary tier.",
        ));
    }
    if input.framing.tiles.len() != 1 {
        return Err(unsupported_boundary_without_tile(
            TilePayloadUnsupportedReason::NonSingleTile,
            input.payload_base,
            "tile groups without exactly one tile are outside the current tile payload boundary tier.",
        ));
    }

    let tile = input.framing.tiles[0];
    if tile.tile_num != 0 || input.grid.tile_cols != 1 || input.grid.tile_rows != 1 {
        return Err(unsupported_boundary_with_tile(
            TilePayloadUnsupportedReason::MultipleTiles,
            tile.tile_num,
            input.payload_base,
            tile.tile_data_offset,
            "only TileNum 0 in a one-tile frame is inside the current tile payload boundary tier.",
        )?);
    }

    let (tile_row, tile_col, mi_row_range, mi_col_range) = grid_ranges(
        input.grid,
        tile.tile_num,
        input.payload_base,
        tile.tile_data_offset,
    )?;
    let tile_bytes = tile_slice(
        input.payload,
        tile.tile_num,
        tile.tile_data_offset,
        tile.tile_size,
    )?;
    let absolute_tile_offset = checked_tile_byte_offset(input.payload_base, tile.tile_data_offset)?;
    let tile_byte_span = checked_tile_byte_span(absolute_tile_offset, tile.tile_size)?;
    let cdf_update_mode = if input.frame.disable_cdf_update {
        CdfUpdateMode::Disabled
    } else {
        CdfUpdateMode::Enabled
    };
    let config = SymbolDecoderConfig::new().with_cdf_update_mode(cdf_update_mode);
    let symbol = SymbolDecoder::with_base_and_config(tile_bytes, absolute_tile_offset, config)?;
    let symbol = SymbolInitBoundary {
        consumed_bits: symbol.consumed_bits().get(),
        symbol_max_bits: symbol.symbol_max_bits(),
        cdf_update_mode,
    };
    let frame_cdfs = FrameCdfSubset::from_defaults();
    let cdf_policy = input
        .frame
        .cdf_policy
        .with_tile_grid(input.grid.tile_cols, input.grid.tile_rows);
    let save_policy = tile_cdf_save_policy(cdf_policy, tile.tile_num)?;
    let cdf = TileCdfWorkUnitBoundary::new(cdf_update_mode, save_policy, frame_cdfs);
    let work_unit = DecodeTileWorkUnit {
        source: input.source,
        selected_layer: input.selected_layer,
        tile_num: tile.tile_num,
        tile_row,
        tile_col,
        mi_row_range,
        mi_col_range,
        tile_bytes,
        tile_byte_span,
        tile_size: tile.tile_size,
        current_q_index_at_entry: input.frame.base_q_idx,
        coeff_frame_facts: input.frame.coeff_frame_facts,
        bru_path: input.frame.bru_path,
        symbol,
        cdf,
    };
    let unsupported = TilePayloadUnsupported::new(
        TilePayloadUnsupportedReason::DecodeTileSyntax,
        Some(tile.tile_num),
        absolute_tile_offset,
        "tile bytes are framed, symbol initialization is bounded, and the first partition CDF subset is selectable, but §5.20.2.1 decode_tile() block syntax is not implemented yet.",
    );

    Ok(DecodeTilePayloadPlan {
        source: input.source,
        selected_layer: input.selected_layer,
        work_units: vec![work_unit],
        unsupported,
        frame_end: FrameEndBoundary::deferred(input.frame.is_last_tile_group),
    })
}

fn tile_slice(
    payload: &[u8],
    tile_num: u32,
    tile_data_offset: u64,
    tile_size: u64,
) -> Result<&[u8], TilePayloadBoundaryError> {
    let end =
        tile_data_offset
            .checked_add(tile_size)
            .ok_or(DecodeLimitError::ArithmeticOverflow {
                name: DecodeLimitName::MaxTilePayloadBytes,
                op: DecodeLimitOp::Add,
                left: tile_data_offset,
                right: tile_size,
            })?;
    if end > payload.len() as u64 {
        return Err(TilePayloadBoundaryError::Malformed(
            TilePayloadMalformed::TileRangeOutOfBounds {
                tile_num,
                tile_data_offset,
                tile_size,
                payload_len: payload.len() as u64,
            },
        ));
    }
    let start = tile_data_offset as usize;
    let end = end as usize;
    Ok(&payload[start..end])
}

fn grid_ranges(
    grid: TileGridFacts<'_>,
    tile_num: u32,
    payload_base: ByteOffset,
    tile_data_offset: u64,
) -> Result<(u32, u32, core::ops::Range<u32>, core::ops::Range<u32>), TilePayloadBoundaryError> {
    if grid.tile_cols == 0 || grid.tile_rows == 0 {
        return Err(invalid_grid(tile_num, payload_base, tile_data_offset));
    }
    let tile_count =
        grid.tile_cols
            .checked_mul(grid.tile_rows)
            .ok_or(DecodeLimitError::ArithmeticOverflow {
                name: DecodeLimitName::MaxTileCount,
                op: DecodeLimitOp::Mul,
                left: u64::from(grid.tile_cols),
                right: u64::from(grid.tile_rows),
            })?;
    if tile_num >= tile_count {
        return Err(invalid_grid(tile_num, payload_base, tile_data_offset));
    }
    let tile_row = tile_num / grid.tile_cols;
    let tile_col = tile_num % grid.tile_cols;
    let row = tile_row as usize;
    let col = tile_col as usize;
    let Some((&mi_row_start, &mi_row_end)) = grid
        .mi_row_starts
        .get(row)
        .zip(grid.mi_row_starts.get(row + 1))
    else {
        return Err(invalid_grid(tile_num, payload_base, tile_data_offset));
    };
    let Some((&mi_col_start, &mi_col_end)) = grid
        .mi_col_starts
        .get(col)
        .zip(grid.mi_col_starts.get(col + 1))
    else {
        return Err(invalid_grid(tile_num, payload_base, tile_data_offset));
    };
    if mi_row_start >= mi_row_end || mi_col_start >= mi_col_end {
        return Err(invalid_grid(tile_num, payload_base, tile_data_offset));
    }
    Ok((
        tile_row,
        tile_col,
        mi_row_start..mi_row_end,
        mi_col_start..mi_col_end,
    ))
}

fn invalid_grid(
    tile_num: u32,
    payload_base: ByteOffset,
    tile_data_offset: u64,
) -> TilePayloadBoundaryError {
    unsupported_boundary_with_tile(
        TilePayloadUnsupportedReason::InvalidTileGrid,
        tile_num,
        payload_base,
        tile_data_offset,
        "tile grid facts do not cover the framed tile number.",
    )
    .unwrap_or_else(|_| {
        unsupported_boundary_without_tile(
            TilePayloadUnsupportedReason::InvalidTileGrid,
            payload_base,
            "tile grid facts do not cover the framed tile number.",
        )
    })
}

fn unsupported_boundary_without_tile(
    reason: TilePayloadUnsupportedReason,
    byte_offset: ByteOffset,
    message: &'static str,
) -> TilePayloadBoundaryError {
    TilePayloadBoundaryError::Unsupported(unsupported_without_tile(reason, byte_offset, message))
}

fn unsupported_boundary_with_tile(
    reason: TilePayloadUnsupportedReason,
    tile_num: u32,
    payload_base: ByteOffset,
    tile_data_offset: u64,
    message: &'static str,
) -> Result<TilePayloadBoundaryError, DecodeLimitError> {
    Ok(TilePayloadBoundaryError::Unsupported(unsupported_tile(
        reason,
        tile_num,
        payload_base,
        tile_data_offset,
        message,
    )?))
}

fn unsupported_without_tile(
    reason: TilePayloadUnsupportedReason,
    byte_offset: ByteOffset,
    message: &'static str,
) -> TilePayloadUnsupported {
    TilePayloadUnsupported::new(reason, None, byte_offset, message)
}

fn unsupported_tile(
    reason: TilePayloadUnsupportedReason,
    tile_num: u32,
    payload_base: ByteOffset,
    tile_data_offset: u64,
    message: &'static str,
) -> Result<TilePayloadUnsupported, DecodeLimitError> {
    let byte_offset = checked_tile_byte_offset(payload_base, tile_data_offset)?;
    Ok(TilePayloadUnsupported::new(
        reason,
        Some(tile_num),
        byte_offset,
        message,
    ))
}

fn checked_tile_byte_offset(base: ByteOffset, delta: u64) -> Result<ByteOffset, DecodeLimitError> {
    checked_byte_offset(base, delta, DecodeLimitName::MaxTilePayloadBytes)
}

fn checked_tile_byte_span(start: ByteOffset, len: u64) -> Result<ByteSpan, DecodeLimitError> {
    checked_byte_span(start, len, DecodeLimitName::MaxTilePayloadBytes)
}

fn checked_byte_offset(
    base: ByteOffset,
    delta: u64,
    name: DecodeLimitName,
) -> Result<ByteOffset, DecodeLimitError> {
    let offset = base
        .get()
        .checked_add(delta)
        .ok_or(DecodeLimitError::ArithmeticOverflow {
            name,
            op: DecodeLimitOp::Add,
            left: base.get(),
            right: delta,
        })?;
    Ok(ByteOffset::new(offset))
}

fn checked_byte_span(
    start: ByteOffset,
    len: u64,
    name: DecodeLimitName,
) -> Result<ByteSpan, DecodeLimitError> {
    let _end = checked_byte_offset(start, len, name)?;
    Ok(ByteSpan::new(start, len))
}

#[cfg(test)]
mod derived_tests;

#[cfg(test)]
mod tests;

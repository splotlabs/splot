// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Crate-private AV2 tile-payload decode boundary planning.
//!
//! Feature tracking: `DECODE-TILE-PAYLOAD-BOUNDARY`.

mod block_decoded_state;
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
#[cfg(test)]
#[path = "tile_payload/test_support_tests.rs"]
mod test_support;
mod tile_frontier;

use core::fmt;
use std::sync::Arc;

use cdf::{TileCdfError, TileCdfPolicyInput, TileCdfWorkUnitBoundary, tile_cdf_save_policy};
use splot_core::headers::tile_group::{TileFramingDefect, TileGroupFraming};
use splot_core::segment::MAX_SEGMENTS;
use splot_core::span::{ByteOffset, ByteSpan};
use splot_core::symbol::{CdfUpdateMode, CdfValidationMode, SymbolDecoder, SymbolDecoderConfig};
use splot_core::types::ObuType;

use crate::{DecodeLimitError, DecodeLimitName, DecodeLimitOp, DecodeLimits};

pub(crate) use block_decoded_state::TileBlockDecodedState;
pub(crate) use cdf::block_context::{
    IntraYMode, SupportedChromaMode, SupportedDirectionalLumaMode, SupportedNonDcLumaMode,
};
pub(crate) use cdf::block_read::BlockSymbolTraceReadError;
pub(crate) use cdf::{
    COMPOUND_MODE_NON_JOINT_CDF_ROW_LEN, COMPOUND_MODE_SAME_REFS_CDF_ROW_LEN, FrameCdfSubset,
    MvCdfSelector, SavedCdfSubset, TileCdfSelector, TileCdfSubset,
};
pub(crate) use coeff_state::{CoeffContextReset, TileCoeffContextState};
#[cfg(test)]
pub(crate) use general_intra_block::GeneralIntraLumaBlockMode;
pub(crate) use general_intra_block::{
    CflIndex, CflParams, GeneralIntraBlockModeError, GeneralIntraBlockModes,
    GeneralIntraChromaBlockMode, GeneralIntraChromaModeContext, GeneralIntraChromaToolConfig,
    decode_general_intra_block_modes_with_fsc_context, decode_general_intra_chroma_block_mode,
    decode_general_intra_luma_block_mode_with_fsc_context, read_general_intra_dip_mode_info,
    read_general_intra_palette_y_mode, read_lossless_luma_tx_size, read_lossless_tx_size,
};
pub(crate) use general_intra_residual::{
    FrameQmScope, FrameQmSegmentScope, FrameQuantizerDeltasScope, FrameQuantizerSnapshot,
    FrameUserQmLevel, FrameUserQmLevels, FrameUserQmScope, GeneralIntraResidualError,
    IntraIstSyntax, LumaCoeffBlock, LumaTransformPartitionContext, LumaTransformPartitionUnits,
    LumaTransformTypeContext, PositionedLumaCoeffBlock, TransformPartitionUnsupported,
    TransformToolResidualPolicy, current_frame_qm_segment_id,
    decode_general_intra_luma_partition_coeffs, decode_general_intra_plane_coeffs,
    is_cctx_geometry_allowed, reconstruct_general_intra_chroma_cctx_pair_into,
    reconstruct_general_intra_chroma_cctx_pair_with_predictions,
    reconstruct_general_intra_coeff_block_rect_into_frame,
    reconstruct_general_intra_coeff_block_rect_with_prediction_into,
    reconstruct_inter_coeff_block_residual_rect_into,
};
pub(crate) use input::{
    FrameCandidateCdfFacts, FrameCandidateCoeffFacts, FrameCandidateTileBoundaryError,
    FrameCandidateTileBoundaryInput, FrameCandidateTileFacts, FrameCandidateTileMalformed,
    TileGroupPositionFacts, plan_derived_tile_payload_boundary,
};
pub(crate) use intra_joint_modes::IsCflContext;
pub(crate) use intra_joint_modes::{
    FrameSegmentIdMap, LumaPalette, TileFscModeState, TileIntraJointModeState,
    TileLumaPaletteState, TileSegmentIdState, TileUseDipState, TileUsesMrlsState, neg_deinterleave,
};
pub(crate) use partition_allowed::get_plane_residual_size;
pub(crate) use partition_size::BlockSize;
pub(crate) use partition_traversal::LrUnitRestorationType;
#[cfg(test)]
pub(crate) use partition_traversal::tests::make_work_unit as make_test_work_unit;
pub(crate) use partition_traversal::{
    DecodeBlockFrontier, DecodedLeafPublication, GeneralIntraLeafMode, GeneralIntraTreeWalkError,
};
pub(crate) use partition_traversal::{
    TilePartitionTraversalError, WienerNsLrSourceBlock, WienerNsLrUnitFilter,
};
#[cfg(test)]
pub(crate) use test_support::encode_symbol_sequence;
pub(crate) use tile_frontier::{
    GeneralIntraMultiblockCursor, GeneralIntraMultiblockError, GeneralIntraMultiblockOutput,
    chroma_subsampling, frame_mi_dimensions,
};

#[derive(Clone, Debug)]
pub(crate) struct TilePayloadBoundaryInput<'payload, 'facts> {
    payload: &'payload [u8],
    payload_base: ByteOffset,
    framing: &'facts TileGroupFraming,
    grid: TileGridFacts<'facts>,
    frame: TileFrameFacts,
    limits: DecodeLimits,
}

impl<'payload, 'facts> TilePayloadBoundaryInput<'payload, 'facts> {
    #[must_use]
    pub(crate) const fn new(
        payload: &'payload [u8],
        payload_base: ByteOffset,
        framing: &'facts TileGroupFraming,
        grid: TileGridFacts<'facts>,
        frame: TileFrameFacts,
        limits: DecodeLimits,
    ) -> Self {
        Self {
            payload,
            payload_base,
            framing,
            grid,
            frame,
            limits,
        }
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TileFrameFacts {
    obu_type: ObuType,
    is_frame_intra: bool,
    is_last_tile_group: bool,
    base_q_idx: u32,
    coeff_frame_facts: TileCoeffFrameFacts,
    disable_cdf_update: bool,
    cdf_policy: TileCdfPolicyInput,
    initial_cdfs: Option<Arc<FrameCdfSubset>>,
}

impl TileFrameFacts {
    #[must_use]
    pub(crate) const fn new(
        obu_type: ObuType,
        is_frame_intra: bool,
        is_last_tile_group: bool,
        base_q_idx: u32,
        disable_cdf_update: bool,
    ) -> Self {
        Self {
            obu_type,
            is_frame_intra,
            is_last_tile_group,
            base_q_idx,
            coeff_frame_facts: TileCoeffFrameFacts::default_for_base_q(base_q_idx),
            disable_cdf_update,
            cdf_policy: TileCdfPolicyInput::single_tile_default(),
            initial_cdfs: None,
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

    #[must_use]
    pub(crate) fn with_initial_cdfs(mut self, initial_cdfs: Arc<FrameCdfSubset>) -> Self {
        self.initial_cdfs = Some(initial_cdfs);
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TileCoeffFrameFacts {
    enable_fsc: bool,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TileCoeffFrameFactsInput {
    pub(crate) enable_fsc: bool,
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DecodeTilePayloadPlan<'a> {
    work_units: Vec<DecodeTileWorkUnit<'a>>,
    frame_end: FrameEndBoundary,
}

impl<'a> DecodeTilePayloadPlan<'a> {
    #[must_use]
    pub(crate) fn work_units(&self) -> &[DecodeTileWorkUnit<'a>] {
        &self.work_units
    }

    pub(crate) fn work_units_mut(&mut self) -> &mut [DecodeTileWorkUnit<'a>] {
        &mut self.work_units
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) const fn frame_end(&self) -> FrameEndBoundary {
        self.frame_end
    }

    pub(crate) fn append_continuation(
        &mut self,
        mut continuation: DecodeTilePayloadPlan<'a>,
    ) -> Result<(), TilePayloadBoundaryError> {
        let expected = self
            .work_units
            .last()
            .and_then(|tile| tile.tile_num.checked_add(1));
        let actual = continuation.work_units.first().map(|tile| tile.tile_num);
        if self.frame_end.reaches_last_tile_group || expected != actual {
            return Err(TilePayloadBoundaryError::Malformed(
                TilePayloadMalformed::NonContiguousTileGroups { expected, actual },
            ));
        }
        self.work_units.append(&mut continuation.work_units);
        self.frame_end = continuation.frame_end;
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DecodeTileWorkUnit<'a> {
    tile_num: u32,
    tile_row: u32,
    tile_col: u32,
    mi_row_range: core::ops::Range<u32>,
    mi_col_range: core::ops::Range<u32>,
    tile_bytes: &'a [u8],
    tile_byte_span: ByteSpan,
    tile_size: u64,
    coeff_frame_facts: TileCoeffFrameFacts,
    symbol: SymbolInitBoundary,
    cdf: TileCdfWorkUnitBoundary,
}

impl<'a> DecodeTileWorkUnit<'a> {
    #[must_use]
    pub(crate) const fn tile_num(&self) -> u32 {
        self.tile_num
    }

    #[must_use]
    #[cfg(any(test, feature = "fuzzing"))]
    pub(crate) const fn tile_row(&self) -> u32 {
        self.tile_row
    }

    #[must_use]
    #[cfg(any(test, feature = "fuzzing"))]
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
    pub(crate) const fn coeff_frame_facts(&self) -> TileCoeffFrameFacts {
        self.coeff_frame_facts
    }

    #[must_use]
    #[cfg(any(test, feature = "fuzzing"))]
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

    pub(crate) fn frame_cdfs(&self) -> Arc<FrameCdfSubset> {
        self.cdf.frame_cdfs_shared()
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
    #[cfg(any(test, feature = "fuzzing"))]
    pub(crate) const fn consumed_bits(self) -> u64 {
        self.consumed_bits
    }

    #[must_use]
    #[cfg(any(test, feature = "fuzzing"))]
    pub(crate) const fn symbol_max_bits(self) -> i64 {
        self.symbol_max_bits
    }

    #[must_use]
    #[cfg(any(test, feature = "fuzzing"))]
    pub(crate) const fn cdf_update_mode(self) -> CdfUpdateMode {
        self.cdf_update_mode
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FrameEndBoundary {
    reaches_last_tile_group: bool,
}

impl FrameEndBoundary {
    const fn deferred(reaches_last_tile_group: bool) -> Self {
        Self {
            reaches_last_tile_group,
        }
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) const fn reaches_last_tile_group(self) -> bool {
        self.reaches_last_tile_group
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TilePayloadUnsupportedReason {
    MissingTileFramingRecords,
    NonClosedLoopKey,
    NonIntraFrame,
}

crate::impl_reason_labels!(pub(crate) TilePayloadUnsupportedReason {
    MissingTileFramingRecords => "missing_tile_framing_records",
    NonClosedLoopKey => "non_closed_loop_key",
    NonIntraFrame => "non_intra_frame",
});

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TilePayloadUnsupported {
    reason: TilePayloadUnsupportedReason,
    tile_num: Option<u32>,
    byte_offset: ByteOffset,
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
            message,
        }
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) const fn reason(self) -> TilePayloadUnsupportedReason {
        self.reason
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) const fn tile_num(self) -> Option<u32> {
        self.tile_num
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) const fn byte_offset(self) -> ByteOffset {
        self.byte_offset
    }

    #[must_use]
    #[cfg(test)]
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
    InvalidTileGrid {
        tile_num: u32,
    },
    NonContiguousTileGroups {
        expected: Option<u32>,
        actual: Option<u32>,
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
            Self::InvalidTileGrid { tile_num } => {
                write!(f, "tile grid facts do not cover framed tile {tile_num}")
            }
            Self::NonContiguousTileGroups { expected, actual } => write!(
                f,
                "tile-group continuation starts at {actual:?}, expected {expected:?}"
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
    match (input.frame.obu_type, input.frame.is_frame_intra) {
        (ObuType::ClosedLoopKey | ObuType::OpenLoopKey, true)
        | (
            ObuType::LeadingTileGroup
            | ObuType::RegularTileGroup
            | ObuType::Switch
            | ObuType::RasFrame,
            false,
        ) => {}
        (
            ObuType::LeadingTileGroup
            | ObuType::RegularTileGroup
            | ObuType::Switch
            | ObuType::RasFrame,
            true,
        )
        | (ObuType::ClosedLoopKey | ObuType::OpenLoopKey, false) => {
            return Err(unsupported_boundary_without_tile(
                TilePayloadUnsupportedReason::NonIntraFrame,
                input.payload_base,
                "the tile payload frame type does not match the OBU frame family.",
            ));
        }
        _ => {
            return Err(unsupported_boundary_without_tile(
                TilePayloadUnsupportedReason::NonClosedLoopKey,
                input.payload_base,
                "the OBU type is not a supported intra or inter tile-group frame.",
            ));
        }
    }
    if input.framing.tiles.is_empty() {
        return Err(unsupported_boundary_without_tile(
            TilePayloadUnsupportedReason::MissingTileFramingRecords,
            input.payload_base,
            "tile groups without tile framing records are outside the current tile payload boundary tier.",
        ));
    }

    let cdf_update_mode = if input.frame.disable_cdf_update {
        CdfUpdateMode::Disabled
    } else {
        CdfUpdateMode::Enabled
    };
    let frame_cdfs = match &input.frame.initial_cdfs {
        Some(cdfs) => Arc::clone(cdfs),
        None => Arc::new(FrameCdfSubset::default_for_base_q(input.frame.base_q_idx)),
    };
    let cdf_policy = input
        .frame
        .cdf_policy
        .with_tile_grid(input.grid.tile_cols, input.grid.tile_rows);
    let mut work_units = Vec::with_capacity(input.framing.tiles.len());
    for tile in &input.framing.tiles {
        let (tile_row, tile_col, mi_row_range, mi_col_range) =
            grid_ranges(input.grid, tile.tile_num)?;
        let tile_bytes = tile_slice(
            input.payload,
            tile.tile_num,
            tile.tile_data_offset,
            tile.tile_size,
        )?;
        let absolute_tile_offset =
            checked_tile_byte_offset(input.payload_base, tile.tile_data_offset)?;
        let tile_byte_span = checked_tile_byte_span(absolute_tile_offset, tile.tile_size)?;
        let config = SymbolDecoderConfig::new()
            .with_cdf_update_mode(cdf_update_mode)
            .with_cdf_validation_mode(CdfValidationMode::Trusted);
        let symbol = SymbolDecoder::with_base_and_config(tile_bytes, absolute_tile_offset, config)?;
        let symbol = SymbolInitBoundary {
            consumed_bits: symbol.consumed_bits().get(),
            symbol_max_bits: symbol.symbol_max_bits(),
            cdf_update_mode,
        };
        let save_policy = tile_cdf_save_policy(cdf_policy, tile.tile_num)?;
        let cdf =
            TileCdfWorkUnitBoundary::new(cdf_update_mode, save_policy, Arc::clone(&frame_cdfs));
        work_units.push(DecodeTileWorkUnit {
            tile_num: tile.tile_num,
            tile_row,
            tile_col,
            mi_row_range,
            mi_col_range,
            tile_bytes,
            tile_byte_span,
            tile_size: tile.tile_size,
            coeff_frame_facts: input.frame.coeff_frame_facts,
            symbol,
            cdf,
        });
    }
    Ok(DecodeTilePayloadPlan {
        work_units,
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
) -> Result<(u32, u32, core::ops::Range<u32>, core::ops::Range<u32>), TilePayloadBoundaryError> {
    if grid.tile_cols == 0 || grid.tile_rows == 0 {
        return Err(invalid_grid(tile_num));
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
        return Err(invalid_grid(tile_num));
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
        return Err(invalid_grid(tile_num));
    };
    let Some((&mi_col_start, &mi_col_end)) = grid
        .mi_col_starts
        .get(col)
        .zip(grid.mi_col_starts.get(col + 1))
    else {
        return Err(invalid_grid(tile_num));
    };
    if mi_row_start >= mi_row_end || mi_col_start >= mi_col_end {
        return Err(invalid_grid(tile_num));
    }
    Ok((
        tile_row,
        tile_col,
        mi_row_start..mi_row_end,
        mi_col_start..mi_col_end,
    ))
}

fn invalid_grid(tile_num: u32) -> TilePayloadBoundaryError {
    TilePayloadBoundaryError::Malformed(TilePayloadMalformed::InvalidTileGrid { tile_num })
}

fn unsupported_boundary_without_tile(
    reason: TilePayloadUnsupportedReason,
    byte_offset: ByteOffset,
    message: &'static str,
) -> TilePayloadBoundaryError {
    TilePayloadBoundaryError::Unsupported(TilePayloadUnsupported::new(
        reason,
        None,
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
    checked_byte_offset(start, len, name)?;
    Ok(ByteSpan::new(start, len))
}

#[cfg(test)]
mod derived_tests;

#[cfg(test)]
mod tests;

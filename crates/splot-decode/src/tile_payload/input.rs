// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Crate-private derivation of tile-payload boundary input from parser facts.
//!
//! Feature tracking: `DECODE-TILE-PAYLOAD-INPUT-DERIVATION`.

use core::fmt;

use splot_core::Error;
use splot_core::annexb::ObuEnvelope;
use splot_core::bitio::BitReader;
use splot_core::headers::frame::{FrameHeaderCore, FrameHeaderParseStatus};
use splot_core::headers::sequence::SequenceTqEntropyConfig;
use splot_core::headers::tile_group::{
    TileGroupLayout, TileGroupStructure, TileGroupStructureOutcome, parse_tile_group_framing,
    parse_tile_group_structure,
};
use splot_core::span::ByteOffset;
use splot_core::stream::BitstreamFormat;
use splot_core::types::ObuType;

use super::cdf::TileCdfPolicyInput;
use super::{
    DecodeTilePayloadPlan, TileBruPath, TileCoeffFrameFacts, TileCoeffFrameFactsInput,
    TileFrameFacts, TileGridFacts, TilePayloadBoundaryError, TilePayloadBoundaryInput,
    TilePayloadSource, plan_tile_payload_boundary,
};
use crate::{
    DecodeLimitError, DecodeLimitName, DecodeLimitOp, DecodeLimits, DecodeObuSourceKind,
    DecodePlannedObu, DecodeStreamPlan,
};

/// Sequence CDF facts needed by the tile CDF save policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FrameCandidateCdfFacts {
    enable_avg_cdf: bool,
    avg_cdf_type: bool,
}

impl FrameCandidateCdfFacts {
    /// Creates CDF policy facts from the active sequence header.
    #[must_use]
    pub(crate) const fn new(enable_avg_cdf: bool, avg_cdf_type: bool) -> Self {
        Self {
            enable_avg_cdf,
            avg_cdf_type,
        }
    }
}

/// Sequence facts needed by future coefficient decoding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FrameCandidateCoeffFacts {
    enable_fsc: bool,
    enable_idtx_intra: bool,
    enable_intra_ist: bool,
    enable_inter_ist: bool,
    enable_chroma_dctonly: bool,
    enable_cctx: bool,
}

impl FrameCandidateCoeffFacts {
    /// Creates coefficient frame facts from the active sequence header.
    // Each bool is a distinct AV2 sequence-level syntax flag; bundling them would obscure the spec mapping.
    #[allow(clippy::fn_params_excessive_bools)]
    #[must_use]
    pub(crate) const fn new(
        enable_fsc: bool,
        enable_idtx_intra: bool,
        enable_intra_ist: bool,
        enable_inter_ist: bool,
        enable_chroma_dctonly: bool,
        enable_cctx: bool,
    ) -> Self {
        Self {
            enable_fsc,
            enable_idtx_intra,
            enable_intra_ist,
            enable_inter_ist,
            enable_chroma_dctonly,
            enable_cctx,
        }
    }

    /// Creates coefficient frame facts from a parsed AV2 § 5.4.8 sequence
    /// transform/quant/entropy configuration.
    #[must_use]
    pub(crate) const fn from_tq(tq: &SequenceTqEntropyConfig) -> Self {
        Self::new(
            tq.enable_fsc,
            tq.enable_idtx_intra,
            tq.enable_intra_ist,
            tq.enable_inter_ist,
            tq.enable_chroma_dctonly,
            tq.enable_cctx,
        )
    }
}

/// Normalized parser facts required by the tile-payload boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FrameCandidateTileFacts<'a> {
    obu_type: ObuType,
    frame_is_intra: bool,
    is_bridge: bool,
    tile_cols: u32,
    tile_rows: u32,
    mi_col_starts: &'a [u32],
    mi_row_starts: &'a [u32],
    tile_cols_log2: u8,
    tile_rows_log2: u8,
    tile_size_bytes: Option<u32>,
    context_update_tile_id: u32,
    base_q_idx: u32,
    coeff_frame_facts: TileCoeffFrameFacts,
    disable_cdf_update: bool,
    tile_group_structure_start_bits: u64,
}

impl<'a> FrameCandidateTileFacts<'a> {
    /// Derives normalized tile facts from a state-aware frame-header core parse.
    ///
    /// # Errors
    /// Returns a local derivation error when the frame header did not reach the
    /// complete intra path or lacks facts required by the tile boundary.
    pub(crate) fn from_frame_core(
        core: &'a FrameHeaderCore,
        coeff: FrameCandidateCoeffFacts,
    ) -> Result<Self, FrameCandidateTileBoundaryError> {
        if core.status != FrameHeaderParseStatus::IntraHeaderComplete {
            return Err(FrameCandidateTileBoundaryError::Unsupported {
                reason: FrameCandidateTileUnsupportedReason::IncompleteFrameHeader,
            });
        }
        if core.frame_is_intra != Some(true) {
            return Err(FrameCandidateTileBoundaryError::Unsupported {
                reason: FrameCandidateTileUnsupportedReason::NonIntraFrame,
            });
        }
        if core.is_bridge {
            return Err(FrameCandidateTileBoundaryError::Unsupported {
                reason: FrameCandidateTileUnsupportedReason::BridgeFrame,
            });
        }
        let tile_info = core
            .tile_info
            .as_ref()
            .ok_or(FrameCandidateTileBoundaryError::MissingFact { fact: "tile_info" })?;
        let quant =
            core.quantization_params
                .ok_or(FrameCandidateTileBoundaryError::MissingFact {
                    fact: "quantization_params",
                })?;
        let disable_cdf_update =
            core.disable_cdf_update
                .ok_or(FrameCandidateTileBoundaryError::MissingFact {
                    fact: "disable_cdf_update",
                })?;
        let lossless = core
            .lossless_info
            .ok_or(FrameCandidateTileBoundaryError::MissingFact {
                fact: "lossless_info",
            })?;
        let tail = core
            .intra_tail
            .ok_or(FrameCandidateTileBoundaryError::MissingFact { fact: "intra_tail" })?;
        let coeff_frame_facts = TileCoeffFrameFacts::new(TileCoeffFrameFactsInput {
            enable_fsc: coeff.enable_fsc,
            enable_idtx_intra: coeff.enable_idtx_intra,
            enable_intra_ist: coeff.enable_intra_ist,
            enable_inter_ist: coeff.enable_inter_ist,
            enable_chroma_dctonly: coeff.enable_chroma_dctonly,
            enable_cctx: coeff.enable_cctx,
            reduced_tx_set: usize::from(tail.reduced_tx_set),
            lossless_array: lossless.lossless_array,
            allow_tcq: lossless.allow_tcq,
            allow_parity_hiding: lossless.allow_parity_hiding,
            base_q_idx: quant.base_q_idx,
        });

        Ok(Self {
            obu_type: core.obu_type,
            frame_is_intra: true,
            is_bridge: false,
            tile_cols: tile_info.tile_cols,
            tile_rows: tile_info.tile_rows,
            mi_col_starts: &tile_info.mi_col_starts,
            mi_row_starts: &tile_info.mi_row_starts,
            tile_cols_log2: tile_info.tile_cols_log2,
            tile_rows_log2: tile_info.tile_rows_log2,
            tile_size_bytes: tile_info.tile_size_bytes,
            context_update_tile_id: tile_info.context_update_tile_id,
            base_q_idx: quant.base_q_idx,
            coeff_frame_facts,
            disable_cdf_update,
            tile_group_structure_start_bits: core.consumed_bits.checked_add(1).ok_or(
                DecodeLimitError::ArithmeticOverflow {
                    name: DecodeLimitName::MaxInputBytes,
                    op: DecodeLimitOp::Add,
                    left: core.consumed_bits,
                    right: 1,
                },
            )?,
        })
    }

    /// Derives normalized tile facts from a state-aware **inter** frame-header core
    /// parse (AV2 § 5.18.2 `InterHeaderComplete`).
    ///
    /// This mirrors [`Self::from_frame_core`] for the inter path: the tile geometry,
    /// `base_q_idx`, `disable_cdf_update`, and coefficient frame facts are read from
    /// the same shared `core` fields, but `reduced_tx_set` comes from the parsed
    /// `core.inter_tail` (the inter tail) instead of `core.intra_tail`, and the
    /// completion gate is `InterHeaderComplete` with `frame_is_intra == false`.
    ///
    /// # Errors
    /// Returns a local derivation error when the frame header did not reach the
    /// complete inter path or lacks facts required by the tile boundary.
    pub(crate) fn from_inter_frame_core(
        core: &'a FrameHeaderCore,
        coeff: FrameCandidateCoeffFacts,
    ) -> Result<Self, FrameCandidateTileBoundaryError> {
        if core.status != FrameHeaderParseStatus::InterHeaderComplete {
            return Err(FrameCandidateTileBoundaryError::Unsupported {
                reason: FrameCandidateTileUnsupportedReason::IncompleteFrameHeader,
            });
        }
        if core.frame_is_intra != Some(false) {
            return Err(FrameCandidateTileBoundaryError::Unsupported {
                reason: FrameCandidateTileUnsupportedReason::NonIntraFrame,
            });
        }
        if core.is_bridge {
            return Err(FrameCandidateTileBoundaryError::Unsupported {
                reason: FrameCandidateTileUnsupportedReason::BridgeFrame,
            });
        }
        let tile_info = core
            .tile_info
            .as_ref()
            .ok_or(FrameCandidateTileBoundaryError::MissingFact { fact: "tile_info" })?;
        let quant =
            core.quantization_params
                .ok_or(FrameCandidateTileBoundaryError::MissingFact {
                    fact: "quantization_params",
                })?;
        let disable_cdf_update =
            core.disable_cdf_update
                .ok_or(FrameCandidateTileBoundaryError::MissingFact {
                    fact: "disable_cdf_update",
                })?;
        let lossless = core
            .lossless_info
            .ok_or(FrameCandidateTileBoundaryError::MissingFact {
                fact: "lossless_info",
            })?;
        let tail = core
            .inter_tail
            .as_ref()
            .ok_or(FrameCandidateTileBoundaryError::MissingFact { fact: "inter_tail" })?;
        let coeff_frame_facts = TileCoeffFrameFacts::new(TileCoeffFrameFactsInput {
            enable_fsc: coeff.enable_fsc,
            enable_idtx_intra: coeff.enable_idtx_intra,
            enable_intra_ist: coeff.enable_intra_ist,
            enable_inter_ist: coeff.enable_inter_ist,
            enable_chroma_dctonly: coeff.enable_chroma_dctonly,
            enable_cctx: coeff.enable_cctx,
            reduced_tx_set: usize::from(tail.reduced_tx_set),
            lossless_array: lossless.lossless_array,
            allow_tcq: lossless.allow_tcq,
            allow_parity_hiding: lossless.allow_parity_hiding,
            base_q_idx: quant.base_q_idx,
        });

        Ok(Self {
            obu_type: core.obu_type,
            frame_is_intra: false,
            is_bridge: false,
            tile_cols: tile_info.tile_cols,
            tile_rows: tile_info.tile_rows,
            mi_col_starts: &tile_info.mi_col_starts,
            mi_row_starts: &tile_info.mi_row_starts,
            tile_cols_log2: tile_info.tile_cols_log2,
            tile_rows_log2: tile_info.tile_rows_log2,
            tile_size_bytes: tile_info.tile_size_bytes,
            context_update_tile_id: tile_info.context_update_tile_id,
            base_q_idx: quant.base_q_idx,
            coeff_frame_facts,
            disable_cdf_update,
            tile_group_structure_start_bits: core.consumed_bits.checked_add(1).ok_or(
                DecodeLimitError::ArithmeticOverflow {
                    name: DecodeLimitName::MaxInputBytes,
                    op: DecodeLimitOp::Add,
                    left: core.consumed_bits,
                    right: 1,
                },
            )?,
        })
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) const fn new_for_test(
        obu_type: ObuType,
        frame_is_intra: bool,
        is_bridge: bool,
        tile_cols: u32,
        tile_rows: u32,
        mi_col_starts: &'a [u32],
        mi_row_starts: &'a [u32],
        tile_size_bytes: Option<u32>,
        context_update_tile_id: u32,
        base_q_idx: u32,
        disable_cdf_update: bool,
    ) -> Self {
        Self {
            obu_type,
            frame_is_intra,
            is_bridge,
            tile_cols,
            tile_rows,
            mi_col_starts,
            mi_row_starts,
            tile_cols_log2: 0,
            tile_rows_log2: 0,
            tile_size_bytes,
            context_update_tile_id,
            base_q_idx,
            coeff_frame_facts: TileCoeffFrameFacts::default_for_base_q(base_q_idx),
            disable_cdf_update,
            tile_group_structure_start_bits: 8,
        }
    }

    #[cfg(test)]
    pub(crate) const fn with_tile_group_structure_start_bits(mut self, bits: u64) -> Self {
        self.tile_group_structure_start_bits = bits;
        self
    }
}

/// Tile-group position facts supplied by stateful frame traversal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TileGroupPositionFacts {
    is_first_tile_group: bool,
    is_last_tile_group: bool,
}

impl TileGroupPositionFacts {
    /// Creates tile-group position facts.
    #[must_use]
    pub(crate) const fn new(is_first_tile_group: bool, is_last_tile_group: bool) -> Self {
        Self {
            is_first_tile_group,
            is_last_tile_group,
        }
    }
}

/// Input to the crate-private tile-payload derivation bridge.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FrameCandidateTileBoundaryInput<'payload, 'facts> {
    plan: &'facts DecodeStreamPlan,
    candidate: &'facts DecodePlannedObu,
    input_bytes: &'payload [u8],
    envelope: ObuEnvelope<'payload>,
    position: TileGroupPositionFacts,
    facts: FrameCandidateTileFacts<'facts>,
    cdf: FrameCandidateCdfFacts,
    limits: DecodeLimits,
}

impl<'payload, 'facts> FrameCandidateTileBoundaryInput<'payload, 'facts> {
    /// Creates tile-payload derivation input from planned provenance and parser facts.
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub(crate) const fn new(
        plan: &'facts DecodeStreamPlan,
        candidate: &'facts DecodePlannedObu,
        input_bytes: &'payload [u8],
        envelope: ObuEnvelope<'payload>,
        position: TileGroupPositionFacts,
        facts: FrameCandidateTileFacts<'facts>,
        cdf: FrameCandidateCdfFacts,
        limits: DecodeLimits,
    ) -> Self {
        Self {
            plan,
            candidate,
            input_bytes,
            envelope,
            position,
            facts,
            cdf,
            limits,
        }
    }
}

/// Error from crate-private tile-payload input derivation.
#[derive(Debug, thiserror::Error)]
pub(crate) enum FrameCandidateTileBoundaryError {
    /// Candidate metadata is not usable as source-backed tile input.
    #[error("malformed frame-candidate tile input: {0}")]
    Malformed(FrameCandidateTileMalformed),
    /// Required parser facts are absent.
    #[error("missing parser fact for tile-payload derivation: {fact}")]
    MissingFact {
        /// Missing fact label.
        fact: &'static str,
    },
    /// The frame candidate is outside the current derivation tier.
    #[error("unsupported frame-candidate tile input: {reason}")]
    Unsupported {
        /// Unsupported reason.
        reason: FrameCandidateTileUnsupportedReason,
    },
    /// A decode resource limit rejected derivation before tile work was retained.
    #[error("frame-candidate tile derivation rejected by resource limit: {0}")]
    Limit(#[from] DecodeLimitError),
    /// The derived input reached the existing tile-payload boundary.
    #[error("tile-payload boundary failed after derivation: {0}")]
    Boundary(#[from] TilePayloadBoundaryError),
}

/// Malformed derivation input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FrameCandidateTileMalformed {
    /// The candidate is not contained in the supplied stream plan.
    CandidateNotInPlan,
    /// The plan format and candidate source kind disagree.
    PlanSourceKindMismatch {
        /// Plan container format.
        format: BitstreamFormat,
        /// Candidate source kind.
        source_kind: DecodeObuSourceKind,
    },
    /// A field on the borrowed OBU envelope disagrees with the planned OBU.
    CandidateEnvelopeMismatch {
        /// Mismatched field label.
        field: &'static str,
    },
    /// The declared OBU size is smaller than its parsed header.
    ObuSizeSmallerThanHeader {
        /// Declared `num_bytes_in_obu`.
        size: u32,
        /// Header size in bytes.
        header_size: u8,
    },
    /// The OBU or tile-group payload range falls outside the source container.
    SourceRangeOutOfBounds {
        /// Range label.
        range: &'static str,
    },
    /// The § 5.19 structure is not complete.
    TileGroupStructureIncomplete,
    /// The § 5.19 structure is structurally invalid.
    TileGroupStructureInvalid,
    /// The § 5.19-derived payload region does not match the OBU payload.
    TileGroupPayloadRangeInvalid,
    /// The tile-group range is invalid.
    TileGroupRangeInvalid {
        /// `tg_start`.
        tg_start: u32,
        /// `tg_end`.
        tg_end: u32,
    },
}

impl fmt::Display for FrameCandidateTileMalformed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CandidateNotInPlan => f.write_str("candidate is not present in stream plan"),
            Self::PlanSourceKindMismatch {
                format,
                source_kind,
            } => write!(
                f,
                "plan format {format:?} does not match candidate source kind {source_kind:?}"
            ),
            Self::CandidateEnvelopeMismatch { field } => {
                write!(f, "candidate/envelope mismatch for {field}")
            }
            Self::ObuSizeSmallerThanHeader { size, header_size } => write!(
                f,
                "declared OBU size {size} is smaller than header size {header_size}"
            ),
            Self::SourceRangeOutOfBounds { range } => {
                write!(f, "{range} range is outside its source container")
            }
            Self::TileGroupStructureIncomplete => {
                f.write_str("tile-group structure did not complete")
            }
            Self::TileGroupStructureInvalid => f.write_str("tile-group structure is invalid"),
            Self::TileGroupPayloadRangeInvalid => {
                f.write_str("tile-group payload range is not contained in OBU payload")
            }
            Self::TileGroupRangeInvalid { tg_start, tg_end } => {
                write!(f, "invalid tile-group range {tg_start}..={tg_end}")
            }
        }
    }
}

/// Unsupported derivation reason.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FrameCandidateTileUnsupportedReason {
    /// Planned OBU is not a frame candidate.
    CandidateNotFrame,
    /// Tile group is not the first group of the coded frame.
    NonFirstTileGroup,
    /// Tile group is not the final group of the coded frame.
    NonLastTileGroup,
    /// Frame header did not parse to complete intra facts.
    IncompleteFrameHeader,
    /// Frame is not intra.
    NonIntraFrame,
    /// Bridge frame behavior is outside this tier.
    BridgeFrame,
    /// Tile group is not the one-tile, one-group minimal tier.
    NonSingleTileGroup,
}

impl fmt::Display for FrameCandidateTileUnsupportedReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::CandidateNotFrame => "candidate_not_frame",
            Self::NonFirstTileGroup => "non_first_tile_group",
            Self::NonLastTileGroup => "non_last_tile_group",
            Self::IncompleteFrameHeader => "incomplete_frame_header",
            Self::NonIntraFrame => "non_intra_frame",
            Self::BridgeFrame => "bridge_frame",
            Self::NonSingleTileGroup => "non_single_tile_group",
        })
    }
}

/// Derives and plans the minimal tile-payload boundary for one frame candidate.
pub(crate) fn plan_derived_tile_payload_boundary<'payload>(
    input: &FrameCandidateTileBoundaryInput<'payload, '_>,
) -> Result<DecodeTilePayloadPlan<'payload>, FrameCandidateTileBoundaryError> {
    validate_candidate(
        input.plan,
        input.candidate,
        input.input_bytes,
        input.envelope,
    )?;
    input.limits.ensure_mul(
        DecodeLimitName::MaxTileCount,
        u64::from(input.facts.tile_cols),
        u64::from(input.facts.tile_rows),
    )?;
    validate_supported_position(input.candidate, input.position, input.facts)?;

    let structure = derive_tile_group_structure(input.envelope, input.facts)?;
    let (payload, payload_base) = tile_group_payload_region(input.envelope, structure)?;
    let group_tile_count = tile_group_tile_count(structure)?;
    input
        .limits
        .ensure(DecodeLimitName::MaxTileCount, group_tile_count)?;
    if structure.tg_start != 0 || structure.tg_end != 0 {
        return Err(FrameCandidateTileBoundaryError::Unsupported {
            reason: FrameCandidateTileUnsupportedReason::NonSingleTileGroup,
        });
    }

    let tile_size_bytes = input.facts.tile_size_bytes.unwrap_or(1);
    let framing = parse_tile_group_framing(
        payload,
        structure.tg_start,
        structure.tg_end,
        tile_size_bytes,
        input.facts.is_bridge,
    );
    let cdf_policy = TileCdfPolicyInput::new(
        input.facts.tile_cols,
        input.facts.tile_rows,
        input.cdf.enable_avg_cdf,
        input.cdf.avg_cdf_type,
        input.facts.context_update_tile_id,
    );
    let boundary_input = TilePayloadBoundaryInput::new(
        payload,
        payload_base,
        &framing,
        TilePayloadSource::new(
            input.candidate.source_kind(),
            input.candidate.ivf_frame(),
            input.candidate.index(),
            input.candidate.offset(),
        ),
        input.plan.selected_layer(),
        TileGridFacts::new(
            input.facts.tile_cols,
            input.facts.tile_rows,
            input.facts.mi_col_starts,
            input.facts.mi_row_starts,
        ),
        TileFrameFacts::new(
            input.facts.obu_type,
            input.facts.frame_is_intra,
            input.position.is_first_tile_group,
            input.position.is_last_tile_group,
            input.facts.is_bridge,
            TileBruPath::NotUsed,
            input.facts.base_q_idx,
            input.facts.disable_cdf_update,
        )
        .with_coeff_frame_facts(input.facts.coeff_frame_facts)
        .with_cdf_policy(cdf_policy),
        input.limits,
    );

    Ok(plan_tile_payload_boundary(&boundary_input)?)
}

fn validate_candidate(
    plan: &DecodeStreamPlan,
    candidate: &DecodePlannedObu,
    input_bytes: &[u8],
    envelope: ObuEnvelope<'_>,
) -> Result<(), FrameCandidateTileBoundaryError> {
    if !plan.obus().any(|planned| planned == candidate) {
        return Err(FrameCandidateTileBoundaryError::Malformed(
            FrameCandidateTileMalformed::CandidateNotInPlan,
        ));
    }
    // Accept an intra key `FrameCandidate` or an inter `InterFrameCandidate`
    // (DECODE-FIRST-INTER-FRAME-FRONTIER); `is_frame_candidate()` covers both.
    if !candidate.role().is_frame_candidate() {
        return Err(FrameCandidateTileBoundaryError::Unsupported {
            reason: FrameCandidateTileUnsupportedReason::CandidateNotFrame,
        });
    }
    match (plan.format(), candidate.source_kind()) {
        (BitstreamFormat::AnnexB, DecodeObuSourceKind::AnnexB)
        | (BitstreamFormat::Ivf, DecodeObuSourceKind::Ivf) => {}
        (format, source_kind) => {
            return Err(FrameCandidateTileBoundaryError::Malformed(
                FrameCandidateTileMalformed::PlanSourceKindMismatch {
                    format,
                    source_kind,
                },
            ));
        }
    }
    if candidate.offset() != envelope.offset {
        return mismatch("offset");
    }
    if candidate.size() != envelope.size {
        return mismatch("size");
    }
    if candidate.header() != envelope.header {
        return mismatch("header");
    }
    if plan.input_len_bytes() != input_bytes.len() as u64 {
        return mismatch("input_len_bytes");
    }

    let payload_len = declared_payload_len(envelope)?;
    if candidate.payload_len() != payload_len {
        return mismatch("payload_len");
    }
    if envelope.payload.len() as u64 != payload_len {
        return mismatch("payload");
    }
    validate_envelope_payload_slice(input_bytes, envelope, payload_len)?;

    let obu_end = checked_offset_value(envelope.offset, u64::from(envelope.size))?;
    if obu_end > plan.input_len_bytes() {
        return Err(FrameCandidateTileBoundaryError::Malformed(
            FrameCandidateTileMalformed::SourceRangeOutOfBounds { range: "obu" },
        ));
    }

    match candidate.source_kind() {
        DecodeObuSourceKind::AnnexB => {
            if candidate.ivf_frame().is_some() {
                return mismatch("ivf_frame");
            }
        }
        DecodeObuSourceKind::Ivf => {
            let Some(frame) = candidate.ivf_frame() else {
                return mismatch("ivf_frame");
            };
            let frame_start = frame.frame_payload_offset().get();
            let frame_end = frame_start
                .checked_add(u64::from(frame.frame_payload_size()))
                .ok_or(DecodeLimitError::ArithmeticOverflow {
                    name: DecodeLimitName::MaxInputBytes,
                    op: DecodeLimitOp::Add,
                    left: frame_start,
                    right: u64::from(frame.frame_payload_size()),
                })?;
            if envelope.offset.get() < frame_start || obu_end > frame_end {
                return Err(FrameCandidateTileBoundaryError::Malformed(
                    FrameCandidateTileMalformed::SourceRangeOutOfBounds {
                        range: "ivf_frame_payload",
                    },
                ));
            }
        }
    }

    Ok(())
}

fn validate_envelope_payload_slice(
    input_bytes: &[u8],
    envelope: ObuEnvelope<'_>,
    payload_len: u64,
) -> Result<(), FrameCandidateTileBoundaryError> {
    let payload_offset = checked_payload_offset(envelope)?;
    let payload_start = usize::try_from(payload_offset.get()).map_err(|_| {
        FrameCandidateTileBoundaryError::Malformed(
            FrameCandidateTileMalformed::SourceRangeOutOfBounds { range: "payload" },
        )
    })?;
    let payload_end = payload_offset.get().checked_add(payload_len).ok_or(
        DecodeLimitError::ArithmeticOverflow {
            name: DecodeLimitName::MaxInputBytes,
            op: DecodeLimitOp::Add,
            left: payload_offset.get(),
            right: payload_len,
        },
    )?;
    let payload_end = usize::try_from(payload_end).map_err(|_| {
        FrameCandidateTileBoundaryError::Malformed(
            FrameCandidateTileMalformed::SourceRangeOutOfBounds { range: "payload" },
        )
    })?;
    let expected = input_bytes.get(payload_start..payload_end).ok_or(
        FrameCandidateTileBoundaryError::Malformed(
            FrameCandidateTileMalformed::SourceRangeOutOfBounds { range: "payload" },
        ),
    )?;
    if !core::ptr::eq(expected, envelope.payload) {
        return mismatch("payload_source");
    }
    Ok(())
}

fn mismatch<T>(field: &'static str) -> Result<T, FrameCandidateTileBoundaryError> {
    Err(FrameCandidateTileBoundaryError::Malformed(
        FrameCandidateTileMalformed::CandidateEnvelopeMismatch { field },
    ))
}

fn validate_supported_position(
    candidate: &DecodePlannedObu,
    position: TileGroupPositionFacts,
    facts: FrameCandidateTileFacts<'_>,
) -> Result<(), FrameCandidateTileBoundaryError> {
    if !position.is_first_tile_group {
        return Err(FrameCandidateTileBoundaryError::Unsupported {
            reason: FrameCandidateTileUnsupportedReason::NonFirstTileGroup,
        });
    }
    if !position.is_last_tile_group {
        return Err(FrameCandidateTileBoundaryError::Unsupported {
            reason: FrameCandidateTileUnsupportedReason::NonLastTileGroup,
        });
    }
    if facts.is_bridge {
        return Err(FrameCandidateTileBoundaryError::Unsupported {
            reason: FrameCandidateTileUnsupportedReason::BridgeFrame,
        });
    }
    // The tile-payload boundary is supported for an intra `OBU_CLOSED_LOOP_KEY` frame
    // and for an inter `OBU_REGULAR_TILE_GROUP` frame (DECODE-FIRST-INTER-FRAME-FRONTIER).
    // An intra frame must carry the key OBU type, and an inter frame the regular
    // tile-group OBU type; any other (frame_is_intra, obu_type) pairing is rejected.
    match (facts.frame_is_intra, candidate.obu_type()) {
        (true, ObuType::ClosedLoopKey) | (false, ObuType::RegularTileGroup) => {}
        _ => {
            return Err(FrameCandidateTileBoundaryError::Unsupported {
                reason: FrameCandidateTileUnsupportedReason::CandidateNotFrame,
            });
        }
    }
    if facts.tile_cols != 1 || facts.tile_rows != 1 {
        return Err(FrameCandidateTileBoundaryError::Unsupported {
            reason: FrameCandidateTileUnsupportedReason::NonSingleTileGroup,
        });
    }
    Ok(())
}

fn tile_group_payload_region(
    envelope: ObuEnvelope<'_>,
    structure: TileGroupStructure,
) -> Result<(&[u8], ByteOffset), FrameCandidateTileBoundaryError> {
    if structure.outcome != TileGroupStructureOutcome::Complete {
        return Err(FrameCandidateTileBoundaryError::Malformed(
            FrameCandidateTileMalformed::TileGroupStructureIncomplete,
        ));
    }
    let Some(header_bytes) = structure.header_bytes else {
        return Err(FrameCandidateTileBoundaryError::Malformed(
            FrameCandidateTileMalformed::TileGroupStructureIncomplete,
        ));
    };
    let Some(payload_size) = structure.payload_size else {
        return Err(FrameCandidateTileBoundaryError::Malformed(
            FrameCandidateTileMalformed::TileGroupStructureIncomplete,
        ));
    };
    let declared_payload_len = declared_payload_len(envelope)?;
    let payload_end =
        header_bytes
            .checked_add(payload_size)
            .ok_or(DecodeLimitError::ArithmeticOverflow {
                name: DecodeLimitName::MaxTilePayloadBytes,
                op: DecodeLimitOp::Add,
                left: header_bytes,
                right: payload_size,
            })?;
    if payload_end != declared_payload_len || payload_end > envelope.payload.len() as u64 {
        return Err(FrameCandidateTileBoundaryError::Malformed(
            FrameCandidateTileMalformed::TileGroupPayloadRangeInvalid,
        ));
    }
    let start = usize::try_from(header_bytes).map_err(|_| {
        FrameCandidateTileBoundaryError::Malformed(
            FrameCandidateTileMalformed::TileGroupPayloadRangeInvalid,
        )
    })?;
    let end = usize::try_from(payload_end).map_err(|_| {
        FrameCandidateTileBoundaryError::Malformed(
            FrameCandidateTileMalformed::TileGroupPayloadRangeInvalid,
        )
    })?;
    let payload =
        envelope
            .payload
            .get(start..end)
            .ok_or(FrameCandidateTileBoundaryError::Malformed(
                FrameCandidateTileMalformed::TileGroupPayloadRangeInvalid,
            ))?;
    let payload_offset = checked_payload_offset(envelope)?;
    let payload_base = super::checked_byte_offset(
        payload_offset,
        header_bytes,
        DecodeLimitName::MaxTilePayloadBytes,
    )?;
    Ok((payload, payload_base))
}

fn derive_tile_group_structure(
    envelope: ObuEnvelope<'_>,
    facts: FrameCandidateTileFacts<'_>,
) -> Result<TileGroupStructure, FrameCandidateTileBoundaryError> {
    let payload_offset = checked_payload_offset(envelope)?;
    let mut reader = BitReader::new(envelope.payload, payload_offset);
    advance_reader_bits(&mut reader, facts.tile_group_structure_start_bits)?;
    let layout = TileGroupLayout::new(
        facts.tile_cols,
        facts.tile_rows,
        facts.tile_cols_log2,
        facts.tile_rows_log2,
    );
    parse_tile_group_structure(&mut reader, layout, declared_payload_len(envelope)?).map_err(
        |error| match error {
            Error::UnexpectedEof { .. } => FrameCandidateTileBoundaryError::Malformed(
                FrameCandidateTileMalformed::TileGroupStructureIncomplete,
            ),
            _ => FrameCandidateTileBoundaryError::Malformed(
                FrameCandidateTileMalformed::TileGroupStructureInvalid,
            ),
        },
    )
}

fn advance_reader_bits(
    reader: &mut BitReader<'_>,
    mut bits: u64,
) -> Result<(), FrameCandidateTileBoundaryError> {
    while bits > 0 {
        let step = bits.min(32) as u32;
        if let Err(error) = reader.read_bits(step) {
            return match error {
                Error::UnexpectedEof { .. } => Err(FrameCandidateTileBoundaryError::Malformed(
                    FrameCandidateTileMalformed::TileGroupStructureIncomplete,
                )),
                _ => Err(FrameCandidateTileBoundaryError::Malformed(
                    FrameCandidateTileMalformed::TileGroupStructureInvalid,
                )),
            };
        }
        bits -= u64::from(step);
    }
    Ok(())
}

fn tile_group_tile_count(
    structure: TileGroupStructure,
) -> Result<u64, FrameCandidateTileBoundaryError> {
    if structure.tg_end < structure.tg_start {
        return Err(FrameCandidateTileBoundaryError::Malformed(
            FrameCandidateTileMalformed::TileGroupRangeInvalid {
                tg_start: structure.tg_start,
                tg_end: structure.tg_end,
            },
        ));
    }
    Ok(u64::from(structure.tg_end - structure.tg_start) + 1)
}

fn declared_payload_len(envelope: ObuEnvelope<'_>) -> Result<u64, FrameCandidateTileBoundaryError> {
    let header_size = u64::from(envelope.header.header_size_bytes);
    u64::from(envelope.size).checked_sub(header_size).ok_or(
        FrameCandidateTileBoundaryError::Malformed(
            FrameCandidateTileMalformed::ObuSizeSmallerThanHeader {
                size: envelope.size,
                header_size: envelope.header.header_size_bytes,
            },
        ),
    )
}

fn checked_payload_offset(
    envelope: ObuEnvelope<'_>,
) -> Result<ByteOffset, FrameCandidateTileBoundaryError> {
    Ok(ByteOffset::new(checked_offset_value(
        envelope.offset,
        u64::from(envelope.header.header_size_bytes),
    )?))
}

fn checked_offset_value(
    offset: ByteOffset,
    delta: u64,
) -> Result<u64, FrameCandidateTileBoundaryError> {
    offset
        .get()
        .checked_add(delta)
        .ok_or_else(|| DecodeLimitError::ArithmeticOverflow {
            name: DecodeLimitName::MaxInputBytes,
            op: DecodeLimitOp::Add,
            left: offset.get(),
            right: delta,
        })
        .map_err(FrameCandidateTileBoundaryError::from)
}

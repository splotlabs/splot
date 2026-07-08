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

use super::cdf::{FrameCdfSubset, TileCdfPolicyInput};
use super::{
    DecodeTilePayloadPlan, TileBruPath, TileCoeffFrameFacts, TileCoeffFrameFactsInput,
    TileFrameFacts, TileGridFacts, TilePayloadBoundaryError, TilePayloadBoundaryInput,
    TilePayloadSource, plan_tile_payload_boundary,
};
use crate::{
    DecodeLimitError, DecodeLimitName, DecodeLimitOp, DecodeLimits, DecodeObuSourceKind,
    DecodePlannedObu, DecodeStreamPlan,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FrameCandidateCdfFacts {
    enable_avg_cdf: bool,
    avg_cdf_type: bool,
}

impl FrameCandidateCdfFacts {
    #[must_use]
    pub(crate) const fn new(enable_avg_cdf: bool, avg_cdf_type: bool) -> Self {
        Self {
            enable_avg_cdf,
            avg_cdf_type,
        }
    }
}

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
    pub(crate) fn from_frame_core(
        core: &'a FrameHeaderCore,
        coeff: FrameCandidateCoeffFacts,
    ) -> Result<Self, FrameCandidateTileBoundaryError> {
        Self::from_core(core, coeff, FrameCandidateTilePath::Intra)
    }

    pub(crate) fn from_inter_frame_core(
        core: &'a FrameHeaderCore,
        coeff: FrameCandidateCoeffFacts,
    ) -> Result<Self, FrameCandidateTileBoundaryError> {
        Self::from_core(core, coeff, FrameCandidateTilePath::Inter)
    }

    fn from_core(
        core: &'a FrameHeaderCore,
        coeff: FrameCandidateCoeffFacts,
        path: FrameCandidateTilePath,
    ) -> Result<Self, FrameCandidateTileBoundaryError> {
        if core.status != path.expected_status() {
            return Err(FrameCandidateTileBoundaryError::Unsupported {
                reason: FrameCandidateTileUnsupportedReason::IncompleteFrameHeader,
            });
        }
        if core.frame_is_intra != Some(path.frame_is_intra()) {
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
        let reduced_tx_set = path.reduced_tx_set(core)?;
        let coeff_frame_facts = TileCoeffFrameFacts::new(TileCoeffFrameFactsInput {
            enable_fsc: coeff.enable_fsc,
            enable_idtx_intra: coeff.enable_idtx_intra,
            enable_intra_ist: coeff.enable_intra_ist,
            enable_inter_ist: coeff.enable_inter_ist,
            enable_chroma_dctonly: coeff.enable_chroma_dctonly,
            enable_cctx: coeff.enable_cctx,
            reduced_tx_set,
            lossless_array: lossless.lossless_array,
            allow_tcq: lossless.allow_tcq,
            allow_parity_hiding: lossless.allow_parity_hiding,
            base_q_idx: quant.base_q_idx,
        });

        Ok(Self {
            obu_type: core.obu_type,
            frame_is_intra: path.frame_is_intra(),
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FrameCandidateTilePath {
    Intra,
    Inter,
}

impl FrameCandidateTilePath {
    const fn expected_status(self) -> FrameHeaderParseStatus {
        match self {
            Self::Intra => FrameHeaderParseStatus::IntraHeaderComplete,
            Self::Inter => FrameHeaderParseStatus::InterHeaderComplete,
        }
    }

    const fn frame_is_intra(self) -> bool {
        matches!(self, Self::Intra)
    }

    fn reduced_tx_set(
        self,
        core: &FrameHeaderCore,
    ) -> Result<usize, FrameCandidateTileBoundaryError> {
        match self {
            Self::Intra => core
                .intra_tail
                .as_ref()
                .map(|tail| usize::from(tail.reduced_tx_set))
                .ok_or(FrameCandidateTileBoundaryError::MissingFact { fact: "intra_tail" }),
            Self::Inter => core
                .inter_tail
                .as_ref()
                .map(|tail| usize::from(tail.reduced_tx_set))
                .ok_or(FrameCandidateTileBoundaryError::MissingFact { fact: "inter_tail" }),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TileGroupPositionFacts {
    is_first_tile_group: bool,
    is_last_tile_group: bool,
}

impl TileGroupPositionFacts {
    #[must_use]
    pub(crate) const fn new(is_first_tile_group: bool, is_last_tile_group: bool) -> Self {
        Self {
            is_first_tile_group,
            is_last_tile_group,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct FrameCandidateTileBoundaryInput<'payload, 'facts> {
    plan: &'facts DecodeStreamPlan,
    candidate: &'facts DecodePlannedObu,
    input_bytes: &'payload [u8],
    envelope: ObuEnvelope<'payload>,
    position: TileGroupPositionFacts,
    facts: FrameCandidateTileFacts<'facts>,
    cdf: FrameCandidateCdfFacts,
    limits: DecodeLimits,
    initial_cdfs: Option<FrameCdfSubset>,
}

impl<'payload, 'facts> FrameCandidateTileBoundaryInput<'payload, 'facts> {
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
            initial_cdfs: None,
        }
    }

    #[must_use]
    pub(crate) fn with_initial_cdfs(mut self, initial_cdfs: FrameCdfSubset) -> Self {
        self.initial_cdfs = Some(initial_cdfs);
        self
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum FrameCandidateTileBoundaryError {
    #[error("malformed frame-candidate tile input: {0}")]
    Malformed(FrameCandidateTileMalformed),
    #[error("missing parser fact for tile-payload derivation: {fact}")]
    MissingFact { fact: &'static str },
    #[error("unsupported frame-candidate tile input: {reason}")]
    Unsupported {
        reason: FrameCandidateTileUnsupportedReason,
    },
    #[error("frame-candidate tile derivation rejected by resource limit: {0}")]
    Limit(#[from] DecodeLimitError),
    #[error("tile-payload boundary failed after derivation: {0}")]
    Boundary(#[from] TilePayloadBoundaryError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FrameCandidateTileMalformed {
    CandidateNotInPlan,
    PlanSourceKindMismatch {
        format: BitstreamFormat,
        source_kind: DecodeObuSourceKind,
    },
    CandidateEnvelopeMismatch {
        field: &'static str,
    },
    ObuSizeSmallerThanHeader {
        size: u32,
        header_size: u8,
    },
    SourceRangeOutOfBounds {
        range: &'static str,
    },
    TileGroupStructureIncomplete,
    TileGroupStructureInvalid,
    TileGroupPayloadRangeInvalid,
    TileGroupRangeInvalid {
        tg_start: u32,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FrameCandidateTileUnsupportedReason {
    CandidateNotFrame,
    NonFirstTileGroup,
    NonLastTileGroup,
    IncompleteFrameHeader,
    NonIntraFrame,
    BridgeFrame,
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
    let total_tiles = u64::from(input.facts.tile_cols) * u64::from(input.facts.tile_rows);
    if total_tiles == 0 || structure.tg_start != 0 || u64::from(structure.tg_end) + 1 != total_tiles
    {
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
    let mut frame = TileFrameFacts::new(
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
    .with_cdf_policy(cdf_policy);
    if let Some(cdfs) = input.initial_cdfs.as_ref() {
        frame = frame.with_initial_cdfs(cdfs.clone());
    }
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
        frame,
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
    match (facts.frame_is_intra, candidate.obu_type()) {
        (true, ObuType::ClosedLoopKey) | (false, ObuType::RegularTileGroup) => {}
        _ => {
            return Err(FrameCandidateTileBoundaryError::Unsupported {
                reason: FrameCandidateTileUnsupportedReason::CandidateNotFrame,
            });
        }
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

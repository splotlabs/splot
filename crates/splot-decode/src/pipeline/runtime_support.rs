// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::headers::frame::FrameSize;
use splot_core::headers::sequence::ChromaFormatIdc;
use splot_core::span::ByteOffset;
use splot_recon::BitDepth;

use crate::DecodeLimitName;
use crate::bitstream::stream_plan::DecodeSourceIssue;
use crate::bitstream::tile_payload::{
    FrameCandidateTileBoundaryError, FrameCandidateTileMalformed,
};
use crate::error::{DecodeError, DecodeUnsupportedFeature, Result};
use crate::support::pipeline_limits::decoded_frame_storage_budget;

use super::bytes_per_sample;

const SPEC_SECTION: &str = "7.1";

#[allow(clippy::needless_pass_by_value)]
pub(super) fn decode_tile_boundary_error(error: FrameCandidateTileBoundaryError) -> DecodeError {
    match error {
        FrameCandidateTileBoundaryError::Limit(source) => DecodeError::Limit { source },
        FrameCandidateTileBoundaryError::Malformed(malformed) => unsupported(
            malformed_tile_boundary_reason(malformed),
            None,
            "decode runtime could not derive a source-backed tile payload boundary",
        ),
        FrameCandidateTileBoundaryError::MissingFact { .. } => unsupported(
            "missing_tile_fact",
            None,
            "decode runtime requires complete parser-derived tile facts",
        ),
        FrameCandidateTileBoundaryError::Unsupported { .. }
        | FrameCandidateTileBoundaryError::Boundary(_) => unsupported(
            "unsupported_tile_boundary",
            None,
            "decode runtime requires source-backed tile work units",
        ),
    }
}

fn malformed_tile_boundary_reason(malformed: FrameCandidateTileMalformed) -> &'static str {
    match malformed {
        FrameCandidateTileMalformed::CandidateNotInPlan => "candidate_not_in_plan",
        FrameCandidateTileMalformed::PlanSourceKindMismatch { .. } => "plan_source_kind_mismatch",
        FrameCandidateTileMalformed::CandidateEnvelopeMismatch { field } => match field {
            "payload_source" => "payload_source_mismatch",
            "offset" => "candidate_offset_mismatch",
            "size" => "candidate_size_mismatch",
            "header" => "candidate_header_mismatch",
            "payload_len" => "candidate_payload_len_mismatch",
            "payload" => "candidate_payload_mismatch",
            "input_len_bytes" => "input_len_mismatch",
            "ivf_frame" => "ivf_frame_mismatch",
            _ => "candidate_envelope_mismatch",
        },
        FrameCandidateTileMalformed::ObuSizeSmallerThanHeader { .. } => {
            "obu_size_smaller_than_header"
        }
        FrameCandidateTileMalformed::SourceRangeOutOfBounds { .. } => "source_range_out_of_bounds",
        FrameCandidateTileMalformed::TileGroupStructureIncomplete => {
            "tile_group_structure_incomplete"
        }
        FrameCandidateTileMalformed::TileGroupStructureInvalid => "tile_group_structure_invalid",
        FrameCandidateTileMalformed::TileGroupPayloadRangeInvalid => {
            "tile_group_payload_range_invalid"
        }
        FrameCandidateTileMalformed::TileGroupRangeInvalid { .. } => "tile_group_range_invalid",
        FrameCandidateTileMalformed::TileGroupPositionMismatch { .. } => {
            "tile_group_position_mismatch"
        }
    }
}

pub(crate) fn ensure_runtime_limits(
    limits: crate::DecodeLimits,
    width: u32,
    height: u32,
    tile_payload_bytes: u64,
    bit_depth: BitDepth,
    chroma_format: ChromaFormatIdc,
) -> Result<()> {
    limits.ensure(DecodeLimitName::MaxFrameWidth, u64::from(width))?;
    limits.ensure(DecodeLimitName::MaxFrameHeight, u64::from(height))?;
    let budget = decoded_frame_storage_budget(
        FrameSize::new(width, height),
        chroma_format,
        bytes_per_sample(bit_depth),
    )?;
    limits.ensure(DecodeLimitName::MaxLumaSamplesPerFrame, budget.luma_samples)?;
    limits.ensure(DecodeLimitName::MaxDecodedFrameBytes, budget.decoded_bytes)?;
    limits.ensure(DecodeLimitName::MaxTileCount, 1)?;
    limits.ensure(DecodeLimitName::MaxTilePayloadBytes, tile_payload_bytes)?;
    limits.ensure_allocation_len(DecodeLimitName::MaxDecodedFrameBytes, budget.luma_samples)?;
    limits.ensure_allocation_len(
        DecodeLimitName::MaxDecodedFrameBytes,
        budget.chroma_samples_per_plane,
    )?;
    Ok(())
}

pub(crate) fn unsupported_with_spec(
    reason: &'static str,
    byte_offset: Option<ByteOffset>,
    message: &'static str,
    spec_section: &'static str,
) -> DecodeError {
    DecodeError::UnsupportedFeature {
        unsupported: Box::new(DecodeUnsupportedFeature::new(
            reason,
            spec_section,
            message,
            byte_offset,
        )),
    }
}

pub(crate) fn unsupported(
    reason: &'static str,
    byte_offset: Option<ByteOffset>,
    message: &'static str,
) -> DecodeError {
    unsupported_with_spec(reason, byte_offset, message, SPEC_SECTION)
}

pub(crate) fn unsupported_at(
    reason: &'static str,
    byte_offset: ByteOffset,
    message: &'static str,
) -> DecodeError {
    unsupported(reason, Some(byte_offset), message)
}

pub(crate) fn unsupported_feature_at(
    reason: &'static str,
    byte_offset: ByteOffset,
    message: &'static str,
    spec_section: &'static str,
) -> DecodeError {
    unsupported_with_spec(reason, Some(byte_offset), message, spec_section)
}

pub(crate) fn malformed_tile_payload(
    byte_offset: ByteOffset,
    spec_section: &'static str,
    error: impl core::fmt::Display,
) -> DecodeError {
    DecodeError::MalformedSource {
        issue: DecodeSourceIssue::tile_payload(byte_offset, spec_section, error.to_string()),
    }
}

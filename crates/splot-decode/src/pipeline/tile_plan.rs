// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Tile-group boundary planning and frame-header continuation checks.

use std::sync::Arc;

use splot_core::annexb::ObuEnvelope;
use splot_core::bitio::BitReader;
use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::sequence::SequenceHeader;
use splot_core::headers::tile_group::{
    FrameHeaderCopyOutcome, RecordedFrameHeaderBits, parse_frame_header_copy,
};

use super::runtime_support::{decode_tile_boundary_error, unsupported_at};
use crate::bitstream::byte_stream::SourceBytes;
use crate::bitstream::tile_payload::{
    FrameCandidateCdfFacts, FrameCandidateCoeffFacts, FrameCandidateTileBoundaryError,
    FrameCandidateTileBoundaryInput, FrameCandidateTileFacts, FrameCdfSubset,
    TileGroupPositionFacts,
};
use crate::error::Result;
use crate::{DecodeOptions, DecodePlannedObu};

#[derive(Clone, Copy)]
pub(crate) enum TileFactsKind {
    Intra,
    Inter,
}

#[expect(clippy::too_many_arguments)]
pub(crate) fn derive_tile_plan_with<'payload>(
    candidate: &DecodePlannedObu,
    bytes: SourceBytes<'payload>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    options: &DecodeOptions,
    kind: TileFactsKind,
    initial_cdfs: Option<&Arc<FrameCdfSubset>>,
    scratch: &mut crate::bitstream::tile_payload::TilePayloadScratch,
) -> Result<crate::bitstream::tile_payload::DecodeTilePayloadPlan<'payload>> {
    let envelope = planned_envelope(bytes, candidate)?;
    let tq = sequence.transform_quant_entropy.as_ref().ok_or_else(|| {
        unsupported_at(
            "missing_tq_entropy_config",
            envelope.offset,
            "decode runtime requires sequence transform/quant/entropy config",
        )
    })?;
    let coeff = FrameCandidateCoeffFacts::from_tq(tq);
    let facts = match kind {
        TileFactsKind::Intra => FrameCandidateTileFacts::from_frame_core(core, coeff),
        TileFactsKind::Inter => FrameCandidateTileFacts::from_inter_frame_core(core, coeff),
    }
    .map_err(|error| decode_tile_boundary_error(error, envelope.offset))?;
    let cdf = FrameCandidateCdfFacts::new(tq.enable_avg_cdf, tq.avg_cdf_type != 0);
    let candidates = core::iter::once(candidate.clone()).chain(
        crate::bitstream::stream_plan::tile_group_continuations(bytes, candidate),
    );
    let group_count = candidates.clone().count();
    let recorded_header = record_frame_header(envelope, core, group_count > 1)?;
    let mut merged: Option<crate::bitstream::tile_payload::DecodeTilePayloadPlan<'payload>> = None;
    for (group_index, group_candidate) in candidates.enumerate() {
        if group_index != 0 {
            core::mem::swap(
                &mut scratch.work_units,
                &mut scratch.continuation_work_units,
            );
        }
        let group_envelope = planned_envelope(bytes, &group_candidate)?;
        let group_facts = if group_index == 0 {
            facts
        } else {
            facts.with_tile_group_structure_start_bits(continuation_structure_start_bits(
                group_envelope,
                recorded_header.as_ref().ok_or_else(|| {
                    unsupported_at(
                        "frame_header_copy_source_truncated",
                        envelope.offset,
                        "first tile-group frame header was not recorded for continuation validation",
                    )
                })?,
            )?)
        };
        let mut input = FrameCandidateTileBoundaryInput::new(
            &group_candidate,
            bytes,
            group_envelope,
            TileGroupPositionFacts::new(group_index == 0, group_index + 1 == group_count),
            group_facts,
            cdf,
            options.limits(),
        );
        if let Some(cdfs) = initial_cdfs {
            input = input.with_initial_cdfs(Arc::clone(cdfs));
        }
        let group_plan =
            crate::bitstream::tile_payload::plan_derived_tile_payload_boundary_with_scratch(
                &input, scratch,
            )
            .map_err(|error| decode_tile_boundary_error(error, group_envelope.offset))?;
        if let Some(plan) = merged.as_mut() {
            plan.append_continuation(group_plan, &mut scratch.continuation_work_units)
                .map_err(FrameCandidateTileBoundaryError::from)
                .map_err(|error| decode_tile_boundary_error(error, group_envelope.offset))?;
        } else {
            merged = Some(group_plan);
        }
    }
    merged.ok_or_else(|| {
        unsupported_at(
            "missing_tile_group",
            envelope.offset,
            "decode runtime requires at least one tile group for a coded frame",
        )
    })
}

fn planned_envelope<'a>(
    bytes: SourceBytes<'a>,
    planned: &DecodePlannedObu,
) -> Result<ObuEnvelope<'a>> {
    let start = planned.offset().get();
    let payload = start
        .checked_add(u64::from(planned.header().header_size_bytes))
        .zip(start.checked_add(u64::from(planned.size())))
        .and_then(|(payload_start, end)| bytes.get(payload_start, end))
        .ok_or_else(|| {
            unsupported_at(
                "source_range_out_of_bounds",
                planned.offset(),
                "planned tile-group payload is outside the decode input",
            )
        })?;
    Ok(ObuEnvelope {
        offset: planned.offset(),
        size: planned.size(),
        header: planned.header(),
        payload,
    })
}

/// Checks the first tile group's frame header and, only when continuation
/// groups will compare against it, keeps a copy of its bits.
fn record_frame_header(
    envelope: ObuEnvelope<'_>,
    core: &FrameHeaderCore,
    continuations: bool,
) -> Result<Option<RecordedFrameHeaderBits>> {
    let mut reader = BitReader::new(envelope.payload, envelope.payload_offset());
    if reader.read_bit().ok() != Some(1) {
        return Err(unsupported_at(
            "missing_first_tile_group",
            envelope.offset,
            "coded frame must begin with is_first_tile_group equal to 1",
        ));
    }
    let truncated = || {
        unsupported_at(
            "frame_header_copy_source_truncated",
            envelope.offset,
            "first tile-group frame header could not be recorded for continuation validation",
        )
    };
    if !continuations {
        return if reader.remaining_bits() < core.consumed_bits {
            Err(truncated())
        } else {
            Ok(None)
        };
    }
    RecordedFrameHeaderBits::record(&mut reader, core.consumed_bits)
        .map(Some)
        .map_err(|_| truncated())
}

pub(super) fn continuation_structure_start_bits(
    envelope: ObuEnvelope<'_>,
    recorded: &RecordedFrameHeaderBits,
) -> Result<u64> {
    let mut reader = BitReader::new(envelope.payload, envelope.payload_offset());
    if reader.read_bit().ok() != Some(0) {
        return Err(unsupported_at(
            "unexpected_first_tile_group",
            envelope.offset,
            "tile-group continuation must set is_first_tile_group to 0",
        ));
    }
    let frame_header_present = reader.read_bit().map_err(|_| {
        unsupported_at(
            "tile_group_prefix_parse",
            envelope.offset,
            "tile-group continuation ends before frame_header_present_flag",
        )
    })? != 0;
    if frame_header_present {
        match parse_frame_header_copy(&mut reader, recorded) {
            FrameHeaderCopyOutcome::Matches => {}
            FrameHeaderCopyOutcome::Mismatch { .. } => {
                return Err(unsupported_at(
                    "frame_header_copy_mismatch",
                    envelope.offset,
                    "tile-group continuation frame_header_copy differs from the first group",
                ));
            }
            FrameHeaderCopyOutcome::Truncated { .. } => {
                return Err(unsupported_at(
                    "frame_header_copy_truncated",
                    envelope.offset,
                    "tile-group continuation ends inside frame_header_copy",
                ));
            }
            _ => {
                return Err(unsupported_at(
                    "frame_header_copy_invalid",
                    envelope.offset,
                    "tile-group continuation frame_header_copy is invalid",
                ));
            }
        }
    }
    Ok(reader.consumed_bits())
}

pub(crate) fn derive_tile_plan<'payload>(
    candidate: &DecodePlannedObu,
    bytes: SourceBytes<'payload>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    options: &DecodeOptions,
) -> Result<crate::bitstream::tile_payload::DecodeTilePayloadPlan<'payload>> {
    let mut scratch = crate::bitstream::tile_payload::TilePayloadScratch::default();
    derive_tile_plan_with(
        candidate,
        bytes,
        sequence,
        core,
        options,
        TileFactsKind::Intra,
        None,
        &mut scratch,
    )
}

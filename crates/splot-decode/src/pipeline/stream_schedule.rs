// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Runtime stream and frame-unit scheduling.

use super::{PipelineFrameRate, unsupported, unsupported_at};

use splot_core::annexb::ObuEnvelope;
use splot_core::ivf::{IvfHeader, IvfWarning};
use splot_core::span::ByteOffset;
use splot_core::stream::{ParsedBitstream, ParsedIvfBitstream};
use splot_core::types::ObuType;

use crate::error::Result;
use crate::support::capability::missing_capability_message;
use crate::{DecodePlannedObu, DecodeStreamPlan};

#[derive(Clone, Copy, Debug)]
pub(super) enum RuntimeStream<'a> {
    AnnexB {
        obus: &'a [ObuEnvelope<'a>],
    },
    Ivf {
        ivf: &'a ParsedIvfBitstream<'a>,
        header: IvfHeader,
    },
}

impl<'a> RuntimeStream<'a> {
    pub(super) const fn ivf_header(self) -> Option<IvfHeader> {
        match self {
            Self::AnnexB { .. } => None,
            Self::Ivf { header, .. } => Some(header),
        }
    }

    pub(super) const fn frame_rate(self) -> PipelineFrameRate {
        match self {
            Self::AnnexB { .. } => PipelineFrameRate::ANNEX_B_DEFAULT,
            Self::Ivf { header, .. } => PipelineFrameRate::from_ivf_header(header),
        }
    }

    pub(super) fn leading_obus(self) -> Result<&'a [ObuEnvelope<'a>]> {
        match self {
            Self::AnnexB { obus } => Ok(obus),
            Self::Ivf { ivf, .. } => ivf
                .frames
                .first()
                .map(|frame| frame.obus.as_slice())
                .ok_or_else(|| {
                    unsupported(
                        "missing_first_ivf_frame",
                        None,
                        "decode runtime requires at least one IVF frame",
                    )
                }),
        }
    }
}

pub(super) fn require_runtime_stream<'a>(
    parsed: &'a ParsedBitstream<'a>,
) -> Result<RuntimeStream<'a>> {
    match parsed {
        ParsedBitstream::AnnexB(partial) => {
            if partial.error.is_some() {
                return Err(unsupported(
                    "annex_b_runtime_parse_error",
                    None,
                    "runtime reparse of the bounded Annex B input must stay complete",
                ));
            }
            if partial.obus.is_empty() {
                return Err(unsupported(
                    "empty_annex_b_input",
                    None,
                    "decode runtime requires at least one Annex B OBU",
                ));
            }
            Ok(RuntimeStream::AnnexB {
                obus: partial.obus.as_slice(),
            })
        }
        ParsedBitstream::Ivf(ivf) => {
            let header = require_multiframe_ivf(ivf)?;
            Ok(RuntimeStream::Ivf { ivf, header })
        }
    }
}

pub(crate) fn following_inter_envelope<'a>(
    ivf: &'a ParsedIvfBitstream<'a>,
    candidate: &DecodePlannedObu,
    next_unvalidated_following_ivf_record: &mut usize,
) -> Result<(&'a [ObuEnvelope<'a>], ObuEnvelope<'a>)> {
    for (ivf_frame_index, ivf_frame) in ivf.frames.iter().enumerate() {
        let Some(position) = ivf_frame
            .obus
            .iter()
            .position(|envelope| envelope.offset == candidate.offset())
        else {
            continue;
        };
        require_following_ivf_obu_order_through(
            ivf,
            next_unvalidated_following_ivf_record,
            ivf_frame_index,
        )?;
        let inter_envelope = ivf_frame.obus[position];
        require_inter_frame_obu(inter_envelope, "missing_inter_frame_obu")?;
        if let Some(start) = leading_record_inter_frame_unit_start(
            ivf_frame_index,
            position,
            ivf_frame.obus.as_slice(),
        ) {
            return Ok((&ivf_frame.obus[start..position], inter_envelope));
        }
        let Some(td_index) = ivf_frame.obus[..position]
            .iter()
            .rposition(|envelope| envelope.header.obu_type == ObuType::TemporalDelimiter)
        else {
            return Err(unsupported_at(
                "missing_inter_temporal_delimiter",
                candidate.offset(),
                missing_capability_message!("inter.ivf_frame_unit_order"),
            ));
        };
        return Ok((&ivf_frame.obus[td_index + 1..position], inter_envelope));
    }
    Err(unsupported_at(
        "missing_inter_ivf_obu",
        candidate.offset(),
        "the planned inter candidate offset was not found in the parsed IVF payloads",
    ))
}

pub(super) fn following_annexb_inter_envelope<'a>(
    obus: &'a [ObuEnvelope<'a>],
    candidate: &DecodePlannedObu,
    next_unvalidated_following_obu: &mut usize,
) -> Result<(&'a [ObuEnvelope<'a>], ObuEnvelope<'a>)> {
    let Some(position) = obus
        .iter()
        .position(|envelope| envelope.offset == candidate.offset())
    else {
        return Err(unsupported_at(
            "missing_inter_annexb_obu",
            candidate.offset(),
            "the planned inter candidate offset was not found in the parsed Annex B OBUs",
        ));
    };
    require_following_annexb_obu_order_through(obus, next_unvalidated_following_obu, position)?;
    let inter_envelope = obus[position];
    require_inter_frame_obu(inter_envelope, "missing_inter_frame_obu")?;
    if let Some(start) = leading_record_inter_frame_unit_start(0, position, obus) {
        return Ok((&obus[start..position], inter_envelope));
    }
    let Some(td_index) = obus[..position]
        .iter()
        .rposition(|envelope| envelope.header.obu_type == ObuType::TemporalDelimiter)
    else {
        return Err(unsupported_at(
            "missing_inter_temporal_delimiter",
            candidate.offset(),
            missing_capability_message!("inter.ivf_frame_unit_order"),
        ));
    };
    Ok((&obus[td_index + 1..position], inter_envelope))
}

pub(super) fn following_key_frame_unit<'a>(
    ivf: &'a ParsedIvfBitstream<'a>,
    candidate: &DecodePlannedObu,
    next_unvalidated_following_ivf_record: &mut usize,
) -> Result<(ObuEnvelope<'a>, &'a [ObuEnvelope<'a>], ObuEnvelope<'a>)> {
    for (ivf_frame_index, ivf_frame) in ivf.frames.iter().enumerate() {
        let Some(position) = ivf_frame
            .obus
            .iter()
            .position(|envelope| envelope.offset == candidate.offset())
        else {
            continue;
        };
        require_following_key_ivf_obu_order_through(
            ivf,
            next_unvalidated_following_ivf_record,
            ivf_frame_index,
        )?;
        let obus = ivf_frame.obus.as_slice();
        let ([_, sequence_envelope, key_envelope], _) = require_leading_frame_unit(obus)?;
        if key_envelope.offset != candidate.offset() {
            return Err(unsupported_at(
                "unexpected_key_obu_order",
                candidate.offset(),
                missing_capability_message!("frame.sequence repeated_key_frame_unit"),
            ));
        }
        let indices = minimal_frame_unit_indices(obus)?;
        if position != indices.frame {
            return Err(unsupported_at(
                "unexpected_key_obu_order",
                candidate.offset(),
                missing_capability_message!("frame.sequence repeated_key_frame_unit"),
            ));
        }
        return Ok((
            sequence_envelope,
            leading_film_grain_obus(obus)?,
            key_envelope,
        ));
    }
    Err(unsupported_at(
        "missing_key_ivf_obu",
        candidate.offset(),
        "the planned key candidate offset was not found in the parsed IVF payloads",
    ))
}

pub(super) fn following_annexb_key_frame_unit<'a>(
    obus: &'a [ObuEnvelope<'a>],
    candidate: &DecodePlannedObu,
    next_unvalidated_following_obu: &mut usize,
) -> Result<(ObuEnvelope<'a>, &'a [ObuEnvelope<'a>], ObuEnvelope<'a>)> {
    let Some(position) = obus
        .iter()
        .position(|envelope| envelope.offset == candidate.offset())
    else {
        return Err(unsupported_at(
            "missing_key_annexb_obu",
            candidate.offset(),
            "the planned key candidate offset was not found in the parsed Annex B OBUs",
        ));
    };
    let start = obus
        .iter()
        .take(position + 1)
        .rposition(|envelope| envelope.header.obu_type == ObuType::TemporalDelimiter)
        .ok_or_else(|| {
            unsupported_at(
                "missing_key_temporal_delimiter",
                candidate.offset(),
                missing_capability_message!("frame.sequence repeated_key_frame_unit"),
            )
        })?;
    require_following_annexb_obu_order_through(
        obus,
        next_unvalidated_following_obu,
        start.saturating_sub(1),
    )?;
    let frame_unit = &obus[start..=position];
    let ([_, sequence_envelope, key_envelope], _) = require_leading_frame_unit(frame_unit)?;
    if key_envelope.offset != candidate.offset() {
        return Err(unsupported_at(
            "unexpected_key_obu_order",
            candidate.offset(),
            missing_capability_message!("frame.sequence repeated_key_frame_unit"),
        ));
    }
    *next_unvalidated_following_obu = position.saturating_add(1);
    Ok((
        sequence_envelope,
        leading_film_grain_obus(frame_unit)?,
        key_envelope,
    ))
}

pub(super) fn leading_record_inter_frame_unit_start(
    ivf_frame_index: usize,
    position: usize,
    obus: &[ObuEnvelope<'_>],
) -> Option<usize> {
    if ivf_frame_index != 0 {
        return None;
    }
    let Ok((_, frame_unit_len)) = require_leading_frame_unit(obus) else {
        return None;
    };
    let mut index = frame_unit_len;
    while index <= position {
        let unit_start = index;
        index = skip_film_grain_obus(obus, index);
        let envelope = obus.get(index)?;
        if !is_inter_frame_obu(envelope.header.obu_type) {
            return None;
        }
        if index == position {
            return Some(unit_start);
        }
        index += 1;
    }
    None
}

pub(super) fn require_following_ivf_obu_order_through(
    ivf: &ParsedIvfBitstream<'_>,
    next_unvalidated_following_ivf_record: &mut usize,
    target_ivf_frame_index: usize,
) -> Result<()> {
    let validation_end = target_ivf_frame_index.saturating_add(1);
    for (ivf_frame_index, frame) in ivf
        .frames
        .iter()
        .enumerate()
        .take(validation_end)
        .skip(*next_unvalidated_following_ivf_record)
    {
        require_following_ivf_record_obu_order(frame.obus.as_slice(), ivf_frame_index)?;
    }
    *next_unvalidated_following_ivf_record =
        (*next_unvalidated_following_ivf_record).max(validation_end);
    Ok(())
}

pub(super) fn require_following_key_ivf_obu_order_through(
    ivf: &ParsedIvfBitstream<'_>,
    next_unvalidated_following_ivf_record: &mut usize,
    target_ivf_frame_index: usize,
) -> Result<()> {
    let validation_end = target_ivf_frame_index.saturating_add(1);
    for frame in ivf
        .frames
        .iter()
        .take(validation_end)
        .skip(*next_unvalidated_following_ivf_record)
    {
        require_leading_ivf_obu_order(frame.obus.as_slice())?;
    }
    *next_unvalidated_following_ivf_record =
        (*next_unvalidated_following_ivf_record).max(validation_end);
    Ok(())
}

pub(super) fn require_following_ivf_record_obu_order(
    obus: &[ObuEnvelope<'_>],
    ivf_frame_index: usize,
) -> Result<()> {
    if ivf_frame_index == 0 {
        require_leading_ivf_obu_order(obus)
    } else {
        require_inter_obu_order(obus)
    }
}

pub(super) fn require_leading_ivf_obu_order(obus: &[ObuEnvelope<'_>]) -> Result<()> {
    let (_, frame_unit_len) = require_leading_frame_unit(obus)?;
    let mut index = frame_unit_len;
    while index < obus.len() {
        index = skip_film_grain_obus(obus, index);
        let Some(envelope) = obus.get(index).copied() else {
            return Err(unsupported_at(
                "unexpected_leading_obu_after_key",
                obus.last()
                    .map_or(ByteOffset::new(0), |envelope| envelope.offset),
                missing_capability_message!("inter.ivf_frame_unit_order"),
            ));
        };
        require_inter_frame_obu(envelope, "unexpected_leading_obu_after_key")?;
        index += 1;
    }
    Ok(())
}

pub(super) fn skip_film_grain_obus(obus: &[ObuEnvelope<'_>], mut index: usize) -> usize {
    while obus
        .get(index)
        .is_some_and(|envelope| envelope.header.obu_type == ObuType::FilmGrain)
    {
        index += 1;
    }
    index
}

pub(super) fn require_inter_obu_order(obus: &[ObuEnvelope<'_>]) -> Result<()> {
    let mut index = 0;
    while index < obus.len() {
        let td_envelope = obus[index];
        if td_envelope.header.obu_type != ObuType::TemporalDelimiter {
            return Err(unsupported_at(
                "unexpected_inter_obu_order",
                td_envelope.offset,
                missing_capability_message!("inter.ivf_frame_unit_order"),
            ));
        }
        index += 1;
        index = skip_film_grain_obus(obus, index);
        let Some(frame_envelope) = obus.get(index).copied() else {
            return Err(unsupported_at(
                "unexpected_inter_obu_order",
                td_envelope.offset,
                missing_capability_message!("inter.ivf_frame_unit_order"),
            ));
        };
        require_inter_frame_obu(frame_envelope, "unexpected_inter_obu_order")?;
        index += 1;
    }
    Ok(())
}

pub(super) fn require_following_annexb_obu_order_through(
    obus: &[ObuEnvelope<'_>],
    next_unvalidated_following_obu: &mut usize,
    target_position: usize,
) -> Result<()> {
    while *next_unvalidated_following_obu <= target_position {
        let leading_frame_index = skip_film_grain_obus(obus, *next_unvalidated_following_obu);
        if leading_record_inter_frame_unit_start(0, leading_frame_index, obus).is_some() {
            let envelope = obus[leading_frame_index];
            require_inter_frame_obu(envelope, "unexpected_leading_obu_after_key")?;
            *next_unvalidated_following_obu = leading_frame_index.saturating_add(1);
            continue;
        }

        let Some(td_envelope) = obus.get(*next_unvalidated_following_obu).copied() else {
            return Err(unsupported(
                "unexpected_inter_obu_order",
                None,
                missing_capability_message!("inter.ivf_frame_unit_order"),
            ));
        };
        require_obu_type(
            td_envelope,
            ObuType::TemporalDelimiter,
            "unexpected_inter_obu_order",
        )?;
        let frame_index =
            skip_film_grain_obus(obus, next_unvalidated_following_obu.saturating_add(1));
        let Some(frame_envelope) = obus.get(frame_index).copied() else {
            return Err(unsupported_at(
                "unexpected_inter_obu_order",
                td_envelope.offset,
                missing_capability_message!("inter.ivf_frame_unit_order"),
            ));
        };
        require_inter_frame_obu(frame_envelope, "unexpected_inter_obu_order")?;
        *next_unvalidated_following_obu = frame_index.saturating_add(1);
    }
    Ok(())
}
pub(super) fn ensure_multiframe_plan_shape(plan: &DecodeStreamPlan) -> Result<()> {
    let frame_count = plan.frame_candidate_count();
    if frame_count == 0 {
        return Err(unsupported(
            "unsupported_frame_candidate_count",
            None,
            "decode runtime requires at least one selected key frame candidate",
        ));
    }
    if plan.obu_count() >= 3 {
        Ok(())
    } else {
        Err(unsupported(
            "unexpected_planned_stream_shape",
            None,
            "decode runtime requires a leading [TD, SEQ, CLK] frame unit",
        ))
    }
}
pub(super) fn require_multiframe_ivf(ivf: &ParsedIvfBitstream<'_>) -> Result<IvfHeader> {
    let Some(header) = ivf.header else {
        return Err(unsupported(
            "missing_ivf_header",
            None,
            "decode runtime requires a complete IVF header",
        ));
    };
    let parsed_frame_count = ivf.frames.len() as u64;
    let header_frame_count = u64::from(header.frame_count);
    let header_count_matches = header_frame_count == 0 || header_frame_count == parsed_frame_count;
    let all_frame_records_positive = ivf.frames.iter().all(|frame| frame.frame.size > 0);
    if header.fourcc != *b"AV02"
        || header.width == 0
        || header.height == 0
        || ivf.frames.is_empty()
        || !header_count_matches
        || !all_frame_records_positive
        || !supported_ivf_warnings(&ivf.warnings)
        || ivf.error.is_some()
    {
        return Err(unsupported(
            "unsupported_ivf_shape",
            None,
            missing_capability_message!("container.ivf_av02_frame_records"),
        ));
    }
    Ok(header)
}

pub(super) fn supported_ivf_warnings(warnings: &[IvfWarning]) -> bool {
    warnings
        .iter()
        .all(|warning| matches!(warning, IvfWarning::TrailingPartialFrameHeader { .. }))
}

pub(crate) fn require_minimal_obu_order<'a>(
    obus: &'a [ObuEnvelope<'a>],
) -> Result<[ObuEnvelope<'a>; 3]> {
    let indices = minimal_frame_unit_indices(obus)?;
    Ok([
        obus[indices.temporal_delimiter],
        obus[indices.sequence],
        obus[indices.frame],
    ])
}

pub(super) fn require_leading_frame_unit<'a>(
    obus: &'a [ObuEnvelope<'a>],
) -> Result<([ObuEnvelope<'a>; 3], usize)> {
    let frame_unit = require_minimal_obu_order(obus)?;
    require_obu_type(
        frame_unit[0],
        ObuType::TemporalDelimiter,
        "missing_temporal_delimiter",
    )?;
    require_obu_type(
        frame_unit[1],
        ObuType::SequenceHeader,
        "missing_sequence_header",
    )?;
    require_obu_type(
        frame_unit[2],
        ObuType::ClosedLoopKey,
        "missing_closed_loop_key",
    )?;
    Ok((frame_unit, minimal_frame_unit_indices(obus)?.len()))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MinimalFrameUnitIndices {
    temporal_delimiter: usize,
    sequence: usize,
    first_film_grain: usize,
    frame: usize,
}

impl MinimalFrameUnitIndices {
    pub(super) const fn len(self) -> usize {
        self.frame + 1
    }
}

pub(super) fn leading_film_grain_obus<'a>(
    obus: &'a [ObuEnvelope<'a>],
) -> Result<&'a [ObuEnvelope<'a>]> {
    let indices = minimal_frame_unit_indices(obus)?;
    Ok(&obus[indices.first_film_grain..indices.frame])
}

fn minimal_frame_unit_indices(obus: &[ObuEnvelope<'_>]) -> Result<MinimalFrameUnitIndices> {
    if obus.is_empty() {
        return Err(unsupported(
            "unexpected_obu_order",
            None,
            "decode runtime requires a leading temporal delimiter, optional operating-point metadata, sequence header, optional film-grain state, and closed-loop-key OBU",
        ));
    }
    let mut sequence_index = 1usize;
    while obus
        .get(sequence_index)
        .is_some_and(|envelope| envelope.header.obu_type == ObuType::OperatingPointSet)
    {
        sequence_index += 1;
    }
    let frame_index = sequence_index.checked_add(1).ok_or_else(|| {
        unsupported(
            "unexpected_obu_order",
            None,
            "decode runtime requires a leading temporal delimiter, optional operating-point metadata, sequence header, optional film-grain state, and closed-loop-key OBU",
        )
    })?;
    let mut frame_index = frame_index;
    while obus
        .get(frame_index)
        .is_some_and(|envelope| envelope.header.obu_type == ObuType::FilmGrain)
    {
        frame_index += 1;
    }
    if frame_index >= obus.len() {
        return Err(unsupported(
            "unexpected_obu_order",
            None,
            "decode runtime requires a leading temporal delimiter, optional operating-point metadata, sequence header, optional film-grain state, and closed-loop-key OBU",
        ));
    }
    Ok(MinimalFrameUnitIndices {
        temporal_delimiter: 0,
        sequence: sequence_index,
        first_film_grain: sequence_index + 1,
        frame: frame_index,
    })
}

pub(super) fn require_obu_type(
    envelope: ObuEnvelope<'_>,
    expected: ObuType,
    reason: &'static str,
) -> Result<()> {
    if envelope.header.obu_type == expected {
        Ok(())
    } else {
        Err(unsupported_at(
            reason,
            envelope.offset,
            missing_capability_message!("obu.order minimal_frame_unit"),
        ))
    }
}

const fn is_inter_frame_obu(obu_type: ObuType) -> bool {
    matches!(obu_type, ObuType::RegularTileGroup | ObuType::RegularTip)
}

pub(super) fn require_inter_frame_obu(
    envelope: ObuEnvelope<'_>,
    reason: &'static str,
) -> Result<()> {
    if is_inter_frame_obu(envelope.header.obu_type) {
        Ok(())
    } else {
        Err(unsupported_at(
            reason,
            envelope.offset,
            missing_capability_message!("obu.order minimal_frame_unit"),
        ))
    }
}

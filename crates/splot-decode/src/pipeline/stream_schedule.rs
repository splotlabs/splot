// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Runtime stream and frame-unit scheduling.

use super::{PipelineFrameRate, unsupported, unsupported_at};

use splot_core::annexb::ObuEnvelope;
use splot_core::ivf::IvfHeader;
use splot_core::span::ByteOffset;
use splot_core::types::ObuType;

use crate::bitstream::byte_stream::{FlatParsedBitstream, FlatParsedIvfBitstream};
use crate::error::Result;
use crate::support::capability::missing_capability_message;
use crate::{DecodePlannedObu, DecodeStreamPlan};

#[derive(Clone, Copy, Debug)]
pub(super) enum RuntimeStream<'a> {
    AnnexB {
        obus: &'a [ObuEnvelope<'a>],
    },
    Ivf {
        ivf: &'a FlatParsedIvfBitstream<'a>,
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
                .map(|frame| ivf.frame_obus(frame))
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
    parsed: &'a FlatParsedBitstream<'a>,
) -> Result<RuntimeStream<'a>> {
    match parsed {
        FlatParsedBitstream::AnnexB(partial) => {
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
        FlatParsedBitstream::Ivf(ivf) => {
            let Some(header) = ivf.header else {
                return Err(unsupported(
                    "missing_ivf_header",
                    None,
                    "decode runtime requires a complete IVF header",
                ));
            };
            Ok(RuntimeStream::Ivf { ivf, header })
        }
    }
}

pub(crate) fn following_inter_envelope<'a>(
    ivf: &'a FlatParsedIvfBitstream<'a>,
    candidate: &DecodePlannedObu,
    next_unvalidated_following_ivf_record: &mut usize,
) -> Result<(&'a [ObuEnvelope<'a>], ObuEnvelope<'a>)> {
    for (ivf_frame_index, ivf_frame) in ivf.frames.iter().enumerate() {
        let obus = ivf.frame_obus(ivf_frame);
        let Some(position) = obus
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
        let inter_envelope = obus[position];
        require_inter_frame_obu(inter_envelope, "missing_inter_frame_obu")?;
        if let Some(start) = prefix_after_previous_inter_frame(position, obus) {
            return Ok((&obus[start..position], inter_envelope));
        }
        if let Some(start) = leading_record_inter_frame_unit_start(ivf_frame_index, position, obus)
        {
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
        return Ok((&obus[td_index..position], inter_envelope));
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
    if let Some(start) = prefix_after_previous_inter_frame(position, obus) {
        return Ok((&obus[start..position], inter_envelope));
    }
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
    Ok((&obus[td_index..position], inter_envelope))
}

pub(super) fn following_key_frame_unit<'a>(
    ivf: &'a FlatParsedIvfBitstream<'a>,
    candidate: &DecodePlannedObu,
    next_unvalidated_following_ivf_record: &mut usize,
) -> Result<(ObuEnvelope<'a>, &'a [ObuEnvelope<'a>], ObuEnvelope<'a>)> {
    for (ivf_frame_index, ivf_frame) in ivf.frames.iter().enumerate() {
        let obus = ivf.frame_obus(ivf_frame);
        let Some(position) = obus
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
        let ([_, sequence_envelope, key_envelope], _) = require_key_frame_unit(obus)?;
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
        return Ok((sequence_envelope, leading_prefix_obus(obus)?, key_envelope));
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
    let ([_, sequence_envelope, key_envelope], _) = require_key_frame_unit(frame_unit)?;
    if key_envelope.offset != candidate.offset() {
        return Err(unsupported_at(
            "unexpected_key_obu_order",
            candidate.offset(),
            missing_capability_message!("frame.sequence repeated_key_frame_unit"),
        ));
    }
    *next_unvalidated_following_obu = skip_frame_suffix_obus(obus, position.saturating_add(1));
    Ok((
        sequence_envelope,
        leading_prefix_obus(frame_unit)?,
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
        index = skip_frame_prefix_obus(obus, index);
        let envelope = obus.get(index)?;
        if !is_inter_frame_obu(envelope.header.obu_type) {
            return None;
        }
        if index == position {
            return Some(unit_start);
        }
        index = skip_frame_suffix_obus(obus, index + 1);
    }
    None
}

pub(super) fn require_following_ivf_obu_order_through(
    ivf: &FlatParsedIvfBitstream<'_>,
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
        require_following_ivf_record_obu_order(ivf.frame_obus(frame), ivf_frame_index)?;
    }
    *next_unvalidated_following_ivf_record =
        (*next_unvalidated_following_ivf_record).max(validation_end);
    Ok(())
}

pub(super) fn require_following_key_ivf_obu_order_through(
    ivf: &FlatParsedIvfBitstream<'_>,
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
        require_key_ivf_obu_order(ivf.frame_obus(frame))?;
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
    require_inter_obus_after_key(obus, frame_unit_len)
}

fn require_key_ivf_obu_order(obus: &[ObuEnvelope<'_>]) -> Result<()> {
    let (_, frame_unit_len) = require_key_frame_unit(obus)?;
    require_inter_obus_after_key(obus, frame_unit_len)
}

fn require_inter_obus_after_key(obus: &[ObuEnvelope<'_>], frame_unit_len: usize) -> Result<()> {
    let mut index = frame_unit_len;
    while index < obus.len() {
        if obus[index].header.obu_type == ObuType::TemporalDelimiter {
            index += 1;
        }
        index = skip_frame_prefix_obus(obus, index);
        let Some(envelope) = obus.get(index).copied() else {
            return Err(unsupported_at(
                "unexpected_leading_obu_after_key",
                obus.last()
                    .map_or(ByteOffset::new(0), |envelope| envelope.offset),
                missing_capability_message!("inter.ivf_frame_unit_order"),
            ));
        };
        require_inter_frame_obu(envelope, "unexpected_leading_obu_after_key")?;
        index = skip_frame_suffix_obus(obus, index + 1);
    }
    Ok(())
}

pub(super) fn skip_frame_prefix_obus(obus: &[ObuEnvelope<'_>], mut index: usize) -> usize {
    while obus.get(index).is_some_and(is_frame_prefix_obu) {
        index += 1;
    }
    index
}

fn skip_frame_suffix_obus(obus: &[ObuEnvelope<'_>], mut index: usize) -> usize {
    while obus.get(index).is_some_and(is_frame_suffix_obu) {
        index += 1;
    }
    index
}

fn is_frame_prefix_obu(envelope: &ObuEnvelope<'_>) -> bool {
    matches!(
        envelope.header.obu_type,
        ObuType::SequenceHeader
            | ObuType::ContentInterpretation
            | ObuType::MultiFrameHeader
            | ObuType::BufferRemovalTiming
            | ObuType::QuantizationMatrix
            | ObuType::FilmGrain
            | ObuType::Padding
    ) || matches!(
        envelope.header.obu_type,
        ObuType::MetadataShort | ObuType::MetadataGroup
    ) && envelope
        .payload
        .first()
        .is_some_and(|first| first & 0x80 == 0)
}

fn is_frame_suffix_obu(envelope: &ObuEnvelope<'_>) -> bool {
    envelope.header.obu_type == ObuType::Padding
        || is_tile_group_continuation(envelope)
        || matches!(
            envelope.header.obu_type,
            ObuType::MetadataShort | ObuType::MetadataGroup
        ) && envelope
            .payload
            .first()
            .is_some_and(|first| first & 0x80 != 0)
}

/// A § 5.19 tile-group OBU that continues the frame unit already opened by an
/// earlier tile group, i.e. one whose leading `is_first_tile_group` bit is 0.
fn is_tile_group_continuation(envelope: &ObuEnvelope<'_>) -> bool {
    envelope.header.obu_type.is_tile_group()
        && envelope
            .payload
            .first()
            .is_some_and(|first| first & 0x80 == 0)
}

pub(super) fn require_inter_obu_order(obus: &[ObuEnvelope<'_>]) -> Result<()> {
    let mut index = 0;
    let Some(first) = obus.first().copied() else {
        return Ok(());
    };
    require_obu_type(
        first,
        ObuType::TemporalDelimiter,
        "unexpected_inter_obu_order",
    )?;
    index += 1;
    while index < obus.len() {
        if obus[index].header.obu_type == ObuType::TemporalDelimiter {
            index += 1;
        }
        index = skip_frame_prefix_obus(obus, index);
        let Some(frame_envelope) = obus.get(index).copied() else {
            return Err(unsupported_at(
                "unexpected_inter_obu_order",
                obus.last()
                    .map_or(ByteOffset::new(0), |envelope| envelope.offset),
                missing_capability_message!("inter.ivf_frame_unit_order"),
            ));
        };
        require_inter_frame_obu(frame_envelope, "unexpected_inter_obu_order")?;
        index = skip_frame_suffix_obus(obus, index + 1);
    }
    Ok(())
}

pub(super) fn require_following_annexb_obu_order_through(
    obus: &[ObuEnvelope<'_>],
    next_unvalidated_following_obu: &mut usize,
    target_position: usize,
) -> Result<()> {
    while *next_unvalidated_following_obu <= target_position {
        let leading_frame_index = skip_frame_prefix_obus(obus, *next_unvalidated_following_obu);
        if leading_record_inter_frame_unit_start(0, leading_frame_index, obus).is_some() {
            let envelope = obus[leading_frame_index];
            require_inter_frame_obu(envelope, "unexpected_leading_obu_after_key")?;
            *next_unvalidated_following_obu =
                skip_frame_suffix_obus(obus, leading_frame_index.saturating_add(1));
            continue;
        }

        if let Some(frame_envelope) = obus.get(leading_frame_index).copied()
            && is_inter_frame_obu(frame_envelope.header.obu_type)
        {
            *next_unvalidated_following_obu =
                skip_frame_suffix_obus(obus, leading_frame_index.saturating_add(1));
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
            skip_frame_prefix_obus(obus, next_unvalidated_following_obu.saturating_add(1));
        let Some(frame_envelope) = obus.get(frame_index).copied() else {
            return Err(unsupported_at(
                "unexpected_inter_obu_order",
                td_envelope.offset,
                missing_capability_message!("inter.ivf_frame_unit_order"),
            ));
        };
        require_inter_frame_obu(frame_envelope, "unexpected_inter_obu_order")?;
        *next_unvalidated_following_obu =
            skip_frame_suffix_obus(obus, frame_index.saturating_add(1));
    }
    Ok(())
}

fn prefix_after_previous_inter_frame(position: usize, obus: &[ObuEnvelope<'_>]) -> Option<usize> {
    let previous = obus[..position]
        .iter()
        .rposition(|envelope| is_inter_frame_obu(envelope.header.obu_type))?;
    Some(skip_frame_suffix_obus(obus, previous.saturating_add(1)))
}
pub(super) fn ensure_multiframe_plan_shape(plan: &DecodeStreamPlan) -> Result<()> {
    if plan.frame_candidate_count() > 0 && plan.obu_count() >= 3 {
        Ok(())
    } else {
        Err(unsupported(
            "unexpected_planned_stream_shape",
            None,
            "decode runtime requires a leading [TD, SEQ, CLK] frame unit",
        ))
    }
}
pub(crate) fn require_minimal_obu_order<'a>(
    obus: &'a [ObuEnvelope<'a>],
) -> Result<[ObuEnvelope<'a>; 3]> {
    let indices = minimal_frame_unit_indices(obus)?;
    Ok([obus[0], obus[indices.sequence], obus[indices.frame]])
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
    if !matches!(
        frame_unit[2].header.obu_type,
        ObuType::ClosedLoopKey | ObuType::OpenLoopKey | ObuType::RasFrame
    ) {
        return Err(unsupported_at(
            "missing_random_access_frame",
            frame_unit[2].offset,
            "decode runtime requires a closed-loop key, open-loop key, or RAS random-access frame",
        ));
    }
    Ok((frame_unit, minimal_frame_unit_indices(obus)?.len()))
}

fn require_key_frame_unit<'a>(
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
    if !matches!(
        frame_unit[2].header.obu_type,
        ObuType::ClosedLoopKey | ObuType::OpenLoopKey
    ) {
        return Err(unsupported_at(
            "missing_key_frame",
            frame_unit[2].offset,
            missing_capability_message!("frame.sequence repeated_key_frame_unit"),
        ));
    }
    Ok((frame_unit, minimal_frame_unit_indices(obus)?.len()))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MinimalFrameUnitIndices {
    sequence: usize,
    frame: usize,
    suffix_end: usize,
}

impl MinimalFrameUnitIndices {
    pub(super) const fn len(self) -> usize {
        self.suffix_end
    }
}

pub(super) fn leading_prefix_obus<'a>(
    obus: &'a [ObuEnvelope<'a>],
) -> Result<&'a [ObuEnvelope<'a>]> {
    let indices = minimal_frame_unit_indices(obus)?;
    Ok(&obus[..indices.frame])
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
    frame_index = skip_frame_prefix_obus(obus, frame_index);
    if frame_index >= obus.len() {
        return Err(unsupported(
            "unexpected_obu_order",
            None,
            "decode runtime requires a leading temporal delimiter, optional operating-point metadata, sequence header, optional film-grain state, and closed-loop-key OBU",
        ));
    }
    Ok(MinimalFrameUnitIndices {
        sequence: sequence_index,
        frame: frame_index,
        suffix_end: skip_frame_suffix_obus(obus, frame_index + 1),
    })
}

pub(super) fn frame_suffix_obus<'a>(
    stream: RuntimeStream<'a>,
    candidate: &DecodePlannedObu,
) -> Result<&'a [ObuEnvelope<'a>]> {
    match stream {
        RuntimeStream::AnnexB { obus } => suffix_after_candidate(obus, candidate),
        RuntimeStream::Ivf { ivf, .. } => {
            for frame in &ivf.frames {
                let obus = ivf.frame_obus(frame);
                if obus
                    .iter()
                    .any(|envelope| envelope.offset == candidate.offset())
                {
                    return suffix_after_candidate(obus, candidate);
                }
            }
            Err(unsupported_at(
                "missing_frame_suffix_candidate",
                candidate.offset(),
                "planned frame candidate was not found while resolving suffix metadata",
            ))
        }
    }
}

fn suffix_after_candidate<'a>(
    obus: &'a [ObuEnvelope<'a>],
    candidate: &DecodePlannedObu,
) -> Result<&'a [ObuEnvelope<'a>]> {
    let position = obus
        .iter()
        .position(|envelope| envelope.offset == candidate.offset())
        .ok_or_else(|| {
            unsupported_at(
                "missing_frame_suffix_candidate",
                candidate.offset(),
                "planned frame candidate was not found while resolving suffix metadata",
            )
        })?;
    let mut start = position + 1;
    while obus.get(start).is_some_and(|envelope| {
        envelope.header.obu_type == candidate.obu_type()
            && envelope
                .payload
                .first()
                .is_some_and(|first| first & 0x80 == 0)
    }) {
        start += 1;
    }
    let end = skip_frame_suffix_obus(obus, start);
    Ok(&obus[start..end])
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
    (obu_type.is_tile_group() && !matches!(obu_type, ObuType::ClosedLoopKey))
        || obu_type.is_tip_frame()
        || obu_type.is_sef()
        || matches!(obu_type, ObuType::BridgeFrame)
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

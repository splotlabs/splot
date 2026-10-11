// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Runtime stream and frame-unit scheduling.

use super::{PipelineFrameRate, unsupported, unsupported_at};

use splot_core::annexb::ObuEnvelope;
use splot_core::ivf::IvfHeader;
use splot_core::span::ByteOffset;
use splot_core::stream::BitstreamFormat;
use splot_core::types::ObuType;

use crate::bitstream::byte_stream::{SourceUnit, UnitBytes, UnitSource, recycle, runtime_obus};
use crate::bitstream::stream_plan::CandidateScan;
use crate::error::Result;
use crate::support::capability::missing_capability_message;
use crate::{DecodeLimits, DecodePlannedObu, DecodeStreamPlan};

/// The temporal units the frame loop reads frame units from, one at a time.
pub(super) struct RuntimeStream<'a> {
    source: &'a mut dyn UnitSource,
    buffers: &'a mut Vec<std::sync::Arc<Vec<u8>>>,
    limits: DecodeLimits,
    current: Option<SourceUnit>,
    /// The index of the current unit among the units read so far.
    record: usize,
    scan: CandidateScan,
}

impl<'a> RuntimeStream<'a> {
    pub(super) fn new(
        source: &'a mut dyn UnitSource,
        buffers: &'a mut Vec<std::sync::Arc<Vec<u8>>>,
        limits: DecodeLimits,
    ) -> Self {
        Self {
            source,
            buffers,
            limits,
            current: None,
            record: 0,
            scan: CandidateScan::default(),
        }
    }

    pub(super) fn ivf_header(&self) -> Option<IvfHeader> {
        self.source.ivf_header()
    }

    pub(super) fn frame_rate(&self) -> PipelineFrameRate {
        self.ivf_header().map_or(
            PipelineFrameRate::ANNEX_B_DEFAULT,
            PipelineFrameRate::from_ivf_header,
        )
    }

    /// The plan the current unit's candidates are checked against.
    pub(super) fn plan(&self) -> &'static DecodeStreamPlan {
        let ivf = self
            .current
            .as_ref()
            .is_some_and(|unit| unit.ivf_frame.is_some());
        DecodeStreamPlan::runtime(if ivf {
            BitstreamFormat::Ivf
        } else {
            BitstreamFormat::AnnexB
        })
    }

    /// Makes the next unit current; `false` at end of input.
    fn advance(&mut self) -> Result<bool> {
        let Some(unit) = self.source.next_unit(self.buffers)? else {
            return Ok(false);
        };
        if self.current.replace(unit).is_some() {
            self.record += 1;
        }
        Ok(true)
    }

    /// The first unit, or `None` when the input holds none.
    pub(super) fn first_unit(&mut self) -> Result<Option<UnitBytes>> {
        Ok(self
            .advance()?
            .then(|| self.current.as_ref().map(|unit| unit.bytes.clone()))
            .flatten())
    }

    /// The next frame candidate in the current unit, planned again.
    fn next_in_unit(&mut self) -> Result<Option<(DecodePlannedObu, UnitBytes)>> {
        let Some(unit) = &self.current else {
            return Ok(None);
        };
        Ok(self
            .scan
            .next(unit.bytes.view(), unit.ivf_frame, self.limits)?
            .map(|found| (found, unit.bytes.clone())))
    }

    /// The leading unit's first frame candidate. A scan error comes back at
    /// once; a missing candidate waits for the frame-unit checks.
    pub(super) fn first_candidate(&mut self) -> Result<Result<DecodePlannedObu>> {
        Ok(self
            .next_in_unit()?
            .map(|(candidate, _)| candidate)
            .ok_or_else(|| {
                unsupported(
                    "missing_frame_candidate",
                    None,
                    "decode runtime requires one selected key frame candidate",
                )
            }))
    }

    /// The next frame candidate and the unit holding it; `None` at end of
    /// input. Each unit read on the way is checked as a key frame unit when
    /// the candidate is a closed-loop key, else in inter order. Units after
    /// the last candidate are not checked.
    pub(super) fn candidate(
        &mut self,
        storage: &mut Vec<ObuEnvelope<'static>>,
    ) -> Result<Option<(DecodePlannedObu, UnitBytes)>> {
        let mut skipped: Vec<UnitBytes> = Vec::new();
        let mut fresh = false;
        loop {
            if let Some((candidate, unit)) = self.next_in_unit()? {
                let check = if candidate.obu_type() == ObuType::ClosedLoopKey {
                    require_key_ivf_obu_order
                } else {
                    require_inter_obu_order
                };
                for skipped in &skipped {
                    check_unit(skipped, storage, check)?;
                }
                if fresh {
                    check_unit(&unit, storage, check)?;
                }
                return Ok(Some((candidate, unit)));
            }
            if fresh && let Some(unit) = &self.current {
                skipped.push(unit.bytes.clone());
            }
            if !self.advance()? {
                return Ok(None);
            }
            fresh = true;
        }
    }

    /// The OBUs of `unit`, parsed into `buffer`.
    pub(super) fn obus<'s, 'u>(
        unit: &'u UnitBytes,
        buffer: &'s mut Vec<ObuEnvelope<'u>>,
    ) -> Result<&'s [ObuEnvelope<'u>]> {
        runtime_obus(unit.view(), buffer)?;
        Ok(buffer)
    }

    pub(super) fn inter_envelope<'o>(
        &self,
        obus: &'o [ObuEnvelope<'o>],
        candidate: &DecodePlannedObu,
    ) -> Result<(&'o [ObuEnvelope<'o>], ObuEnvelope<'o>)> {
        ivf_inter_envelope(obus, self.record, candidate)
    }
}

fn check_unit(
    unit: &UnitBytes,
    storage: &mut Vec<ObuEnvelope<'static>>,
    check: fn(&[ObuEnvelope<'_>]) -> Result<()>,
) -> Result<()> {
    let mut obus = recycle(core::mem::take(storage));
    runtime_obus(unit.view(), &mut obus)?;
    check(&obus)?;
    *storage = recycle(obus);
    Ok(())
}

fn missing_inter_ivf_obu(offset: Option<ByteOffset>) -> crate::DecodeError {
    unsupported(
        "missing_inter_ivf_obu",
        offset,
        "the planned inter candidate offset was not found in the parsed IVF payloads",
    )
}

fn missing_key_ivf_obu(offset: Option<ByteOffset>) -> crate::DecodeError {
    unsupported(
        "missing_key_ivf_obu",
        offset,
        "the planned key candidate offset was not found in the parsed IVF payloads",
    )
}

/// The prefix and frame OBU of an inter `candidate` inside IVF record `record`.
pub(crate) fn ivf_inter_envelope<'a>(
    obus: &'a [ObuEnvelope<'a>],
    record: usize,
    candidate: &DecodePlannedObu,
) -> Result<(&'a [ObuEnvelope<'a>], ObuEnvelope<'a>)> {
    let position = obus
        .iter()
        .position(|envelope| envelope.offset == candidate.offset())
        .ok_or_else(|| missing_inter_ivf_obu(Some(candidate.offset())))?;
    inter_frame_envelope(obus, position, record)
}

fn inter_frame_envelope<'a>(
    obus: &'a [ObuEnvelope<'a>],
    position: usize,
    record_index: usize,
) -> Result<(&'a [ObuEnvelope<'a>], ObuEnvelope<'a>)> {
    let inter_envelope = obus[position];
    require_inter_frame_obu(inter_envelope, "missing_inter_frame_obu")?;
    if let Some(start) = prefix_after_previous_inter_frame(position, obus) {
        return Ok((&obus[start..position], inter_envelope));
    }
    if let Some(start) = leading_record_inter_frame_unit_start(record_index, position, obus) {
        return Ok((&obus[start..position], inter_envelope));
    }
    let Some(td_index) = obus[..position]
        .iter()
        .rposition(|envelope| envelope.header.obu_type == ObuType::TemporalDelimiter)
    else {
        return Err(unsupported_at(
            "missing_inter_temporal_delimiter",
            inter_envelope.offset,
            missing_capability_message!("inter.ivf_frame_unit_order"),
        ));
    };
    Ok((&obus[td_index..position], inter_envelope))
}

pub(super) fn ivf_key_frame_unit<'a>(
    obus: &'a [ObuEnvelope<'a>],
    candidate: &DecodePlannedObu,
) -> Result<(ObuEnvelope<'a>, &'a [ObuEnvelope<'a>], ObuEnvelope<'a>)> {
    let position = obus
        .iter()
        .position(|envelope| envelope.offset == candidate.offset())
        .ok_or_else(|| missing_key_ivf_obu(Some(candidate.offset())))?;
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
    Ok((sequence_envelope, leading_prefix_obus(obus)?, key_envelope))
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

#[cfg(test)]
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

fn prefix_after_previous_inter_frame(position: usize, obus: &[ObuEnvelope<'_>]) -> Option<usize> {
    let previous = obus[..position]
        .iter()
        .rposition(|envelope| is_inter_frame_obu(envelope.header.obu_type))?;
    Some(skip_frame_suffix_obus(obus, previous.saturating_add(1)))
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

pub(super) fn suffix_after_candidate<'a>(
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

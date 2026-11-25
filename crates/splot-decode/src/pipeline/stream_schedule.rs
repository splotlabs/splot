// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Runtime stream and frame-unit scheduling.

use super::{PipelineFrameRate, unsupported, unsupported_at};

use splot_core::annexb::ObuEnvelope;
use splot_core::ivf::IvfHeader;
use splot_core::span::ByteOffset;
use splot_core::types::{GLOBAL_XLAYER_ID, ObuType};

use crate::bitstream::byte_stream::{SourceUnit, UnitBytes, UnitSource, recycle, runtime_obus};
use crate::bitstream::stream_plan::{ActiveLayerFilter, CandidateScan};
use crate::error::Result;
use crate::support::capability::missing_capability_message;
use crate::{DecodeLimits, DecodeOptions, DecodePlannedObu};

type OnSkippedTuFn<'a> = dyn FnMut(&[ObuEnvelope<'_>], bool, bool) -> Result<()> + 'a;

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
        options: &DecodeOptions,
    ) -> Self {
        Self {
            source,
            buffers,
            limits: options.limits(),
            current: None,
            record: 0,
            scan: CandidateScan::new_with_selections(
                options.temporal_layer_selection(),
                options.embedded_layer_selection(),
                options.extended_layer_selection(),
                options.operating_point_selection(),
            ),
        }
    }

    pub(super) const fn layer_filter(&self) -> &ActiveLayerFilter {
        self.scan.layer_filter()
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

    /// The caller data of the current unit.
    pub(super) fn props(&self) -> crate::DataProps {
        self.current
            .as_ref()
            .map_or_else(crate::DataProps::default, |unit| unit.props)
    }

    /// Makes the next unit current; `false` at end of input. While the input
    /// is idle, `settle` hands over the frames still decoding on request.
    fn advance(&mut self, settle: &mut dyn FnMut() -> Result<()>) -> Result<bool> {
        while self.source.await_input() {
            settle()?;
        }
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
            .advance(&mut || Ok(()))?
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

    pub(super) fn unit_only_filtered_unselected_xlayer<'b>(
        &self,
        unit: &'b UnitBytes,
        buffer: &mut Vec<ObuEnvelope<'b>>,
    ) -> bool {
        let filter = self.scan.layer_filter();
        if filter.extended_filtering_disabled() {
            return false;
        }
        if runtime_obus(unit.view(), buffer).is_err() {
            return false;
        }
        if !buffer
            .first()
            .is_some_and(|env| env.header.obu_type == ObuType::TemporalDelimiter)
        {
            return false;
        }
        let mut saw_unselected_xlayer_frame = false;
        for env in buffer.iter() {
            if is_frame_obu(env.header.obu_type) {
                if filter.is_xlayer_selected(env.header.extended_layer_id) {
                    return false;
                }
                saw_unselected_xlayer_frame = true;
            }
        }
        saw_unselected_xlayer_frame
    }

    pub(super) fn has_unfiltered_multistream_rap(&self, unit: &UnitBytes) -> Result<bool> {
        if self.scan.layer_filter().extended_filtering_disabled() {
            return Ok(false);
        }
        let mut obus = Vec::new();
        runtime_obus(unit.view(), &mut obus)?;
        validate_tu_multistream_order(&obus)?;
        Ok(find_multistream_leading_rap_unit(&obus).is_some())
    }

    /// The next frame candidate and the unit holding it; `None` at end of
    /// input. Each unit read on the way is checked as a key frame unit when
    /// the candidate is a closed-loop key, else in inter order. Units after
    /// the last candidate are not checked. `settle` is for [`Self::advance`].
    pub(super) fn candidate(
        &mut self,
        storage: &mut Vec<ObuEnvelope<'static>>,
        settle: &mut dyn FnMut() -> Result<()>,
        on_skipped: &mut OnSkippedTuFn<'_>,
    ) -> Result<Option<(DecodePlannedObu, UnitBytes)>> {
        let mut skipped: Vec<UnitBytes> = Vec::new();
        let mut fresh = false;
        loop {
            if let Some((candidate, unit)) = self.next_in_unit()? {
                let filter = self.scan.layer_filter();
                for skipped in &skipped {
                    process_skipped_unit(skipped, storage, filter, on_skipped)?;
                }
                if fresh {
                    check_unit(&unit, storage, candidate.obu_type())?;
                }
                return Ok(Some((candidate, unit)));
            }
            if fresh && let Some(unit) = &self.current {
                skipped.push(unit.bytes.clone());
            }
            if !self.advance(settle)? {
                let eof_offset = self.current.as_ref().map_or(ByteOffset::new(0), |unit| {
                    ByteOffset::new(unit.bytes.view().end())
                });
                self.scan.finish_eof(eof_offset)?;
                return Ok(None);
            }
            fresh = true;
        }
    }

    /// The selected OBUs of `unit`, parsed into `buffer`.
    pub(super) fn obus<'s, 'u>(
        &self,
        unit: &'u UnitBytes,
        buffer: &'s mut Vec<ObuEnvelope<'u>>,
    ) -> Result<&'s [ObuEnvelope<'u>]> {
        runtime_obus(unit.view(), buffer)?;
        let filter = self.scan.layer_filter();
        buffer.retain(|envelope| filter.is_obu_selected(envelope.header));
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

fn first_unit_frame_is_key(obus: &[ObuEnvelope<'_>]) -> bool {
    let Some(first) = obus.first() else {
        return false;
    };
    if first.header.obu_type != ObuType::TemporalDelimiter {
        return false;
    }
    let mut index = 1usize;
    while obus.get(index).is_some_and(is_pre_sequence_obu) {
        index += 1;
    }
    if obus
        .get(index)
        .is_some_and(|envelope| envelope.header.obu_type == ObuType::SequenceHeader)
    {
        index += 1;
    }
    index = skip_frame_prefix_obus(obus, index);
    obus.get(index)
        .is_some_and(|envelope| envelope.header.obu_type == ObuType::ClosedLoopKey)
}

fn unit_order_check(
    obus: &[ObuEnvelope<'_>],
    candidate_obu: Option<ObuType>,
) -> fn(&[ObuEnvelope<'_>]) -> Result<()> {
    let has_prior_inter_frame = obus
        .iter()
        .take_while(|envelope| envelope.header.obu_type != ObuType::ClosedLoopKey)
        .any(|envelope| is_inter_frame_obu(envelope.header.obu_type));
    if (candidate_obu == Some(ObuType::ClosedLoopKey) && !has_prior_inter_frame)
        || first_unit_frame_is_key(obus)
    {
        require_key_ivf_obu_order
    } else {
        require_inter_obu_order
    }
}

fn validate_tu_multistream_order(obus: &[ObuEnvelope<'_>]) -> Result<()> {
    let mut saw_msdo = false;
    let mut saw_after_msdo = false;
    let mut last_celu_xlayer: Option<u8> = None;
    for envelope in obus {
        let header = envelope.header;
        match header.obu_type {
            ObuType::TemporalDelimiter => {
                saw_msdo = false;
                saw_after_msdo = false;
                last_celu_xlayer = None;
            }
            ObuType::Msdo => {
                if saw_msdo || saw_after_msdo {
                    return Err(unsupported_at(
                        "unexpected_msdo_order",
                        envelope.offset,
                        "OBU_MSDO must appear at most once immediately after OBU_TEMPORAL_DELIMITER",
                    ));
                }
                saw_msdo = true;
            }
            ObuType::Padding => {
                if header.extended_layer_id != GLOBAL_XLAYER_ID {
                    let id = header.extended_layer_id.get();
                    if let Some(prev) = last_celu_xlayer
                        && id < prev
                    {
                        return Err(unsupported_at(
                            "unexpected_xlayer_order",
                            envelope.offset,
                            "coded extended layer units must appear in ascending obu_xlayer_id order",
                        ));
                    }
                    last_celu_xlayer = Some(id);
                    saw_after_msdo = true;
                }
            }
            _ if header.extended_layer_id == GLOBAL_XLAYER_ID => {
                saw_after_msdo = true;
            }
            _ => {
                let id = header.extended_layer_id.get();
                if let Some(prev) = last_celu_xlayer
                    && id < prev
                {
                    return Err(unsupported_at(
                        "unexpected_xlayer_order",
                        envelope.offset,
                        "coded extended layer units must appear in ascending obu_xlayer_id order",
                    ));
                }
                last_celu_xlayer = Some(id);
                saw_after_msdo = true;
            }
        }
    }
    Ok(())
}

fn check_unit(
    unit: &UnitBytes,
    storage: &mut Vec<ObuEnvelope<'static>>,
    candidate_obu: ObuType,
) -> Result<()> {
    let mut obus = recycle(core::mem::take(storage));
    runtime_obus(unit.view(), &mut obus)?;
    let check = unit_order_check(&obus, Some(candidate_obu));
    check(&obus)?;
    validate_tu_multistream_order(&obus)?;
    *storage = recycle(obus);
    Ok(())
}

pub(super) fn unit_rap_flags(unit: &UnitBytes) -> Result<(bool, bool)> {
    let mut obus = Vec::new();
    runtime_obus(unit.view(), &mut obus)?;
    validate_tu_multistream_order(&obus)?;
    Ok(tu_rap_flags(&obus))
}

fn tu_rap_flags(obus: &[ObuEnvelope<'_>]) -> (bool, bool) {
    let mut has_rap = false;
    let mut has_clk = false;
    for env in obus {
        match env.header.obu_type {
            ObuType::ClosedLoopKey => {
                has_rap = true;
                has_clk = true;
            }
            ObuType::OpenLoopKey | ObuType::RasFrame => {
                has_rap = true;
            }
            _ => {}
        }
    }
    (has_rap, has_clk)
}

fn process_skipped_unit(
    unit: &UnitBytes,
    storage: &mut Vec<ObuEnvelope<'static>>,
    filter: &ActiveLayerFilter,
    on_skipped: &mut OnSkippedTuFn<'_>,
) -> Result<()> {
    let mut obus = recycle(core::mem::take(storage));
    runtime_obus(unit.view(), &mut obus)?;
    let check = unit_order_check(&obus, None);
    let check_result = check(&obus).and_then(|()| validate_tu_multistream_order(&obus));
    if check_result.is_ok() {
        let (tu_has_rap, tu_has_clk) = tu_rap_flags(&obus);
        obus.retain(|envelope| filter.is_obu_selected(envelope.header));
        let callback_result = on_skipped(&obus, tu_has_rap, tu_has_clk);
        *storage = recycle(obus);
        return callback_result;
    }
    *storage = recycle(obus);
    check_result
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
    let key_envelope = obus[position];
    if key_envelope.header.obu_type != ObuType::ClosedLoopKey {
        return Err(unsupported_at(
            "unexpected_key_obu_order",
            candidate.offset(),
            missing_capability_message!("frame.sequence repeated_key_frame_unit"),
        ));
    }
    if let Some(start) = prefix_after_previous_inter_frame(position, obus) {
        let sequence_envelope = obus[..position]
            .iter()
            .rev()
            .take_while(|envelope| envelope.header.obu_type != ObuType::TemporalDelimiter)
            .find(|envelope| {
                envelope.header.obu_type == ObuType::SequenceHeader
                    && envelope.header.extended_layer_id == key_envelope.header.extended_layer_id
            })
            .copied()
            .ok_or_else(|| {
                unsupported_at(
                    "missing_sequence_header",
                    candidate.offset(),
                    missing_capability_message!("obu.order minimal_frame_unit"),
                )
            })?;
        return Ok((sequence_envelope, &obus[start..position], key_envelope));
    }
    let ([_, sequence_envelope, first_key], indices) = require_key_frame_unit(obus)?;
    if first_key.offset != candidate.offset() {
        return Err(unsupported_at(
            "unexpected_key_obu_order",
            candidate.offset(),
            missing_capability_message!("frame.sequence repeated_key_frame_unit"),
        ));
    }
    Ok((sequence_envelope, &obus[..indices.frame], first_key))
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
        if !is_frame_obu(envelope.header.obu_type) {
            return None;
        }
        if index == position {
            return Some(unit_start);
        }
        index = skip_frame_suffix_obus(obus, index + 1);
    }
    None
}

fn is_trailing_metadata_or_padding(envelope: &ObuEnvelope<'_>) -> bool {
    matches!(
        envelope.header.obu_type,
        ObuType::MetadataShort | ObuType::MetadataGroup | ObuType::Padding
    )
}

pub(super) fn require_key_ivf_obu_order(obus: &[ObuEnvelope<'_>]) -> Result<()> {
    let (_, indices) = require_key_frame_unit(obus)?;
    if obus[indices.suffix_end..]
        .iter()
        .all(is_trailing_metadata_or_padding)
    {
        return Ok(());
    }
    require_inter_frame_units(obus, indices.suffix_end, "unexpected_leading_obu_after_key")
}

pub(super) fn require_inter_frame_units(
    obus: &[ObuEnvelope<'_>],
    mut index: usize,
    reason: &'static str,
) -> Result<()> {
    while index < obus.len() {
        if obus[index].header.obu_type == ObuType::TemporalDelimiter {
            index += 1;
        }
        index = skip_frame_prefix_obus(obus, index);
        let Some(envelope) = obus.get(index).copied() else {
            return Err(unsupported_at(
                reason,
                obus.last()
                    .map_or(ByteOffset::new(0), |envelope| envelope.offset),
                missing_capability_message!("inter.ivf_frame_unit_order"),
            ));
        };
        if envelope.header.obu_type == ObuType::ClosedLoopKey {
            let has_sequence = obus[..index]
                .iter()
                .rev()
                .take_while(|prior| prior.header.obu_type != ObuType::TemporalDelimiter)
                .any(|prior| {
                    prior.header.obu_type == ObuType::SequenceHeader
                        && prior.header.extended_layer_id == envelope.header.extended_layer_id
                });
            if !has_sequence {
                return Err(unsupported_at(
                    reason,
                    envelope.offset,
                    missing_capability_message!("inter.ivf_frame_unit_order"),
                ));
            }
        } else {
            require_inter_frame_obu(envelope, reason)?;
        }
        index = skip_frame_suffix_obus(obus, index + 1);
        if obus[index..].iter().all(is_trailing_metadata_or_padding) {
            return Ok(());
        }
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
    is_pre_sequence_obu(envelope)
        || matches!(
            envelope.header.obu_type,
            ObuType::SequenceHeader
                | ObuType::ContentInterpretation
                | ObuType::MultiFrameHeader
                | ObuType::BufferRemovalTiming
                | ObuType::QuantizationMatrix
                | ObuType::FilmGrain
        )
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
    const REASON: &str = "unexpected_inter_obu_order";
    let Some(first) = obus.first().copied() else {
        return Ok(());
    };
    require_obu_type(first, ObuType::TemporalDelimiter, REASON)?;
    require_inter_frame_units(obus, 1, REASON)
}

fn prefix_after_previous_inter_frame(position: usize, obus: &[ObuEnvelope<'_>]) -> Option<usize> {
    let previous = obus[..position]
        .iter()
        .rposition(|envelope| is_frame_obu(envelope.header.obu_type))?;
    Some(skip_frame_suffix_obus(obus, previous.saturating_add(1)))
}
#[cfg(test)]
pub(crate) fn require_minimal_obu_order<'a>(
    obus: &'a [ObuEnvelope<'a>],
) -> Result<[ObuEnvelope<'a>; 3]> {
    let indices = minimal_frame_unit_indices(obus)?;
    Ok([obus[0], obus[indices.sequence], obus[indices.frame]])
}

pub(super) fn require_leading_frame_unit<'a>(
    obus: &'a [ObuEnvelope<'a>],
) -> Result<([ObuEnvelope<'a>; 3], usize)> {
    validate_tu_multistream_order(obus)?;
    match require_frame_unit(
        obus,
        &[
            ObuType::ClosedLoopKey,
            ObuType::OpenLoopKey,
            ObuType::RasFrame,
        ],
        "missing_random_access_frame",
        "decode runtime requires a closed-loop key, open-loop key, or RAS random-access frame",
    ) {
        Ok((frame_unit, indices)) => Ok((frame_unit, indices.suffix_end)),
        Err(error) => {
            if let Some(found) = find_multistream_leading_rap_unit(obus) {
                Ok((found, obus.len()))
            } else {
                Err(error)
            }
        }
    }
}

fn find_multistream_leading_rap_unit<'a>(
    obus: &'a [ObuEnvelope<'a>],
) -> Option<[ObuEnvelope<'a>; 3]> {
    let first = *obus.first()?;
    if first.header.obu_type != ObuType::TemporalDelimiter {
        return None;
    }
    let has_multistream_hls = obus.iter().any(|env| {
        env.header.extended_layer_id == GLOBAL_XLAYER_ID
            && matches!(
                env.header.obu_type,
                ObuType::Msdo | ObuType::LayerConfigurationRecord
            )
    });
    if !has_multistream_hls {
        return None;
    }
    for (idx, seq) in obus.iter().enumerate().skip(1) {
        if seq.header.obu_type != ObuType::SequenceHeader
            || seq.header.extended_layer_id == GLOBAL_XLAYER_ID
        {
            continue;
        }
        let xlayer = seq.header.extended_layer_id;
        let frame_idx = skip_frame_prefix_obus(obus, idx + 1);
        if let Some(frame) = obus.get(frame_idx).copied()
            && frame.header.extended_layer_id == xlayer
            && matches!(
                frame.header.obu_type,
                ObuType::ClosedLoopKey | ObuType::OpenLoopKey | ObuType::RasFrame
            )
        {
            return Some([first, *seq, frame]);
        }
    }
    None
}

fn require_key_frame_unit<'a>(
    obus: &'a [ObuEnvelope<'a>],
) -> Result<([ObuEnvelope<'a>; 3], MinimalFrameUnitIndices)> {
    require_frame_unit(
        obus,
        &[ObuType::ClosedLoopKey, ObuType::OpenLoopKey],
        "missing_key_frame",
        missing_capability_message!("frame.sequence repeated_key_frame_unit"),
    )
}

fn require_frame_unit<'a>(
    obus: &'a [ObuEnvelope<'a>],
    frames: &[ObuType],
    reason: &'static str,
    message: &'static str,
) -> Result<([ObuEnvelope<'a>; 3], MinimalFrameUnitIndices)> {
    let indices = minimal_frame_unit_indices(obus)?;
    let frame_unit = [obus[0], obus[indices.sequence], obus[indices.frame]];
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
    if !frames.contains(&frame_unit[2].header.obu_type) {
        return Err(unsupported_at(reason, frame_unit[2].offset, message));
    }
    Ok((frame_unit, indices))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MinimalFrameUnitIndices {
    sequence: usize,
    frame: usize,
    suffix_end: usize,
}

#[cfg(test)]
pub(super) fn leading_prefix_obus<'a>(
    obus: &'a [ObuEnvelope<'a>],
) -> Result<&'a [ObuEnvelope<'a>]> {
    let indices = minimal_frame_unit_indices(obus)?;
    Ok(&obus[..indices.frame])
}

fn is_pre_sequence_obu(envelope: &ObuEnvelope<'_>) -> bool {
    matches!(
        envelope.header.obu_type,
        ObuType::Msdo
            | ObuType::LayerConfigurationRecord
            | ObuType::OperatingPointSet
            | ObuType::AtlasSegment
            | ObuType::Padding
    ) || matches!(
        envelope.header.obu_type,
        ObuType::MetadataShort | ObuType::MetadataGroup
    ) && envelope
        .payload
        .first()
        .is_some_and(|first| first & 0x80 == 0)
}

fn minimal_frame_unit_indices(obus: &[ObuEnvelope<'_>]) -> Result<MinimalFrameUnitIndices> {
    let mut sequence = 1usize;
    while obus.get(sequence).is_some_and(is_pre_sequence_obu) {
        sequence += 1;
    }
    let frame = skip_frame_prefix_obus(obus, sequence + 1);
    if frame >= obus.len() {
        return Err(unsupported(
            "unexpected_obu_order",
            None,
            "decode runtime requires a leading temporal delimiter, optional operating-point metadata, sequence header, optional film-grain state, and closed-loop-key OBU",
        ));
    }
    Ok(MinimalFrameUnitIndices {
        sequence,
        frame,
        suffix_end: skip_frame_suffix_obus(obus, frame + 1),
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
            && envelope.header.extended_layer_id == candidate.header().extended_layer_id
            && envelope.header.embedded_layer_id == candidate.header().embedded_layer_id
            && envelope.header.temporal_layer_id == candidate.header().temporal_layer_id
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

const fn is_frame_obu(obu_type: ObuType) -> bool {
    is_inter_frame_obu(obu_type) || matches!(obu_type, ObuType::ClosedLoopKey)
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

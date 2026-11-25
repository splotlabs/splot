// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Bounded stream planning for raw Annex B and IVF inputs, and the
//! forward-only temporal-unit source the decode loop reads them through.
//!
//! Feature tracking: `DECODE-BYTE-STREAM-PLANNER`, `DECODE-STREAMING-API`.

use std::io::{Read, Take};
use std::sync::Arc;

use splot_core::annexb::{AnnexBObuCursor, ObuEnvelope, PartialParse};
use splot_core::ivf::{IVF_FRAME_HEADER_SIZE, IvfError, IvfFrame, IvfHeader, is_ivf};
use splot_core::span::ByteOffset;
use splot_core::stream::BitstreamFormat;
use splot_core::stream_reader::{ReaderError, StreamUnit, TemporalUnitReader};

use crate::bitstream::stream_plan::{
    ActiveLayerFilter, DecodeIvfFrameContext, DecodeLayerSelection, DecodeStreamPlan,
    DecodeUnsupportedStructure, IvfPlanner, PlanBuilder, classify_obu, issue_from_ivf_error,
    issue_from_unsupported_ivf_codec, plan_annex_b,
};
use crate::error::{DecodeError, Result};
use crate::{DecodeLimitName, DecodeLimits, DecodeOptions};

/// Input bytes addressed by absolute stream offset: the whole input, or one
/// temporal unit that starts at `base`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SourceBytes<'a> {
    bytes: &'a [u8],
    base: u64,
}

impl<'a> SourceBytes<'a> {
    pub(crate) const fn new(bytes: &'a [u8], base: u64) -> Self {
        Self { bytes, base }
    }

    pub(crate) const fn bytes(self) -> &'a [u8] {
        self.bytes
    }

    pub(crate) const fn base(self) -> u64 {
        self.base
    }

    /// The bytes at absolute offsets `start..end`, if this window holds them.
    pub(crate) fn get(self, start: u64, end: u64) -> Option<&'a [u8]> {
        let start = usize::try_from(start.checked_sub(self.base)?).ok()?;
        let end = usize::try_from(end.checked_sub(self.base)?).ok()?;
        self.bytes.get(start..end)
    }

    pub(crate) fn end(self) -> u64 {
        self.base.saturating_add(self.bytes.len() as u64)
    }
}

impl<'a> From<&'a [u8]> for SourceBytes<'a> {
    fn from(bytes: &'a [u8]) -> Self {
        Self::new(bytes, 0)
    }
}

/// One temporal unit's bytes, owned and shared with the tasks that parse it.
#[derive(Clone, Debug)]
pub(crate) struct UnitBytes {
    bytes: Arc<Vec<u8>>,
    base: u64,
}

impl UnitBytes {
    pub(crate) const fn new(bytes: Arc<Vec<u8>>, base: u64) -> Self {
        Self { bytes, base }
    }

    pub(crate) fn view(&self) -> SourceBytes<'_> {
        SourceBytes::new(&self.bytes, self.base)
    }
}

pub(crate) fn plan_byte_stream(bytes: &[u8], options: &DecodeOptions) -> Result<DecodeStreamPlan> {
    options.validate_selections()?;
    let limits = options.limits();
    let input_len = bytes.len() as u64;
    limits.ensure(DecodeLimitName::MaxInputBytes, input_len)?;
    if is_ivf(bytes) {
        return plan_ivf(bytes, options);
    }
    plan_annex_b(
        &parse_bounded_annex_b(bytes, options)?,
        input_len,
        options,
        true,
    )
}

/// One byte past the input limit: reading it proves the input is too long.
fn input_cap(limits: DecodeLimits) -> u64 {
    limits
        .max_input_bytes()
        .max_value()
        .map_or(u64::MAX, |max| max.saturating_add(1))
}

fn plan_ivf(bytes: &[u8], options: &DecodeOptions) -> Result<DecodeStreamPlan> {
    let limits = options.limits();
    let mut units = TemporalUnitReader::with_max_unit_bytes(bytes, usize::MAX);
    let mut builder = PlanBuilder::new(BitstreamFormat::Ivf, bytes.len() as u64, options, true);
    let mut planner = IvfPlanner::default();
    let mut counts = (0u64, 0u64);
    let mut first_unsupported = None;
    let mut selected_layer = None;
    let mut layer_filter = ActiveLayerFilter::from_options(options);
    let mut spare = Vec::new();
    let mut warning = None;
    let error = loop {
        let unit = match units.next_unit() {
            Ok(unit) => unit,
            Err(ReaderError::Ivf(error)) => {
                if let IvfError::TruncatedFramePayload { frame_index, .. } = error {
                    limits.ensure(DecodeLimitName::MaxIvfFrameRecords, frame_index as u64 + 1)?;
                }
                break Some(error);
            }
            Err(error) => return Err(DecodeError::reader(error)),
        };
        let Some(StreamUnit::IvfFrame {
            index,
            pts,
            payload_offset,
            payload,
        }) = unit
        else {
            if let Some(StreamUnit::IvfWarning(found)) = unit {
                warning = Some(found);
            }
            break None;
        };
        limits.ensure(DecodeLimitName::MaxIvfFrameRecords, index as u64 + 1)?;
        let mut obus = recycle(core::mem::take(&mut spare));
        let frame_error = parse_bounded_annex_b_at(
            payload,
            payload_offset,
            limits,
            &mut selected_layer,
            &mut layer_filter,
            &mut counts.0,
            &mut counts.1,
            &mut first_unsupported,
            &mut obus,
        )?;
        let frame = IvfFrame {
            index,
            header_offset: ByteOffset::new(
                payload_offset
                    .get()
                    .saturating_sub(IVF_FRAME_HEADER_SIZE as u64),
            ),
            payload_offset,
            size: payload.len() as u32,
            pts,
            payload,
        };
        planner.push_frame(&mut builder, frame, &obus, frame_error.as_ref());
        spare = recycle(obus);
        if frame_error.is_some() {
            break None;
        }
    };
    let header = units.ivf_header();
    planner.finish(builder, header, warning.as_slice(), error.as_ref())
}

/// Empties `obus` and keeps its storage for envelopes of another lifetime.
pub(crate) fn recycle<'b>(mut obus: Vec<ObuEnvelope<'_>>) -> Vec<ObuEnvelope<'b>> {
    obus.clear();
    obus.into_iter().map_while(|_| None).collect()
}

/// Buffers a decode keeps for the next decode on its context to read input with.
#[derive(Default)]
pub(crate) struct InputScratch {
    pub(crate) records: Vec<Arc<Vec<u8>>>,
    pub(crate) obus: Vec<ObuEnvelope<'static>>,
}

/// Copies `bytes` into a buffer of `buffers` that no task still reads.
pub(crate) fn fill_buffer(buffers: &mut Vec<Arc<Vec<u8>>>, bytes: &[u8]) -> Arc<Vec<u8>> {
    let index = buffers
        .iter_mut()
        .position(|buffer| Arc::get_mut(buffer).is_some())
        .unwrap_or_else(|| {
            buffers.push(Arc::default());
            buffers.len() - 1
        });
    let buffer = &mut buffers[index];
    if let Some(stored) = Arc::get_mut(buffer) {
        stored.clear();
        stored.extend_from_slice(bytes);
    }
    Arc::clone(buffer)
}

/// One non-empty temporal unit: an IVF frame record, or the Annex B OBUs from
/// one temporal delimiter up to the next.
#[derive(Debug)]
pub(crate) struct SourceUnit {
    pub(crate) bytes: UnitBytes,
    pub(crate) ivf_frame: Option<DecodeIvfFrameContext>,
    pub(crate) props: crate::DataProps,
}

/// Where the decode loop reads its temporal units from.
pub(crate) trait UnitSource: Send {
    /// The next non-empty temporal unit, copied into a free buffer of
    /// `buffers`; `None` at end of input.
    fn next_unit(&mut self, buffers: &mut Vec<Arc<Vec<u8>>>) -> Result<Option<SourceUnit>>;

    /// The IVF file header, once the first unit is read.
    fn ivf_header(&self) -> Option<IvfHeader>;

    /// Waits while no unit is ready. Returns `true` when the caller wants the
    /// pictures of the frames still decoding before the next unit comes, and
    /// `false` once [`Self::next_unit`] can answer.
    fn await_input(&mut self) -> bool {
        false
    }
}

/// Reads IVF or Annex B input forward only, one temporal unit at a time.
pub(crate) struct ReaderUnits<R: Read> {
    units: TemporalUnitReader<Take<R>>,
    limits: DecodeLimits,
    cap: u64,
    fourcc_checked: bool,
    yielded: bool,
}

impl<R: Read> ReaderUnits<R> {
    pub(crate) fn new(reader: R, limits: DecodeLimits) -> Self {
        let cap = input_cap(limits);
        Self {
            units: TemporalUnitReader::with_max_unit_bytes(reader.take(cap), usize::MAX),
            limits,
            cap,
            fourcc_checked: false,
            yielded: false,
        }
    }

    fn ensure_input(&self) -> Result<()> {
        let read = self.cap - self.units.get_ref().limit();
        self.limits
            .ensure(DecodeLimitName::MaxInputBytes, read)
            .map(drop)
            .map_err(DecodeError::from)
    }

    fn reader_error(&self, error: ReaderError) -> DecodeError {
        let ReaderError::Ivf(error) = error else {
            return self
                .ensure_input()
                .err()
                .unwrap_or_else(|| DecodeError::reader(error));
        };
        if let IvfError::TruncatedFramePayload { frame_index, .. } = error
            && let Err(source) = self
                .limits
                .ensure(DecodeLimitName::MaxIvfFrameRecords, frame_index as u64 + 1)
        {
            return source.into();
        }
        self.ensure_input()
            .err()
            .unwrap_or_else(|| DecodeError::MalformedSource {
                issue: issue_from_ivf_error(&error),
            })
    }

    fn ensure_fourcc(&mut self) -> Result<()> {
        if let Some(header) = self.units.ivf_header()
            && !core::mem::replace(&mut self.fourcc_checked, true)
            && header.fourcc != *b"AV02"
        {
            return Err(DecodeError::MalformedSource {
                issue: issue_from_unsupported_ivf_codec(header.fourcc),
            });
        }
        Ok(())
    }
}

impl<R: Read + Send> UnitSource for ReaderUnits<R> {
    fn next_unit(&mut self, buffers: &mut Vec<Arc<Vec<u8>>>) -> Result<Option<SourceUnit>> {
        loop {
            let unit = match self.units.next_temporal_unit() {
                Ok(unit) => unit,
                Err(error) => return Err(self.reader_error(error)),
            };
            let Some(unit) = unit else {
                self.ensure_fourcc()?;
                self.ensure_input()?;
                if !self.yielded && self.units.ivf_header().is_none() {
                    return Err(crate::pipeline::unsupported(
                        "empty_annex_b_input",
                        None,
                        "decode runtime requires at least one Annex B OBU",
                    ));
                }
                return Ok(None);
            };
            let mut props = crate::DataProps::default();
            let mut ivf_frame = None;
            if let Some(pts) = unit.pts {
                self.limits
                    .ensure(DecodeLimitName::MaxIvfFrameRecords, unit.index as u64 + 1)?;
                if unit.bytes.is_empty() {
                    continue;
                }
                props = props.with_pts(pts.cast_signed());
                ivf_frame = Some(DecodeIvfFrameContext::new(
                    unit.index,
                    unit.offset,
                    unit.bytes.len() as u32,
                    pts,
                ));
            }
            let unit = SourceUnit {
                bytes: UnitBytes::new(fill_buffer(buffers, unit.bytes), unit.offset.get()),
                ivf_frame,
                props,
            };
            self.ensure_fourcc()?;
            self.ensure_input()?;
            self.yielded = true;
            return Ok(Some(unit));
        }
    }

    fn ivf_header(&self) -> Option<IvfHeader> {
        self.units.ivf_header()
    }
}

/// Parses the OBUs the decode pass acts on; reserved OBUs are dropped.
pub(crate) fn runtime_obus<'a>(
    bytes: SourceBytes<'a>,
    obus: &mut Vec<ObuEnvelope<'a>>,
) -> Result<()> {
    let mut cursor = AnnexBObuCursor::new(bytes.bytes(), ByteOffset::new(bytes.base()));
    while let Some(envelope) = cursor
        .next_obu()
        .map_err(|error| DecodeError::MalformedSource {
            issue: crate::bitstream::stream_plan::issue_from_core_error(
                crate::DecodeSourceIssueKind::AnnexBParseError,
                None,
                &error,
            ),
        })?
    {
        if !envelope.header.obu_type.is_reserved() {
            obus.push(envelope);
        }
    }
    Ok(())
}

fn parse_bounded_annex_b<'a>(bytes: &'a [u8], options: &DecodeOptions) -> Result<PartialParse<'a>> {
    let mut obu_count = 0u64;
    let mut frame_candidate_count = 0u64;
    let mut first_unsupported = None;
    let mut selected_layer = None;
    let mut layer_filter = ActiveLayerFilter::from_options(options);
    let mut obus = Vec::new();
    let error = parse_bounded_annex_b_at(
        bytes,
        ByteOffset::new(0),
        options.limits(),
        &mut selected_layer,
        &mut layer_filter,
        &mut obu_count,
        &mut frame_candidate_count,
        &mut first_unsupported,
        &mut obus,
    )?;
    Ok(PartialParse { obus, error })
}

#[expect(clippy::too_many_arguments)]
fn parse_bounded_annex_b_at<'a>(
    input: &'a [u8],
    base_offset: ByteOffset,
    limits: DecodeLimits,
    selected_layer: &mut Option<DecodeLayerSelection>,
    layer_filter: &mut ActiveLayerFilter,
    obu_count: &mut u64,
    frame_candidate_count: &mut u64,
    first_unsupported: &mut Option<DecodeUnsupportedStructure>,
    obus: &mut Vec<ObuEnvelope<'a>>,
) -> Result<Option<splot_core::Error>> {
    let mut cursor = AnnexBObuCursor::new(input, base_offset);

    while cursor.has_remaining() {
        let next_obu_count = obu_count.saturating_add(1);
        ensure_or_first_unsupported(
            limits,
            DecodeLimitName::MaxObus,
            next_obu_count,
            first_unsupported.as_ref(),
        )?;

        match cursor.next_obu() {
            Ok(Some(envelope)) => {
                let current_layer = selected_layer.unwrap_or_default();
                let classification = classify_obu(envelope, current_layer, layer_filter);
                DecodeLayerSelection::adopt_with_filter(
                    selected_layer,
                    envelope.header.extended_layer_id,
                    layer_filter,
                );
                if matches!(classification, Ok(role) if role.is_frame_candidate()) {
                    let next_frame_candidate_count = frame_candidate_count.saturating_add(1);
                    ensure_or_first_unsupported(
                        limits,
                        DecodeLimitName::MaxFramesToDecode,
                        next_frame_candidate_count,
                        first_unsupported.as_ref(),
                    )?;
                    *frame_candidate_count = next_frame_candidate_count;
                }
                obus.push(envelope);
                *obu_count = next_obu_count;
                record_first_unsupported(first_unsupported, classification.map(|_| ()))?;
            }
            Ok(None) => {
                break;
            }
            Err(error) => {
                return Ok(Some(error));
            }
        }
    }

    Ok(None)
}

fn ensure_or_first_unsupported(
    limits: DecodeLimits,
    name: DecodeLimitName,
    value: u64,
    first_unsupported: Option<&DecodeUnsupportedStructure>,
) -> Result<()> {
    match limits.ensure(name, value) {
        Ok(_) => Ok(()),
        Err(source) => match first_unsupported {
            Some(unsupported) => Err(DecodeError::UnsupportedStructure {
                unsupported: unsupported.clone(),
            }),
            None => Err(DecodeError::Limit { source }),
        },
    }
}

fn record_first_unsupported(
    first_unsupported: &mut Option<DecodeUnsupportedStructure>,
    result: Result<()>,
) -> Result<()> {
    match result {
        Ok(()) => Ok(()),
        Err(DecodeError::UnsupportedStructure { unsupported }) => {
            if first_unsupported.is_none() {
                *first_unsupported = Some(unsupported);
            }
            Ok(())
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
#[path = "byte_stream_tests.rs"]
mod tests;

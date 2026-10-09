// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Bounded stream planning for raw Annex B and IVF inputs, and the record
//! reader the decode pass reads IVF input through again.
//!
//! Feature tracking: `DECODE-BYTE-STREAM-PLANNER`.

use std::hash::BuildHasher;
use std::io::{Cursor, ErrorKind, Read, Seek, SeekFrom};
use std::sync::Arc;

use splot_core::annexb::{AnnexBObuCursor, ObuEnvelope, PartialParse};
use splot_core::ivf::{IVF_FRAME_HEADER_SIZE, IvfFrame, IvfHeader, is_ivf};
use splot_core::obu::ObuHeader;
use splot_core::span::ByteOffset;
use splot_core::stream::BitstreamFormat;
use splot_core::stream_reader::{ReaderError, StreamUnit, TemporalUnitReader};
use splot_core::types::ObuType;

use crate::bitstream::stream_plan::{
    DecodeIvfFrameContext, DecodeLayerSelection, DecodeStreamPlan, DecodeUnsupportedStructure,
    IvfPlanner, PlanBuilder, ensure_supported_obu, plan_annex_b,
};
use crate::error::{DecodeError, Result};
use crate::{DecodeLimitName, DecodeLimits, DecodeOptions};

/// A seekable decode input. Planning reads it once; decode reads it again.
pub(crate) trait ReadSeek: Read + Seek + Send {}

impl<R: Read + Seek + Send + ?Sized> ReadSeek for R {}

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
    prepare(&mut Cursor::new(bytes), options, Vec::new(), true).map(|prepared| prepared.plan)
}

/// A planned input and what the decode pass needs to read it again.
pub(crate) struct PreparedStream {
    pub(crate) plan: DecodeStreamPlan,
    pub(crate) input: PreparedInput,
}

/// IVF input is read again record by record. Annex B input stays whole in
/// memory; split it at temporal delimiters if large Annex B inputs matter.
pub(crate) enum PreparedInput {
    AnnexB(UnitBytes),
    /// The header, the input end planning read, and a hash of each non-empty
    /// record it checked.
    Ivf(IvfHeader, u64, RecordHashes),
}

/// Hashes of the IVF records planning checked, under keys drawn for this
/// decode, so a record rewritten between the passes cannot be made to match.
pub(crate) struct RecordHashes {
    keys: std::hash::RandomState,
    hashes: Vec<u64>,
}

impl RecordHashes {
    /// Empties `hashes`, a list an earlier decode returned, under fresh keys.
    pub(crate) fn new(mut hashes: Vec<u64>) -> Self {
        hashes.clear();
        Self {
            keys: std::hash::RandomState::new(),
            hashes,
        }
    }

    /// Hashes a record's bytes with its input offset, so moved framing fails too.
    fn push(&mut self, offset: u64, bytes: &[u8]) {
        self.hashes.push(self.keys.hash_one((offset, bytes)));
    }

    fn matches(&self, record: usize, offset: u64, bytes: &[u8]) -> bool {
        self.hashes.get(record) == Some(&self.keys.hash_one((offset, bytes)))
    }

    pub(crate) fn into_vec(self) -> Vec<u64> {
        self.hashes
    }
}

/// Plans `reader` without keeping IVF payloads: each frame record is parsed
/// and planned, then dropped. IVF record hashes go into `hashes`, which a
/// caller keeps between decodes. The plan keeps no per-OBU entries; the decode
/// pass plans each record's OBUs again when it reads the record.
pub(crate) fn prepare_stream(
    reader: &mut dyn ReadSeek,
    options: &DecodeOptions,
    hashes: Vec<u64>,
) -> Result<PreparedStream> {
    prepare(reader, options, hashes, false)
}

fn prepare(
    reader: &mut dyn ReadSeek,
    options: &DecodeOptions,
    hashes: Vec<u64>,
    keep_obus: bool,
) -> Result<PreparedStream> {
    let limits = options.limits();
    let input_len = reader.seek(SeekFrom::End(0)).map_err(DecodeError::input)?;
    limits.ensure(DecodeLimitName::MaxInputBytes, input_len)?;
    rewind(reader)?;
    let mut magic = [0u8; 4];
    let ivf = match reader.read_exact(&mut magic) {
        Ok(()) => is_ivf(&magic),
        Err(error) if error.kind() == ErrorKind::UnexpectedEof => false,
        Err(error) => return Err(DecodeError::input(error)),
    };
    rewind(reader)?;
    if ivf {
        let (plan, header, end, hashes) = plan_ivf(
            reader,
            input_len,
            limits,
            RecordHashes::new(hashes),
            keep_obus,
        )?;
        return Ok(PreparedStream {
            plan,
            input: PreparedInput::Ivf(header, end, hashes),
        });
    }
    let mut bytes = Vec::new();
    let _ = bytes.try_reserve_exact(usize::try_from(input_len).unwrap_or(0));
    Read::take(&mut *reader, input_cap(limits))
        .read_to_end(&mut bytes)
        .map_err(DecodeError::input)?;
    let input_len = bytes.len() as u64;
    limits.ensure(DecodeLimitName::MaxInputBytes, input_len)?;
    let bytes = UnitBytes::new(Arc::new(bytes), 0);
    let plan = plan_annex_b(
        &parse_bounded_annex_b(bytes.view().bytes(), limits)?,
        input_len,
        options,
        keep_obus,
    )?;
    Ok(PreparedStream {
        plan,
        input: PreparedInput::AnnexB(bytes),
    })
}

/// One byte past the input limit: reading it proves the input is too long,
/// whatever length the seek reported.
fn input_cap(limits: DecodeLimits) -> u64 {
    limits
        .max_input_bytes()
        .max_value()
        .map_or(u64::MAX, |max| max.saturating_add(1))
}

fn rewind(reader: &mut dyn ReadSeek) -> Result<()> {
    reader
        .seek(SeekFrom::Start(0))
        .map(drop)
        .map_err(DecodeError::input)
}

fn plan_ivf(
    reader: &mut dyn ReadSeek,
    input_len: u64,
    limits: DecodeLimits,
    mut hashes: RecordHashes,
    keep_obus: bool,
) -> Result<(DecodeStreamPlan, IvfHeader, u64, RecordHashes)> {
    let cap = input_cap(limits);
    let mut capped = Read::take(&mut *reader, cap);
    let mut units = TemporalUnitReader::with_max_unit_bytes(&mut capped, usize::MAX);
    let mut builder = PlanBuilder::new(BitstreamFormat::Ivf, input_len, limits, keep_obus);
    let mut planner = IvfPlanner::default();
    let mut counts = (0u64, 0u64);
    let mut first_unsupported = None;
    let mut spare = Vec::new();
    let mut warning = None;
    let error = loop {
        let unit = match units.next_unit() {
            Ok(unit) => unit,
            Err(ReaderError::Ivf(error)) => {
                if let splot_core::ivf::IvfError::TruncatedFramePayload { frame_index, .. } = error
                {
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
        if !payload.is_empty() {
            hashes.push(payload_offset.get(), payload);
        }
        let mut obus = recycle(core::mem::take(&mut spare));
        let frame_error = parse_bounded_annex_b_at(
            payload,
            payload_offset,
            limits,
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
    drop(units);
    let end = cap - capped.limit();
    limits.ensure(DecodeLimitName::MaxInputBytes, end)?;
    builder.input_len_bytes = end;
    let plan = planner.finish(builder, header, warning.as_slice(), error.as_ref())?;
    let header = header.ok_or_else(|| {
        crate::pipeline::unsupported(
            "missing_ivf_header",
            None,
            "decode runtime requires a complete IVF header",
        )
    })?;
    Ok((plan, header, end, hashes))
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

/// Reads IVF frame records again during decode, one record at a time, into
/// reused buffers that the tasks parsing a record's frames share. Planning
/// already checked the container, so a short read here is the end of input,
/// and a record past the planned end, or one whose bytes differ from what
/// planning hashed, means the input changed between passes.
pub(crate) struct IvfRecords<'r> {
    reader: &'r mut dyn ReadSeek,
    position: u64,
    end: u64,
    hashes: &'r RecordHashes,
    current: Option<UnitBytes>,
    record: usize,
    frames_read: usize,
    pts: u64,
    buffers: &'r mut Vec<Arc<Vec<u8>>>,
}

impl<'r> IvfRecords<'r> {
    pub(crate) fn new(
        reader: &'r mut dyn ReadSeek,
        header: IvfHeader,
        end: u64,
        hashes: &'r RecordHashes,
        buffers: &'r mut Vec<Arc<Vec<u8>>>,
    ) -> Result<Self> {
        let position = u64::from(header.header_len);
        reader
            .seek(SeekFrom::Start(position))
            .map_err(DecodeError::input)?;
        Ok(Self {
            reader,
            position,
            end,
            hashes,
            current: None,
            record: 0,
            frames_read: 0,
            pts: 0,
            buffers,
        })
    }

    /// The current record and its index among non-empty records.
    pub(crate) fn current(&self) -> Option<(&UnitBytes, usize)> {
        self.current.as_ref().map(|unit| (unit, self.record))
    }

    /// The IVF frame context of the current record.
    pub(crate) fn frame(&self) -> Option<DecodeIvfFrameContext> {
        let view = self.current.as_ref()?.view();
        Some(DecodeIvfFrameContext::new(
            self.frames_read - 1,
            ByteOffset::new(view.base()),
            view.bytes().len() as u32,
            self.pts,
        ))
    }

    /// Makes the next non-empty record current; `false` at end of input.
    pub(crate) fn advance(&mut self) -> Result<bool> {
        let mut header = [0u8; IVF_FRAME_HEADER_SIZE];
        loop {
            match self.reader.read_exact(&mut header) {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::UnexpectedEof => return Ok(false),
                Err(error) => return Err(DecodeError::input(error)),
            }
            let size = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
            self.frames_read += 1;
            self.pts = u64::from_le_bytes([
                header[4], header[5], header[6], header[7], header[8], header[9], header[10],
                header[11],
            ]);
            let base = self.position + IVF_FRAME_HEADER_SIZE as u64;
            self.position = base + u64::from(size);
            if self.position > self.end {
                return Err(input_changed());
            }
            if size == 0 {
                continue;
            }
            if self.current.take().is_some() {
                self.record += 1;
            }
            let free = self
                .buffers
                .iter_mut()
                .position(|buffer| Arc::get_mut(buffer).is_some());
            let index = free.unwrap_or_else(|| {
                self.buffers.push(Arc::default());
                self.buffers.len() - 1
            });
            let buffer = &mut self.buffers[index];
            if let Some(bytes) = Arc::get_mut(buffer) {
                bytes.resize(size as usize, 0);
                self.reader.read_exact(bytes).map_err(DecodeError::input)?;
                if !self.hashes.matches(self.record, base, bytes) {
                    return Err(input_changed());
                }
            }
            self.current = Some(UnitBytes::new(Arc::clone(buffer), base));
            return Ok(true);
        }
    }

    /// [`Self::advance`], then checks the new record's OBUs with `check`.
    pub(crate) fn advance_checked(
        &mut self,
        storage: &mut Vec<ObuEnvelope<'static>>,
        check: fn(&[ObuEnvelope<'_>], usize) -> Result<()>,
    ) -> Result<bool> {
        if !self.advance()? {
            return Ok(false);
        }
        let Some(unit) = &self.current else {
            return Ok(false);
        };
        let mut obus = recycle(core::mem::take(storage));
        runtime_obus(unit.view(), &mut obus)?;
        check(&obus, self.record)?;
        *storage = recycle(obus);
        Ok(true)
    }
}

fn input_changed() -> DecodeError {
    DecodeError::input(std::io::Error::new(
        ErrorKind::InvalidData,
        "IVF input changed between planning and decode",
    ))
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

fn parse_bounded_annex_b(bytes: &[u8], limits: DecodeLimits) -> Result<PartialParse<'_>> {
    let mut obu_count = 0u64;
    let mut frame_candidate_count = 0u64;
    let mut first_unsupported = None;
    let mut obus = Vec::new();
    let error = parse_bounded_annex_b_at(
        bytes,
        ByteOffset::new(0),
        limits,
        &mut obu_count,
        &mut frame_candidate_count,
        &mut first_unsupported,
        &mut obus,
    )?;
    Ok(PartialParse { obus, error })
}

fn parse_bounded_annex_b_at<'a>(
    input: &'a [u8],
    base_offset: ByteOffset,
    limits: DecodeLimits,
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
                if is_selected_frame_candidate(envelope.header) {
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
                record_first_unsupported(
                    first_unsupported,
                    ensure_supported_obu(envelope, DecodeLayerSelection::base()),
                )?;
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

fn is_selected_frame_candidate(header: ObuHeader) -> bool {
    let selected_layer = DecodeLayerSelection::base();
    matches!(
        header.obu_type,
        ObuType::ClosedLoopKey | ObuType::RegularTileGroup | ObuType::RegularTip
    ) && header.temporal_layer_id == selected_layer.temporal_layer_id()
        && header.embedded_layer_id == selected_layer.embedded_layer_id()
        && header.extended_layer_id == selected_layer.extended_layer_id()
}

#[cfg(test)]
#[path = "byte_stream_tests.rs"]
mod tests;

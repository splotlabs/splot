// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Bounded byte-consuming stream planning for raw Annex B and IVF inputs.
//!
//! Feature tracking: `DECODE-BYTE-STREAM-PLANNER`.

use core::ops::Range;

use splot_core::annexb::{AnnexBObuCursor, ObuEnvelope, PartialParse};
use splot_core::ivf::{
    IvfError, IvfFrame, IvfFrameCursor, IvfFrameRead, IvfHeader, IvfWarning, is_ivf,
    parse_ivf_header,
};
use splot_core::obu::ObuHeader;
use splot_core::span::ByteOffset;
use splot_core::stream::BitstreamFormat;
use splot_core::types::ObuType;

use crate::bitstream::stream_plan::{
    DecodeLayerSelection, DecodeStreamPlan, DecodeUnsupportedStructure, ensure_supported_obu,
    plan_flat_stream,
};
use crate::error::{DecodeError, Result};
use crate::{DecodeLimitName, DecodeLimits, DecodeOptions};

pub(crate) fn plan_byte_stream(bytes: &[u8], options: &DecodeOptions) -> Result<DecodeStreamPlan> {
    prepare_byte_stream(bytes, options).map(PreparedByteStream::into_plan)
}

pub(crate) fn prepare_byte_stream<'a>(
    bytes: &'a [u8],
    options: &DecodeOptions,
) -> Result<PreparedByteStream<'a>> {
    let input_len_bytes = bytes.len() as u64;
    let limits = options.limits();
    limits.ensure(DecodeLimitName::MaxInputBytes, input_len_bytes)?;

    let mut parsed = parse_bounded_bitstream(bytes, limits)?;
    let plan = plan_flat_stream(&parsed, input_len_bytes, options)?;
    parsed.discard_runtime_noops();
    Ok(PreparedByteStream { plan, parsed })
}

pub(crate) struct PreparedByteStream<'a> {
    plan: DecodeStreamPlan,
    parsed: FlatParsedBitstream<'a>,
}

impl<'a> PreparedByteStream<'a> {
    pub(crate) fn plan(&self) -> &DecodeStreamPlan {
        &self.plan
    }

    pub(crate) fn parsed(&self) -> &FlatParsedBitstream<'a> {
        &self.parsed
    }

    fn into_plan(self) -> DecodeStreamPlan {
        self.plan
    }
}

#[derive(Debug)]
pub(crate) enum FlatParsedBitstream<'a> {
    AnnexB(PartialParse<'a>),
    Ivf(FlatParsedIvfBitstream<'a>),
}

impl FlatParsedBitstream<'_> {
    pub(crate) const fn format(&self) -> BitstreamFormat {
        match self {
            Self::AnnexB(_) => BitstreamFormat::AnnexB,
            Self::Ivf(_) => BitstreamFormat::Ivf,
        }
    }

    pub(crate) fn discard_runtime_noops(&mut self) {
        match self {
            Self::AnnexB(partial) => partial
                .obus
                .retain(|obu| !obu.header.obu_type.is_reserved()),
            Self::Ivf(ivf) => ivf.discard_runtime_noops(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct FlatParsedIvfBitstream<'a> {
    pub(crate) header: Option<IvfHeader>,
    pub(crate) frames: Vec<FlatParsedIvfFrame<'a>>,
    pub(crate) obus: Vec<ObuEnvelope<'a>>,
    pub(crate) warnings: Vec<IvfWarning>,
    pub(crate) error: Option<IvfError>,
}

impl<'a> FlatParsedIvfBitstream<'a> {
    pub(crate) fn frame_obus(&self, frame: &FlatParsedIvfFrame<'_>) -> &[ObuEnvelope<'a>] {
        self.obus.get(frame.obus.clone()).unwrap_or(&[])
    }

    fn discard_runtime_noops(&mut self) {
        let mut write = 0;
        for frame in &mut self.frames {
            let start = write;
            for read in frame.obus.clone() {
                let obu = self.obus[read];
                if !obu.header.obu_type.is_reserved() {
                    self.obus[write] = obu;
                    write += 1;
                }
            }
            frame.obus = start..write;
        }
        self.obus.truncate(write);
        self.frames.retain(|frame| frame.frame.size != 0);
    }
}

#[derive(Debug)]
pub(crate) struct FlatParsedIvfFrame<'a> {
    pub(crate) frame: IvfFrame<'a>,
    pub(crate) obus: Range<usize>,
    pub(crate) error: Option<splot_core::Error>,
}

pub(crate) fn parse_bounded_bitstream(
    bytes: &[u8],
    limits: DecodeLimits,
) -> Result<FlatParsedBitstream<'_>> {
    if is_ivf(bytes) {
        return Ok(FlatParsedBitstream::Ivf(parse_bounded_ivf(bytes, limits)?));
    }

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
    Ok(FlatParsedBitstream::AnnexB(PartialParse { obus, error }))
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

fn parse_bounded_ivf(input: &[u8], limits: DecodeLimits) -> Result<FlatParsedIvfBitstream<'_>> {
    let header = match parse_ivf_header(input) {
        Ok(header) => header,
        Err(error) => {
            return Ok(FlatParsedIvfBitstream {
                header: None,
                frames: Vec::new(),
                obus: Vec::new(),
                warnings: Vec::new(),
                error: Some(error),
            });
        }
    };

    let mut frames = Vec::new();
    let mut obus = Vec::new();
    let mut warnings = Vec::new();
    let mut cursor = IvfFrameCursor::new(input, header);
    let mut obu_count = 0u64;
    let mut frame_candidate_count = 0u64;
    let mut first_unsupported = None;

    while cursor.has_remaining() {
        if cursor.has_complete_frame_header() {
            limits.ensure(
                DecodeLimitName::MaxIvfFrameRecords,
                cursor.next_frame_index() as u64 + 1,
            )?;
        }

        match cursor.next_frame_record() {
            Ok(IvfFrameRead::Frame(frame)) => {
                let obu_start = obus.len();
                let error = parse_bounded_annex_b_at(
                    frame.payload,
                    frame.payload_offset,
                    limits,
                    &mut obu_count,
                    &mut frame_candidate_count,
                    &mut first_unsupported,
                    &mut obus,
                )?;
                let has_error = error.is_some();
                frames.push(FlatParsedIvfFrame {
                    frame,
                    obus: obu_start..obus.len(),
                    error,
                });
                if has_error {
                    break;
                }
            }
            Ok(IvfFrameRead::Warning(warning)) => {
                warnings.push(warning);
                break;
            }
            Ok(IvfFrameRead::End) => break,
            Err(error) => {
                return Ok(FlatParsedIvfBitstream {
                    header: Some(header),
                    frames,
                    obus,
                    warnings,
                    error: Some(error),
                });
            }
        }
    }

    Ok(FlatParsedIvfBitstream {
        header: Some(header),
        frames,
        obus,
        warnings,
        error: None,
    })
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

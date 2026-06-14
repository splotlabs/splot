// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Bounded byte-consuming stream planning for raw Annex B and IVF inputs.
//!
//! Feature tracking: `DECODE-BYTE-STREAM-PLANNER`.

use splot_core::annexb::{ObuEnvelope, PartialParse};
use splot_core::ivf::{
    IVF_FRAME_HEADER_SIZE, IvfError, IvfFrame, IvfWarning, is_ivf, parse_ivf_header,
};
use splot_core::leb128::read_leb128;
use splot_core::obu::{ObuHeader, read_obu_header_from_slice};
use splot_core::span::ByteOffset;
use splot_core::stream::{ParsedBitstream, ParsedIvfBitstream, ParsedIvfFrame};
use splot_core::types::ObuType;

use crate::error::Result;
use crate::stream_plan::{DecodeLayerSelection, DecodeStreamInput, DecodeStreamPlan, plan_stream};
use crate::{DecodeLimitName, DecodeLimits, DecodeOptions};

/// Builds a deterministic decode stream plan from raw bytes.
///
/// This entry point enforces byte-traversal limits while walking the source and
/// then reuses the existing parsed-stream planner for layer selection and
/// unsupported-structure classification.
pub(crate) fn plan_byte_stream(bytes: &[u8], options: DecodeOptions) -> Result<DecodeStreamPlan> {
    let input_len_bytes = bytes.len() as u64;
    let limits = options.limits();
    limits.ensure(DecodeLimitName::MaxInputBytes, input_len_bytes)?;

    let parsed = parse_bounded_bitstream(bytes, limits)?;
    plan_stream(DecodeStreamInput::new(&parsed, input_len_bytes), options)
}

fn parse_bounded_bitstream<'a>(
    bytes: &'a [u8],
    limits: DecodeLimits,
) -> Result<ParsedBitstream<'a>> {
    if is_ivf(bytes) {
        return Ok(ParsedBitstream::Ivf(parse_bounded_ivf(bytes, limits)?));
    }

    let mut obu_count = 0u64;
    let mut frame_candidate_count = 0u64;
    Ok(ParsedBitstream::AnnexB(parse_bounded_annex_b_at(
        bytes,
        ByteOffset::new(0),
        limits,
        &mut obu_count,
        &mut frame_candidate_count,
    )?))
}

fn parse_bounded_annex_b_at<'a>(
    input: &'a [u8],
    base_offset: ByteOffset,
    limits: DecodeLimits,
    obu_count: &mut u64,
    frame_candidate_count: &mut u64,
) -> Result<PartialParse<'a>> {
    let mut obus = Vec::new();
    let mut cursor = 0usize;

    while cursor < input.len() {
        let next_obu_count = obu_count.saturating_add(1);
        limits.ensure(DecodeLimitName::MaxObus, next_obu_count)?;

        match parse_one_obu_at(input, cursor, base_offset) {
            Ok((envelope, next)) => {
                if is_selected_frame_candidate(envelope.header) {
                    let next_frame_candidate_count = frame_candidate_count.saturating_add(1);
                    limits.ensure(
                        DecodeLimitName::MaxFramesToDecode,
                        next_frame_candidate_count,
                    )?;
                    *frame_candidate_count = next_frame_candidate_count;
                }
                obus.push(envelope);
                *obu_count = next_obu_count;
                cursor = next;
            }
            Err(error) => {
                return Ok(PartialParse {
                    obus,
                    error: Some(error),
                });
            }
        }
    }

    Ok(PartialParse { obus, error: None })
}

fn parse_one_obu_at(
    input: &[u8],
    cursor: usize,
    base_offset: ByteOffset,
) -> splot_core::Result<(ObuEnvelope<'_>, usize)> {
    let prefix_offset = base_offset.saturating_add(cursor as u64);
    let prefix = read_leb128(input, ByteOffset::new(cursor as u64))
        .map_err(|error| rebase_leb128_error(error, base_offset))?;
    let header_start = cursor.saturating_add(usize::from(prefix.bytes_read));
    let size = prefix.value;

    if size == 0 {
        return Err(splot_core::Error::ObuSizeOutOfRange {
            offset: prefix_offset,
            size: 0,
        });
    }

    let size_usize = size as usize;
    let remaining = input.len().saturating_sub(header_start);
    let header_offset = base_offset.saturating_add(header_start as u64);
    if size_usize > remaining {
        return Err(splot_core::Error::ObuPayloadOutOfRange {
            offset: header_offset,
            size,
            remaining,
        });
    }

    let obu_end = header_start.saturating_add(size_usize);
    let Some(obu_bytes) = input.get(header_start..obu_end) else {
        return Err(splot_core::Error::ObuPayloadOutOfRange {
            offset: header_offset,
            size,
            remaining,
        });
    };

    let header = read_obu_header_from_slice(obu_bytes, header_offset)?;
    let payload = obu_bytes
        .get(usize::from(header.header_size_bytes)..)
        .unwrap_or(&[]);

    Ok((
        ObuEnvelope {
            offset: header_offset,
            size,
            header,
            payload,
        },
        obu_end,
    ))
}

fn parse_bounded_ivf<'a>(input: &'a [u8], limits: DecodeLimits) -> Result<ParsedIvfBitstream<'a>> {
    let header = match parse_ivf_header(input) {
        Ok(header) => header,
        Err(error) => {
            return Ok(ParsedIvfBitstream {
                header: None,
                frames: Vec::new(),
                warnings: Vec::new(),
                error: Some(error),
            });
        }
    };

    let mut frames = Vec::new();
    let mut warnings = Vec::new();
    let mut cursor = usize::from(header.header_len);
    let mut frame_index = 0usize;
    let mut obu_count = 0u64;
    let mut frame_candidate_count = 0u64;

    while cursor < input.len() {
        let remaining_header = input.len().saturating_sub(cursor);
        if remaining_header < IVF_FRAME_HEADER_SIZE {
            if frame_index > 0 {
                warnings.push(IvfWarning::TrailingPartialFrameHeader {
                    frame_index,
                    offset: ByteOffset::new(input.len() as u64),
                    needed: IVF_FRAME_HEADER_SIZE.saturating_sub(remaining_header),
                });
                break;
            }
            return Ok(ParsedIvfBitstream {
                header: Some(header),
                frames,
                warnings,
                error: Some(IvfError::TruncatedFrameHeader {
                    frame_index,
                    offset: ByteOffset::new(input.len() as u64),
                    needed: IVF_FRAME_HEADER_SIZE.saturating_sub(remaining_header),
                }),
            });
        }

        limits.ensure(DecodeLimitName::MaxIvfFrameRecords, frame_index as u64 + 1)?;

        let Some(size) = read_u32_le(input, cursor) else {
            return Ok(ParsedIvfBitstream {
                header: Some(header),
                frames,
                warnings,
                error: Some(IvfError::TruncatedFrameHeader {
                    frame_index,
                    offset: ByteOffset::new(input.len() as u64),
                    needed: IVF_FRAME_HEADER_SIZE.saturating_sub(remaining_header),
                }),
            });
        };
        let Some(pts) = read_u64_le(input, cursor.saturating_add(4)) else {
            return Ok(ParsedIvfBitstream {
                header: Some(header),
                frames,
                warnings,
                error: Some(IvfError::TruncatedFrameHeader {
                    frame_index,
                    offset: ByteOffset::new(input.len() as u64),
                    needed: IVF_FRAME_HEADER_SIZE.saturating_sub(remaining_header),
                }),
            });
        };

        let payload_start = cursor.saturating_add(IVF_FRAME_HEADER_SIZE);
        let remaining_payload = input.len().saturating_sub(payload_start);
        let size_usize = size as usize;
        if size_usize > remaining_payload {
            return Ok(ParsedIvfBitstream {
                header: Some(header),
                frames,
                warnings,
                error: Some(IvfError::TruncatedFramePayload {
                    frame_index,
                    offset: ByteOffset::new(input.len() as u64),
                    size,
                    remaining: remaining_payload,
                }),
            });
        }

        let payload_end = payload_start.saturating_add(size_usize);
        let Some(payload) = input.get(payload_start..payload_end) else {
            return Ok(ParsedIvfBitstream {
                header: Some(header),
                frames,
                warnings,
                error: Some(IvfError::TruncatedFramePayload {
                    frame_index,
                    offset: ByteOffset::new(input.len() as u64),
                    size,
                    remaining: remaining_payload,
                }),
            });
        };

        let frame = IvfFrame {
            index: frame_index,
            header_offset: ByteOffset::new(cursor as u64),
            payload_offset: ByteOffset::new(payload_start as u64),
            size,
            pts,
            payload,
        };
        let partial = parse_bounded_annex_b_at(
            payload,
            ByteOffset::new(payload_start as u64),
            limits,
            &mut obu_count,
            &mut frame_candidate_count,
        )?;
        let has_error = partial.error.is_some();
        frames.push(ParsedIvfFrame {
            frame,
            obus: partial.obus,
            error: partial.error,
        });
        if has_error {
            break;
        }

        cursor = payload_end;
        frame_index = frame_index.saturating_add(1);
    }

    Ok(ParsedIvfBitstream {
        header: Some(header),
        frames,
        warnings,
        error: None,
    })
}

fn is_selected_frame_candidate(header: ObuHeader) -> bool {
    let selected_layer = DecodeLayerSelection::base();
    header.obu_type == ObuType::ClosedLoopKey
        && header.temporal_layer_id == selected_layer.temporal_layer_id()
        && header.embedded_layer_id == selected_layer.embedded_layer_id()
        && header.extended_layer_id == selected_layer.extended_layer_id()
}

fn rebase_leb128_error(error: splot_core::Error, base_offset: ByteOffset) -> splot_core::Error {
    match error {
        splot_core::Error::UnexpectedEof { offset, needed } => splot_core::Error::UnexpectedEof {
            offset: base_offset.saturating_add(offset.get()),
            needed,
        },
        splot_core::Error::InvalidLeb128 { offset, message } => splot_core::Error::InvalidLeb128 {
            offset: base_offset.saturating_add(offset.get()),
            message,
        },
        other => other,
    }
}

fn read_u32_le(input: &[u8], offset: usize) -> Option<u32> {
    let bytes = input.get(offset..offset.checked_add(4)?)?;
    Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn read_u64_le(input: &[u8], offset: usize) -> Option<u64> {
    let bytes = input.get(offset..offset.checked_add(8)?)?;
    Some(u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn raw_obu_limit_is_checked_before_parsing_next_obu() {
        let bytes = [0x01, 0x08, 0x05, 0x10];
        let options = DecodeOptions::new(
            DecodeLimits::unlimited().with_max_obus(crate::DecodeLimitThreshold::Max(1)),
        );

        let error = plan_byte_stream(&bytes, options).unwrap_err();

        assert!(matches!(
            error,
            crate::DecodeError::Limit {
                source
            } if source.name() == DecodeLimitName::MaxObus
        ));
    }

    #[test]
    fn raw_frame_candidate_limit_is_checked_before_later_malformed_bytes() {
        let bytes = [0x01, 0x10, 0x01, 0x10, 0x05, 0x10];
        let options = DecodeOptions::new(
            DecodeLimits::unlimited()
                .with_max_frames_to_decode(crate::DecodeLimitThreshold::Max(1)),
        );

        let error = plan_byte_stream(&bytes, options).unwrap_err();

        assert!(matches!(
            error,
            crate::DecodeError::Limit {
                source
            } if source.name() == DecodeLimitName::MaxFramesToDecode
        ));
    }
}

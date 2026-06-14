// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Bounded byte-consuming stream planning for raw Annex B and IVF inputs.
//!
//! Feature tracking: `DECODE-BYTE-STREAM-PLANNER`.

use splot_core::annexb::{AnnexBObuCursor, PartialParse};
use splot_core::ivf::{IvfFrameCursor, IvfFrameRead, is_ivf, parse_ivf_header};
use splot_core::obu::ObuHeader;
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
    let mut cursor = AnnexBObuCursor::new(input, base_offset);

    while cursor.has_remaining() {
        let next_obu_count = obu_count.saturating_add(1);
        limits.ensure(DecodeLimitName::MaxObus, next_obu_count)?;

        match cursor.next_obu() {
            Ok(Some(envelope)) => {
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
            }
            Ok(None) => {
                break;
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
    let mut cursor = IvfFrameCursor::new(input, header);
    let mut obu_count = 0u64;
    let mut frame_candidate_count = 0u64;

    while cursor.has_remaining() {
        if cursor.has_complete_frame_header() {
            limits.ensure(
                DecodeLimitName::MaxIvfFrameRecords,
                cursor.next_frame_index() as u64 + 1,
            )?;
        }

        match cursor.next_frame_record() {
            Ok(IvfFrameRead::Frame(frame)) => {
                let partial = parse_bounded_annex_b_at(
                    frame.payload,
                    frame.payload_offset,
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
            }
            Ok(IvfFrameRead::Warning(warning)) => {
                warnings.push(warning);
                break;
            }
            Ok(IvfFrameRead::End) => break,
            Err(error) => {
                return Ok(ParsedIvfBitstream {
                    header: Some(header),
                    frames,
                    warnings,
                    error: Some(error),
                });
            }
        }
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

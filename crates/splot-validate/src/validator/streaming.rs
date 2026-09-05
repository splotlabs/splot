// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Streaming, `Read`-based validation (`INFRA-VALIDATE-STREAMING-READER`).
//!
//! [`validate_reader_with_options`] drives a [`TemporalUnitReader`] one temporal
//! unit at a time, feeding OBUs through the same per-OBU engine as the in-memory
//! path ([`super::runner::process_obu`]) and emitting deferred diagnostics in the
//! exact order [`super::runner::validate_bytes_with_options`] uses. Peak input
//! memory is bounded to one unit instead of the whole stream.

use std::fmt;
use std::io::Read;

use splot_core::annexb::parse_annex_b_obus_partial_at;
use splot_core::span::ByteOffset;
use splot_core::stream_reader::{ReaderError, StreamUnit, TemporalUnitReader};

use crate::context::ValidatorContext;
use crate::diagnostic::ValidationReport;
use crate::options::ValidationOptions;

use super::diagnostics::{ivf_error_diagnostic, ivf_warning_diagnostic, parse_error_diagnostic};
use super::runner::process_obu;

/// A streaming-validation failure that is **not** a bitstream conformance finding.
///
/// Truncated and malformed bitstreams remain `ValidationReport` diagnostics (never
/// an error); only a genuine reader I/O failure or an over-cap unit aborts.
#[derive(Debug)]
pub enum StreamValidateError {
    /// A non-EOF I/O error from the underlying reader.
    Io(std::io::Error),
    /// A declared temporal unit exceeded the reader's per-unit byte cap.
    UnitTooLarge {
        /// Absolute offset of the offending unit.
        offset: ByteOffset,
        /// Declared unit size in bytes.
        declared: u64,
        /// The configured cap.
        cap: usize,
    },
}

impl fmt::Display for StreamValidateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "input read error: {error}"),
            Self::UnitTooLarge {
                offset,
                declared,
                cap,
            } => write!(
                f,
                "temporal unit at byte {offset} declares {declared} byte(s), exceeding the {cap}-byte cap"
            ),
        }
    }
}

impl std::error::Error for StreamValidateError {}

/// Validates a forward-only `Read` stream, bounding peak input memory to one
/// temporal unit. Produces a [`ValidationReport`] byte-identical to
/// [`super::runner::validate_bytes_with_options`] for the same bitstream.
///
/// # Errors
/// Returns [`StreamValidateError`] for a genuine reader I/O failure or an over-cap
/// unit. Truncated/malformed bitstreams are reported as diagnostics, not errors.
pub(super) fn validate_reader_with_options<R: Read>(
    reader: R,
    options: &ValidationOptions,
) -> Result<ValidationReport, StreamValidateError> {
    run_stream(TemporalUnitReader::new(reader), options)
}

/// Drives a constructed [`TemporalUnitReader`] (the cap is the reader's). Split
/// out so tests can supply a custom cap.
fn run_stream<R: Read>(
    mut reader: TemporalUnitReader<R>,
    options: &ValidationOptions,
) -> Result<ValidationReport, StreamValidateError> {
    let mut report = ValidationReport::new();
    let mut context = ValidatorContext::default();

    let mut annexb_terminal_error = None;
    let mut ivf_warnings = Vec::new();
    let mut ivf_container_error = None;

    loop {
        match reader.next_unit() {
            Ok(None) => break,
            Ok(Some(StreamUnit::AnnexBObu { offset, bytes })) => {
                let parsed = parse_annex_b_obus_partial_at(bytes, offset);
                for obu in &parsed.obus {
                    process_obu(&mut context, obu, options, &mut report);
                }
                if let Some(error) = parsed.error {
                    annexb_terminal_error = Some(error);
                    break;
                }
            }
            Ok(Some(StreamUnit::IvfFrame {
                payload_offset,
                payload,
            })) => {
                let parsed = parse_annex_b_obus_partial_at(payload, payload_offset);
                for obu in &parsed.obus {
                    process_obu(&mut context, obu, options, &mut report);
                }
                if let Some(error) = &parsed.error {
                    report.push(parse_error_diagnostic(error));
                }
            }
            Ok(Some(StreamUnit::IvfWarning(warning))) => ivf_warnings.push(warning),
            Err(ReaderError::Ivf(error)) => {
                ivf_container_error = Some(error);
                break;
            }
            Err(ReaderError::Io(error)) => return Err(StreamValidateError::Io(error)),
            Err(ReaderError::UnitTooLarge {
                offset,
                declared,
                cap,
            }) => {
                return Err(StreamValidateError::UnitTooLarge {
                    offset,
                    declared,
                    cap,
                });
            }
        }
    }

    context.finish(options, &mut report);

    if let Some(error) = annexb_terminal_error {
        report.push(parse_error_diagnostic(&error));
    }
    for warning in &ivf_warnings {
        report.push(ivf_warning_diagnostic(warning));
    }
    if let Some(error) = ivf_container_error {
        report.push(ivf_error_diagnostic(&error));
    }

    Ok(report)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::io::Cursor;

    use proptest::prelude::*;
    use splot_core::ivf::{IvfHeader, write_ivf_frame, write_ivf_header};

    use crate::validator::Validator;

    fn signature(report: &ValidationReport) -> Vec<String> {
        report
            .diagnostics
            .iter()
            .map(std::string::ToString::to_string)
            .collect()
    }

    /// Streams `data` with an unbounded cap so the comparison is pure equivalence
    /// (never aborted by the size cap).
    fn streamed_signature(data: &[u8]) -> Vec<String> {
        let reader =
            TemporalUnitReader::with_max_unit_bytes(Cursor::new(data.to_vec()), usize::MAX);
        let report =
            run_stream(reader, &ValidationOptions::default()).expect("cursor never errors");
        signature(&report)
    }

    fn assert_equivalent(data: &[u8]) {
        let in_memory = Validator::new(false).validate_bytes(data);
        assert_eq!(
            streamed_signature(data),
            signature(&in_memory),
            "stream/in-memory mismatch for input {data:02X?}"
        );
    }

    fn ivf(frames: &[&[u8]]) -> Vec<u8> {
        let mut bytes = Vec::new();
        write_ivf_header(
            &mut bytes,
            &IvfHeader::new(*b"AV02", 16, 16, 24, 1, frames.len() as u32),
        )
        .unwrap();
        for (i, payload) in frames.iter().enumerate() {
            write_ivf_frame(&mut bytes, i as u64, payload).unwrap();
        }
        bytes
    }

    #[test]
    fn equivalent_on_curated_cases() {
        let mut cases: Vec<Vec<u8>> = vec![
            vec![],                             // empty
            vec![0x01, 0x08],                   // one temporal delimiter
            vec![0x01, 0x08, 0x02, 0x04, 0xAB], // TD + sequence header w/ payload
            vec![0x02, 0x88, 0x05],             // TD with non-global xlayer -> error diag
            vec![0x80],                         // truncated leb128
            vec![0x05, 0x08],                   // truncated OBU payload
            vec![0x00],                         // zero-size OBU
            vec![0x02, 0x88, 0x05, 0x05, 0x08], // good OBU then truncated OBU
            vec![0x44, 0x4B, 0x49],             // "DKI" -> Annex B (not IVF)
            b"DKIF".to_vec(),                   // IVF signature only -> truncated header
        ];
        cases.push(ivf(&[&[0x01, 0x08]]));
        cases.push(ivf(&[&[0x01, 0x08], &[0x01, 0x08]]));
        cases.push(ivf(&[&[0x01, 0x08, 0x02, 0x04, 0xAB]]));
        cases.push(ivf(&[&[0x02, 0x88, 0x05]])); // frame OBU producing an error diag
        cases.push(ivf(&[&[0x05, 0x08]])); // Annex B error inside a frame payload

        let mut truncated_header = ivf(&[]);
        truncated_header.extend_from_slice(&[0x05, 0x00]);
        cases.push(truncated_header);

        let mut trailing = ivf(&[&[0x01, 0x08]]);
        trailing.extend_from_slice(&[0x05, 0x00]);
        cases.push(trailing);

        let mut truncated_payload = ivf(&[&[0x01, 0x08]]);
        truncated_payload.extend_from_slice(&5u32.to_le_bytes());
        truncated_payload.extend_from_slice(&0u64.to_le_bytes());
        truncated_payload.extend_from_slice(&[0x01, 0x08]);
        cases.push(truncated_payload);

        for case in &cases {
            assert_equivalent(case);
        }
    }

    #[test]
    fn validate_reader_matches_validate_bytes_via_default_cap() {
        let data = ivf(&[&[0x01, 0x08]]);
        let validator = Validator::new(false);
        let streamed = validator
            .validate_reader(Cursor::new(data.clone()))
            .unwrap();
        assert_eq!(
            signature(&streamed),
            signature(&validator.validate_bytes(&data))
        );
    }

    #[test]
    fn over_cap_unit_is_a_reader_error_not_a_diagnostic() {
        let mut data = vec![0xE8, 0x07];
        data.extend_from_slice(&[0u8; 8]);
        let reader = TemporalUnitReader::with_max_unit_bytes(Cursor::new(data), 16);
        let err = run_stream(reader, &ValidationOptions::default()).unwrap_err();
        assert!(matches!(
            err,
            StreamValidateError::UnitTooLarge {
                declared: 1000,
                cap: 16,
                ..
            }
        ));
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(128))]

        /// Streaming must produce byte-identical reports to the in-memory path for
        /// arbitrary raw input.
        #[test]
        fn streamed_matches_in_memory(data in proptest::collection::vec(any::<u8>(), 0..512)) {
            let in_memory = Validator::new(false).validate_bytes(&data);
            prop_assert_eq!(streamed_signature(&data), signature(&in_memory));
        }

        /// Same guarantee for arbitrary DKIF-prefixed (IVF) input.
        #[test]
        fn streamed_matches_in_memory_ivf(tail in proptest::collection::vec(any::<u8>(), 0..512)) {
            let mut data = b"DKIF".to_vec();
            data.extend_from_slice(&tail);
            let in_memory = Validator::new(false).validate_bytes(&data);
            prop_assert_eq!(streamed_signature(&data), signature(&in_memory));
        }
    }
}

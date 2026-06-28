// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Generic OBU-header checks that do not depend on a specific payload syntax:
//! the empty-payload trailing-bits guard and the reserved-OBU-type rules (AV2
//! § 5.2.3 / § 5.3 / § 6.2.2 / Table 6.1).

use splot_core::annexb::ObuEnvelope;
use splot_core::bitio::BitReader;
use splot_core::obu::parse_trailing_bits;
use splot_core::types::ObuType;

use super::{Check, emit, syntax_error_diagnostic};
use crate::diagnostic::{Severity, ValidationReport};

/// OBUs with empty payload syntax still carry `trailing_bits` when their declared
/// payload is non-empty. Until full payload dispatch exists, only these OBU types
/// can be checked without guessing where payload syntax ends.
pub(super) struct TrailingBitsForEmptySyntaxObus;

impl Check for TrailingBitsForEmptySyntaxObus {
    fn id(&self) -> &'static str {
        // Registry identifier only; emitted diagnostics use syntax_error_diagnostic() rule ids.
        "trailing-bits/empty-syntax-obu-payload"
    }

    fn spec_section(&self) -> Option<&'static str> {
        Some("5.2.3")
    }

    fn run(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        if obu.payload.is_empty() || !has_empty_payload_syntax(obu.header.obu_type) {
            return;
        }

        let payload_offset = obu
            .offset
            .saturating_add(u64::from(obu.header.header_size_bytes));
        let mut reader = BitReader::new(obu.payload, payload_offset);
        let nb_bits = (obu.payload.len() as u64).saturating_mul(8);
        if let Err(error) = parse_trailing_bits(&mut reader, nb_bits)
            && let Some(diagnostic) = syntax_error_diagnostic(&error)
        {
            report.push(diagnostic);
        }
    }
}

fn has_empty_payload_syntax(obu_type: ObuType) -> bool {
    matches!(obu_type, ObuType::TemporalDelimiter)
}

/// Informational: reserved OBU types are ignored by conformant decoders (AV2 Table 6.1).
pub(super) struct ReservedObuType;

impl Check for ReservedObuType {
    fn id(&self) -> &'static str {
        "obu-header/reserved-obu-type"
    }

    fn spec_section(&self) -> Option<&'static str> {
        Some("6.2.2")
    }

    fn run(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        if obu.header.obu_type.is_reserved() {
            emit(
                report,
                self,
                Severity::Info,
                obu,
                format!(
                    "reserved obu_type {} is ignored by conformant decoders",
                    obu.header.obu_type.raw()
                ),
            );
        }
    }
}

/// A reserved OBU that carries payload must have at least one non-zero payload byte
/// (AV2 § 5.3 / § 6.2.3: `trailing_one_bit` shall be 1).
pub(super) struct ReservedObuAllZeroPayload;

impl Check for ReservedObuAllZeroPayload {
    fn id(&self) -> &'static str {
        "obu-reserved/all-zero-payload"
    }

    fn spec_section(&self) -> Option<&'static str> {
        Some("5.3")
    }

    fn run(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        if obu.header.obu_type.is_reserved()
            && !obu.payload.is_empty()
            && obu.payload.iter().all(|&byte| byte == 0)
        {
            emit(
                report,
                self,
                Severity::Error,
                obu,
                "reserved OBU payload is entirely zero; AV2 § 5.3 requires at least one non-zero \
                 payload byte (including the trailing bit)"
                    .to_owned(),
            );
        }
    }
}

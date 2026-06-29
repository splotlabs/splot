// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `OBU_PADDING` syntax check (AV2 § 5.16 / § 6.15).

use splot_core::annexb::ObuEnvelope;
use splot_core::headers::padding::parse_padding_obu;
use splot_core::types::ObuType;

use super::{Check, payload_parse_error_diagnostic, syntax_error_diagnostic};
use crate::diagnostic::ValidationReport;

/// `OBU_PADDING` syntax: full `padding_obu()` parse (AV2 § 5.16 / § 6.15). The parser
/// consumes the entire payload (padding bytes plus its own `trailing_bits()`), so there
/// is no separate payload-tail step.
pub(super) struct PaddingSyntax;

impl Check for PaddingSyntax {
    fn id(&self) -> &'static str {
        "padding/syntax"
    }

    fn spec_section(&self) -> Option<&'static str> {
        Some("5.16")
    }

    fn run(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        if obu.header.obu_type != ObuType::Padding {
            return;
        }
        if let Err(error) = parse_padding_obu(obu.payload, obu.payload_offset()) {
            let diagnostic = syntax_error_diagnostic(&error)
                .unwrap_or_else(|| payload_parse_error_diagnostic(&error, "5.16"));
            report.push(diagnostic);
        }
    }
}

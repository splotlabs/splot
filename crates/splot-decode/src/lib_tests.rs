// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

#[test]
fn unsupported_feature_diagnostic_has_stable_fields() {
    let diagnostic = unsupported_feature_diagnostic();

    assert_eq!(diagnostic.rule_id, UNSUPPORTED_FEATURE_RULE_ID);
    assert_eq!(diagnostic.severity, DecodeSeverity::Error);
    assert_eq!(diagnostic.severity.as_str(), "Error");
    assert_eq!(diagnostic.spec_section, Some("7.1"));
    assert_eq!(
        diagnostic.message,
        "Byte stream planning succeeded, but `splot decode` runtime output is not implemented yet."
    );
}

#[test]
fn unsupported_feature_diagnostic_function_returns_public_descriptor() {
    assert_eq!(
        unsupported_feature_diagnostic(),
        UNSUPPORTED_FEATURE_DIAGNOSTIC
    );
}

#[test]
fn decode_severity_displays_stable_spelling() {
    assert_eq!(DecodeSeverity::Error.to_string(), "Error");
}

// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

#[test]
fn decode_severity_displays_stable_spelling() {
    assert_eq!(DecodeSeverity::Error.to_string(), "Error");
}

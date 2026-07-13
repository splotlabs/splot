// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use crate::{
    DecodeDiagnosticReport, DecodeError, DecodeUnsupportedFeature, UNSUPPORTED_FEATURE_RULE_ID,
};

#[test]
fn missing_capability_message_formats_id_and_context() {
    assert_eq!(
        super::missing_capability_message!("intra.10bit.non_dc"),
        "unsupported capability: intra.10bit.non_dc"
    );
    assert_eq!(
        super::missing_capability_message!(
            "intra.directional.d45",
            plane = "luma",
            neighbour = "above_right",
            block = "64x64",
        ),
        "unsupported capability: intra.directional.d45 plane=luma neighbour=above_right block=64x64"
    );
}

#[test]
fn compressed_runtime_message_keeps_unsupported_feature_rule() {
    let message = super::missing_capability_message!(
        "intra.directional.neighbour_reorder",
        neighbour = "directional",
    );
    let error = DecodeError::UnsupportedFeature {
        unsupported: Box::new(DecodeUnsupportedFeature::new(
            "general_intra_directional_neighbour_reorder",
            crate::pipeline::GENERAL_INTRA_MODE_SPEC_SECTION,
            message,
            None,
        )),
    };

    assert_eq!(
        DecodeDiagnosticReport::from_decode_error(&error)
            .map(|report| (report.diagnostic.rule_id, report.diagnostic.message)),
        Some((UNSUPPORTED_FEATURE_RULE_ID, message))
    );
}

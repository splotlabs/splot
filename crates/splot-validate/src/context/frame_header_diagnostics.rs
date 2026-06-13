// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Shared frame-header diagnostic constructors.

use super::*;

/// Builds a frame-header reference diagnostic located at `obu`.
pub(super) fn frame_header_error(
    rule_id: &'static str,
    spec_section: &'static str,
    obu: &ObuEnvelope<'_>,
    message: String,
) -> Diagnostic {
    Diagnostic::error(rule_id, message)
        .with_spec_section(spec_section)
        .with_byte_offset(obu.offset)
}

/// Builds the `hls/unavailable-multi-frame-header` diagnostic (AV2 § 7.3.8.7). Only
/// emitted under the default (external-disabled) options; external multi-frame
/// headers are not modeled, so under `ExternalHlsMode::Provided` the reference is left
/// unresolved without a hard error (see `resolve_frame_header_reference`).
pub(super) fn frame_header_unavailable_mfh(cur_mfh_id: MfhId, obu: &ObuEnvelope<'_>) -> Diagnostic {
    Diagnostic::error(
        "hls/unavailable-multi-frame-header",
        format!(
            "frame header references cur_mfh_id {}, but no multi-frame header with that id is \
             available in-band (external HLS is disabled)",
            cur_mfh_id.get()
        ),
    )
    .with_spec_section("7.3.8.7")
    .with_byte_offset(obu.offset)
}

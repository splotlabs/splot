// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Metadata OBU syntax (`OBU_METADATA_SHORT` / `OBU_METADATA_GROUP`) and the
//! locally decidable § 6.16 per-unit conformance diagnostics (AV2 § 5.17 / § 6.16).
//! Stateful HDR, scan-type, and timecode consistency checks live in the validator
//! context. Metadata persistence, cancellation, and applicability are decoder state.

use splot_core::annexb::ObuEnvelope;
use splot_core::bitio::BitReader;
use splot_core::headers::metadata::{
    MetadataGroupUnit, MetadataPayload, MetadataType, MetadataUnit, parse_metadata_group,
    parse_metadata_short,
};
use splot_core::types::ObuType;

use super::{
    Check, finish_payload_or_emit, payload_parse_error_diagnostic, syntax_error_diagnostic,
};
use crate::diagnostic::{Diagnostic, ValidationReport};

/// Metadata OBU syntax: full `metadata_short_obu()` / `metadata_group_obu()` parse and
/// the locally-decidable § 6.16 conformance diagnostics (AV2 § 5.17 / § 6.16). The
/// stateful HDR, scan-type, and timecode consistency checks live in the validator
/// context. § 6.16.3 persistence, cancellation, and layer applicability are owned by
/// the decoder's output-effects state. Decoded-frame-hash verification (§ 6.16.13)
/// remains decoder-output dependent.
pub(super) struct MetadataSyntax;

impl Check for MetadataSyntax {
    fn id(&self) -> &'static str {
        "metadata/syntax"
    }

    fn spec_section(&self) -> Option<&'static str> {
        Some("5.17")
    }

    fn run(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        match obu.header.obu_type {
            ObuType::MetadataShort => check_metadata_short(obu, report),
            ObuType::MetadataGroup => check_metadata_group(obu, report),
            _ => {}
        }
    }
}

/// Validates `metadata_short_obu()` (AV2 § 5.17.2 / § 6.16.2) and emits the local
/// diagnostics, then the payload tail.
fn check_metadata_short(obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
    let mut reader = BitReader::new(obu.payload, obu.payload_offset());
    match parse_metadata_short(&mut reader, obu.payload.len()) {
        Ok(metadata) => {
            if metadata.muh_layer_idc >= 3 {
                report.push(
                    Diagnostic::error(
                        "metadata/short-layer-idc-out-of-range",
                        format!(
                            "muh_layer_idc {} must be less than 3 for OBU_METADATA_SHORT",
                            metadata.muh_layer_idc
                        ),
                    )
                    .with_spec_section("6.16.2")
                    .with_byte_offset(obu.offset),
                );
            }
            if !metadata.muh_cancel_flag && metadata.muh_persistence_idc >= 4 {
                report.push(
                    Diagnostic::warning(
                        "metadata/persistence-idc-reserved",
                        format!(
                            "muh_persistence_idc {} is reserved for AOMedia use; not defined by \
                             this version of the specification",
                            metadata.muh_persistence_idc
                        ),
                    )
                    .with_spec_section("6.16.3")
                    .with_byte_offset(obu.offset),
                );
            }
            if let Some(unit) = &metadata.unit {
                check_metadata_unit_payload(unit, obu, report);
            }
            finish_payload_or_emit(&mut reader, obu.payload, false, report);
        }
        Err(error) => report.push(
            syntax_error_diagnostic(&error)
                .unwrap_or_else(|| payload_parse_error_diagnostic(&error, "5.17.2")),
        ),
    }
}

/// Validates `metadata_group_obu()` (AV2 § 5.17.3 / § 6.16.3) and emits the local
/// per-unit diagnostics, then the payload tail.
fn check_metadata_group(obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
    let mut reader = BitReader::new(obu.payload, obu.payload_offset());
    match parse_metadata_group(&mut reader, obu.header.extended_layer_id) {
        Ok(group) => {
            for unit in &group.units {
                check_metadata_group_unit(unit, obu, report);
            }
            finish_payload_or_emit(&mut reader, obu.payload, false, report);
        }
        Err(error) => report.push(
            syntax_error_diagnostic(&error)
                .unwrap_or_else(|| payload_parse_error_diagnostic(&error, "5.17.3")),
        ),
    }
}

/// Emits the locally-decidable § 6.16.3 / § 6.16.11 diagnostics for one metadata group
/// unit.
fn check_metadata_group_unit(
    unit: &MetadataGroupUnit,
    obu: &ObuEnvelope<'_>,
    report: &mut ValidationReport,
) {
    if let Some(reserved) = unit.muh_reserved_zero_2bits
        && reserved != 0
    {
        report.push(
            Diagnostic::warning(
                "metadata/group-reserved-bits-nonzero",
                format!(
                    "muh_reserved_zero_2bits must be 0 (found {reserved}); the value is ignored \
                     by a decoder"
                ),
            )
            .with_spec_section("6.16.3")
            .with_byte_offset(obu.offset),
        );
    }

    if let Some(layer_idc) = unit.muh_layer_idc
        && layer_idc >= 4
    {
        report.push(
            Diagnostic::warning(
                "metadata/group-layer-idc-reserved",
                format!(
                    "muh_layer_idc {layer_idc} is reserved for AOMedia use; not defined by this \
                     version of the specification"
                ),
            )
            .with_spec_section("6.16.3")
            .with_byte_offset(obu.offset),
        );
    }

    if let Some(persistence_idc) = unit.muh_persistence_idc
        && persistence_idc >= 4
    {
        report.push(
            Diagnostic::warning(
                "metadata/persistence-idc-reserved",
                format!(
                    "muh_persistence_idc {persistence_idc} is reserved for AOMedia use; not \
                     defined by this version of the specification"
                ),
            )
            .with_spec_section("6.16.3")
            .with_byte_offset(obu.offset),
        );
    }

    if let Some(xlayer_map) = unit.muh_xlayer_map
        && (xlayer_map & (1 << 31)) != 0
    {
        report.push(
            Diagnostic::error(
                "metadata/group-xlayer-map-global-bit-set",
                "bit 31 of muh_xlayer_map must be 0",
            )
            .with_spec_section("6.16.3")
            .with_byte_offset(obu.offset),
        );
    }

    let obu_mlayer_id = obu.header.embedded_layer_id.get();
    if obu_mlayer_id > 0 {
        let below_obu_mlayer_mask = (1u16 << obu_mlayer_id) - 1;
        if unit
            .muh_mlayer_maps
            .iter()
            .any(|&map| u16::from(map) & below_obu_mlayer_mask != 0)
        {
            report.push(
                Diagnostic::error(
                    "metadata/group-mlayer-map-below-obu-mlayer",
                    format!(
                        "muh_mlayer_map must not set a bit below obu_mlayer_id ({obu_mlayer_id})"
                    ),
                )
                .with_spec_section("6.16.3")
                .with_byte_offset(obu.offset),
            );
        }
    }

    if unit.metadata_type == MetadataType::TemporalPointInfo {
        report.push(
            Diagnostic::error(
                "metadata/temporal-point-info-not-short",
                "METADATA_TYPE_TEMPORAL_POINT_INFO must only appear in an OBU_METADATA_SHORT",
            )
            .with_spec_section("6.16.11")
            .with_byte_offset(obu.offset),
        );
    }

    if let Some(metadata_unit) = &unit.unit {
        check_metadata_unit_payload(metadata_unit, obu, report);
    }
}

/// Emits the locally-decidable § 6.16.7 timecode and § 6.16.10 scan-type range
/// diagnostics for one metadata unit payload.
fn check_metadata_unit_payload(
    unit: &MetadataUnit,
    obu: &ObuEnvelope<'_>,
    report: &mut ValidationReport,
) {
    match &unit.payload {
        MetadataPayload::Timecode(timecode) => {
            if timecode.counting_type >= 7 {
                report.push(
                    Diagnostic::warning(
                        "metadata/timecode-counting-type-reserved",
                        format!(
                            "counting_type {} is reserved for AOMedia use; not defined by this \
                             version of the specification",
                            timecode.counting_type
                        ),
                    )
                    .with_spec_section("6.16.7")
                    .with_byte_offset(obu.offset),
                );
            }
            if let Some(seconds) = timecode.seconds_value
                && seconds > 59
            {
                report.push(
                    Diagnostic::error(
                        "metadata/timecode-seconds-out-of-range",
                        format!("seconds_value {seconds} must be in the range 0 to 59"),
                    )
                    .with_spec_section("6.16.7")
                    .with_byte_offset(obu.offset),
                );
            }
            if let Some(minutes) = timecode.minutes_value
                && minutes > 59
            {
                report.push(
                    Diagnostic::error(
                        "metadata/timecode-minutes-out-of-range",
                        format!("minutes_value {minutes} must be in the range 0 to 59"),
                    )
                    .with_spec_section("6.16.7")
                    .with_byte_offset(obu.offset),
                );
            }
            if let Some(hours) = timecode.hours_value
                && hours > 23
            {
                report.push(
                    Diagnostic::error(
                        "metadata/timecode-hours-out-of-range",
                        format!("hours_value {hours} must be in the range 0 to 23"),
                    )
                    .with_spec_section("6.16.7")
                    .with_byte_offset(obu.offset),
                );
            }
        }
        MetadataPayload::ScanType(scan_type) if scan_type.mps_pic_struct_type > 12 => {
            report.push(
                Diagnostic::error(
                    "metadata/scan-type-pic-struct-reserved",
                    format!(
                        "mps_pic_struct_type {} is reserved (must be 12 or less)",
                        scan_type.mps_pic_struct_type
                    ),
                )
                .with_spec_section("6.16.10")
                .with_byte_offset(obu.offset),
            );
        }
        MetadataPayload::DecodedFrameHash(frame_hash) if frame_hash.reserved != 0 => {
            report.push(
                Diagnostic::warning(
                    "metadata/decoded-frame-hash-reserved-nonzero",
                    format!(
                        "reserved must be 0 (found {}); the value is ignored by a decoder",
                        frame_hash.reserved
                    ),
                )
                .with_spec_section("6.16.13")
                .with_byte_offset(obu.offset),
            );
        }
        _ => {}
    }
}

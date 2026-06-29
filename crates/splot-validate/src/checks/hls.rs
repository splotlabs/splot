// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! High-level-syntax OBU checks: MSDO, multi-frame header, layer configuration
//! record, atlas segment, operating point set, buffer removal timing, quantizer
//! matrix, film grain, and content interpretation (AV2 § 5.6 – § 5.15). Each
//! parses the full payload, emits the locally decidable conformance diagnostics,
//! and validates the § 5.2.1 payload tail. Cross-OBU stateful checks live in
//! [`crate::context`].

use std::collections::BTreeSet;

use splot_core::annexb::ObuEnvelope;
use splot_core::bitio::BitReader;
use splot_core::headers::atlas_segment::{
    AtlasModeInfo, AtlasSegment, AtlasSegmentMode, parse_atlas_segment,
};
use splot_core::headers::buffer_removal_timing::parse_buffer_removal_timing;
use splot_core::headers::content_interpretation::parse_content_interpretation;
use splot_core::headers::film_grain::parse_film_grain;
use splot_core::headers::layer_config_record::{
    LayerConfigurationRecord, parse_layer_config_record,
};
use splot_core::headers::operating_point_set::parse_operating_point_set;
use splot_core::headers::quantizer_matrix::parse_quantizer_matrix;
use splot_core::hls::{parse_msdo, parse_multi_frame_header};
use splot_core::types::ObuType;

use super::{
    Check, finish_payload_or_emit, payload_parse_error_diagnostic, run_payload_syntax_check,
};
use crate::diagnostic::{Diagnostic, ValidationReport};

/// `OBU_MSDO` layer-id and `num_streams_minus_2` constraints (AV2 § 6.6).
pub(super) struct MsdoSyntax;

impl Check for MsdoSyntax {
    fn id(&self) -> &'static str {
        "msdo/syntax"
    }

    fn spec_section(&self) -> Option<&'static str> {
        Some("6.6")
    }

    fn run(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        if obu.header.obu_type != ObuType::Msdo {
            return;
        }

        let header = &obu.header;
        if header.temporal_layer_id.get() != 0
            || header.embedded_layer_id.get() != 0
            || !header.extended_layer_id.is_global()
        {
            report.push(
                Diagnostic::error(
                    "msdo/non-global-layer-id",
                    format!(
                        "OBU_MSDO requires obu_tlayer_id == 0, obu_mlayer_id == 0, and \
                         obu_xlayer_id == GLOBAL_XLAYER_ID (found tlayer={}, mlayer={}, xlayer={})",
                        header.temporal_layer_id.get(),
                        header.embedded_layer_id.get(),
                        header.extended_layer_id.get()
                    ),
                )
                .with_spec_section("6.6")
                .with_byte_offset(obu.offset),
            );
        }

        let mut reader = BitReader::new(obu.payload, obu.payload_offset());
        match parse_msdo(&mut reader) {
            Ok(msdo) => {
                if msdo.num_streams_minus_2 > 2 {
                    report.push(
                        Diagnostic::error(
                            "msdo/too-many-streams",
                            format!(
                                "num_streams_minus_2 {} must not exceed 2",
                                msdo.num_streams_minus_2
                            ),
                        )
                        .with_spec_section("6.6")
                        .with_byte_offset(obu.offset),
                    );
                }
                for (i, sub) in msdo.sub_streams().iter().enumerate() {
                    if msdo.multistream_profile_idc.get() < sub.sub_stream_max_profile {
                        report.push(
                            Diagnostic::error(
                                "msdo/profile-below-substream-max",
                                format!(
                                    "multistream_profile_idc {} is below \
                                     sub_stream_max_profile[{i}] {} (§ 6.6: \
                                     multistream_profile_idc must be >= every \
                                     sub_stream_max_profile[i])",
                                    msdo.multistream_profile_idc.get(),
                                    sub.sub_stream_max_profile
                                ),
                            )
                            .with_spec_section("6.6")
                            .with_byte_offset(obu.offset),
                        );
                    }
                }
                if crate::annex_a::is_reserved_profile(msdo.multistream_profile_idc.get()) {
                    report.push(
                        Diagnostic::error(
                            "annex-a/profile-reserved",
                            format!(
                                "multistream_profile_idc {} is reserved (5..=30); it conforms \
                                 to no AV2 profile defined in this version of the specification \
                                 (§ 6.6 binds its value space to seq_profile_idc / Annex A.2 \
                                 Table A.1; the spec's \"Table A.4\" cross-reference is an \
                                 erratum)",
                                msdo.multistream_profile_idc.get()
                            ),
                        )
                        .with_spec_section("A.2")
                        .with_byte_offset(obu.offset),
                    );
                }
                finish_payload_or_emit(&mut reader, obu.payload, false, report);
            }
            Err(error) => report.push(payload_parse_error_diagnostic(&error, "5.6")),
        }
    }
}

/// `OBU_MULTI_FRAME_HEADER` local id ranges (AV2 § 5.7 / § 6.4.1).
pub(super) struct MultiFrameHeaderSyntax;

impl Check for MultiFrameHeaderSyntax {
    fn id(&self) -> &'static str {
        "mfh/syntax"
    }

    fn spec_section(&self) -> Option<&'static str> {
        Some("5.7")
    }

    fn run(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        if obu.header.obu_type != ObuType::MultiFrameHeader {
            return;
        }

        let mut reader = BitReader::new(obu.payload, obu.payload_offset());
        match parse_multi_frame_header(&mut reader) {
            Ok(mfh) => {
                if !mfh.seq_header_id_in_range() {
                    report.push(
                        Diagnostic::error(
                            "mfh/seq-header-id-out-of-range",
                            format!(
                                "mfh_seq_header_id {} must be less than MAX_SEQ_NUM (16)",
                                mfh.mfh_seq_header_id
                            ),
                        )
                        .with_spec_section("6.4.1")
                        .with_byte_offset(obu.offset),
                    );
                }
                if !mfh.mfh_id_in_range() {
                    report.push(
                        Diagnostic::error(
                            "mfh/id-out-of-range",
                            format!("mfhId {} must be less than MAX_MFH_NUM (16)", mfh.mfh_id()),
                        )
                        .with_spec_section("5.7")
                        .with_byte_offset(obu.offset),
                    );
                }
                finish_payload_or_emit(&mut reader, obu.payload, true, report);
            }
            Err(error) => report.push(payload_parse_error_diagnostic(&error, "5.7")),
        }
    }
}

/// `OBU_LAYER_CONFIGURATION_RECORD` syntax: full `layer_config_record_obu()` parse,
/// the reserved-zero-bits anomaly, and payload-tail conformance (AV2 § 5.8 / § 6.8).
/// Cross-OBU LCR/atlas availability is stateful and handled in [`crate::context`].
pub(super) struct LayerConfigRecordSyntax;

impl Check for LayerConfigRecordSyntax {
    fn id(&self) -> &'static str {
        "lcr/syntax"
    }

    fn spec_section(&self) -> Option<&'static str> {
        Some("5.8")
    }

    fn run(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        run_payload_syntax_check(
            obu,
            report,
            ObuType::LayerConfigurationRecord,
            "5.8",
            true,
            |reader| parse_layer_config_record(reader, obu.header.extended_layer_id),
            |record, obu, report| {
                if record.has_nonzero_reserved_bits() {
                    report.push(
                        Diagnostic::warning(
                            "lcr/reserved-bits-nonzero",
                            "a layer configuration record reserved-zero field is non-zero; \
                             the value is ignored by a decoder",
                        )
                        .with_spec_section("6.8")
                        .with_byte_offset(obu.offset),
                    );
                }
                check_layer_config_record_semantics(record, obu, report);
            },
        );
    }
}

/// Checks the locally decidable § 6.8.2 / § 6.8.3 / § 6.8.4 layer-configuration-record
/// id, map, and aggregate-info value-space constraints on a parsed record and pushes any
/// `lcr/*` diagnostics.
fn check_layer_config_record_semantics(
    record: &LayerConfigurationRecord,
    obu: &ObuEnvelope<'_>,
    report: &mut ValidationReport,
) {
    match record {
        LayerConfigurationRecord::Global(global) => {
            if global.global_config_record_id == 0 {
                report.push(
                    Diagnostic::error(
                        "lcr/global-id-out-of-range",
                        "lcr_global_config_record_id must be in the range 1 to 7 (found 0)",
                    )
                    .with_spec_section("6.8.2")
                    .with_byte_offset(obu.offset),
                );
            }
            if global.xlayer_map == 0 {
                report.push(
                    Diagnostic::error(
                        "lcr/xlayer-map-empty",
                        "lcr_xlayer_map must be in the range 1 to (1 << 31) - 1 (found 0)",
                    )
                    .with_spec_section("6.8.2")
                    .with_byte_offset(obu.offset),
                );
            }
            if global.dependent_xlayers_flag {
                report.push(
                    Diagnostic::warning(
                        "lcr/dependent-xlayers-flag-nonzero",
                        "lcr_dependent_xlayers_flag must be 0; the value is ignored by a decoder",
                    )
                    .with_spec_section("6.8.2")
                    .with_byte_offset(obu.offset),
                );
            }
            if let Some(aggregate) = global.aggregate_info.as_ref() {
                if !crate::annex_a::is_defined_config_idc(aggregate.config_idc) {
                    report.push(
                        Diagnostic::error(
                            "lcr/config-idc-reserved",
                            format!(
                                "lcr_config_idc {} is reserved (Annex A.3 Table A.5 defines \
                                 multi-sequence configurations 0..=2; 3..=63 are reserved for \
                                 future extensions of this specification)",
                                aggregate.config_idc
                            ),
                        )
                        .with_spec_section("6.8.4")
                        .with_byte_offset(obu.offset),
                    );
                }
                if crate::annex_a::is_reserved_level(aggregate.aggregate_level_idx) {
                    report.push(
                        Diagnostic::error(
                            "lcr/aggregate-level-idx-reserved",
                            format!(
                                "lcr_aggregate_level_idx {} is reserved (Annex A.4 Table A.7 \
                                 reserves level indices 22..=30)",
                                aggregate.aggregate_level_idx
                            ),
                        )
                        .with_spec_section("6.8.4")
                        .with_byte_offset(obu.offset),
                    );
                }
                if !crate::annex_a::is_defined_max_interop(aggregate.max_interop) {
                    report.push(
                        Diagnostic::error(
                            "lcr/max-interop-reserved",
                            format!(
                                "lcr_max_interop {} is reserved (Annex A.3 Table A.3 defines \
                                 interoperability points 0, 1, 2, and 15; 3..=14 are reserved)",
                                aggregate.max_interop
                            ),
                        )
                        .with_spec_section("6.8.4")
                        .with_byte_offset(obu.offset),
                    );
                }
            }
        }
        LayerConfigurationRecord::Local(local) if local.local_id == 0 => {
            report.push(
                Diagnostic::error("lcr/local-id-zero", "lcr_local_id must not be equal to 0")
                    .with_spec_section("6.8.3")
                    .with_byte_offset(obu.offset),
            );
        }
        _ => {}
    }
}

/// `OBU_ATLAS_SEGMENT` syntax: full `atlas_segment_info_obu()` parse (including the
/// mode and segment/region range checks) and payload-tail conformance
/// (AV2 § 5.9 / § 6.9). Cross-OBU atlas availability is stateful and handled in
/// [`crate::context`].
pub(super) struct AtlasSegmentSyntax;

impl Check for AtlasSegmentSyntax {
    fn id(&self) -> &'static str {
        "atlas/syntax"
    }

    fn spec_section(&self) -> Option<&'static str> {
        Some("5.9")
    }

    fn run(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        run_payload_syntax_check(
            obu,
            report,
            ObuType::AtlasSegment,
            "5.9",
            true,
            parse_atlas_segment,
            check_atlas_segment_semantics,
        );
    }
}

/// Checks the locally decidable § 6.9 atlas-segment constraints on a parsed atlas and
/// pushes any `atlas/*` diagnostics.
fn check_atlas_segment_semantics(
    atlas: &AtlasSegment,
    obu: &ObuEnvelope<'_>,
    report: &mut ValidationReport,
) {
    if matches!(
        atlas.mode,
        AtlasSegmentMode::Multistream | AtlasSegmentMode::MultistreamAlpha
    ) && !obu.header.extended_layer_id.is_global()
    {
        report.push(
            Diagnostic::error(
                "atlas/multistream-requires-global-xlayer",
                format!(
                    "a multistream atlas (ats_atlas_segment_mode_idc {}) requires \
                     obu_xlayer_id == GLOBAL_XLAYER_ID, found {}",
                    atlas.mode.idc(),
                    obu.header.extended_layer_id.get()
                ),
            )
            .with_spec_section("6.9")
            .with_byte_offset(obu.offset),
        );
    }

    let input_stream_ids: Vec<u8> = match &atlas.mode_info {
        AtlasModeInfo::Basic(basic) => basic
            .segments
            .iter()
            .filter_map(|segment| segment.input_stream_id)
            .collect(),
        AtlasModeInfo::Multistream(msi) | AtlasModeInfo::MultistreamAlpha(msi) => msi
            .segments
            .iter()
            .map(|segment| segment.input_stream_id)
            .collect(),
        _ => Vec::new(),
    };
    let mut seen = BTreeSet::new();
    if input_stream_ids.iter().any(|id| !seen.insert(*id)) {
        report.push(
            Diagnostic::error(
                "atlas/duplicate-input-stream-id",
                "ats_input_stream_id / ats_msi_input_stream_id values of an atlas must be unique",
            )
            .with_spec_section("6.9.6")
            .with_byte_offset(obu.offset),
        );
    }
}

/// `OBU_OPERATING_POINT_SET` syntax: full `operating_point_set_obu()` parse (including
/// `operating_point_payload()` and its children) and payload-tail conformance
/// (AV2 § 5.10 / § 5.11). The locally decidable § 6.10 semantics and cross-OBU OPS
/// availability are stateful and handled in [`crate::context`].
pub(super) struct OperatingPointSetSyntax;

impl Check for OperatingPointSetSyntax {
    fn id(&self) -> &'static str {
        "ops/syntax"
    }

    fn spec_section(&self) -> Option<&'static str> {
        Some("5.10")
    }

    fn run(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        run_payload_syntax_check(
            obu,
            report,
            ObuType::OperatingPointSet,
            "5.10",
            true,
            |reader| parse_operating_point_set(reader, obu.header.extended_layer_id),
            |_, _, _| {},
        );
    }
}

/// `OBU_BUFFER_REMOVAL_TIMING` syntax: full `buffer_removal_timing_obu()` parse and
/// payload-tail conformance (AV2 § 5.12). The cross-OBU OPS reference checks are
/// stateful and handled in [`crate::context`].
pub(super) struct BufferRemovalTimingSyntax;

impl Check for BufferRemovalTimingSyntax {
    fn id(&self) -> &'static str {
        "brt/syntax"
    }

    fn spec_section(&self) -> Option<&'static str> {
        Some("5.12")
    }

    fn run(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        run_payload_syntax_check(
            obu,
            report,
            ObuType::BufferRemovalTiming,
            "5.12",
            false,
            parse_buffer_removal_timing,
            |_, _, _| {},
        );
    }
}

/// `OBU_QUANTIZATION_MATRIX` syntax: full `quantizer_matrix_obu()` parse and
/// payload-tail conformance (AV2 § 5.13). The cross-OBU § 6.12 duplicate-reset /
/// duplicate-level checks are stateful and handled in [`crate::context`].
pub(super) struct QuantizerMatrixSyntax;

impl Check for QuantizerMatrixSyntax {
    fn id(&self) -> &'static str {
        "qm/syntax"
    }

    fn spec_section(&self) -> Option<&'static str> {
        Some("5.13")
    }

    fn run(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        run_payload_syntax_check(
            obu,
            report,
            ObuType::QuantizationMatrix,
            "5.13",
            false,
            parse_quantizer_matrix,
            |_, _, _| {},
        );
    }
}

/// `OBU_FILM_GRAIN` syntax: full `film_grain_obu()` / `film_grain_model()` parse and
/// payload-tail conformance (AV2 § 5.14 / § 5.18.10.2). The cross-OBU § 6.13 update-
/// flags / chroma-idc / duplicate-slot checks are stateful and handled in
/// [`crate::context`].
pub(super) struct FilmGrainSyntax;

impl Check for FilmGrainSyntax {
    fn id(&self) -> &'static str {
        "film-grain/syntax"
    }

    fn spec_section(&self) -> Option<&'static str> {
        Some("5.14")
    }

    fn run(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        run_payload_syntax_check(
            obu,
            report,
            ObuType::FilmGrain,
            "5.14",
            false,
            parse_film_grain,
            |_, _, _| {},
        );
    }
}

/// `OBU_CONTENT_INTERPRETATION` syntax: reserved-bits and payload-tail conformance
/// (AV2 § 5.15 / § 6.14). Cross-embedded-layer timing consistency and repeated-CI
/// identity are stateful and handled in [`crate::context`].
pub(super) struct ContentInterpretationSyntax;

impl Check for ContentInterpretationSyntax {
    fn id(&self) -> &'static str {
        "content-interpretation/syntax"
    }

    fn spec_section(&self) -> Option<&'static str> {
        Some("5.15")
    }

    fn run(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        run_payload_syntax_check(
            obu,
            report,
            ObuType::ContentInterpretation,
            "5.15",
            true,
            parse_content_interpretation,
            |content_interpretation, obu, report| {
                if content_interpretation.reserved_2bit != 0 {
                    report.push(
                        Diagnostic::warning(
                            "content-interpretation/reserved-bits-nonzero",
                            format!(
                                "ci_reserved_2bit must be 0 (found {}); the value is ignored by a \
                                 decoder",
                                content_interpretation.reserved_2bit
                            ),
                        )
                        .with_spec_section("6.14")
                        .with_byte_offset(obu.offset),
                    );
                }
                if let Some(chroma) = content_interpretation.chroma_sample_position
                    && (chroma.top > 5 || chroma.bottom > 5)
                {
                    report.push(
                        Diagnostic::error(
                            "content-interpretation/chroma-sample-position-out-of-range",
                            format!(
                                "ci_chroma_sample_position top {} / bottom {} must each be <= 5",
                                chroma.top, chroma.bottom
                            ),
                        )
                        .with_spec_section("6.14")
                        .with_byte_offset(obu.offset),
                    );
                }
                if let Some(aspect_ratio) = content_interpretation.aspect_ratio
                    && aspect_ratio.aspect_ratio_idc != 255
                    && aspect_ratio.aspect_ratio_idc > 16
                {
                    report.push(
                        Diagnostic::error(
                            "content-interpretation/aspect-ratio-idc-out-of-range",
                            format!(
                                "ci_aspect_ratio_idc {} must be <= 16 when not equal to 255",
                                aspect_ratio.aspect_ratio_idc
                            ),
                        )
                        .with_spec_section("6.14")
                        .with_byte_offset(obu.offset),
                    );
                }
            },
        );
    }
}

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
        // Registry identifier; emitted diagnostics use their own rule ids.
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
                // AV2 § 6.6 (mirror `06-syntax-structures-semantics.md` line 1347):
                // "It is a requirement of bitstream conformance that
                // multistream_profile_idc is greater than or equal to
                // sub_stream_max_profile[i] for all i in the range 0 to
                // num_streams_minus_2 + 1, inclusive." Locally decidable from the
                // in-band MSDO alone — it is never suppressed by external HLS (an
                // external HLS set cannot redefine the in-band MSDO's own fields).
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
                // AV2 § 6.6 (mirror `06-syntax-structures-semantics.md` lines 1339-1341):
                // "The allowed values for multistream_profile_idc are the same as those
                // for seq_profile_idc as defined in Table A.4." The "Table A.4" reference
                // is a spec erratum: the seq_profile_idc value space is defined by
                // Annex A.2 Table A.1 (Table A.4 holds interoperability-point rows). The
                // value space is all this sentence constrains — there is no claim beyond
                // it — so a reserved (5..=30) value is flagged with the shared
                // `annex-a/profile-reserved` id (the same value-space verdict the
                // activated-header check emits for seq_profile_idc). Locally decidable;
                // not suppressed by external HLS.
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
                // AV2 § 5.2.1: OBU_MSDO is non-extensible, so the remaining payload
                // bits must form valid trailing_bits().
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
        // Registry identifier; emitted diagnostics use their own rule ids.
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
                // AV2 § 5.2.1: the multi-frame header is extensible, so a fully
                // parsed MFH (now including seg_info()) must have a valid
                // obu_extension_flag / trailing_bits tail.
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
        // Registry identifier; emitted diagnostics use their own rule ids.
        "lcr/syntax"
    }

    fn spec_section(&self) -> Option<&'static str> {
        Some("5.8")
    }

    fn run(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        // AV2 § 5.2.1: the layer configuration record is extensible.
        run_payload_syntax_check(
            obu,
            report,
            ObuType::LayerConfigurationRecord,
            "5.8",
            true,
            |reader| parse_layer_config_record(reader, obu.header.extended_layer_id),
            |record, obu, report| {
                if record.has_nonzero_reserved_bits() {
                    // AV2 § 6.8: the lcr_*_reserved_zero_* fields must be 0, but a
                    // decoder ignores the value, so a non-zero value is a producer
                    // anomaly (warning) rather than a decode-breaking error.
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
                // AV2 § 6.8.2: lcr_global_config_record_id is in the range 1..7.
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
                // AV2 § 6.8.2: lcr_xlayer_map is in the range 1..(1 << 31) - 1.
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
                // AV2 § 6.8.2: lcr_dependent_xlayers_flag must be 0, but a decoder
                // ignores the value, so a set flag is a producer anomaly (warning).
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
                // AV2 § 6.8.4 (mirror `06-syntax-structures-semantics.md` lines 1744-1759)
                // gives three "shall not contain values outside Annex A" requirements on the
                // aggregate info fields. Each is decidable from the parsed global LCR's
                // lcr_aggregate_info() alone (no activation — the requirement is on the
                // bitstream *containing* the value), so these are local value-space checks
                // like `annex-a/profile-reserved`. lcr_max_tier_flag is a 1-bit field with no
                // such clause, so it has no value-space check.
                if !crate::annex_a::is_defined_config_idc(aggregate.config_idc) {
                    // Mirror lines 1744-1747: Annex A.3 Table A.5 defines configurations 0..=2;
                    // 3..=63 are reserved for future extensions.
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
                    // Mirror lines 1749-1752: lcr_aggregate_level_idx shall not be outside
                    // Annex A. Annex A.4 Table A.7 (mirror line 321) reserves level indices
                    // 22..=30; the 5-bit field's other values are defined levels (0..=21) or
                    // "Maximum parameters" (31).
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
                    // Mirror lines 1757-1759: lcr_max_interop shall not be outside Annex A.
                    // Annex A.3 Table A.3 (mirror lines 125-138) defines interoperability
                    // points 0, 1, 2, and 15 ("max"); 3..=14 are reserved.
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
        // AV2 § 6.8.3: lcr_local_id is not equal to 0.
        LayerConfigurationRecord::Local(local) if local.local_id == 0 => {
            report.push(
                Diagnostic::error("lcr/local-id-zero", "lcr_local_id must not be equal to 0")
                    .with_spec_section("6.8.3")
                    .with_byte_offset(obu.offset),
            );
        }
        // A conformant local record, or (since `LayerConfigurationRecord` is
        // `#[non_exhaustive]`) a future scope variant, is left unchecked here.
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
        // Registry identifier; emitted diagnostics use their own rule ids.
        "atlas/syntax"
    }

    fn spec_section(&self) -> Option<&'static str> {
        Some("5.9")
    }

    fn run(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        // AV2 § 5.2.1: the atlas segment info OBU is extensible.
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
    // AV2 § 6.9: MULTISTREAM_ATLAS / MULTISTREAM_ALPHA_ATLAS require obu_xlayer_id to
    // equal GLOBAL_XLAYER_ID.
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

    // AV2 § 6.9.6: ats_input_stream_id values of a basic atlas must be unique; AV2
    // § 6.9.4 gives ats_msi_input_stream_id the same semantics, so the multistream
    // modes share the requirement.
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
        // Registry identifier; emitted diagnostics use their own rule ids.
        "ops/syntax"
    }

    fn spec_section(&self) -> Option<&'static str> {
        Some("5.10")
    }

    fn run(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        // AV2 § 5.2.1: the operating point set OBU is extensible. The locally
        // decidable § 6.10 semantics are stateful, so there is no per-OBU `check`.
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
        // Registry identifier; emitted diagnostics use their own rule ids.
        "brt/syntax"
    }

    fn spec_section(&self) -> Option<&'static str> {
        Some("5.12")
    }

    fn run(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        // AV2 § 5.2.1: OBU_BUFFER_REMOVAL_TIMING is not extensible (trailing_bits()
        // only). The cross-OBU OPS reference checks are stateful, so there is no
        // per-OBU `check`.
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
        // Registry identifier; emitted diagnostics use their own rule ids.
        "qm/syntax"
    }

    fn spec_section(&self) -> Option<&'static str> {
        Some("5.13")
    }

    fn run(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        // AV2 § 5.2.1: OBU_QUANTIZATION_MATRIX is not extensible (trailing_bits()
        // only). The cross-OBU § 6.12 duplicate checks are stateful, so there is no
        // per-OBU `check`.
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
        // Registry identifier; emitted diagnostics use their own rule ids.
        "film-grain/syntax"
    }

    fn spec_section(&self) -> Option<&'static str> {
        Some("5.14")
    }

    fn run(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        // AV2 § 5.2.1: OBU_FILM_GRAIN is not extensible (trailing_bits() only). The
        // cross-OBU § 6.13 checks are stateful, so there is no per-OBU `check`.
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
        // Registry identifier; emitted diagnostics use their own rule ids.
        "content-interpretation/syntax"
    }

    fn spec_section(&self) -> Option<&'static str> {
        Some("5.15")
    }

    fn run(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        // AV2 § 5.2.1: OBU_CONTENT_INTERPRETATION is extensible.
        run_payload_syntax_check(
            obu,
            report,
            ObuType::ContentInterpretation,
            "5.15",
            true,
            parse_content_interpretation,
            |content_interpretation, obu, report| {
                if content_interpretation.reserved_2bit != 0 {
                    // AV2 § 6.14: ci_reserved_2bit must be 0, but a decoder ignores
                    // the value, so a non-zero value is a producer anomaly (warning)
                    // rather than a hard, decode-breaking conformance error.
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
                // AV2 § 6.14: when present, ci_chroma_sample_position_top and
                // ci_chroma_sample_position_bottom must each be <= 5 (6 is the
                // inferred CSP_UNSPECIFIED, which is not coded).
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
                // AV2 § 6.14: when ci_aspect_ratio_idc is not 255 (the extended-SAR
                // marker), it must be <= 16.
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

// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Pluggable conformance checks run against parsed OBUs.
//!
//! Each [`Check`] enforces one constraint and emits structured [`Diagnostic`]s.
//! The checks here are the straightforward, header-only constraints from AV2
//! v1.0.0 § 6.2.2 (they do not require an activated sequence header). OBU
//! ordering and sequence/frame-level conformance are future work.
//
// TODO(spec: AV2-7.3-OBU-ORDERING): add OBU-ordering checks.

use std::collections::BTreeSet;

use splot_core::annexb::ObuEnvelope;
use splot_core::bitio::BitReader;
use splot_core::error::{
    AtlasSegmentErrorKind, ByteAlignmentErrorKind, Error, LayerConfigRecordErrorKind,
    SequenceHeaderErrorKind, TrailingBitsErrorKind,
};
use splot_core::headers::atlas_segment::{
    AtlasModeInfo, AtlasSegment, AtlasSegmentMode, parse_atlas_segment,
};
use splot_core::headers::content_interpretation::parse_content_interpretation;
use splot_core::headers::layer_config_record::{
    LayerConfigurationRecord, parse_layer_config_record,
};
use splot_core::headers::sequence::parse_sequence_header;
use splot_core::hls::{parse_msdo, parse_multi_frame_header};
use splot_core::obu::{finish_obu_payload, parse_trailing_bits};
use splot_core::tile::{MAX_TILE_COLS, MAX_TILE_ROWS, TileParams};
use splot_core::types::ObuType;

use crate::diagnostic::{Diagnostic, Severity, ValidationReport};
use crate::error_location::{error_bit_offset, error_offset};

/// A single conformance check over one OBU envelope.
pub trait Check {
    /// Stable rule id reported in diagnostics.
    fn id(&self) -> &'static str;
    /// Spec section this check enforces, if any.
    fn spec_section(&self) -> Option<&'static str>;
    /// Runs the check, pushing any findings into `report`.
    fn run(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport);
}

/// Returns the default check registry, in execution order.
#[must_use]
pub fn default_checks() -> Vec<Box<dyn Check>> {
    vec![
        Box::new(ReservedObuType),
        Box::new(ReservedObuAllZeroPayload),
        Box::new(TrailingBitsForEmptySyntaxObus),
        Box::new(SequenceHeaderSyntax),
        Box::new(MsdoSyntax),
        Box::new(MultiFrameHeaderSyntax),
        Box::new(LayerConfigRecordSyntax),
        Box::new(AtlasSegmentSyntax),
        Box::new(ContentInterpretationSyntax),
        Box::new(GlobalXLayerRequired),
        Box::new(GlobalXLayerRequiresBaseLayers),
        Box::new(GlobalXLayerAllowedTypes),
        Box::new(BaseLayerOnlyTypes),
        Box::new(TemporalLayerZeroOnlyTypes),
    ]
}

/// Converts core payload-boundary syntax errors into stable validator diagnostics.
#[must_use]
pub(crate) fn syntax_error_diagnostic(error: &Error) -> Option<Diagnostic> {
    match error {
        Error::InvalidTrailingBits {
            offset,
            bit_offset,
            kind,
        } => {
            let rule_id = match kind {
                TrailingBitsErrorKind::Empty => "trailing-bits/empty",
                TrailingBitsErrorKind::MissingOneBit => "trailing-bits/missing-one-bit",
                TrailingBitsErrorKind::ZeroBitNotZero => "trailing-bits/zero-bit-not-zero",
            };
            Some(
                Diagnostic::error(rule_id, kind.to_string())
                    .with_spec_section("6.2.3")
                    .with_byte_offset(*offset)
                    .with_bit_offset(*bit_offset),
            )
        }
        Error::InvalidByteAlignment {
            offset,
            bit_offset,
            kind,
        } => {
            let rule_id = match kind {
                ByteAlignmentErrorKind::ZeroBitNotZero => "byte-alignment/zero-bit-not-zero",
            };
            Some(
                Diagnostic::error(rule_id, kind.to_string())
                    .with_spec_section("6.2.4")
                    .with_byte_offset(*offset)
                    .with_bit_offset(*bit_offset),
            )
        }
        Error::InvalidSequenceHeader {
            offset,
            bit_offset,
            kind,
        } => {
            let (rule_id, spec_section) = match kind {
                SequenceHeaderErrorKind::SeqHeaderIdOutOfRange => {
                    ("sequence-header/seq-header-id-out-of-range", "6.4.1")
                }
                SequenceHeaderErrorKind::ChromaFormatOutOfRange => {
                    ("sequence-header/chroma-format-out-of-range", "6.4.1")
                }
                SequenceHeaderErrorKind::BitDepthOutOfRange => {
                    ("sequence-header/bit-depth-out-of-range", "6.4.1")
                }
                SequenceHeaderErrorKind::SeqMaxMlayerCountOutOfRange => {
                    ("sequence-header/seq-max-mlayer-count-out-of-range", "6.4.1")
                }
                SequenceHeaderErrorKind::CropLeftOutOfRange => {
                    ("sequence-header/crop-left-out-of-range", "6.4.1")
                }
                SequenceHeaderErrorKind::CropRightOutOfRange => {
                    ("sequence-header/crop-right-out-of-range", "6.4.1")
                }
                SequenceHeaderErrorKind::CropTopOutOfRange => {
                    ("sequence-header/crop-top-out-of-range", "6.4.1")
                }
                SequenceHeaderErrorKind::CropBottomOutOfRange => {
                    ("sequence-header/crop-bottom-out-of-range", "6.4.1")
                }
                SequenceHeaderErrorKind::TimingNumUnitsZero => {
                    ("sequence-header/timing-num-units-zero", "6.4.1")
                }
                SequenceHeaderErrorKind::TimingDisplayTickZero => {
                    ("sequence-header/timing-display-tick-zero", "6.4.12")
                }
                SequenceHeaderErrorKind::TimingTimeScaleZero => {
                    ("sequence-header/timing-time-scale-zero", "6.4.12")
                }
                SequenceHeaderErrorKind::TimingNumTicksOutOfRange => (
                    "sequence-header/timing-num-ticks-per-picture-out-of-range",
                    "6.4.12",
                ),
            };
            Some(
                Diagnostic::error(rule_id, kind.to_string())
                    .with_spec_section(spec_section)
                    .with_byte_offset(*offset)
                    .with_bit_offset(*bit_offset),
            )
        }
        Error::InvalidObuExtension { offset, bit_offset } => Some(
            Diagnostic::error(
                "obu-header/extension-flag-not-zero",
                "obu_extension_flag must be 0 in this specification version",
            )
            .with_spec_section("6.2.1")
            .with_byte_offset(*offset)
            .with_bit_offset(*bit_offset),
        ),
        Error::InvalidLayerConfigRecord {
            offset,
            bit_offset,
            kind,
        } => {
            let (rule_id, spec_section) = match kind {
                LayerConfigRecordErrorKind::PayloadSizeOverflow => {
                    ("lcr/payload-size-overflow", "6.8.6")
                }
            };
            Some(
                Diagnostic::error(rule_id, kind.to_string())
                    .with_spec_section(spec_section)
                    .with_byte_offset(*offset)
                    .with_bit_offset(*bit_offset),
            )
        }
        Error::InvalidAtlasSegment {
            offset,
            bit_offset,
            kind,
        } => {
            let (rule_id, spec_section) = match kind {
                AtlasSegmentErrorKind::ModeOutOfRange => ("atlas/segment-mode-out-of-range", "6.9"),
                AtlasSegmentErrorKind::RegionDimensionOutOfRange => {
                    ("atlas/region-dimension-out-of-range", "6.9.3.1")
                }
                AtlasSegmentErrorKind::SegmentCountOutOfRange => {
                    ("atlas/segment-count-out-of-range", "6.9.6")
                }
            };
            Some(
                Diagnostic::error(rule_id, kind.to_string())
                    .with_spec_section(spec_section)
                    .with_byte_offset(*offset)
                    .with_bit_offset(*bit_offset),
            )
        }
        _ => None,
    }
}

/// Builds and pushes a diagnostic located at `obu`, tagged with `check`'s id and section.
fn emit(
    report: &mut ValidationReport,
    check: &dyn Check,
    severity: Severity,
    obu: &ObuEnvelope<'_>,
    message: String,
) {
    let mut diagnostic =
        Diagnostic::new(severity, check.id(), message).with_byte_offset(obu.offset);
    if let Some(section) = check.spec_section() {
        diagnostic = diagnostic.with_spec_section(section);
    }
    report.push(diagnostic);
}

/// OBUs with empty payload syntax still carry `trailing_bits` when their declared
/// payload is non-empty. Until full payload dispatch exists, only these OBU types
/// can be checked without guessing where payload syntax ends.
struct TrailingBitsForEmptySyntaxObus;

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

/// Parses the full `sequence_header_obu()` syntax (general fields plus every
/// implemented § 5.4 child config) and maps violations into stable diagnostics.
///
/// A child config that is intentionally bounded (`seg_info()` / `tile_params()`) is
/// not a conformance error; the header is accepted in that case. A fully parsed
/// header additionally has its § 5.2.1 payload tail (`obu_extension_flag` /
/// `trailing_bits`) validated, so a truncated or malformed child config — which the
/// general-only check used to miss — is now reported by `validate`.
struct SequenceHeaderSyntax;

impl Check for SequenceHeaderSyntax {
    fn id(&self) -> &'static str {
        // Registry identifier only; emitted diagnostics use syntax_error_diagnostic() rule ids.
        "sequence-header/syntax"
    }

    fn spec_section(&self) -> Option<&'static str> {
        Some("5.4")
    }

    fn run(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        if obu.header.obu_type != ObuType::SequenceHeader {
            return;
        }

        let mut reader = BitReader::new(obu.payload, obu.payload_offset());
        match parse_sequence_header(&mut reader) {
            Ok(header) => {
                // Local §6.17.7 tile constraints on a parsed sequence tile config.
                if let Some(tile) = header.tile.as_ref()
                    && let Some(params) = tile.params.as_ref()
                {
                    check_tile_params(params, obu, report);
                }
                // A reserved-level tile config (bounded) is intentional, not an error;
                // only validate the payload tail when fully parsed.
                if header.is_fully_parsed()
                    && let Err(error) = finish_obu_payload(
                        &mut reader,
                        obu.payload,
                        obu.header.obu_type.is_extensible_obu(),
                    )
                    && let Some(diagnostic) = syntax_error_diagnostic(&error)
                {
                    report.push(diagnostic);
                }
            }
            Err(error) => {
                let diagnostic = syntax_error_diagnostic(&error)
                    .unwrap_or_else(|| payload_parse_error_diagnostic(&error, "5.4"));
                report.push(diagnostic);
            }
        }
    }
}

/// Checks the local §6.17.7 tile constraints on a parsed `tile_params()` result and
/// pushes any `tile-params/*` diagnostics.
///
/// The tile-count limits (§6.17.7.2) are reachable for a non-uniform config that codes
/// more than `MAX_TILE_COLS` / `MAX_TILE_ROWS` tiles. The frame-coverage checks
/// (§6.17.7.3) are a defensive, never-false-positive cross-check: the `ns()`-bounded
/// non-uniform parse caps each tile to the remaining superblocks, so `start_sb` always
/// lands exactly on `sb_cols` / `sb_rows`. They are therefore **unreachable for any
/// stream that parses without error** — a parse error would surface first — and exist
/// only to guard the invariant if `TileParams` is ever produced another way.
pub(crate) fn check_tile_params(
    params: &TileParams,
    obu: &ObuEnvelope<'_>,
    report: &mut ValidationReport,
) {
    if params.tile_cols > MAX_TILE_COLS {
        report.push(tile_params_error(
            "tile-params/tile-cols-out-of-range",
            "6.17.7.2",
            obu,
            format!(
                "TileCols {} must be less than or equal to MAX_TILE_COLS ({MAX_TILE_COLS})",
                params.tile_cols
            ),
        ));
    }
    if params.tile_rows > MAX_TILE_ROWS {
        report.push(tile_params_error(
            "tile-params/tile-rows-out-of-range",
            "6.17.7.2",
            obu,
            format!(
                "TileRows {} must be less than or equal to MAX_TILE_ROWS ({MAX_TILE_ROWS})",
                params.tile_rows
            ),
        ));
    }
    if !params.uniform_spacing {
        if !params.covers_cols {
            report.push(tile_params_error(
                "tile-params/nonuniform-cols-do-not-cover-frame",
                "6.17.7.3",
                obu,
                format!(
                    "non-uniform tile column widths must sum to sbCols ({})",
                    params.sb_cols
                ),
            ));
        }
        if !params.covers_rows {
            report.push(tile_params_error(
                "tile-params/nonuniform-rows-do-not-cover-frame",
                "6.17.7.3",
                obu,
                format!(
                    "non-uniform tile row heights must sum to sbRows ({})",
                    params.sb_rows
                ),
            ));
        }
    }
}

/// Builds a §6.17.7 `tile-params/*` diagnostic located at `obu`.
fn tile_params_error(
    rule_id: &'static str,
    spec_section: &'static str,
    obu: &ObuEnvelope<'_>,
    message: String,
) -> Diagnostic {
    Diagnostic::error(rule_id, message)
        .with_spec_section(spec_section)
        .with_byte_offset(obu.offset)
}

fn payload_parse_error_diagnostic(error: &Error, spec_section: &'static str) -> Diagnostic {
    let mut diagnostic = Diagnostic::error("bitstream/parse-error", error.to_string())
        .with_spec_section(spec_section);
    if let Some(offset) = error_offset(error) {
        diagnostic = diagnostic.with_byte_offset(offset);
    }
    if let Some(bit_offset) = error_bit_offset(error) {
        diagnostic = diagnostic.with_bit_offset(bit_offset);
    }
    diagnostic
}

/// `OBU_MSDO` layer-id and `num_streams_minus_2` constraints (AV2 § 6.6).
struct MsdoSyntax;

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
                // AV2 § 5.2.1: OBU_MSDO is non-extensible, so the remaining payload
                // bits must form valid trailing_bits().
                if let Err(error) = finish_obu_payload(&mut reader, obu.payload, false)
                    && let Some(diagnostic) = syntax_error_diagnostic(&error)
                {
                    report.push(diagnostic);
                }
            }
            Err(error) => report.push(payload_parse_error_diagnostic(&error, "5.6")),
        }
    }
}

/// `OBU_MULTI_FRAME_HEADER` local id ranges (AV2 § 5.7 / § 6.4.1).
struct MultiFrameHeaderSyntax;

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
                if let Err(error) = finish_obu_payload(&mut reader, obu.payload, true)
                    && let Some(diagnostic) = syntax_error_diagnostic(&error)
                {
                    report.push(diagnostic);
                }
            }
            Err(error) => report.push(payload_parse_error_diagnostic(&error, "5.7")),
        }
    }
}

/// `OBU_LAYER_CONFIGURATION_RECORD` syntax: full `layer_config_record_obu()` parse,
/// the reserved-zero-bits anomaly, and payload-tail conformance (AV2 § 5.8 / § 6.8).
/// Cross-OBU LCR/atlas availability is stateful and handled in [`crate::context`].
struct LayerConfigRecordSyntax;

impl Check for LayerConfigRecordSyntax {
    fn id(&self) -> &'static str {
        // Registry identifier; emitted diagnostics use their own rule ids.
        "lcr/syntax"
    }

    fn spec_section(&self) -> Option<&'static str> {
        Some("5.8")
    }

    fn run(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        if obu.header.obu_type != ObuType::LayerConfigurationRecord {
            return;
        }

        let mut reader = BitReader::new(obu.payload, obu.payload_offset());
        match parse_layer_config_record(&mut reader, obu.header.extended_layer_id) {
            Ok(record) => {
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
                check_layer_config_record_semantics(&record, obu, report);
                // AV2 § 5.2.1: the layer configuration record is extensible, so its
                // payload tail must be a valid obu_extension_flag / trailing_bits.
                if let Err(error) = finish_obu_payload(&mut reader, obu.payload, true)
                    && let Some(diagnostic) = syntax_error_diagnostic(&error)
                {
                    report.push(diagnostic);
                }
            }
            Err(error) => {
                let diagnostic = syntax_error_diagnostic(&error)
                    .unwrap_or_else(|| payload_parse_error_diagnostic(&error, "5.8"));
                report.push(diagnostic);
            }
        }
    }
}

/// Checks the locally decidable § 6.8.2 / § 6.8.3 layer-configuration-record id and
/// map constraints on a parsed record and pushes any `lcr/*` diagnostics.
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
struct AtlasSegmentSyntax;

impl Check for AtlasSegmentSyntax {
    fn id(&self) -> &'static str {
        // Registry identifier; emitted diagnostics use their own rule ids.
        "atlas/syntax"
    }

    fn spec_section(&self) -> Option<&'static str> {
        Some("5.9")
    }

    fn run(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        if obu.header.obu_type != ObuType::AtlasSegment {
            return;
        }

        let mut reader = BitReader::new(obu.payload, obu.payload_offset());
        match parse_atlas_segment(&mut reader) {
            Ok(atlas) => {
                check_atlas_segment_semantics(&atlas, obu, report);
                // AV2 § 5.2.1: the atlas segment info OBU is extensible, so its payload
                // tail must be a valid obu_extension_flag / trailing_bits.
                if let Err(error) = finish_obu_payload(&mut reader, obu.payload, true)
                    && let Some(diagnostic) = syntax_error_diagnostic(&error)
                {
                    report.push(diagnostic);
                }
            }
            Err(error) => {
                let diagnostic = syntax_error_diagnostic(&error)
                    .unwrap_or_else(|| payload_parse_error_diagnostic(&error, "5.9"));
                report.push(diagnostic);
            }
        }
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

/// `OBU_CONTENT_INTERPRETATION` syntax: reserved-bits and payload-tail conformance
/// (AV2 § 5.15 / § 6.14). Cross-embedded-layer timing consistency and repeated-CI
/// identity are stateful and handled in [`crate::context`].
struct ContentInterpretationSyntax;

impl Check for ContentInterpretationSyntax {
    fn id(&self) -> &'static str {
        // Registry identifier; emitted diagnostics use their own rule ids.
        "content-interpretation/syntax"
    }

    fn spec_section(&self) -> Option<&'static str> {
        Some("5.15")
    }

    fn run(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        if obu.header.obu_type != ObuType::ContentInterpretation {
            return;
        }

        let mut reader = BitReader::new(obu.payload, obu.payload_offset());
        match parse_content_interpretation(&mut reader) {
            Ok(content_interpretation) => {
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
                // AV2 § 5.2.1: OBU_CONTENT_INTERPRETATION is extensible, so a fully
                // parsed CI OBU must have a valid obu_extension_flag / trailing_bits
                // tail.
                if let Err(error) = finish_obu_payload(&mut reader, obu.payload, true)
                    && let Some(diagnostic) = syntax_error_diagnostic(&error)
                {
                    report.push(diagnostic);
                }
            }
            Err(error) => report.push(
                syntax_error_diagnostic(&error)
                    .unwrap_or_else(|| payload_parse_error_diagnostic(&error, "5.15")),
            ),
        }
    }
}

/// Informational: reserved OBU types are ignored by conformant decoders (AV2 Table 6.1).
struct ReservedObuType;

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
struct ReservedObuAllZeroPayload;

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

/// `OBU_MSDO` / `OBU_TEMPORAL_DELIMITER` must use `obu_xlayer_id == GLOBAL_XLAYER_ID` (§ 6.2.2).
struct GlobalXLayerRequired;

impl Check for GlobalXLayerRequired {
    fn id(&self) -> &'static str {
        "obu-header/global-xlayer-required"
    }

    fn spec_section(&self) -> Option<&'static str> {
        Some("6.2.2")
    }

    fn run(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        let header = &obu.header;
        if header.obu_type.requires_global_xlayer() && !header.extended_layer_id.is_global() {
            emit(
                report,
                self,
                Severity::Error,
                obu,
                format!(
                    "{} requires obu_xlayer_id == GLOBAL_XLAYER_ID (31), found {}",
                    header.obu_type.spec_name(),
                    header.extended_layer_id.get()
                ),
            );
        }
    }
}

/// `obu_xlayer_id == GLOBAL_XLAYER_ID` requires base embedded and temporal layers (§ 6.2.2).
struct GlobalXLayerRequiresBaseLayers;

impl Check for GlobalXLayerRequiresBaseLayers {
    fn id(&self) -> &'static str {
        "obu-header/global-xlayer-requires-base-layers"
    }

    fn spec_section(&self) -> Option<&'static str> {
        Some("6.2.2")
    }

    fn run(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        let header = &obu.header;
        if header.extended_layer_id.is_global()
            && (header.embedded_layer_id.get() != 0 || header.temporal_layer_id.get() != 0)
        {
            emit(
                report,
                self,
                Severity::Error,
                obu,
                format!(
                    "obu_xlayer_id == GLOBAL_XLAYER_ID requires obu_mlayer_id and obu_tlayer_id == 0 \
                     (found mlayer={}, tlayer={})",
                    header.embedded_layer_id.get(),
                    header.temporal_layer_id.get()
                ),
            );
        }
    }
}

/// `obu_xlayer_id == GLOBAL_XLAYER_ID` is only allowed for certain OBU types (§ 6.2.2).
struct GlobalXLayerAllowedTypes;

impl Check for GlobalXLayerAllowedTypes {
    fn id(&self) -> &'static str {
        "obu-header/global-xlayer-allowed-types"
    }

    fn spec_section(&self) -> Option<&'static str> {
        Some("6.2.2")
    }

    fn run(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        let header = &obu.header;
        if header.extended_layer_id.is_global() && !header.obu_type.permits_global_xlayer() {
            emit(
                report,
                self,
                Severity::Error,
                obu,
                format!(
                    "{} is not permitted to use obu_xlayer_id == GLOBAL_XLAYER_ID",
                    header.obu_type.spec_name()
                ),
            );
        }
    }
}

/// Sequence header, temporal delimiter, LCR, OPS, and atlas segment must be base-layer (§ 6.2.2).
struct BaseLayerOnlyTypes;

impl Check for BaseLayerOnlyTypes {
    fn id(&self) -> &'static str {
        "obu-header/base-layer-only-types"
    }

    fn spec_section(&self) -> Option<&'static str> {
        Some("6.2.2")
    }

    fn run(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        let header = &obu.header;
        if header.obu_type.requires_base_temporal_and_embedded_layer()
            && (header.temporal_layer_id.get() != 0 || header.embedded_layer_id.get() != 0)
        {
            emit(
                report,
                self,
                Severity::Error,
                obu,
                format!(
                    "{} requires obu_tlayer_id and obu_mlayer_id == 0 (found tlayer={}, mlayer={})",
                    header.obu_type.spec_name(),
                    header.temporal_layer_id.get(),
                    header.embedded_layer_id.get()
                ),
            );
        }
    }
}

/// Closed/open-loop key, switch, and RAS frames must have `obu_tlayer_id == 0` (§ 6.2.2).
struct TemporalLayerZeroOnlyTypes;

impl Check for TemporalLayerZeroOnlyTypes {
    fn id(&self) -> &'static str {
        "obu-header/temporal-layer-zero-only-types"
    }

    fn spec_section(&self) -> Option<&'static str> {
        Some("6.2.2")
    }

    fn run(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        let header = &obu.header;
        if header.obu_type.requires_base_temporal_layer() && header.temporal_layer_id.get() != 0 {
            emit(
                report,
                self,
                Severity::Error,
                obu,
                format!(
                    "{} requires obu_tlayer_id == 0 (found {})",
                    header.obu_type.spec_name(),
                    header.temporal_layer_id.get()
                ),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use splot_core::annexb::ObuEnvelope;
    use splot_core::obu::ObuHeader;
    use splot_core::span::{BitOffset, ByteOffset};
    use splot_core::types::{EmbeddedLayerId, ExtendedLayerId, TemporalLayerId};

    /// A minimal sequence-header OBU envelope for `check_tile_params` unit tests.
    fn dummy_obu() -> ObuEnvelope<'static> {
        ObuEnvelope {
            offset: ByteOffset::new(0),
            size: 1,
            header: ObuHeader {
                has_header_extension: false,
                obu_type: ObuType::SequenceHeader,
                temporal_layer_id: TemporalLayerId::from_bits(0),
                embedded_layer_id: EmbeddedLayerId::from_bits(0),
                extended_layer_id: ExtendedLayerId::from_bits(0),
                header_size_bytes: 1,
            },
            payload: &[],
        }
    }

    /// A valid single-tile uniform `TileParams` to mutate per test.
    fn base_tile_params() -> TileParams {
        TileParams {
            tile_cols: 1,
            tile_rows: 1,
            tile_cols_log2: 0,
            tile_rows_log2: 0,
            sb_cols: 1,
            sb_rows: 1,
            uniform_spacing: true,
            covers_cols: true,
            covers_rows: true,
        }
    }

    #[test]
    fn check_tile_params_accepts_valid_layout() {
        let mut report = ValidationReport::new();
        check_tile_params(&base_tile_params(), &dummy_obu(), &mut report);
        assert!(
            !report
                .diagnostics
                .iter()
                .any(|d| d.rule_id.starts_with("tile-params/")),
            "report was: {report}"
        );
    }

    #[test]
    fn check_tile_params_flags_too_many_columns_and_rows() {
        let params = TileParams {
            tile_cols: MAX_TILE_COLS + 1,
            tile_rows: MAX_TILE_ROWS + 1,
            ..base_tile_params()
        };
        let mut report = ValidationReport::new();
        check_tile_params(&params, &dummy_obu(), &mut report);
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.rule_id == "tile-params/tile-cols-out-of-range")
        );
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.rule_id == "tile-params/tile-rows-out-of-range")
        );
    }

    #[test]
    fn check_tile_params_flags_nonuniform_coverage_gaps() {
        let params = TileParams {
            uniform_spacing: false,
            covers_cols: false,
            covers_rows: false,
            ..base_tile_params()
        };
        let mut report = ValidationReport::new();
        check_tile_params(&params, &dummy_obu(), &mut report);
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.rule_id == "tile-params/nonuniform-cols-do-not-cover-frame")
        );
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.rule_id == "tile-params/nonuniform-rows-do-not-cover-frame")
        );
    }

    #[test]
    fn check_tile_params_ignores_coverage_for_uniform_layout() {
        // covers_* are always true for uniform layouts; even if they were not, a
        // uniform layout must not emit the non-uniform coverage diagnostics.
        let params = TileParams {
            uniform_spacing: true,
            covers_cols: false,
            covers_rows: false,
            ..base_tile_params()
        };
        let mut report = ValidationReport::new();
        check_tile_params(&params, &dummy_obu(), &mut report);
        assert!(
            !report
                .diagnostics
                .iter()
                .any(|d| d.rule_id.starts_with("tile-params/nonuniform"))
        );
    }

    #[test]
    fn syntax_error_diagnostic_maps_trailing_bits_errors() {
        let diagnostic = syntax_error_diagnostic(&Error::InvalidTrailingBits {
            offset: ByteOffset::new(3),
            bit_offset: BitOffset::from_bits(1),
            kind: TrailingBitsErrorKind::ZeroBitNotZero,
        });
        assert!(
            diagnostic.is_some(),
            "trailing-bit error should map to a diagnostic"
        );
        let diagnostic =
            diagnostic.unwrap_or_else(|| Diagnostic::error("trailing-bits/test", "missing"));
        assert_eq!(diagnostic.rule_id, "trailing-bits/zero-bit-not-zero");
        assert_eq!(diagnostic.spec_section.as_deref(), Some("6.2.3"));
        assert_eq!(diagnostic.byte_offset, Some(ByteOffset::new(3)));
        assert_eq!(diagnostic.bit_offset, Some(BitOffset::from_bits(1)));
    }

    #[test]
    fn syntax_error_diagnostic_maps_byte_alignment_errors() {
        let diagnostic = syntax_error_diagnostic(&Error::InvalidByteAlignment {
            offset: ByteOffset::new(7),
            bit_offset: BitOffset::from_bits(5),
            kind: ByteAlignmentErrorKind::ZeroBitNotZero,
        });
        assert!(
            diagnostic.is_some(),
            "byte-alignment error should map to a diagnostic"
        );
        let diagnostic =
            diagnostic.unwrap_or_else(|| Diagnostic::error("byte-alignment/test", "missing"));
        assert_eq!(diagnostic.rule_id, "byte-alignment/zero-bit-not-zero");
        assert_eq!(diagnostic.spec_section.as_deref(), Some("6.2.4"));
        assert_eq!(diagnostic.byte_offset, Some(ByteOffset::new(7)));
        assert_eq!(diagnostic.bit_offset, Some(BitOffset::from_bits(5)));
    }

    #[test]
    fn syntax_error_diagnostic_maps_obu_extension_flag() {
        let diagnostic = syntax_error_diagnostic(&Error::InvalidObuExtension {
            offset: ByteOffset::new(9),
            bit_offset: BitOffset::from_bits(3),
        })
        .unwrap_or_else(|| Diagnostic::error("obu-header/test", "missing"));
        assert_eq!(diagnostic.rule_id, "obu-header/extension-flag-not-zero");
        assert_eq!(diagnostic.spec_section.as_deref(), Some("6.2.1"));
        assert_eq!(diagnostic.byte_offset, Some(ByteOffset::new(9)));
    }

    #[test]
    fn syntax_error_diagnostic_maps_timing_errors() {
        for (kind, rule_id) in [
            (
                SequenceHeaderErrorKind::TimingDisplayTickZero,
                "sequence-header/timing-display-tick-zero",
            ),
            (
                SequenceHeaderErrorKind::TimingTimeScaleZero,
                "sequence-header/timing-time-scale-zero",
            ),
            (
                SequenceHeaderErrorKind::TimingNumTicksOutOfRange,
                "sequence-header/timing-num-ticks-per-picture-out-of-range",
            ),
        ] {
            let diagnostic = syntax_error_diagnostic(&Error::InvalidSequenceHeader {
                offset: ByteOffset::new(4),
                bit_offset: BitOffset::from_bits(0),
                kind,
            })
            .unwrap_or_else(|| Diagnostic::error("sequence-header/test", "missing"));
            assert_eq!(diagnostic.rule_id, rule_id);
            assert_eq!(diagnostic.spec_section.as_deref(), Some("6.4.12"));
        }
    }

    #[test]
    fn syntax_error_diagnostic_maps_sequence_header_errors() {
        let diagnostic = syntax_error_diagnostic(&Error::InvalidSequenceHeader {
            offset: ByteOffset::new(11),
            bit_offset: BitOffset::from_bits(2),
            kind: SequenceHeaderErrorKind::ChromaFormatOutOfRange,
        });
        assert!(
            diagnostic.is_some(),
            "sequence-header error should map to a diagnostic"
        );
        let diagnostic =
            diagnostic.unwrap_or_else(|| Diagnostic::error("sequence-header/test", "missing"));
        assert_eq!(
            diagnostic.rule_id,
            "sequence-header/chroma-format-out-of-range"
        );
        assert_eq!(diagnostic.spec_section.as_deref(), Some("6.4.1"));
        assert_eq!(diagnostic.byte_offset, Some(ByteOffset::new(11)));
        assert_eq!(diagnostic.bit_offset, Some(BitOffset::from_bits(2)));
    }
}

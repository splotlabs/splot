// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `OBU_SEQUENCE_HEADER` syntax check and the locally decidable § 6.17.7
//! `tile_params()` constraints (AV2 § 5.4).

use splot_core::annexb::ObuEnvelope;
use splot_core::bitio::BitReader;
use splot_core::headers::sequence::parse_sequence_header;
use splot_core::tile::{MAX_TILE_COLS, MAX_TILE_ROWS, TileParams};
use splot_core::types::ObuType;

use super::{
    Check, finish_payload_or_emit, payload_parse_error_diagnostic, syntax_error_diagnostic,
};
use crate::diagnostic::{Diagnostic, ValidationReport};

/// Parses the full `sequence_header_obu()` syntax (general fields plus every
/// implemented § 5.4 child config) and maps violations into stable diagnostics.
///
/// A child config that is intentionally bounded (`seg_info()` / `tile_params()`) is
/// not a conformance error; the header is accepted in that case. A fully parsed
/// header additionally has its § 5.2.1 payload tail (`obu_extension_flag` /
/// `trailing_bits`) validated, so a truncated or malformed child config — which the
/// general-only check used to miss — is now reported by `validate`.
pub(super) struct SequenceHeaderSyntax;

impl Check for SequenceHeaderSyntax {
    fn id(&self) -> &'static str {
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
                if let Some(tile) = header.tile.as_ref()
                    && let Some(params) = tile.params.as_ref()
                {
                    check_tile_params(params, obu, report);
                }
                if header.is_fully_parsed() {
                    finish_payload_or_emit(
                        &mut reader,
                        obu.payload,
                        obu.header.obu_type.is_extensible_obu(),
                        report,
                    );
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

#[cfg(test)]
mod tests {
    use super::*;
    use splot_core::obu::ObuHeader;
    use splot_core::span::ByteOffset;
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
}

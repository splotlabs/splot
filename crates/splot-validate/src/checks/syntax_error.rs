// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Maps `splot_core` payload-boundary syntax errors to stable validator diagnostics.
//!
//! [`syntax_error_diagnostic`] is the single entry point: it dispatches on the
//! [`Error`] variant and delegates each error family to a small per-family mapper
//! that owns that family's `rule_id` / `spec_section` table. [`payload_parse_error_diagnostic`]
//! is the generic fallback for errors that carry no dedicated conformance mapping.

use splot_core::error::{
    AtlasSegmentErrorKind, ByteAlignmentErrorKind, Error, LayerConfigRecordErrorKind,
    MetadataErrorKind, PaddingErrorKind, SequenceHeaderErrorKind, TileParamsErrorKind,
    TrailingBitsErrorKind,
};
use splot_core::span::{BitOffset, ByteOffset};

use crate::diagnostic::Diagnostic;
use crate::error_location::{error_bit_offset, error_offset};

/// Converts core payload-boundary syntax errors into stable validator diagnostics.
#[must_use]
pub(crate) fn syntax_error_diagnostic(error: &Error) -> Option<Diagnostic> {
    match error {
        Error::InvalidTrailingBits {
            offset,
            bit_offset,
            kind,
        } => Some(trailing_bits_diagnostic(*kind, *offset, *bit_offset)),
        Error::InvalidByteAlignment {
            offset,
            bit_offset,
            kind,
        } => Some(byte_alignment_diagnostic(*kind, *offset, *bit_offset)),
        Error::InvalidSequenceHeader {
            offset,
            bit_offset,
            kind,
        } => Some(sequence_header_diagnostic(*kind, *offset, *bit_offset)),
        Error::InvalidTileParams {
            offset,
            bit_offset,
            kind,
        } => Some(tile_params_diagnostic(*kind, *offset, *bit_offset)),
        Error::InvalidObuExtension { offset, bit_offset } => {
            Some(obu_extension_diagnostic(*offset, *bit_offset))
        }
        Error::InvalidLayerConfigRecord {
            offset,
            bit_offset,
            kind,
        } => Some(layer_config_record_diagnostic(*kind, *offset, *bit_offset)),
        Error::InvalidAtlasSegment {
            offset,
            bit_offset,
            kind,
        } => Some(atlas_segment_diagnostic(*kind, *offset, *bit_offset)),
        Error::InvalidQuantizerMatrix {
            offset,
            bit_offset,
            message,
        } => Some(quantizer_matrix_diagnostic(message, *offset, *bit_offset)),
        Error::InvalidPadding {
            offset,
            bit_offset,
            kind,
        } => Some(padding_diagnostic(*kind, *offset, *bit_offset)),
        Error::InvalidMetadata {
            offset,
            bit_offset,
            kind,
        } => Some(metadata_diagnostic(*kind, *offset, *bit_offset)),
        _ => None,
    }
}

/// Locates a fully built diagnostic at the given byte/bit offset.
fn located(diagnostic: Diagnostic, offset: ByteOffset, bit_offset: BitOffset) -> Diagnostic {
    diagnostic
        .with_byte_offset(offset)
        .with_bit_offset(bit_offset)
}

fn trailing_bits_diagnostic(
    kind: TrailingBitsErrorKind,
    offset: ByteOffset,
    bit_offset: BitOffset,
) -> Diagnostic {
    let rule_id = match kind {
        TrailingBitsErrorKind::Empty => "trailing-bits/empty",
        TrailingBitsErrorKind::MissingOneBit => "trailing-bits/missing-one-bit",
        TrailingBitsErrorKind::ZeroBitNotZero => "trailing-bits/zero-bit-not-zero",
    };
    located(
        Diagnostic::error(rule_id, kind.to_string()).with_spec_section("6.2.3"),
        offset,
        bit_offset,
    )
}

fn byte_alignment_diagnostic(
    kind: ByteAlignmentErrorKind,
    offset: ByteOffset,
    bit_offset: BitOffset,
) -> Diagnostic {
    let rule_id = match kind {
        ByteAlignmentErrorKind::ZeroBitNotZero => "byte-alignment/zero-bit-not-zero",
    };
    located(
        Diagnostic::error(rule_id, kind.to_string()).with_spec_section("6.2.4"),
        offset,
        bit_offset,
    )
}

fn sequence_header_diagnostic(
    kind: SequenceHeaderErrorKind,
    offset: ByteOffset,
    bit_offset: BitOffset,
) -> Diagnostic {
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
    located(
        Diagnostic::error(rule_id, kind.to_string()).with_spec_section(spec_section),
        offset,
        bit_offset,
    )
}

fn tile_params_diagnostic(
    kind: TileParamsErrorKind,
    offset: ByteOffset,
    bit_offset: BitOffset,
) -> Diagnostic {
    let rule_id = match kind {
        TileParamsErrorKind::TileColsOutOfRange => "tile-params/tile-cols-out-of-range",
        TileParamsErrorKind::TileRowsOutOfRange => "tile-params/tile-rows-out-of-range",
    };
    located(
        Diagnostic::error(rule_id, kind.to_string()).with_spec_section("6.17.7.2"),
        offset,
        bit_offset,
    )
}

fn obu_extension_diagnostic(offset: ByteOffset, bit_offset: BitOffset) -> Diagnostic {
    located(
        Diagnostic::error(
            "obu-header/extension-flag-not-zero",
            "obu_extension_flag must be 0 in this specification version",
        )
        .with_spec_section("6.2.1"),
        offset,
        bit_offset,
    )
}

fn layer_config_record_diagnostic(
    kind: LayerConfigRecordErrorKind,
    offset: ByteOffset,
    bit_offset: BitOffset,
) -> Diagnostic {
    let (rule_id, spec_section) = match kind {
        LayerConfigRecordErrorKind::PayloadSizeOverflow => ("lcr/payload-size-overflow", "6.8.6"),
    };
    located(
        Diagnostic::error(rule_id, kind.to_string()).with_spec_section(spec_section),
        offset,
        bit_offset,
    )
}

fn atlas_segment_diagnostic(
    kind: AtlasSegmentErrorKind,
    offset: ByteOffset,
    bit_offset: BitOffset,
) -> Diagnostic {
    let (rule_id, spec_section) = match kind {
        AtlasSegmentErrorKind::ModeOutOfRange => ("atlas/segment-mode-out-of-range", "6.9"),
        AtlasSegmentErrorKind::RegionDimensionOutOfRange => {
            ("atlas/region-dimension-out-of-range", "6.9.3.1")
        }
        AtlasSegmentErrorKind::SegmentCountOutOfRange => {
            ("atlas/segment-count-out-of-range", "6.9.6")
        }
    };
    located(
        Diagnostic::error(rule_id, kind.to_string()).with_spec_section(spec_section),
        offset,
        bit_offset,
    )
}

fn quantizer_matrix_diagnostic(
    message: &str,
    offset: ByteOffset,
    bit_offset: BitOffset,
) -> Diagnostic {
    located(
        Diagnostic::error("qm/quant-delta-out-of-range", message.to_owned())
            .with_spec_section("6.4.11"),
        offset,
        bit_offset,
    )
}

fn padding_diagnostic(
    kind: PaddingErrorKind,
    offset: ByteOffset,
    bit_offset: BitOffset,
) -> Diagnostic {
    let (rule_id, spec_section) = match kind {
        PaddingErrorKind::AllZeroPayload => ("padding/all-zero-payload", "5.16"),
        PaddingErrorKind::InvalidTrailingBits => ("padding/invalid-trailing-bits", "5.16"),
    };
    located(
        Diagnostic::error(rule_id, kind.to_string()).with_spec_section(spec_section),
        offset,
        bit_offset,
    )
}

fn metadata_diagnostic(
    kind: MetadataErrorKind,
    offset: ByteOffset,
    bit_offset: BitOffset,
) -> Diagnostic {
    let (rule_id, spec_section) = match kind {
        MetadataErrorKind::UnitPayloadUnderflow => ("metadata/unit-payload-underflow", "6.16.1"),
        MetadataErrorKind::GroupUnitCountTooLarge => {
            ("metadata/group-unit-count-too-large", "6.16.3")
        }
        MetadataErrorKind::GroupHeaderUnderflow => ("metadata/group-header-underflow", "6.16.3"),
    };
    located(
        Diagnostic::error(rule_id, kind.to_string()).with_spec_section(spec_section),
        offset,
        bit_offset,
    )
}

/// Builds the generic `bitstream/parse-error` diagnostic for an error with no
/// dedicated conformance mapping, tagging the best-effort spec section and location.
pub(crate) fn payload_parse_error_diagnostic(
    error: &Error,
    spec_section: &'static str,
) -> Diagnostic {
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

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn syntax_error_diagnostic_maps_tile_param_errors() {
        for (kind, rule_id) in [
            (
                TileParamsErrorKind::TileColsOutOfRange,
                "tile-params/tile-cols-out-of-range",
            ),
            (
                TileParamsErrorKind::TileRowsOutOfRange,
                "tile-params/tile-rows-out-of-range",
            ),
        ] {
            let diagnostic = syntax_error_diagnostic(&Error::InvalidTileParams {
                offset: ByteOffset::new(13),
                bit_offset: BitOffset::from_bits(4),
                kind,
            })
            .unwrap_or_else(|| Diagnostic::error("tile-params/test", "missing"));
            assert_eq!(diagnostic.rule_id, rule_id);
            assert_eq!(diagnostic.spec_section.as_deref(), Some("6.17.7.2"));
            assert_eq!(diagnostic.byte_offset, Some(ByteOffset::new(13)));
            assert_eq!(diagnostic.bit_offset, Some(BitOffset::from_bits(4)));
        }
    }
}

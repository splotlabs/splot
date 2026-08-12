// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use splot_core::Error as CoreError;
use splot_core::error::SymbolDecoderErrorKind;
use splot_core::span::ByteOffset;
use splot_core::symbol::{CdfUpdateMode, Symbol, SymbolDecoder, SymbolDecoderConfig};
use splot_core::symbol_encoder::SymbolEncoder;

use super::super::cdf::block_read::BlockSymbolTraceReadError;
use super::super::cdf::{FrameCdfSubset, TileCdfArray, TileCdfError};
use super::*;

fn symbol_decoder(payload: &[u8], base: ByteOffset, mode: CdfUpdateMode) -> SymbolDecoder<'_> {
    SymbolDecoder::with_base_and_config(
        payload,
        base,
        SymbolDecoderConfig::new().with_cdf_update_mode(mode),
    )
    .unwrap()
}

fn encode_refined_eob() -> (Vec<u8>, TileCdfSubset) {
    let mut tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut encoder = SymbolEncoder::new();
    for (selector, symbol) in [
        (
            TileCdfSelector::EobPt {
                size: EobPtSize::Pt128,
                coeff_cdf_q_ctx: 0,
                eob_ctx: 0,
            },
            3,
        ),
        (TileCdfSelector::EobExtra { coeff_cdf_q_ctx: 0 }, 1),
    ] {
        tile.with_row_mut(selector, |row| {
            encoder.write_symbol_u16(row, Symbol::new(symbol))
        })
        .unwrap()
        .unwrap();
    }
    encoder.write_literal(1, 1).unwrap();
    (encoder.finish().unwrap().into_bytes(), tile)
}

#[test]
fn nonzero_eob_covers_direct_refined_and_max_values() {
    for (input, expected) in [
        (
            NonZeroCoeffEobInput {
                eob_pt: 1,
                eob_extra: false,
                eob_extra_bits: 0,
            },
            1,
        ),
        (
            NonZeroCoeffEobInput {
                eob_pt: 6,
                eob_extra: true,
                eob_extra_bits: 0b110,
            },
            31,
        ),
        (
            NonZeroCoeffEobInput {
                eob_pt: 11,
                eob_extra: true,
                eob_extra_bits: 0xff,
            },
            1024,
        ),
    ] {
        assert_eq!(nonzero_coeff_eob(input).unwrap().eob(), expected);
    }
}

#[test]
fn nonzero_eob_rejects_invalid_points_and_refinements() {
    assert!(matches!(
        nonzero_coeff_eob(NonZeroCoeffEobInput {
            eob_pt: 0,
            eob_extra: false,
            eob_extra_bits: 0,
        }),
        Err(CoeffLoopContextError::InvalidEobPoint { eob_pt: 0 })
    ));
    assert!(matches!(
        nonzero_coeff_eob(NonZeroCoeffEobInput {
            eob_pt: 1,
            eob_extra: true,
            eob_extra_bits: 0,
        }),
        Err(CoeffLoopContextError::UnexpectedEobRefinement {
            eob_pt: 1,
            eob_extra: true,
            eob_extra_bits: 0,
        })
    ));
    assert!(matches!(
        nonzero_coeff_eob(NonZeroCoeffEobInput {
            eob_pt: 4,
            eob_extra: false,
            eob_extra_bits: 0b10,
        }),
        Err(CoeffLoopContextError::EobExtraBitsOutOfRange {
            eob_pt: 4,
            eob_extra_bits: 2,
            max_eob_extra_bits: 1,
        })
    ));
}

#[test]
fn live_eob_read_uses_selected_cdfs_and_literal_refinement() {
    let (payload, expected_tile) = encode_refined_eob();
    let mut tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut symbols = symbol_decoder(&payload, ByteOffset::new(0), CdfUpdateMode::Enabled);

    let read = read_nonzero_coeff_eob(
        &mut tile,
        &mut symbols,
        NonZeroCoeffEobSymbolInput {
            size: EobPtSize::Pt128,
            coeff_cdf_q_ctx: 0,
            eob_ctx: 0,
        },
    )
    .unwrap();

    assert_eq!(read.eob().eob(), 8);
    assert_eq!(symbols.symbol_count(), 3);
    assert_eq!(tile, expected_tile);
    assert!(symbols.finish().is_ok());
}

#[test]
fn live_eob_read_honors_disabled_cdf_updates() {
    let (payload, _) = encode_refined_eob();
    let mut tile = FrameCdfSubset::from_defaults().tile_copy();
    let before = tile.clone();
    let mut symbols = symbol_decoder(&payload, ByteOffset::new(0), CdfUpdateMode::Disabled);

    let read = read_nonzero_coeff_eob(
        &mut tile,
        &mut symbols,
        NonZeroCoeffEobSymbolInput {
            size: EobPtSize::Pt128,
            coeff_cdf_q_ctx: 0,
            eob_ctx: 0,
        },
    )
    .unwrap();

    assert_eq!(read.eob().eob(), 8);
    assert_eq!(symbols.symbol_count(), 3);
    assert_eq!(tile, before);
}

#[test]
fn live_eob_read_rejects_selector_before_consumption() {
    let mut tile = FrameCdfSubset::from_defaults().tile_copy();
    let before = tile.clone();
    let mut symbols = symbol_decoder(&[0x80], ByteOffset::new(7), CdfUpdateMode::Enabled);
    let checkpoint = symbols.checkpoint();

    let error = read_nonzero_coeff_eob(
        &mut tile,
        &mut symbols,
        NonZeroCoeffEobSymbolInput {
            size: EobPtSize::Pt16,
            coeff_cdf_q_ctx: 4,
            eob_ctx: 0,
        },
    )
    .unwrap_err();

    assert!(matches!(
        error,
        CoeffLoopContextError::EobSymbolRead(BlockSymbolTraceReadError::Cdf(
            TileCdfError::SelectorOutOfRange {
                array: TileCdfArray::EobPt,
                index_name: "coeff_cdf_q_ctx",
                actual: 4,
                max_exclusive: 4,
            }
        ))
    ));
    assert_eq!(symbols.checkpoint(), checkpoint);
    assert_eq!(tile, before);
}

#[test]
fn live_eob_read_reports_truncation_and_wrapped_offsets() {
    let mut tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut truncated = symbol_decoder(&[], ByteOffset::new(23), CdfUpdateMode::Enabled);
    let _ = read_nonzero_coeff_eob(
        &mut tile,
        &mut truncated,
        NonZeroCoeffEobSymbolInput {
            size: EobPtSize::Pt16,
            coeff_cdf_q_ctx: 0,
            eob_ctx: 0,
        },
    )
    .unwrap();
    assert!(matches!(
        truncated.finish(),
        Err(CoreError::InvalidSymbolDecoderState {
            offset,
            kind: SymbolDecoderErrorKind::SymbolMaxBitsTooSmall { .. },
            ..
        }) if offset == ByteOffset::new(23)
    ));

    let mut invalid_width = symbol_decoder(&[0x80], ByteOffset::new(31), CdfUpdateMode::Enabled);
    let error = read_eob_literal(&mut invalid_width, 33, "eob_extra_bit").unwrap_err();
    assert!(matches!(
        error,
        CoeffLoopContextError::EobLiteralRead {
            syntax: "eob_extra_bit",
            source: CoreError::InvalidSymbolDecoderState {
                offset,
                kind: SymbolDecoderErrorKind::LiteralWidthTooLarge {
                    requested: 33,
                    max: 32,
                },
                ..
            },
        } if offset == ByteOffset::new(32)
    ));
}

#[test]
fn coefficient_error_classifier_preserves_leaf_taxonomy() {
    let pt512 = CoeffLoopContextError::InvalidPt512EobExtra { eob_pt_extra: 3 };
    assert_eq!(
        classify_coeff_parse_error(&pt512, "5.20.7.27"),
        CoeffParseErrorClass::Malformed {
            spec_section: "6.19.7.23"
        }
    );

    let golomb = read_quant::CoeffReadQuantError::OverlongGolombPrefix { index: 0 };
    assert_eq!(
        classify_coeff_parse_error(&golomb, "5.20.7.27"),
        CoeffParseErrorClass::Malformed {
            spec_section: "6.19.7.24"
        }
    );

    let magnitude = quant_state::CoeffQuantStateWriteError::QuantMagnitudeOutOfRange {
        index: 0,
        magnitude: 1 << 20,
    };
    assert_eq!(
        classify_coeff_parse_error(&magnitude, "5.20.7.27"),
        CoeffParseErrorClass::Malformed {
            spec_section: "6.19.7.23"
        }
    );

    let entropy = BlockSymbolTraceReadError::Cdf(TileCdfError::UnexpectedSelector);
    assert_eq!(
        classify_coeff_parse_error(&entropy, "5.20.7.27"),
        CoeffParseErrorClass::EntropyState
    );

    let invariant = CoeffLoopContextError::InvalidEobPoint { eob_pt: 12 };
    assert_eq!(
        classify_coeff_parse_error(&invariant, "5.20.7.27"),
        CoeffParseErrorClass::CoefficientState
    );

    let mut allocation = Vec::<u8>::new();
    let allocation = allocation.try_reserve(usize::MAX).unwrap_err();
    assert_eq!(
        classify_coeff_parse_error(&allocation, "5.20.7.27"),
        CoeffParseErrorClass::Allocation
    );
}

#[test]
fn coefficient_error_classifier_keeps_defensive_eof_syntax_specific() {
    let eof = CoreError::UnexpectedEof {
        offset: ByteOffset::new(23),
        needed: 1,
    };
    assert_eq!(
        classify_coeff_parse_error(&eof, "5.20.6.3"),
        CoeffParseErrorClass::Malformed {
            spec_section: "5.20.6.3"
        }
    );

    let read_quant = read_quant::CoeffReadQuantError::LiteralRead {
        index: 0,
        syntax: "coeff_rem",
        source: eof,
    };
    assert_eq!(
        classify_coeff_parse_error(&read_quant, "5.20.7.27"),
        CoeffParseErrorClass::Malformed {
            spec_section: "5.20.7.28"
        }
    );
}

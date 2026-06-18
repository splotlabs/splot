// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use splot_core::span::ByteOffset;
use splot_core::symbol::{CdfUpdateMode, SymbolDecoder, SymbolDecoderConfig};

use super::super::cdf::{
    EobPtSize, FrameCdfSubset, TileCdfArray, TileCdfError, TileCdfSelector, TileCdfSubset,
};
use super::*;

fn symbol_decoder(payload: &[u8], mode: CdfUpdateMode) -> SymbolDecoder<'_> {
    SymbolDecoder::with_base_and_config(
        payload,
        ByteOffset::new(0),
        SymbolDecoderConfig::new().with_cdf_update_mode(mode),
    )
    .unwrap()
}

fn eob_symbol_input(size: EobPtSize) -> NonZeroCoeffEobSymbolInput {
    NonZeroCoeffEobSymbolInput {
        size,
        coeff_cdf_q_ctx: 0,
        eob_ctx: 0,
    }
}

fn read_eob_with_payload<'a>(
    payload: &'a [u8],
    size: EobPtSize,
    mode: CdfUpdateMode,
) -> Result<(TileCdfSubset, SymbolDecoder<'a>, NonZeroCoeffEobSymbolRead), CoeffLoopContextError> {
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();
    let mut symbols = symbol_decoder(payload, mode);
    let read = read_nonzero_coeff_eob(&mut tile, &mut symbols, eob_symbol_input(size))?;
    Ok((tile, symbols, read))
}

fn find_eob_payload(
    size: EobPtSize,
    predicate: impl Fn(NonZeroCoeffEobSymbolRead) -> bool,
) -> ([u8; 3], NonZeroCoeffEobSymbolRead) {
    let mut found = None;
    'search: for first in u8::MIN..=u8::MAX {
        for second in u8::MIN..=u8::MAX {
            let payload = [first, second, 0x80];
            let Ok((_, _, read)) = read_eob_with_payload(&payload, size, CdfUpdateMode::Enabled)
            else {
                continue;
            };
            if predicate(read) {
                found = Some((payload, read));
                break 'search;
            }
        }
    }
    found.unwrap()
}

fn direct_eob_read(
    tile: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: NonZeroCoeffEobSymbolInput,
) -> Result<NonZeroCoeffEobSymbolRead, CoeffLoopContextError> {
    let eob_pt_symbol = tile
        .read_block_symbol_trace(
            TileCdfSelector::EobPt {
                size: input.size,
                coeff_cdf_q_ctx: input.coeff_cdf_q_ctx,
                eob_ctx: input.eob_ctx,
            },
            symbols,
        )?
        .get();
    let eob_pt_extra_width = eob_pt_extra_width(input.size, eob_pt_symbol);
    let eob_pt_extra = read_eob_literal(symbols, eob_pt_extra_width, "eob_pt_extra")?;
    let eob_pt = resolved_eob_pt(eob_pt_symbol, eob_pt_extra_width, eob_pt_extra);
    let (eob_extra, eob_extra_bits) = if eob_pt >= 3 {
        let eob_extra = tile
            .read_block_symbol_trace(
                TileCdfSelector::EobExtra {
                    coeff_cdf_q_ctx: input.coeff_cdf_q_ctx,
                },
                symbols,
            )?
            .get()
            != 0;
        (
            eob_extra,
            read_eob_literal(symbols, (eob_pt - 3) as u32, "eob_extra_bit")?,
        )
    } else {
        (false, 0)
    };
    let eob = nonzero_coeff_eob(NonZeroCoeffEobInput {
        eob_pt,
        eob_extra,
        eob_extra_bits: eob_extra_bits as usize,
    })?;
    Ok(NonZeroCoeffEobSymbolRead {
        eob,
        eob_pt_symbol,
        eob_pt_extra,
        eob_extra,
        eob_extra_bits,
    })
}

#[test]
fn eob_point_extra_width_tracks_size_specific_extensions() {
    assert_eq!(eob_pt_extra_width(EobPtSize::Pt16, 5), 0);
    assert_eq!(eob_pt_extra_width(EobPtSize::Pt128, 7), 0);
    assert_eq!(eob_pt_extra_width(EobPtSize::Pt256, 7), 1);
    assert_eq!(eob_pt_extra_width(EobPtSize::Pt512, 7), 2);
    assert_eq!(eob_pt_extra_width(EobPtSize::Pt1024, 7), 2);
}

#[test]
fn eob_size_from_transform_log2_maps_all_spec_classes() {
    let cases = [
        (2, 2, EobPtSize::Pt16),
        (3, 2, EobPtSize::Pt32),
        (3, 3, EobPtSize::Pt64),
        (4, 3, EobPtSize::Pt128),
        (4, 4, EobPtSize::Pt256),
        (5, 4, EobPtSize::Pt512),
        (5, 5, EobPtSize::Pt1024),
    ];

    for (tx_width_log2, tx_height_log2, expected) in cases {
        assert_eq!(
            eob_pt_size_from_tx_log2(tx_width_log2, tx_height_log2).unwrap(),
            expected,
            "{tx_width_log2}x{tx_height_log2}"
        );
    }
}

#[test]
fn eob_size_from_transform_log2_clamps_large_dimensions() {
    assert_eq!(eob_pt_size_from_tx_log2(6, 6).unwrap(), EobPtSize::Pt1024);
    assert_eq!(
        eob_pt_size_from_tx_log2(usize::MAX, usize::MAX).unwrap(),
        EobPtSize::Pt1024
    );
}

#[test]
fn eob_size_from_transform_log2_rejects_invalid_dimensions() {
    let bad_width = eob_pt_size_from_tx_log2(1, 2).unwrap_err();
    let bad_height = eob_pt_size_from_tx_log2(2, 1).unwrap_err();

    assert!(matches!(
        bad_width,
        CoeffLoopContextError::InvalidEobTransformLog2 {
            axis: "width",
            value: 1,
            minimum: 2
        }
    ));
    assert!(matches!(
        bad_height,
        CoeffLoopContextError::InvalidEobTransformLog2 {
            axis: "height",
            value: 1,
            minimum: 2
        }
    ));
}

#[test]
fn eob_context_maps_luma_inter_flag_and_chroma_override() {
    assert_eq!(eob_context(0, false), 0);
    assert_eq!(eob_context(0, true), 1);
    assert_eq!(eob_context(1, false), 2);
    assert_eq!(eob_context(2, true), 2);
    assert_eq!(eob_context(usize::MAX, false), 2);
}

#[test]
fn nonzero_coeff_eob_symbol_input_derives_size_context_and_preserves_q_ctx() {
    let luma_inter = nonzero_coeff_eob_symbol_input(NonZeroCoeffEobContextInput {
        plane: 0,
        is_inter: true,
        tx_width_log2: 4,
        tx_height_log2: 3,
        coeff_cdf_q_ctx: 3,
    })
    .unwrap();
    let chroma_intra = nonzero_coeff_eob_symbol_input(NonZeroCoeffEobContextInput {
        plane: 1,
        is_inter: false,
        tx_width_log2: 5,
        tx_height_log2: 4,
        coeff_cdf_q_ctx: 2,
    })
    .unwrap();

    assert_eq!(
        luma_inter,
        NonZeroCoeffEobSymbolInput {
            size: EobPtSize::Pt128,
            coeff_cdf_q_ctx: 3,
            eob_ctx: 1
        }
    );
    assert_eq!(
        chroma_intra,
        NonZeroCoeffEobSymbolInput {
            size: EobPtSize::Pt512,
            coeff_cdf_q_ctx: 2,
            eob_ctx: 2
        }
    );
}

#[test]
fn read_nonzero_coeff_eob_from_context_matches_explicit_selector_read() {
    let (payload, _) = find_eob_payload(EobPtSize::Pt128, |read| read.eob().eob_pt() >= 3);
    let explicit_input = NonZeroCoeffEobSymbolInput {
        size: EobPtSize::Pt128,
        coeff_cdf_q_ctx: 0,
        eob_ctx: 0,
    };
    let context_input = NonZeroCoeffEobContextInput {
        plane: 0,
        is_inter: false,
        tx_width_log2: 4,
        tx_height_log2: 3,
        coeff_cdf_q_ctx: 0,
    };
    let frame = FrameCdfSubset::from_defaults();
    let mut explicit_tile = frame.tile_copy();
    let mut derived_tile = frame.tile_copy();
    let mut explicit_symbols = symbol_decoder(&payload, CdfUpdateMode::Enabled);
    let mut derived_symbols = symbol_decoder(&payload, CdfUpdateMode::Enabled);

    let expected =
        read_nonzero_coeff_eob(&mut explicit_tile, &mut explicit_symbols, explicit_input).unwrap();
    let actual =
        read_nonzero_coeff_eob_from_context(&mut derived_tile, &mut derived_symbols, context_input)
            .unwrap();

    assert_eq!(actual, expected);
    assert_eq!(derived_tile, explicit_tile);
    assert_eq!(
        derived_symbols.consumed_bits(),
        explicit_symbols.consumed_bits()
    );
    assert_eq!(
        derived_symbols.symbol_count(),
        explicit_symbols.symbol_count()
    );
}

#[test]
fn read_nonzero_coeff_eob_from_context_rejects_invalid_log2_before_mutation() {
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();
    let tile_before = tile.clone();
    let mut symbols = symbol_decoder(&[0x00, 0x80], CdfUpdateMode::Enabled);
    let consumed_before = symbols.consumed_bits();
    let symbol_count_before = symbols.symbol_count();

    let err = read_nonzero_coeff_eob_from_context(
        &mut tile,
        &mut symbols,
        NonZeroCoeffEobContextInput {
            plane: 0,
            is_inter: false,
            tx_width_log2: 1,
            tx_height_log2: 2,
            coeff_cdf_q_ctx: 0,
        },
    )
    .unwrap_err();

    assert!(matches!(
        err,
        CoeffLoopContextError::InvalidEobTransformLog2 {
            axis: "width",
            value: 1,
            minimum: 2
        }
    ));
    assert_eq!(tile, tile_before);
    assert_eq!(symbols.consumed_bits(), consumed_before);
    assert_eq!(symbols.symbol_count(), symbol_count_before);
}

#[test]
fn read_nonzero_coeff_eob_from_context_propagates_symbol_reader_errors() {
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();
    let mut symbols = symbol_decoder(&[0x00, 0x80], CdfUpdateMode::Enabled);

    let err = read_nonzero_coeff_eob_from_context(
        &mut tile,
        &mut symbols,
        NonZeroCoeffEobContextInput {
            plane: 0,
            is_inter: false,
            tx_width_log2: 2,
            tx_height_log2: 2,
            coeff_cdf_q_ctx: 4,
        },
    )
    .unwrap_err();

    assert!(matches!(
        err,
        CoeffLoopContextError::EobSymbolRead(BlockSymbolTraceReadError::Cdf(
            TileCdfError::SelectorOutOfRange {
                array: TileCdfArray::EobPt,
                index_name: "coeff_cdf_q_ctx",
                actual: 4,
                max_exclusive: 4,
            }
        ))
    ));
}

#[test]
fn read_nonzero_coeff_eob_matches_direct_symbol_sequence() {
    let (payload, _) = find_eob_payload(EobPtSize::Pt128, |read| read.eob().eob_pt() >= 3);
    let input = eob_symbol_input(EobPtSize::Pt128);
    let frame = FrameCdfSubset::from_defaults();
    let mut direct_tile = frame.tile_copy();
    let mut helper_tile = frame.tile_copy();
    let mut direct_symbols = symbol_decoder(&payload, CdfUpdateMode::Enabled);
    let mut helper_symbols = symbol_decoder(&payload, CdfUpdateMode::Enabled);

    let expected = direct_eob_read(&mut direct_tile, &mut direct_symbols, input).unwrap();
    let actual = read_nonzero_coeff_eob(&mut helper_tile, &mut helper_symbols, input).unwrap();

    assert_eq!(actual, expected);
    assert_eq!(
        helper_symbols.consumed_bits(),
        direct_symbols.consumed_bits()
    );
    assert_eq!(helper_symbols.symbol_count(), direct_symbols.symbol_count());
    assert_eq!(
        helper_tile.row(TileCdfSelector::EobPt {
            size: input.size,
            coeff_cdf_q_ctx: input.coeff_cdf_q_ctx,
            eob_ctx: input.eob_ctx,
        }),
        direct_tile.row(TileCdfSelector::EobPt {
            size: input.size,
            coeff_cdf_q_ctx: input.coeff_cdf_q_ctx,
            eob_ctx: input.eob_ctx,
        })
    );
    assert_eq!(
        helper_tile.row(TileCdfSelector::EobExtra {
            coeff_cdf_q_ctx: input.coeff_cdf_q_ctx,
        }),
        direct_tile.row(TileCdfSelector::EobExtra {
            coeff_cdf_q_ctx: input.coeff_cdf_q_ctx,
        })
    );
}

#[test]
fn read_nonzero_coeff_eob_reads_size_specific_eob_point_extra_literals() {
    let (_, read) = find_eob_payload(EobPtSize::Pt256, |read| read.eob_pt_symbol() == 7);

    assert!(read.eob_pt_extra() <= 1);
    assert_eq!(read.eob().eob_pt(), 8 + read.eob_pt_extra() as usize);
}

#[test]
fn read_nonzero_coeff_eob_short_payload_literal_path_reaches_exit_symbol_error() {
    let mut found = None;

    'search: {
        let payload = Vec::new();
        if short_payload_reaches_exhausted_eob_literal(&payload) {
            found = Some(payload);
            break 'search;
        }

        for byte in u8::MIN..=u8::MAX {
            let payload = vec![byte];
            if short_payload_reaches_exhausted_eob_literal(&payload) {
                found = Some(payload);
                break 'search;
            }
        }

        for bytes in u16::MIN..=u16::MAX {
            let payload = bytes.to_be_bytes().to_vec();
            if short_payload_reaches_exhausted_eob_literal(&payload) {
                found = Some(payload);
                break 'search;
            }
        }
    }

    let payload = found.unwrap();
    let (_, symbols, read) =
        read_eob_with_payload(&payload, EobPtSize::Pt256, CdfUpdateMode::Enabled).unwrap();

    assert!(payload.len() <= 2);
    assert_eq!(read.eob_pt_symbol(), 7);
    assert_eq!(
        eob_pt_extra_width(EobPtSize::Pt256, read.eob_pt_symbol()),
        1
    );
    assert!(symbols.symbol_max_bits() < 0);
    // Short tile payloads fail at the final symbol boundary once the preloaded
    // arithmetic state is exhausted; literal reads do not over-read the slice.
    assert!(symbols.finish().is_err());
}

#[test]
fn read_nonzero_coeff_eob_reads_eob_extra_and_literal_refinements() {
    let (_, read) = find_eob_payload(EobPtSize::Pt128, |read| {
        read.eob().eob_pt() >= 4 && read.eob_extra() && read.eob_extra_bits() != 0
    });

    let direct = nonzero_coeff_eob(NonZeroCoeffEobInput {
        eob_pt: read.eob().eob_pt(),
        eob_extra: read.eob_extra(),
        eob_extra_bits: read.eob_extra_bits() as usize,
    })
    .unwrap();

    assert_eq!(read.eob(), direct);
    assert!(read.eob_extra_bits() < (1 << (read.eob().eob_pt() - 3)));
}

#[test]
fn read_nonzero_coeff_eob_respects_disabled_cdf_update_mode() {
    let (payload, _) = find_eob_payload(EobPtSize::Pt128, |read| read.eob().eob_pt() >= 3);
    let input = eob_symbol_input(EobPtSize::Pt128);
    let eob_pt_selector = TileCdfSelector::EobPt {
        size: input.size,
        coeff_cdf_q_ctx: input.coeff_cdf_q_ctx,
        eob_ctx: input.eob_ctx,
    };
    let eob_extra_selector = TileCdfSelector::EobExtra {
        coeff_cdf_q_ctx: input.coeff_cdf_q_ctx,
    };
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();
    let eob_pt_before = tile.row(eob_pt_selector).unwrap().to_vec();
    let eob_extra_before = tile.row(eob_extra_selector).unwrap().to_vec();
    let mut symbols = symbol_decoder(&payload, CdfUpdateMode::Disabled);

    let read = read_nonzero_coeff_eob(&mut tile, &mut symbols, input).unwrap();

    assert!(read.eob().eob_pt() >= 3);
    assert_eq!(tile.row(eob_pt_selector).unwrap(), eob_pt_before.as_slice());
    assert_eq!(
        tile.row(eob_extra_selector).unwrap(),
        eob_extra_before.as_slice()
    );
    assert!(symbols.symbol_count() >= 2);
}

#[test]
fn read_nonzero_coeff_eob_invalid_selector_fails_before_symbol_read() {
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();
    let valid_selector = TileCdfSelector::EobPt {
        size: EobPtSize::Pt16,
        coeff_cdf_q_ctx: 0,
        eob_ctx: 0,
    };
    let valid_before = tile.row(valid_selector).unwrap().to_vec();
    let mut symbols = symbol_decoder(&[0x00, 0x80], CdfUpdateMode::Enabled);
    let consumed_before = symbols.consumed_bits();
    let symbol_count_before = symbols.symbol_count();

    let err = read_nonzero_coeff_eob(
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
        err,
        CoeffLoopContextError::EobSymbolRead(BlockSymbolTraceReadError::Cdf(
            TileCdfError::SelectorOutOfRange {
                array: TileCdfArray::EobPt,
                index_name: "coeff_cdf_q_ctx",
                actual: 4,
                max_exclusive: 4,
            }
        ))
    ));
    assert_eq!(symbols.consumed_bits(), consumed_before);
    assert_eq!(symbols.symbol_count(), symbol_count_before);
    assert_eq!(tile.row(valid_selector).unwrap(), valid_before.as_slice());
}

#[test]
fn read_eob_literal_wraps_symbol_decoder_errors() {
    let mut symbols = symbol_decoder(&[0x80], CdfUpdateMode::Enabled);

    let err = read_eob_literal(&mut symbols, 33, "eob_extra_bit").unwrap_err();

    assert!(matches!(
        err,
        CoeffLoopContextError::EobLiteralRead {
            syntax: "eob_extra_bit",
            ..
        }
    ));
}

fn short_payload_reaches_exhausted_eob_literal(payload: &[u8]) -> bool {
    let Ok((_, symbols, read)) =
        read_eob_with_payload(payload, EobPtSize::Pt256, CdfUpdateMode::Enabled)
    else {
        return false;
    };

    read.eob_pt_symbol() == 7 && symbols.symbol_max_bits() < 0 && symbols.finish().is_err()
}

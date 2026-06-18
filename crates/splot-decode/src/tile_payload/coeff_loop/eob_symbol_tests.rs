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

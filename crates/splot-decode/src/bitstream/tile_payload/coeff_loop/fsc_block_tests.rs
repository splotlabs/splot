// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use splot_core::headers::sequence::ChromaFormatIdc;
use splot_core::span::ByteOffset;
use splot_core::symbol::{CdfUpdateMode, Symbol, SymbolDecoder, SymbolDecoderConfig};
use splot_core::symbol_encoder::SymbolEncoder;

use super::super::super::cdf::{CoeffCdfSelector, FrameCdfSubset, TileCdfSelector, TileCdfSubset};
use super::super::super::coeff_state::TileCoeffContextState;
use super::super::branch::NonZeroCoeffBlockStart;
use super::super::read_quant::CoeffReadQuantError;
use super::super::{TX_GEOMETRY, read_nonzero_coeff_block_start};
use super::*;

const TX_4X4: usize = 0;

fn symbol_decoder(payload: &[u8]) -> SymbolDecoder<'_> {
    SymbolDecoder::with_base_and_config(
        payload,
        ByteOffset::new(0),
        SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Enabled),
    )
    .unwrap()
}

fn write_symbol(
    tile: &mut TileCdfSubset,
    encoder: &mut SymbolEncoder,
    selector: TileCdfSelector,
    symbol: u8,
) {
    tile.with_row_mut(selector, |row| {
        encoder.write_symbol_u16(row, Symbol::new(symbol))
    })
    .unwrap()
    .unwrap();
}

fn read_start(tile: &mut TileCdfSubset, symbols: &mut SymbolDecoder<'_>) -> NonZeroCoeffBlockStart {
    read_nonzero_coeff_block_start(tile, symbols, &TX_GEOMETRY[TX_4X4], 0, false).unwrap()
}

fn context() -> TileCoeffContextState {
    TileCoeffContextState::new_for_tile_chroma(0..4, 0..4, ChromaFormatIdc::Yuv444).unwrap()
}

/// Decodes one 4x4 FSC block at `(1, 1)` into a zeroed 16-entry `quant`.
fn decode(
    state: &mut TileCoeffContextState,
    tile: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    quant: &mut [i32; 16],
) -> Result<usize, CoeffFscBranchError> {
    let start = read_start(tile, symbols);
    let lines = state.block_lines(0, [1, 1, 1, 1]).unwrap();
    decode_fsc_transform_block(state, tile, symbols, &start, &lines, quant)
}

fn levels_are_zero(state: &mut TileCoeffContextState) -> bool {
    state
        .dc_lines_and_levels(0)
        .unwrap()
        .2
        .iter()
        .all(|&word| word == 0)
}

fn encode_fsc_two_coefficients() -> (Vec<u8>, TileCdfSubset) {
    let mut tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut encoder = SymbolEncoder::new();
    let selectors = [
        (
            TileCdfSelector::EobPt {
                size: super::super::EobPtSize::Pt16,
                eob_ctx: 0,
            },
            1,
        ),
        (
            TileCdfSelector::Coeff(CoeffCdfSelector::BaseBob {
                tx_size_ctx: 0,
                ctx: 2,
            }),
            2,
        ),
        (
            TileCdfSelector::Coeff(CoeffCdfSelector::BrIdtx {
                tx_size_ctx: 0,
                ctx: 0,
            }),
            3,
        ),
        (
            TileCdfSelector::Coeff(CoeffCdfSelector::BaseIdtx {
                tx_size_ctx: 0,
                ctx: 3,
            }),
            3,
        ),
        (
            TileCdfSelector::Coeff(CoeffCdfSelector::BrIdtx {
                tx_size_ctx: 0,
                ctx: 5,
            }),
            3,
        ),
        (
            TileCdfSelector::Coeff(CoeffCdfSelector::IdtxSign {
                tx_size_ctx: 0,
                ctx: 0,
            }),
            1,
        ),
    ];
    for (selector, symbol) in selectors {
        write_symbol(&mut tile, &mut encoder, selector, symbol);
    }
    encoder.write_unary(1, 5).unwrap();
    encoder.write_literal(1, 1).unwrap();
    write_symbol(
        &mut tile,
        &mut encoder,
        TileCdfSelector::Coeff(CoeffCdfSelector::IdtxSign {
            tx_size_ctx: 0,
            ctx: 4,
        }),
        0,
    );
    encoder.write_unary(1, 5).unwrap();
    encoder.write_literal(1, 1).unwrap();
    (encoder.finish().unwrap().into_bytes(), tile)
}

fn encode_start_only() -> Vec<u8> {
    let mut tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut encoder = SymbolEncoder::new();
    write_symbol(
        &mut tile,
        &mut encoder,
        TileCdfSelector::EobPt {
            size: super::super::EobPtSize::Pt16,
            eob_ctx: 0,
        },
        1,
    );
    encoder.finish().unwrap().into_bytes()
}

/// An `eob` of 1 whose level 6 coefficient has an overlong Golomb prefix.
fn encode_overlong_remainder() -> Vec<u8> {
    let mut tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut encoder = SymbolEncoder::new();
    let selectors = [
        (
            TileCdfSelector::EobPt {
                size: super::super::EobPtSize::Pt16,
                eob_ctx: 0,
            },
            0,
        ),
        (
            TileCdfSelector::Coeff(CoeffCdfSelector::BaseBob {
                tx_size_ctx: 0,
                ctx: 2,
            }),
            2,
        ),
        (
            TileCdfSelector::Coeff(CoeffCdfSelector::BrIdtx {
                tx_size_ctx: 0,
                ctx: 0,
            }),
            3,
        ),
        (
            TileCdfSelector::Coeff(CoeffCdfSelector::IdtxSign {
                tx_size_ctx: 0,
                ctx: 0,
            }),
            0,
        ),
    ];
    for (selector, symbol) in selectors {
        write_symbol(&mut tile, &mut encoder, selector, symbol);
    }
    encoder.write_unary(5, 5).unwrap();
    encoder.write_unary(21, 21).unwrap();
    encoder.finish().unwrap().into_bytes()
}

#[test]
fn fsc_block_reads_bob_idtx_br_sign_quant_and_commits() {
    let (payload, expected_tile) = encode_fsc_two_coefficients();
    let mut tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut symbols = symbol_decoder(&payload);
    let mut state = context();
    let mut quant = [0; 16];

    let end = decode(&mut state, &mut tile, &mut symbols, &mut quant).unwrap();

    assert_eq!(end, 16);
    let mut expected = [0; 16];
    (expected[11], expected[15]) = (-9, 9);
    assert_eq!(quant, expected);
    assert_eq!(tile, expected_tile);
    assert_eq!(symbols.symbol_count(), 13);
    let finish = symbols.finish();
    assert!(finish.is_ok(), "{finish:?}");
    assert_eq!(state.above_level(0).unwrap()[1], 4);
    assert_eq!(state.left_level(0).unwrap()[1], 4);
    assert_eq!(state.above_dc(0).unwrap()[1], 0);
    assert!(levels_are_zero(&mut state));
}

#[test]
fn fsc_block_overlong_remainder_fails_and_leaves_the_grid_and_context_clean() {
    let payload = encode_overlong_remainder();
    let mut tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut symbols = symbol_decoder(&payload);
    let mut state = context();
    let before = state.clone();
    let mut quant = [0; 16];

    let result = decode(&mut state, &mut tile, &mut symbols, &mut quant);

    assert!(matches!(
        result,
        Err(CoeffFscBranchError::ReadQuant(
            CoeffReadQuantError::OverlongGolombPrefix { .. }
        ))
    ));
    assert_eq!(state, before);
    assert!(levels_are_zero(&mut state));
}

#[test]
fn fsc_block_at_payload_end_decodes_without_panicking_and_cleans_the_grid() {
    let payload = encode_start_only();
    let mut tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut symbols = symbol_decoder(&payload);
    let mut state = context();
    let mut quant = [0; 16];

    let _ = decode(&mut state, &mut tile, &mut symbols, &mut quant);

    assert!(levels_are_zero(&mut state));
}

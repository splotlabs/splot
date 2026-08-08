// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use splot_core::headers::sequence::ChromaFormatIdc;
use splot_core::span::ByteOffset;
use splot_core::symbol::{CdfUpdateMode, Symbol, SymbolDecoder, SymbolDecoderConfig};
use splot_core::symbol_encoder::SymbolEncoder;

use super::super::super::cdf::{CoeffCdfSelector, FrameCdfSubset, TileCdfSelector, TileCdfSubset};
use super::super::super::coeff_state::TileCoeffContextState;
use super::super::branch::{NonZeroCoeffBlockStart, NonZeroCoeffBlockStartInput};
use super::super::{
    AllZeroCoeffBlockInput, NonZeroCoeffEobContextInput, read_nonzero_coeff_block_start,
};
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

fn read_start(
    tile: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    x4: usize,
    y4: usize,
) -> NonZeroCoeffBlockStart {
    read_nonzero_coeff_block_start(
        tile,
        symbols,
        NonZeroCoeffBlockStartInput {
            block: AllZeroCoeffBlockInput {
                plane: 0,
                x4,
                y4,
                w4: 1,
                h4: 1,
            },
            eob: NonZeroCoeffEobContextInput {
                plane: 0,
                is_inter: false,
                tx_width_log2: 2,
                tx_height_log2: 2,
                coeff_cdf_q_ctx: 0,
            },
        },
    )
    .unwrap()
}

fn staged_input(
    start: NonZeroCoeffBlockStart,
    block: AllZeroCoeffBlockInput,
    tx_size: usize,
    coeff_cdf_q_ctx: usize,
) -> CoeffFscStagedTxSizeNonZeroInput {
    CoeffFscStagedTxSizeNonZeroInput {
        block,
        start,
        tx_size,
        plane_tx_type: 0,
        coeff_cdf_q_ctx,
    }
}

fn luma_block(x4: usize, y4: usize) -> AllZeroCoeffBlockInput {
    AllZeroCoeffBlockInput {
        plane: 0,
        x4,
        y4,
        w4: 1,
        h4: 1,
    }
}

fn encode_fsc_two_coefficients() -> (Vec<u8>, TileCdfSubset) {
    let mut tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut encoder = SymbolEncoder::new();
    let selectors = [
        (
            TileCdfSelector::EobPt {
                size: super::super::EobPtSize::Pt16,
                coeff_cdf_q_ctx: 0,
                eob_ctx: 0,
            },
            1,
        ),
        (
            TileCdfSelector::Coeff(CoeffCdfSelector::BaseBob {
                coeff_cdf_q_ctx: 0,
                tx_size_ctx: 0,
                ctx: 2,
            }),
            2,
        ),
        (
            TileCdfSelector::Coeff(CoeffCdfSelector::BrIdtx {
                coeff_cdf_q_ctx: 0,
                tx_size_ctx: 0,
                ctx: 0,
            }),
            3,
        ),
        (
            TileCdfSelector::Coeff(CoeffCdfSelector::BaseIdtx {
                coeff_cdf_q_ctx: 0,
                tx_size_ctx: 0,
                ctx: 3,
            }),
            3,
        ),
        (
            TileCdfSelector::Coeff(CoeffCdfSelector::BrIdtx {
                coeff_cdf_q_ctx: 0,
                tx_size_ctx: 0,
                ctx: 5,
            }),
            3,
        ),
        (
            TileCdfSelector::Coeff(CoeffCdfSelector::IdtxSign {
                coeff_cdf_q_ctx: 0,
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
            coeff_cdf_q_ctx: 0,
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
            coeff_cdf_q_ctx: 0,
            eob_ctx: 0,
        },
        1,
    );
    encoder.finish().unwrap().into_bytes()
}

#[test]
fn staged_fsc_live_path_reads_bob_idtx_br_sign_quant_and_commits() {
    let (payload, expected_tile) = encode_fsc_two_coefficients();
    let mut tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut symbols = symbol_decoder(&payload);
    let start = read_start(&mut tile, &mut symbols, 1, 1);
    let eob = start.eob_read().eob().eob();
    let mut state =
        TileCoeffContextState::new_for_tile_chroma(0..4, 0..4, ChromaFormatIdc::Yuv444).unwrap();

    let block = apply_staged_nonzero_coeff_fsc_branch_from_tx_size(
        &mut state,
        &mut tile,
        &mut symbols,
        staged_input(start, luma_block(1, 1), TX_4X4, 0),
    )
    .unwrap();

    assert_eq!(eob, 2);
    assert_eq!(block.level_at(2, 3).unwrap(), 6);
    assert_eq!(block.level_at(3, 3).unwrap(), 6);
    for selector in [
        TileCdfSelector::Coeff(CoeffCdfSelector::BaseBob {
            coeff_cdf_q_ctx: 0,
            tx_size_ctx: 0,
            ctx: 2,
        }),
        TileCdfSelector::Coeff(CoeffCdfSelector::BrIdtx {
            coeff_cdf_q_ctx: 0,
            tx_size_ctx: 0,
            ctx: 0,
        }),
        TileCdfSelector::Coeff(CoeffCdfSelector::BaseIdtx {
            coeff_cdf_q_ctx: 0,
            tx_size_ctx: 0,
            ctx: 3,
        }),
        TileCdfSelector::Coeff(CoeffCdfSelector::BrIdtx {
            coeff_cdf_q_ctx: 0,
            tx_size_ctx: 0,
            ctx: 5,
        }),
        TileCdfSelector::Coeff(CoeffCdfSelector::IdtxSign {
            coeff_cdf_q_ctx: 0,
            tx_size_ctx: 0,
            ctx: 0,
        }),
        TileCdfSelector::Coeff(CoeffCdfSelector::IdtxSign {
            coeff_cdf_q_ctx: 0,
            tx_size_ctx: 0,
            ctx: 4,
        }),
    ] {
        assert_eq!(
            tile.row(selector),
            expected_tile.row(selector),
            "{selector:?}"
        );
    }
    assert_eq!(block.quant_at(11).unwrap(), -9);
    assert_eq!(block.quant_at(15).unwrap(), 9);
    assert_eq!(symbols.symbol_count(), 13);
    let finish = symbols.finish();
    assert!(finish.is_ok(), "{finish:?}");
    assert_eq!(block.quant_sign_at(2, 3).unwrap(), -1);
    assert_eq!(block.quant_sign_at(3, 3).unwrap(), 1);
    assert_eq!(state.above_level(0).unwrap()[1], 4);
    assert_eq!(state.left_level(0).unwrap()[1], 4);
    assert_eq!(state.above_dc(0).unwrap()[1], 0);
    assert_eq!(state.left_dc(0).unwrap()[1], 0);
    assert_eq!(tile, expected_tile);
}

#[test]
fn staged_fsc_preflights_selector_geometry_and_plane_without_consumption() {
    let cases = [
        (luma_block(1, 1), usize::MAX),
        (
            AllZeroCoeffBlockInput {
                w4: 2,
                ..luma_block(1, 1)
            },
            TX_4X4,
        ),
        (
            AllZeroCoeffBlockInput {
                plane: 1,
                ..luma_block(1, 1)
            },
            TX_4X4,
        ),
    ];
    for (block, tx_size) in cases {
        let payload = encode_start_only();
        let mut tile = FrameCdfSubset::from_defaults().tile_copy();
        let mut symbols = symbol_decoder(&payload);
        let start = read_start(&mut tile, &mut symbols, 1, 1);
        let tile_before = tile.clone();
        let checkpoint = symbols.checkpoint();
        let mut state =
            TileCoeffContextState::new_for_tile_chroma(0..4, 0..4, ChromaFormatIdc::Yuv444)
                .unwrap();
        let state_before = state.clone();

        let result = apply_staged_nonzero_coeff_fsc_branch_from_tx_size(
            &mut state,
            &mut tile,
            &mut symbols,
            staged_input(start, block, tx_size, 0),
        );

        assert!(result.is_err());
        assert_eq!(state, state_before);
        assert_eq!(tile, tile_before);
        assert_eq!(symbols.checkpoint(), checkpoint);
    }
}

#[test]
fn staged_fsc_invalid_cdf_selector_preserves_context_and_reader_state() {
    let payload = encode_start_only();
    let mut tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut symbols = symbol_decoder(&payload);
    let start = read_start(&mut tile, &mut symbols, 1, 1);
    let tile_before = tile.clone();
    let checkpoint = symbols.checkpoint();
    let mut state =
        TileCoeffContextState::new_for_tile_chroma(0..4, 0..4, ChromaFormatIdc::Yuv444).unwrap();
    let state_before = state.clone();

    let result = apply_staged_nonzero_coeff_fsc_branch_from_tx_size(
        &mut state,
        &mut tile,
        &mut symbols,
        staged_input(start, luma_block(1, 1), TX_4X4, 4),
    );

    assert!(matches!(result, Err(CoeffFscBranchError::Level(_))));
    assert_eq!(state, state_before);
    assert_eq!(tile, tile_before);
    assert_eq!(symbols.checkpoint(), checkpoint);
}

#[test]
fn staged_fsc_context_commit_failure_preserves_context() {
    let (payload, expected_tile) = encode_fsc_two_coefficients();
    let mut tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut symbols = symbol_decoder(&payload);
    let start = read_start(&mut tile, &mut symbols, 2, 2);
    let mut state =
        TileCoeffContextState::new_for_tile_chroma(0..1, 0..1, ChromaFormatIdc::Yuv444).unwrap();
    let state_before = state.clone();

    let result = apply_staged_nonzero_coeff_fsc_branch_from_tx_size(
        &mut state,
        &mut tile,
        &mut symbols,
        staged_input(start, luma_block(2, 2), TX_4X4, 0),
    );

    assert!(matches!(
        result,
        Err(CoeffFscBranchError::Quant(
            CoeffFscQuantPassError::ContextUpdate(_)
        ))
    ));
    assert_eq!(state, state_before);
    assert_eq!(tile, expected_tile);
    assert_eq!(symbols.symbol_count(), 13);
}

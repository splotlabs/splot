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
use super::geometry::{
    CoeffOrdinaryBranchTxSetBaseConfig, CoeffOrdinaryStagedLosslessNonZeroInput,
    CoeffOrdinaryTxSizeGeometryConfig, apply_staged_nonzero_coeff_ordinary_branch_from_lossless,
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

fn read_start(tile: &mut TileCdfSubset, symbols: &mut SymbolDecoder<'_>) -> NonZeroCoeffBlockStart {
    read_nonzero_coeff_block_start(
        tile,
        symbols,
        NonZeroCoeffBlockStartInput {
            block: AllZeroCoeffBlockInput {
                plane: 0,
                x4: 1,
                y4: 1,
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

fn base_config(parity_hiding: bool, use_tcq: bool) -> CoeffOrdinaryBranchTxSetBaseConfig {
    CoeffOrdinaryBranchTxSetBaseConfig {
        reduced_tx_set: 0,
        enable_chroma_dctonly: false,
        uv_mode: 0,
        angle_delta_uv: 0,
        luma_tx_type: 0,
        chroma_inter_tx_type: 0,
        parity_hiding,
        use_tcq,
    }
}

fn staged_input(
    start: NonZeroCoeffBlockStart,
    tx_size: usize,
    coeff_cdf_q_ctx: usize,
    start_x: usize,
    lossless: bool,
    base_config: CoeffOrdinaryBranchTxSetBaseConfig,
) -> CoeffOrdinaryStagedLosslessNonZeroInput {
    CoeffOrdinaryStagedLosslessNonZeroInput {
        geometry: CoeffOrdinaryTxSizeGeometryConfig {
            plane: 0,
            start_x,
            start_y: 4,
            tx_size,
        },
        start,
        coeff_cdf_q_ctx,
        is_inter: false,
        base_config,
        lossless,
    }
}

fn encode_lossless_extended_quant() -> (Vec<u8>, TileCdfSubset) {
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
        0,
    );
    write_symbol(
        &mut tile,
        &mut encoder,
        TileCdfSelector::Coeff(CoeffCdfSelector::BaseLfEob {
            coeff_cdf_q_ctx: 0,
            tx_size: 0,
            ctx: 0,
        }),
        4,
    );
    write_symbol(
        &mut tile,
        &mut encoder,
        TileCdfSelector::Coeff(CoeffCdfSelector::BrLf {
            coeff_cdf_q_ctx: 0,
            ctx: 0,
        }),
        3,
    );
    write_symbol(
        &mut tile,
        &mut encoder,
        TileCdfSelector::DcSign {
            coeff_cdf_q_ctx: 0,
            plane_type: 0,
            group: 0,
            ctx: 0,
        },
        1,
    );
    encoder.write_unary(1, 5).unwrap();
    encoder.write_literal(1, 1).unwrap();
    (encoder.finish().unwrap().into_bytes(), tile)
}

fn encode_evolving_tcq_levels() -> (Vec<u8>, TileCdfSubset) {
    let mut tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut encoder = SymbolEncoder::new();
    let selectors = [
        (
            TileCdfSelector::EobPt {
                size: super::super::EobPtSize::Pt16,
                coeff_cdf_q_ctx: 0,
                eob_ctx: 0,
            },
            2,
        ),
        (TileCdfSelector::EobExtra { coeff_cdf_q_ctx: 0 }, 0),
        (
            TileCdfSelector::Coeff(CoeffCdfSelector::BaseLfEob {
                coeff_cdf_q_ctx: 0,
                tx_size: 0,
                ctx: 1,
            }),
            0,
        ),
        (
            TileCdfSelector::Coeff(CoeffCdfSelector::BaseLf {
                coeff_cdf_q_ctx: 0,
                tx_size: 0,
                ctx: 9,
                tcq_ctx: 0,
            }),
            2,
        ),
        (
            TileCdfSelector::Coeff(CoeffCdfSelector::BaseLf {
                coeff_cdf_q_ctx: 0,
                tx_size: 0,
                ctx: 2,
                tcq_ctx: 1,
            }),
            1,
        ),
    ];
    for (selector, symbol) in selectors {
        write_symbol(&mut tile, &mut encoder, selector, symbol);
    }
    encoder.write_literal(0, 1).unwrap();
    encoder.write_literal(0, 1).unwrap();
    write_symbol(
        &mut tile,
        &mut encoder,
        TileCdfSelector::DcSign {
            coeff_cdf_q_ctx: 0,
            plane_type: 0,
            group: 0,
            ctx: 0,
        },
        0,
    );
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
        0,
    );
    encoder.finish().unwrap().into_bytes()
}

#[test]
fn staged_ordinary_live_path_reads_base_br_sign_quant_and_commits() {
    let (payload, expected_tile) = encode_lossless_extended_quant();
    let mut tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut symbols = symbol_decoder(&payload);
    let start = read_start(&mut tile, &mut symbols);
    let mut state =
        TileCoeffContextState::new_for_tile_chroma(0..4, 0..4, ChromaFormatIdc::Yuv444).unwrap();

    let block = apply_staged_nonzero_coeff_ordinary_branch_from_lossless(
        &mut state,
        &mut tile,
        &mut symbols,
        staged_input(start, TX_4X4, 0, 4, true, base_config(true, true)),
    )
    .unwrap();

    assert_eq!(block.level_at(0, 0).unwrap(), 8);
    assert_eq!(block.quant_at(0).unwrap(), -11);
    assert_eq!(state.above_level(0).unwrap()[1], 4);
    assert_eq!(state.left_level(0).unwrap()[1], 4);
    assert_eq!(state.above_dc(0).unwrap()[1], 1);
    assert_eq!(state.left_dc(0).unwrap()[1], 1);
    assert_eq!(symbols.symbol_count(), 7);
    assert_eq!(tile, expected_tile);
    assert!(symbols.finish().is_ok());
}

#[test]
fn staged_ordinary_live_path_applies_evolving_tcq_state() {
    let mut default_tile = FrameCdfSubset::from_defaults().tile_copy();
    let (payload, mut expected_tile) = encode_evolving_tcq_levels();
    let mut tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut symbols = symbol_decoder(&payload);
    let start = read_start(&mut tile, &mut symbols);
    let mut state =
        TileCoeffContextState::new_for_tile_chroma(0..4, 0..4, ChromaFormatIdc::Yuv444).unwrap();

    let block = apply_staged_nonzero_coeff_ordinary_branch_from_lossless(
        &mut state,
        &mut tile,
        &mut symbols,
        staged_input(start, TX_4X4, 0, 4, false, base_config(false, true)),
    )
    .unwrap();

    assert_eq!(block.level_at(0, 1).unwrap(), 1);
    assert_eq!(block.level_at(1, 0).unwrap(), 2);
    assert_eq!(block.level_at(0, 0).unwrap(), 1);
    assert_eq!(block.quant_at(1).unwrap(), 2);
    assert_eq!(block.quant_at(4).unwrap(), 4);
    assert_eq!(block.quant_at(0).unwrap(), 1);
    assert_eq!(state.above_level(0).unwrap()[1], 4);
    assert_eq!(state.left_level(0).unwrap()[1], 4);
    assert_eq!(state.above_dc(0).unwrap()[1], 2);
    assert_eq!(state.left_dc(0).unwrap()[1], 2);
    let evolved_tcq_selector = TileCdfSelector::Coeff(CoeffCdfSelector::BaseLf {
        coeff_cdf_q_ctx: 0,
        tx_size: 0,
        ctx: 2,
        tcq_ctx: 1,
    });
    let initial_tcq_selector = TileCdfSelector::Coeff(CoeffCdfSelector::BaseLf {
        coeff_cdf_q_ctx: 0,
        tx_size: 0,
        ctx: 2,
        tcq_ctx: 0,
    });
    assert_ne!(
        tile.with_row_mut(evolved_tcq_selector, |row| row.to_vec()),
        default_tile.with_row_mut(evolved_tcq_selector, |row| row.to_vec())
    );
    assert_eq!(
        tile.with_row_mut(evolved_tcq_selector, |row| row.to_vec()),
        expected_tile.with_row_mut(evolved_tcq_selector, |row| row.to_vec())
    );
    assert_eq!(
        tile.with_row_mut(initial_tcq_selector, |row| row.to_vec()),
        default_tile.with_row_mut(initial_tcq_selector, |row| row.to_vec())
    );
    assert_eq!(symbols.symbol_count(), 8);
    assert_eq!(tile, expected_tile);
    assert!(symbols.finish().is_ok());
}

#[test]
fn staged_ordinary_preflights_geometry_and_bounds_without_consumption() {
    for (tx_size, start_x) in [(usize::MAX, 4), (1, 4)] {
        let payload = encode_start_only();
        let mut tile = FrameCdfSubset::from_defaults().tile_copy();
        let mut symbols = symbol_decoder(&payload);
        let start = read_start(&mut tile, &mut symbols);
        let tile_before = tile.clone();
        let checkpoint = symbols.checkpoint();
        let mut state =
            TileCoeffContextState::new_for_tile_chroma(0..4, 0..4, ChromaFormatIdc::Yuv444)
                .unwrap();
        let state_before = state.clone();

        let result = apply_staged_nonzero_coeff_ordinary_branch_from_lossless(
            &mut state,
            &mut tile,
            &mut symbols,
            staged_input(start, tx_size, 0, start_x, true, base_config(false, false)),
        );

        assert!(result.is_err());
        assert_eq!(state, state_before);
        assert_eq!(tile, tile_before);
        assert_eq!(symbols.checkpoint(), checkpoint);
    }
}

#[test]
fn staged_ordinary_selector_error_preserves_context_and_reader_state() {
    let payload = encode_start_only();
    let mut tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut symbols = symbol_decoder(&payload);
    let start = read_start(&mut tile, &mut symbols);
    let tile_before = tile.clone();
    let checkpoint = symbols.checkpoint();
    let mut state =
        TileCoeffContextState::new_for_tile_chroma(0..4, 0..4, ChromaFormatIdc::Yuv444).unwrap();
    let state_before = state.clone();

    let result = apply_staged_nonzero_coeff_ordinary_branch_from_lossless(
        &mut state,
        &mut tile,
        &mut symbols,
        staged_input(start, TX_4X4, 4, 4, true, base_config(false, false)),
    );

    assert!(matches!(result, Err(CoeffOrdinaryBranchError::Ordinary(_))));
    assert_eq!(state, state_before);
    assert_eq!(tile, tile_before);
    assert_eq!(symbols.checkpoint(), checkpoint);
}

#[test]
fn staged_ordinary_context_commit_failure_preserves_context_after_reads() {
    let (payload, expected_tile) = encode_lossless_extended_quant();
    let mut tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut symbols = symbol_decoder(&payload);
    let start = read_start(&mut tile, &mut symbols);
    let mut state =
        TileCoeffContextState::new_for_tile_chroma(0..4, 0..4, ChromaFormatIdc::Yuv444).unwrap();
    let state_before = state.clone();

    let result = apply_staged_nonzero_coeff_ordinary_branch_from_lossless(
        &mut state,
        &mut tile,
        &mut symbols,
        staged_input(
            start,
            TX_4X4,
            0,
            usize::MAX,
            true,
            base_config(false, false),
        ),
    );

    assert!(matches!(
        result,
        Err(CoeffOrdinaryBranchError::Ordinary(
            CoeffOrdinaryPassError::ContextUpdate(_)
        ))
    ));
    assert_eq!(state, state_before);
    assert_eq!(tile, expected_tile);
    assert_eq!(symbols.symbol_count(), 7);
    assert!(symbols.finish().is_ok());
}

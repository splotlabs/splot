// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use splot_core::headers::sequence::ChromaFormatIdc;
use splot_core::span::ByteOffset;
use splot_core::symbol::{CdfUpdateMode, Symbol, SymbolDecoder, SymbolDecoderConfig};
use splot_core::symbol_encoder::SymbolEncoder;

use super::super::super::cdf::{CoeffCdfSelector, FrameCdfSubset, TileCdfSelector, TileCdfSubset};
use super::super::super::coeff_state::{CoeffBlockLines, TileCoeffContextState};
use super::super::branch::NonZeroCoeffBlockStart;
use super::super::{TX_GEOMETRY, read_nonzero_coeff_block_start};
use super::geometry::{
    CoeffOrdinaryBranchTxSetBaseConfig, CoeffOrdinaryStagedLosslessNonZeroInput,
    CoeffOrdinaryTxSizeGeometryConfig, apply_staged_nonzero_coeff_ordinary_branch_from_lossless,
};

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

fn levels_clear(state: &mut TileCoeffContextState) -> bool {
    let (_, _, levels) = state.dc_lines_and_levels(0).unwrap();
    levels.iter().all(|&word| word == 0)
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

fn staged_input<'l>(
    lines: &'l CoeffBlockLines,
    start: NonZeroCoeffBlockStart,
    tx_size: usize,
    start_x: usize,
    lossless: bool,
    base_config: CoeffOrdinaryBranchTxSetBaseConfig,
) -> CoeffOrdinaryStagedLosslessNonZeroInput<'l> {
    CoeffOrdinaryStagedLosslessNonZeroInput {
        geometry: CoeffOrdinaryTxSizeGeometryConfig {
            plane: 0,
            start_x,
            start_y: 4,
            tx_size,
        },
        start,
        lines,
        plane_tx_type: base_config.luma_tx_type,
        parity_hiding: base_config.parity_hiding,
        use_tcq: base_config.use_tcq,
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
            eob_ctx: 0,
        },
        0,
    );
    write_symbol(
        &mut tile,
        &mut encoder,
        TileCdfSelector::Coeff(CoeffCdfSelector::BaseLfEob { tx_size: 0, ctx: 0 }),
        4,
    );
    write_symbol(
        &mut tile,
        &mut encoder,
        TileCdfSelector::Coeff(CoeffCdfSelector::BrLf { ctx: 0 }),
        3,
    );
    write_symbol(
        &mut tile,
        &mut encoder,
        TileCdfSelector::DcSign {
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
                eob_ctx: 0,
            },
            2,
        ),
        (TileCdfSelector::EobExtra, 0),
        (
            TileCdfSelector::Coeff(CoeffCdfSelector::BaseLfEob { tx_size: 0, ctx: 1 }),
            0,
        ),
        (
            TileCdfSelector::Coeff(CoeffCdfSelector::BaseLf {
                tx_size: 0,
                ctx: 9,
                tcq_ctx: 0,
            }),
            2,
        ),
        (
            TileCdfSelector::Coeff(CoeffCdfSelector::BaseLf {
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

    let mut quant = [0; 16];
    let lines = state.block_lines(0, [1, 1, 1, 1]).unwrap();
    let end = apply_staged_nonzero_coeff_ordinary_branch_from_lossless(
        &mut state,
        &mut tile,
        &mut symbols,
        staged_input(&lines, start, TX_4X4, 4, true, base_config(true, true)),
        &mut quant,
    )
    .unwrap();

    assert_eq!((end, quant[0]), (1, -11));
    assert!(levels_clear(&mut state));
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

    let mut quant = [0; 16];
    let lines = state.block_lines(0, [1, 1, 1, 1]).unwrap();
    let end = apply_staged_nonzero_coeff_ordinary_branch_from_lossless(
        &mut state,
        &mut tile,
        &mut symbols,
        staged_input(&lines, start, TX_4X4, 4, false, base_config(false, true)),
        &mut quant,
    )
    .unwrap();

    assert_eq!(end, 5);
    assert_eq!(quant[..5], [1, 2, 0, 0, 4]);
    assert!(levels_clear(&mut state));
    assert_eq!(state.above_level(0).unwrap()[1], 4);
    assert_eq!(state.left_level(0).unwrap()[1], 4);
    assert_eq!(state.above_dc(0).unwrap()[1], 2);
    assert_eq!(state.left_dc(0).unwrap()[1], 2);
    let evolved_tcq_selector = TileCdfSelector::Coeff(CoeffCdfSelector::BaseLf {
        tx_size: 0,
        ctx: 2,
        tcq_ctx: 1,
    });
    let initial_tcq_selector = TileCdfSelector::Coeff(CoeffCdfSelector::BaseLf {
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
fn staged_ordinary_preflights_plane_and_bounds_without_consumption() {
    for (plane, quant_len) in [(1, 16), (0, 17)] {
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
        let lines = state.block_lines(plane, [1, 1, 1, 1]).unwrap();

        let result = apply_staged_nonzero_coeff_ordinary_branch_from_lossless(
            &mut state,
            &mut tile,
            &mut symbols,
            staged_input(&lines, start, TX_4X4, 4, true, base_config(false, false)),
            &mut vec![0; quant_len],
        );

        assert!(result.is_err());
        assert_eq!(state, state_before);
        assert_eq!(tile, tile_before);
        assert_eq!(symbols.checkpoint(), checkpoint);
    }
}

#[test]
fn block_lines_reject_a_block_outside_the_tile() {
    let state =
        TileCoeffContextState::new_for_tile_chroma(0..4, 0..4, ChromaFormatIdc::Yuv444).unwrap();
    assert!(state.block_lines(0, [usize::MAX >> 2, 1, 1, 1]).is_err());
    assert!(state.block_lines(0, [4, 1, 1, 1]).is_err());
    assert!(state.block_lines(0, [3, 1, 0, 1]).is_err());
    assert!(state.block_lines(3, [1, 1, 1, 1]).is_err());
    assert_eq!(state.block_lines(0, [3, 1, 4, 1]).unwrap().x4, 3);
}

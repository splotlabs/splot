// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use splot_core::symbol::{CdfUpdateMode, SymbolDecoderConfig};

use super::*;
use crate::bitstream::tile_payload::{FrameCdfSubset, encode_symbol_sequence};

#[test]
fn inactive_state_has_no_grid() {
    let state = GdfState::inactive();

    assert!(!state.active);
    assert!(state.into_grid(ByteOffset::new(0)).unwrap().is_none());
}

#[test]
fn use_gdf_symbols_round_trip() {
    let payload =
        encode_symbol_sequence(&[(TileCdfSelector::UseGdf, 1), (TileCdfSelector::UseGdf, 0)]);
    let mut cdfs = FrameCdfSubset::from_defaults().tile_copy();
    let mut symbols = SymbolDecoder::with_base_and_config(
        &payload,
        ByteOffset::new(0),
        SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Disabled),
    )
    .unwrap();

    assert_eq!(
        read_use_gdf(&mut cdfs, &mut symbols, ByteOffset::new(0)).unwrap(),
        1
    );
    assert_eq!(
        read_use_gdf(&mut cdfs, &mut symbols, ByteOffset::new(0)).unwrap(),
        0
    );
    assert_eq!(symbols.symbol_count(), 2);
}

#[test]
fn unvisited_gdf_unit_fails_closed() {
    let state = GdfState {
        active: true,
        block_size: 64,
        sb_size4: 16,
        sb_per_gdf: 1,
        grid_rows: 1,
        grid_cols: 1,
        values: vec![2],
    };

    assert!(state.into_grid(ByteOffset::new(0)).is_err());
}

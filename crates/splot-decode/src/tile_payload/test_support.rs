// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Shared tile-payload test helpers.

#![allow(clippy::unwrap_used)]

use splot_core::symbol::{CdfUpdateMode, Symbol};
use splot_core::symbol_encoder::{SymbolEncoder, SymbolEncoderConfig};

use super::cdf::{FrameCdfSubset, TileCdfSelector};

pub(crate) fn encode_symbol_sequence(sequence: &[(TileCdfSelector, u8)]) -> Vec<u8> {
    let mut tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut encoder = SymbolEncoder::with_config(
        SymbolEncoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Disabled),
    );
    for &(selector, value) in sequence {
        tile.with_row_mut(selector, |row| {
            encoder.write_symbol(row, Symbol::new(value))
        })
        .unwrap()
        .unwrap();
    }
    encoder.finish().unwrap().into_bytes()
}

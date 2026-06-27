// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Shared coefficient-loop test helpers.
//!
//! These fixtures and setup helpers are reused byte-for-byte across several
//! `coeff_loop` `*_tests` submodules; a single copy here avoids drift.

#![allow(clippy::unwrap_used, clippy::panic)]

use splot_core::span::ByteOffset;
use splot_core::symbol::{CdfUpdateMode, SymbolDecoder, SymbolDecoderConfig};

use super::super::cdf::{FrameCdfSubset, TileCdfSubset};
use super::super::coeff_state::{CoeffContextUpdate, TileCoeffContextState};
use super::branch::{CoeffBlockEobBranch, NonZeroCoeffBlockStart, NonZeroCoeffBlockStartInput};
use super::{CoeffBlockEobBranchInput, read_coeff_block_eob_branch};

/// Builds a CDF-update-enabled symbol decoder over `payload` at offset 0.
pub(crate) fn symbol_decoder(payload: &[u8]) -> SymbolDecoder<'_> {
    SymbolDecoder::with_base_and_config(
        payload,
        ByteOffset::new(0),
        SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Enabled),
    )
    .unwrap()
}

/// A 32x32 coeff context state seeded with one prior 6x6 luma block update.
pub(crate) fn seeded_context_state() -> TileCoeffContextState {
    let mut state = TileCoeffContextState::new(32, 32).unwrap();
    state
        .update_after_coeffs(CoeffContextUpdate {
            plane: 0,
            x4: 0,
            y4: 0,
            w4: 6,
            h4: 6,
            cul_level: 1,
            dc_category: 1,
        })
        .unwrap();
    state
}

/// Narrows an EOB branch to its non-zero start, or `None` for the all-zero arm.
fn branch_nonzero(branch: CoeffBlockEobBranch) -> Option<NonZeroCoeffBlockStart> {
    match branch {
        CoeffBlockEobBranch::AllZero(_) => None,
        CoeffBlockEobBranch::NonZero(start) => Some(start),
    }
}

/// Reads the non-zero EOB branch for `start` over a default tile and returns the
/// tile, the live decoder, and the non-zero block start (or `None` if any step
/// fails or the branch is all-zero).
pub(crate) fn setup_start_with_input(
    payload: &[u8],
    start: NonZeroCoeffBlockStartInput,
) -> Option<(TileCdfSubset, SymbolDecoder<'_>, NonZeroCoeffBlockStart)> {
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();
    let mut symbols = symbol_decoder(payload);
    let mut state = TileCoeffContextState::new(4, 4).ok()?;
    let branch = read_coeff_block_eob_branch(
        &mut state,
        &mut tile,
        &mut symbols,
        CoeffBlockEobBranchInput::NonZero(start),
    )
    .ok()?;
    Some((tile, symbols, branch_nonzero(branch)?))
}

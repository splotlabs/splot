// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Shared coefficient-loop test helpers.

#![allow(clippy::unwrap_used, clippy::panic)]

use splot_core::span::ByteOffset;
use splot_core::symbol::{CdfUpdateMode, SymbolBitPosition, SymbolDecoder, SymbolDecoderConfig};

use super::super::cdf::{FrameCdfSubset, TileCdfSubset};
use super::super::coeff_state::{CoeffContextUpdate, TileCoeffContextState};
use super::branch::{NonZeroCoeffBlockStart, NonZeroCoeffBlockStartInput};
use super::ordinary_pass::CoeffOrdinaryBranch;
use super::read_nonzero_coeff_block_start;
use super::scan_walk::{NonZeroCoeffScanWalk, walk_nonzero_coeff_scan};
use super::{AllZeroCoeffBlockInput, NonZeroCoeffEobContextInput};

pub(crate) type BranchRun<T> = (
    T,
    TileCoeffContextState,
    TileCdfSubset,
    SymbolBitPosition,
    u64,
);
pub(crate) type OrdinaryBranchRun = BranchRun<CoeffOrdinaryBranch>;

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
    seeded_luma_context_state(32, 32, 6, 6, 1, 1)
}

pub(crate) fn seeded_6x6_context_state() -> TileCoeffContextState {
    seeded_luma_context_state(6, 6, 6, 6, 1, 1)
}

pub(crate) fn seeded_luma_context_state(
    columns: usize,
    rows: usize,
    w4: usize,
    h4: usize,
    cul_level: u8,
    dc_category: u8,
) -> TileCoeffContextState {
    let mut state = TileCoeffContextState::new(columns, rows).unwrap();
    state
        .update_after_coeffs(CoeffContextUpdate {
            plane: 0,
            x4: 0,
            y4: 0,
            w4,
            h4,
            cul_level,
            dc_category,
        })
        .unwrap();
    state
}

pub(crate) fn run_ordinary_branch<'a>(
    payload: &'a [u8],
    apply: impl FnOnce(
        &mut TileCoeffContextState,
        &mut TileCdfSubset,
        &mut SymbolDecoder<'a>,
    ) -> CoeffOrdinaryBranch,
) -> OrdinaryBranchRun {
    run_optional_branch(payload, |context_state, tile, symbols| {
        Some(apply(context_state, tile, symbols))
    })
    .unwrap()
}

pub(crate) fn run_optional_branch<'a, T>(
    payload: &'a [u8],
    apply: impl FnOnce(
        &mut TileCoeffContextState,
        &mut TileCdfSubset,
        &mut SymbolDecoder<'a>,
    ) -> Option<T>,
) -> Option<BranchRun<T>> {
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();
    let mut symbols = symbol_decoder(payload);
    let mut context_state = seeded_context_state();
    let branch = apply(&mut context_state, &mut tile, &mut symbols)?;
    Some((
        branch,
        context_state,
        tile,
        symbols.consumed_bits(),
        symbols.symbol_count(),
    ))
}

/// Reads the non-zero EOB start over a default tile and returns the tile, the
/// live decoder, and the non-zero block start.
pub(crate) fn setup_start_with_input(
    payload: &[u8],
    start: NonZeroCoeffBlockStartInput,
) -> Option<(TileCdfSubset, SymbolDecoder<'_>, NonZeroCoeffBlockStart)> {
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();
    let mut symbols = symbol_decoder(payload);
    let start = read_nonzero_coeff_block_start(&mut tile, &mut symbols, start).ok()?;
    Some((tile, symbols, start))
}

pub(crate) fn setup_luma_8x8_walk<'scan>(
    payload: &[u8],
    scan: &'scan [u16],
) -> Option<NonZeroCoeffScanWalk<'scan>> {
    let (_, _, start) = setup_start_with_input(
        payload,
        NonZeroCoeffBlockStartInput {
            block: AllZeroCoeffBlockInput {
                plane: 0,
                x4: 0,
                y4: 0,
                w4: 2,
                h4: 2,
            },
            eob: NonZeroCoeffEobContextInput {
                plane: 0,
                is_inter: false,
                tx_width_log2: 3,
                tx_height_log2: 3,
                coeff_cdf_q_ctx: 0,
            },
        },
    )?;
    if start.eob_read().eob().eob() != scan.len() {
        return None;
    }
    walk_nonzero_coeff_scan(&start, scan).ok()
}

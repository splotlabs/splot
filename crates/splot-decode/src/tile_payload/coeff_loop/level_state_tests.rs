// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::panic)]

use splot_core::span::ByteOffset;
use splot_core::symbol::{CdfUpdateMode, SymbolDecoder, SymbolDecoderConfig};

use super::super::cdf::{CoeffCdfSelector, FrameCdfSubset, TileCdfSubset};
use super::super::coeff_state::{TileCoeffContextState, TileCoeffStateError};
use super::base_symbol::{
    CoeffBaseRangeRead, CoeffBaseSymbolRead, CoeffBaseSymbolReadInput, CoeffBaseSymbolSource,
    read_nonzero_coeff_base_symbols,
};
use super::branch::{CoeffBlockEobBranch, NonZeroCoeffBlockStart, NonZeroCoeffBlockStartInput};
use super::level_state::{CoeffLevelStateWriteError, apply_nonzero_coeff_base_levels};
use super::scan_walk::{NonZeroCoeffScanWalk, walk_nonzero_coeff_scan};
use super::*;

const BASE_LEVELS: u32 = 2;
const PAYLOAD_SUFFIXES: [[u8; 3]; 4] = [
    [0x00, 0x00, 0x80],
    [0xff, 0x00, 0x80],
    [0x55, 0xaa, 0x80],
    [0xff, 0xff, 0x80],
];
const SCAN: [u16; 4] = [0, 8, 1, 9];
const ALT_SCAN: [u16; 4] = [0, 8, 9, 1];
const LARGE_SCAN: [u16; 4] = [0, 1, 8, 63];

fn symbol_decoder(payload: &[u8]) -> SymbolDecoder<'_> {
    SymbolDecoder::with_base_and_config(
        payload,
        ByteOffset::new(0),
        SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Enabled),
    )
    .unwrap()
}

fn branch_nonzero(branch: CoeffBlockEobBranch) -> Option<NonZeroCoeffBlockStart> {
    match branch {
        CoeffBlockEobBranch::AllZero(_) => None,
        CoeffBlockEobBranch::NonZero(start) => Some(start),
    }
}

fn setup_start<'a>(
    payload: &'a [u8],
    w4: usize,
    h4: usize,
) -> Option<(TileCdfSubset, SymbolDecoder<'a>, NonZeroCoeffBlockStart)> {
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();
    let mut symbols = symbol_decoder(payload);
    let mut state = TileCoeffContextState::new(4, 4).ok()?;
    let branch = read_coeff_block_eob_branch(
        &mut state,
        &mut tile,
        &mut symbols,
        CoeffBlockEobBranchInput::NonZero(NonZeroCoeffBlockStartInput {
            block: AllZeroCoeffBlockInput {
                plane: 0,
                x4: 0,
                y4: 0,
                w4,
                h4,
            },
            eob: NonZeroCoeffEobContextInput {
                plane: 0,
                is_inter: false,
                tx_width_log2: 3,
                tx_height_log2: 3,
                coeff_cdf_q_ctx: 0,
            },
        }),
    )
    .ok()?;
    let start = branch_nonzero(branch)?;
    Some((tile, symbols, start))
}

fn base_eob_selector() -> CoeffCdfSelector {
    CoeffCdfSelector::BaseEob {
        coeff_cdf_q_ctx: 0,
        tx_size: 0,
        ctx: 0,
    }
}

fn base_selector() -> CoeffCdfSelector {
    CoeffCdfSelector::Base {
        coeff_cdf_q_ctx: 0,
        tx_size: 0,
        ctx: 0,
        tcq_ctx: 0,
    }
}

fn br_selector() -> CoeffCdfSelector {
    CoeffCdfSelector::Br {
        coeff_cdf_q_ctx: 0,
        ctx: 0,
    }
}

fn inputs_for(walk: &NonZeroCoeffScanWalk) -> Vec<CoeffBaseSymbolReadInput> {
    walk.entries()
        .iter()
        .copied()
        .enumerate()
        .map(|(index, entry)| CoeffBaseSymbolReadInput {
            entry,
            base: if index == 0 {
                CoeffBaseSymbolSource::BaseEob {
                    selector: base_eob_selector(),
                }
            } else {
                CoeffBaseSymbolSource::Base {
                    selector: base_selector(),
                }
            },
            base_levels: BASE_LEVELS,
            base_range: CoeffBaseRangeRead::Enabled {
                selector: br_selector(),
            },
        })
        .collect()
}

fn setup_reads(
    payload: &[u8],
    scan: &[u16],
    w4: usize,
    h4: usize,
) -> Option<(
    NonZeroCoeffBlockStart,
    NonZeroCoeffScanWalk,
    Vec<CoeffBaseSymbolRead>,
)> {
    let (mut tile, mut symbols, start) = setup_start(payload, w4, h4)?;
    if start.eob_read().eob().eob() != scan.len() {
        return None;
    }
    let walk = walk_nonzero_coeff_scan(&start, scan).ok()?;
    let inputs = inputs_for(&walk);
    let reads = read_nonzero_coeff_base_symbols(&mut tile, &mut symbols, &walk, &inputs).ok()?;
    Some((start, walk, reads))
}

fn read_payload(payload: &[u8]) -> Option<Vec<CoeffBaseSymbolRead>> {
    let (_, _, reads) = setup_reads(payload, &SCAN, 2, 2)?;
    Some(reads)
}

fn find_payload(predicate: impl Fn(&[CoeffBaseSymbolRead]) -> bool) -> [u8; 5] {
    for first in u8::MIN..=u8::MAX {
        for second in u8::MIN..=u8::MAX {
            for suffix in PAYLOAD_SUFFIXES {
                let payload = [first, second, suffix[0], suffix[1], suffix[2]];
                let Some(reads) = read_payload(&payload) else {
                    continue;
                };
                if predicate(&reads) {
                    return payload;
                }
            }
        }
    }
    panic!("no coefficient level state payload found");
}

#[test]
fn coefficient_level_state_write_sets_levels_from_scan_entries() {
    let payload = find_payload(|reads| reads.iter().any(|read| read.base_range_symbol().is_some()));
    let (start, walk, reads) = setup_reads(&payload, &SCAN, 2, 2).unwrap();
    let eob_read = start.eob_read();

    let state = apply_nonzero_coeff_base_levels(start, &walk, &reads).unwrap();

    assert_eq!(state.eob_read(), eob_read);
    let block = state.block();
    let mut expected = vec![0; block.level().len()];
    for read in &reads {
        let entry = read.entry();
        expected[entry.row() * block.width() + entry.col()] = read.level();
        assert_eq!(
            block.level_at(entry.row(), entry.col()).unwrap(),
            read.level()
        );
    }
    assert_eq!(block.level(), expected.as_slice());
    assert!(block.quant_sign().iter().all(|sign| *sign == 0));
    assert!(block.quant().iter().all(|quant| *quant == 0));
}

#[test]
fn coefficient_level_state_write_rejects_read_count_mismatch() {
    let payload = find_payload(|reads| !reads.is_empty());
    let (start, walk, mut reads) = setup_reads(&payload, &SCAN, 2, 2).unwrap();
    let before = start.clone();
    reads.pop();

    let err = apply_nonzero_coeff_base_levels(start.clone(), &walk, &reads).unwrap_err();

    assert!(matches!(
        err,
        CoeffLevelStateWriteError::ReadCountMismatch {
            reads: 3,
            entries: 4
        }
    ));
    assert_eq!(start, before);
    assert!(start.block().level().iter().all(|level| *level == 0));
}

#[test]
fn coefficient_level_state_write_rejects_scan_entry_mismatch() {
    let payload = find_payload(|reads| !reads.is_empty());
    let (start, _walk, reads) = setup_reads(&payload, &SCAN, 2, 2).unwrap();
    let alt_walk = walk_nonzero_coeff_scan(&start, &ALT_SCAN).unwrap();

    let err = apply_nonzero_coeff_base_levels(start, &alt_walk, &reads).unwrap_err();

    assert!(matches!(
        err,
        CoeffLevelStateWriteError::ScanEntryMismatch { index: 0, .. }
    ));
}

#[test]
fn coefficient_level_state_write_rejects_mismatched_block_geometry() {
    let payload = find_payload(|reads| !reads.is_empty());
    let (_large_start, large_walk, reads) = setup_reads(&payload, &LARGE_SCAN, 2, 2).unwrap();
    let (_tile, _symbols, small_start) = setup_start(&payload, 1, 1).unwrap();

    let err = apply_nonzero_coeff_base_levels(small_start, &large_walk, &reads).unwrap_err();

    assert!(matches!(
        err,
        CoeffLevelStateWriteError::State(TileCoeffStateError::TransformCoordinateOutOfBounds {
            row: 7,
            col: 7,
            height: 4,
            width: 4
        })
    ));
}

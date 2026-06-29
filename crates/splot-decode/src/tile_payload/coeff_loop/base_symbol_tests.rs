// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::panic)]

use splot_core::span::ByteOffset;
use splot_core::symbol::{CdfUpdateMode, Symbol, SymbolDecoder, SymbolDecoderConfig};
use splot_core::symbol_encoder::SymbolEncoder;

use super::super::cdf::block_read::BlockSymbolTraceReadError;
use super::super::cdf::{
    CoeffCdfSelector, FrameCdfSubset, TileCdfArray, TileCdfError, TileCdfSelector, TileCdfSubset,
};
use super::super::coeff_state::TileCoeffContextState;
use super::base_symbol::{
    CoeffBaseRangeRead, CoeffBaseSymbolRead, CoeffBaseSymbolReadError, CoeffBaseSymbolReadInput,
    CoeffBaseSymbolSource, read_nonzero_coeff_base_symbols,
};
use super::branch::{CoeffBlockEobBranch, NonZeroCoeffBlockStartInput};
use super::scan_walk::{CoeffScanEntry, NonZeroCoeffScanWalk, walk_nonzero_coeff_scan};
use super::*;

const BASE_LEVELS: u32 = 2;
const PAYLOAD_SUFFIXES: [[u8; 3]; 4] = [
    [0x00, 0x00, 0x80],
    [0xff, 0x00, 0x80],
    [0x55, 0xaa, 0x80],
    [0xff, 0xff, 0x80],
];
const SCAN: [u16; 4] = [0, 8, 1, 9];

type BaseReadTuple = (CoeffScanEntry, u8, Option<u8>, u32);

fn symbol_decoder(payload: &[u8], mode: CdfUpdateMode) -> SymbolDecoder<'_> {
    SymbolDecoder::with_base_and_config(
        payload,
        ByteOffset::new(0),
        SymbolDecoderConfig::new().with_cdf_update_mode(mode),
    )
    .unwrap()
}

fn branch_nonzero(branch: CoeffBlockEobBranch) -> Option<super::branch::NonZeroCoeffBlockStart> {
    match branch {
        CoeffBlockEobBranch::AllZero(_) => None,
        CoeffBlockEobBranch::NonZero(start) => Some(start),
    }
}

fn setup_walk(
    payload: &[u8],
    mode: CdfUpdateMode,
) -> Option<(TileCdfSubset, SymbolDecoder<'_>, NonZeroCoeffScanWalk)> {
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();
    let mut symbols = symbol_decoder(payload, mode);
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
        }),
    )
    .ok()?;
    let start = branch_nonzero(branch)?;
    if start.eob_read().eob().eob() != SCAN.len() {
        return None;
    }
    let walk = walk_nonzero_coeff_scan(&start, &SCAN).ok()?;
    Some((tile, symbols, walk))
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

fn invalid_base_eob_selector() -> CoeffCdfSelector {
    CoeffCdfSelector::BaseEob {
        coeff_cdf_q_ctx: 4,
        tx_size: 0,
        ctx: 0,
    }
}

fn invalid_br_selector() -> CoeffCdfSelector {
    CoeffCdfSelector::Br {
        coeff_cdf_q_ctx: 0,
        ctx: 7,
    }
}

fn inputs_for(
    walk: &NonZeroCoeffScanWalk,
    base_levels: u32,
    base_range: CoeffBaseRangeRead,
) -> Vec<CoeffBaseSymbolReadInput> {
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
            base_levels,
            base_range,
        })
        .collect()
}

fn read_payload(
    payload: &[u8],
    mode: CdfUpdateMode,
    base_levels: u32,
    base_range: CoeffBaseRangeRead,
) -> Option<Vec<CoeffBaseSymbolRead>> {
    let (mut tile, mut symbols, walk) = setup_walk(payload, mode)?;
    let inputs = inputs_for(&walk, base_levels, base_range);
    read_nonzero_coeff_base_symbols(&mut tile, &mut symbols, &walk, &inputs).ok()
}

fn find_payload(predicate: impl Fn(&[CoeffBaseSymbolRead]) -> bool) -> [u8; 5] {
    for first in u8::MIN..=u8::MAX {
        for second in u8::MIN..=u8::MAX {
            for suffix in PAYLOAD_SUFFIXES {
                let payload = [first, second, suffix[0], suffix[1], suffix[2]];
                let Some(reads) = read_payload(
                    &payload,
                    CdfUpdateMode::Enabled,
                    BASE_LEVELS,
                    CoeffBaseRangeRead::Enabled {
                        selector: br_selector(),
                    },
                ) else {
                    continue;
                };
                if predicate(&reads) {
                    return payload;
                }
            }
        }
    }
    panic!("no coefficient base symbol payload found");
}

fn direct_read(
    tile: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    inputs: &[CoeffBaseSymbolReadInput],
) -> Result<Vec<BaseReadTuple>, CoeffBaseSymbolReadError> {
    let mut reads = Vec::new();
    for input in inputs {
        let selector = match input.base {
            CoeffBaseSymbolSource::BaseEob { selector }
            | CoeffBaseSymbolSource::Base { selector } => selector,
        };
        let base_symbol = tile
            .read_block_symbol_trace(TileCdfSelector::Coeff(selector), symbols)?
            .get();
        let mut level = match input.base {
            CoeffBaseSymbolSource::BaseEob { .. } => u32::from(base_symbol) + 1,
            CoeffBaseSymbolSource::Base { .. } => u32::from(base_symbol),
        };
        let base_range_symbol = if level > input.base_levels {
            match input.base_range {
                CoeffBaseRangeRead::Enabled { selector } => {
                    let symbol = tile
                        .read_block_symbol_trace(TileCdfSelector::Coeff(selector), symbols)?
                        .get();
                    level += u32::from(symbol);
                    Some(symbol)
                }
                CoeffBaseRangeRead::Disabled => None,
            }
        } else {
            None
        };
        reads.push((input.entry, base_symbol, base_range_symbol, level));
    }
    Ok(reads)
}

fn as_tuples(reads: &[CoeffBaseSymbolRead]) -> Vec<BaseReadTuple> {
    reads
        .iter()
        .map(|read| {
            (
                read.entry(),
                read.base_symbol(),
                read.base_range_symbol(),
                read.level(),
            )
        })
        .collect()
}

#[test]
fn coefficient_base_symbol_read_matches_direct_sequence() {
    let payload = find_payload(|reads| reads.iter().any(|read| read.base_range_symbol().is_some()));
    let (mut direct_tile, mut direct_symbols, direct_walk) =
        setup_walk(&payload, CdfUpdateMode::Enabled).unwrap();
    let (mut helper_tile, mut helper_symbols, helper_walk) =
        setup_walk(&payload, CdfUpdateMode::Enabled).unwrap();
    let inputs = inputs_for(
        &direct_walk,
        BASE_LEVELS,
        CoeffBaseRangeRead::Enabled {
            selector: br_selector(),
        },
    );

    let expected = direct_read(&mut direct_tile, &mut direct_symbols, &inputs).unwrap();
    let actual = read_nonzero_coeff_base_symbols(
        &mut helper_tile,
        &mut helper_symbols,
        &helper_walk,
        &inputs,
    )
    .unwrap();

    assert_eq!(helper_walk.entries(), direct_walk.entries());
    assert_eq!(as_tuples(&actual), expected);
    assert!(actual.iter().any(|read| read.base_range_symbol().is_some()));
    assert_eq!(
        helper_symbols.consumed_bits(),
        direct_symbols.consumed_bits()
    );
    assert_eq!(helper_symbols.symbol_count(), direct_symbols.symbol_count());
    assert_eq!(helper_tile, direct_tile);
}

#[test]
fn coefficient_base_symbol_read_rejects_mismatched_scan_entries_before_read() {
    let payload = find_payload(|reads| !reads.is_empty());
    let (mut tile, mut symbols, walk) = setup_walk(&payload, CdfUpdateMode::Enabled).unwrap();
    let tile_before = tile.clone();
    let consumed_before = symbols.consumed_bits();
    let symbol_count_before = symbols.symbol_count();
    let mut inputs = inputs_for(
        &walk,
        BASE_LEVELS,
        CoeffBaseRangeRead::Enabled {
            selector: br_selector(),
        },
    );
    inputs[0].entry = walk.entries()[1];

    let err = read_nonzero_coeff_base_symbols(&mut tile, &mut symbols, &walk, &inputs).unwrap_err();

    assert!(matches!(
        err,
        CoeffBaseSymbolReadError::ScanEntryMismatch { index: 0, .. }
    ));
    assert_eq!(tile, tile_before);
    assert_eq!(symbols.consumed_bits(), consumed_before);
    assert_eq!(symbols.symbol_count(), symbol_count_before);
}

#[test]
fn coefficient_base_symbol_read_rejects_reached_invalid_selector_before_symbol_read() {
    let payload = find_payload(|reads| !reads.is_empty());
    let (mut tile, mut symbols, walk) = setup_walk(&payload, CdfUpdateMode::Enabled).unwrap();
    let tile_before = tile.clone();
    let consumed_before = symbols.consumed_bits();
    let symbol_count_before = symbols.symbol_count();
    let mut inputs = inputs_for(
        &walk,
        BASE_LEVELS,
        CoeffBaseRangeRead::Enabled {
            selector: br_selector(),
        },
    );
    inputs[0].base = CoeffBaseSymbolSource::BaseEob {
        selector: invalid_base_eob_selector(),
    };

    let err = read_nonzero_coeff_base_symbols(&mut tile, &mut symbols, &walk, &inputs).unwrap_err();

    assert!(matches!(
        err,
        CoeffBaseSymbolReadError::SymbolRead(BlockSymbolTraceReadError::Cdf(
            TileCdfError::SelectorOutOfRange {
                array: TileCdfArray::CoeffBaseEob,
                index_name: "coeff_cdf_q_ctx",
                actual: 4,
                max_exclusive: 4,
            }
        ))
    ));
    assert_eq!(tile, tile_before);
    assert_eq!(symbols.consumed_bits(), consumed_before);
    assert_eq!(symbols.symbol_count(), symbol_count_before);
}

#[test]
fn coefficient_base_symbol_read_honors_disabled_updates_and_unreached_br_selector() {
    let payload = find_payload(|reads| !reads.is_empty());
    let (mut tile, mut symbols, walk) = setup_walk(&payload, CdfUpdateMode::Disabled).unwrap();
    let tile_before = tile.clone();
    let consumed_before = symbols.consumed_bits();
    let symbol_count_before = symbols.symbol_count();
    let inputs = inputs_for(
        &walk,
        u32::MAX,
        CoeffBaseRangeRead::Enabled {
            selector: invalid_br_selector(),
        },
    );

    let reads = read_nonzero_coeff_base_symbols(&mut tile, &mut symbols, &walk, &inputs).unwrap();

    assert_eq!(reads.len(), walk.entries().len());
    assert!(reads.iter().all(|read| read.base_range_symbol().is_none()));
    assert_eq!(tile, tile_before);
    assert!(symbols.consumed_bits() > consumed_before);
    assert!(symbols.symbol_count() > symbol_count_before);
}

#[test]
fn coefficient_base_symbol_read_disabled_base_range_skips_br_above_threshold() {
    let payload = find_payload(|reads| reads.iter().any(|read| read.base_range_symbol().is_some()));
    let (mut tile, mut symbols, walk) = setup_walk(&payload, CdfUpdateMode::Enabled).unwrap();
    let inputs = inputs_for(&walk, BASE_LEVELS, CoeffBaseRangeRead::Disabled);

    let reads = read_nonzero_coeff_base_symbols(&mut tile, &mut symbols, &walk, &inputs).unwrap();

    assert!(reads.iter().all(|read| read.base_range_symbol().is_none()));
    assert!(reads.iter().any(|read| read.level() > BASE_LEVELS));
}

#[test]
fn coefficient_base_symbol_read_count_mismatch_preserves_state() {
    let payload = find_payload(|reads| !reads.is_empty());
    let (mut tile, mut symbols, walk) = setup_walk(&payload, CdfUpdateMode::Enabled).unwrap();
    let tile_before = tile.clone();
    let consumed_before = symbols.consumed_bits();
    let symbol_count_before = symbols.symbol_count();
    let mut inputs = inputs_for(
        &walk,
        BASE_LEVELS,
        CoeffBaseRangeRead::Enabled {
            selector: br_selector(),
        },
    );
    inputs.pop();

    let err = read_nonzero_coeff_base_symbols(&mut tile, &mut symbols, &walk, &inputs).unwrap_err();

    assert!(matches!(
        err,
        CoeffBaseSymbolReadError::InputCountMismatch {
            inputs: 3,
            entries: 4
        }
    ));
    assert_eq!(tile, tile_before);
    assert_eq!(symbols.consumed_bits(), consumed_before);
    assert_eq!(symbols.symbol_count(), symbol_count_before);
}

/// Writes one `coeff_base`/`coeff_base_eob`/`coeff_br` symbol to `tile`'s CDF row
/// for `selector` (adapting it), mirroring the decoder's `read_block_symbol_trace`.
fn encode_coeff_symbol(
    tile: &mut TileCdfSubset,
    encoder: &mut SymbolEncoder,
    selector: CoeffCdfSelector,
    symbol: u8,
) {
    tile.with_row_mut(TileCdfSelector::Coeff(selector), |row| {
        encoder.write_symbol(row, Symbol::new(symbol))
    })
    .unwrap()
    .unwrap();
}

#[test]
fn coefficient_base_symbols_roundtrip_through_symbol_encoder() {
    let entry0 = CoeffScanEntry::for_test(1, 8, 1, 0);
    let entry1 = CoeffScanEntry::for_test(0, 0, 0, 0);
    let walk = NonZeroCoeffScanWalk::from_entries_for_test(vec![entry0, entry1]);
    let base_range = CoeffBaseRangeRead::Enabled {
        selector: br_selector(),
    };
    let inputs = vec![
        CoeffBaseSymbolReadInput {
            entry: entry0,
            base: CoeffBaseSymbolSource::BaseEob {
                selector: base_eob_selector(),
            },
            base_levels: BASE_LEVELS,
            base_range,
        },
        CoeffBaseSymbolReadInput {
            entry: entry1,
            base: CoeffBaseSymbolSource::Base {
                selector: base_selector(),
            },
            base_levels: BASE_LEVELS,
            base_range,
        },
    ];

    let mut enc_tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut encoder = SymbolEncoder::new();
    encode_coeff_symbol(&mut enc_tile, &mut encoder, base_eob_selector(), 2);
    encode_coeff_symbol(&mut enc_tile, &mut encoder, br_selector(), 1);
    encode_coeff_symbol(&mut enc_tile, &mut encoder, base_selector(), 1);
    let bytes = encoder.finish().unwrap().into_bytes();

    let mut dec_tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut symbols = symbol_decoder(&bytes, CdfUpdateMode::Enabled);
    let reads =
        read_nonzero_coeff_base_symbols(&mut dec_tile, &mut symbols, &walk, &inputs).unwrap();

    assert_eq!(
        as_tuples(&reads),
        vec![(entry0, 2u8, Some(1u8), 4u32), (entry1, 1u8, None, 1u32)]
    );
    assert_eq!(
        dec_tile
            .row(TileCdfSelector::Coeff(base_eob_selector()))
            .unwrap(),
        enc_tile
            .row(TileCdfSelector::Coeff(base_eob_selector()))
            .unwrap()
    );
}

#[test]
fn coefficient_base_eob_only_roundtrips_through_symbol_encoder() {
    let entry = CoeffScanEntry::for_test(0, 0, 0, 0);
    let walk = NonZeroCoeffScanWalk::from_entries_for_test(vec![entry]);
    let inputs = vec![CoeffBaseSymbolReadInput {
        entry,
        base: CoeffBaseSymbolSource::BaseEob {
            selector: base_eob_selector(),
        },
        base_levels: BASE_LEVELS,
        base_range: CoeffBaseRangeRead::Enabled {
            selector: br_selector(),
        },
    }];

    let mut enc_tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut encoder = SymbolEncoder::new();
    encode_coeff_symbol(&mut enc_tile, &mut encoder, base_eob_selector(), 1);
    let bytes = encoder.finish().unwrap().into_bytes();

    let mut dec_tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut symbols = symbol_decoder(&bytes, CdfUpdateMode::Enabled);
    let reads =
        read_nonzero_coeff_base_symbols(&mut dec_tile, &mut symbols, &walk, &inputs).unwrap();

    assert_eq!(as_tuples(&reads), vec![(entry, 1u8, None, 2u32)]);
}

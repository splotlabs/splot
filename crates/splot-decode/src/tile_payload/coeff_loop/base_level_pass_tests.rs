// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::panic, clippy::unwrap_used)]

use splot_core::span::ByteOffset;
use splot_core::symbol::{CdfUpdateMode, SymbolDecoder, SymbolDecoderConfig};

use super::super::cdf::{CoeffCdfSelector, FrameCdfSubset, TileCdfSubset};
use super::super::coeff_state::TileCoeffContextState;
use super::base_level_pass::{
    CoeffBaseDerivedLevelPassConfig, CoeffBaseDerivedLevelPassError,
    NonZeroCoeffBaseDerivedLevelPass, apply_nonzero_coeff_base_derived_level_pass,
};
use super::base_symbol::{CoeffBaseRangeRead, CoeffBaseSymbolSource};
use super::branch::{CoeffBlockEobBranch, NonZeroCoeffBlockStart, NonZeroCoeffBlockStartInput};
use super::max_level::{COEFF_BASE_RANGE, CoeffTransformClass, NUM_BASE_LEVELS};
use super::quant_state::next_tcq_state;
use super::scan_walk::{NonZeroCoeffScanWalk, walk_nonzero_coeff_scan};
use super::*;

const SCAN: [u16; 4] = [0, 8, 1, 9];
const DC_FIRST_SCAN: [u16; 4] = [9, 8, 1, 0];
const PAYLOAD_SUFFIXES: [[u8; 3]; 4] = [
    [0x00, 0x00, 0x80],
    [0xff, 0x00, 0x80],
    [0x55, 0xaa, 0x80],
    [0xff, 0xff, 0x80],
];

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
    plane: usize,
    scan: &[u16],
) -> Option<(
    TileCdfSubset,
    SymbolDecoder<'a>,
    NonZeroCoeffBlockStart,
    NonZeroCoeffScanWalk,
)> {
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
                plane,
                x4: 0,
                y4: 0,
                w4: 2,
                h4: 2,
            },
            eob: NonZeroCoeffEobContextInput {
                plane,
                is_inter: false,
                tx_width_log2: 3,
                tx_height_log2: 3,
                coeff_cdf_q_ctx: 0,
            },
        }),
    )
    .ok()?;
    let start = branch_nonzero(branch)?;
    if start.eob_read().eob().eob() != scan.len() {
        return None;
    }
    let walk = walk_nonzero_coeff_scan(&start, scan).ok()?;
    Some((tile, symbols, start, walk))
}

fn luma_config(parity_hiding: bool, use_tcq: bool) -> CoeffBaseDerivedLevelPassConfig {
    CoeffBaseDerivedLevelPassConfig {
        coeff_cdf_q_ctx: 0,
        tx_size_ctx: 0,
        tx_width_log2: 3,
        tx_width: 8,
        tx_height: 8,
        plane: 0,
        tx_class: CoeffTransformClass::TwoD,
        parity_hiding,
        use_tcq,
    }
}

fn chroma_config() -> CoeffBaseDerivedLevelPassConfig {
    CoeffBaseDerivedLevelPassConfig {
        plane: 1,
        ..luma_config(false, false)
    }
}

fn run_pass(
    payload: &[u8],
    plane: usize,
    scan: &[u16],
    config: CoeffBaseDerivedLevelPassConfig,
) -> Option<NonZeroCoeffBaseDerivedLevelPass> {
    let (mut tile, mut symbols, start, walk) = setup_start(payload, plane, scan)?;
    apply_nonzero_coeff_base_derived_level_pass(&mut tile, &mut symbols, start, walk, config).ok()
}

fn find_payload(
    plane: usize,
    scan: &[u16],
    config: CoeffBaseDerivedLevelPassConfig,
    predicate: impl Fn(&NonZeroCoeffBaseDerivedLevelPass) -> bool,
) -> [u8; 5] {
    for first in u8::MIN..=u8::MAX {
        for second in u8::MIN..=u8::MAX {
            for suffix in PAYLOAD_SUFFIXES {
                let payload = [first, second, suffix[0], suffix[1], suffix[2]];
                let Some(pass) = run_pass(&payload, plane, scan, config) else {
                    continue;
                };
                if predicate(&pass) {
                    return payload;
                }
            }
        }
    }
    panic!("no coefficient base/level payload found");
}

fn base_tcq_ctx(input: &super::base_symbol::CoeffBaseSymbolReadInput) -> Option<usize> {
    match input.base {
        CoeffBaseSymbolSource::Base {
            selector:
                CoeffCdfSelector::Base { tcq_ctx, .. } | CoeffCdfSelector::BaseLf { tcq_ctx, .. },
        } => Some(tcq_ctx),
        _ => None,
    }
}

#[test]
fn coefficient_base_level_pass_derives_later_contexts_from_written_levels() {
    let payload = find_payload(0, &SCAN, luma_config(false, false), |_| true);
    let pass = run_pass(&payload, 0, &SCAN, luma_config(false, false)).unwrap();
    let first = pass.base_reads()[0];
    let second_input = pass.derived_inputs()[1];

    assert_eq!(pass.eob_read().eob().eob(), SCAN.len());
    assert_eq!(pass.walk().entries().len(), SCAN.len());
    assert_eq!(
        pass.block()
            .level_at(first.entry().row(), first.entry().col())
            .unwrap(),
        first.level()
    );
    assert!(matches!(
        pass.derived_inputs()[0].base,
        CoeffBaseSymbolSource::BaseEob {
            selector: CoeffCdfSelector::BaseLfEob { ctx: 1, .. }
        }
    ));
    assert!(matches!(
        second_input.base,
        CoeffBaseSymbolSource::Base {
            selector: CoeffCdfSelector::BaseLf { ctx, tcq_ctx: 0, .. }
        } if ctx > 9
    ));
}

#[test]
fn coefficient_base_level_pass_tracks_first_pass_tcq_state_for_selectors() {
    let config = luma_config(false, true);
    let payload = find_payload(0, &SCAN, config, |pass| {
        pass.derived_inputs()
            .iter()
            .filter_map(base_tcq_ctx)
            .any(|tcq_ctx| tcq_ctx == 1)
    });
    let pass = run_pass(&payload, 0, &SCAN, config).unwrap();
    let expected_tcq_state = pass.base_reads().iter().fold(0usize, |tcq_state, read| {
        next_tcq_state(tcq_state, read.level()).unwrap()
    });

    assert_eq!(pass.first_pass().tcq_state(), expected_tcq_state);
    assert!(
        pass.derived_inputs()
            .iter()
            .filter_map(base_tcq_ctx)
            .any(|tcq_ctx| tcq_ctx == 1)
    );
}

#[test]
fn coefficient_base_level_pass_tracks_parity_hiding_summary_before_dc() {
    let config = luma_config(true, false);
    let payload = find_payload(0, &SCAN, config, |pass| {
        pass.first_pass().num_nonzero() > 0 || pass.first_pass().sum_abs1() > 0
    });
    let pass = run_pass(&payload, 0, &SCAN, config).unwrap();
    let mut expected_sum_abs1 = 0u32;
    let mut expected_num_nonzero = 0usize;
    for read in pass
        .base_reads()
        .iter()
        .copied()
        .filter(|read| read.entry().scan_index() > 0)
    {
        let clipped = read.level().min(NUM_BASE_LEVELS + COEFF_BASE_RANGE + 1);
        expected_sum_abs1 ^= clipped & 1;
        if read.level() != 0 {
            expected_num_nonzero += 1;
        }
    }

    assert_eq!(pass.first_pass().sum_abs1(), expected_sum_abs1);
    assert_eq!(pass.first_pass().num_nonzero(), expected_num_nonzero);
    assert!(!pass.first_pass().is_hidden());
}

#[test]
fn coefficient_base_level_pass_disables_chroma_low_frequency_base_range() {
    let config = chroma_config();
    let payload = find_payload(1, &DC_FIRST_SCAN, config, |pass| {
        pass.base_reads()[0].level() > 4 && pass.base_reads()[0].base_range_symbol().is_none()
    });
    let pass = run_pass(&payload, 1, &DC_FIRST_SCAN, config).unwrap();

    assert!(matches!(
        pass.derived_inputs()[0].base,
        CoeffBaseSymbolSource::BaseEob {
            selector: CoeffCdfSelector::BaseLfEobUv { .. }
        }
    ));
    assert!(matches!(
        pass.derived_inputs()[0].base_range,
        CoeffBaseRangeRead::Disabled
    ));
    assert!(pass.base_reads()[0].level() > 4);
    assert_eq!(pass.base_reads()[0].base_range_symbol(), None);
}

#[test]
fn coefficient_base_level_pass_rejects_static_config_before_base_consumption() {
    let payload = find_payload(0, &SCAN, luma_config(false, false), |_| true);
    let (mut tile, mut symbols, start, walk) = setup_start(&payload, 0, &SCAN).unwrap();
    let tile_before = tile.clone();
    let consumed_before = symbols.consumed_bits();
    let symbol_count_before = symbols.symbol_count();
    let err = apply_nonzero_coeff_base_derived_level_pass(
        &mut tile,
        &mut symbols,
        start,
        walk,
        CoeffBaseDerivedLevelPassConfig {
            tx_width: 16,
            ..luma_config(false, false)
        },
    )
    .unwrap_err();

    assert!(matches!(
        err,
        CoeffBaseDerivedLevelPassError::BlockGeometryMismatch {
            block_width: 8,
            block_height: 8,
            config_width: 16,
            config_height: 8
        }
    ));
    assert_eq!(tile, tile_before);
    assert_eq!(symbols.consumed_bits(), consumed_before);
    assert_eq!(symbols.symbol_count(), symbol_count_before);
}

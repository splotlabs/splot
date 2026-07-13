// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::panic, clippy::unwrap_used)]

use splot_core::symbol::SymbolDecoder;

use super::super::cdf::{CoeffCdfSelector, TileCdfSelector, TileCdfSubset};
use super::base_level_pass::{
    CoeffBaseDerivedLevelPassConfig, CoeffBaseDerivedLevelPassError,
    NonZeroCoeffBaseDerivedLevelPass, apply_nonzero_coeff_base_derived_level_pass,
};
use super::branch::{NonZeroCoeffBlockStart, NonZeroCoeffBlockStartInput};
use super::max_level::{COEFF_BASE_RANGE, CoeffTransformClass, NUM_BASE_LEVELS};
use super::quant_state::next_tcq_state;
use super::scan_walk::{NonZeroCoeffScanWalk, walk_nonzero_coeff_scan};
use super::test_support::setup_start_with_input;
use super::*;

const SCAN: [u16; 4] = [0, 8, 1, 9];
const DC_FIRST_SCAN: [u16; 4] = [9, 8, 1, 0];
const DC_LAST_HIDDEN_SCAN: [u16; 5] = [0, 1, 8, 9, 2];
const PAYLOAD_SUFFIXES: [[u8; 3]; 4] = [
    [0x00, 0x00, 0x80],
    [0xff, 0x00, 0x80],
    [0x55, 0xaa, 0x80],
    [0xff, 0xff, 0x80],
];

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
    let (tile, symbols, start) = setup_start_with_input(
        payload,
        NonZeroCoeffBlockStartInput {
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
        },
    )?;
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

fn base_lf_rows(tile: &TileCdfSubset, first_ctx: usize, tcq_ctx: usize) -> Vec<Vec<i32>> {
    (first_ctx..)
        .map_while(|ctx| {
            tile.row(TileCdfSelector::Coeff(CoeffCdfSelector::BaseLf {
                coeff_cdf_q_ctx: 0,
                tx_size: 0,
                ctx,
                tcq_ctx,
            }))
            .ok()
            .map(<[i32]>::to_vec)
        })
        .collect()
}

fn base_lf_row_changed(
    tile: &TileCdfSubset,
    rows_before: &[Vec<i32>],
    first_ctx: usize,
    tcq_ctx: usize,
) -> bool {
    rows_before.iter().enumerate().any(|(offset, before)| {
        tile.row(TileCdfSelector::Coeff(CoeffCdfSelector::BaseLf {
            coeff_cdf_q_ctx: 0,
            tx_size: 0,
            ctx: first_ctx + offset,
            tcq_ctx,
        }))
        .is_ok_and(|row| row != before.as_slice())
    })
}

fn uses_tcq_context_one(pass: &NonZeroCoeffBaseDerivedLevelPass) -> bool {
    let mut tcq_state = 0usize;
    pass.walk().entries().iter().any(|entry| {
        let uses_context_one = (tcq_state >> 1) & 1 == 1;
        let level = pass.block().level_at(entry.row(), entry.col()).unwrap();
        tcq_state = next_tcq_state(tcq_state, level).unwrap();
        uses_context_one
    })
}

#[test]
fn coefficient_base_level_pass_derives_later_contexts_from_written_levels() {
    let config = luma_config(false, false);
    let payload = find_payload(0, &SCAN, config, |_| true);
    let (mut tile, mut symbols, start, walk) = setup_start(&payload, 0, &SCAN).unwrap();
    let rows_before = base_lf_rows(&tile, 10, 0);
    let pass =
        apply_nonzero_coeff_base_derived_level_pass(&mut tile, &mut symbols, start, walk, config)
            .unwrap();

    assert_eq!(pass.eob_read().eob().eob(), SCAN.len());
    assert_eq!(pass.walk().entries().len(), SCAN.len());
    assert!(base_lf_row_changed(&tile, &rows_before, 10, 0));
}

#[test]
fn coefficient_base_level_pass_tracks_first_pass_tcq_state_for_selectors() {
    let config = luma_config(false, true);
    let payload = find_payload(0, &SCAN, config, uses_tcq_context_one);
    let (mut tile, mut symbols, start, walk) = setup_start(&payload, 0, &SCAN).unwrap();
    let rows_before = base_lf_rows(&tile, 0, 1);
    let pass =
        apply_nonzero_coeff_base_derived_level_pass(&mut tile, &mut symbols, start, walk, config)
            .unwrap();
    let expected_tcq_state = pass
        .walk()
        .entries()
        .iter()
        .fold(0usize, |tcq_state, entry| {
            let level = pass.block().level_at(entry.row(), entry.col()).unwrap();
            next_tcq_state(tcq_state, level).unwrap()
        });

    assert_eq!(pass.first_pass().tcq_state(), expected_tcq_state);
    assert!(base_lf_row_changed(&tile, &rows_before, 0, 1));
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
    for entry in pass
        .walk()
        .entries()
        .iter()
        .filter(|entry| entry.scan_index() > 0)
    {
        let level = pass.block().level_at(entry.row(), entry.col()).unwrap();
        let clipped = level.min(NUM_BASE_LEVELS + COEFF_BASE_RANGE + 1);
        expected_sum_abs1 ^= clipped & 1;
        if level != 0 {
            expected_num_nonzero += 1;
        }
    }

    assert_eq!(pass.first_pass().sum_abs1(), expected_sum_abs1);
    assert_eq!(pass.first_pass().num_nonzero(), expected_num_nonzero);
    assert!(!pass.first_pass().is_hidden());
}

#[test]
fn coefficient_base_level_pass_consumes_parity_hidden_base_row() {
    let config = luma_config(true, false);
    let payload = find_payload(0, &DC_LAST_HIDDEN_SCAN, config, |pass| {
        pass.first_pass().is_hidden() && pass.first_pass().num_nonzero() >= 4
    });
    let (mut tile, mut symbols, start, walk) =
        setup_start(&payload, 0, &DC_LAST_HIDDEN_SCAN).unwrap();
    let ph_rows_before = (0..5)
        .map(|ctx| {
            tile.row(TileCdfSelector::Coeff(CoeffCdfSelector::BasePh {
                coeff_cdf_q_ctx: 0,
                ctx,
            }))
            .unwrap()
            .to_vec()
        })
        .collect::<Vec<_>>();
    let pass =
        apply_nonzero_coeff_base_derived_level_pass(&mut tile, &mut symbols, start, walk, config)
            .unwrap();
    let final_entry = pass.walk().entries().last().copied().unwrap();

    assert_eq!(pass.eob_read().eob().eob(), DC_LAST_HIDDEN_SCAN.len());
    assert!(pass.first_pass().is_hidden());
    assert_eq!(final_entry.scan_index(), 0);
    assert!((0..5).any(|ctx| {
        tile.row(TileCdfSelector::Coeff(CoeffCdfSelector::BasePh {
            coeff_cdf_q_ctx: 0,
            ctx,
        }))
        .is_ok_and(|row| row != ph_rows_before[ctx].as_slice())
    }));
}

#[test]
fn coefficient_base_level_pass_disables_chroma_low_frequency_base_range() {
    let config = chroma_config();
    let payload = find_payload(1, &DC_FIRST_SCAN, config, |pass| {
        let entry = pass.walk().entries()[0];
        pass.block()
            .level_at(entry.row(), entry.col())
            .is_ok_and(|level| level > 4)
    });
    let (mut tile, mut symbols, start, walk) = setup_start(&payload, 1, &DC_FIRST_SCAN).unwrap();
    let symbol_count_before = symbols.symbol_count();
    let pass =
        apply_nonzero_coeff_base_derived_level_pass(&mut tile, &mut symbols, start, walk, config)
            .unwrap();
    let first_entry = pass.walk().entries()[0];

    assert!(
        pass.block()
            .level_at(first_entry.row(), first_entry.col())
            .is_ok_and(|level| level > 4)
    );
    assert_eq!(
        symbols.symbol_count() - symbol_count_before,
        DC_FIRST_SCAN.len() as u64
    );
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

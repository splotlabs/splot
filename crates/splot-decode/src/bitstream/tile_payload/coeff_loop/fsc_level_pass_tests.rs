// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::panic, clippy::unwrap_used)]

use splot_core::symbol::SymbolDecoder;

use super::super::cdf::{CoeffCdfSelector, TileCdfSelector, TileCdfSubset};
use super::branch::{NonZeroCoeffBlockStart, NonZeroCoeffBlockStartInput};
use super::fsc_level_pass::{
    CoeffFscLevelPassConfig, CoeffFscLevelPassError, CoeffFscLevelSymbolSource,
    NonZeroCoeffFscLevelPass, apply_nonzero_coeff_fsc_level_pass,
};
use super::scan_walk::{FscCoeffScanWalk, walk_fsc_coeff_scan};
use super::test_support::setup_start_with_input;
use super::*;

const SCAN: [u16; 4] = [0, 8, 1, 9];
const PAYLOAD_SUFFIXES: [[u8; 3]; 4] = [
    [0x00, 0x00, 0x80],
    [0xff, 0x00, 0x80],
    [0x55, 0xaa, 0x80],
    [0xff, 0xff, 0x80],
];

fn config() -> CoeffFscLevelPassConfig {
    CoeffFscLevelPassConfig {
        coeff_cdf_q_ctx: 0,
        tx_size_ctx: 4,
        tx_width: 8,
        tx_height: 8,
    }
}

fn setup_start(
    payload: &[u8],
    seg_eob: usize,
) -> Option<(
    TileCdfSubset,
    SymbolDecoder<'_>,
    NonZeroCoeffBlockStart,
    FscCoeffScanWalk,
)> {
    let (tile, symbols, start) = setup_start_with_input(
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
    if start.eob_read().eob().eob() != SCAN.len() - 2 {
        return None;
    }
    let walk = walk_fsc_coeff_scan(&start, seg_eob, &SCAN).ok()?;
    Some((tile, symbols, start, walk))
}

fn run_pass(payload: &[u8], seg_eob: usize) -> Option<NonZeroCoeffFscLevelPass> {
    let (mut tile, mut symbols, start, walk) = setup_start(payload, seg_eob)?;
    apply_nonzero_coeff_fsc_level_pass(&mut tile, &mut symbols, start, walk, config()).ok()
}

fn find_payload(seg_eob: usize, predicate: impl Fn(&NonZeroCoeffFscLevelPass) -> bool) -> [u8; 5] {
    for first in u8::MIN..=u8::MAX {
        for second in u8::MIN..=u8::MAX {
            for suffix in PAYLOAD_SUFFIXES {
                let payload = [first, second, suffix[0], suffix[1], suffix[2]];
                let Some(pass) = run_pass(&payload, seg_eob) else {
                    continue;
                };
                if predicate(&pass) {
                    return payload;
                }
            }
        }
    }
    panic!("no coefficient FSC level payload found");
}

fn base_selector(input: &super::fsc_level_pass::CoeffFscLevelReadInput) -> CoeffCdfSelector {
    match input.base {
        CoeffFscLevelSymbolSource::BaseBob { selector }
        | CoeffFscLevelSymbolSource::BaseIdtx { selector } => selector,
    }
}

#[test]
fn coefficient_fsc_level_pass_reads_basebob_then_baseidtx_and_writes_levels() {
    let payload = find_payload(4, |_| true);
    let pass = run_pass(&payload, 4).unwrap();

    assert_eq!(pass.eob_read().eob().eob(), 2);
    assert_eq!(pass.walk().bob(), 2);
    assert_eq!(pass.walk().seg_eob(), 4);
    assert_eq!(pass.level_reads().len(), 2);
    assert!(pass.level_reads()[0].base_symbol() <= 3);
    assert_eq!(pass.derived_inputs()[0].entry.scan_index(), 2);
    assert!(matches!(
        pass.derived_inputs()[0].base,
        CoeffFscLevelSymbolSource::BaseBob {
            selector: CoeffCdfSelector::BaseBob {
                coeff_cdf_q_ctx: 0,
                tx_size_ctx: 2,
                ctx: 2
            }
        }
    ));
    assert!(matches!(
        pass.derived_inputs()[1].base,
        CoeffFscLevelSymbolSource::BaseIdtx {
            selector: CoeffCdfSelector::BaseIdtx {
                coeff_cdf_q_ctx: 0,
                tx_size_ctx: 2,
                ..
            }
        }
    ));
    for read in pass.level_reads() {
        let entry = read.entry();
        assert_eq!(
            pass.block().level_at(entry.row(), entry.col()).unwrap(),
            read.level()
        );
    }
    assert!(pass.block().quant().iter().all(|quant| *quant == 0));
    assert!(pass.block().quant_sign().iter().all(|sign| *sign == 0));
}

#[test]
fn coefficient_fsc_level_pass_derives_idtx_context_from_written_neighbour_level() {
    let payload = find_payload(4, |pass| {
        pass.level_reads()[0].level() > 0
            && matches!(
                base_selector(&pass.derived_inputs()[1]),
                CoeffCdfSelector::BaseIdtx { ctx, .. } if ctx > 0
            )
    });
    let pass = run_pass(&payload, 4).unwrap();
    let first = pass.level_reads()[0];
    let second_input = pass.derived_inputs()[1];

    assert_eq!(first.entry().pos(), 1);
    assert_eq!(second_input.entry.pos(), 9);
    assert_eq!(
        pass.block()
            .level_at(first.entry().row(), first.entry().col())
            .unwrap(),
        first.level()
    );
    assert!(matches!(
        base_selector(&second_input),
        CoeffCdfSelector::BaseIdtx { ctx, .. } if ctx > 0
    ));
}

#[test]
fn coefficient_fsc_level_pass_conditionally_reads_bridtx() {
    let payload = find_payload(4, |pass| {
        pass.level_reads()
            .iter()
            .any(|read| read.base_range_symbol().is_some())
    });
    let shadow = run_pass(&payload, 4).unwrap();
    let br_selector = shadow
        .derived_inputs()
        .iter()
        .zip(shadow.level_reads())
        .find_map(|(input, read)| read.base_range_symbol().map(|_| input.base_range))
        .unwrap();
    let (mut tile, mut symbols, start, walk) = setup_start(&payload, 4).unwrap();
    let br_before = tile
        .row(TileCdfSelector::Coeff(br_selector))
        .unwrap()
        .to_vec();
    let pass =
        apply_nonzero_coeff_fsc_level_pass(&mut tile, &mut symbols, start, walk, config()).unwrap();

    assert!(
        pass.level_reads()
            .iter()
            .any(|read| read.base_range_symbol().is_some())
    );
    assert_ne!(
        tile.row(TileCdfSelector::Coeff(br_selector)).unwrap(),
        br_before.as_slice()
    );
}

#[test]
fn coefficient_fsc_level_pass_rejects_static_config_before_consumption() {
    let payload = find_payload(4, |_| true);
    let (mut tile, mut symbols, start, walk) = setup_start(&payload, 4).unwrap();
    let tile_before = tile.clone();
    let consumed_before = symbols.consumed_bits();
    let symbol_count_before = symbols.symbol_count();
    let block_before = start.block().clone();
    let err = apply_nonzero_coeff_fsc_level_pass(
        &mut tile,
        &mut symbols,
        start,
        walk,
        CoeffFscLevelPassConfig {
            tx_width: 16,
            ..config()
        },
    )
    .unwrap_err();

    assert!(matches!(
        err,
        CoeffFscLevelPassError::BlockGeometryMismatch {
            block_width: 8,
            block_height: 8,
            config_width: 16,
            config_height: 8
        }
    ));
    assert_eq!(tile, tile_before);
    assert_eq!(symbols.consumed_bits(), consumed_before);
    assert_eq!(symbols.symbol_count(), symbol_count_before);
    assert!(block_before.level().iter().all(|level| *level == 0));
}

// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::panic, clippy::unwrap_used)]

use splot_core::symbol::SymbolDecoder;

use super::super::cdf::{CoeffCdfSelector, TileCdfSubset};
use super::branch::NonZeroCoeffBlockStartInput;
use super::fsc_level_pass::{
    CoeffFscLevelPassConfig, NonZeroCoeffFscLevelPass, apply_nonzero_coeff_fsc_level_pass,
};
use super::fsc_sign_pass::{
    CoeffFscSignPassError, CoeffFscSignReadSource, CoeffFscSignReadSymbol, NonZeroCoeffFscSignPass,
    apply_nonzero_coeff_fsc_sign_pass,
};
use super::max_level::COEFF_BASE_RANGE;
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

fn setup_level_pass(
    payload: &[u8],
    seg_eob: usize,
) -> Option<(
    TileCdfSubset,
    SymbolDecoder<'_>,
    FscCoeffScanWalk,
    NonZeroCoeffFscLevelPass,
)> {
    let (mut tile, mut symbols, start) = setup_start_with_input(
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
    let pass =
        apply_nonzero_coeff_fsc_level_pass(&mut tile, &mut symbols, start, walk.clone(), config())
            .ok()?;
    Some((tile, symbols, walk, pass))
}

fn run_pass(payload: &[u8], seg_eob: usize) -> Option<NonZeroCoeffFscSignPass> {
    let (mut tile, mut symbols, _walk, level_pass) = setup_level_pass(payload, seg_eob)?;
    apply_nonzero_coeff_fsc_sign_pass(&mut tile, &mut symbols, level_pass, &SCAN, config()).ok()
}

fn find_payload(seg_eob: usize, predicate: impl Fn(&NonZeroCoeffFscSignPass) -> bool) -> [u8; 5] {
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
    panic!("no coefficient FSC sign payload found");
}

fn expected_quant_sign(sign: bool) -> i8 {
    if sign { -1 } else { 1 }
}

#[test]
fn coefficient_fsc_sign_pass_reads_idtx_signs_and_writes_quant_signs() {
    let payload = find_payload(4, |pass| {
        pass.sign_reads()
            .iter()
            .filter(|read| read.level() != 0)
            .count()
            == 2
    });
    let pass = run_pass(&payload, 4).unwrap();

    assert_eq!(pass.eob_read().eob().eob(), 2);
    assert_eq!(pass.level_walk().bob(), 2);
    assert_eq!(pass.level_walk().seg_eob(), 4);
    assert_eq!(pass.level_reads().len(), 2);
    assert_eq!(pass.sign_entries().len(), 2);
    assert_eq!(pass.sign_reads().len(), 2);
    assert_eq!(pass.sign_entries(), pass.level_walk().entries());
    for read in pass.sign_reads().iter().filter(|read| read.level() != 0) {
        assert!(matches!(
            read.symbol(),
            CoeffFscSignReadSymbol::IdtxSign { .. }
        ));
        assert_eq!(
            pass.block()
                .quant_sign_at(read.entry().row(), read.entry().col())
                .unwrap(),
            expected_quant_sign(read.sign())
        );
    }
    for input in pass.sign_inputs().iter().filter(|input| input.level != 0) {
        assert!(matches!(
            input.source,
            CoeffFscSignReadSource::IdtxSign {
                selector: CoeffCdfSelector::IdtxSign {
                    coeff_cdf_q_ctx: 0,
                    tx_size_ctx: 2,
                    ..
                }
            }
        ));
    }
    assert!(pass.block().quant().iter().all(|quant| *quant == 0));
}

#[test]
fn coefficient_fsc_sign_pass_derives_context_from_written_quant_sign() {
    let payload = find_payload(4, |pass| {
        pass.sign_reads()[0].level() != 0
            && pass.sign_reads()[1].level() != 0
            && matches!(
                pass.sign_inputs()[1].source,
                CoeffFscSignReadSource::IdtxSign {
                    selector: CoeffCdfSelector::IdtxSign { ctx, .. }
                } if ctx > 0
            )
    });
    let pass = run_pass(&payload, 4).unwrap();
    let first = pass.sign_reads()[0];
    let second = pass.sign_reads()[1];
    let expected_base_ctx = if first.sign() { 2 } else { 1 };
    let expected_ctx = if second.level() > COEFF_BASE_RANGE {
        expected_base_ctx + 2
    } else {
        expected_base_ctx
    };

    assert_eq!(
        pass.block()
            .quant_sign_at(first.entry().row(), first.entry().col())
            .unwrap(),
        expected_quant_sign(first.sign())
    );
    assert!(matches!(
        pass.sign_inputs()[1].source,
        CoeffFscSignReadSource::IdtxSign {
            selector: CoeffCdfSelector::IdtxSign { ctx, .. }
        } if ctx == expected_ctx
    ));
}

#[test]
fn coefficient_fsc_sign_pass_rejects_static_config_before_consumption() {
    let payload = find_payload(4, |_| true);
    let (mut tile, mut symbols, _walk, level_pass) = setup_level_pass(&payload, 4).unwrap();
    let tile_before = tile.clone();
    let consumed_before = symbols.consumed_bits();
    let symbol_count_before = symbols.symbol_count();
    let block_before = level_pass.block().clone();
    let err = apply_nonzero_coeff_fsc_sign_pass(
        &mut tile,
        &mut symbols,
        level_pass,
        &SCAN,
        CoeffFscLevelPassConfig {
            tx_width: 16,
            ..config()
        },
    )
    .unwrap_err();

    assert!(matches!(
        err,
        CoeffFscSignPassError::BlockGeometryMismatch {
            block_width: 8,
            block_height: 8,
            config_width: 16,
            config_height: 8
        }
    ));
    assert_eq!(tile, tile_before);
    assert_eq!(symbols.consumed_bits(), consumed_before);
    assert_eq!(symbols.symbol_count(), symbol_count_before);
    assert!(block_before.quant_sign().iter().all(|sign| *sign == 0));
}

#[test]
fn coefficient_fsc_sign_pass_rejects_short_scan_before_consumption() {
    let payload = find_payload(4, |_| true);
    let (mut tile, mut symbols, _walk, level_pass) = setup_level_pass(&payload, 4).unwrap();
    let tile_before = tile.clone();
    let consumed_before = symbols.consumed_bits();
    let symbol_count_before = symbols.symbol_count();
    let err = apply_nonzero_coeff_fsc_sign_pass(
        &mut tile,
        &mut symbols,
        level_pass,
        &SCAN[..3],
        config(),
    )
    .unwrap_err();

    assert!(matches!(
        err,
        CoeffFscSignPassError::ScanTooShort {
            seg_eob: 4,
            scan_len: 3
        }
    ));
    assert_eq!(tile, tile_before);
    assert_eq!(symbols.consumed_bits(), consumed_before);
    assert_eq!(symbols.symbol_count(), symbol_count_before);
}

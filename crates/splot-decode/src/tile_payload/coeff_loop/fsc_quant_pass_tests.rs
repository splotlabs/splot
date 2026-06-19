// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::panic, clippy::unwrap_used)]

use splot_core::span::ByteOffset;
use splot_core::symbol::{CdfUpdateMode, SymbolDecoder, SymbolDecoderConfig};

use super::super::cdf::{FrameCdfSubset, TileCdfSubset};
use super::super::coeff_state::{CoeffContextUpdate, TileCoeffContextState, TileCoeffStateError};
use super::branch::{CoeffBlockEobBranch, NonZeroCoeffBlockStart, NonZeroCoeffBlockStartInput};
use super::fsc_level_pass::{
    CoeffFscLevelPassConfig, NonZeroCoeffFscLevelPass, apply_nonzero_coeff_fsc_level_pass,
};
use super::fsc_quant_pass::{
    CoeffFscContextCommitConfig, CoeffFscQuantPassError, NonZeroCoeffFscQuantPass,
    apply_nonzero_coeff_fsc_quant_pass, apply_nonzero_coeff_fsc_quant_pass_with_context_commit,
};
use super::fsc_sign_pass::{
    CoeffFscSignPassError, CoeffFscSignRead, NonZeroCoeffFscSignPass,
    apply_nonzero_coeff_fsc_sign_pass,
};
use super::max_level::{COEFF_BASE_RANGE, NUM_BASE_LEVELS};
use super::quant_state::{CoeffQuantStateAccumulator, CoeffQuantStateConfig};
use super::read_quant::CoeffReadQuantPath;
use super::read_quant::{CoeffReadQuantConfig, CoeffReadQuantInput, CoeffReadQuantState};
use super::scan_walk::{FscCoeffScanWalk, walk_fsc_coeff_scan};
use super::*;

const SCAN: [u16; 4] = [0, 8, 1, 9];
const PAYLOAD_SUFFIXES: [[u8; 6]; 6] = [
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x80],
    [0xff, 0x00, 0x00, 0x00, 0x00, 0x80],
    [0x55, 0xaa, 0x00, 0x00, 0x00, 0x80],
    [0xff, 0xff, 0x00, 0x00, 0x00, 0x80],
    [0x00, 0x00, 0b0011_0100, 0x00, 0x00, 0x80],
    [0xff, 0xff, 0b0011_0100, 0xff, 0x00, 0x80],
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

fn config() -> CoeffFscLevelPassConfig {
    CoeffFscLevelPassConfig {
        coeff_cdf_q_ctx: 0,
        tx_size_ctx: 4,
        tx_width: 8,
        tx_height: 8,
    }
}

fn context_commit_config() -> CoeffFscContextCommitConfig {
    CoeffFscContextCommitConfig {
        plane: 0,
        x4: 1,
        y4: 2,
        w4: 2,
        h4: 2,
    }
}

fn chroma_context_commit_config() -> CoeffFscContextCommitConfig {
    CoeffFscContextCommitConfig {
        plane: 1,
        ..context_commit_config()
    }
}

fn out_of_bounds_context_commit_config() -> CoeffFscContextCommitConfig {
    CoeffFscContextCommitConfig {
        x4: 5,
        ..context_commit_config()
    }
}

fn seeded_context_state() -> TileCoeffContextState {
    let mut state = TileCoeffContextState::new(6, 6).unwrap();
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

fn setup_level_pass<'a>(
    payload: &'a [u8],
    seg_eob: usize,
) -> Option<(
    TileCdfSubset,
    SymbolDecoder<'a>,
    FscCoeffScanWalk,
    NonZeroCoeffFscLevelPass,
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
    if start.eob_read().eob().eob() != SCAN.len() - 2 {
        return None;
    }
    let walk = walk_fsc_coeff_scan(&start, seg_eob, &SCAN).ok()?;
    let pass =
        apply_nonzero_coeff_fsc_level_pass(&mut tile, &mut symbols, start, walk.clone(), config())
            .ok()?;
    Some((tile, symbols, walk, pass))
}

fn setup_sign_pass<'a>(
    payload: &'a [u8],
    seg_eob: usize,
) -> Option<(TileCdfSubset, SymbolDecoder<'a>, NonZeroCoeffFscSignPass)> {
    let (mut tile, mut symbols, _walk, level_pass) = setup_level_pass(payload, seg_eob)?;
    let sign_pass =
        apply_nonzero_coeff_fsc_sign_pass(&mut tile, &mut symbols, level_pass, &SCAN, config())
            .ok()?;
    Some((tile, symbols, sign_pass))
}

fn run_pass(payload: &[u8], seg_eob: usize) -> Option<NonZeroCoeffFscQuantPass> {
    let (mut tile, mut symbols, _walk, level_pass) = setup_level_pass(payload, seg_eob)?;
    apply_nonzero_coeff_fsc_quant_pass(&mut tile, &mut symbols, level_pass, &SCAN, config()).ok()
}

fn find_payload(seg_eob: usize, predicate: impl Fn(&NonZeroCoeffFscQuantPass) -> bool) -> [u8; 8] {
    for first in u8::MIN..=u8::MAX {
        for second in u8::MIN..=u8::MAX {
            for suffix in PAYLOAD_SUFFIXES {
                let payload = [
                    first, second, suffix[0], suffix[1], suffix[2], suffix[3], suffix[4], suffix[5],
                ];
                let Some(pass) = run_pass(&payload, seg_eob) else {
                    continue;
                };
                if predicate(&pass) {
                    return payload;
                }
            }
        }
    }
    panic!("no coefficient FSC quant payload found");
}

fn batched_sign_then_quant_for_payload(
    payload: &[u8],
    seg_eob: usize,
) -> Option<(Vec<CoeffFscSignRead>, Vec<i32>)> {
    let (_tile, mut symbols, sign_pass) = setup_sign_pass(payload, seg_eob)?;
    let sign_reads = sign_pass.sign_reads().to_vec();
    let mut block = sign_pass.block().clone();
    let mut read_quant_state = CoeffReadQuantState::new(CoeffReadQuantConfig {
        is_hidden: false,
        allow_tcq: false,
        hr_level_avg: 0,
    });
    let mut quant_state = CoeffQuantStateAccumulator::new(CoeffQuantStateConfig {
        is_hidden: false,
        sum_abs1: 0,
        use_tcq: false,
        lossless: false,
    });
    for (index, (entry, sign)) in sign_pass
        .sign_entries()
        .iter()
        .copied()
        .zip(sign_reads.iter().copied())
        .enumerate()
    {
        let level = block.level_at(entry.row(), entry.col()).ok()?;
        let read_quant = read_quant_state
            .read_one(
                &mut symbols,
                index,
                CoeffReadQuantInput {
                    entry,
                    level,
                    max_level: NUM_BASE_LEVELS + COEFF_BASE_RANGE + 1,
                },
            )
            .ok()?;
        let write = quant_state
            .apply_entry(index, entry, level, sign.sign(), read_quant.quant_input())
            .ok()?;
        block.set_quant(write.entry().pos(), write.quant()).ok()?;
    }
    Some((sign_reads, block.quant().to_vec()))
}

fn find_order_sensitive_payload() -> (
    [u8; 8],
    NonZeroCoeffFscQuantPass,
    Vec<CoeffFscSignRead>,
    Vec<i32>,
) {
    for first in u8::MIN..=u8::MAX {
        for second in u8::MIN..=u8::MAX {
            for suffix in PAYLOAD_SUFFIXES {
                let payload = [
                    first, second, suffix[0], suffix[1], suffix[2], suffix[3], suffix[4], suffix[5],
                ];
                let Some(interleaved) = run_pass(&payload, 4) else {
                    continue;
                };
                let Some((batched_signs, batched_quant)) =
                    batched_sign_then_quant_for_payload(&payload, 4)
                else {
                    continue;
                };
                if interleaved.sign_reads() != batched_signs.as_slice()
                    || interleaved.block().quant() != batched_quant.as_slice()
                {
                    return (payload, interleaved, batched_signs, batched_quant);
                }
            }
        }
    }
    panic!("no coefficient FSC quant order-sensitive payload found");
}

#[test]
fn coefficient_fsc_quant_pass_reads_quant_and_writes_signed_quant() {
    let payload = find_payload(2, |pass| {
        pass.quant_state().dc_category() != 0
            && pass
                .read_quants()
                .iter()
                .any(|read| matches!(read.path(), CoeffReadQuantPath::Extended { .. }))
    });
    let pass = run_pass(&payload, 2).unwrap();

    assert_eq!(pass.eob_read().eob().eob(), 2);
    assert_eq!(pass.level_walk().bob(), 0);
    assert_eq!(pass.level_walk().seg_eob(), 2);
    assert_eq!(pass.level_reads().len(), 2);
    assert_eq!(pass.sign_entries().len(), 2);
    assert_eq!(pass.sign_inputs().len(), 2);
    assert_eq!(pass.read_quants().len(), 2);
    assert_eq!(pass.quant_state().writes().len(), 2);

    for ((read, sign), write) in pass
        .read_quants()
        .iter()
        .zip(pass.sign_reads())
        .zip(pass.quant_state().writes())
    {
        assert_eq!(read.quant_input().entry, sign.entry());
        assert_eq!(write.entry(), sign.entry());
        assert_eq!(write.level(), sign.level());
        assert_eq!(write.read_quant(), read.quant_input().quant);
        let magnitude = i32::try_from(read.quant_input().quant).unwrap();
        let expected = if sign.sign() { -magnitude } else { magnitude };
        assert_eq!(write.quant(), expected);
        assert_eq!(
            pass.block().quant_at(write.entry().pos()).unwrap(),
            expected
        );
    }

    let dc = pass
        .quant_state()
        .writes()
        .iter()
        .find(|write| write.entry().pos() == 0 && write.read_quant() > 0)
        .copied()
        .unwrap();
    assert_eq!(pass.quant_state().dc_category(), u8::from(!dc.sign()) + 1);
    assert!(pass.quant_state().cul_level() > 0);
}

#[test]
fn coefficient_fsc_quant_pass_preserves_zero_entries_and_quant_signs() {
    let payload = find_payload(4, |pass| {
        pass.sign_reads()[0].level() == 0
            && pass.sign_reads()[1].level() == 0
            && pass.sign_reads()[2..].iter().any(|read| read.level() != 0)
    });
    let pass = run_pass(&payload, 4).unwrap();

    assert_eq!(pass.sign_entries().len(), 4);
    assert_eq!(pass.read_quants().len(), 4);
    for index in 0..2 {
        let entry = pass.sign_entries()[index];
        assert_eq!(pass.sign_reads()[index].level(), 0);
        assert_eq!(pass.read_quants()[index].quant_input().quant, 0);
        assert_eq!(pass.block().quant_at(entry.pos()).unwrap(), 0);
        assert_eq!(
            pass.block()
                .quant_sign_at(entry.row(), entry.col())
                .unwrap(),
            0
        );
        assert_eq!(
            pass.read_quants()[index].path(),
            CoeffReadQuantPath::BelowThreshold
        );
    }
    for sign in pass.sign_reads().iter().filter(|read| read.level() != 0) {
        let expected = if sign.sign() { -1 } else { 1 };
        assert_eq!(
            pass.block()
                .quant_sign_at(sign.entry().row(), sign.entry().col())
                .unwrap(),
            expected
        );
    }
}

#[test]
fn coefficient_fsc_quant_pass_interleaves_sign_and_quant_reads() {
    let (payload, interleaved, batched_signs, batched_quant) = find_order_sensitive_payload();

    let extended_index = interleaved
        .read_quants()
        .iter()
        .position(|read| matches!(read.path(), CoeffReadQuantPath::Extended { .. }))
        .unwrap();
    assert!(
        batched_signs[extended_index + 1..]
            .iter()
            .any(|read| read.level() != 0),
        "payload did not place an extended read_quant before a later sign: {payload:?}"
    );
    assert_ne!(
        interleaved.block().quant(),
        batched_quant.as_slice(),
        "payload unexpectedly matched batch quant order: {payload:?}"
    );
}

#[test]
fn coefficient_fsc_quant_pass_with_context_commit_updates_tile_context_lines() {
    let payload = find_payload(2, |pass| {
        pass.quant_state().cul_level() > 0 && pass.quant_state().dc_category() != 0
    });
    let expected = run_pass(&payload, 2).unwrap();
    let (mut tile, mut symbols, _walk, level_pass) = setup_level_pass(&payload, 2).unwrap();
    let mut context_state = seeded_context_state();

    let pass = apply_nonzero_coeff_fsc_quant_pass_with_context_commit(
        &mut context_state,
        &mut tile,
        &mut symbols,
        level_pass,
        &SCAN,
        config(),
        context_commit_config(),
    )
    .unwrap();
    let quant_state = pass.quant_state();

    assert_eq!(pass, expected);
    assert_eq!(
        &context_state.above_level(0).unwrap()[1..3],
        &[quant_state.cul_level(); 2]
    );
    assert_eq!(
        &context_state.left_level(0).unwrap()[2..4],
        &[quant_state.cul_level(); 2]
    );
    assert_eq!(
        &context_state.above_dc(0).unwrap()[1..3],
        &[quant_state.dc_category(); 2]
    );
    assert_eq!(
        &context_state.left_dc(0).unwrap()[2..4],
        &[quant_state.dc_category(); 2]
    );
    assert_eq!(context_state.above_level(0).unwrap()[0], 1);
    assert_eq!(context_state.left_level(0).unwrap()[0], 1);
}

#[test]
fn coefficient_fsc_quant_pass_with_context_commit_preserves_context_on_pass_failure() {
    let payload = find_payload(2, |_| true);
    let (mut tile, mut symbols, _walk, level_pass) = setup_level_pass(&payload, 2).unwrap();
    let mut context_state = seeded_context_state();
    let context_before = context_state.clone();
    let consumed_before = symbols.consumed_bits();
    let symbol_count_before = symbols.symbol_count();
    let tile_before = tile.clone();

    let err = apply_nonzero_coeff_fsc_quant_pass_with_context_commit(
        &mut context_state,
        &mut tile,
        &mut symbols,
        level_pass,
        &SCAN,
        CoeffFscLevelPassConfig {
            tx_width: 4,
            ..config()
        },
        context_commit_config(),
    )
    .unwrap_err();

    assert!(matches!(
        err,
        CoeffFscQuantPassError::Sign(CoeffFscSignPassError::BlockGeometryMismatch { .. })
    ));
    assert_eq!(context_state, context_before);
    assert_eq!(symbols.consumed_bits(), consumed_before);
    assert_eq!(symbols.symbol_count(), symbol_count_before);
    assert_eq!(tile, tile_before);
}

#[test]
fn coefficient_fsc_quant_pass_with_context_commit_rejects_chroma_plane_before_pass() {
    let payload = find_payload(2, |_| true);
    let (mut tile, mut symbols, _walk, level_pass) = setup_level_pass(&payload, 2).unwrap();
    let mut context_state = seeded_context_state();
    let context_before = context_state.clone();
    let consumed_before = symbols.consumed_bits();
    let symbol_count_before = symbols.symbol_count();
    let tile_before = tile.clone();

    let err = apply_nonzero_coeff_fsc_quant_pass_with_context_commit(
        &mut context_state,
        &mut tile,
        &mut symbols,
        level_pass,
        &SCAN,
        config(),
        chroma_context_commit_config(),
    )
    .unwrap_err();

    assert!(matches!(
        err,
        CoeffFscQuantPassError::NonLumaPlane { plane: 1 }
    ));
    assert_eq!(context_state, context_before);
    assert_eq!(symbols.consumed_bits(), consumed_before);
    assert_eq!(symbols.symbol_count(), symbol_count_before);
    assert_eq!(tile, tile_before);
}

#[test]
fn coefficient_fsc_quant_pass_with_context_commit_preserves_context_on_update_failure() {
    let payload = find_payload(2, |_| true);
    let (mut tile, mut symbols, _walk, level_pass) = setup_level_pass(&payload, 2).unwrap();
    let mut context_state = seeded_context_state();
    let context_before = context_state.clone();

    let err = apply_nonzero_coeff_fsc_quant_pass_with_context_commit(
        &mut context_state,
        &mut tile,
        &mut symbols,
        level_pass,
        &SCAN,
        config(),
        out_of_bounds_context_commit_config(),
    )
    .unwrap_err();

    assert!(matches!(
        err,
        CoeffFscQuantPassError::ContextUpdate(TileCoeffStateError::ContextRangeOutOfBounds {
            context: "above",
            start: 5,
            end: 7,
            len: 6
        })
    ));
    assert_eq!(context_state, context_before);
}

#[test]
fn coefficient_fsc_quant_pass_rejects_static_config_before_consumption() {
    let payload = find_payload(2, |_| true);
    let (mut tile, mut symbols, _walk, level_pass) = setup_level_pass(&payload, 2).unwrap();
    let consumed_before = symbols.consumed_bits();
    let symbol_count_before = symbols.symbol_count();
    let tile_before = tile.clone();

    let err = apply_nonzero_coeff_fsc_quant_pass(
        &mut tile,
        &mut symbols,
        level_pass,
        &SCAN,
        CoeffFscLevelPassConfig {
            tx_width: 4,
            ..config()
        },
    )
    .unwrap_err();

    assert!(matches!(
        err,
        CoeffFscQuantPassError::Sign(CoeffFscSignPassError::BlockGeometryMismatch { .. })
    ));
    assert_eq!(symbols.consumed_bits(), consumed_before);
    assert_eq!(symbols.symbol_count(), symbol_count_before);
    assert_eq!(tile, tile_before);
}

#[test]
fn coefficient_fsc_quant_pass_rejects_short_scan_before_consumption() {
    let payload = find_payload(2, |_| true);
    let (mut tile, mut symbols, _walk, level_pass) = setup_level_pass(&payload, 2).unwrap();
    let consumed_before = symbols.consumed_bits();
    let symbol_count_before = symbols.symbol_count();
    let tile_before = tile.clone();

    let err = apply_nonzero_coeff_fsc_quant_pass(
        &mut tile,
        &mut symbols,
        level_pass,
        &SCAN[..1],
        config(),
    )
    .unwrap_err();

    assert!(matches!(
        err,
        CoeffFscQuantPassError::Sign(CoeffFscSignPassError::ScanTooShort {
            seg_eob: 2,
            scan_len: 1
        })
    ));
    assert_eq!(symbols.consumed_bits(), consumed_before);
    assert_eq!(symbols.symbol_count(), symbol_count_before);
    assert_eq!(tile, tile_before);
}

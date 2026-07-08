// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::panic)]

use splot_core::span::ByteOffset;
use splot_core::symbol::{CdfUpdateMode, SymbolDecoder, SymbolDecoderConfig};

use super::super::super::cdf::FrameCdfSubset;
use super::super::super::coeff_state::{TileCoeffContextState, TransformCoeffBlockState};
use super::super::branch::{CoeffBlockEobBranch, NonZeroCoeffBlockStartInput};
use super::super::scan_walk::{NonZeroCoeffScanWalk, walk_nonzero_coeff_scan};
use super::super::sign_symbol::{
    CoeffSignRead, CoeffSignReadInput, CoeffSignReadSource, read_nonzero_coeff_signs,
};
use super::super::*;
use super::*;

const EOB_SCAN: [u16; 4] = [0, 8, 1, 9];
const ALT_SCAN: [u16; 4] = [0, 8, 9, 1];
const PAYLOAD_SUFFIXES: [[u8; 3]; 4] = [
    [0x00, 0x00, 0x80],
    [0xff, 0x00, 0x80],
    [0x55, 0xaa, 0x80],
    [0xff, 0xff, 0x80],
];

fn symbol_decoder(payload: &[u8], mode: CdfUpdateMode) -> SymbolDecoder<'_> {
    SymbolDecoder::with_base_and_config(
        payload,
        ByteOffset::new(0),
        SymbolDecoderConfig::new().with_cdf_update_mode(mode),
    )
    .unwrap()
}

fn branch_nonzero(
    branch: CoeffBlockEobBranch,
) -> Option<super::super::branch::NonZeroCoeffBlockStart> {
    match branch {
        CoeffBlockEobBranch::AllZero(_) => None,
        CoeffBlockEobBranch::NonZero(start) => Some(start),
    }
}

fn setup_walk(payload: &[u8], scan: &[u16]) -> Option<NonZeroCoeffScanWalk> {
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();
    let mut symbols = symbol_decoder(payload, CdfUpdateMode::Enabled);
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
    if start.eob_read().eob().eob() != scan.len() {
        return None;
    }
    walk_nonzero_coeff_scan(&start, scan).ok()
}

fn find_eob_payload() -> [u8; 5] {
    for first in u8::MIN..=u8::MAX {
        for second in u8::MIN..=u8::MAX {
            for suffix in PAYLOAD_SUFFIXES {
                let payload = [first, second, suffix[0], suffix[1], suffix[2]];
                if setup_walk(&payload, &EOB_SCAN).is_some() {
                    return payload;
                }
            }
        }
    }
    panic!("no coefficient quant EOB payload found");
}

fn block_for(walk: &NonZeroCoeffScanWalk) -> TransformCoeffBlockState {
    let mut block = TransformCoeffBlockState::new(8, 8).unwrap();
    for (index, entry) in walk.entries().iter().copied().enumerate() {
        let level = match index {
            0 => 3,
            1 => 2,
            2 => 0,
            _ => 1,
        };
        block.set_level(entry.row(), entry.col(), level).unwrap();
        block
            .set_quant_sign(
                entry.row(),
                entry.col(),
                if index % 2 == 0 { 7 } else { -7 },
            )
            .unwrap();
    }
    block
}

fn signs_for(block: &TransformCoeffBlockState, walk: &NonZeroCoeffScanWalk) -> Vec<CoeffSignRead> {
    let inputs: Vec<_> = walk
        .entries()
        .iter()
        .copied()
        .map(|entry| CoeffSignReadInput {
            entry,
            source: if block.level_at(entry.row(), entry.col()).unwrap() == 0 {
                CoeffSignReadSource::None
            } else {
                CoeffSignReadSource::SignBit
            },
        })
        .collect();
    let mut tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut symbols = symbol_decoder(&[0xff, 0xff, 0x80], CdfUpdateMode::Enabled);
    read_nonzero_coeff_signs(&mut tile, &mut symbols, block, walk, &inputs).unwrap()
}

fn quant_inputs_for(walk: &NonZeroCoeffScanWalk, quants: &[u32]) -> Vec<CoeffQuantReadInput> {
    walk.entries()
        .iter()
        .copied()
        .zip(quants.iter().copied())
        .enumerate()
        .map(|(index, (entry, quant))| CoeffQuantReadInput {
            entry,
            quant,
            hr_level_avg: (index as u32 + 1) * 10,
        })
        .collect()
}

fn config() -> CoeffQuantStateConfig {
    CoeffQuantStateConfig {
        is_hidden: false,
        sum_abs1: 0,
        use_tcq: false,
        lossless: false,
    }
}

#[test]
fn coefficient_quant_state_writes_signed_quant_and_summary_state() {
    let payload = find_eob_payload();
    let walk = setup_walk(&payload, &EOB_SCAN).unwrap();
    let mut block = block_for(&walk);
    let quant_sign_before = block.quant_sign().to_vec();
    let signs = signs_for(&block, &walk);
    let inputs = quant_inputs_for(&walk, &[2, 1, 0, 3]);

    let state =
        apply_nonzero_coeff_quant_state(&mut block, &walk, &signs, &inputs, config()).unwrap();

    assert_eq!(state.writes().len(), walk.entries().len());
    for ((write, sign), input) in state.writes().iter().zip(&signs).zip(&inputs) {
        let expected = if sign.sign() {
            -(input.quant as i32)
        } else {
            input.quant as i32
        };
        assert_eq!(write.entry(), input.entry);
        assert_eq!(write.level(), sign.level());
        assert_eq!(write.sign(), sign.sign());
        assert_eq!(write.read_quant(), input.quant);
        assert_eq!(write.quant(), expected);
        assert!(write.cul_level() <= 4);
        assert!(write.dc_category() <= 2);
        assert_eq!(write.tcq_state(), 0);
        assert_eq!(write.hr_level_avg(), input.hr_level_avg);
        assert_eq!(block.quant_at(input.entry.pos()).unwrap(), expected);
    }
    let dc_entry_index = walk
        .entries()
        .iter()
        .position(|entry| entry.pos() == 0)
        .unwrap();
    let expected_dc = if signs[dc_entry_index].sign() { 1 } else { 2 };
    assert_eq!(state.cul_level(), 4);
    assert_eq!(state.dc_category(), expected_dc);
    assert_eq!(state.hr_level_avg(), 40);
    assert_eq!(block.quant_sign(), quant_sign_before);
}

#[test]
fn coefficient_quant_state_applies_hidden_parity_and_tcq() {
    let payload = find_eob_payload();
    let walk = setup_walk(&payload, &EOB_SCAN).unwrap();
    let mut block = block_for(&walk);
    let signs = signs_for(&block, &walk);
    let inputs = quant_inputs_for(&walk, &[0, 0, 0, 1]);
    let hidden_tcq = CoeffQuantStateConfig {
        is_hidden: true,
        sum_abs1: 1,
        use_tcq: true,
        lossless: false,
    };

    let state =
        apply_nonzero_coeff_quant_state(&mut block, &walk, &signs, &inputs, hidden_tcq).unwrap();

    let dc_write = state
        .writes()
        .iter()
        .find(|write| write.entry().scan_index() == 0)
        .unwrap();
    assert_eq!(dc_write.read_quant(), 1);
    assert_eq!(dc_write.quant().unsigned_abs(), 6);
    assert_eq!(
        block.quant_at(dc_write.entry().pos()).unwrap(),
        dc_write.quant()
    );
    assert_eq!(state.cul_level(), 3);
    assert_eq!(state.tcq_state(), 4);
    assert_eq!(state.hr_level_avg(), 40);
}

#[test]
fn coefficient_quant_state_preserves_quant_sign() {
    let payload = find_eob_payload();
    let walk = setup_walk(&payload, &EOB_SCAN).unwrap();
    let mut block = block_for(&walk);
    let quant_sign_before = block.quant_sign().to_vec();
    let signs = signs_for(&block, &walk);
    let inputs = quant_inputs_for(&walk, &[5, 4, 0, 2]);

    apply_nonzero_coeff_quant_state(&mut block, &walk, &signs, &inputs, config()).unwrap();

    assert_eq!(block.quant_sign(), quant_sign_before);
}

#[test]
fn coefficient_quant_state_rejects_mismatches_before_mutation() {
    let payload = find_eob_payload();
    let walk = setup_walk(&payload, &EOB_SCAN).unwrap();
    let alt_walk = setup_walk(&payload, &ALT_SCAN).unwrap();
    let block = block_for(&walk);
    let signs = signs_for(&block, &walk);
    let inputs = quant_inputs_for(&walk, &[2, 1, 0, 3]);

    let mut count_block = block.clone();
    let count_before = count_block.clone();
    let mut short_inputs = inputs.clone();
    short_inputs.pop();
    let err =
        apply_nonzero_coeff_quant_state(&mut count_block, &walk, &signs, &short_inputs, config())
            .unwrap_err();
    assert!(matches!(
        err,
        CoeffQuantStateWriteError::InputCountMismatch {
            inputs: 3,
            entries: 4
        }
    ));
    assert_eq!(count_block, count_before);

    let mut sign_block = block.clone();
    let sign_before = sign_block.clone();
    let err =
        apply_nonzero_coeff_quant_state(&mut sign_block, &alt_walk, &signs, &inputs, config())
            .unwrap_err();
    assert!(matches!(
        err,
        CoeffQuantStateWriteError::SignEntryMismatch { index: 0, .. }
    ));
    assert_eq!(sign_block, sign_before);
}

#[test]
fn coefficient_quant_state_requires_hidden_parity_sign() {
    let payload = find_eob_payload();
    let walk = setup_walk(&payload, &EOB_SCAN).unwrap();
    let mut block = block_for(&walk);
    let hidden_entry = walk
        .entries()
        .iter()
        .copied()
        .find(|entry| entry.scan_index() == 0)
        .unwrap();
    block
        .set_level(hidden_entry.row(), hidden_entry.col(), 0)
        .unwrap();
    let before = block.clone();
    let signs = signs_for(&block, &walk);
    let inputs = quant_inputs_for(&walk, &[0, 0, 0, 1]);
    let hidden = CoeffQuantStateConfig {
        is_hidden: true,
        sum_abs1: 1,
        use_tcq: false,
        lossless: false,
    };

    let err =
        apply_nonzero_coeff_quant_state(&mut block, &walk, &signs, &inputs, hidden).unwrap_err();

    assert!(matches!(
        err,
        CoeffQuantStateWriteError::HiddenParityMissingSign { entry, .. }
            if entry == hidden_entry
    ));
    assert_eq!(block, before);
}

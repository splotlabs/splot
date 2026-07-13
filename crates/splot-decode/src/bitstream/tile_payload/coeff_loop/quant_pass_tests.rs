// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use splot_core::span::ByteOffset;
use splot_core::symbol::{CdfUpdateMode, SymbolDecoderConfig};

use super::super::read_quant::CoeffReadQuantPath;
use super::*;

fn symbol_decoder(payload: &[u8]) -> SymbolDecoder<'_> {
    SymbolDecoder::with_base_and_config(
        payload,
        ByteOffset::new(0),
        SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Enabled),
    )
    .unwrap()
}

fn walk() -> NonZeroCoeffScanWalk<'static> {
    NonZeroCoeffScanWalk::from_entries_for_test(vec![
        CoeffScanEntry::for_test(1, 1, 0, 1),
        CoeffScanEntry::for_test(0, 0, 0, 0),
    ])
}

fn block_for(walk: &NonZeroCoeffScanWalk<'_>, levels: &[u32]) -> TransformCoeffBlockState {
    block_for_extent(2, 2, walk, levels)
}

fn block_for_extent(
    width: usize,
    height: usize,
    walk: &NonZeroCoeffScanWalk<'_>,
    levels: &[u32],
) -> TransformCoeffBlockState {
    let mut block = TransformCoeffBlockState::new(width, height).unwrap();
    for (entry, level) in walk.entries().zip(levels.iter().copied()) {
        block.set_level(entry.row(), entry.col(), level).unwrap();
        block.set_quant_sign(entry.row(), entry.col(), 11).unwrap();
    }
    block
}

fn signs_for(
    walk: &NonZeroCoeffScanWalk<'_>,
    levels: &[u32],
    signs: &[bool],
) -> Vec<CoeffSignRead> {
    walk.entries()
        .zip(levels.iter().copied())
        .zip(signs.iter().copied())
        .map(|((entry, level), sign)| {
            CoeffSignRead::for_test(
                entry,
                level,
                CoeffSignReadSymbol::SignBit { bit: sign },
                sign,
            )
        })
        .collect()
}

fn inputs_for(walk: &NonZeroCoeffScanWalk<'_>, max_levels: &[u32]) -> Vec<CoeffQuantPassInput> {
    walk.entries()
        .zip(max_levels.iter().copied())
        .map(|(entry, max_level)| CoeffQuantPassInput { entry, max_level })
        .collect()
}

fn config() -> CoeffQuantPassConfig {
    CoeffQuantPassConfig {
        is_hidden: false,
        sum_abs1: 0,
        use_tcq: false,
        lossless: false,
        hr_level_avg: 16,
    }
}

fn max_level_config() -> CoeffQuantPassMaxLevelConfig {
    CoeffQuantPassMaxLevelConfig {
        plane: 0,
        tx_class: CoeffTransformClass::TwoD,
    }
}

#[test]
fn coefficient_quant_pass_reads_quant_and_writes_signed_quant() {
    let walk = walk();
    let levels = [3, 2];
    let signs = signs_for(&walk, &levels, &[false, true]);
    let inputs = inputs_for(&walk, &[3, 5]);
    let mut block = block_for(&walk, &levels);
    let quant_sign_before = block.quant_sign().to_vec();
    let mut symbols = symbol_decoder(&[0b0011_0100, 0x80]);

    let pass =
        apply_nonzero_coeff_quant_pass(&mut symbols, &mut block, &walk, &signs, &inputs, config())
            .unwrap();

    assert_eq!(pass.read_quants().len(), 2);
    assert_eq!(
        pass.read_quants()[0].path(),
        CoeffReadQuantPath::Extended {
            m: 4,
            k: 5,
            c_max: 6,
            q: 2,
            length: 4,
            x_base: 32,
            coeff_rem: 10,
            x: 42,
        }
    );
    assert_eq!(pass.read_quants()[0].quant_input().quant, 45);
    assert_eq!(
        pass.read_quants()[1].path(),
        CoeffReadQuantPath::BelowThreshold
    );
    assert_eq!(pass.read_quants()[1].quant_input().quant, 2);
    assert_eq!(pass.quant_state().hr_level_avg(), 29);
    let mut entries = walk.entries();
    let first = entries.next().unwrap();
    let second = entries.next().unwrap();
    assert_eq!(block.quant_at(first.pos()).unwrap(), 45);
    assert_eq!(block.quant_at(second.pos()).unwrap(), -2);
    assert_eq!(pass.quant_state().dc_category(), 1);
    assert_eq!(block.quant_sign(), quant_sign_before);
    assert_eq!(symbols.symbol_count(), 7);
}

#[test]
fn coefficient_quant_pass_derives_low_frequency_max_levels() {
    let walk = NonZeroCoeffScanWalk::from_entries_for_test(vec![
        CoeffScanEntry::for_test(1, 1, 0, 1),
        CoeffScanEntry::for_test(0, 15, 3, 3),
    ]);
    let levels = [7, 5];
    let signs = signs_for(&walk, &levels, &[false, false]);
    let mut block = block_for_extent(4, 4, &walk, &levels);
    let mut symbols = symbol_decoder(&[0xff, 0x80]);
    let consumed_before = symbols.consumed_bits();

    let pass = apply_nonzero_coeff_quant_pass_with_derived_max_levels(
        &mut symbols,
        &mut block,
        &walk,
        &signs,
        max_level_config(),
        config(),
    )
    .unwrap();

    assert_eq!(pass.read_quants().len(), 2);
    assert_eq!(
        pass.read_quants()[0].path(),
        CoeffReadQuantPath::BelowThreshold
    );
    assert_eq!(
        pass.read_quants()[1].path(),
        CoeffReadQuantPath::BelowThreshold
    );
    assert_eq!(block.quant_at(1).unwrap(), 7);
    assert_eq!(block.quant_at(15).unwrap(), 5);
    assert_eq!(symbols.consumed_bits(), consumed_before);
}

#[test]
fn coefficient_quant_pass_derives_hidden_final_max_level() {
    let entry = CoeffScanEntry::for_test(0, 0, 0, 0);
    let walk = NonZeroCoeffScanWalk::from_entries_for_test(vec![entry]);
    let levels = [3];
    let signs = signs_for(&walk, &levels, &[false]);
    let mut block = block_for(&walk, &levels);
    let mut symbols = symbol_decoder(&[0b1000_0000]);
    let config = CoeffQuantPassConfig {
        is_hidden: true,
        sum_abs1: 1,
        ..config()
    };

    let pass = apply_nonzero_coeff_quant_pass_with_derived_max_levels(
        &mut symbols,
        &mut block,
        &walk,
        &signs,
        max_level_config(),
        config,
    )
    .unwrap();

    assert_eq!(
        pass.read_quants()[0].path(),
        CoeffReadQuantPath::Extended {
            m: 3,
            k: 4,
            c_max: 6,
            q: 0,
            length: 3,
            x_base: 0,
            coeff_rem: 0,
            x: 0,
        }
    );
    assert_eq!(pass.read_quants()[0].quant_input().quant, 3);
    assert_eq!(block.quant_at(entry.pos()).unwrap(), 7);
    assert_eq!(symbols.symbol_count(), 4);
}

#[test]
fn coefficient_quant_pass_applies_hidden_parity_consistently() {
    let entry = CoeffScanEntry::for_test(0, 0, 0, 0);
    let walk = NonZeroCoeffScanWalk::from_entries_for_test(vec![entry]);
    let levels = [2];
    let signs = signs_for(&walk, &levels, &[false]);
    let inputs = inputs_for(&walk, &[3]);
    let mut block = block_for(&walk, &levels);
    let mut symbols = symbol_decoder(&[0b1000_0100, 0x80]);
    let consumed_before = symbols.consumed_bits();
    let config = CoeffQuantPassConfig {
        is_hidden: true,
        sum_abs1: 1,
        use_tcq: false,
        lossless: false,
        hr_level_avg: 64,
    };

    let pass =
        apply_nonzero_coeff_quant_pass(&mut symbols, &mut block, &walk, &signs, &inputs, config)
            .unwrap();

    assert_eq!(
        pass.read_quants()[0].path(),
        CoeffReadQuantPath::BelowThreshold
    );
    assert_eq!(pass.read_quants()[0].quant_input().quant, 2);
    assert_eq!(pass.read_quants()[0].quant_input().hr_level_avg, 64);
    assert_eq!(pass.quant_state().tcq_state(), 0);
    assert_eq!(pass.quant_state().cul_level(), 4);
    assert_eq!(pass.quant_state().dc_category(), 2);
    assert_eq!(block.quant_at(entry.pos()).unwrap(), 5);
    assert_eq!(symbols.consumed_bits(), consumed_before);
}

#[test]
fn coefficient_quant_pass_applies_tcq_consistently() {
    let entry = CoeffScanEntry::for_test(0, 0, 0, 0);
    let walk = NonZeroCoeffScanWalk::from_entries_for_test(vec![entry]);
    let levels = [1];
    let signs = signs_for(&walk, &levels, &[false]);
    let inputs = inputs_for(&walk, &[3]);
    let mut block = block_for(&walk, &levels);
    let mut symbols = symbol_decoder(&[0xff, 0x80]);
    let consumed_before = symbols.consumed_bits();
    let config = CoeffQuantPassConfig {
        use_tcq: true,
        ..config()
    };

    let pass =
        apply_nonzero_coeff_quant_pass(&mut symbols, &mut block, &walk, &signs, &inputs, config)
            .unwrap();

    assert_eq!(
        pass.read_quants()[0].path(),
        CoeffReadQuantPath::BelowThreshold
    );
    assert_eq!(pass.read_quants()[0].quant_input().quant, 1);
    assert_eq!(pass.quant_state().tcq_state(), 4);
    assert_eq!(pass.quant_state().cul_level(), 1);
    assert_eq!(pass.quant_state().dc_category(), 2);
    assert_eq!(block.quant_at(entry.pos()).unwrap(), 2);
    assert_eq!(symbols.consumed_bits(), consumed_before);
}

#[test]
fn coefficient_quant_pass_allows_hidden_dc_without_parity_sign_when_sum_abs1_zero() {
    let entry = CoeffScanEntry::for_test(0, 0, 0, 0);
    let walk = NonZeroCoeffScanWalk::from_entries_for_test(vec![entry]);
    let levels = [0];
    let signs = vec![CoeffSignRead::for_test(
        entry,
        0,
        CoeffSignReadSymbol::None,
        false,
    )];
    let inputs = inputs_for(&walk, &[5]);
    let mut block = block_for(&walk, &levels);
    let mut symbols = symbol_decoder(&[0xff, 0x80]);
    let consumed_before = symbols.consumed_bits();
    let config = CoeffQuantPassConfig {
        is_hidden: true,
        sum_abs1: 0,
        ..config()
    };

    let pass =
        apply_nonzero_coeff_quant_pass(&mut symbols, &mut block, &walk, &signs, &inputs, config)
            .unwrap();

    assert_eq!(
        pass.read_quants()[0].path(),
        CoeffReadQuantPath::BelowThreshold
    );
    assert_eq!(pass.read_quants()[0].quant_input().quant, 0);
    assert_eq!(pass.quant_state().cul_level(), 0);
    assert_eq!(pass.quant_state().dc_category(), 0);
    assert_eq!(block.quant_at(entry.pos()).unwrap(), 0);
    assert_eq!(symbols.consumed_bits(), consumed_before);
}

#[test]
fn coefficient_quant_pass_derived_max_levels_rejects_bad_facts_before_consumption() {
    let walk = walk();
    let levels = [3, 2];
    let signs = signs_for(&walk, &levels, &[false, true]);
    let mut block = block_for(&walk, &levels);
    let before = block.clone();
    let mut symbols = symbol_decoder(&[0xff, 0x80]);
    let consumed_before = symbols.consumed_bits();
    let config = CoeffQuantPassConfig {
        is_hidden: true,
        use_tcq: true,
        ..config()
    };

    let err = apply_nonzero_coeff_quant_pass_with_derived_max_levels(
        &mut symbols,
        &mut block,
        &walk,
        &signs,
        max_level_config(),
        config,
    )
    .unwrap_err();

    assert!(matches!(
        err,
        CoeffQuantPassError::InconsistentHiddenParityConfig {
            use_tcq: true,
            lossless: false,
        }
    ));
    assert_eq!(symbols.consumed_bits(), consumed_before);
    assert_eq!(block, before);
}

#[test]
fn coefficient_quant_pass_rejects_bad_facts_before_consumption() {
    let walk = walk();
    let levels = [3, 2];
    let signs = signs_for(&walk, &levels, &[false, true]);
    let inputs = inputs_for(&walk, &[3, 5]);
    let block = block_for(&walk, &levels);

    let mut mismatch_block = block.clone();
    let mismatch_before = mismatch_block.clone();
    let mut mismatch_signs = signs.clone();
    mismatch_signs[0] = CoeffSignRead::for_test(
        walk.entries().nth(1).unwrap(),
        levels[0],
        CoeffSignReadSymbol::SignBit { bit: false },
        false,
    );
    let mut mismatch_symbols = symbol_decoder(&[0xff, 0x80]);
    let consumed_before = mismatch_symbols.consumed_bits();
    let err = apply_nonzero_coeff_quant_pass(
        &mut mismatch_symbols,
        &mut mismatch_block,
        &walk,
        &mismatch_signs,
        &inputs,
        config(),
    )
    .unwrap_err();
    assert!(matches!(
        err,
        CoeffQuantPassError::SignEntryMismatch { index: 0, .. }
    ));
    assert_eq!(mismatch_symbols.consumed_bits(), consumed_before);
    assert_eq!(mismatch_block, mismatch_before);

    let mut max_block = block.clone();
    let max_before = max_block.clone();
    let mut max_symbols = symbol_decoder(&[0xff, 0x80]);
    let invalid_inputs = inputs_for(&walk, &[0, 5]);
    let invalid_config = CoeffQuantPassConfig {
        use_tcq: true,
        ..config()
    };
    let consumed_before = max_symbols.consumed_bits();
    let err = apply_nonzero_coeff_quant_pass(
        &mut max_symbols,
        &mut max_block,
        &walk,
        &signs,
        &invalid_inputs,
        invalid_config,
    )
    .unwrap_err();
    assert!(matches!(
        err,
        CoeffQuantPassError::InvalidMaxLevel {
            index: 0,
            max_level: 0,
            use_tcq: true,
        }
    ));
    assert_eq!(max_symbols.consumed_bits(), consumed_before);
    assert_eq!(max_block, max_before);

    let mut hidden_tcq_block = block.clone();
    let hidden_tcq_before = hidden_tcq_block.clone();
    let mut hidden_tcq_symbols = symbol_decoder(&[0xff, 0x80]);
    let hidden_tcq_config = CoeffQuantPassConfig {
        is_hidden: true,
        use_tcq: true,
        ..config()
    };
    let consumed_before = hidden_tcq_symbols.consumed_bits();
    let err = apply_nonzero_coeff_quant_pass(
        &mut hidden_tcq_symbols,
        &mut hidden_tcq_block,
        &walk,
        &signs,
        &inputs,
        hidden_tcq_config,
    )
    .unwrap_err();
    assert!(matches!(
        err,
        CoeffQuantPassError::InconsistentHiddenParityConfig {
            use_tcq: true,
            lossless: false,
        }
    ));
    assert_eq!(hidden_tcq_symbols.consumed_bits(), consumed_before);
    assert_eq!(hidden_tcq_block, hidden_tcq_before);

    let mut hidden_lossless_block = block.clone();
    let hidden_lossless_before = hidden_lossless_block.clone();
    let mut hidden_lossless_symbols = symbol_decoder(&[0xff, 0x80]);
    let hidden_lossless_config = CoeffQuantPassConfig {
        is_hidden: true,
        lossless: true,
        ..config()
    };
    let consumed_before = hidden_lossless_symbols.consumed_bits();
    let err = apply_nonzero_coeff_quant_pass(
        &mut hidden_lossless_symbols,
        &mut hidden_lossless_block,
        &walk,
        &signs,
        &inputs,
        hidden_lossless_config,
    )
    .unwrap_err();
    assert!(matches!(
        err,
        CoeffQuantPassError::InconsistentHiddenParityConfig {
            use_tcq: false,
            lossless: true,
        }
    ));
    assert_eq!(hidden_lossless_symbols.consumed_bits(), consumed_before);
    assert_eq!(hidden_lossless_block, hidden_lossless_before);
}

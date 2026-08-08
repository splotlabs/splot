// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use super::*;

fn entry(scan_index: usize, pos: usize) -> CoeffScanEntry {
    CoeffScanEntry::new(scan_index, pos, pos / 8, pos % 8)
}

const fn input(quant: u32) -> CoeffQuantReadInput {
    CoeffQuantReadInput { quant }
}

#[test]
fn accumulator_applies_sign_and_updates_context_summary() {
    let dc = entry(0, 0);
    let mut state = CoeffQuantStateAccumulator::new(CoeffQuantStateConfig {
        is_hidden: false,
        sum_abs1: 0,
        use_tcq: false,
        lossless: false,
    });
    let write = state.apply_entry(0, dc, true, input(3)).unwrap();
    let summary = NonZeroCoeffQuantState::from_accumulator(state);

    assert_eq!(write.entry(), dc);
    assert_eq!(write.quant(), -3);
    assert_eq!(summary.cul_level(), 3);
    assert_eq!(summary.dc_category(), 1);
}

#[test]
fn hidden_parity_and_tcq_adjust_quant() {
    let dc = entry(0, 0);
    let mut hidden = CoeffQuantStateAccumulator::new(CoeffQuantStateConfig {
        is_hidden: true,
        sum_abs1: 1,
        use_tcq: false,
        lossless: false,
    });
    assert_eq!(
        hidden.apply_entry(0, dc, false, input(2)).unwrap().quant(),
        5
    );

    let mut tcq = CoeffQuantStateAccumulator::new(CoeffQuantStateConfig {
        is_hidden: false,
        sum_abs1: 0,
        use_tcq: true,
        lossless: false,
    });
    assert_eq!(tcq.apply_entry(0, dc, false, input(1)).unwrap().quant(), 2);
}

#[test]
fn hidden_parity_overflow_is_reported() {
    let dc = entry(0, 0);
    let mut state = CoeffQuantStateAccumulator::new(CoeffQuantStateConfig {
        is_hidden: true,
        sum_abs1: 1,
        use_tcq: false,
        lossless: false,
    });
    assert!(matches!(
        state.apply_entry(0, dc, false, input(u32::MAX)),
        Err(CoeffQuantStateWriteError::QuantOverflow {
            operation: "2 * quant + sumAbs1",
            ..
        })
    ));
}

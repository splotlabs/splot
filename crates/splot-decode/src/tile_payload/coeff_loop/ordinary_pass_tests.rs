// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::panic)]

use splot_core::span::ByteOffset;
use splot_core::symbol::{CdfUpdateMode, SymbolBitPosition, SymbolDecoder, SymbolDecoderConfig};

use super::super::cdf::{CoeffCdfSelector, FrameCdfSubset, TileCdfSubset};
use super::super::coeff_state::TileCoeffContextState;
use super::base_level_pass::{
    CoeffBaseDerivedLevelPassConfig, CoeffBaseDerivedLevelPassError,
    NonZeroCoeffBaseDerivedLevelPass,
};
use super::base_symbol::{
    CoeffBaseRangeRead, CoeffBaseSymbolRead, CoeffBaseSymbolReadError, CoeffBaseSymbolReadInput,
    CoeffBaseSymbolSource, read_nonzero_coeff_base_symbols,
};
use super::branch::{CoeffBlockEobBranch, NonZeroCoeffBlockStart, NonZeroCoeffBlockStartInput};
use super::level_state::apply_nonzero_coeff_base_levels;
use super::max_level::CoeffTransformClass;
use super::ordinary_pass::{
    CoeffOrdinaryDerivedBasePassInput, CoeffOrdinaryPassError, CoeffOrdinaryPassInput,
    NonZeroCoeffOrdinaryDerivedBasePass, NonZeroCoeffOrdinaryPass,
    apply_nonzero_coeff_ordinary_pass, apply_nonzero_coeff_ordinary_pass_with_derived_base,
};
use super::quant_pass::{
    CoeffQuantPassConfig, CoeffQuantPassMaxLevelConfig,
    apply_nonzero_coeff_quant_pass_with_derived_max_levels,
};
use super::scan_walk::{NonZeroCoeffScanWalk, walk_nonzero_coeff_scan};
use super::sign_symbol::{
    CoeffDcSignSelector, CoeffSignCdfSyntax, CoeffSignRead, CoeffSignReadInput,
    CoeffSignReadSource, read_nonzero_coeff_signs,
};
use super::*;

const BASE_LEVELS: u32 = 2;
const SCAN: [u16; 4] = [0, 8, 1, 9];
const DC_LAST_HIDDEN_SCAN: [u16; 5] = [0, 1, 8, 9, 2];
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
    Some((tile, symbols, branch_nonzero(branch)?))
}

fn setup_start_and_walk<'a>(
    payload: &'a [u8],
    scan: &[u16],
) -> Option<(
    TileCdfSubset,
    SymbolDecoder<'a>,
    NonZeroCoeffBlockStart,
    NonZeroCoeffScanWalk,
)> {
    let (tile, symbols, start) = setup_start(payload)?;
    if start.eob_read().eob().eob() != scan.len() {
        return None;
    }
    let walk = walk_nonzero_coeff_scan(&start, scan).ok()?;
    Some((tile, symbols, start, walk))
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

fn dc_sign_selector() -> CoeffDcSignSelector {
    CoeffDcSignSelector {
        coeff_cdf_q_ctx: 0,
        plane_type: 0,
        group: 0,
        ctx: 0,
    }
}

fn base_inputs_for(walk: &NonZeroCoeffScanWalk) -> Vec<CoeffBaseSymbolReadInput> {
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

fn sign_inputs_for(reads: &[CoeffBaseSymbolRead]) -> Vec<CoeffSignReadInput> {
    reads
        .iter()
        .map(|read| {
            let entry = read.entry();
            let source = if read.level() == 0 {
                CoeffSignReadSource::None
            } else if entry.pos() == 0 {
                CoeffSignReadSource::Cdf {
                    syntax: CoeffSignCdfSyntax::DcSign,
                    selector: dc_sign_selector(),
                }
            } else {
                CoeffSignReadSource::SignBit
            };
            CoeffSignReadInput { entry, source }
        })
        .collect()
}

fn sign_bit_inputs_for(walk: &NonZeroCoeffScanWalk) -> Vec<CoeffSignReadInput> {
    walk.entries()
        .iter()
        .copied()
        .map(|entry| CoeffSignReadInput {
            entry,
            source: CoeffSignReadSource::SignBit,
        })
        .collect()
}

fn max_level_config() -> CoeffQuantPassMaxLevelConfig {
    CoeffQuantPassMaxLevelConfig {
        plane: 0,
        tx_class: CoeffTransformClass::TwoD,
    }
}

fn order_sensitive_max_level_config() -> CoeffQuantPassMaxLevelConfig {
    CoeffQuantPassMaxLevelConfig {
        plane: 1,
        tx_class: CoeffTransformClass::TwoD,
    }
}

fn luma_base_config(parity_hiding: bool, use_tcq: bool) -> CoeffBaseDerivedLevelPassConfig {
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

fn quant_config() -> CoeffQuantPassConfig {
    CoeffQuantPassConfig {
        is_hidden: false,
        sum_abs1: 0,
        use_tcq: false,
        lossless: false,
        hr_level_avg: 16,
    }
}

fn order_sensitive_quant_config() -> CoeffQuantPassConfig {
    CoeffQuantPassConfig {
        use_tcq: true,
        hr_level_avg: 0,
        ..quant_config()
    }
}

fn payload_from(first: u8, second: u8, suffix: [u8; 3]) -> [u8; 12] {
    [
        first, second, suffix[0], suffix[1], suffix[2], 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x80,
    ]
}

fn base_reads_for_payload(
    payload: &[u8],
) -> Option<(NonZeroCoeffScanWalk, Vec<CoeffBaseSymbolRead>)> {
    let (mut tile, mut symbols, _start, walk) = setup_start_and_walk(payload, &SCAN)?;
    let inputs = base_inputs_for(&walk);
    let reads = read_nonzero_coeff_base_symbols(&mut tile, &mut symbols, &walk, &inputs).ok()?;
    Some((walk, reads))
}

fn derived_first_pass_for_payload(
    payload: &[u8],
    scan: &[u16],
    config: CoeffBaseDerivedLevelPassConfig,
) -> Option<NonZeroCoeffBaseDerivedLevelPass> {
    let (mut tile, mut symbols, start, walk) = setup_start_and_walk(payload, scan)?;
    super::base_level_pass::apply_nonzero_coeff_base_derived_level_pass(
        &mut tile,
        &mut symbols,
        start,
        walk,
        config,
    )
    .ok()
}

fn find_payload() -> [u8; 12] {
    for first in u8::MIN..=u8::MAX {
        for second in u8::MIN..=u8::MAX {
            for suffix in PAYLOAD_SUFFIXES {
                let payload = payload_from(first, second, suffix);
                let Some((_walk, reads)) = base_reads_for_payload(&payload) else {
                    continue;
                };
                if reads.iter().any(|read| read.level() > 0) {
                    return payload;
                }
            }
        }
    }
    panic!("no ordinary coefficient pass payload found");
}

fn derived_pass_for_payload(
    payload: &[u8],
    scan: &[u16],
    config: CoeffBaseDerivedLevelPassConfig,
    lossless: bool,
) -> Option<NonZeroCoeffOrdinaryDerivedBasePass> {
    let (mut tile, mut symbols, start, walk) = setup_start_and_walk(payload, scan)?;
    let sign_inputs = sign_bit_inputs_for(&walk);
    apply_nonzero_coeff_ordinary_pass_with_derived_base(
        &mut tile,
        &mut symbols,
        CoeffOrdinaryDerivedBasePassInput {
            start,
            scan,
            base_config: config,
            sign_inputs: &sign_inputs,
            lossless,
        },
    )
    .ok()
}

fn find_derived_payload(
    scan: &[u16],
    config: CoeffBaseDerivedLevelPassConfig,
    predicate: impl Fn(&NonZeroCoeffOrdinaryDerivedBasePass) -> bool,
) -> [u8; 12] {
    for first in u8::MIN..=u8::MAX {
        for second in u8::MIN..=u8::MAX {
            for suffix in PAYLOAD_SUFFIXES {
                let payload = payload_from(first, second, suffix);
                let Some(pass) = derived_pass_for_payload(&payload, scan, config, false) else {
                    continue;
                };
                if predicate(&pass) {
                    return payload;
                }
            }
        }
    }
    panic!("no ordinary derived-base coefficient pass payload found");
}

fn explicit_pass_from_derived(
    payload: &[u8],
    scan: &[u16],
    derived_first_pass: &NonZeroCoeffBaseDerivedLevelPass,
    config: CoeffBaseDerivedLevelPassConfig,
    sign_inputs: &[CoeffSignReadInput],
    lossless: bool,
) -> NonZeroCoeffOrdinaryPass {
    let (mut tile, mut symbols, start) = setup_start(payload).unwrap();
    let first_pass = derived_first_pass.first_pass();
    apply_nonzero_coeff_ordinary_pass(
        &mut tile,
        &mut symbols,
        CoeffOrdinaryPassInput {
            start,
            scan,
            base_inputs: derived_first_pass.derived_inputs(),
            sign_inputs,
            max_level_config: CoeffQuantPassMaxLevelConfig {
                plane: config.plane,
                tx_class: config.tx_class,
            },
            quant_config: CoeffQuantPassConfig {
                is_hidden: first_pass.is_hidden(),
                sum_abs1: first_pass.sum_abs1(),
                use_tcq: config.use_tcq,
                lossless,
                hr_level_avg: 99,
            },
        },
    )
    .unwrap()
}

fn after_base_prefix(
    payload: &[u8],
    base_inputs: &[CoeffBaseSymbolReadInput],
) -> (TileCdfSubset, SymbolBitPosition, u64) {
    let (mut tile, mut symbols, start) = setup_start(payload).unwrap();
    let walk = walk_nonzero_coeff_scan(&start, &SCAN).unwrap();
    read_nonzero_coeff_base_symbols(&mut tile, &mut symbols, &walk, base_inputs).unwrap();
    (tile, symbols.consumed_bits(), symbols.symbol_count())
}

fn ordinary_pass_for_payload(payload: &[u8]) -> Option<NonZeroCoeffOrdinaryPass> {
    let (walk, _reads) = base_reads_for_payload(payload)?;
    let base_inputs = base_inputs_for(&walk);
    let sign_inputs = sign_bit_inputs_for(&walk);
    let (mut tile, mut symbols, start) = setup_start(payload)?;

    apply_nonzero_coeff_ordinary_pass(
        &mut tile,
        &mut symbols,
        CoeffOrdinaryPassInput {
            start,
            scan: &SCAN,
            base_inputs: &base_inputs,
            sign_inputs: &sign_inputs,
            max_level_config: order_sensitive_max_level_config(),
            quant_config: order_sensitive_quant_config(),
        },
    )
    .ok()
}

fn batched_sign_then_quant_for_payload(payload: &[u8]) -> Option<(Vec<CoeffSignRead>, Vec<i32>)> {
    let (mut tile, mut symbols, start) = setup_start(payload)?;
    let walk = walk_nonzero_coeff_scan(&start, &SCAN).ok()?;
    let base_inputs = base_inputs_for(&walk);
    let base_reads =
        read_nonzero_coeff_base_symbols(&mut tile, &mut symbols, &walk, &base_inputs).ok()?;
    let level_state = apply_nonzero_coeff_base_levels(start, &walk, &base_reads).ok()?;
    let sign_inputs = sign_bit_inputs_for(&walk);
    let sign_reads = read_nonzero_coeff_signs(
        &mut tile,
        &mut symbols,
        level_state.block(),
        &walk,
        &sign_inputs,
    )
    .ok()?;
    let (_eob_read, mut block) = level_state.into_parts();
    apply_nonzero_coeff_quant_pass_with_derived_max_levels(
        &mut symbols,
        &mut block,
        &walk,
        &sign_reads,
        order_sensitive_max_level_config(),
        order_sensitive_quant_config(),
    )
    .ok()?;

    Some((sign_reads, block.quant().to_vec()))
}

fn find_order_sensitive_payload() -> (
    [u8; 12],
    NonZeroCoeffOrdinaryPass,
    Vec<CoeffSignRead>,
    Vec<i32>,
) {
    for first in u8::MIN..=u8::MAX {
        for second in u8::MIN..=u8::MAX {
            for suffix in PAYLOAD_SUFFIXES {
                let payload = payload_from(first, second, suffix);
                let Some(interleaved) = ordinary_pass_for_payload(&payload) else {
                    continue;
                };
                let Some((batched_signs, batched_quant)) =
                    batched_sign_then_quant_for_payload(&payload)
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
    panic!("no ordinary coefficient pass order-sensitive payload found");
}

#[test]
fn coefficient_ordinary_pass_composes_level_sign_and_quant_writes() {
    let payload = find_payload();
    let (expected_walk, expected_reads) = base_reads_for_payload(&payload).unwrap();
    let base_inputs = base_inputs_for(&expected_walk);
    let sign_inputs = sign_inputs_for(&expected_reads);
    let (mut tile, mut symbols, start) = setup_start(&payload).unwrap();

    let pass = apply_nonzero_coeff_ordinary_pass(
        &mut tile,
        &mut symbols,
        CoeffOrdinaryPassInput {
            start,
            scan: &SCAN,
            base_inputs: &base_inputs,
            sign_inputs: &sign_inputs,
            max_level_config: max_level_config(),
            quant_config: quant_config(),
        },
    )
    .unwrap();

    assert_eq!(pass.eob_read().eob().eob(), SCAN.len());
    assert_eq!(pass.walk(), &expected_walk);
    assert_eq!(pass.base_reads(), expected_reads.as_slice());
    assert_eq!(pass.sign_reads().len(), expected_reads.len());
    assert_eq!(pass.quant_pass().read_quants().len(), expected_reads.len());
    assert_eq!(
        pass.quant_pass().quant_state().writes().len(),
        expected_reads.len()
    );
    for read in pass.base_reads() {
        let entry = read.entry();
        assert_eq!(
            pass.block().level_at(entry.row(), entry.col()).unwrap(),
            read.level()
        );
    }
    for write in pass.quant_pass().quant_state().writes() {
        assert_eq!(
            pass.block().quant_at(write.entry().pos()).unwrap(),
            write.quant()
        );
    }
    assert!(pass.block().quant().iter().any(|quant| *quant != 0));
}

#[test]
fn coefficient_ordinary_pass_interleaves_sign_and_quant_reads() {
    let (payload, interleaved, batched_signs, batched_quant) = find_order_sensitive_payload();

    assert_ne!(
        interleaved.sign_reads(),
        batched_signs.as_slice(),
        "payload unexpectedly matched batch sign order: {payload:?}"
    );
    assert_ne!(
        interleaved.block().quant(),
        batched_quant.as_slice(),
        "payload unexpectedly matched batch quant order: {payload:?}"
    );
}

#[test]
fn coefficient_ordinary_pass_rejects_base_inputs_before_base_consumption() {
    let payload = find_payload();
    let (walk, reads) = base_reads_for_payload(&payload).unwrap();
    let mut base_inputs = base_inputs_for(&walk);
    base_inputs.pop();
    let sign_inputs = sign_inputs_for(&reads);
    let (mut tile, mut symbols, start) = setup_start(&payload).unwrap();
    let tile_before = tile.clone();
    let consumed_before = symbols.consumed_bits();
    let symbol_count_before = symbols.symbol_count();

    let err = apply_nonzero_coeff_ordinary_pass(
        &mut tile,
        &mut symbols,
        CoeffOrdinaryPassInput {
            start,
            scan: &SCAN,
            base_inputs: &base_inputs,
            sign_inputs: &sign_inputs,
            max_level_config: max_level_config(),
            quant_config: quant_config(),
        },
    )
    .unwrap_err();

    assert!(matches!(
        err,
        CoeffOrdinaryPassError::Base(CoeffBaseSymbolReadError::InputCountMismatch {
            inputs: 3,
            entries: 4
        })
    ));
    assert_eq!(tile, tile_before);
    assert_eq!(symbols.consumed_bits(), consumed_before);
    assert_eq!(symbols.symbol_count(), symbol_count_before);
}

#[test]
fn coefficient_ordinary_pass_stops_after_sign_preflight_failure() {
    let payload = find_payload();
    let (walk, reads) = base_reads_for_payload(&payload).unwrap();
    let base_inputs = base_inputs_for(&walk);
    let mut sign_inputs = sign_inputs_for(&reads);
    sign_inputs.pop();
    let (tile_after_base, consumed_after_base, symbols_after_base) =
        after_base_prefix(&payload, &base_inputs);
    let (mut tile, mut symbols, start) = setup_start(&payload).unwrap();

    let err = apply_nonzero_coeff_ordinary_pass(
        &mut tile,
        &mut symbols,
        CoeffOrdinaryPassInput {
            start,
            scan: &SCAN,
            base_inputs: &base_inputs,
            sign_inputs: &sign_inputs,
            max_level_config: max_level_config(),
            quant_config: quant_config(),
        },
    )
    .unwrap_err();

    assert!(matches!(err, CoeffOrdinaryPassError::Sign(_)));
    assert_eq!(tile, tile_after_base);
    assert_eq!(symbols.consumed_bits(), consumed_after_base);
    assert_eq!(symbols.symbol_count(), symbols_after_base);
}

#[test]
fn coefficient_ordinary_pass_with_derived_base_matches_explicit_inputs() {
    let config = luma_base_config(false, false);
    let payload = find_derived_payload(&SCAN, config, |pass| {
        pass.block().quant().iter().any(|quant| *quant != 0)
    });
    let derived_first_pass = derived_first_pass_for_payload(&payload, &SCAN, config).unwrap();
    let sign_inputs = sign_bit_inputs_for(derived_first_pass.walk());
    let explicit = explicit_pass_from_derived(
        &payload,
        &SCAN,
        &derived_first_pass,
        config,
        &sign_inputs,
        false,
    );
    let (mut tile, mut symbols, start) = setup_start(&payload).unwrap();
    let derived = apply_nonzero_coeff_ordinary_pass_with_derived_base(
        &mut tile,
        &mut symbols,
        CoeffOrdinaryDerivedBasePassInput {
            start,
            scan: &SCAN,
            base_config: config,
            sign_inputs: &sign_inputs,
            lossless: false,
        },
    )
    .unwrap();

    assert_eq!(derived.eob_read(), explicit.eob_read());
    assert_eq!(derived.walk(), explicit.walk());
    assert_eq!(
        derived.derived_base_inputs(),
        derived_first_pass.derived_inputs()
    );
    assert_eq!(derived.base_reads(), explicit.base_reads());
    assert_eq!(derived.sign_reads(), explicit.sign_reads());
    assert_eq!(derived.quant_pass(), explicit.quant_pass());
    assert_eq!(derived.block(), explicit.block());
}

#[test]
fn coefficient_ordinary_pass_with_derived_base_feeds_hidden_summary_to_quant() {
    let config = luma_base_config(true, false);
    let payload = find_derived_payload(&DC_LAST_HIDDEN_SCAN, config, |pass| {
        let first_pass = pass.base_level_pass().first_pass();
        first_pass.is_hidden() && first_pass.sum_abs1() > 0
    });
    let pass = derived_pass_for_payload(&payload, &DC_LAST_HIDDEN_SCAN, config, false).unwrap();
    let first_pass = pass.base_level_pass().first_pass();
    let dc_write = pass
        .quant_pass()
        .quant_state()
        .writes()
        .iter()
        .find(|write| write.entry().scan_index() == 0)
        .copied()
        .unwrap();

    assert!(first_pass.is_hidden());
    assert!(first_pass.sum_abs1() > 0);
    assert_eq!(
        dc_write.quant().unsigned_abs(),
        dc_write.read_quant() * 2 + first_pass.sum_abs1()
    );
}

#[test]
fn coefficient_ordinary_pass_with_derived_base_rejects_first_pass_config_before_consumption() {
    let payload = find_payload();
    let (mut tile, mut symbols, start, walk) = setup_start_and_walk(&payload, &SCAN).unwrap();
    let sign_inputs = sign_bit_inputs_for(&walk);
    let tile_before = tile.clone();
    let consumed_before = symbols.consumed_bits();
    let symbol_count_before = symbols.symbol_count();

    let err = apply_nonzero_coeff_ordinary_pass_with_derived_base(
        &mut tile,
        &mut symbols,
        CoeffOrdinaryDerivedBasePassInput {
            start,
            scan: &SCAN,
            base_config: luma_base_config(true, true),
            sign_inputs: &sign_inputs,
            lossless: false,
        },
    )
    .unwrap_err();

    assert!(matches!(
        err,
        CoeffOrdinaryPassError::BaseDerived(
            CoeffBaseDerivedLevelPassError::InconsistentParityAndTcq
        )
    ));
    assert_eq!(tile, tile_before);
    assert_eq!(symbols.consumed_bits(), consumed_before);
    assert_eq!(symbols.symbol_count(), symbol_count_before);
}

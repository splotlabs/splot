// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::panic)]

use splot_core::span::ByteOffset;
use splot_core::symbol::{CdfUpdateMode, SymbolDecoderConfig};

use super::super::super::cdf::block_read::BlockSymbolTraceReadError;
use super::super::super::cdf::{FrameCdfSubset, TileCdfArray, TileCdfError};
use super::super::super::coeff_state::TransformCoeffBlockState;
use super::super::branch::NonZeroCoeffBlockStartInput;
use super::super::max_level::CoeffTransformClass;
use super::super::scan_walk::{NonZeroCoeffScanWalk, walk_nonzero_coeff_scan};
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
static ABOVE_DC_POSITIVE: [u8; 4] = [2, 2, 0, 0];
static LEFT_DC_ZERO: [u8; 4] = [0, 0, 0, 0];

fn symbol_decoder(payload: &[u8], mode: CdfUpdateMode) -> SymbolDecoder<'_> {
    SymbolDecoder::with_base_and_config(
        payload,
        ByteOffset::new(0),
        SymbolDecoderConfig::new().with_cdf_update_mode(mode),
    )
    .unwrap()
}

fn setup_walk<'scan>(payload: &[u8], scan: &'scan [u16]) -> Option<NonZeroCoeffScanWalk<'scan>> {
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();
    let mut symbols = symbol_decoder(payload, CdfUpdateMode::Enabled);
    let start = read_nonzero_coeff_block_start(
        &mut tile,
        &mut symbols,
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
    )
    .ok()?;
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
    panic!("no coefficient sign EOB payload found");
}

fn block_for(walk: &NonZeroCoeffScanWalk<'_>) -> TransformCoeffBlockState {
    let mut block = TransformCoeffBlockState::new(8, 8).unwrap();
    for (index, entry) in walk.entries().enumerate() {
        let level = match index {
            0 => 3,
            1 => 2,
            2 => 0,
            _ => 1,
        };
        block.set_level(entry.row(), entry.col(), level).unwrap();
    }
    block
}

fn dc_sign_selector() -> CoeffDcSignSelector {
    CoeffDcSignSelector {
        coeff_cdf_q_ctx: 0,
        plane_type: 0,
        group: 0,
        ctx: 0,
    }
}

fn invalid_dc_sign_selector() -> CoeffDcSignSelector {
    CoeffDcSignSelector {
        coeff_cdf_q_ctx: 4,
        plane_type: 0,
        group: 0,
        ctx: 0,
    }
}

fn scan_entry(scan_index: usize, row: usize, col: usize) -> CoeffScanEntry {
    CoeffScanEntry::for_test(
        scan_index,
        row.saturating_mul(8).saturating_add(col),
        row,
        col,
    )
}

fn source_config(
    plane: usize,
    tx_class: CoeffTransformClass,
    is_hidden: bool,
    sum_abs1: u32,
) -> CoeffSignSourceDeriveConfig<'static> {
    CoeffSignSourceDeriveConfig {
        coeff_cdf_q_ctx: 3,
        plane,
        plane_type: usize::from(plane > 0),
        tx_class,
        is_hidden,
        sum_abs1,
        above_dc: &ABOVE_DC_POSITIVE,
        left_dc: &LEFT_DC_ZERO,
        x4: 0,
        y4: 0,
        w4: 2,
        h4: 2,
    }
}

fn inputs_for(walk: &NonZeroCoeffScanWalk<'_>) -> Vec<CoeffSignReadInput> {
    walk.entries()
        .enumerate()
        .map(|(index, entry)| CoeffSignReadInput {
            entry,
            source: match index {
                0 => CoeffSignReadSource::Cdf {
                    syntax: CoeffSignCdfSyntax::DcSign,
                    selector: dc_sign_selector(),
                },
                1 => CoeffSignReadSource::Cdf {
                    syntax: CoeffSignCdfSyntax::DcSignHorzVert,
                    selector: dc_sign_selector(),
                },
                2 => CoeffSignReadSource::None,
                _ => CoeffSignReadSource::SignBit,
            },
        })
        .collect()
}

#[test]
fn coefficient_sign_source_derives_luma_dc_context_and_hidden_group() {
    let entry = scan_entry(0, 0, 0);
    let walk = NonZeroCoeffScanWalk::from_entries_for_test(vec![entry]);
    let block = TransformCoeffBlockState::new(8, 8).unwrap();

    let inputs = derive_nonzero_coeff_sign_inputs(
        &block,
        &walk,
        source_config(0, CoeffTransformClass::TwoD, true, 3),
    )
    .unwrap();

    assert_eq!(
        inputs,
        vec![CoeffSignReadInput {
            entry,
            source: CoeffSignReadSource::Cdf {
                syntax: CoeffSignCdfSyntax::DcSign,
                selector: CoeffDcSignSelector {
                    coeff_cdf_q_ctx: 3,
                    plane_type: 0,
                    group: 1,
                    ctx: 2,
                },
            },
        }]
    );
}

#[test]
fn coefficient_sign_source_derives_horizontal_and_vertical_axis_contexts() {
    let horizontal_entry = scan_entry(2, 5, 0);
    let horizontal_walk = NonZeroCoeffScanWalk::from_entries_for_test(vec![horizontal_entry]);
    let vertical_entry = scan_entry(2, 0, 5);
    let vertical_walk = NonZeroCoeffScanWalk::from_entries_for_test(vec![vertical_entry]);
    let mut block = TransformCoeffBlockState::new(8, 8).unwrap();
    block
        .set_level(horizontal_entry.row(), horizontal_entry.col(), 1)
        .unwrap();
    block
        .set_level(vertical_entry.row(), vertical_entry.col(), 1)
        .unwrap();

    let horizontal = derive_nonzero_coeff_sign_inputs(
        &block,
        &horizontal_walk,
        source_config(0, CoeffTransformClass::Horizontal, false, 0),
    )
    .unwrap();
    let vertical = derive_nonzero_coeff_sign_inputs(
        &block,
        &vertical_walk,
        source_config(0, CoeffTransformClass::Vertical, false, 0),
    )
    .unwrap();

    for (entry, inputs) in [(horizontal_entry, horizontal), (vertical_entry, vertical)] {
        assert_eq!(
            inputs,
            vec![CoeffSignReadInput {
                entry,
                source: CoeffSignReadSource::Cdf {
                    syntax: CoeffSignCdfSyntax::DcSignHorzVert,
                    selector: CoeffDcSignSelector {
                        coeff_cdf_q_ctx: 3,
                        plane_type: 0,
                        group: 0,
                        ctx: 0,
                    },
                },
            }]
        );
    }
}

#[test]
fn coefficient_sign_source_derives_sign_bit_and_skip_sources() {
    let sign_bit_entry = scan_entry(3, 1, 1);
    let zero_entry = scan_entry(2, 2, 2);
    let chroma_dc_entry = scan_entry(1, 0, 0);
    let walk = NonZeroCoeffScanWalk::from_entries_for_test(vec![sign_bit_entry, zero_entry]);
    let chroma_walk = NonZeroCoeffScanWalk::from_entries_for_test(vec![chroma_dc_entry]);
    let mut block = TransformCoeffBlockState::new(8, 8).unwrap();
    block
        .set_level(sign_bit_entry.row(), sign_bit_entry.col(), 4)
        .unwrap();
    block
        .set_level(chroma_dc_entry.row(), chroma_dc_entry.col(), 2)
        .unwrap();

    let inputs = derive_nonzero_coeff_sign_inputs(
        &block,
        &walk,
        source_config(0, CoeffTransformClass::TwoD, false, 0),
    )
    .unwrap();
    let chroma_inputs = derive_nonzero_coeff_sign_inputs(
        &block,
        &chroma_walk,
        source_config(1, CoeffTransformClass::TwoD, false, 0),
    )
    .unwrap();

    assert_eq!(inputs[0].source, CoeffSignReadSource::SignBit);
    assert_eq!(inputs[1].source, CoeffSignReadSource::None);
    assert_eq!(chroma_inputs[0].source, CoeffSignReadSource::SignBit);
}

#[test]
fn coefficient_sign_source_derivation_reports_state_errors() {
    let entry = CoeffScanEntry::for_test(0, 99, 4, 0);
    let walk = NonZeroCoeffScanWalk::from_entries_for_test(vec![entry]);
    let block = TransformCoeffBlockState::new(4, 4).unwrap();

    let err = derive_nonzero_coeff_sign_inputs(
        &block,
        &walk,
        source_config(0, CoeffTransformClass::TwoD, false, 0),
    )
    .unwrap_err();

    assert!(matches!(
        err,
        CoeffSignSourceDeriveError::State(
            super::super::super::coeff_state::TileCoeffStateError::TransformCoordinateOutOfBounds {
                row: 4,
                col: 0,
                height: 4,
                width: 4,
            }
        )
    ));
}

#[test]
fn coefficient_sign_read_consumes_mixed_sources() {
    let payload = find_eob_payload();
    let walk = setup_walk(&payload, &EOB_SCAN).unwrap();
    let block = block_for(&walk);
    let mut tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut symbols = symbol_decoder(&[0xff, 0xff, 0x80], CdfUpdateMode::Enabled);
    let consumed_before = symbols.consumed_bits();
    let inputs = inputs_for(&walk);

    let reads = read_nonzero_coeff_signs(&mut tile, &mut symbols, &block, &walk, &inputs).unwrap();

    assert_eq!(reads.len(), walk.entries().len());
    assert!(matches!(
        reads[0].symbol(),
        CoeffSignReadSymbol::Cdf {
            syntax: CoeffSignCdfSyntax::DcSign,
            ..
        }
    ));
    assert!(matches!(
        reads[1].symbol(),
        CoeffSignReadSymbol::Cdf {
            syntax: CoeffSignCdfSyntax::DcSignHorzVert,
            ..
        }
    ));
    assert_eq!(reads[2].level(), 0);
    assert_eq!(reads[2].symbol(), CoeffSignReadSymbol::None);
    assert!(!reads[2].sign());
    assert!(matches!(
        reads[3].symbol(),
        CoeffSignReadSymbol::SignBit { .. }
    ));
    for (read, input) in reads.iter().zip(&inputs) {
        assert_eq!(read.entry(), input.entry);
    }
    assert!(symbols.consumed_bits() > consumed_before);
    assert!(symbols.symbol_count() >= 2);
}

#[test]
fn coefficient_sign_read_rejects_missing_required_sign_before_consumption() {
    let payload = find_eob_payload();
    let walk = setup_walk(&payload, &EOB_SCAN).unwrap();
    let block = block_for(&walk);
    let mut tile = FrameCdfSubset::from_defaults().tile_copy();
    let tile_before = tile.clone();
    let mut symbols = symbol_decoder(&[0xff, 0x80], CdfUpdateMode::Enabled);
    let consumed_before = symbols.consumed_bits();
    let symbol_count_before = symbols.symbol_count();
    let mut inputs = inputs_for(&walk);
    inputs[0].source = CoeffSignReadSource::None;

    let err =
        read_nonzero_coeff_signs(&mut tile, &mut symbols, &block, &walk, &inputs).unwrap_err();

    assert!(matches!(
        err,
        CoeffSignReadError::MissingRequiredSign {
            index: 0,
            level: 3,
            ..
        }
    ));
    assert_eq!(tile, tile_before);
    assert_eq!(symbols.consumed_bits(), consumed_before);
    assert_eq!(symbols.symbol_count(), symbol_count_before);
}

#[test]
fn coefficient_sign_read_rejects_scan_entry_mismatch_before_consumption() {
    let payload = find_eob_payload();
    let walk = setup_walk(&payload, &EOB_SCAN).unwrap();
    let alt_walk = setup_walk(&payload, &ALT_SCAN).unwrap();
    let block = block_for(&walk);
    let mut tile = FrameCdfSubset::from_defaults().tile_copy();
    let tile_before = tile.clone();
    let mut symbols = symbol_decoder(&[0xff, 0x80], CdfUpdateMode::Enabled);
    let consumed_before = symbols.consumed_bits();
    let inputs = inputs_for(&walk);

    let err =
        read_nonzero_coeff_signs(&mut tile, &mut symbols, &block, &alt_walk, &inputs).unwrap_err();

    assert!(matches!(
        err,
        CoeffSignReadError::ScanEntryMismatch { index: 0, .. }
    ));
    assert_eq!(tile, tile_before);
    assert_eq!(symbols.consumed_bits(), consumed_before);
}

#[test]
fn coefficient_sign_read_rejects_invalid_cdf_selector_before_symbol_read() {
    let payload = find_eob_payload();
    let walk = setup_walk(&payload, &EOB_SCAN).unwrap();
    let block = block_for(&walk);
    let mut tile = FrameCdfSubset::from_defaults().tile_copy();
    let tile_before = tile.clone();
    let mut symbols = symbol_decoder(&[0xff, 0x80], CdfUpdateMode::Enabled);
    let consumed_before = symbols.consumed_bits();
    let symbol_count_before = symbols.symbol_count();
    let mut inputs = inputs_for(&walk);
    inputs[0].source = CoeffSignReadSource::Cdf {
        syntax: CoeffSignCdfSyntax::DcSign,
        selector: invalid_dc_sign_selector(),
    };

    let err =
        read_nonzero_coeff_signs(&mut tile, &mut symbols, &block, &walk, &inputs).unwrap_err();

    assert!(matches!(
        err,
        CoeffSignReadError::SymbolRead(BlockSymbolTraceReadError::Cdf(
            TileCdfError::SelectorOutOfRange {
                array: TileCdfArray::DcSign,
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
fn coefficient_sign_read_rejects_input_count_mismatch_before_consumption() {
    let payload = find_eob_payload();
    let walk = setup_walk(&payload, &EOB_SCAN).unwrap();
    let block = block_for(&walk);
    let mut tile = FrameCdfSubset::from_defaults().tile_copy();
    let tile_before = tile.clone();
    let mut symbols = symbol_decoder(&[0xff, 0x80], CdfUpdateMode::Enabled);
    let consumed_before = symbols.consumed_bits();
    let symbol_count_before = symbols.symbol_count();
    let mut inputs = inputs_for(&walk);
    inputs.pop();

    let err =
        read_nonzero_coeff_signs(&mut tile, &mut symbols, &block, &walk, &inputs).unwrap_err();

    assert!(matches!(
        err,
        CoeffSignReadError::InputCountMismatch {
            inputs: 3,
            entries: 4
        }
    ));
    assert_eq!(tile, tile_before);
    assert_eq!(symbols.consumed_bits(), consumed_before);
    assert_eq!(symbols.symbol_count(), symbol_count_before);
}

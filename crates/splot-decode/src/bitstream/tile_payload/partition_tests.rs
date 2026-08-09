// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use splot_core::span::ByteOffset;
use splot_core::symbol::{CdfUpdateMode, SymbolDecoderConfig};

use super::super::cdf::FrameCdfSubset;
use super::*;

const BLOCK_4X4: u8 = 0;
const BLOCK_16X16: usize = 6;
const BLOCK_32X32: usize = 9;

fn allowed(partitions: &[PartitionType]) -> AllowedPartitions {
    let mut flags = [false; EXT_PARTITION_TYPES];
    for partition in partitions {
        flags[partition.index()] = true;
    }
    AllowedPartitions::new(flags)
}

fn partition_context() -> PartitionContextInput<'static> {
    static LEFT0: [u8; 32] = [BLOCK_4X4; 32];
    static LEFT1: [u8; 32] = [BLOCK_4X4; 32];
    static ABOVE0: [u8; 32] = [BLOCK_4X4; 32];
    static ABOVE1: [u8; 32] = [BLOCK_4X4; 32];
    PartitionContextInput::new(BLOCK_32X32, 0, 0, 0, [&LEFT0, &LEFT1], [&ABOVE0, &ABOVE1]).unwrap()
}

fn square_context() -> SquareSplitContextInput<'static> {
    static GRID: [u8; 4] = [BLOCK_4X4; 4];
    SquareSplitContextInput::new(BLOCK_16X16, 0, 0, 0, false, false, &GRID, 2).unwrap()
}

fn input(
    allowed: AllowedPartitions,
    implied_partition: Option<PartitionType>,
    bru_active: bool,
    rect_type: Option<RectPartitionType>,
) -> ReadPartitionDecisionInput<'static> {
    ReadPartitionDecisionInput::new(
        allowed,
        implied_partition,
        bru_active,
        rect_type,
        partition_context(),
        square_context(),
    )
}

fn decoder(payload: &'static [u8], update_mode: CdfUpdateMode) -> SymbolDecoder<'static> {
    SymbolDecoder::with_base_and_config(
        payload,
        ByteOffset::new(0),
        SymbolDecoderConfig::new().with_cdf_update_mode(update_mode),
    )
    .unwrap()
}

fn cdfs() -> TileCdfSubset {
    FrameCdfSubset::from_defaults().tile_copy()
}

fn decision(
    input: ReadPartitionDecisionInput<'static>,
    payload: &'static [u8],
) -> (
    Result<PartitionType, PartitionDecisionError>,
    TileCdfSubset,
    SymbolDecoder<'static>,
) {
    let mut cdfs = cdfs();
    let mut symbols = decoder(payload, CdfUpdateMode::Enabled);
    let result = read_partition_decision(input, &mut cdfs, &mut symbols);
    (result, cdfs, symbols)
}

#[test]
fn implied_partition_returns_without_symbol_consumption() {
    let mut cdfs = cdfs();
    let before = cdfs.clone();
    let mut symbols = decoder(&[0x00, 0x80], CdfUpdateMode::Enabled);

    let result = read_partition_decision(
        input(
            allowed(&[PartitionType::Horz, PartitionType::Vert]),
            Some(PartitionType::Vert),
            true,
            None,
        ),
        &mut cdfs,
        &mut symbols,
    )
    .unwrap();

    assert_eq!(result, PartitionType::Vert);
    assert_eq!(symbols.symbol_count(), 0);
    assert_eq!(cdfs, before);
}

#[test]
fn single_allowed_returns_in_spec_order_without_symbol_consumption() {
    let (result, cdfs_after, symbols) = decision(
        input(allowed(&[PartitionType::Horz4B]), None, true, None),
        &[0xFF, 0xFF],
    );

    assert_eq!(result.unwrap(), PartitionType::Horz4B);
    assert_eq!(symbols.symbol_count(), 0);
    assert_eq!(cdfs_after, cdfs());
}

#[test]
fn inactive_bru_returns_none_without_symbol_consumption() {
    let (result, cdfs_after, symbols) = decision(
        input(
            allowed(&[
                PartitionType::None,
                PartitionType::Horz,
                PartitionType::Vert,
            ]),
            None,
            false,
            None,
        ),
        &[0xFF, 0xFF],
    );

    assert_eq!(result.unwrap(), PartitionType::None);
    assert_eq!(symbols.symbol_count(), 0);
    assert_eq!(cdfs_after, cdfs());
}

#[test]
fn disallowed_implied_partition_falls_through_to_single_allowed() {
    let (result, cdfs_after, symbols) = decision(
        input(
            allowed(&[PartitionType::Horz]),
            Some(PartitionType::Vert),
            true,
            None,
        ),
        &[0xFF, 0xFF],
    );

    assert_eq!(result.unwrap(), PartitionType::Horz);
    assert_eq!(symbols.symbol_count(), 0);
    assert_eq!(cdfs_after, cdfs());
}

#[test]
fn disallowed_implied_partition_falls_through_to_reached_syntax() {
    let (result, _, symbols) = decision(
        input(
            allowed(&[PartitionType::None, PartitionType::Horz]),
            Some(PartitionType::Vert),
            true,
            Some(RectPartitionType::Horz),
        ),
        &[0x00, 0x80],
    );
    let result = result.unwrap();

    assert_eq!(result, PartitionType::None);
    assert_eq!(symbols.symbol_count(), 1);
}

#[test]
fn inactive_bru_returns_none_even_when_none_is_disallowed() {
    let (result, cdfs_after, symbols) = decision(
        input(
            allowed(&[PartitionType::Horz, PartitionType::Vert]),
            None,
            false,
            None,
        ),
        &[0xFF, 0xFF],
    );

    assert_eq!(result.unwrap(), PartitionType::None);
    assert_eq!(symbols.symbol_count(), 0);
    assert_eq!(cdfs_after, cdfs());
}

#[test]
fn do_split_false_returns_none_and_stops() {
    let (result, cdfs_after, symbols) = decision(
        input(
            allowed(&[
                PartitionType::None,
                PartitionType::Split,
                PartitionType::Horz,
            ]),
            None,
            true,
            Some(RectPartitionType::Horz),
        ),
        &[0x00, 0x80],
    );
    let result = result.unwrap();

    assert_eq!(result, PartitionType::None);
    assert_eq!(symbols.symbol_count(), 1);
    assert_ne!(cdfs_after, cdfs());
}

#[test]
fn square_split_true_returns_split_before_rect_symbols() {
    let (result, _, symbols) = decision(
        input(
            allowed(&[
                PartitionType::Split,
                PartitionType::Horz,
                PartitionType::Vert,
            ]),
            None,
            true,
            None,
        ),
        &[0xFF, 0xFF],
    );
    let result = result.unwrap();

    assert_eq!(result, PartitionType::Split);
    assert_eq!(symbols.symbol_count(), 1);
}

#[test]
fn rect_type_symbol_selects_vertical_non_extended_partition() {
    let (result, _, symbols) = decision(
        input(
            allowed(&[PartitionType::Horz, PartitionType::Vert]),
            None,
            true,
            None,
        ),
        &[0xFF, 0xFF],
    );
    let result = result.unwrap();

    assert_eq!(result, PartitionType::Vert);
    assert_eq!(symbols.symbol_count(), 1);
}

#[test]
fn forced_horizontal_rect_reads_ext_symbol_once() {
    let (result, _, symbols) = decision(
        input(
            allowed(&[PartitionType::Horz, PartitionType::Horz3]),
            None,
            true,
            Some(RectPartitionType::Horz),
        ),
        &[0xFF, 0xFF],
    );
    let result = result.unwrap();

    assert_eq!(result, PartitionType::Horz3);
    assert_eq!(symbols.symbol_count(), 1);
}

#[test]
fn uneven_four_way_literal_selects_table_variant() {
    let (result, _, symbols) = decision(
        input(
            allowed(&[
                PartitionType::Horz,
                PartitionType::Horz3,
                PartitionType::Horz4A,
                PartitionType::Horz4B,
            ]),
            None,
            true,
            Some(RectPartitionType::Horz),
        ),
        &[0xFF, 0xFF],
    );
    let result = result.unwrap();

    assert_eq!(result, PartitionType::Horz4B);
    assert_eq!(symbols.symbol_count(), 3);
}

#[test]
fn uneven_four_way_without_three_way_reads_only_literal() {
    let (result, _, symbols) = decision(
        input(
            allowed(&[
                PartitionType::Vert,
                PartitionType::Vert4A,
                PartitionType::Vert4B,
            ]),
            None,
            true,
            Some(RectPartitionType::Vert),
        ),
        &[0xFF, 0xFF],
    );
    let result = result.unwrap();

    assert_eq!(result, PartitionType::Vert4B);
    assert_eq!(symbols.symbol_count(), 2);
}

#[test]
fn empty_allowed_set_is_rejected_before_symbol_consumption() {
    let mut cdfs = cdfs();
    let before = cdfs.clone();
    let mut symbols = decoder(&[0x80, 0x00], CdfUpdateMode::Enabled);

    let err = read_partition_decision(
        input(
            AllowedPartitions::new([false; EXT_PARTITION_TYPES]),
            None,
            true,
            None,
        ),
        &mut cdfs,
        &mut symbols,
    )
    .unwrap_err();

    assert!(matches!(err, PartitionDecisionError::EmptyAllowedSet));
    assert_eq!(symbols.symbol_count(), 0);
    assert_eq!(cdfs, before);
}

#[test]
fn impossible_final_table_result_is_rejected() {
    let mut cdfs = cdfs();
    let mut symbols = decoder(&[0x00, 0x80], CdfUpdateMode::Enabled);

    let err = read_partition_decision(
        input(
            allowed(&[PartitionType::Vert, PartitionType::Vert3]),
            None,
            true,
            Some(RectPartitionType::Horz),
        ),
        &mut cdfs,
        &mut symbols,
    )
    .unwrap_err();

    assert!(matches!(
        err,
        PartitionDecisionError::FinalPartitionDisallowed {
            partition: PartitionType::Horz
        }
    ));
}

#[test]
fn cdf_selector_error_fails_before_symbol_consumption() {
    static EMPTY: [u8; 0] = [];
    let grid = vec![BLOCK_4X4; 4];
    let input = ReadPartitionDecisionInput::new(
        allowed(&[PartitionType::None, PartitionType::Horz]),
        None,
        true,
        Some(RectPartitionType::Horz),
        PartitionContextInput::new(BLOCK_32X32, 0, 1, 0, [&EMPTY, &EMPTY], [&EMPTY, &EMPTY])
            .unwrap(),
        SquareSplitContextInput::new(BLOCK_16X16, 0, 0, 0, false, false, &grid, 2).unwrap(),
    );
    let mut cdfs = cdfs();
    let before = cdfs.clone();
    let mut symbols = decoder(&[0x00, 0x80], CdfUpdateMode::Enabled);

    let err = read_partition_decision(input, &mut cdfs, &mut symbols).unwrap_err();

    assert!(matches!(
        err,
        PartitionDecisionError::Cdf(TileCdfError::PartitionNeighborOutOfRange { .. })
    ));
    assert_eq!(symbols.symbol_count(), 0);
    assert_eq!(cdfs, before);
}

#[test]
fn empty_payload_literal_branch_is_deterministic() {
    let mut cdfs = cdfs();
    let mut symbols = decoder(&[], CdfUpdateMode::Enabled);

    let result = read_partition_decision(
        input(
            allowed(&[PartitionType::Vert4A, PartitionType::Vert4B]),
            None,
            true,
            Some(RectPartitionType::Vert),
        ),
        &mut cdfs,
        &mut symbols,
    )
    .unwrap();

    assert_eq!(result, PartitionType::Vert4A);
    assert_eq!(symbols.symbol_count(), 1);
}

#[test]
fn repeated_inputs_are_deterministic() {
    let input = input(
        allowed(&[
            PartitionType::Split,
            PartitionType::Horz,
            PartitionType::Vert,
        ]),
        None,
        true,
        None,
    );

    let (first, first_cdfs, first_symbols) = decision(input, &[0x00, 0x80]);
    let (second, second_cdfs, second_symbols) = decision(input, &[0x00, 0x80]);

    assert_eq!(first.unwrap(), second.unwrap());
    assert_eq!(first_cdfs, second_cdfs);
    assert_eq!(
        first_symbols.consumed_bits().get(),
        second_symbols.consumed_bits().get()
    );
}

#[test]
fn bounded_payload_matrix_never_panics() {
    let allowed_sets = [
        allowed(&[
            PartitionType::None,
            PartitionType::Split,
            PartitionType::Horz,
        ]),
        allowed(&[
            PartitionType::Split,
            PartitionType::Horz,
            PartitionType::Vert,
        ]),
        allowed(&[
            PartitionType::Horz,
            PartitionType::Horz3,
            PartitionType::Horz4A,
        ]),
        allowed(&[
            PartitionType::Vert,
            PartitionType::Vert3,
            PartitionType::Vert4A,
        ]),
    ];
    let payloads: [&[u8]; 5] = [&[], &[0x00], &[0x80], &[0x00, 0x80], &[0xFF, 0xFF]];

    for allowed in allowed_sets {
        for payload in payloads {
            let mut cdfs = cdfs();
            let mut symbols = SymbolDecoder::with_base_and_config(
                payload,
                ByteOffset::new(0),
                SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Enabled),
            )
            .unwrap();
            let _ =
                read_partition_decision(input(allowed, None, true, None), &mut cdfs, &mut symbols);
        }
    }
}

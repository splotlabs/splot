// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use splot_core::error::SymbolCdfErrorKind;
use splot_core::span::ByteOffset;
use splot_core::symbol::{CdfUpdateMode, SymbolDecoderConfig};

use super::super::{FrameCdfSubset, TileCdfArray};
use super::*;

const PAYLOAD: [u8; 2] = [0x80, 0x00];

fn decoder(payload: &'static [u8], mode: CdfUpdateMode) -> SymbolDecoder<'static> {
    SymbolDecoder::with_base_and_config(
        payload,
        ByteOffset::new(0),
        SymbolDecoderConfig::new().with_cdf_update_mode(mode),
    )
    .unwrap()
}

#[test]
fn reads_supported_partition_entry_symbols() {
    let selectors = [
        TileCdfSelector::DoSplit {
            plane_start: 0,
            ctx: 0,
        },
        TileCdfSelector::DoSquareSplit {
            plane_start: 0,
            ctx: 0,
        },
        TileCdfSelector::RectType {
            plane_start: 0,
            ctx: 4,
        },
        TileCdfSelector::DoExtPartition {
            plane_start: 0,
            ctx: 4,
        },
        TileCdfSelector::DoUneven4WayPartition {
            plane_start: 0,
            ctx: 8,
        },
    ];
    let frame = FrameCdfSubset::from_defaults();

    for selector in selectors {
        let mut direct_tile = frame.tile_copy();
        let mut helper_tile = frame.tile_copy();
        let mut direct = decoder(&PAYLOAD, CdfUpdateMode::Enabled);
        let mut helper = decoder(&PAYLOAD, CdfUpdateMode::Enabled);

        let expected = direct_tile
            .with_row_mut(selector, |row| direct.read_symbol_u16(row))
            .unwrap()
            .unwrap();
        let actual = helper_tile
            .read_partition_entry_symbol(selector, &mut helper)
            .unwrap();

        assert_eq!(actual, expected);
        assert_eq!(helper.consumed_bits(), direct.consumed_bits());
        assert_eq!(
            helper_tile
                .with_row_mut(selector, |row| row.to_vec())
                .unwrap(),
            direct_tile
                .with_row_mut(selector, |row| row.to_vec())
                .unwrap()
        );
    }
}

#[test]
fn enabled_updates_only_selected_partition_entry_row() {
    let frame = FrameCdfSubset::from_defaults();
    let selector = TileCdfSelector::DoSplit {
        plane_start: 0,
        ctx: 0,
    };
    let untouched = TileCdfSelector::RectType {
        plane_start: 0,
        ctx: 4,
    };
    let mut tile = frame.tile_copy();
    let selected_before = tile.with_row_mut(selector, |row| row.to_vec()).unwrap();
    let untouched_before = tile.with_row_mut(untouched, |row| row.to_vec()).unwrap();
    let mut symbol = decoder(&PAYLOAD, CdfUpdateMode::Enabled);

    let _ = tile
        .read_partition_entry_symbol(selector, &mut symbol)
        .unwrap();

    assert_ne!(
        tile.with_row_mut(selector, |row| row.to_vec()).unwrap(),
        selected_before.as_slice()
    );
    assert_eq!(
        tile.with_row_mut(untouched, |row| row.to_vec()).unwrap(),
        untouched_before.as_slice()
    );
}

#[test]
fn disabled_update_mode_leaves_selected_row_unchanged() {
    let frame = FrameCdfSubset::from_defaults();
    let selector = TileCdfSelector::DoSquareSplit {
        plane_start: 0,
        ctx: 0,
    };
    let mut tile = frame.tile_copy();
    let before = tile.with_row_mut(selector, |row| row.to_vec()).unwrap();
    let mut symbol = decoder(&PAYLOAD, CdfUpdateMode::Disabled);

    let _ = tile
        .read_partition_entry_symbol(selector, &mut symbol)
        .unwrap();

    assert_eq!(
        tile.with_row_mut(selector, |row| row.to_vec()).unwrap(),
        before.as_slice()
    );
}

fn assert_selector_error_is_inert(
    valid: TileCdfSelector,
    invalid: TileCdfSelector,
    expected: impl FnOnce(&PartitionEntrySymbolReadError) -> bool,
) {
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();
    let before = tile.with_row_mut(valid, |row| row.to_vec()).unwrap();
    let mut symbol = decoder(&PAYLOAD, CdfUpdateMode::Enabled);
    let consumed_before = symbol.consumed_bits();

    let err = tile
        .read_partition_entry_symbol(invalid, &mut symbol)
        .unwrap_err();

    assert!(expected(&err), "unexpected error: {err:?}");
    assert_eq!(symbol.consumed_bits(), consumed_before);
    assert_eq!(
        tile.with_row_mut(valid, |row| row.to_vec()).unwrap(),
        before.as_slice()
    );
}

#[test]
fn selector_error_does_not_advance_symbol_or_mutate_rows() {
    assert_selector_error_is_inert(
        TileCdfSelector::DoSplit {
            plane_start: 0,
            ctx: 0,
        },
        TileCdfSelector::DoSplit {
            plane_start: 0,
            ctx: 64,
        },
        |err| {
            matches!(
                err,
                PartitionEntrySymbolReadError::Cdf(TileCdfError::SelectorOutOfRange {
                    array: TileCdfArray::DoSplit,
                    index_name: "ctx",
                    actual: 64,
                    max_exclusive: 64,
                })
            )
        },
    );
}

#[test]
fn square_split_invalid_plane_fails_before_symbol_read() {
    assert_selector_error_is_inert(
        TileCdfSelector::DoSquareSplit {
            plane_start: 0,
            ctx: 0,
        },
        TileCdfSelector::DoSquareSplit {
            plane_start: 1,
            ctx: 0,
        },
        |err| {
            matches!(
                err,
                PartitionEntrySymbolReadError::Cdf(TileCdfError::SelectorOutOfRange {
                    array: TileCdfArray::DoSquareSplit,
                    index_name: "plane_start",
                    actual: 1,
                    max_exclusive: 1,
                })
            )
        },
    );
}

#[test]
fn symbol_cdf_error_preserves_core_error_and_row() {
    let frame = FrameCdfSubset::from_defaults();
    let selector = TileCdfSelector::DoSplit {
        plane_start: 0,
        ctx: 0,
    };
    let mut tile = frame.tile_copy();
    tile.rows.do_split[0][0] = [0, 0, 0];
    let before = tile.with_row_mut(selector, |row| row.to_vec()).unwrap();
    let mut symbol = decoder(&PAYLOAD, CdfUpdateMode::Enabled);

    let err = tile
        .read_partition_entry_symbol(selector, &mut symbol)
        .unwrap_err();

    assert!(matches!(
        err,
        PartitionEntrySymbolReadError::Symbol(CoreError::InvalidSymbolCdf {
            kind: SymbolCdfErrorKind::ProbabilityOutOfRange { index: 0, value: 0 },
            ..
        })
    ));
    assert_eq!(
        tile.with_row_mut(selector, |row| row.to_vec()).unwrap(),
        before.as_slice()
    );
}

#[test]
fn zero_length_payload_read_matches_direct_symbol_handoff() {
    let frame = FrameCdfSubset::from_defaults();
    let selector = TileCdfSelector::DoSplit {
        plane_start: 0,
        ctx: 0,
    };
    let mut direct_tile = frame.tile_copy();
    let mut helper_tile = frame.tile_copy();
    let mut direct = decoder(&[], CdfUpdateMode::Enabled);
    let mut helper = decoder(&[], CdfUpdateMode::Enabled);

    let expected = direct_tile
        .with_row_mut(selector, |row| direct.read_symbol_u16(row))
        .unwrap()
        .unwrap();
    let actual = helper_tile
        .read_partition_entry_symbol(selector, &mut helper)
        .unwrap();

    assert_eq!(actual, expected);
    assert_eq!(helper.consumed_bits(), direct.consumed_bits());
    assert_eq!(
        helper_tile
            .with_row_mut(selector, |row| row.to_vec())
            .unwrap(),
        direct_tile
            .with_row_mut(selector, |row| row.to_vec())
            .unwrap()
    );
}

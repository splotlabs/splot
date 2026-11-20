// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::span::ByteOffset;
use splot_core::symbol::{CdfUpdateMode, SymbolDecoder, SymbolDecoderConfig};
use splot_recon::ReconError;

use super::super::{
    WienerNsLrTxSkipLookup, derive_cdef_skip_grid, derive_wienerns_lr_tx_skip_grid_retention,
    derive_wienerns_lr_tx_skip_grid_reusing,
};
use super::*;
use crate::bitstream::tile_payload::{encode_symbol_sequence, make_test_work_unit};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Block4x4Extent {
    pub(super) cols: usize,
    pub(super) rows: usize,
}

pub(super) fn collect_inter_luma_transforms(
    (row, col): (usize, usize),
    extent: Block4x4Extent,
    (rows, cols): (usize, usize),
    tx_size: usize,
    partition: LumaTxPartition,
    records: &mut Vec<SelectableLumaTxRecord>,
) -> std::result::Result<(), SelectableTransformRecordError> {
    let end = ((row + extent.rows).min(rows), (col + extent.cols).min(cols));
    records.clear();
    records.extend(inter_luma_transforms((row, col), end, tx_size, partition)?);
    Ok(())
}

const TX_16X16: usize = 2;
const TX_32X32: usize = 3;
const TX_64X64: usize = 4;
const TX_8X8: usize = 1;
const TX_4X32: usize = 19;
const CONFLICTING_TX_SKIP_RECORDS: [WienerNsLrTxSkipTransformRecord; 2] = [
    WienerNsLrTxSkipTransformRecord {
        row: 0,
        col: 0,
        rows: 1,
        cols: 2,
        skip_flag: true,
        empty: false,
    },
    WienerNsLrTxSkipTransformRecord {
        row: 0,
        col: 1,
        rows: 1,
        cols: 1,
        skip_flag: false,
        empty: false,
    },
];

#[test]
fn tx_skip_grid_reuses_small_then_large_then_small_backing() {
    let record = WienerNsLrTxSkipTransformRecord {
        row: 0,
        col: 0,
        rows: 2,
        cols: 3,
        skip_flag: true,
        empty: true,
    };
    let values = vec![7];
    let large = derive_wienerns_lr_tx_skip_grid_reusing(2, 3, &[record], values).unwrap();
    assert_eq!(
        large.lookup(WienerNsLrTxSkipLookup { row: 1, col: 2 }),
        Ok(1)
    );
    let values = large.into_values();
    let capacity = values.capacity();
    assert!(capacity >= 6);

    let small_record = WienerNsLrTxSkipTransformRecord {
        rows: 1,
        cols: 1,
        skip_flag: false,
        empty: false,
        ..record
    };
    let small = derive_wienerns_lr_tx_skip_grid_reusing(1, 1, &[small_record], values).unwrap();
    assert_eq!(
        small.lookup(WienerNsLrTxSkipLookup { row: 0, col: 0 }),
        Ok(0)
    );
    assert_eq!(small.into_values().capacity(), capacity);
}

#[test]
fn inter_partition_reader_uses_sequence_selected_reduced_cdf() {
    for reduced_tx_part_set in [false, true] {
        let payload = encode_symbol_sequence(&[
            (
                TileCdfSelector::TxDoPartition {
                    fsc_mode: 0,
                    is_inter: 1,
                    txfm_split_group: 3,
                },
                1,
            ),
            (
                TileCdfSelector::TxPartitionType {
                    fsc_mode: 0,
                    is_inter: 1,
                    ctx: 3,
                    reduced: reduced_tx_part_set,
                },
                3,
            ),
        ]);
        let mut work_unit = make_test_work_unit(&payload, CdfUpdateMode::Disabled);
        let mut symbols = SymbolDecoder::with_base_and_config(
            &payload,
            ByteOffset::new(0),
            SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Disabled),
        )
        .unwrap();

        assert_eq!(
            read_tx_partition_symbols(
                &mut work_unit,
                &mut symbols,
                TX_16X16,
                6,
                0,
                reduced_tx_part_set,
                ByteOffset::new(0),
            )
            .unwrap(),
            LumaTxPartition::Horz4
        );
        symbols.exit_symbol().unwrap();
    }
}

#[test]
fn inter_reduced_partition_set_suppresses_one_axis_four_way_symbol() {
    for (reduced_tx_part_set, sequence, expected) in [
        (
            false,
            vec![
                (
                    TileCdfSelector::TxDoPartition {
                        fsc_mode: 0,
                        is_inter: 1,
                        txfm_split_group: 8,
                    },
                    1,
                ),
                (
                    TileCdfSelector::Tx2Or3PartitionType {
                        fsc_mode: 0,
                        is_inter: 1,
                        ctx: 0,
                    },
                    1,
                ),
            ],
            LumaTxPartition::Horz4,
        ),
        (
            true,
            vec![(
                TileCdfSelector::TxDoPartition {
                    fsc_mode: 0,
                    is_inter: 1,
                    txfm_split_group: 8,
                },
                1,
            )],
            LumaTxPartition::Horz,
        ),
    ] {
        let payload = encode_symbol_sequence(&sequence);
        let mut work_unit = make_test_work_unit(&payload, CdfUpdateMode::Disabled);
        let mut symbols = SymbolDecoder::with_base_and_config(
            &payload,
            ByteOffset::new(0),
            SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Disabled),
        )
        .unwrap();

        assert_eq!(
            read_tx_partition_symbols(
                &mut work_unit,
                &mut symbols,
                TX_4X32,
                19,
                0,
                reduced_tx_part_set,
                ByteOffset::new(0),
            )
            .unwrap(),
            expected
        );
        symbols.exit_symbol().unwrap();
    }
}

#[test]
fn writer_produced_inter_narrow_four_and_five_way_partitions_are_malformed() {
    for (symbol, expected) in [(3, (0, 2)), (4, (2, 0)), (5, (0, 1)), (6, (1, 0))] {
        let payload = encode_symbol_sequence(&[
            (
                TileCdfSelector::TxDoPartition {
                    fsc_mode: 0,
                    is_inter: 1,
                    txfm_split_group: 1,
                },
                1,
            ),
            (
                TileCdfSelector::TxPartitionType {
                    fsc_mode: 0,
                    is_inter: 1,
                    ctx: 0,
                    reduced: false,
                },
                symbol,
            ),
        ]);
        let mut work_unit = make_test_work_unit(&payload, CdfUpdateMode::Disabled);
        let mut symbols = SymbolDecoder::with_base_and_config(
            &payload,
            ByteOffset::new(41),
            SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Disabled),
        )
        .unwrap();
        let partition = read_tx_partition_symbols(
            &mut work_unit,
            &mut symbols,
            TX_8X8,
            3,
            0,
            false,
            ByteOffset::new(41),
        )
        .unwrap();
        symbols.exit_symbol().unwrap();

        let geometry = collect_inter_luma_transforms(
            (0, 0),
            Block4x4Extent { rows: 2, cols: 2 },
            (2, 2),
            TX_8X8,
            partition,
            &mut Vec::new(),
        )
        .unwrap_err();
        assert_eq!(
            geometry,
            SelectableTransformRecordError::EmptyTransform {
                h4: expected.0,
                w4: expected.1,
            }
        );
        let error = selectable_transform_partition_error(geometry, ByteOffset::new(41));
        match error {
            crate::error::DecodeError::MalformedSource { issue } => {
                assert_eq!(issue.spec_section(), Some("6.19.6.3"));
                assert_eq!(issue.offset(), Some(ByteOffset::new(41)));
            }
            other => panic!("unexpected decode error: {other:?}"),
        }
    }
}

#[test]
fn selectable_tx_grid_returns_vertical_five_partition_in_scan_order() {
    let mut records = Vec::new();
    collect_inter_luma_transforms(
        (0, 0),
        Block4x4Extent { rows: 8, cols: 8 },
        (8, 8),
        TX_32X32,
        LumaTxPartition::Vert5,
        &mut records,
    )
    .unwrap();
    assert_eq!(records.len(), 5);
    assert_eq!(
        records
            .iter()
            .map(|record| (record.row, record.col, record.rows, record.cols))
            .collect::<Vec<_>>(),
        vec![
            (0, 0, 4, 2),
            (4, 0, 4, 2),
            (0, 2, 8, 4),
            (0, 6, 4, 2),
            (4, 6, 4, 2)
        ]
    );
}

#[test]
fn every_inter_layout_covers_each_visible_cell_once() {
    let kinds = [
        LumaTxPartition::None,
        LumaTxPartition::Split,
        LumaTxPartition::Horz,
        LumaTxPartition::Vert,
        LumaTxPartition::Horz4,
        LumaTxPartition::Vert4,
        LumaTxPartition::Horz5,
        LumaTxPartition::Vert5,
    ];
    let mut records = Vec::new();
    for (tx, (&width, &height)) in TX_WIDTH.iter().zip(&TX_HEIGHT).enumerate() {
        let extent = Block4x4Extent {
            cols: width as usize / MI_SIZE,
            rows: height as usize / MI_SIZE,
        };
        for &partition in &kinds {
            let (min_width, min_height) = match partition {
                LumaTxPartition::None => (4, 4),
                LumaTxPartition::Split => (8, 8),
                LumaTxPartition::Horz => (4, 8),
                LumaTxPartition::Vert => (8, 4),
                LumaTxPartition::Horz4 => (4, 16),
                LumaTxPartition::Vert4 => (16, 4),
                LumaTxPartition::Horz5 => (8, 16),
                LumaTxPartition::Vert5 => (16, 8),
            };
            let valid = width >= min_width && height >= min_height;
            for rows in 1..=extent.rows {
                for cols in 1..=extent.cols {
                    if let Err(error) = collect_inter_luma_transforms(
                        (8, 24),
                        extent,
                        (8 + rows, 24 + cols),
                        tx,
                        partition,
                        &mut records,
                    ) {
                        assert!(!valid);
                        assert!(matches!(
                            error,
                            SelectableTransformRecordError::EmptyTransform { .. }
                        ));
                        continue;
                    }
                    assert!(valid);
                    assert!(
                        records.is_sorted_by_key(|r| if partition == LumaTxPartition::Vert5 {
                            (r.col, r.row)
                        } else {
                            (r.row, r.col)
                        })
                    );
                    let mut covered = [[false; 16]; 16];
                    for record in &records {
                        let (r, c) = (record.row - 8, record.col - 24);
                        assert!(r < rows && c < cols);
                        assert_eq!(record.rows, TX_HEIGHT[record.tx_size] as usize / MI_SIZE);
                        assert_eq!(record.cols, TX_WIDTH[record.tx_size] as usize / MI_SIZE);
                        for row in &mut covered[r..(r + record.rows).min(rows)] {
                            for cell in &mut row[c..(c + record.cols).min(cols)] {
                                assert!(!std::mem::replace(cell, true));
                            }
                        }
                    }
                    assert!(
                        covered[..rows]
                            .iter()
                            .all(|row| row[..cols].iter().all(|&v| v))
                    );
                }
            }
        }
    }
}

#[test]
fn tx_skip_grid_retention_preserves_skip_flag_for_nonzero_eob_record() {
    let records = [
        WienerNsLrTxSkipTransformRecord {
            row: 0,
            col: 0,
            rows: 1,
            cols: 1,
            skip_flag: true,
            empty: false,
        },
        WienerNsLrTxSkipTransformRecord {
            row: 0,
            col: 1,
            rows: 1,
            cols: 1,
            skip_flag: false,
            empty: false,
        },
    ];

    let tx_skip = derive_wienerns_lr_tx_skip_grid_retention(1, 2, &records).unwrap();

    assert_eq!(
        tx_skip
            .lookup(super::super::WienerNsLrTxSkipLookup { row: 0, col: 0 })
            .unwrap(),
        1
    );
    assert_eq!(
        tx_skip
            .lookup(super::super::WienerNsLrTxSkipLookup { row: 0, col: 1 })
            .unwrap(),
        0
    );
}

#[test]
fn cdef_skip_grid_writes_final_bit_grid_for_mixed_records() {
    let records = [
        WienerNsLrTxSkipTransformRecord {
            row: 0,
            col: 0,
            rows: 1,
            cols: 2,
            skip_flag: true,
            empty: false,
        },
        WienerNsLrTxSkipTransformRecord {
            row: 0,
            col: 2,
            rows: 1,
            cols: 1,
            skip_flag: false,
            empty: true,
        },
        WienerNsLrTxSkipTransformRecord {
            row: 1,
            col: 0,
            rows: 1,
            cols: 3,
            skip_flag: false,
            empty: true,
        },
    ];

    let actual = derive_cdef_skip_grid(2, 3, &records, Vec::new()).unwrap();
    let expected = crate::filters::cdef::CdefSkipGrid::new(2, 3, vec![1, 1, 0, 0, 0, 0]).unwrap();

    assert_eq!(actual, expected);
}

#[test]
fn cdef_skip_grid_clamps_boundary_overhang_without_changing_predicate() {
    let records = [
        WienerNsLrTxSkipTransformRecord {
            row: 0,
            col: 0,
            rows: 2,
            cols: 2,
            skip_flag: true,
            empty: false,
        },
        WienerNsLrTxSkipTransformRecord {
            row: 2,
            col: 0,
            rows: 3,
            cols: 2,
            skip_flag: false,
            empty: true,
        },
    ];

    let actual = derive_cdef_skip_grid(3, 2, &records, Vec::new()).unwrap();
    let expected = crate::filters::cdef::CdefSkipGrid::new(3, 2, vec![1, 1, 1, 1, 0, 0]).unwrap();

    assert_eq!(actual, expected);
}

#[test]
fn cdef_and_lr_keep_distinct_empty_transform_predicates() {
    let records = [WienerNsLrTxSkipTransformRecord {
        row: 0,
        col: 0,
        rows: 1,
        cols: 2,
        skip_flag: false,
        empty: true,
    }];

    let cdef = derive_cdef_skip_grid(1, 2, &records, Vec::new()).unwrap();
    let lr = derive_wienerns_lr_tx_skip_grid_retention(1, 2, &records).unwrap();

    assert_eq!(
        cdef,
        crate::filters::cdef::CdefSkipGrid::new(1, 2, vec![0, 0]).unwrap()
    );
    for col in 0..2 {
        assert_eq!(
            lr.lookup(super::super::WienerNsLrTxSkipLookup { row: 0, col })
                .unwrap(),
            1
        );
    }
}

#[test]
fn cdef_skip_grid_rejects_conflicting_overlapping_records() {
    assert!(matches!(
        derive_cdef_skip_grid(1, 2, &CONFLICTING_TX_SKIP_RECORDS, Vec::new()).unwrap_err(),
        ReconError::PcWienerInvalidBounds {
            field: "LrTxSkip conflicting transform records"
        }
    ));
}

/// §5.20.6.1 PC-Wiener `LrTxSkip` FilterClass grid retention drops the off-frame MI
/// cells of a bottom-edge skipped block instead of erroring on the overhang. Models
/// the mission-stream case: a skipped 16x16-MI block at MI(256,0) overhangs the 270-row
/// grid by 2; its in-frame rows 256..270 fill, the off-frame rows 270,271 are dropped
/// (they carry no FilterClass), mirroring AVM `av2_set_entropy_contexts` and the
/// §5.20.3.2 `block_coded` clamp. The full grid stays populated by the in-frame cells.
#[test]
fn tx_skip_grid_retention_clamps_bottom_edge_overhang() {
    let records = [
        WienerNsLrTxSkipTransformRecord {
            row: 0,
            col: 0,
            rows: 2,
            cols: 1,
            skip_flag: true,
            empty: true,
        },
        WienerNsLrTxSkipTransformRecord {
            row: 2,
            col: 0,
            rows: 4, // nominal extent overhangs the 4-row grid by 2 rows
            cols: 1,
            skip_flag: false,
            empty: false,
        },
    ];

    let tx_skip = derive_wienerns_lr_tx_skip_grid_retention(4, 1, &records).unwrap();

    let column: Vec<i32> = (0..4)
        .map(|row| {
            tx_skip
                .lookup(super::super::WienerNsLrTxSkipLookup { row, col: 0 })
                .unwrap()
        })
        .collect();
    assert_eq!(column, vec![1, 1, 0, 0]);
}

/// A right-edge analogue: a record whose nominal width overhangs MiCols fills only its
/// in-frame columns; the past-edge columns are dropped, not errored.
#[test]
fn tx_skip_grid_retention_clamps_right_edge_overhang() {
    let records = [WienerNsLrTxSkipTransformRecord {
        row: 0,
        col: 0,
        rows: 1,
        cols: 4, // nominal width overhangs the 2-col grid by 2 cols
        skip_flag: true,
        empty: true,
    }];

    let tx_skip = derive_wienerns_lr_tx_skip_grid_retention(1, 2, &records).unwrap();

    for col in 0..2 {
        assert_eq!(
            tx_skip
                .lookup(super::super::WienerNsLrTxSkipLookup { row: 0, col })
                .unwrap(),
            1
        );
    }
}

/// Two records covering the same MI cell with DISAGREEING `LrTxSkip` values are a
/// hard error: the grid is a single-writer surface, so an overlap that would silently
/// pick one value must fail closed instead.
#[test]
fn tx_skip_grid_retention_rejects_conflicting_overlapping_records() {
    assert!(matches!(
        derive_wienerns_lr_tx_skip_grid_retention(1, 2, &CONFLICTING_TX_SKIP_RECORDS).unwrap_err(),
        ReconError::PcWienerInvalidBounds {
            field: "LrTxSkip conflicting transform records"
        }
    ));
}

/// A genuine out-of-frame ORIGIN (`row >= rows` or `col >= cols`) is STILL a hard
/// error, matching the §5.20.3.2 `block_coded` model: AVM never emits such a record.
#[test]
fn tx_skip_grid_retention_rejects_out_of_frame_origin() {
    let records = [WienerNsLrTxSkipTransformRecord {
        row: 4, // origin at/beyond the 4-row grid
        col: 0,
        rows: 1,
        cols: 1,
        skip_flag: true,
        empty: true,
    }];

    assert!(matches!(
        derive_wienerns_lr_tx_skip_grid_retention(4, 1, &records).unwrap_err(),
        ReconError::PcWienerInvalidBounds {
            field: "LrTxSkip transform record bounds"
        }
    ));
}

#[test]
fn selectable_tx_records_populate_live_tx_skip_grid() {
    let mut records = Vec::new();
    collect_inter_luma_transforms(
        (0, 0),
        Block4x4Extent { rows: 8, cols: 8 },
        (8, 8),
        TX_32X32,
        LumaTxPartition::Split,
        &mut records,
    )
    .unwrap();
    let tx_skip_records = records
        .iter()
        .enumerate()
        .map(|(index, record)| WienerNsLrTxSkipTransformRecord {
            row: record.row as u16,
            col: record.col as u16,
            rows: record.rows as u8,
            cols: record.cols as u8,
            skip_flag: false,
            empty: index != 0,
        })
        .collect::<Vec<_>>();

    let tx_skip = derive_wienerns_lr_tx_skip_grid_retention(8, 8, &tx_skip_records).unwrap();
    assert_eq!(
        (0..8)
            .map(|row| tx_skip
                .lookup(super::super::WienerNsLrTxSkipLookup { row, col: 0 })
                .unwrap())
            .collect::<Vec<_>>(),
        vec![0, 0, 0, 0, 1, 1, 1, 1]
    );
}

#[test]
fn unpartitioned_chunks_retain_their_full_size_at_the_frame_edges() {
    let mut records = Vec::new();
    collect_inter_luma_transforms(
        (256, 16),
        Block4x4Extent { rows: 32, cols: 32 },
        (270, 46),
        TX_64X64,
        LumaTxPartition::None,
        &mut records,
    )
    .unwrap();
    assert_eq!(
        records
            .iter()
            .map(|r| (r.row, r.col, r.rows, r.cols, r.tx_size))
            .collect::<Vec<_>>(),
        [(256, 16, 16, 16, TX_64X64), (256, 32, 16, 16, TX_64X64)]
    );
}

#[test]
fn selectable_transform_record_failures_are_typed_header_state_errors() {
    assert!(matches!(
        selectable_transform_record_error(SelectableTransformRecordError::TableIndex {
            table: "Tx_Width",
            index: usize::MAX
        }),
        crate::error::DecodeError::HeaderState {
            source: crate::error::DecodeHeaderStateError::InvalidSelectableTransformRecords,
        }
    ));
    assert!(matches!(
        selectable_symbol_read_error(ByteOffset::new(9), "5.20.5.11"),
        crate::error::DecodeError::MalformedSource { .. }
    ));
}

// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Loop-restoration source-read regression tests for the minimal inter runtime.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::Cell;

use splot_core::headers::frame::{
    FrameRestorationType, LrPlaneParams, WienerNsFrameFilterBank, WienerNsFrameFilterClass,
};
use splot_core::headers::sequence::{BitDepthIdc, ChromaFormatIdc};
use splot_core::span::ByteOffset;
use splot_recon::{
    BitDepth, DecodedFrame, DecodedFrameInfo, FramePlanes, LoopRestorationSource,
    LoopRestorationSourceBounds, OutputIndex, PixelFormat, Plane, PlaneId, PlaneRect, PlaneSize,
    ReconError,
};

use super::test_support::fixture_sequence_and_key_core;
use crate::error::DecodeError;
use crate::tile_payload::WienerNsLrSourceBlock;
use crate::{DecodeLimitName, DecodeLimitThreshold, DecodeLimits};

const TWO_FRAME_INTER_FIXTURE: &[u8] =
    include_bytes!("../../../../../tests/conformance/vectors/valid/syn-2frame-inter-64x64.ivf");

fn wienerns_lr_source_block() -> WienerNsLrSourceBlock {
    WienerNsLrSourceBlock {
        plane: 0,
        row: 0,
        col: 0,
        unit_row: 0,
        unit_col: 0,
        tile_mi_row_start: 0,
        tile_mi_row_end: 4,
        tile_mi_col_start: 0,
        tile_mi_col_end: 4,
        x: 0,
        y: 6,
        width: 4,
        height: 4,
        luma_start_x: 0,
        luma_end_x: 15,
        luma_start_y: 0,
        luma_end_y: 15,
        frame_luma_end_y: 15,
        luma_stripe_start_y: 8,
        luma_stripe_end_y: 10,
    }
}

fn wienerns_lr_source_read_config() -> super::super::WienerNsLrSourceReadConfig {
    super::super::WienerNsLrSourceReadConfig::CONSERVATIVE
}

fn plane_size(width: usize, height: usize) -> PlaneSize {
    PlaneSize::new(width, height).unwrap()
}

fn plane_rect(x: usize, y: usize, width: usize, height: usize) -> PlaneRect {
    PlaneRect::new(x, y, width, height).unwrap()
}

fn flat_monochrome_frame_u16(value: u16) -> DecodedFrame<u16> {
    let size = plane_size(16, 16);
    let rect = plane_rect(0, 0, 16, 16);
    let info = DecodedFrameInfo::new(
        OutputIndex::new(0),
        BitDepth::Ten,
        PixelFormat::Monochrome,
        size,
        rect,
    )
    .unwrap();
    DecodedFrame::try_new(
        info,
        FramePlanes::new(
            Plane::from_vec(size, 16, rect, vec![value; 16 * 16]).unwrap(),
            None,
            None,
        ),
    )
    .unwrap()
}

fn tx_skip_grid(values: Vec<u8>) -> super::super::WienerNsLrTxSkipGrid {
    super::super::WienerNsLrTxSkipGrid::new(4, 4, values).unwrap()
}

fn storage_inputs<'a>(
    curr_frame: &'a DecodedFrame<u16>,
    cdef_frame: &'a DecodedFrame<u16>,
    tx_skip_grid: &'a super::super::WienerNsLrTxSkipGrid,
) -> super::super::WienerNsLrClassifiedWienerStorageInputs<'a, u16> {
    super::super::WienerNsLrClassifiedWienerStorageInputs {
        curr_frame,
        cdef_frame,
        tx_skip_grid,
    }
}

#[test]
fn wienerns_lr_source_read_frontier_preflights_limit_before_coordinates() {
    let blocks = [WienerNsLrSourceBlock {
        luma_start_x: 32,
        luma_end_x: 31,
        ..wienerns_lr_source_block()
    }];
    let limits = DecodeLimits::unlimited()
        .with_max_loop_restoration_source_reads(DecodeLimitThreshold::Max(0));

    let error = super::super::derive_wienerns_lr_source_read_frontier(
        &blocks,
        ChromaFormatIdc::Yuv420,
        wienerns_lr_source_read_config(),
        ByteOffset::new(74),
        limits,
    )
    .unwrap_err();

    match error {
        DecodeError::Limit { source } => {
            assert_eq!(
                source.name(),
                DecodeLimitName::MaxLoopRestorationSourceReads
            );
            let check = source.check().expect("limit failure carries check");
            assert_eq!(check.threshold(), DecodeLimitThreshold::Max(0));
            assert_eq!(check.actual(), 528);
        }
        _ => panic!("source-read budget must be checked before coordinate enumeration"),
    }
}

#[test]
fn wienerns_lr_source_read_frontier_skips_zero_chroma_luma_taps() {
    let blocks = [WienerNsLrSourceBlock {
        plane: 1,
        ..wienerns_lr_source_block()
    }];
    let mut config = wienerns_lr_source_read_config();
    config.chroma_luma_source_taps[PlaneId::U.index()] =
        [false; super::super::WIENER_NS_CHROMA_SOURCE_TAP_COUNT];
    let expected_output_samples = 16;
    let expected_source_reads = expected_output_samples * (1 + 12 + 4);

    let frontier = super::super::derive_wienerns_lr_source_read_frontier(
        &blocks,
        ChromaFormatIdc::Yuv420,
        config,
        ByteOffset::new(74),
        DecodeLimits::unlimited(),
    )
    .expect("source-read frontier");

    assert_eq!(frontier.output_samples_resolved, expected_output_samples);
    assert_eq!(frontier.source_reads_resolved, expected_source_reads);
}

#[test]
fn wienerns_lr_source_read_frontier_honors_cfl_filter_index_two() {
    let blocks = [WienerNsLrSourceBlock {
        plane: 1,
        ..wienerns_lr_source_block()
    }];
    let mut config = wienerns_lr_source_read_config();
    config.cfl_ds_filter_index = 2;
    config.chroma_luma_source_taps[PlaneId::U.index()] =
        [false; super::super::WIENER_NS_CHROMA_SOURCE_TAP_COUNT];
    let expected_output_samples = 16;
    let expected_source_reads = expected_output_samples * (1 + 12 + 1);

    let frontier = super::super::derive_wienerns_lr_source_read_frontier(
        &blocks,
        ChromaFormatIdc::Yuv420,
        config,
        ByteOffset::new(74),
        DecodeLimits::unlimited(),
    )
    .expect("source-read frontier");

    assert_eq!(frontier.output_samples_resolved, expected_output_samples);
    assert_eq!(frontier.source_reads_resolved, expected_source_reads);
}

#[test]
fn wienerns_lr_source_read_config_uses_parsed_chroma_coefficients() {
    let planes = [
        lr_plane(false, None, None),
        lr_plane(
            true,
            None,
            Some(WienerNsFrameFilterBank {
                classes: vec![WienerNsFrameFilterClass {
                    match_index: 0,
                    merged: true,
                    ref_bank: 0,
                    subset: None,
                    wiener_ns_uv_sym: false,
                    coeffs: vec![0; 18],
                }],
            }),
        ),
    ];

    let config = super::super::wienerns_lr_source_read_config(&planes, 2);

    assert_eq!(
        config.cfl_ds_filter_index, 2,
        "parsed sequence cfl_ds_filter_index is retained for §7.20.3 luma reads"
    );
    assert_eq!(
        config.chroma_luma_source_taps[PlaneId::U.index()],
        [false; super::super::WIENER_NS_CHROMA_SOURCE_TAP_COUNT]
    );
    assert_eq!(
        config.chroma_luma_source_taps[PlaneId::V.index()],
        [true; super::super::WIENER_NS_CHROMA_SOURCE_TAP_COUNT]
    );
}

#[test]
fn wienerns_lr_source_read_frontier_uses_frame_y_bound_for_chroma_luma_reads() {
    let bounds = LoopRestorationSourceBounds {
        luma_start_x: 0,
        luma_end_x: 15,
        luma_start_y: 0,
        luma_end_y: 7,
        luma_stripe_start_y: 0,
        luma_stripe_end_y: 7,
        subsampling_x: 1,
        subsampling_y: 1,
    };
    let mut summary = super::super::WienerNsLrSourceReadFrontier {
        blocks_resolved: 0,
        output_samples_resolved: 0,
        source_reads_resolved: 0,
        curr_frame_source_reads: 0,
        cdef_frame_source_reads: 0,
        first_sample: None,
    };

    super::super::record_wienerns_lr_chroma_luma_source_reads(
        &mut summary,
        wienerns_lr_source_read_config(),
        0,
        7,
        &bounds,
        15,
    )
    .expect("chroma luma source reads");

    assert_eq!(
        summary.first_sample,
        Some(super::super::WienerNsLrSourceReadSample {
            plane: PlaneId::Y,
            x: 0,
            y: 7,
            source: LoopRestorationSource::CdefFrame,
        })
    );
}

#[test]
fn wienerns_lr_source_read_frontier_rejects_unit_coded_chroma_before_source_reads() {
    let blocks = [WienerNsLrSourceBlock {
        plane: PlaneId::U.index(),
        ..wienerns_lr_source_block()
    }];
    let planes = [
        lr_plane(true, None, None),
        lr_plane(false, None, None),
        lr_plane(true, None, None),
    ];
    let config = super::super::wienerns_lr_source_read_config(&planes, 0);
    let limits = DecodeLimits::unlimited()
        .with_max_loop_restoration_source_reads(DecodeLimitThreshold::Max(0));

    let error = super::super::derive_wienerns_lr_runtime_source_frontiers(
        &blocks,
        &planes,
        ChromaFormatIdc::Yuv420,
        config,
        ByteOffset::new(74),
        limits,
    )
    .unwrap_err();

    let DecodeError::UnsupportedFeature { unsupported } = error else {
        panic!("unit-coded chroma filters must fail closed before source-read accounting");
    };
    assert_eq!(
        unsupported.reason(),
        "unsupported_wienerns_lr_unit_chroma_filter_values"
    );
    assert_eq!(unsupported.matrix_row(), "ac0ej3-lr-source-read-frontier");
    assert_eq!(
        unsupported.feature_id(),
        "DECODE-AC0EJ3-LR-SOURCE-READ-FRONTIER"
    );
    assert_eq!(unsupported.spec_section(), "5.20.10.6");
    assert_eq!(unsupported.byte_offset(), Some(ByteOffset::new(74)));
}

#[test]
fn wienerns_lr_classified_wiener_frontier_resolves_dependency_coordinates() {
    let blocks = [wienerns_lr_source_block()];
    let planes = [lr_plane(true, Some(2), None)];

    let frontier = super::super::derive_wienerns_lr_classified_wiener_frontier(
        &blocks,
        &planes,
        DecodeLimits::unlimited(),
    )
    .expect("classified Wiener frontier")
    .expect("classified luma is active");

    assert_eq!(
        frontier.blocks_resolved, 1,
        "classification runs once for the retained 4x4 luma LR block"
    );
    assert_eq!(
        frontier.feature_points_resolved, 36,
        "PC_WIENER_LEAD=1 and PC_WIENER_LAG=4 give a 6x6 feature window"
    );
    assert_eq!(frontier.source_reads_resolved, 36 * 7);
    assert_eq!(frontier.tx_skip_lookups_resolved, 36);
    assert_eq!(
        frontier.curr_frame_source_reads + frontier.cdef_frame_source_reads,
        frontier.source_reads_resolved
    );
    assert_eq!(
        frontier.first_sample,
        Some(super::super::WienerNsLrSourceReadSample {
            plane: PlaneId::Y,
            x: 0,
            y: 6,
            source: LoopRestorationSource::CurrFrame,
        })
    );
    assert_eq!(
        frontier.first_tx_skip_lookup,
        Some(super::super::WienerNsLrTxSkipLookup {
            x: 0,
            y: 8,
            row: 2,
            col: 0,
        })
    );
}

#[test]
fn wienerns_lr_classified_wiener_values_frontier_derives_filter_class() {
    let blocks = [wienerns_lr_source_block()];
    let planes = [lr_plane(true, Some(2), None)];
    let source_calls = Cell::new(0usize);
    let tx_skip_calls = Cell::new(0usize);
    let first_source = Cell::new(None);

    let frontier = super::super::derive_wienerns_lr_classified_wiener_values_frontier::<u8, _, _>(
        &blocks,
        &planes,
        BitDepth::Eight,
        0,
        DecodeLimits::unlimited(),
        |read| {
            source_calls.set(source_calls.get() + 1);
            if first_source.get().is_none() {
                first_source.set(Some(read));
            }
            Ok(12)
        },
        |_| {
            tx_skip_calls.set(tx_skip_calls.get() + 1);
            Ok(0)
        },
    )
    .expect("classified Wiener values frontier")
    .expect("classified luma is active");

    assert_eq!(frontier.blocks_resolved, 1);
    assert_eq!(frontier.source_reads_resolved, 36 * 7);
    assert_eq!(
        frontier.curr_frame_source_reads + frontier.cdef_frame_source_reads,
        frontier.source_reads_resolved
    );
    assert!(
        frontier.curr_frame_source_reads > 0,
        "boundary feature reads should select CurrFrame samples"
    );
    assert!(
        frontier.cdef_frame_source_reads > 0,
        "in-stripe feature reads should select CdefFrame samples"
    );
    assert_eq!(frontier.filter_classes_resolved, 1);
    assert_eq!(
        first_source.get(),
        Some(super::super::WienerNsLrClassifiedWienerValueSourceSample {
            input_x: -1,
            input_y: 5,
            bounds: LoopRestorationSourceBounds {
                luma_start_x: 0,
                luma_end_x: 15,
                luma_start_y: 0,
                luma_end_y: 15,
                luma_stripe_start_y: 8,
                luma_stripe_end_y: 10,
                subsampling_x: 0,
                subsampling_y: 0,
            },
            sample: super::super::WienerNsLrSourceReadSample {
                plane: PlaneId::Y,
                x: 0,
                y: 6,
                source: LoopRestorationSource::CurrFrame,
            },
        })
    );
    assert_eq!(frontier.first_sample, first_source.get());
    assert_eq!(
        frontier.first_filter_class,
        Some(super::super::WienerNsLrFilterClassValue {
            x: 0,
            y: 6,
            row: 1,
            col: 0,
            class: 83,
        })
    );
    assert_eq!(source_calls.get(), 36 * 7);
    assert_eq!(tx_skip_calls.get(), 36);
}

#[test]
fn wienerns_lr_classified_wiener_values_frontier_preflights_limit_before_reads() {
    let blocks = [wienerns_lr_source_block()];
    let planes = [lr_plane(true, Some(2), None)];
    let source_calls = Cell::new(0usize);
    let tx_skip_calls = Cell::new(0usize);
    let limits = DecodeLimits::unlimited()
        .with_max_loop_restoration_source_reads(DecodeLimitThreshold::Max(0));

    let error = super::super::derive_wienerns_lr_classified_wiener_values_frontier::<u8, _, _>(
        &blocks,
        &planes,
        BitDepth::Eight,
        0,
        limits,
        |_| {
            source_calls.set(source_calls.get() + 1);
            Ok(12)
        },
        |_| {
            tx_skip_calls.set(tx_skip_calls.get() + 1);
            Ok(0)
        },
    )
    .unwrap_err();

    match error {
        DecodeError::Limit { source } => {
            assert_eq!(
                source.name(),
                DecodeLimitName::MaxLoopRestorationSourceReads
            );
            let check = source.check().expect("limit failure carries check");
            assert_eq!(check.threshold(), DecodeLimitThreshold::Max(0));
            assert_eq!(check.actual(), 252);
        }
        _ => panic!("classified value reads must preflight source-read budget"),
    }
    assert_eq!(source_calls.get(), 0);
    assert_eq!(tx_skip_calls.get(), 0);
}

#[test]
fn wienerns_lr_classified_wiener_values_frontier_propagates_invalid_tx_skip() {
    let blocks = [wienerns_lr_source_block()];
    let planes = [lr_plane(true, Some(2), None)];

    let error = super::super::derive_wienerns_lr_classified_wiener_values_frontier::<u8, _, _>(
        &blocks,
        &planes,
        BitDepth::Eight,
        0,
        DecodeLimits::unlimited(),
        |_| Ok(12),
        |_| Ok(2),
    )
    .unwrap_err();

    match error {
        DecodeError::Reconstruction {
            source:
                ReconError::PcWienerInvalidTxSkip {
                    x,
                    y,
                    row,
                    col,
                    value,
                },
        } => {
            assert_eq!((x, y, row, col, value), (0, 8, 2, 0, 2));
        }
        _ => panic!("invalid LrTxSkip must remain a structured reconstruction error"),
    }
}

#[test]
fn wienerns_lr_classified_wiener_storage_frontier_reads_frame_and_tx_skip_storage() {
    let blocks = [wienerns_lr_source_block()];
    let planes = [lr_plane(true, Some(2), None)];
    let curr_frame = flat_monochrome_frame_u16(12);
    let cdef_frame = flat_monochrome_frame_u16(12);
    let tx_skip = tx_skip_grid(vec![0; 16]);

    let frontier = super::super::derive_wienerns_lr_classified_wiener_storage_frontier(
        &blocks,
        &planes,
        BitDepth::Ten,
        0,
        DecodeLimits::unlimited(),
        storage_inputs(&curr_frame, &cdef_frame, &tx_skip),
    )
    .expect("storage-backed classified Wiener frontier")
    .expect("classified luma is active");

    assert_eq!(frontier.blocks_resolved, 1);
    assert_eq!(frontier.source_reads_resolved, 36 * 7);
    assert!(
        frontier.curr_frame_source_reads > 0,
        "storage adapter must read selected CurrFrame samples"
    );
    assert!(
        frontier.cdef_frame_source_reads > 0,
        "storage adapter must read selected CdefFrame samples"
    );
    assert_eq!(frontier.filter_classes_resolved, 1);
    assert_eq!(
        frontier.first_filter_class,
        Some(super::super::WienerNsLrFilterClassValue {
            x: 0,
            y: 6,
            row: 1,
            col: 0,
            class: 83,
        })
    );
}

#[test]
fn wienerns_lr_tx_skip_grid_retention_derives_spec_values() {
    let records = [
        super::super::WienerNsLrTxSkipTransformRecord {
            row: 0,
            col: 0,
            rows: 1,
            cols: 1,
            skip_flag: false,
            eob: 7,
            intra_ist: None,
        },
        super::super::WienerNsLrTxSkipTransformRecord {
            row: 0,
            col: 1,
            rows: 1,
            cols: 2,
            skip_flag: true,
            eob: 7,
            intra_ist: None,
        },
        super::super::WienerNsLrTxSkipTransformRecord {
            row: 1,
            col: 0,
            rows: 1,
            cols: 3,
            skip_flag: false,
            eob: 0,
            intra_ist: None,
        },
    ];

    let grid = super::super::derive_wienerns_lr_tx_skip_grid_retention(2, 3, &records)
        .expect("complete LrTxSkip grid");

    let value = |row, col| {
        grid.lookup(super::super::WienerNsLrTxSkipLookup {
            x: col * 4,
            y: row * 4,
            row,
            col,
        })
        .unwrap()
    };
    assert_eq!(value(0, 0), 0, "non-skipped nonzero eob stores false");
    assert_eq!(value(0, 1), 1, "skip_flag stores true");
    assert_eq!(value(0, 2), 1, "skip_flag fills the whole transform extent");
    assert_eq!(value(1, 0), 1, "eob == 0 stores true");
    assert_eq!(value(1, 2), 1, "eob == 0 fills the whole transform extent");
}

#[test]
fn wienerns_lr_tx_skip_grid_retention_rejects_missing_cells() {
    let records = [super::super::WienerNsLrTxSkipTransformRecord {
        row: 0,
        col: 0,
        rows: 1,
        cols: 1,
        skip_flag: false,
        eob: 7,
        intra_ist: None,
    }];

    let error = super::super::derive_wienerns_lr_tx_skip_grid_retention(2, 2, &records)
        .expect_err("incomplete grid must be rejected");

    match error {
        ReconError::BufferLengthMismatch { expected, actual } => {
            assert_eq!((expected, actual), (4, 1));
        }
        _ => panic!("missing LrTxSkip cells must be a structured buffer mismatch"),
    }
}

/// A record whose ORIGIN is genuinely out of the visible MI grid (`row >= rows` or
/// `col >= cols`) is a structured bounds error — AVM never emits one. A record whose
/// in-frame origin merely has an overhanging NOMINAL extent is instead clamped to the
/// frame edge (the §5.20.3.2 `block_coded` model), covered by the grid `clamps_*_edge`
/// tests in `tx_records_grid_tests.rs`.
#[test]
fn wienerns_lr_tx_skip_grid_retention_rejects_out_of_frame_origin_record() {
    let records = [super::super::WienerNsLrTxSkipTransformRecord {
        row: 2, // origin at/beyond the 2-row grid: a genuine out-of-frame write
        col: 0,
        rows: 2,
        cols: 1,
        skip_flag: false,
        eob: 7,
        intra_ist: None,
    }];

    let error = super::super::derive_wienerns_lr_tx_skip_grid_retention(2, 2, &records)
        .expect_err("out-of-frame origin must be rejected");

    match error {
        ReconError::PcWienerInvalidBounds { field } => {
            assert_eq!(field, "LrTxSkip transform record bounds");
        }
        _ => panic!("out-of-frame LrTxSkip origin must be a structured bounds error"),
    }
}

#[test]
fn wienerns_lr_tx_skip_grid_retention_rejects_conflicting_records() {
    let records = [
        super::super::WienerNsLrTxSkipTransformRecord {
            row: 0,
            col: 0,
            rows: 1,
            cols: 1,
            skip_flag: false,
            eob: 7,
            intra_ist: None,
        },
        super::super::WienerNsLrTxSkipTransformRecord {
            row: 0,
            col: 0,
            rows: 1,
            cols: 1,
            skip_flag: true,
            eob: 7,
            intra_ist: None,
        },
    ];

    let error = super::super::derive_wienerns_lr_tx_skip_grid_retention(1, 1, &records)
        .expect_err("conflicting transform records must be rejected");

    match error {
        ReconError::PcWienerInvalidBounds { field } => {
            assert_eq!(field, "LrTxSkip conflicting transform records");
        }
        _ => panic!("conflicting LrTxSkip records must be a structured bounds error"),
    }
}

#[test]
fn wienerns_lr_classified_wiener_storage_frontier_propagates_tx_skip_grid_bounds() {
    let blocks = [wienerns_lr_source_block()];
    let planes = [lr_plane(true, Some(2), None)];
    let curr_frame = flat_monochrome_frame_u16(12);
    let cdef_frame = flat_monochrome_frame_u16(12);
    let tx_skip = super::super::WienerNsLrTxSkipGrid::new(1, 1, vec![0]).unwrap();

    let error = super::super::derive_wienerns_lr_classified_wiener_storage_frontier(
        &blocks,
        &planes,
        BitDepth::Ten,
        0,
        DecodeLimits::unlimited(),
        storage_inputs(&curr_frame, &cdef_frame, &tx_skip),
    )
    .unwrap_err();

    match error {
        DecodeError::Reconstruction {
            source: ReconError::PcWienerInvalidBounds { field },
        } => {
            assert_eq!(field, "LrTxSkip grid lookup");
        }
        _ => panic!("tx-skip storage bounds must remain a structured reconstruction error"),
    }
}

#[test]
fn wienerns_lr_classified_wiener_storage_frontier_propagates_non_boolean_tx_skip() {
    let blocks = [wienerns_lr_source_block()];
    let planes = [lr_plane(true, Some(2), None)];
    let curr_frame = flat_monochrome_frame_u16(12);
    let cdef_frame = flat_monochrome_frame_u16(12);
    let mut values = vec![0; 16];
    values[8] = 2;
    let tx_skip = tx_skip_grid(values);

    let error = super::super::derive_wienerns_lr_classified_wiener_storage_frontier(
        &blocks,
        &planes,
        BitDepth::Ten,
        0,
        DecodeLimits::unlimited(),
        storage_inputs(&curr_frame, &cdef_frame, &tx_skip),
    )
    .unwrap_err();

    match error {
        DecodeError::Reconstruction {
            source:
                ReconError::PcWienerInvalidTxSkip {
                    x,
                    y,
                    row,
                    col,
                    value,
                },
        } => {
            assert_eq!((x, y, row, col, value), (0, 8, 2, 0, 2));
        }
        _ => panic!("non-boolean tx-skip storage must remain a structured reconstruction error"),
    }
}

#[test]
fn classified_wiener_storage_runtime_error_reports_retention_frontier() {
    let error =
        super::super::wienerns_lr_classified_wiener_storage_runtime_error(ByteOffset::new(74));
    let DecodeError::UnsupportedFeature { unsupported } = error else {
        panic!("classified Wiener storage frontier must be an unsupported-feature error");
    };

    assert_eq!(
        unsupported.reason(),
        "unsupported_wienerns_lr_classified_wiener_runtime_storage"
    );
    assert_eq!(
        unsupported.matrix_row(),
        "ac0ej3-lr-classified-wiener-storage"
    );
    assert_eq!(
        unsupported.feature_id(),
        "DECODE-AC0EJ3-LR-CLASSIFIED-WIENER-STORAGE"
    );
    assert_eq!(unsupported.spec_section(), "7.20.4");
    assert_eq!(unsupported.byte_offset(), Some(ByteOffset::new(74)));
    assert!(
        unsupported.message().contains("source-read"),
        "message should say classified source-read dependencies are resolved"
    );
    assert!(
        unsupported
            .message()
            .contains("LrTxSkip lookup coordinates"),
        "message should say tx-skip coordinates are resolved"
    );
    assert!(
        unsupported.message().contains("storage-backed FilterClass"),
        "message should say storage-backed classification is wired"
    );
    assert!(
        unsupported
            .message()
            .contains("decoded 10-bit frame buffers"),
        "message should name the remaining frame retention boundary"
    );
    assert!(
        unsupported.message().contains("retained for filtering"),
        "message should not claim live filter-time storage retention"
    );
    assert!(
        unsupported.message().contains("loop-restoration filtering"),
        "message should keep filtering out of scope"
    );
}

#[test]
fn wienerns_lr_runtime_storage_retention_frontier_counts_ten_bit_buffers_and_tx_skip_grid() {
    let (mut sequence, core) = fixture_sequence_and_key_core(TWO_FRAME_INTER_FIXTURE);
    sequence.general.bit_depth_idc = BitDepthIdc::Ten;
    let frontier = super::super::derive_wienerns_lr_runtime_storage_retention_frontier(
        &sequence,
        &core,
        ByteOffset::new(74),
        DecodeLimits::unlimited(),
    )
    .expect("retention frontier");

    assert_eq!(frontier.bit_depth, BitDepth::Ten);
    assert_eq!(frontier.frame_buffer_count, 2);
    assert_eq!(frontier.frame_buffer_bytes, 64 * 64 * 3 / 2 * 2);
    assert_eq!(
        frontier.retained_frame_buffer_bytes,
        64 * 64 * 3 / 2 * super::super::LR_LIVE_FRAME_SAMPLE_STORAGE_BYTES * 2
    );
    assert_eq!((frontier.tx_skip_rows, frontier.tx_skip_cols), (16, 16));
    assert_eq!(frontier.tx_skip_values, 16 * 16);
    assert_eq!(
        frontier.total_storage_bytes,
        frontier.retained_frame_buffer_bytes
            + frontier.tx_skip_values * super::super::LR_LIVE_TX_SKIP_STORAGE_BYTES_PER_VALUE
    );
}

#[test]
fn wienerns_lr_runtime_storage_retention_frontier_limits_total_storage_before_diagnostic() {
    let (mut sequence, core) = fixture_sequence_and_key_core(TWO_FRAME_INTER_FIXTURE);
    sequence.general.bit_depth_idc = BitDepthIdc::Ten;
    let actual_storage_bytes =
        64_u64 * 64 * 3 / 2 * super::super::LR_LIVE_FRAME_SAMPLE_STORAGE_BYTES * 2
            + 16_u64 * 16 * super::super::LR_LIVE_TX_SKIP_STORAGE_BYTES_PER_VALUE;
    let limits = DecodeLimits::unlimited()
        .with_max_decoded_frame_bytes(DecodeLimitThreshold::Max(12_288))
        .with_max_reference_store_bytes(DecodeLimitThreshold::Max(actual_storage_bytes - 1));

    let error = super::super::derive_wienerns_lr_runtime_storage_retention_frontier(
        &sequence,
        &core,
        ByteOffset::new(74),
        limits,
    )
    .unwrap_err();

    match error {
        DecodeError::Limit { source } => {
            assert_eq!(source.name(), DecodeLimitName::MaxReferenceStoreBytes);
            let check = source.check().expect("limit failure carries check");
            assert_eq!(
                check.threshold(),
                DecodeLimitThreshold::Max(actual_storage_bytes - 1)
            );
            assert_eq!(check.actual(), actual_storage_bytes);
        }
        _ => panic!("storage retention must fail as a resource limit"),
    }
}

#[test]
fn wienerns_lr_runtime_storage_retention_error_reports_unpopulated_boundary() {
    let error = super::super::wienerns_lr_runtime_storage_retention_error(ByteOffset::new(74));
    let DecodeError::UnsupportedFeature { unsupported } = error else {
        panic!("runtime storage-retention frontier must be an unsupported-feature error");
    };

    assert_eq!(
        unsupported.reason(),
        "unsupported_wienerns_lr_runtime_storage_unpopulated"
    );
    assert_eq!(
        unsupported.matrix_row(),
        "ac0ej3-lr-runtime-storage-retention"
    );
    assert_eq!(
        unsupported.feature_id(),
        "DECODE-AC0EJ3-LR-RUNTIME-STORAGE-RETENTION"
    );
    assert_eq!(unsupported.spec_section(), "7.20.4");
    assert_eq!(unsupported.byte_offset(), Some(ByteOffset::new(74)));
    assert!(
        unsupported
            .message()
            .contains("active-bit-depth CurrFrame/CdefFrame"),
        "message should name the retained frame-storage shape"
    );
    assert!(
        unsupported.message().contains("LrTxSkip grid shape"),
        "message should name the tx-skip storage shape"
    );
    assert!(
        unsupported
            .message()
            .contains("has not populated decoded frame samples"),
        "message should not claim populated source samples"
    );
    assert!(
        unsupported.message().contains("not applied"),
        "message should not claim loop-restoration output"
    );
}

#[test]
fn wienerns_lr_runtime_frontier_preflights_classified_and_filter_reads_together() {
    let blocks = [wienerns_lr_source_block()];
    let planes = [lr_plane(true, Some(2), None)];
    let limits = DecodeLimits::unlimited()
        .with_max_loop_restoration_source_reads(DecodeLimitThreshold::Max(779));

    let error = super::super::derive_wienerns_lr_runtime_source_frontiers(
        &blocks,
        &planes,
        ChromaFormatIdc::Yuv420,
        wienerns_lr_source_read_config(),
        ByteOffset::new(74),
        limits,
    )
    .unwrap_err();

    match error {
        DecodeError::Limit { source } => {
            assert_eq!(
                source.name(),
                DecodeLimitName::MaxLoopRestorationSourceReads
            );
            let check = source.check().expect("limit failure carries check");
            assert_eq!(check.threshold(), DecodeLimitThreshold::Max(779));
            assert_eq!(check.actual(), 780);
        }
        _ => panic!("classified plus filter source reads must share the source-read budget"),
    }
}

fn lr_plane(
    frame_filters_on: bool,
    num_filter_classes: Option<u8>,
    frame_filter_bank: Option<WienerNsFrameFilterBank>,
) -> LrPlaneParams {
    LrPlaneParams {
        restoration_type: FrameRestorationType::WienerNonsep,
        frame_filters_on,
        num_filter_classes,
        frame_filter_bank,
    }
}

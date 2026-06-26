// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Live loop-restoration storage allocation regression tests.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use splot_core::headers::sequence::BitDepthIdc;
use splot_core::span::ByteOffset;
use splot_recon::{BitDepth, ReconError};

use super::test_support::fixture_sequence_and_key_core;
use crate::error::DecodeError;
use crate::{DecodeLimitThreshold, DecodeLimits};

const TWO_FRAME_INTER_FIXTURE: &[u8] =
    include_bytes!("../../../../../tests/conformance/vectors/valid/syn-2frame-inter-64x64.ivf");

fn valid_live_storage_frontier() -> super::super::WienerNsLrRuntimeStorageRetentionFrontier {
    let frame_sample_count = 64_u64 * 64 * 3 / 2;
    let retained_frame_buffer_bytes =
        frame_sample_count * super::super::LR_LIVE_FRAME_SAMPLE_STORAGE_BYTES * 2;
    let tx_skip_values = 16_u64 * 16;
    let tx_skip_storage_bytes =
        tx_skip_values * super::super::LR_LIVE_TX_SKIP_STORAGE_BYTES_PER_VALUE;
    super::super::WienerNsLrRuntimeStorageRetentionFrontier {
        bit_depth: BitDepth::Ten,
        frame_buffer_count: 2,
        frame_buffer_bytes: 12_288,
        retained_frame_buffer_bytes,
        tx_skip_rows: 16,
        tx_skip_cols: 16,
        tx_skip_values,
        total_storage_bytes: retained_frame_buffer_bytes + tx_skip_storage_bytes,
    }
}

fn assert_live_storage_guard(
    frontier: super::super::WienerNsLrRuntimeStorageRetentionFrontier,
    expected_context: &'static str,
) {
    let error = super::super::derive_wienerns_lr_live_storage_allocation(frontier).unwrap_err();
    let DecodeError::Reconstruction {
        source: ReconError::ArithmeticOverflow { context },
    } = error
    else {
        panic!("live storage guard should report arithmetic-overflow reconstruction error");
    };
    assert_eq!(context, expected_context);
}

fn valid_live_storage_allocation() -> super::super::WienerNsLrLiveStorageAllocation {
    super::super::derive_wienerns_lr_live_storage_allocation(valid_live_storage_frontier())
        .expect("live storage allocation")
}

fn tx_skip_grid(rows: usize, cols: usize, start: u8) -> super::super::WienerNsLrTxSkipGrid {
    let value_count = rows.checked_mul(cols).expect("test grid dimensions fit");
    let values = (0..value_count)
        .map(|index| start.wrapping_add((index % 2) as u8))
        .collect();
    super::super::WienerNsLrTxSkipGrid::new(rows, cols, values).expect("tx-skip grid")
}

fn assert_live_tx_skip_error(error: DecodeError, expected_field: &'static str) {
    let DecodeError::Reconstruction {
        source: ReconError::PcWienerInvalidBounds { field },
    } = error
    else {
        panic!("live tx-skip guard should report PcWienerInvalidBounds");
    };
    assert_eq!(field, expected_field);
}

#[test]
fn wienerns_lr_live_storage_allocation_shells_count_ten_bit_420_buffers_and_tx_skip_grid() {
    let (mut sequence, core) = fixture_sequence_and_key_core(TWO_FRAME_INTER_FIXTURE);
    sequence.general.bit_depth_idc = BitDepthIdc::Ten;
    let frontier = super::super::derive_wienerns_lr_runtime_storage_retention_frontier(
        &sequence,
        &core,
        ByteOffset::new(74),
        DecodeLimits::unlimited(),
    )
    .expect("retention frontier");

    let allocation: super::super::WienerNsLrLiveStorageAllocation =
        super::super::derive_wienerns_lr_live_storage_allocation(frontier)
            .expect("live storage allocation");

    let frame_samples = 64 * 64 * 3 / 2;
    assert_eq!(allocation.bit_depth(), BitDepth::Ten);
    assert_eq!(allocation.frame_sample_count(), frame_samples);
    assert_eq!(
        allocation.unpopulated_frame_samples(),
        frame_samples * 2,
        "CurrFrame and CdefFrame shells must both remain unpopulated"
    );
    assert_eq!(allocation.tx_skip_dimensions(), (16, 16));
    assert_eq!(allocation.tx_skip_value_count(), 16 * 16);
    assert_eq!(allocation.unpopulated_tx_skip_values(), 16 * 16);
    assert!(
        !allocation.is_fully_populated(),
        "storage shells must not fabricate decoded frame or LrTxSkip values"
    );
}

#[test]
fn wienerns_lr_live_tx_skip_grid_populates_retained_values_without_frame_samples() {
    let mut allocation = valid_live_storage_allocation();
    let grid = tx_skip_grid(16, 16, 0);

    allocation
        .populate_tx_skip_grid(&grid)
        .expect("populate tx-skip grid");

    assert_eq!(allocation.unpopulated_tx_skip_values(), 0);
    assert_eq!(allocation.tx_skip_value(0, 0), Some(0));
    assert_eq!(allocation.tx_skip_value(0, 1), Some(1));
    assert_eq!(allocation.tx_skip_value(15, 15), Some(1));
    assert_eq!(allocation.tx_skip_value(16, 0), None);
    assert_eq!(
        allocation.unpopulated_frame_samples(),
        64 * 64 * 3,
        "tx-skip population must not fabricate CurrFrame/CdefFrame samples"
    );
    assert!(
        !allocation.is_fully_populated(),
        "decoded frame shells remain unpopulated"
    );
}

#[test]
fn wienerns_lr_live_transform_record_handoff_populates_tx_skip_without_frame_samples() {
    let mut allocation = valid_live_storage_allocation();
    let records = [
        super::super::WienerNsLrTxSkipTransformRecord {
            row: 0,
            col: 0,
            rows: 8,
            cols: 16,
            skip_flag: false,
            eob: 0,
            intra_ist: None,
        },
        super::super::WienerNsLrTxSkipTransformRecord {
            row: 8,
            col: 0,
            rows: 8,
            cols: 8,
            skip_flag: false,
            eob: 7,
            intra_ist: None,
        },
        super::super::WienerNsLrTxSkipTransformRecord {
            row: 8,
            col: 8,
            rows: 8,
            cols: 8,
            skip_flag: true,
            eob: 11,
            intra_ist: None,
        },
    ];

    super::super::populate_wienerns_lr_live_tx_skip_from_transform_records(
        &mut allocation,
        16,
        16,
        &records,
    )
    .expect("populate from transform records");

    assert_eq!(allocation.unpopulated_tx_skip_values(), 0);
    assert_eq!(allocation.tx_skip_value(0, 0), Some(1));
    assert_eq!(allocation.tx_skip_value(7, 15), Some(1));
    assert_eq!(allocation.tx_skip_value(8, 0), Some(0));
    assert_eq!(allocation.tx_skip_value(15, 7), Some(0));
    assert_eq!(allocation.tx_skip_value(8, 8), Some(1));
    assert_eq!(allocation.tx_skip_value(15, 15), Some(1));
    assert_eq!(
        allocation.unpopulated_frame_samples(),
        64 * 64 * 3,
        "transform-record handoff must not fabricate CurrFrame/CdefFrame samples"
    );
    assert!(
        !allocation.is_fully_populated(),
        "decoded frame shells remain unpopulated"
    );
}

#[test]
fn wienerns_lr_live_tx_skip_grid_rejects_dimension_mismatch_without_mutation() {
    let mut allocation = valid_live_storage_allocation();
    let bad_grid = tx_skip_grid(8, 32, 1);

    let error = allocation.populate_tx_skip_grid(&bad_grid).unwrap_err();

    assert_live_tx_skip_error(error, "wiener ns lr live tx-skip dimensions");
    assert_eq!(allocation.unpopulated_tx_skip_values(), 16 * 16);
    assert_eq!(allocation.tx_skip_value(0, 0), None);
}

#[test]
fn wienerns_lr_live_tx_skip_grid_rejects_repopulation_without_mutation() {
    let mut allocation = valid_live_storage_allocation();
    let first = tx_skip_grid(16, 16, 0);
    let second = tx_skip_grid(16, 16, 9);

    allocation
        .populate_tx_skip_grid(&first)
        .expect("initial tx-skip population");
    let error = allocation.populate_tx_skip_grid(&second).unwrap_err();

    assert_live_tx_skip_error(error, "wiener ns lr live tx-skip already populated");
    assert_eq!(allocation.unpopulated_tx_skip_values(), 0);
    assert_eq!(allocation.tx_skip_value(0, 0), Some(0));
    assert_eq!(allocation.tx_skip_value(0, 1), Some(1));
}

#[test]
fn wienerns_lr_live_storage_allocation_rejects_invalid_internal_frontiers() {
    let mut frontier = valid_live_storage_frontier();
    frontier.frame_buffer_count = 1;
    assert_live_storage_guard(frontier, "wiener ns lr live frame-buffer count");

    let mut frontier = valid_live_storage_frontier();
    frontier.frame_buffer_bytes = 3;
    assert_live_storage_guard(frontier, "wiener ns lr live frame-buffer byte alignment");

    let mut frontier = valid_live_storage_frontier();
    frontier.tx_skip_rows = 0;
    assert_live_storage_guard(frontier, "wiener ns lr live tx-skip dimensions");

    let mut frontier = valid_live_storage_frontier();
    frontier.tx_skip_values = 255;
    assert_live_storage_guard(frontier, "wiener ns lr live tx-skip value count");
}

#[test]
fn wienerns_lr_live_storage_allocation_keeps_retention_limits_before_diagnostic() {
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

    assert!(
        matches!(error, DecodeError::Limit { .. }),
        "storage limits must fail before live-storage allocation diagnostic"
    );
}

#[test]
fn wienerns_lr_live_storage_allocation_error_reports_unpopulated_boundary() {
    let error = super::super::wienerns_lr_live_storage_allocation_error(ByteOffset::new(74));
    let unsupported = match error {
        DecodeError::UnsupportedFeature { unsupported } => unsupported,
        _ => panic!("live storage-allocation frontier must be an unsupported-feature error"),
    };

    assert_eq!(
        unsupported.reason(),
        "unsupported_wienerns_lr_live_storage_unpopulated"
    );
    assert_eq!(
        unsupported.matrix_row(),
        "ac0ej3-lr-live-storage-allocation"
    );
    assert_eq!(
        unsupported.feature_id(),
        "DECODE-AC0EJ3-LR-LIVE-STORAGE-ALLOCATION"
    );
    assert_eq!(unsupported.spec_section(), "7.20.4");
    assert_eq!(unsupported.byte_offset(), Some(ByteOffset::new(74)));
    assert!(
        unsupported
            .message()
            .contains("allocated private unpopulated CurrFrame"),
        "message should name unpopulated live frame shells"
    );
    assert!(
        unsupported.message().contains("LrTxSkip storage shells"),
        "message should name unpopulated tx-skip storage"
    );
    assert!(
        unsupported
            .message()
            .contains("has not populated decoded frame samples"),
        "message should not claim populated source samples"
    );
    assert!(
        unsupported.message().contains("FilterClass retention"),
        "message should keep classification output out of scope"
    );
}

#[test]
fn wienerns_lr_tx_mode_select_transform_record_error_reports_handoff_frontier() {
    let error =
        super::super::wienerns_lr_tx_mode_select_transform_record_error(ByteOffset::new(74));
    let unsupported = match error {
        DecodeError::UnsupportedFeature { unsupported } => unsupported,
        _ => panic!("tx-mode-select transform frontier must be an unsupported-feature error"),
    };

    assert_eq!(
        unsupported.reason(),
        "unsupported_wienerns_lr_tx_mode_select_transform_records"
    );
    assert_eq!(
        unsupported.matrix_row(),
        "ac0ej3-lr-live-transform-record-handoff"
    );
    assert_eq!(
        unsupported.feature_id(),
        "DECODE-AC0EJ3-LR-LIVE-TRANSFORM-RECORD-HANDOFF"
    );
    assert_eq!(unsupported.spec_section(), "5.20.6.1");
    assert_eq!(unsupported.byte_offset(), Some(ByteOffset::new(74)));
    assert!(
        unsupported.message().contains("TX_MODE_SELECT"),
        "message should name the selectable-transform blocker"
    );
    assert!(
        unsupported
            .message()
            .contains("read_tx_size/read_tx_partition"),
        "message should name the missing transform-record syntax"
    );
}

#[test]
fn wienerns_lr_live_frame_samples_unpopulated_error_reports_handoff_frontier() {
    let error = super::super::wienerns_lr_live_frame_samples_unpopulated_error(ByteOffset::new(74));
    let unsupported = match error {
        DecodeError::UnsupportedFeature { unsupported } => unsupported,
        _ => panic!("live frame-sample frontier must be an unsupported-feature error"),
    };

    assert_eq!(
        unsupported.reason(),
        "unsupported_wienerns_lr_live_frame_samples_unpopulated"
    );
    assert_eq!(
        unsupported.matrix_row(),
        "ac0ej3-lr-live-transform-record-handoff"
    );
    assert_eq!(
        unsupported.feature_id(),
        "DECODE-AC0EJ3-LR-LIVE-TRANSFORM-RECORD-HANDOFF"
    );
    assert_eq!(unsupported.spec_section(), "7.20.4");
    assert!(
        unsupported
            .message()
            .contains("populated the live LrTxSkip shell"),
        "message should name the completed transform-record handoff"
    );
    assert!(
        unsupported
            .message()
            .contains("CurrFrame and CdefFrame samples are still unpopulated"),
        "message should stop before decoded samples"
    );
}

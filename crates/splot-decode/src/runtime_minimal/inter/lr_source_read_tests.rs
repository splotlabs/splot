// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Loop-restoration source-read regression tests for the minimal inter runtime.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use splot_core::headers::frame::{
    FrameRestorationType, LrPlaneParams, WienerNsFrameFilterBank, WienerNsFrameFilterClass,
};
use splot_core::headers::sequence::ChromaFormatIdc;
use splot_core::span::ByteOffset;
use splot_recon::{LoopRestorationSource, LoopRestorationSourceBounds, PlaneId};

use crate::error::DecodeError;
use crate::tile_payload::WienerNsLrSourceBlock;
use crate::{DecodeLimitName, DecodeLimitThreshold, DecodeLimits};

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

    let unsupported = match error {
        DecodeError::UnsupportedFeature { unsupported } => unsupported,
        _ => panic!("unit-coded chroma filters must fail closed before source-read accounting"),
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

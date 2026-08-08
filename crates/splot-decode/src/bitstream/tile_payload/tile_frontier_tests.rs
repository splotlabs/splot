// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use splot_core::bitio::BitReader;
use splot_core::headers::frame::{
    CoreSeqRestorationView, LrGeometry, LrParseOutcome, parse_lr_params_for_inter,
};
use splot_core::headers::sequence::{ChromaFormatIdc, SuperblockSize};
use splot_core::span::ByteOffset;

#[test]
fn temporal_frame_wiener_ns_without_local_bank_keeps_lr_unit_syntax_enabled() {
    let restoration = CoreSeqRestorationView {
        enable_restoration: true,
        lr_pc_wiener_disabled: true,
        lr_wiener_nonsep_disabled: false,
        lr_uv_pc_wiener_disabled: true,
        lr_uv_wiener_nonsep_disabled: false,
    };
    let geometry = LrGeometry::new(SuperblockSize::Block128x128, ChromaFormatIdc::Yuv420);
    let payload = [0xe4];
    let mut reader = BitReader::new(&payload, ByteOffset::new(0));
    let outcome = parse_lr_params_for_inter(
        &mut reader,
        false,
        3,
        restoration,
        geometry,
        100,
        1,
        [0; 3],
        &[Vec::new(), Vec::new(), Vec::new()],
        splot_core::headers::frame::LrTemporalReferenceView::unknown(&[0]),
    )
    .unwrap();
    let LrParseOutcome::Parsed(lr) = outcome else {
        panic!("expected complete temporal Wiener-NS LR params");
    };

    assert_eq!(
        loop_restoration_state(&lr, 3),
        TilePartitionLoopRestorationState::Frame(TilePartitionLoopRestorationFrameState::new(
            [
                TilePartitionLoopRestorationPlaneTool::WienerNs,
                TilePartitionLoopRestorationPlaneTool::None,
                TilePartitionLoopRestorationPlaneTool::None,
            ],
            [true, false, false],
            [256, 0, 0],
        ))
    );
}

#[test]
fn switchable_luma_without_frame_filter_keeps_lr_unit_syntax_enabled() {
    let restoration = CoreSeqRestorationView {
        enable_restoration: true,
        lr_pc_wiener_disabled: false,
        lr_wiener_nonsep_disabled: false,
        lr_uv_pc_wiener_disabled: true,
        lr_uv_wiener_nonsep_disabled: true,
    };
    let geometry = LrGeometry::new(SuperblockSize::Block128x128, ChromaFormatIdc::Monochrome);
    let mut reader = BitReader::new(&[0xd0], ByteOffset::new(0));
    let outcome = parse_lr_params_for_inter(
        &mut reader,
        false,
        1,
        restoration,
        geometry,
        100,
        1,
        [0; 3],
        &[Vec::new(), Vec::new(), Vec::new()],
        splot_core::headers::frame::LrTemporalReferenceView::unknown(&[0]),
    )
    .unwrap();
    let LrParseOutcome::Parsed(lr) = outcome else {
        panic!("expected complete switchable LR params");
    };

    assert_eq!(
        loop_restoration_state(&lr, 1),
        TilePartitionLoopRestorationState::Frame(TilePartitionLoopRestorationFrameState::new(
            [
                TilePartitionLoopRestorationPlaneTool::Switchable,
                TilePartitionLoopRestorationPlaneTool::None,
                TilePartitionLoopRestorationPlaneTool::None,
            ],
            [false; 3],
            [256, 0, 0],
        ))
    );
}

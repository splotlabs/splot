// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Shared, test-only fixture builders reused by both the §5.18 parser tests
//! (`headers::frame::*`) and the sibling writer round-trip tests (`write::frame_*`).
//!
//! Each helper hand-builds a minimal, spec-grounded view so the parser tests and
//! the writer round-trip tests exercise the same canonical inputs without
//! duplicating the literal field-by-field fixtures.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use proptest::prelude::*;

use crate::headers::frame::{CoreSeqQuantView, CoreSeqTileView, GdfGeometry, SegmentationParams};
use crate::headers::sequence::{LevelIdx, SuperblockSize, Tier};
use crate::segment::{MAX_SEGMENTS, SEG_LVL_MAX, SegmentFeature};

/// A proptest strategy over quantizer views spanning bit depth, plane count, and
/// every optional delta-Q / TCQ flag, used by the parser and writer proptests.
pub(crate) fn arbitrary_quant_view() -> impl Strategy<Value = CoreSeqQuantView> {
    (
        prop_oneof![Just(8u8), Just(10u8)],
        prop_oneof![Just(1u8), Just(3u8)],
        any::<[bool; 5]>(),
        any::<[i32; 3]>(),
        any::<[bool; 3]>(),
    )
        .prop_map(
            |(bit_depth, num_planes, flags, bases, tcq)| CoreSeqQuantView {
                bit_depth,
                num_planes,
                separate_uv_delta_q: flags[0],
                equal_ac_dc_q: flags[1],
                y_dc_delta_q_enabled: flags[2],
                uv_dc_delta_q_enabled: flags[3],
                uv_ac_delta_q_enabled: flags[4],
                base_y_dc_delta_q: bases[0],
                base_uv_dc_delta_q: bases[1],
                base_uv_ac_delta_q: bases[2],
                enable_tcq: tcq[0],
                choose_tcq_per_frame: tcq[1],
                enable_parity_hiding: tcq[2],
            },
        )
}

/// An 8-bit, 3-plane view with every optional quantizer read disabled.
pub(crate) fn base_quant() -> CoreSeqQuantView {
    CoreSeqQuantView {
        bit_depth: 8,
        num_planes: 3,
        separate_uv_delta_q: false,
        equal_ac_dc_q: false,
        y_dc_delta_q_enabled: false,
        uv_dc_delta_q_enabled: false,
        uv_ac_delta_q_enabled: false,
        base_y_dc_delta_q: 0,
        base_uv_dc_delta_q: 0,
        base_uv_ac_delta_q: 0,
        enable_tcq: false,
        choose_tcq_per_frame: false,
        enable_parity_hiding: false,
    }
}

/// All-disabled segmentation (or enabled with no features).
pub(crate) fn seg_params(enabled: bool) -> SegmentationParams {
    SegmentationParams {
        segmentation_enabled: enabled,
        reuse_seg_info: false,
        features: [[SegmentFeature::DISABLED; SEG_LVL_MAX]; MAX_SEGMENTS],
        segmentation_update_map: enabled,
        segmentation_temporal_update: false,
        seg_id_pre_skip: false,
        last_active_seg_id: 0,
    }
}

/// A 64x64-superblock sequence with no sequence tile info, level 0 Main tier,
/// and frame-level CDF context updates enabled.
pub(crate) fn base_view() -> CoreSeqTileView {
    CoreSeqTileView {
        seq_tile_info_present_flag: false,
        allow_tile_info_change: false,
        seq_tile_params: None,
        seq_sb_col_starts: Vec::new(),
        seq_sb_row_starts: Vec::new(),
        seq_sb_size: SuperblockSize::Block64x64,
        use_256x256_superblock: false,
        use_128x128_superblock: false,
        enable_avg_cdf: false,
        avg_cdf_type: 0,
        seq_tier: Tier::Main,
        seq_level_idx: LevelIdx::from_bits(0),
    }
}

/// A small single-tile frame (MiCols = MiRows = 256, so MiCols*4 = 1024 > 128),
/// so `gdf_per_block` is coded.
pub(crate) fn base_geometry() -> GdfGeometry<'static> {
    GdfGeometry {
        sb_size: SuperblockSize::Block128x128,
        mi_cols: 256,
        mi_rows: 256,
        tile_cols: 1,
        tile_rows: 1,
        mi_col_starts: &[0],
        mi_row_starts: &[0],
    }
}

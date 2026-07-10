// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Property (no-panic) tests for the [`super`] frame-header parser.

use super::*;
use crate::headers::frame::filtering::CoreSeqFilterView;
use crate::headers::frame::restoration::{CoreSeqCcsoView, CoreSeqRestorationView};
use crate::headers::frame::segmentation::CoreSeqSegView;
use crate::headers::frame::tiling::CoreSeqTileView;
use crate::headers::sequence::{ChromaFormatIdc, LevelIdx, SuperblockSize, Tier};
use crate::segment::{MAX_SEGMENTS, SEG_LVL_MAX, SegmentFeature, SegmentInfo};
use crate::span::ByteOffset;
use crate::tile::TileParams;
use proptest::prelude::*;

use crate::test_support::arbitrary_quant_view;

/// Arbitrary [`CoreSeqSegView`] values, including internally inconsistent ones
/// (hostile `max_segments`, stored info without the present flag).
fn arbitrary_seg_view() -> impl Strategy<Value = CoreSeqSegView> {
    (
        any::<[bool; 4]>(),
        any::<u8>(),
        0..MAX_SEGMENTS,
        0..SEG_LVL_MAX,
        any::<i32>(),
    )
        .prop_map(|(flags, max_segments, seg_idx, feature_idx, data)| {
            let mut features = [[SegmentFeature::DISABLED; SEG_LVL_MAX]; MAX_SEGMENTS];
            features[seg_idx][feature_idx] = SegmentFeature {
                enabled: true,
                data,
            };
            CoreSeqSegView {
                seq_seg_info_present_flag: flags[0],
                seq_allow_seg_info_change: flags[1],
                enable_ext_seg: flags[2],
                max_segments,
                seq_segment_info: flags[3].then_some(SegmentInfo {
                    num_segments: max_segments.min(MAX_SEGMENTS as u8),
                    features,
                }),
            }
        })
}

fn sb_size(idx: u8) -> SuperblockSize {
    match idx % 3 {
        0 => SuperblockSize::Block64x64,
        1 => SuperblockSize::Block128x128,
        _ => SuperblockSize::Block256x256,
    }
}

/// Arbitrary [`CoreSeqTileView`] values, including stored layouts that are
/// ineligible, non-uniform, or absent despite the present flag.
fn arbitrary_tile_view() -> impl Strategy<Value = CoreSeqTileView> {
    (
        any::<[bool; 4]>(),
        (0u32..=64, 0u32..=64, 0u8..=8, 0u8..=8),
        (0u32..=2048, 0u32..=2048),
        any::<[u8; 2]>(),
        (any::<bool>(), 0u8..=3, any::<bool>(), 0u8..=31),
        (
            proptest::collection::vec(0u32..=4096, 0..=64),
            proptest::collection::vec(0u32..=4096, 0..=64),
        ),
    )
        .prop_map(|(flags, counts, grid, sbs, misc, starts)| {
            let (use_256, use_128) = match sbs[0] % 3 {
                0 => (false, false),
                1 => (false, true),
                _ => (true, false),
            };
            CoreSeqTileView {
                seq_tile_info_present_flag: flags[0],
                allow_tile_info_change: flags[1],
                seq_tile_params: flags[2].then_some(TileParams {
                    tile_cols: counts.0,
                    tile_rows: counts.1,
                    tile_cols_log2: counts.2,
                    tile_rows_log2: counts.3,
                    sb_cols: grid.0,
                    sb_rows: grid.1,
                    uniform_spacing: flags[3],
                    covers_cols: true,
                    covers_rows: true,
                }),
                seq_sb_col_starts: starts.0,
                seq_sb_row_starts: starts.1,
                seq_sb_size: sb_size(sbs[1]),
                use_256x256_superblock: use_256,
                use_128x128_superblock: use_128,
                enable_avg_cdf: misc.0,
                avg_cdf_type: misc.1,
                seq_tier: if misc.2 { Tier::High } else { Tier::Main },
                seq_level_idx: LevelIdx::from_bits(misc.3),
            }
        })
}

/// Arbitrary `sequence_filter_config()` (§ 5.4.10) inputs consumed by the
/// § 5.18.2 tail loop-filter cluster.
fn arbitrary_filter_view() -> impl Strategy<Value = CoreSeqFilterView> {
    use crate::headers::sequence::CdefOnSkipTxfm;
    (
        any::<[bool; 4]>(),
        prop_oneof![
            Just(CdefOnSkipTxfm::Adaptive),
            Just(CdefOnSkipTxfm::AlwaysOn),
            Just(CdefOnSkipTxfm::Disabled),
        ],
        0u8..=3,
    )
        .prop_map(
            |(flags, skip_txfm, df_par_bits_minus_2)| CoreSeqFilterView {
                enable_cdef: flags[0],
                enable_gdf: flags[1],
                gdf_unit_matches_sb_size: flags[2],
                disable_loopfilters_across_tiles: flags[3],
                cdef_on_skip_txfm: skip_txfm,
                df_par_bits_minus_2,
                enable_df_sub_pu: false,
                single_picture_header_flag: false,
            },
        )
}

/// Arbitrary [`CoreSeqRestorationView`] values, with `lr_uv_pc_wiener_disabled` tied
/// to `enable_restoration` per the § 5.4.10 inference (mirror :1382).
fn arbitrary_restoration_view() -> impl Strategy<Value = CoreSeqRestorationView> {
    any::<[bool; 4]>().prop_map(|flags| CoreSeqRestorationView {
        enable_restoration: flags[0],
        lr_pc_wiener_disabled: flags[1],
        lr_wiener_nonsep_disabled: flags[2],
        lr_uv_pc_wiener_disabled: flags[0],
        lr_uv_wiener_nonsep_disabled: flags[3],
    })
}

/// Arbitrary [`CoreSeqCcsoView`] values.
fn arbitrary_ccso_view() -> impl Strategy<Value = CoreSeqCcsoView> {
    any::<bool>().prop_map(|enable_ccso| CoreSeqCcsoView {
        enable_ccso,
        single_picture_header_flag: false,
    })
}

/// Arbitrary `chroma_format_idc` values (§ 5.4.1).
fn arbitrary_chroma_format() -> impl Strategy<Value = ChromaFormatIdc> {
    prop_oneof![
        Just(ChromaFormatIdc::Yuv420),
        Just(ChromaFormatIdc::Monochrome),
        Just(ChromaFormatIdc::Yuv444),
        Just(ChromaFormatIdc::Yuv422),
    ]
}

/// Arbitrary [`CoreSeqView`] values within their type ranges, including the
/// § 5.18.6 / § 5.18.7 / § 5.4.10 sub-views consumed by the new intra structure
/// cluster.
fn arbitrary_seq_view() -> impl Strategy<Value = CoreSeqView> {
    (
        (
            1u32..=8,
            0u32..=8,
            0u32..=5,
            any::<[bool; 3]>(),
            0u8..=2,
            (1u32..=16, 1u32..=16),
            (1u32..=65536, 1u32..=65536),
            (0u8..=2, 0u8..=2, any::<bool>()),
        ),
        arbitrary_quant_view(),
        arbitrary_seg_view(),
        arbitrary_tile_view(),
        arbitrary_filter_view(),
        arbitrary_restoration_view(),
        arbitrary_ccso_view(),
        arbitrary_chroma_format(),
    )
        .prop_map(
            |(general, quant, seg, tile, filter, restoration, ccso, chroma_format_idc)| {
                let (
                    num_ref_frames,
                    order_hint_bits,
                    long_term_frame_id_bits,
                    flags,
                    max_mlayer_id,
                    dim_bits,
                    max_dims,
                    scc,
                ) = general;
                CoreSeqView {
                    num_ref_frames,
                    order_hint_bits,
                    long_term_frame_id_bits,
                    enable_short_refresh_frame_flags: flags[0],
                    monotonic_output_order_flag: flags[1],
                    single_picture_header_flag: flags[2],
                    max_mlayer_id,
                    frame_width_bits: dim_bits.0,
                    frame_height_bits: dim_bits.1,
                    max_frame_width: max_dims.0,
                    max_frame_height: max_dims.1,
                    seq_force_screen_content_tools: scc.0,
                    seq_force_integer_mv: scc.1,
                    allow_frame_max_bvp_drl_bits: scc.2,
                    inter: CoreSeqInterView {
                        enable_ref_frame_mvs: flags[0],
                        explicit_ref_frame_map: flags[1],
                        enable_bru: flags[2],
                        enable_tip: flags[0],
                        enable_tip_output: false,
                        enable_tip_hole_fill: flags[1],
                        enable_tip_explicit_qp: false,
                        enable_refinemv: flags[2],
                        enable_tip_refinemv: flags[0] && (scc.1 != 0 || flags[2]),
                        seq_max_drl_bits_minus_1: u32::from(scc.0),
                        allow_frame_max_drl_bits: scc.2,
                        enable_flex_mvres: flags[1],
                        seq_frame_motion_modes_present_flag: flags[2],
                        seq_enabled_motion_modes: [false, flags[0], flags[1], flags[2], false],
                        enable_opfl_refine: scc.1,
                        enable_bawp: flags[0],
                        enable_global_motion: flags[1],
                    },
                    quant,
                    seg,
                    tile,
                    filter,
                    restoration,
                    ccso,
                    chroma_format_idc,
                    film_grain_params_present: Some(false),
                }
            },
        )
}

/// A fixed in-band multi-frame-header record for the `cur_mfh_id > 0` never-panic
/// property: it signals both a frame-size payload and a segment-info arm so the
/// resolved-MFH paths are exercised. Dimensions are bounded to `seq`'s bit widths.
/// `seq_id` is a valid `SequenceHeaderId` provided by the caller (always `Some` for
/// `0`), so this helper itself never constructs an id.
fn arbitrary_mfh_record(seq: &CoreSeqView, seq_id: SequenceHeaderId) -> MultiFrameHeaderRecord {
    use crate::hls::MfhFrameSize;
    let width_bits = seq.frame_width_bits.clamp(1, 16);
    let height_bits = seq.frame_height_bits.clamp(1, 16);
    let mut features = [[SegmentFeature::DISABLED; SEG_LVL_MAX]; MAX_SEGMENTS];
    features[0][0] = SegmentFeature {
        enabled: true,
        data: 1,
    };
    MultiFrameHeaderRecord {
        mfh_id: MfhId::from_raw(1),
        mfh_seq_header_id: seq_id,
        mfh_tlayer_id: crate::types::TemporalLayerId::from_bits(0),
        mfh_mlayer_id: crate::types::EmbeddedLayerId::from_bits(0),
        mfh_frame_size: Some(MfhFrameSize {
            width_bits: width_bits as u8,
            height_bits: height_bits as u8,
            width_minus_1: 0,
            height_minus_1: 0,
        }),
        mfh_seg_info_present_flag: true,
        mfh_ext_seg_flag: Some(false),
        mfh_allow_seg_info_change: Some(false),
        mfh_segment_info: Some(SegmentInfo {
            num_segments: 8,
            features,
        }),
        mfh_deblocking_filter_update: false,
        mfh_apply_deblocking_filter: [false; 4],
        offset: ByteOffset::new(0),
    }
}

proptest! {
    /// The frame-header core parser must never panic on arbitrary input, in either
    /// mode, with no modeled sequence state.
    #[test]
    fn parse_frame_header_core_never_panics(
        data in proptest::collection::vec(any::<u8>(), 0..64),
        raw_type in 0u8..=31,
        first_picture in any::<bool>(),
        core_mode in any::<bool>(),
    ) {
        let obu_type = ObuType::from_raw(raw_type);
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let input = FrameHeaderParseInput {
            obu_type,
            first_picture_in_tu: first_picture,
            active_sequence: None,
            mfh_record: None,
            reference_state: FrameReferenceStateView::unknown(),
            mode: if core_mode {
                FrameHeaderParseMode::Core
            } else {
                FrameHeaderParseMode::ActivationPrefix
            },
        };
        let _ = parse_frame_header_core(&mut reader, &input);
    }

    /// The core body — including the full § 5.18.2 intra structure cluster
    /// (tile_info, quantization, segmentation, QM setup, delta-q, lossless tail)
    /// — must never panic and never over-read for arbitrary payload bytes and
    /// arbitrary [`CoreSeqView`] values within their type ranges.
    #[test]
    fn parse_core_body_with_sequence_never_panics(
        data in proptest::collection::vec(any::<u8>(), 0..96),
        raw_type in 0u8..=31,
        first_picture in any::<bool>(),
        seq in arbitrary_seq_view(),
    ) {
        let obu_type = ObuType::from_raw(raw_type);
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        if let Ok(prefix) =
            parse_frame_header_prefix(&mut reader, obu_type, Some(first_picture))
        {
            let mut core = init_core_from_prefix(&prefix, obu_type, first_picture);
            let mfh_view = match (core.cur_mfh_id.is_zero(), SequenceHeaderId::try_new(0)) {
                (false, Some(seq_id)) => {
                    Some(MfhFrameView::from_record(&arbitrary_mfh_record(&seq, seq_id), &seq))
                }
                _ => None,
            };
            let _ = parse_core_body(
                &mut reader,
                &mut core,
                &seq,
                mfh_view.as_ref(),
                &FrameReferenceStateView::unknown(),
            );
            prop_assert!(reader.consumed_bits() <= (data.len() as u64) * 8);
        }
    }
}

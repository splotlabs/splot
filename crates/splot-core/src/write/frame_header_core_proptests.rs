// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>


#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod proptests {
    use super::*;
    use crate::headers::frame::{
        CoreSeqCcsoView, CoreSeqFilterView, CoreSeqInterView, CoreSeqQuantView,
        CoreSeqRestorationView, CoreSeqSegView, CoreSeqTileView, FrameHeaderParseStatus,
    };
    use crate::headers::sequence::{
        CdefOnSkipTxfm, ChromaFormatIdc, LevelIdx, SuperblockSize, Tier,
    };
    use proptest::prelude::*;

    proptest! {
        /// Every parser-reachable intra frame header round-trips byte-exactly and semantically.
        #[test]
        fn intra_header_round_trips(
            type_idx in 0u8..3,
            first_pic in any::<bool>(),
            grain in any::<bool>(),
            short_refresh in any::<bool>(),
            monotonic in any::<bool>(),
            payload in proptest::collection::vec(any::<u8>(), 1..24),
        ) {
            let obu_type = match type_idx {
                0 => ObuType::ClosedLoopKey,
                1 => ObuType::OpenLoopKey,
                _ => ObuType::RegularTileGroup,
            };
            let seq = proptest_seq(grain, short_refresh, monotonic);
            let mut data = payload.clone();
            data.extend_from_slice(&[0u8; 16]);

            let Ok(core) =
                parse_core_body_for_test(&data, obu_type, first_pic, &seq, None)
            else {
                return Ok(());
            };
            if core.status != FrameHeaderParseStatus::IntraHeaderComplete {
                return Ok(());
            }

            let mut writer = BitWriter::new();
            write_frame_header_core(&mut writer, &core, &seq, None, first_pic).unwrap();
            let written = writer.into_bytes();

            let reparsed =
                parse_core_body_for_test(&written, obu_type, first_pic, &seq, None).unwrap();
            let mut a = reparsed.clone();
            let mut b = core.clone();
            a.consumed_bits = 0;
            b.consumed_bits = 0;
            prop_assert_eq!(a, b);

            let mut writer2 = BitWriter::new();
            write_frame_header_core(&mut writer2, &reparsed, &seq, None, first_pic).unwrap();
            prop_assert_eq!(writer2.into_bytes(), written);
        }

        /// The writer never panics for any core / sequence pair: a non-canonical or
        /// non-intra model returns `Err`, a canonical one returns `Ok`, never a panic.
        #[test]
        fn writer_never_panics(
            payload in proptest::collection::vec(any::<u8>(), 0..24),
            type_idx in any::<u8>(),
        ) {
            let obu_type = match type_idx % 4 {
                0 => ObuType::ClosedLoopKey,
                1 => ObuType::OpenLoopKey,
                2 => ObuType::RegularTileGroup,
                _ => ObuType::RegularSef,
            };
            let seq = proptest_seq(false, false, false);
            let mut data = payload.clone();
            data.extend_from_slice(&[0u8; 16]);
            if let Ok(core) = parse_core_body_for_test(&data, obu_type, false, &seq, None) {
                let mut writer = BitWriter::new();
                let result = write_frame_header_core(&mut writer, &core, &seq, None, false);
                if result.is_err() {
                    prop_assert_eq!(writer.bit_len(), 0);
                }
            }
        }
    }

    /// A `base_seq()`-shaped view (OrderHintBits 4, NumRefFrames 8, screen content off,
    /// 12-bit dims, 4096x2304 max) with grain / short-refresh / monotonic toggles for the
    /// property generator. Rebuilt here so the proptest module is self-contained.
    fn proptest_seq(grain: bool, short_refresh: bool, monotonic: bool) -> CoreSeqView {
        CoreSeqView {
            num_ref_frames: 8,
            order_hint_bits: 4,
            long_term_frame_id_bits: 0,
            enable_short_refresh_frame_flags: short_refresh,
            monotonic_output_order_flag: monotonic,
            single_picture_header_flag: false,
            max_mlayer_id: 0,
            frame_width_bits: 12,
            frame_height_bits: 12,
            max_frame_width: 4096,
            max_frame_height: 2304,
            seq_force_screen_content_tools: 0,
            seq_force_integer_mv: 0,
            allow_frame_max_bvp_drl_bits: false,
            inter: CoreSeqInterView::new_minimal_intra(),
            quant: CoreSeqQuantView {
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
            },
            seg: CoreSeqSegView {
                seq_seg_info_present_flag: false,
                seq_allow_seg_info_change: false,
                enable_ext_seg: false,
                max_segments: 8,
                seq_segment_info: None,
            },
            tile: CoreSeqTileView {
                seq_tile_info_present_flag: false,
                allow_tile_info_change: false,
                seq_tile_params: None,
                seq_sb_col_starts: Vec::new(),
                seq_sb_row_starts: Vec::new(),
                seq_sb_size: SuperblockSize::Block128x128,
                use_256x256_superblock: false,
                use_128x128_superblock: true,
                enable_avg_cdf: false,
                avg_cdf_type: 0,
                seq_tier: Tier::Main,
                seq_level_idx: LevelIdx::from_bits(0),
            },
            filter: CoreSeqFilterView {
                enable_cdef: false,
                enable_gdf: false,
                gdf_unit_matches_sb_size: false,
                disable_loopfilters_across_tiles: false,
                cdef_on_skip_txfm: CdefOnSkipTxfm::Adaptive,
                df_par_bits_minus_2: 0,
                enable_df_sub_pu: false,
                single_picture_header_flag: false,
            },
            restoration: CoreSeqRestorationView {
                enable_restoration: false,
                lr_pc_wiener_disabled: false,
                lr_wiener_nonsep_disabled: false,
                lr_uv_pc_wiener_disabled: false,
                lr_uv_wiener_nonsep_disabled: false,
            },
            ccso: CoreSeqCcsoView {
                enable_ccso: false,
                single_picture_header_flag: false,
            },
            chroma_format_idc: ChromaFormatIdc::Yuv420,
            film_grain_params_present: Some(grain),
        }
    }
}

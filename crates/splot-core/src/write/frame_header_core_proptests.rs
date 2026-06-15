// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

// Property tests for the composing intra frame-header writer (§ 5.18.2): every byte stream
// that parses to an `IntraHeaderComplete` `FrameHeaderCore` writes back byte-exactly and
// reparses to an equal core.
//
// `include!`d into `crate::write::frame_header_core`. The generator drives random bytes
// through the §5.18.2 core parser against a fixed sequence view; only the inputs that reach
// `IntraHeaderComplete` are round-tripped (the rest are discarded), so the test exercises a
// wide spread of canonical intra headers without hand-enumerating each field.

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod proptests {
    use super::*;
    use crate::headers::frame::{
        CoreSeqCcsoView, CoreSeqFilterView, CoreSeqInterView, CoreSeqQuantView,
        CoreSeqRestorationView, CoreSeqSegView, CoreSeqTileView, FrameHeaderParseStatus,
        FrameReferenceStateView, init_core_from_prefix, parse_core_body, parse_frame_header_prefix,
    };
    use crate::headers::sequence::{
        CdefOnSkipTxfm, ChromaFormatIdc, LevelIdx, SuperblockSize, Tier,
    };
    use proptest::prelude::*;

    /// Parses a frame-header body against a directly built [`CoreSeqView`] (the proptest
    /// equivalent of the parser's `parse_body_with_mfh` helper).
    fn parse_core_body_for_test(
        data: &[u8],
        obu_type: ObuType,
        first_pic: bool,
        seq: &CoreSeqView,
        mfh: Option<&MfhFrameView>,
    ) -> crate::error::Result<FrameHeaderCore> {
        let mut reader = crate::bitio::BitReader::new(data, crate::span::ByteOffset::new(0));
        let prefix = parse_frame_header_prefix(&mut reader, obu_type, Some(first_pic))?;
        let mut core = init_core_from_prefix(&prefix, obu_type, first_pic);
        parse_core_body(
            &mut reader,
            &mut core,
            seq,
            mfh,
            &FrameReferenceStateView::unknown(),
        )?;
        core.consumed_bits = reader.consumed_bits();
        Ok(core)
    }

    proptest! {
        /// Every parser-reachable intra frame header round-trips byte-exactly and semantically.
        #[test]
        fn intra_header_round_trips(
            type_idx in 0u8..3,
            first_pic in any::<bool>(),
            grain in any::<bool>(),
            short_refresh in any::<bool>(),
            monotonic in any::<bool>(),
            // A pool of payload bytes the parser walks; padded so a field never hits EOF.
            payload in proptest::collection::vec(any::<u8>(), 1..24),
        ) {
            let obu_type = match type_idx {
                0 => ObuType::ClosedLoopKey,
                1 => ObuType::OpenLoopKey,
                _ => ObuType::RegularTileGroup,
            };
            let seq = proptest_seq(grain, short_refresh, monotonic);
            // Pad so a parser field never runs past the payload (we only keep complete parses).
            let mut data = payload.clone();
            data.extend_from_slice(&[0u8; 16]);

            let Ok(mut core) =
                parse_core_body_for_test(&data, obu_type, first_pic, &seq, None)
            else {
                return Ok(());
            };
            if core.status != FrameHeaderParseStatus::IntraHeaderComplete {
                return Ok(());
            }

            // The writer emits the *canonical* encoding of the core. A random payload may be a
            // non-canonical bitstream (e.g. a non-minimal descriptor the reader accepts), so a
            // byte comparison against the original input would be unsound; the byte-exact
            // property is proved against the hand-built canonical fixtures in the tests module.
            // Here the property is the writer being a canonicalizing inverse:
            //   1. parse(write(core)) == core  (semantic round-trip), and
            //   2. write(parse(write(core))) == write(core)  (a fixed point — re-encoding the
            //      written form reproduces it exactly).
            //
            // `consumed_bits` is the one model field that is NOT canonical-invariant: a
            // non-minimal descriptor the reader accepts makes the parse consume more bits than
            // the canonical re-encoding emits. The writer's reject-completeness `consumed_bits`
            // gate (finding 6) requires `core.consumed_bits == canonical length`, so set it to
            // the canonical drafted length here before the round-trip — the structural fields are
            // canonical-invariant, so this is exactly the value `parse(write(core))` produces.
            let Ok(glue) = check_frame_header_core_encodable(&core, &seq, None) else {
                return Ok(());
            };
            let mut draft = BitWriter::new();
            write_intra_header_into(&mut draft, &core, &seq, None, &glue).unwrap();
            core.consumed_bits = draft.bit_len();

            let mut writer = BitWriter::new();
            write_frame_header_core(&mut writer, &core, &seq, None).unwrap();
            let written = writer.into_bytes();

            // 1. Semantic round-trip (ignoring consumed_bits — the written buffer is exactly
            //    the header, while `data` carried padding).
            let reparsed =
                parse_core_body_for_test(&written, obu_type, first_pic, &seq, None).unwrap();
            let mut a = reparsed.clone();
            let mut b = core.clone();
            a.consumed_bits = 0;
            b.consumed_bits = 0;
            prop_assert_eq!(a, b);

            // 2. Fixed point: re-encoding the reparsed core reproduces the written bytes.
            let mut writer2 = BitWriter::new();
            write_frame_header_core(&mut writer2, &reparsed, &seq, None).unwrap();
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
                // Whatever the status, the call returns a Result without panicking. On a
                // reject the buffer is untouched.
                let result = write_frame_header_core(&mut writer, &core, &seq, None);
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
            inter: CoreSeqInterView {
                enable_ref_frame_mvs: false,
                explicit_ref_frame_map: false,
                enable_bru: false,
                enable_tip: false,
                seq_max_drl_bits_minus_1: 0,
                allow_frame_max_drl_bits: false,
                enable_flex_mvres: false,
                seq_frame_motion_modes_present_flag: false,
                seq_enabled_motion_modes: [false; 5],
                enable_opfl_refine: 0,
            },
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

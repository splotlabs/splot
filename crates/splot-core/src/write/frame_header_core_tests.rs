// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

// End-to-end parse -> write -> parse round-trip and rejection tests for the composing intra
// frame-header writer (§ 5.18.2 `frame_header_info()`).
//
// `include!`d into `crate::write::frame_header_core` so `super::*` resolves to
// `write_frame_header_core` and its private helpers. Each round-trip test builds a canonical
// intra frame-header byte stream, parses it to an `IntraHeaderComplete` `FrameHeaderCore`,
// writes that core back with `write_frame_header_core`, and asserts the bytes are byte-exact
// and reparse to an equal core. The rejection tests confirm reject-before-write
// (`bit_len() == 0`) for every non-canonical model.

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]
mod tests {
    use super::*;
    use crate::headers::frame::{
        CoreSeqCcsoView, CoreSeqFilterView, CoreSeqInterView, CoreSeqQuantView,
        CoreSeqRestorationView, CoreSeqSegView, CoreSeqTileView, FrameHeaderParseStatus, FrameSize,
        FrameReferenceStateView, init_core_from_prefix, parse_core_body,
        parse_frame_header_prefix,
    };
    use crate::headers::sequence::{CdefOnSkipTxfm, ChromaFormatIdc, LevelIdx, SuperblockSize, Tier};

    /// Parses a frame-header body (activation prefix + `parse_core_body`) against a directly
    /// built [`CoreSeqView`] / [`MfhFrameView`], the writer-test equivalent of the parser's
    /// in-module `parse_body_with_mfh` helper (which is not reachable from this module).
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

    /// MSB-first bit builder mirroring the parser test helpers (`info.rs::tests::Bits`).
    #[derive(Default, Clone)]
    struct Bits {
        bits: Vec<u8>,
    }

    impl Bits {
        fn bit(&mut self, bit: u8) {
            self.bits.push(bit & 1);
        }

        fn f(&mut self, value: u32, width: u32) {
            for shift in (0..width).rev() {
                self.bit(((value >> shift) & 1) as u8);
            }
        }

        fn uvlc(&mut self, value: u32) {
            let code_num = value + 1;
            let leading_zeros = u32::BITS - 1 - code_num.leading_zeros();
            for _ in 0..leading_zeros {
                self.bit(0);
            }
            self.bit(1);
            if leading_zeros > 0 {
                self.f(code_num - (1 << leading_zeros), leading_zeros);
            }
        }

        fn into_bytes(self) -> Vec<u8> {
            let mut bytes = Vec::new();
            for chunk in self.bits.chunks(8) {
                let mut byte = 0u8;
                for (i, bit) in chunk.iter().enumerate() {
                    byte |= *bit << (7 - i);
                }
                bytes.push(byte);
            }
            bytes
        }
    }

    // ---- sub-view builders (mirror info.rs::tests::base_* / test_sub_views) -----------

    fn base_quant() -> CoreSeqQuantView {
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

    fn base_seg() -> CoreSeqSegView {
        CoreSeqSegView {
            seq_seg_info_present_flag: false,
            seq_allow_seg_info_change: false,
            enable_ext_seg: false,
            max_segments: 8,
            seq_segment_info: None,
        }
    }

    fn base_tile() -> CoreSeqTileView {
        CoreSeqTileView {
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
        }
    }

    fn base_filter() -> CoreSeqFilterView {
        CoreSeqFilterView {
            enable_cdef: false,
            enable_gdf: false,
            gdf_unit_matches_sb_size: false,
            disable_loopfilters_across_tiles: false,
            cdef_on_skip_txfm: CdefOnSkipTxfm::Adaptive,
            df_par_bits_minus_2: 0,
            single_picture_header_flag: false,
        }
    }

    fn base_restoration() -> CoreSeqRestorationView {
        CoreSeqRestorationView {
            enable_restoration: false,
            lr_pc_wiener_disabled: false,
            lr_wiener_nonsep_disabled: false,
            lr_uv_pc_wiener_disabled: false,
            lr_uv_wiener_nonsep_disabled: false,
        }
    }

    fn base_ccso() -> CoreSeqCcsoView {
        CoreSeqCcsoView {
            enable_ccso: false,
            single_picture_header_flag: false,
        }
    }

    fn base_inter() -> CoreSeqInterView {
        CoreSeqInterView {
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
        }
    }

    /// A representative non-single-picture sequence view (mirrors `info.rs::tests::base_seq`):
    /// OrderHintBits 4, NumRefFrames 8, no long-term ids, full refresh signaling, screen
    /// content forced off, 12-bit frame dimensions, 4096x2304 maximum, grain absent.
    fn base_seq() -> CoreSeqView {
        CoreSeqView {
            num_ref_frames: 8,
            order_hint_bits: 4,
            long_term_frame_id_bits: 0,
            enable_short_refresh_frame_flags: false,
            monotonic_output_order_flag: false,
            single_picture_header_flag: false,
            max_mlayer_id: 0,
            frame_width_bits: 12,
            frame_height_bits: 12,
            max_frame_width: 4096,
            max_frame_height: 2304,
            seq_force_screen_content_tools: 0,
            seq_force_integer_mv: 0,
            allow_frame_max_bvp_drl_bits: false,
            inter: base_inter(),
            quant: base_quant(),
            seg: base_seg(),
            tile: base_tile(),
            filter: base_filter(),
            restoration: base_restoration(),
            ccso: base_ccso(),
            chroma_format_idc: ChromaFormatIdc::Yuv420,
            film_grain_params_present: Some(false),
        }
    }

    fn parse(
        data: &[u8],
        obu_type: ObuType,
        first_pic: bool,
        seq: &CoreSeqView,
    ) -> FrameHeaderCore {
        parse_core_body_for_test(data, obu_type, first_pic, seq, None).unwrap()
    }

    fn write_core(core: &FrameHeaderCore, seq: &CoreSeqView, first_pic: bool) -> Vec<u8> {
        let mut writer = BitWriter::new();
        write_frame_header_core(&mut writer, core, seq, None, first_pic).unwrap();
        writer.into_bytes()
    }

    /// Asserts `written` and `original` carry the same first `bits` bits, MSB-first. The
    /// `written` buffer zero-pads its final partial byte, while `original` may keep arbitrary
    /// payload bits after the header in that byte, so the comparison is bit-granular.
    fn assert_bits_equal(written: &[u8], original: &[u8], bits: u64, obu_type: ObuType) {
        let full_bytes = (bits / 8) as usize;
        assert_eq!(
            &written[..full_bytes],
            &original[..full_bytes],
            "{obu_type:?}: written whole bytes are not exact"
        );
        let rem = (bits % 8) as u32;
        if rem != 0 {
            // Compare the high `rem` bits of the next byte; mask off the low bits (the
            // writer's zero pad vs the original's trailing payload).
            let mask = 0xffu8 << (8 - rem);
            assert_eq!(
                written[full_bytes] & mask,
                original[full_bytes] & mask,
                "{obu_type:?}: written partial byte's header bits are not exact"
            );
        }
    }

    /// Parses `data` to an `IntraHeaderComplete` core, writes it back, and asserts byte-exact
    /// output and an equal reparse (the full parse -> write -> parse round-trip). Returns the
    /// parsed core for further assertions.
    fn assert_roundtrip(
        data: &[u8],
        obu_type: ObuType,
        first_pic: bool,
        seq: &CoreSeqView,
    ) -> FrameHeaderCore {
        let core = parse(data, obu_type, first_pic, seq);
        assert_eq!(
            core.status,
            FrameHeaderParseStatus::IntraHeaderComplete,
            "fixture must parse to IntraHeaderComplete"
        );
        let written = write_core(&core, seq, first_pic);
        // Bit-exact over the consumed frame header (every descriptor is canonical here). The
        // written buffer zero-pads its final partial byte, while `data` may carry arbitrary
        // bits after the header in that same byte, so compare bit-for-bit up to consumed_bits.
        assert_bits_equal(&written, data, core.consumed_bits, obu_type);
        // Semantic round-trip: reparse equals the original core (consumed_bits may differ if
        // the original had trailing payload, so compare the structural fields via reparse).
        let reparsed = parse(&written, obu_type, first_pic, seq);
        assert_cores_equal(&reparsed, &core);
        core
    }

    /// Compares the two cores ignoring `consumed_bits` (the written buffer is exactly the
    /// frame header, while a fixture may carry trailing payload bytes).
    fn assert_cores_equal(a: &FrameHeaderCore, b: &FrameHeaderCore) {
        let mut a = a.clone();
        let mut b = b.clone();
        a.consumed_bits = 0;
        b.consumed_bits = 0;
        assert_eq!(a, b, "parse(write(core)) != core");
    }

    // ---- the canonical intra body fixtures --------------------------------------------

    /// CLK, cur_mfh_id == 0, full non-lossless intra path with grain absent (the exact bytes
    /// from `info.rs::tests::frame_header_core_reads_direct_sequence_reference`).
    fn clk_direct_reference_bits() -> Bits {
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(1); // seq_header_id_in_frame_header
        bits.bit(0); // immediate_output_frame
        bits.bit(0); // implicit_output_frame
        bits.bit(1); // frame_size_override_flag
        bits.f(5, 4); // order_hint
        // refresh_frame_flags: CLK + max_mlayer_id == 0 -> allFrames (no bits)
        bits.f(1920 - 1, 12); // frame_width_minus_1
        bits.f(1080 - 1, 12); // frame_height_minus_1
        bits.bit(0); // allow_intrabc
        bits.bit(0); // disable_cdf_update
        bits.bit(1); // uniform_tile_spacing_flag
        bits.bit(0); // increment_tile_cols_log2 = 0
        bits.bit(0); // increment_tile_rows_log2 = 0
        bits.f(90, 8); // base_q_idx
        bits.bit(0); // segmentation_enabled
        bits.bit(0); // using_qmatrix
        bits.bit(0); // delta_q_present
        bits.bit(0); // apply_deblocking_filter[0]
        bits.bit(0); // apply_deblocking_filter[1]
        bits.bit(0); // tx_mode_select = 0 -> TX_MODE_LARGEST
        bits.f(0, 2); // reduced_tx_set = 0
        bits
    }

    #[test]
    fn clk_non_lossless_round_trips() {
        let data = clk_direct_reference_bits().into_bytes();
        let core = assert_roundtrip(&data, ObuType::ClosedLoopKey, true, &base_seq());
        assert_eq!(core.frame_type, Some(FrameType::Key));
        assert_eq!(core.refresh_frame_flags, Some((1 << 8) - 1));
        assert!(core.cur_mfh_id.is_zero());
    }

    #[test]
    fn olk_round_trips() {
        // OLK: immediate_output_frame inferred false (no bit), KEY long_term (lt_bits == 0 ->
        // no bit), refresh_frame_flags direct f(NumRefFrames) (not CLK closed-loop).
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(2); // seq_header_id
        // OLK -> immediate_output_frame inferred false (no bit). monotonic == 0 so
        // implicit_output_frame is coded.
        bits.bit(0); // implicit_output_frame
        bits.bit(1); // frame_size_override_flag
        bits.f(3, 4); // order_hint
        bits.f(0b0000_0101, 8); // refresh_frame_flags f(NumRefFrames == 8) direct
        bits.f(640 - 1, 12); // frame_width_minus_1
        bits.f(480 - 1, 12); // frame_height_minus_1
        bits.bit(0); // allow_intrabc
        bits.bit(0); // disable_cdf_update
        bits.bit(1); // uniform_tile_spacing_flag
        bits.bit(0); // increment_tile_cols_log2
        bits.bit(0); // increment_tile_rows_log2
        bits.f(64, 8); // base_q_idx
        bits.bit(0); // segmentation_enabled
        bits.bit(0); // using_qmatrix
        bits.bit(0); // delta_q_present
        bits.bit(0); // apply_deblocking_filter[0]
        bits.bit(0); // apply_deblocking_filter[1]
        bits.bit(0); // tx_mode_select
        bits.f(0, 2); // reduced_tx_set
        let data = bits.into_bytes();
        let core = assert_roundtrip(&data, ObuType::OpenLoopKey, true, &base_seq());
        assert_eq!(core.immediate_output_frame, Some(false));
        assert_eq!(core.refresh_frame_flags, Some(0b0000_0101));
    }

    #[test]
    fn intra_only_round_trips() {
        // A RegularTileGroup that derives to INTRA_ONLY: frame_is_inter f(1) == 0.
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(0); // seq_header_id
        bits.bit(0); // frame_is_inter == 0 -> INTRA_ONLY_FRAME
        bits.bit(0); // immediate_output_frame
        bits.bit(0); // implicit_output_frame
        bits.bit(1); // frame_size_override_flag
        bits.f(7, 4); // order_hint
        bits.f(0b0000_0010, 8); // refresh_frame_flags f(NumRefFrames) (INTRA_ONLY direct arm)
        bits.f(320 - 1, 12); // frame_width_minus_1
        bits.f(240 - 1, 12); // frame_height_minus_1
        bits.bit(0); // allow_intrabc
        bits.bit(0); // disable_cdf_update
        bits.bit(1); // uniform_tile_spacing_flag
        bits.bit(0); // increment_tile_cols_log2
        bits.bit(0); // increment_tile_rows_log2
        bits.f(100, 8); // base_q_idx
        bits.bit(0); // segmentation_enabled
        bits.bit(0); // using_qmatrix
        bits.bit(0); // delta_q_present
        bits.bit(0); // apply_deblocking_filter[0]
        bits.bit(0); // apply_deblocking_filter[1]
        bits.bit(0); // tx_mode_select
        bits.f(0, 2); // reduced_tx_set
        let data = bits.into_bytes();
        let core = assert_roundtrip(&data, ObuType::RegularTileGroup, false, &base_seq());
        assert_eq!(core.frame_type, Some(FrameType::IntraOnly));
        assert_eq!(core.refresh_frame_flags, Some(0b0000_0010));
    }

    #[test]
    fn single_picture_key_round_trips() {
        // single_picture_header_flag forces KEY, no frame_type/output/override bits. With
        // single_picture set on every sub-view that consults it, gdf/cdef infer frame_enable.
        let mut seq = base_seq();
        seq.single_picture_header_flag = true;
        seq.filter.single_picture_header_flag = true;
        seq.ccso.single_picture_header_flag = true;
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(0); // seq_header_id
        // single_picture: no frame_type arm, no output flags, no frame_size_override_flag.
        bits.f(9, 4); // order_hint
        // refresh_frame_flags: CLK + max_mlayer_id == 0 -> allFrames (no bits)
        // frame_size: override inferred false -> dims come from seq maxima (4096x2304).
        bits.bit(0); // allow_intrabc
        bits.bit(0); // disable_cdf_update
        // tile_info() for 4096x2304 with 128x128 superblocks.
        bits.bit(1); // uniform_tile_spacing_flag
        bits.bit(0); // increment_tile_cols_log2
        bits.bit(0); // increment_tile_rows_log2
        bits.f(120, 8); // base_q_idx
        bits.bit(0); // segmentation_enabled
        bits.bit(0); // using_qmatrix
        bits.bit(0); // delta_q_present
        bits.bit(0); // apply_deblocking_filter[0]
        bits.bit(0); // apply_deblocking_filter[1]
        // gdf/cdef: single_picture -> frame_enable inferred true, but enable_gdf/cdef == 0
        // short-circuits before the inference is reached (no bits either way).
        bits.bit(0); // tx_mode_select
        bits.f(0, 2); // reduced_tx_set
        let data = bits.into_bytes();
        let core = assert_roundtrip(&data, ObuType::ClosedLoopKey, true, &seq);
        assert_eq!(core.frame_size_override_flag, Some(false));
        assert_eq!(core.frame_size, Some(FrameSize::new(4096, 2304)));
        assert_eq!(core.immediate_output_frame, Some(true));
        assert_eq!(core.implicit_output_frame, Some(false));
    }

    #[test]
    fn lossless_round_trips() {
        // base_q_idx == 0 with no delta-Q -> CodedLossless == 1: read_tx_mode() infers
        // ONLY_4X4 (no tx_mode_select bit), and deblocking is skipped (coded_lossless).
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(0); // seq_header_id
        bits.bit(0); // immediate_output_frame
        bits.bit(0); // implicit_output_frame
        bits.bit(1); // frame_size_override_flag
        bits.f(0, 4); // order_hint
        bits.f(256 - 1, 12); // frame_width_minus_1
        bits.f(256 - 1, 12); // frame_height_minus_1
        bits.bit(0); // allow_intrabc
        bits.bit(0); // disable_cdf_update
        bits.bit(1); // uniform_tile_spacing_flag
        bits.bit(0); // increment_tile_cols_log2
        bits.bit(0); // increment_tile_rows_log2
        bits.f(0, 8); // base_q_idx == 0 -> lossless candidate
        bits.bit(0); // segmentation_enabled
        bits.bit(0); // using_qmatrix
        // delta_q_present is not read when base_q_idx == 0.
        // lossless tail: coded_lossless -> allow_tcq/parity inference (no bits), deblocking
        // skipped, gdf/cdef disabled, lr/ccso disabled.
        // read_tx_mode(): coded_lossless -> ONLY_4X4 (no bit). reduced_tx_set f(2) still read.
        bits.f(0, 2); // reduced_tx_set
        let data = bits.into_bytes();
        let core = assert_roundtrip(&data, ObuType::ClosedLoopKey, true, &base_seq());
        assert!(core.lossless_info.as_ref().unwrap().coded_lossless);
    }

    #[test]
    fn grain_present_round_trips() {
        // film_grain_params_present == 1 with an output gate that codes apply_grain.
        // immediate_output_frame == 0 and implicit_output_frame == 0 (not single-picture) ->
        // the (!immediate && !implicit) gate is true, so apply_grain f(1) is read.
        let mut seq = base_seq();
        seq.film_grain_params_present = Some(true);
        let mut bits = clk_direct_reference_bits();
        // The base fixture ends after reduced_tx_set; append the grain config. With
        // apply_grain == 0 the film_grain_config() reads only the apply_grain bit.
        bits.bit(0); // apply_grain == 0
        let data = bits.into_bytes();
        let core = assert_roundtrip(&data, ObuType::ClosedLoopKey, true, &seq);
        assert!(!core.intra_tail.as_ref().unwrap().film_grain.apply_grain);
    }

    #[test]
    fn multi_tile_round_trips() {
        // A 1920x1080 frame with explicit non-default tile log2 increments -> a multi-tile
        // layout, exercising the tile_info() / gdf geometry paths through the writer.
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(1); // seq_header_id
        bits.bit(0); // immediate_output_frame
        bits.bit(0); // implicit_output_frame
        bits.bit(1); // frame_size_override_flag
        bits.f(5, 4); // order_hint
        bits.f(1920 - 1, 12); // frame_width_minus_1
        bits.f(1080 - 1, 12); // frame_height_minus_1
        bits.bit(0); // allow_intrabc
        bits.bit(0); // disable_cdf_update
        // tile_info(): 1920x1080 @ 128x128 -> sbCols 15, sbRows 9. Request 2 tile cols.
        bits.bit(1); // uniform_tile_spacing_flag
        bits.bit(1); // increment_tile_cols_log2 = 1 (TileColsLog2 -> 1)
        bits.bit(0); // stop incrementing cols
        bits.bit(0); // increment_tile_rows_log2 = 0
        // multi-tile -> context_update_tile_id f(TileColsLog2 + TileRowsLog2 == 1) + tile
        // size bytes minus 1 f(2).
        bits.f(0, 1); // context_update_tile_id
        bits.f(0, 2); // tile_size_bytes_minus_1
        bits.f(90, 8); // base_q_idx
        bits.bit(0); // segmentation_enabled
        bits.bit(0); // using_qmatrix
        bits.bit(0); // delta_q_present
        bits.bit(0); // apply_deblocking_filter[0]
        bits.bit(0); // apply_deblocking_filter[1]
        bits.bit(0); // tx_mode_select
        bits.f(0, 2); // reduced_tx_set
        let data = bits.into_bytes();
        let core = assert_roundtrip(&data, ObuType::ClosedLoopKey, true, &base_seq());
        let tile = core.tile_info.as_ref().unwrap();
        assert!(tile.tile_cols >= 2, "fixture should be multi-tile");
    }

    #[test]
    fn mfh_nonzero_round_trips() {
        // cur_mfh_id == 1 resolved against an MFH record: frame_size override flag == 0 uses
        // the MFH default dims, and the writer threads the same MfhFrameView.
        let mfh_size = crate::hls::MfhFrameSize {
            width_bits: 12,
            height_bits: 12,
            width_minus_1: 1920 - 1,
            height_minus_1: 1080 - 1,
        };
        let record = crate::hls::MultiFrameHeaderRecord {
            mfh_id: crate::hls::MfhId::from_raw(1),
            mfh_seq_header_id: crate::headers::sequence::SequenceHeaderId::try_new(0).unwrap(),
            mfh_tlayer_id: crate::types::TemporalLayerId::from_bits(0),
            mfh_mlayer_id: crate::types::EmbeddedLayerId::from_bits(0),
            mfh_frame_size: Some(mfh_size),
            mfh_seg_info_present_flag: false,
            mfh_ext_seg_flag: None,
            mfh_allow_seg_info_change: None,
            mfh_segment_info: None,
            mfh_deblocking_filter_update: false,
            mfh_apply_deblocking_filter: [false; 4],
            offset: crate::span::ByteOffset::new(0),
        };
        let seq = base_seq();
        let view = MfhFrameView::from_record(&record, &seq);

        let mut bits = Bits::default();
        bits.uvlc(1); // cur_mfh_id == 1
        bits.bit(0); // immediate_output_frame
        bits.bit(0); // implicit_output_frame
        bits.bit(0); // frame_size_override_flag == 0 (MFH default dims, no size bits)
        bits.f(7, 4); // order_hint
        // refresh_frame_flags: CLK + max_mlayer_id == 0 -> allFrames (no bits)
        bits.bit(0); // allow_intrabc
        bits.bit(0); // disable_cdf_update
        bits.bit(1); // uniform_tile_spacing_flag
        bits.bit(0); // increment_tile_cols_log2
        bits.bit(0); // increment_tile_rows_log2
        bits.f(90, 8); // base_q_idx
        bits.bit(0); // segmentation_enabled
        bits.bit(0); // using_qmatrix
        bits.bit(0); // delta_q_present
        bits.bit(0); // apply_deblocking_filter[0]
        bits.bit(0); // apply_deblocking_filter[1]
        bits.bit(0); // tx_mode_select
        bits.f(0, 2); // reduced_tx_set
        let data = bits.into_bytes();

        let core =
            parse_core_body_for_test(&data, ObuType::ClosedLoopKey, true, &seq, Some(&view))
                .unwrap();
        assert_eq!(core.status, FrameHeaderParseStatus::IntraHeaderComplete);
        assert_eq!(core.frame_size, Some(FrameSize::new(1920, 1080)));

        let mut writer = BitWriter::new();
        write_frame_header_core(&mut writer, &core, &seq, Some(&view), true).unwrap();
        let written = writer.into_bytes();
        assert_bits_equal(&written, &data, core.consumed_bits, ObuType::ClosedLoopKey);
        let reparsed =
            parse_core_body_for_test(&written, ObuType::ClosedLoopKey, true, &seq, Some(&view))
                .unwrap();
        assert_cores_equal(&reparsed, &core);
    }

    // ---- rejection tests (reject-before-write, bit_len() == 0) -------------------------

    /// Builds a valid CLK intra core for mutation in the rejection tests.
    fn valid_core() -> (FrameHeaderCore, CoreSeqView) {
        let seq = base_seq();
        let data = clk_direct_reference_bits().into_bytes();
        let core = parse(&data, ObuType::ClosedLoopKey, true, &seq);
        assert_eq!(core.status, FrameHeaderParseStatus::IntraHeaderComplete);
        (core, seq)
    }

    /// Asserts the writer rejects `core` with `what`, leaving the buffer untouched. `first_pic`
    /// is the `FirstPictureInTU` the core was parsed with, threaded so the `starts_cvs`
    /// derivation matches (a genuine parsed core is accepted apart from the mutated field).
    fn assert_rejected_what(
        core: &FrameHeaderCore,
        seq: &CoreSeqView,
        first_pic: bool,
        what: &'static str,
    ) {
        let mut writer = BitWriter::new();
        let err = write_frame_header_core(&mut writer, core, seq, None, first_pic).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader { what },
            "expected reject {what}"
        );
        assert_eq!(writer.bit_len(), 0, "{what}: bits written on reject");
    }

    #[test]
    fn reject_non_intra_status() {
        // Every non-IntraHeaderComplete status is rejected at `status` before any bit.
        for status in [
            FrameHeaderParseStatus::ActivationFieldsOnly,
            FrameHeaderParseStatus::ShowExistingFrameComplete,
            FrameHeaderParseStatus::StoppedInsideFilterParams,
            FrameHeaderParseStatus::StoppedInsideIntraTail,
            FrameHeaderParseStatus::StoppedInsideInterControl,
            FrameHeaderParseStatus::UnsupportedUntilFeature {
                feature_id: "AV2-5.18.2-FRAME-HEADER-INFO",
            },
            FrameHeaderParseStatus::StoppedBeforeWienerNsFilter {
                feature_id: "AV2-5.18.7-SEGMENTATION-TILING",
            },
        ] {
            let (mut core, seq) = valid_core();
            core.status = status;
            assert_rejected_what(&core, &seq, true, "status");
        }
    }

    #[test]
    fn reject_missing_required_option() {
        // A None on any required intra-path Option rejects with that field's label.
        let (mut core, seq) = valid_core();
        core.tile_info = None;
        assert_rejected_what(&core, &seq, true, "tile_info");

        let (mut core, seq) = valid_core();
        core.intra_tail = None;
        assert_rejected_what(&core, &seq, true, "intra_tail");

        let (mut core, seq) = valid_core();
        core.lr_params = None;
        assert_rejected_what(&core, &seq, true, "lr_params");

        let (mut core, seq) = valid_core();
        core.order_hint_lsb = None;
        assert_rejected_what(&core, &seq, true, "order_hint_lsb");
    }

    #[test]
    fn reject_lr_params_partial_set() {
        // `LrPartialParams` is #[non_exhaustive], so it cannot be built directly. Obtain a
        // real partial straight from `parse_lr_params` (the same bits the restoration parser's
        // `lr_frame_filters_on_stops_before_wienerns` test uses), then graft it onto a valid
        // IntraHeaderComplete core so the dedicated `lr_params_partial` reject fires.
        use crate::headers::frame::{LrGeometry, LrParseOutcome, parse_lr_params};
        let restoration = CoreSeqRestorationView {
            enable_restoration: true,
            lr_pc_wiener_disabled: false,
            lr_wiener_nonsep_disabled: false,
            lr_uv_pc_wiener_disabled: true,
            lr_uv_wiener_nonsep_disabled: false,
        };
        let geometry = LrGeometry::new(SuperblockSize::Block128x128, ChromaFormatIdc::Yuv420);
        let mut bits = Bits::default();
        // The known Wiener-stop bit pattern: plane-0 tool_index -> RESTORE_WIENER_NONSEP,
        // frame_filters_on[0] == 1, num_filter_classes_idx, planes 1/2 RESTORE_NONE, luma size.
        ns_bits(&mut bits, 2, 4); // plane 0 tool_index == 2 (n == 4 with these disables)
        bits.bit(1); // frame_filters_on[0] == 1
        bits.f(4, 3); // num_filter_classes_idx == 4
        ns_bits(&mut bits, 0, 2); // plane 1 -> RESTORE_NONE
        ns_bits(&mut bits, 0, 2); // plane 2 -> RESTORE_NONE
        bits.bit(1); // luma size flag -> read_wienerns_filter (honest stop)
        let mut data = bits.into_bytes();
        data.extend_from_slice(&[0u8; 4]);
        let mut reader = crate::bitio::BitReader::new(&data, crate::span::ByteOffset::new(0));
        let outcome =
            parse_lr_params(&mut reader, false, 3, &restoration, geometry, 100).unwrap();
        let partial = match outcome {
            LrParseOutcome::StoppedBeforeWienerNsFilter { partial, .. } => partial,
            other => panic!("expected StoppedBeforeWienerNsFilter, got {other:?}"),
        };

        let (mut core, seq) = valid_core();
        core.lr_params_partial = Some(partial);
        assert_rejected_what(&core, &seq, true, "lr_params_partial");
    }

    /// `ns(n)` encoding into a `Bits` builder, mirroring `info.rs::tests::Bits::ns`.
    fn ns_bits(bits: &mut Bits, value: u32, n: u32) {
        let w = u32::BITS - n.leading_zeros();
        let m = (1u32 << w) - n;
        if value < m {
            bits.f(value, w - 1);
        } else {
            bits.f(value + m, w);
        }
    }

    #[test]
    fn reject_show_existing_model() {
        // show_existing_frame == Some(true) is a SEF model the intra writer never emits.
        let (mut core, seq) = valid_core();
        core.show_existing_frame = Some(true);
        assert_rejected_what(&core, &seq, true, "show_existing_frame");
    }

    #[test]
    fn reject_inferred_immediate_output_disagrees() {
        // OLK infers immediate_output_frame == false; storing true is non-canonical. Build an
        // OLK core via its obu_type so frame_type stays an intra type (KEY).
        let mut bits = Bits::default();
        bits.uvlc(0);
        bits.uvlc(0);
        bits.bit(0); // implicit_output_frame (OLK: immediate inferred false, no bit)
        bits.bit(1); // frame_size_override_flag
        bits.f(3, 4); // order_hint
        bits.f(0, 8); // refresh_frame_flags direct
        bits.f(64 - 1, 12); // frame_width_minus_1
        bits.f(64 - 1, 12); // frame_height_minus_1
        bits.bit(0); // allow_intrabc
        bits.bit(0); // disable_cdf_update
        bits.bit(1); // uniform_tile_spacing_flag
        bits.f(90, 8); // base_q_idx (single superblock -> no increment/context bits)
        bits.bit(0); // segmentation_enabled
        bits.bit(0); // using_qmatrix
        bits.bit(0); // delta_q_present
        bits.bit(0); // apply_deblocking_filter[0]
        bits.bit(0); // apply_deblocking_filter[1]
        bits.bit(0); // tx_mode_select
        bits.f(0, 2); // reduced_tx_set
        let data = bits.into_bytes();
        let seq = base_seq();
        let mut core = parse(&data, ObuType::OpenLoopKey, true, &seq);
        assert_eq!(core.status, FrameHeaderParseStatus::IntraHeaderComplete);
        core.immediate_output_frame = Some(true); // OLK infers false
        assert_rejected_what(&core, &seq, true, "immediate_output_frame");
    }

    #[test]
    fn reject_refresh_flags_arm_cannot_represent() {
        // CLK short-refresh arm encodes a single-bit refresh_frame_flags; a multi-bit value
        // cannot be represented, so it is rejected. Use a seq with short-refresh and a
        // non-closed-loop key so the short arm is selected.
        let mut seq = base_seq();
        seq.enable_short_refresh_frame_flags = true;
        seq.max_mlayer_id = 1; // CLK no longer takes the inferred all-frames arm
        let mut bits = Bits::default();
        bits.uvlc(0);
        bits.uvlc(0);
        bits.bit(0); // immediate_output_frame
        bits.bit(0); // implicit_output_frame
        bits.bit(1); // frame_size_override_flag
        bits.f(3, 4); // order_hint
        bits.f(2, 3); // frame_to_refresh f(CeilLog2(8) == 3) -> refresh == 1 << 2
        bits.f(64 - 1, 12);
        bits.f(64 - 1, 12);
        bits.bit(0); // allow_intrabc
        bits.bit(0); // disable_cdf_update
        bits.bit(1); // uniform_tile_spacing_flag
        bits.f(90, 8); // base_q_idx
        bits.bit(0); // segmentation_enabled
        bits.bit(0); // using_qmatrix
        bits.bit(0); // delta_q_present
        bits.bit(0); // apply_deblocking_filter[0]
        bits.bit(0); // apply_deblocking_filter[1]
        bits.bit(0); // tx_mode_select
        bits.f(0, 2); // reduced_tx_set
        let data = bits.into_bytes();
        let mut core = parse(&data, ObuType::ClosedLoopKey, true, &seq);
        assert_eq!(core.status, FrameHeaderParseStatus::IntraHeaderComplete);
        assert_eq!(core.refresh_frame_flags, Some(1 << 2));
        // Mutate to a multi-bit value the short arm cannot represent.
        core.refresh_frame_flags = Some(0b0000_0011);
        assert_rejected_what(&core, &seq, true, "refresh_frame_flags");
    }

    #[test]
    fn long_term_id_overflow_is_rejected_not_panicked() {
        // Regression (#4i adversarial review, PH-1): a KEY core with long_term_id == i64::MAX
        // must reject cleanly, not panic on the `long_term_id + 1` increment (workspace
        // overflow-checks = true traps a bare add). valid_core() is a CLK key frame, so the
        // long_term_id encodability check is reached.
        let (mut core, seq) = valid_core();
        core.long_term_id = Some(i64::MAX);
        assert_rejected_what(&core, &seq, true, "long_term_id");
    }

    #[test]
    fn single_picture_open_loop_key_round_trips() {
        // Regression (#4i adversarial review, RT-1): a single-picture OBU_OPEN_LOOP_KEY frame
        // forces KEY and immediate_output_frame = 1 (inferred, no bit), so the OLK
        // immediate_output_frame disagreement check must be gated on !single_picture; otherwise
        // this genuine IntraHeaderComplete model is falsely rejected. OLK is not closed-loop, so
        // refresh_frame_flags takes the direct f(NumRefFrames) arm (not the all-frames inference).
        let mut seq = base_seq();
        seq.single_picture_header_flag = true;
        seq.filter.single_picture_header_flag = true;
        seq.ccso.single_picture_header_flag = true;
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(0); // seq_header_id
        bits.f(9, 4); // order_hint
        bits.f(0, 8); // refresh_frame_flags (OLK -> direct f(NumRefFrames == 8) arm)
        bits.bit(0); // allow_intrabc
        bits.bit(0); // disable_cdf_update
        bits.bit(1); // uniform_tile_spacing_flag
        bits.bit(0); // increment_tile_cols_log2
        bits.bit(0); // increment_tile_rows_log2
        bits.f(120, 8); // base_q_idx
        bits.bit(0); // segmentation_enabled
        bits.bit(0); // using_qmatrix
        bits.bit(0); // delta_q_present
        bits.bit(0); // apply_deblocking_filter[0]
        bits.bit(0); // apply_deblocking_filter[1]
        bits.bit(0); // tx_mode_select
        bits.f(0, 2); // reduced_tx_set
        let data = bits.into_bytes();
        let core = assert_roundtrip(&data, ObuType::OpenLoopKey, true, &seq);
        assert_eq!(core.frame_type, Some(FrameType::Key));
        assert_eq!(core.immediate_output_frame, Some(true));
    }

    // ---- reject-completeness tests (#4i review findings 1-8) ---------------------------

    /// A representative single-picture sequence (mirrors `single_picture_key_round_trips`),
    /// with `single_picture_header_flag` set on every sub-view that consults it.
    fn single_picture_seq() -> CoreSeqView {
        let mut seq = base_seq();
        seq.single_picture_header_flag = true;
        seq.filter.single_picture_header_flag = true;
        seq.ccso.single_picture_header_flag = true;
        seq
    }

    /// The canonical single-picture CLK body bytes (the fixture from
    /// `single_picture_key_round_trips`), parsing to an `IntraHeaderComplete` core.
    fn single_picture_clk_bits() -> Bits {
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(0); // seq_header_id
        bits.f(9, 4); // order_hint
        bits.bit(0); // allow_intrabc
        bits.bit(0); // disable_cdf_update
        bits.bit(1); // uniform_tile_spacing_flag
        bits.bit(0); // increment_tile_cols_log2
        bits.bit(0); // increment_tile_rows_log2
        bits.f(120, 8); // base_q_idx
        bits.bit(0); // segmentation_enabled
        bits.bit(0); // using_qmatrix
        bits.bit(0); // delta_q_present
        bits.bit(0); // apply_deblocking_filter[0]
        bits.bit(0); // apply_deblocking_filter[1]
        bits.bit(0); // tx_mode_select
        bits.f(0, 2); // reduced_tx_set
        bits
    }

    /// Builds a valid single-picture CLK intra core for mutation in the single-picture-gated
    /// rejection tests.
    fn valid_single_picture_core() -> (FrameHeaderCore, CoreSeqView) {
        let seq = single_picture_seq();
        let data = single_picture_clk_bits().into_bytes();
        let core = parse(&data, ObuType::ClosedLoopKey, true, &seq);
        assert_eq!(core.status, FrameHeaderParseStatus::IntraHeaderComplete);
        (core, seq)
    }

    #[test]
    fn reject_mismatched_inferred_frame_type() {
        // Finding 1 (info.rs:1146/1183/1190): a single-picture / CLK / OLK core derives KEY with
        // no frame_type bit; storing IntraOnly is parser-unreachable. valid_core() is a CLK key
        // frame, so the expected derived type is KEY.
        let (mut core, seq) = valid_core();
        core.frame_type = Some(FrameType::IntraOnly);
        assert_rejected_what(&core, &seq, true, "frame_type");
    }

    #[test]
    fn reject_mfh_view_on_direct_reference() {
        // Finding 2 (info.rs:1593-1596): a cur_mfh_id == 0 (direct) core takes the mfh = None
        // arms; supplying an MFH view is parser-unreachable. valid_core() is cur_mfh_id == 0.
        let (core, seq) = valid_core();
        assert!(core.cur_mfh_id.is_zero());
        let record = crate::hls::MultiFrameHeaderRecord {
            mfh_id: crate::hls::MfhId::from_raw(1),
            mfh_seq_header_id: crate::headers::sequence::SequenceHeaderId::try_new(0).unwrap(),
            mfh_tlayer_id: crate::types::TemporalLayerId::from_bits(0),
            mfh_mlayer_id: crate::types::EmbeddedLayerId::from_bits(0),
            mfh_frame_size: None,
            mfh_seg_info_present_flag: false,
            mfh_ext_seg_flag: None,
            mfh_allow_seg_info_change: None,
            mfh_segment_info: None,
            mfh_deblocking_filter_update: false,
            mfh_apply_deblocking_filter: [false; 4],
            offset: crate::span::ByteOffset::new(0),
        };
        let view = MfhFrameView::from_record(&record, &seq);
        let mut writer = BitWriter::new();
        let err = write_frame_header_core(&mut writer, &core, &seq, Some(&view), true).unwrap_err();
        assert_eq!(err, WriteError::NonCanonicalFrameHeader { what: "mfh_record" });
        assert_eq!(writer.bit_len(), 0, "mfh_record: bits written on reject");
    }

    #[test]
    fn reject_show_existing_frame_none() {
        // Finding 3 (info.rs:1145/1167): every IntraHeaderComplete core sets
        // show_existing_frame = Some(false); a None reparses to Some(false), so it must reject.
        let (mut core, seq) = valid_core();
        core.show_existing_frame = None;
        assert_rejected_what(&core, &seq, true, "show_existing_frame");
    }

    #[test]
    fn reject_mirrored_allow_intrabc_disagrees() {
        // Finding 4 (info.rs:1613): the flat core.allow_intrabc mirrors intrabc.allow_intrabc;
        // the writer emits from core.intrabc, so a disagreeing flat field is parser-unreachable.
        let (mut core, seq) = valid_core();
        let mirrored = core.intrabc.as_ref().unwrap().allow_intrabc;
        core.allow_intrabc = Some(!mirrored);
        assert_rejected_what(&core, &seq, true, "allow_intrabc");
    }

    #[test]
    fn reject_forbidden_ref_long_term_id_mismatch() {
        // Finding 5 (info.rs:1217/1222-1223): forbidden_ref_long_term_id is derived from the
        // ref_long_term_ids vs the reserved all-ones value. valid_core() codes no ref ids, so the
        // derived flag is false; storing true is parser-unreachable.
        let (mut core, seq) = valid_core();
        assert!(!core.forbidden_ref_long_term_id);
        core.forbidden_ref_long_term_id = true;
        assert_rejected_what(&core, &seq, true, "forbidden_ref_long_term_id");
    }

    #[test]
    fn reject_single_picture_output_inference_disagrees() {
        // Finding 6 (info.rs:1148-1149): single_picture infers immediate=true / implicit=false
        // with no bits; any other pair is parser-unreachable.
        let (mut core, seq) = valid_single_picture_core();
        core.immediate_output_frame = Some(false);
        assert_rejected_what(&core, &seq, true, "immediate_output_frame");

        let (mut core, seq) = valid_single_picture_core();
        core.implicit_output_frame = Some(true);
        assert_rejected_what(&core, &seq, true, "implicit_output_frame");
    }

    #[test]
    fn reject_stale_long_term_id_on_no_bit_arms() {
        // Finding 7 (info.rs:1150/1207): single_picture leaves long_term_id None; a non-single
        // INTRA_ONLY frame leaves it the Some(-1) sentinel. Any other value is parser-unreachable.
        let (mut core, seq) = valid_single_picture_core();
        assert_eq!(core.long_term_id, None);
        core.long_term_id = Some(0);
        assert_rejected_what(&core, &seq, true, "long_term_id");

        // Non-single INTRA_ONLY: build via a RegularTileGroup deriving to IntraOnly.
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(0); // seq_header_id
        bits.bit(0); // frame_is_inter == 0 -> INTRA_ONLY_FRAME
        bits.bit(0); // immediate_output_frame
        bits.bit(0); // implicit_output_frame
        bits.bit(1); // frame_size_override_flag
        bits.f(7, 4); // order_hint
        bits.f(0b0000_0010, 8); // refresh_frame_flags
        bits.f(320 - 1, 12); // frame_width_minus_1
        bits.f(240 - 1, 12); // frame_height_minus_1
        bits.bit(0); // allow_intrabc
        bits.bit(0); // disable_cdf_update
        bits.bit(1); // uniform_tile_spacing_flag
        bits.bit(0); // increment_tile_cols_log2
        bits.bit(0); // increment_tile_rows_log2
        bits.f(100, 8); // base_q_idx
        bits.bit(0); // segmentation_enabled
        bits.bit(0); // using_qmatrix
        bits.bit(0); // delta_q_present
        bits.bit(0); // apply_deblocking_filter[0]
        bits.bit(0); // apply_deblocking_filter[1]
        bits.bit(0); // tx_mode_select
        bits.f(0, 2); // reduced_tx_set
        let data = bits.into_bytes();
        let seq = base_seq();
        let mut core = parse(&data, ObuType::RegularTileGroup, false, &seq);
        assert_eq!(core.frame_type, Some(FrameType::IntraOnly));
        assert_eq!(core.long_term_id, Some(-1));
        core.long_term_id = Some(0); // not the -1 sentinel
        assert_rejected_what(&core, &seq, false, "long_term_id");
    }

    #[test]
    fn reject_reached_qm_reset_true() {
        // Finding 8 (info.rs:1242): every IntraHeaderComplete path leaves reached_qm_reset false
        // (RAS/SWITCH are non-intra; single-picture returns before the derivation). A true value
        // is parser-unreachable.
        let (mut core, seq) = valid_core();
        assert!(!core.reached_qm_reset);
        core.reached_qm_reset = true;
        assert_rejected_what(&core, &seq, true, "reached_qm_reset");
    }

    // ---- reject-completeness tests (#4i second-round review findings) ------------------

    #[test]
    fn reject_non_single_bridge_intra_model() {
        // Round-2 finding 1 (info.rs:1157-1163): a NON-single bridge frame reads
        // bridge_frame_ref_idx, then takes the inter arm (frame_type = Inter) — it never reaches
        // IntraHeaderComplete. The generic frame_type derivation would map a non-single
        // BridgeFrame to the IntraOnly expectation, so a hand-built non-single bridge intra core
        // must be rejected. Start from a valid (non-single) CLK core, retype it to a bridge.
        let (mut core, seq) = valid_core();
        assert!(!seq.single_picture_header_flag);
        core.obu_type = ObuType::BridgeFrame;
        core.is_bridge = true;
        core.bridge_frame_ref_idx = Some(0);
        // A BridgeFrame is not OBU_CLOSED_LOOP_KEY, so its parser-derived starts_cvs is false;
        // match it so the test reaches its intended bridge reject (not `starts_cvs`).
        core.starts_cvs = false;
        // The composer supports no bridge path (a non-single bridge is inter).
        assert_rejected_what(&core, &seq, true, "bridge_unsupported");
    }

    #[test]
    fn reject_stale_bridge_frame_ref_idx_on_non_bridge() {
        // Round-2 finding 3 (info.rs:1131-1133): the parser leaves bridge_frame_ref_idx = None
        // for every non-bridge header; the writer emits it only on the is_bridge arm, so a stale
        // value on a non-bridge core would be silently dropped. valid_core() is a non-bridge CLK.
        let (mut core, seq) = valid_core();
        assert!(!core.is_bridge);
        core.bridge_frame_ref_idx = Some(3);
        assert_rejected_what(&core, &seq, true, "bridge_frame_ref_idx");
    }

    #[test]
    fn reject_stale_frame_to_show_map_idx() {
        // Round-2 finding 4 (info.rs:1478): frame_to_show_map_idx is read only on the
        // show-existing-frame path; the intra path leaves it None.
        let (mut core, seq) = valid_core();
        assert_eq!(core.frame_to_show_map_idx, None);
        core.frame_to_show_map_idx = Some(2);
        assert_rejected_what(&core, &seq, true, "frame_to_show_map_idx");
    }

    #[test]
    fn reject_stale_inter_control() {
        // Round-2 finding 5 (info.rs:1368): `inter` is the non-intra control region, None on
        // every intra-complete path. `InterControl` derives Default, so build a default one.
        let (mut core, seq) = valid_core();
        assert!(core.inter.is_none());
        core.inter = Some(crate::headers::frame::InterControl::default());
        assert_rejected_what(&core, &seq, true, "inter");
    }

    #[test]
    fn reject_stale_sef_film_grain() {
        // Round-2 finding 7 (info.rs:1518): sef_film_grain is the show-existing-frame
        // film_grain_config, None on the intra path. `FilmGrainConfig` is #[non_exhaustive], so
        // clone a real one from the intra tail's parsed film_grain rather than constructing it.
        let (mut core, seq) = valid_core();
        assert!(core.sef_film_grain.is_none());
        let grain = core.intra_tail.as_ref().unwrap().film_grain;
        core.sef_film_grain = Some(grain);
        assert_rejected_what(&core, &seq, true, "sef_film_grain");
    }

    #[test]
    fn reject_stale_sef_trailing_bits() {
        // Round-2 finding 8 (info.rs:1526): sef_trailing_bits is the SEF-only trailing-bits
        // boundary, None on the intra path.
        let (mut core, seq) = valid_core();
        assert!(core.sef_trailing_bits.is_none());
        core.sef_trailing_bits = Some(crate::headers::frame::SefTrailingBits::Valid);
        assert_rejected_what(&core, &seq, true, "sef_trailing_bits");
    }

    #[test]
    fn reject_stale_starts_cvs() {
        // Round-3 finding (info.rs:1065): starts_cvs is derived, not coded
        // (`obu_type == OBU_CLOSED_LOOP_KEY && FirstPictureInTU`); the prefix writes no bits for
        // it, so a mutated value would reparse to the FirstPictureInTU-derived one and silently
        // round-trip wrong. valid_core() is a CLK parsed with first_picture_in_tu = true, so its
        // starts_cvs is true; flipping it to false is parser-unreachable for that input.
        let (mut core, seq) = valid_core();
        assert_eq!(core.obu_type, ObuType::ClosedLoopKey);
        assert!(core.starts_cvs);
        core.starts_cvs = false;
        assert_rejected_what(&core, &seq, true, "starts_cvs");

        // Conversely, the same CLK parsed with first_picture_in_tu = false yields
        // starts_cvs == false and round-trips when written with first_picture_in_tu = false (the
        // derivation matches, so the write is accepted).
        let data = clk_direct_reference_bits().into_bytes();
        let core = parse(&data, ObuType::ClosedLoopKey, false, &seq);
        assert!(!core.starts_cvs);
        let written = write_core(&core, &seq, false);
        assert_bits_equal(&written, &data, core.consumed_bits, ObuType::ClosedLoopKey);
        let reparsed = parse(&written, ObuType::ClosedLoopKey, false, &seq);
        assert_cores_equal(&reparsed, &core);
    }

    #[test]
    fn single_picture_sef_round_trips() {
        // Round-2 finding 2 (info.rs:1135-1150): the single-picture branch forces a KEY intra
        // frame and returns BEFORE the is_sef() check (:1166) for ANY obu_type, so a
        // single-picture SEF OBU parses to IntraHeaderComplete (a key frame). The writer must NOT
        // reject it (the is_sef/is_tip rejection is gated on !single_picture). End-to-end
        // byte-exact round-trip on the SEF obu_type with a single-picture sequence.
        let seq = single_picture_seq();
        // A single-picture SEF reads no bridge_frame_ref_idx (not a bridge) and skips the
        // frame-type / output block, so its body is the single-picture CLK body. RegularSef takes
        // the direct refresh_frame_flags arm (not CLK closed-loop), so include those 8 bits.
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(0); // seq_header_id
        bits.f(9, 4); // order_hint
        bits.f(0, 8); // refresh_frame_flags f(NumRefFrames == 8) direct (RegularSef not CLK)
        bits.bit(0); // allow_intrabc
        bits.bit(0); // disable_cdf_update
        bits.bit(1); // uniform_tile_spacing_flag
        bits.bit(0); // increment_tile_cols_log2
        bits.bit(0); // increment_tile_rows_log2
        bits.f(120, 8); // base_q_idx
        bits.bit(0); // segmentation_enabled
        bits.bit(0); // using_qmatrix
        bits.bit(0); // delta_q_present
        bits.bit(0); // apply_deblocking_filter[0]
        bits.bit(0); // apply_deblocking_filter[1]
        bits.bit(0); // tx_mode_select
        bits.f(0, 2); // reduced_tx_set
        let data = bits.into_bytes();
        let core = assert_roundtrip(&data, ObuType::RegularSef, true, &seq);
        assert_eq!(core.frame_type, Some(FrameType::Key));
        assert_eq!(core.show_existing_frame, Some(false));
        assert_eq!(core.immediate_output_frame, Some(true));
        assert_eq!(core.sef_film_grain, None);
        assert_eq!(core.frame_to_show_map_idx, None);
    }

    #[test]
    fn reject_single_picture_bridge() {
        // The single-picture OBU_BRIDGE_FRAME parser bug is now FIXED
        // (frame-header-single-picture-bridge-fix): per spec §5.18.2 mirror :4971-5065 a
        // single-picture bridge is forced to a KEY intra frame but takes the `IsBridge`
        // early-return arm, so parse_single_picture_bridge_tail reads only the modeled prefix
        // (bridge_frame_overwrite_flag / KEY refresh / non-override frame_size / screen_content /
        // intrabc) and stops with InterStop::BruInactiveOrBridgeReturn — its core reaches
        // UnsupportedUntilFeature, NOT IntraHeaderComplete. The composer only writes
        // IntraHeaderComplete cores, so the status gate rejects it up front. (A hand-constructed
        // IntraHeaderComplete bridge core is still caught by the explicit bridge gate — see
        // reject_non_single_bridge_intra_model.)
        let mut seq = base_seq();
        seq.single_picture_header_flag = true;
        seq.filter.single_picture_header_flag = true;
        let mut bits = Bits::default();
        bits.uvlc(0); // seq_header_id_in_frame_header
        bits.f(5, 3); // bridge_frame_ref_idx = 5 f(CeilLog2(8) == 3) — read before single-pic
        bits.bit(0); // bridge_frame_overwrite_flag = 0 (mirror :4423)
        // refresh_frame_flags: overwrite == 0 -> inferred 1 << bridge_frame_ref_idx, no bits
        // (§ 6.17.2 + AVM). frame_size(): non-override default dims, no bits. screen_content: no bits.
        bits.bit(0); // allow_intrabc = 0 (intrabc_params(), mirror :4571) -> STOP at bridge return
        let data = bits.into_bytes();
        let core =
            parse_core_body_for_test(&data, ObuType::BridgeFrame, true, &seq, None).unwrap();
        assert!(matches!(
            core.status,
            FrameHeaderParseStatus::UnsupportedUntilFeature { .. }
        ));
        assert!(core.is_bridge);
        let mut writer = BitWriter::new();
        let err = write_frame_header_core(&mut writer, &core, &seq, None, true).unwrap_err();
        assert_eq!(err, WriteError::NonCanonicalFrameHeader { what: "status" });
        assert_eq!(writer.bit_len(), 0);
    }
}

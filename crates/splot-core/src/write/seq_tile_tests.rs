// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

// Unit, byte-exact, and rejection tests for the § 5.4.10 filter / § 5.4.2 tile writers and
// the composing `write_sequence_header`. `include!`d into `crate::write::seq_tile` so
// `super::*` resolves to its writers and private helpers. The property tests live in the
// sibling `seq_tile_proptests.rs` (split to keep each source under the line-budget).

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::too_many_lines)]
mod tests {
    use super::*;
    use crate::bitio::BitReader;
    use crate::headers::sequence::{
        LevelIdx, parse_sequence_filter_config, parse_sequence_header, parse_sequence_tile_config,
    };
    use crate::span::ByteOffset;
    use crate::tile::TileParamsInput;

    /// MSB-first bit builder mirroring the `Bits` helper in `headers::sequence`'s own
    /// tests, so this module reuses the same hand-built, spec-grounded fixtures.
    #[derive(Default)]
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

    fn tile_input(frame_width: u32, frame_height: u32) -> TileParamsInput {
        TileParamsInput {
            frame_width,
            frame_height,
            uniform_sb_size: SuperblockSize::Block64x64,
            sb_size: SuperblockSize::Block64x64,
            is_bridge: false,
            seq_tier: Tier::Main,
            seq_level_idx: LevelIdx::from_bits(0),
        }
    }

    // =========================================================================
    // Shared still-picture header fixture (mirrors the parser's own
    // `push_still_picture_header_until_tile`, then appends a chosen tile region).
    // =========================================================================

    /// Appends a still-picture `sequence_header_obu()` up to (but not including)
    /// `sequence_tile_config()`, field-for-field with the parser. `seq_level_idx` selects
    /// the level (single-picture headers never code `seq_tier`). 16x8 frame, BLOCK_64X64.
    fn push_still_picture_header_until_tile(bits: &mut Bits, seq_level_idx: u32) {
        // general (single_picture_header_flag = 1, chroma 4:2:0)
        bits.uvlc(0); // seq_header_id
        bits.f(0, 5); // seq_profile_idc
        bits.bit(1); // single_picture_header_flag
        bits.f(seq_level_idx, 5); // seq_level_idx (single picture -> no seq_tier)
        bits.uvlc(0); // chroma_format_idc = CHROMA_FORMAT_420
        bits.uvlc(0); // bit_depth_idc
        bits.f(3, 4); // frame_width_bits_minus_1
        bits.f(3, 4); // frame_height_bits_minus_1
        bits.f(15, 4); // max_frame_width_minus_1 -> 16
        bits.f(7, 4); // max_frame_height_minus_1 -> 8
        bits.bit(0); // seq_cropping_window_present_flag
        // sequence_partition_config (not monochrome, single picture)
        bits.bit(0); // use_256x256_superblock
        bits.bit(0); // use_128x128_superblock -> seqSbSize = BLOCK_64X64
        bits.bit(0); // enable_sdp
        bits.bit(0); // enable_ext_partitions
        bits.bit(0); // reduce_pb_aspect_ratio
        // sequence_segment_config
        bits.bit(0); // enable_ext_seg -> MaxSegments = 8
        bits.bit(0); // seq_seg_info_present_flag
        // sequence_intra_config (not monochrome)
        bits.bit(0); // enable_dip
        bits.bit(0); // enable_intra_edge_filter
        bits.bit(0); // enable_mrls
        bits.bit(0); // enable_cfl_intra
        bits.f(0, 2); // cfl_ds_filter_index
        bits.bit(0); // enable_mhccp
        bits.bit(0); // enable_ibp
        // sequence_inter_config (single_picture_header_flag branch)
        bits.bit(0); // enable_refmvbank
        bits.bit(1); // disable_drl_reorder -> DRL_REORDER_DISABLED
        bits.bit(0); // seq_max_bvp_drl_bits_minus_1 = ns(3) -> 0
        bits.bit(0); // allow_frame_max_bvp_drl_bits
        bits.bit(0); // enable_bawp
        // sequence_scc_config (single picture -> no signalled bits)
        // sequence_transform_quant_entropy_config (not monochrome, single picture)
        bits.bit(0); // enable_fsc
        bits.bit(0); // enable_idtx_intra
        bits.bit(0); // enable_intra_ist
        bits.bit(0); // enable_inter_ist
        bits.bit(0); // enable_chroma_dctonly
        bits.bit(0); // reduced_tx_part_set
        bits.bit(0); // enable_cctx
        bits.bit(0); // enable_tcq
        bits.bit(0); // enable_parity_hiding
        bits.bit(0); // separate_uv_delta_q
        bits.bit(1); // equal_ac_dc_q -> skip y/uv dc delta reads
        bits.f(0, 5); // base_uv_ac_delta_q
        bits.bit(0); // uv_ac_delta_q_enabled
        // sequence_filter_config (single picture, seqSbSize = BLOCK_64X64)
        bits.bit(0); // disable_loopfilters_across_tiles
        bits.bit(0); // enable_cdef
        bits.bit(0); // enable_gdf
        bits.bit(0); // enable_restoration
        bits.bit(0); // enable_ccso
        bits.f(0, 2); // df_par_bits_minus_2
    }

    fn parse_header(bytes: &[u8]) -> SequenceHeader {
        let mut reader = BitReader::new(bytes, ByteOffset::new(0));
        parse_sequence_header(&mut reader).unwrap()
    }

    fn write_header(header: &SequenceHeader) -> Vec<u8> {
        let mut writer = BitWriter::new();
        write_sequence_header(&mut writer, header).unwrap();
        writer.into_bytes()
    }

    /// Semantic round-trip: `parse(write(h)) == h`, plus byte-stability.
    fn assert_header_roundtrip(header: &SequenceHeader) {
        let bytes = write_header(header);
        let reparsed = parse_header(&bytes);
        assert_eq!(&reparsed, header, "parse(write(h)) != h");
        assert_eq!(write_header(&reparsed), bytes, "write not idempotent");
    }

    // =========================================================================
    // Byte-exact full-header round-trips on canonical fixtures
    // =========================================================================

    #[test]
    fn full_still_picture_header_no_tile_byte_exact() {
        // The whole payload + the trailing-pad pattern matches the parser's
        // `push_still_picture_header` fixture; the writer reproduces it byte-exact.
        let mut bits = Bits::default();
        push_still_picture_header_until_tile(&mut bits, 0);
        bits.bit(0); // seq_tile_info_present_flag (fully parsed, no tile bits)
        bits.bit(0); // film_grain_params_present
        let data = bits.into_bytes();
        let header = parse_header(&data);
        assert!(header.is_fully_parsed());
        let written = write_header(&header);
        assert_eq!(written, data, "no-tile header not byte-exact");
        assert_header_roundtrip(&header);
    }

    #[test]
    fn full_header_uniform_single_tile_byte_exact() {
        // 16x8 -> single uniform tile; only the uniform flag bit is signalled.
        let mut bits = Bits::default();
        push_still_picture_header_until_tile(&mut bits, 0);
        bits.bit(1); // seq_tile_info_present_flag
        bits.bit(0); // allow_tile_info_change
        bits.bit(1); // uniform_tile_spacing_flag (single tile -> no increments)
        bits.bit(0); // film_grain_params_present
        let data = bits.into_bytes();
        let header = parse_header(&data);
        assert!(header.is_fully_parsed());
        let tile = header.tile.as_ref().unwrap();
        assert!(tile.params.unwrap().uniform_spacing);
        assert_eq!(tile.seq_sb_col_starts, vec![0]);
        let written = write_header(&header);
        assert_eq!(written, data, "uniform single-tile header not byte-exact");
        assert_header_roundtrip(&header);
    }

    #[test]
    fn full_header_uniform_two_columns_byte_exact() {
        // 256x8 -> miCols 64, sbCols 4; tileColsLog2 incremented once (1,0), one row (0).
        let mut bits = Bits::default();
        push_still_picture_header_until_tile_wide(&mut bits);
        bits.bit(1); // seq_tile_info_present_flag
        bits.bit(1); // allow_tile_info_change
        bits.bit(1); // uniform_tile_spacing_flag
        bits.bit(1); // increment_tile_cols_log2 = 1
        bits.bit(0); // increment_tile_cols_log2 = 0 (stop)
        bits.bit(0); // increment_tile_rows_log2 = 0 (stop)
        bits.bit(0); // film_grain_params_present
        let data = bits.into_bytes();
        let header = parse_header(&data);
        assert!(header.is_fully_parsed());
        let tile = header.tile.as_ref().unwrap();
        let params = tile.params.unwrap();
        assert!(params.uniform_spacing);
        assert_eq!(params.tile_cols, 2);
        assert_eq!(tile.seq_sb_col_starts, vec![0, 2]);
        let written = write_header(&header);
        assert_eq!(written, data, "uniform two-column header not byte-exact");
        assert_header_roundtrip(&header);
    }

    #[test]
    fn full_header_non_uniform_two_columns_byte_exact() {
        // 128x8 -> sbCols 2, sbRows 1; non-uniform two 1-superblock columns.
        let mut bits = Bits::default();
        push_still_picture_header_until_tile_128wide(&mut bits);
        bits.bit(1); // seq_tile_info_present_flag
        bits.bit(0); // allow_tile_info_change
        bits.bit(0); // uniform_tile_spacing_flag = 0
        bits.bit(0); // ns(2) width_in_sbs_minus_1 = 0 -> first column 1 sb wide
        bits.bit(0); // film_grain_params_present
        let data = bits.into_bytes();
        let header = parse_header(&data);
        assert!(header.is_fully_parsed());
        let tile = header.tile.as_ref().unwrap();
        let params = tile.params.unwrap();
        assert!(!params.uniform_spacing);
        assert_eq!(params.tile_cols, 2);
        assert_eq!(tile.seq_sb_col_starts, vec![0, 1]);
        let written = write_header(&header);
        assert_eq!(written, data, "non-uniform two-column header not byte-exact");
        assert_header_roundtrip(&header);
    }

    /// 256-wide still-picture header (max_frame_width_minus_1 = 255 needs 8 width bits).
    fn push_still_picture_header_until_tile_wide(bits: &mut Bits) {
        push_still_picture_header_until_tile_dims(bits, 255, 7, 8, 4);
    }

    /// 128-wide still-picture header (max_frame_width_minus_1 = 127 needs 7 width bits).
    fn push_still_picture_header_until_tile_128wide(bits: &mut Bits) {
        push_still_picture_header_until_tile_dims(bits, 127, 7, 7, 4);
    }

    /// Generalized still-picture prefix with explicit frame dimensions. `w_minus_1` /
    /// `h_minus_1` are the coded `max_frame_*_minus_1`; `w_bits` / `h_bits` their field
    /// widths (`frame_*_bits`). 4:2:0, BLOCK_64X64, level 0.
    fn push_still_picture_header_until_tile_dims(
        bits: &mut Bits,
        w_minus_1: u32,
        h_minus_1: u32,
        w_bits: u32,
        h_bits: u32,
    ) {
        bits.uvlc(0); // seq_header_id
        bits.f(0, 5); // seq_profile_idc
        bits.bit(1); // single_picture_header_flag
        bits.f(0, 5); // seq_level_idx
        bits.uvlc(0); // chroma_format_idc
        bits.uvlc(0); // bit_depth_idc
        bits.f(w_bits - 1, 4); // frame_width_bits_minus_1
        bits.f(h_bits - 1, 4); // frame_height_bits_minus_1
        bits.f(w_minus_1, w_bits); // max_frame_width_minus_1
        bits.f(h_minus_1, h_bits); // max_frame_height_minus_1
        bits.bit(0); // seq_cropping_window_present_flag
        // partition
        bits.bit(0); // use_256x256_superblock
        bits.bit(0); // use_128x128_superblock
        bits.bit(0); // enable_sdp
        bits.bit(0); // enable_ext_partitions
        bits.bit(0); // reduce_pb_aspect_ratio
        // segment
        bits.bit(0); // enable_ext_seg
        bits.bit(0); // seq_seg_info_present_flag
        // intra
        bits.bit(0); // enable_dip
        bits.bit(0); // enable_intra_edge_filter
        bits.bit(0); // enable_mrls
        bits.bit(0); // enable_cfl_intra
        bits.f(0, 2); // cfl_ds_filter_index
        bits.bit(0); // enable_mhccp
        bits.bit(0); // enable_ibp
        // inter (single picture)
        bits.bit(0); // enable_refmvbank
        bits.bit(1); // disable_drl_reorder
        bits.bit(0); // seq_max_bvp_drl_bits_minus_1
        bits.bit(0); // allow_frame_max_bvp_drl_bits
        bits.bit(0); // enable_bawp
        // tq-entropy (single picture)
        bits.bit(0); // enable_fsc
        bits.bit(0); // enable_idtx_intra
        bits.bit(0); // enable_intra_ist
        bits.bit(0); // enable_inter_ist
        bits.bit(0); // enable_chroma_dctonly
        bits.bit(0); // reduced_tx_part_set
        bits.bit(0); // enable_cctx
        bits.bit(0); // enable_tcq
        bits.bit(0); // enable_parity_hiding
        bits.bit(0); // separate_uv_delta_q
        bits.bit(1); // equal_ac_dc_q
        bits.f(0, 5); // base_uv_ac_delta_q
        bits.bit(0); // uv_ac_delta_q_enabled
        // filter (single picture)
        bits.bit(0); // disable_loopfilters_across_tiles
        bits.bit(0); // enable_cdef
        bits.bit(0); // enable_gdf
        bits.bit(0); // enable_restoration
        bits.bit(0); // enable_ccso
        bits.f(0, 2); // df_par_bits_minus_2
    }

    // =========================================================================
    // Reserved-level residual -> UnwritableSequenceHeader
    // =========================================================================

    #[test]
    fn reserved_level_tile_residual_is_unwritable() {
        // seq_level_idx 22 is reserved: seq_tile_info_present bounds at tile_params, so the
        // header is unfully-parsed and the writer rejects it before any bit.
        let mut bits = Bits::default();
        push_still_picture_header_until_tile(&mut bits, 22);
        bits.bit(1); // seq_tile_info_present_flag -> reserved-level residual
        bits.bit(0); // allow_tile_info_change
        let data = bits.into_bytes();
        let header = parse_header(&data);
        assert!(!header.is_fully_parsed());
        assert_eq!(
            header.unimplemented_at,
            Some("AV2-5.4.2-SEQUENCE-TILE-CONFIG")
        );
        let mut writer = BitWriter::new();
        let err = write_sequence_header(&mut writer, &header).unwrap_err();
        assert_eq!(
            err,
            WriteError::UnwritableSequenceHeader {
                feature: "AV2-5.4.2-SEQUENCE-TILE-CONFIG"
            }
        );
        assert_eq!(writer.bit_len(), 0, "no bit written for an unwritable header");
    }

    // =========================================================================
    // write_sequence_header byte-alignment guard
    // =========================================================================

    #[test]
    fn write_sequence_header_rejects_unaligned_writer() {
        let mut bits = Bits::default();
        push_still_picture_header_until_tile(&mut bits, 0);
        bits.bit(0); // seq_tile_info_present_flag
        bits.bit(0); // film_grain_params_present
        let data = bits.into_bytes();
        let header = parse_header(&data);

        let mut writer = BitWriter::new();
        writer.write_bit(1).unwrap(); // push the writer off a byte boundary
        let err = write_sequence_header(&mut writer, &header).unwrap_err();
        assert_eq!(err, WriteError::WriterNotByteAligned);
    }

    // =========================================================================
    // Per-config filter round-trip + branch coverage
    // =========================================================================

    fn parse_filter(bytes: &[u8], single_picture: bool, sb: SuperblockSize) -> SequenceFilterConfig {
        let mut reader = BitReader::new(bytes, ByteOffset::new(0));
        parse_sequence_filter_config(&mut reader, single_picture, sb).unwrap()
    }

    fn write_filter(c: &SequenceFilterConfig, single_picture: bool, sb: SuperblockSize) -> Vec<u8> {
        let mut writer = BitWriter::new();
        write_sequence_filter_config(&mut writer, c, single_picture, sb).unwrap();
        writer.into_bytes()
    }

    fn assert_filter_roundtrip(c: &SequenceFilterConfig, single_picture: bool, sb: SuperblockSize) {
        let bytes = write_filter(c, single_picture, sb);
        let reparsed = parse_filter(&bytes, single_picture, sb);
        assert_eq!(&reparsed, c, "filter parse(write(c)) != c");
        assert_eq!(write_filter(&reparsed, single_picture, sb), bytes);
    }

    #[test]
    fn filter_all_branches_round_trip() {
        // Exercise: gdf on/off x sb size, restoration on (uv present / mirrored), ccso,
        // cdef enum (3 variants) x single_picture, df_par_bits.
        let mut bits = Bits::default();
        bits.bit(1); // disable_loopfilters_across_tiles
        bits.bit(1); // enable_cdef
        bits.bit(1); // enable_gdf
        bits.bit(1); // gdf_unit_matches_sb_size (gdf && 64x64)
        bits.bit(1); // enable_restoration
        bits.bit(1); // lr_pc_wiener_disabled
        bits.bit(0); // lr_wiener_nonsep_disabled
        bits.bit(1); // lr_tools_uv_present
        bits.bit(1); // lr_uv_wiener_nonsep_disabled (signalled)
        bits.bit(1); // enable_ccso
        bits.bit(1); // ccso_unit_matches_sb_size
        bits.bit(0); // cdef_on_skip_txfm_always_on = 0
        bits.bit(1); // cdef_on_skip_txfm_disabled = 1 -> Disabled
        bits.f(3, 2); // df_par_bits_minus_2
        let data = bits.into_bytes();
        let c = parse_filter(&data, false, SuperblockSize::Block64x64);
        assert_eq!(c.cdef_on_skip_txfm, CdefOnSkipTxfm::Disabled);
        assert!(c.gdf_unit_matches_sb_size);
        assert!(c.lr_tools_uv_present);
        assert_eq!(write_filter(&c, false, SuperblockSize::Block64x64), data);
        assert_filter_roundtrip(&c, false, SuperblockSize::Block64x64);
    }

    #[test]
    fn filter_restoration_mirrored_uv_round_trips() {
        // restoration on, lr_tools_uv_present = 0 -> lr_uv_wiener_nonsep_disabled mirrors
        // lr_wiener_nonsep_disabled (no bit).
        let mut bits = Bits::default();
        bits.bit(0); // disable_loopfilters_across_tiles
        bits.bit(0); // enable_cdef
        bits.bit(0); // enable_gdf
        bits.bit(1); // enable_restoration
        bits.bit(0); // lr_pc_wiener_disabled
        bits.bit(1); // lr_wiener_nonsep_disabled
        bits.bit(0); // lr_tools_uv_present -> mirrored
        bits.bit(0); // enable_ccso
        bits.bit(1); // cdef_on_skip_txfm_always_on -> AlwaysOn
        bits.f(0, 2); // df_par_bits_minus_2
        let data = bits.into_bytes();
        let c = parse_filter(&data, false, SuperblockSize::Block64x64);
        assert!(!c.lr_tools_uv_present);
        assert_eq!(c.lr_uv_wiener_nonsep_disabled, c.lr_wiener_nonsep_disabled);
        assert_eq!(c.cdef_on_skip_txfm, CdefOnSkipTxfm::AlwaysOn);
        assert_eq!(write_filter(&c, false, SuperblockSize::Block64x64), data);
        assert_filter_roundtrip(&c, false, SuperblockSize::Block64x64);
    }

    #[test]
    fn filter_single_picture_infers_adaptive_cdef() {
        // single picture -> CdefOnSkipTxfm inferred Adaptive, no cdef bits.
        let mut bits = Bits::default();
        bits.bit(0); // disable_loopfilters_across_tiles
        bits.bit(0); // enable_cdef
        bits.bit(0); // enable_gdf
        bits.bit(0); // enable_restoration
        bits.bit(0); // enable_ccso
        bits.f(2, 2); // df_par_bits_minus_2
        let data = bits.into_bytes();
        let c = parse_filter(&data, true, SuperblockSize::Block64x64);
        assert_eq!(c.cdef_on_skip_txfm, CdefOnSkipTxfm::Adaptive);
        assert_eq!(write_filter(&c, true, SuperblockSize::Block64x64), data);
        assert_filter_roundtrip(&c, true, SuperblockSize::Block64x64);
    }

    #[test]
    fn filter_gdf_gate_off_when_not_64x64_round_trips() {
        // enable_gdf with a 128x128 seqSbSize -> gdf_unit_matches_sb_size not signalled.
        let mut bits = Bits::default();
        bits.bit(0); // disable_loopfilters_across_tiles
        bits.bit(0); // enable_cdef
        bits.bit(1); // enable_gdf (but sb != 64x64 -> no gdf_unit bit)
        bits.bit(0); // enable_restoration
        bits.bit(0); // enable_ccso
        bits.bit(0); // cdef_on_skip_txfm_always_on = 0
        bits.bit(0); // cdef_on_skip_txfm_disabled = 0 -> Adaptive
        bits.f(0, 2); // df_par_bits_minus_2
        let data = bits.into_bytes();
        let c = parse_filter(&data, false, SuperblockSize::Block128x128);
        assert!(!c.gdf_unit_matches_sb_size);
        assert_eq!(write_filter(&c, false, SuperblockSize::Block128x128), data);
        assert_filter_roundtrip(&c, false, SuperblockSize::Block128x128);
    }

    // =========================================================================
    // Per-config tile round-trip
    // =========================================================================

    fn parse_tile(bytes: &[u8], input: TileParamsInput) -> SequenceTileConfig {
        let mut reader = BitReader::new(bytes, ByteOffset::new(0));
        parse_sequence_tile_config(&mut reader, input).unwrap()
    }

    fn write_tile(c: &SequenceTileConfig, input: TileParamsInput) -> Vec<u8> {
        let mut writer = BitWriter::new();
        write_sequence_tile_config(&mut writer, c, input).unwrap();
        writer.into_bytes()
    }

    fn assert_tile_roundtrip(c: &SequenceTileConfig, input: TileParamsInput) {
        let bytes = write_tile(c, input);
        let reparsed = parse_tile(&bytes, input);
        assert_eq!(&reparsed, c, "tile parse(write(c)) != c");
        assert_eq!(write_tile(&reparsed, input), bytes);
    }

    #[test]
    fn tile_absent_round_trips() {
        let mut bits = Bits::default();
        bits.bit(0); // seq_tile_info_present_flag = 0
        let data = bits.into_bytes();
        let c = parse_tile(&data, tile_input(16, 8));
        assert!(!c.seq_tile_info_present_flag);
        assert_eq!(write_tile(&c, tile_input(16, 8)), data);
        assert_tile_roundtrip(&c, tile_input(16, 8));
    }

    #[test]
    fn tile_uniform_single_byte_exact() {
        let mut bits = Bits::default();
        bits.bit(1); // seq_tile_info_present_flag
        bits.bit(0); // allow_tile_info_change
        bits.bit(1); // uniform_tile_spacing_flag
        let data = bits.into_bytes();
        let c = parse_tile(&data, tile_input(16, 8));
        assert_eq!(write_tile(&c, tile_input(16, 8)), data);
        assert_tile_roundtrip(&c, tile_input(16, 8));
    }

    #[test]
    fn tile_uniform_increment_run_hits_max_byte_exact() {
        // 512x8 -> miCols 128, sbCols 8; maxLog2TileCols = tile_log2(1, 8) = 3. Driving the
        // column increment to its maximum (three 1 bits, NO terminating 0 because the loop
        // exits at the while condition) -> 8 single-superblock columns.
        let input = tile_input(512, 8);
        let mut bits = Bits::default();
        bits.bit(1); // seq_tile_info_present_flag
        bits.bit(0); // allow_tile_info_change
        bits.bit(1); // uniform_tile_spacing_flag
        bits.bit(1); // increment_tile_cols_log2 = 1
        bits.bit(1); // increment_tile_cols_log2 = 1
        bits.bit(1); // increment_tile_cols_log2 = 1 (now tileColsLog2 == maxLog2TileCols -> no stop bit)
        // single row (sbRows 1 -> maxLog2TileRows 0 -> no row bits)
        let data = bits.into_bytes();
        let c = parse_tile(&data, input);
        let params = c.params.unwrap();
        assert!(params.uniform_spacing);
        assert_eq!(params.tile_cols, 8);
        assert_eq!(c.seq_sb_col_starts, vec![0, 1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(
            write_tile(&c, input),
            data,
            "increment-run-hits-max not byte-exact (a spurious terminating 0 would shift)"
        );
        assert_tile_roundtrip(&c, input);
    }

    #[test]
    fn tile_non_uniform_wide_first_column_byte_exact() {
        // 256x8 -> sbCols 4. Non-uniform multi-column layout: a 2-wide first column then
        // two 1-wide columns (starts [0, 2, 3]), exercising a width_in_sbs_minus_1 > 0 and
        // a width recovered from a delta that is not the last-tile edge case.
        let input = tile_input(256, 8);
        let mut bits = Bits::default();
        bits.bit(1); // seq_tile_info_present_flag
        bits.bit(1); // allow_tile_info_change
        bits.bit(0); // uniform_tile_spacing_flag = 0
        // ns(4) of width_in_sbs_minus_1 = 1 (size 2): n=4 -> w=3, m=4; 1 < 4 -> w-1=2 bits = 0b001.
        bits.bit(0);
        bits.bit(1);
        // remaining columns: ns(2) then ns(1); first reads 1 bit = 0 -> size 1.
        bits.bit(0);
        let data = bits.into_bytes();
        let c = parse_tile(&data, input);
        let params = c.params.unwrap();
        assert!(!params.uniform_spacing);
        assert_eq!(c.seq_sb_col_starts, vec![0, 2, 3]);
        assert_eq!(params.tile_cols, 3);
        assert_eq!(
            write_tile(&c, input),
            data,
            "non-uniform wide first column not byte-exact"
        );
        assert_tile_roundtrip(&c, input);
    }

    #[test]
    fn tile_non_uniform_two_columns_byte_exact() {
        let input = tile_input(128, 8);
        let mut bits = Bits::default();
        bits.bit(1); // seq_tile_info_present_flag
        bits.bit(1); // allow_tile_info_change
        bits.bit(0); // uniform_tile_spacing_flag = 0
        bits.bit(0); // ns(2) width_in_sbs_minus_1 = 0
        let data = bits.into_bytes();
        let c = parse_tile(&data, input);
        assert_eq!(c.seq_sb_col_starts, vec![0, 1]);
        assert_eq!(write_tile(&c, input), data);
        assert_tile_roundtrip(&c, input);
    }

    #[test]
    fn tile_uniform_two_rows_byte_exact() {
        // 8x256 frame -> sbRows 4; one column, two rows via the row increment run.
        let input = TileParamsInput {
            frame_width: 16,
            frame_height: 256,
            ..tile_input(16, 256)
        };
        let mut bits = Bits::default();
        bits.bit(1); // seq_tile_info_present_flag
        bits.bit(0); // allow_tile_info_change
        bits.bit(1); // uniform_tile_spacing_flag
        // single column (sbCols 1 -> minLog2TileCols == maxLog2TileCols == 0, no col bits)
        bits.bit(1); // increment_tile_rows_log2 = 1
        bits.bit(0); // increment_tile_rows_log2 = 0 (stop)
        let data = bits.into_bytes();
        let c = parse_tile(&data, input);
        let params = c.params.unwrap();
        assert!(params.uniform_spacing);
        assert_eq!(params.tile_rows, 2);
        assert_eq!(c.seq_sb_row_starts, vec![0, 2]);
        assert_eq!(write_tile(&c, input), data);
        assert_tile_roundtrip(&c, input);
    }

    /// Drift guard for the locally-duplicated `Tile_Width_Scaling_Factor` /
    /// `Tile_Area_Scaling_Factor` tables (`seq_tile.rs`). The tables are copied from the
    /// (private) parser copies in `crate::tile`; this test makes a single-entry typo in
    /// either local copy a guaranteed round-trip failure, so the duplication cannot drift
    /// silently.
    ///
    /// The frame is sized so BOTH tables are *load-bearing* at every `(tier, level)`: at a
    /// 32768x32768 frame with 64x64 superblocks, `sbCols == sbRows == 512`, so
    /// `minLog2TileCols = tile_log2(width_sf*16, 512) >= 1` (width-table-driven) and
    /// `minLog2Tiles = tile_log2(area_sf*576, 512*512)` strictly exceeds it
    /// (area-table-driven), making `minLog2TileRows` area-driven too. A uniform config that
    /// sits at the minimum codes its column/row counts straight from those minimums, so a
    /// wrong table entry shifts the re-emitted increment run and the round-trip diverges.
    /// (Contrast the `tile_round_trips` proptest, whose <=512 frame sizes keep
    /// `minLog2TileCols == minLog2Tiles == 0` for every level, so it never exercises the
    /// tables — see the §5.4.2 review thread.) The `tile_cols >= 2` / `tile_rows >= 2`
    /// asserts fail loudly if a future change shrinks the frame and makes this guard vacuous.
    #[test]
    fn scaling_tables_drive_layout_across_all_levels() {
        for tier in [Tier::Main, Tier::High] {
            for level in 0u8..=21 {
                let input = TileParamsInput {
                    frame_width: 32768,
                    frame_height: 32768,
                    uniform_sb_size: SuperblockSize::Block64x64,
                    sb_size: SuperblockSize::Block64x64,
                    is_bridge: false,
                    seq_tier: tier,
                    seq_level_idx: LevelIdx::from_bits(level),
                };
                let mut bits = Bits::default();
                bits.bit(1); // seq_tile_info_present_flag
                bits.bit(0); // allow_tile_info_change
                bits.bit(1); // uniform_tile_spacing_flag
                bits.bit(0); // col increment stop -> tileColsLog2 == minLog2TileCols (width table)
                bits.bit(0); // row increment stop -> tileRowsLog2 == minLog2TileRows (area table)
                bits.f(0, 11); // padding so the parser never peeks past EOF
                let data = bits.into_bytes();

                let c = parse_tile(&data, input);
                let params = c.params.expect("uniform tile params");
                assert!(params.uniform_spacing);
                assert!(
                    params.tile_cols >= 2,
                    "{tier:?} L{level}: width table not load-bearing (tile_cols={})",
                    params.tile_cols
                );
                assert!(
                    params.tile_rows >= 2,
                    "{tier:?} L{level}: area table not load-bearing (tile_rows={})",
                    params.tile_rows
                );
                assert_tile_roundtrip(&c, input);
            }
        }
    }

    // =========================================================================
    // Rejection / gated-off / domain tests — each asserts bit_len() == 0
    // =========================================================================

    fn full_no_tile_header() -> SequenceHeader {
        let mut bits = Bits::default();
        push_still_picture_header_until_tile(&mut bits, 0);
        bits.bit(0); // seq_tile_info_present_flag
        bits.bit(0); // film_grain_params_present
        parse_header(&bits.into_bytes())
    }

    fn assert_filter_rejected(mutate: impl FnOnce(&mut SequenceFilterConfig), what: &str) {
        let mut header = full_no_tile_header();
        let mut filter = header.filter.unwrap();
        mutate(&mut filter);
        header.filter = Some(filter);
        let mut writer = BitWriter::new();
        let err = write_sequence_header(&mut writer, &header).unwrap_err();
        assert!(
            matches!(err, WriteError::NonCanonicalSequenceValue { what: w } if w == what),
            "expected NonCanonicalSequenceValue {{ what: {what} }}, got {err:?}"
        );
        assert_eq!(writer.bit_len(), 0, "{what}: bits written on reject");
    }

    #[test]
    fn filter_reject_gdf_unit_without_gate() {
        // single-picture full header -> seqSbSize 64x64; enable_gdf is false, so the gate
        // (enable_gdf && 64x64) is off and a set gdf_unit_matches_sb_size is non-canonical.
        assert_filter_rejected(|f| f.gdf_unit_matches_sb_size = true, "gdf_unit_matches_sb_size");
    }

    #[test]
    fn filter_reject_lr_uv_pc_wiener_mismatch() {
        // restoration is false in the fixture, so lr_uv_pc_wiener_disabled must equal it.
        assert_filter_rejected(|f| f.lr_uv_pc_wiener_disabled = true, "lr_uv_pc_wiener_disabled");
    }

    #[test]
    fn filter_reject_restoration_subfield_without_gate() {
        // restoration off -> lr_pc_wiener_disabled must stay false.
        assert_filter_rejected(|f| f.lr_pc_wiener_disabled = true, "restoration_subfields");
    }

    #[test]
    fn filter_reject_ccso_unit_without_gate() {
        assert_filter_rejected(|f| f.ccso_unit_matches_sb_size = true, "ccso_unit_matches_sb_size");
    }

    #[test]
    fn filter_reject_non_adaptive_cdef_single_picture() {
        // The fixture is single-picture, so cdef_on_skip_txfm must be Adaptive.
        assert_filter_rejected(|f| f.cdef_on_skip_txfm = CdefOnSkipTxfm::AlwaysOn, "cdef_on_skip_txfm");
    }

    #[test]
    fn filter_reject_df_par_bits_too_wide() {
        // df_par_bits_minus_2 must fit f(2).
        let mut header = full_no_tile_header();
        let mut filter = header.filter.unwrap();
        filter.df_par_bits_minus_2 = 4; // > 3 doesn't fit f(2)
        header.filter = Some(filter);
        let mut writer = BitWriter::new();
        let err = write_sequence_header(&mut writer, &header).unwrap_err();
        assert_eq!(
            err,
            WriteError::ValueTooWide {
                value: 4,
                width_bits: 2
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn tile_reject_present_flag_with_payload() {
        // seq_tile_info_present_flag false but params present -> non-canonical.
        let input = tile_input(16, 8);
        let mut bits = Bits::default();
        bits.bit(1);
        bits.bit(0);
        bits.bit(1);
        let c = parse_tile(&bits.into_bytes(), input);
        // Build an inconsistent config: flag false but keep the params.
        let bad = SequenceTileConfig {
            seq_tile_info_present_flag: false,
            ..c
        };
        let mut writer = BitWriter::new();
        let err = write_sequence_tile_config(&mut writer, &bad, input).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalSequenceValue {
                what: "seq_tile_info_present_flag"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn tile_reject_is_bridge_input() {
        // is_bridge is a §5.18.7.4 frame concept; the §5.4.2 sequence config never sets it.
        // With is_bridge the parser reads zero layout bits (uniform inferred), so this
        // sequence writer rejects it before any bit rather than emit a flag the parse skips.
        let mut bits = Bits::default();
        bits.bit(1); // seq_tile_info_present_flag
        bits.bit(0); // allow_tile_info_change
        bits.bit(1); // uniform_tile_spacing_flag
        let c = parse_tile(&bits.into_bytes(), tile_input(16, 8));
        let bridge_input = TileParamsInput {
            is_bridge: true,
            ..tile_input(16, 8)
        };
        let mut writer = BitWriter::new();
        let err = write_sequence_tile_config(&mut writer, &c, bridge_input).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalSequenceValue { what: "is_bridge" }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn tile_reject_reserved_level_params_none() {
        // present flag set, params None -> UnwritableSequenceHeader at the tile-config level.
        let input = tile_input(16, 8);
        let bad = SequenceTileConfig {
            seq_tile_info_present_flag: true,
            allow_tile_info_change: Some(false),
            params: None,
            seq_sb_col_starts: Vec::new(),
            seq_sb_row_starts: Vec::new(),
        };
        let mut writer = BitWriter::new();
        let err = write_sequence_tile_config(&mut writer, &bad, input).unwrap_err();
        assert_eq!(
            err,
            WriteError::UnwritableSequenceHeader {
                feature: "AV2-5.4.2-SEQUENCE-TILE-CONFIG"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn tile_reject_grid_mismatch() {
        // Parse a 16x8 single-tile config, then write it against a 128x8 input: the stored
        // sb grid no longer matches the recomputed grid.
        let mut bits = Bits::default();
        bits.bit(1);
        bits.bit(0);
        bits.bit(1);
        let c = parse_tile(&bits.into_bytes(), tile_input(16, 8));
        let mut writer = BitWriter::new();
        let err = write_sequence_tile_config(&mut writer, &c, tile_input(128, 8)).unwrap_err();
        assert!(matches!(
            err,
            WriteError::NonCanonicalSequenceValue { .. }
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn tile_reject_corrupt_start_array() {
        // A non-monotonic start array could never have been produced by the parser.
        let input = tile_input(128, 8);
        let mut bits = Bits::default();
        bits.bit(1);
        bits.bit(1);
        bits.bit(0); // non-uniform
        bits.bit(0); // ns(2) -> 0
        let mut c = parse_tile(&bits.into_bytes(), input);
        c.seq_sb_col_starts = vec![0, 0]; // duplicate start -> size 0 tile, invalid
        let mut writer = BitWriter::new();
        let err = write_sequence_tile_config(&mut writer, &c, input).unwrap_err();
        assert!(matches!(
            err,
            WriteError::NonCanonicalSequenceValue {
                what: "seq_sb_col_starts"
            }
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn sequence_header_reject_missing_child() {
        let mut header = full_no_tile_header();
        header.filter = None;
        let mut writer = BitWriter::new();
        let err = write_sequence_header(&mut writer, &header).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalSequenceValue { what: "filter" }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn sequence_header_reject_missing_film_grain() {
        let mut header = full_no_tile_header();
        header.film_grain_params_present = None;
        let mut writer = BitWriter::new();
        let err = write_sequence_header(&mut writer, &header).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalSequenceValue {
                what: "film_grain_params_present"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    // =========================================================================
    // Never-panics: writing arbitrary mutated configs and parser-reachable models.
    // =========================================================================

    #[test]
    fn writer_never_panics_on_truncated_fixtures() {
        // Parse the full header at every byte prefix where it parses, then write it; the
        // writer must never panic (it may reject).
        let mut bits = Bits::default();
        push_still_picture_header_until_tile(&mut bits, 0);
        bits.bit(0);
        bits.bit(0);
        let full = bits.into_bytes();
        let mut reader = BitReader::new(&full, ByteOffset::new(0));
        if let Ok(header) = parse_sequence_header(&mut reader) {
            let mut writer = BitWriter::new();
            let _ = write_sequence_header(&mut writer, &header);
        }
    }
}

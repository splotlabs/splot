// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

// Unit, byte-exact, and rejection tests for the § 5.18.5.2 / § 5.18.7.9 / § 5.18.7.10 frame
// loop-filter writers.

// `include!`d into `crate::write::frame_filters` so `super::*` resolves to its writers and
// private helpers (the property tests live in the sibling `frame_filters_proptests.rs`).

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::too_many_lines)]
mod tests {
    use super::*;
    use crate::bitio::BitReader;
    use crate::headers::frame::{
        CdefStrengthSet, parse_cdef_params, parse_deblocking_filter_params, parse_gdf_params,
    };
    use crate::headers::sequence::SuperblockSize;
    use crate::span::ByteOffset;
    use crate::test_support::base_geometry;

    use crate::test_bits::Bits;

    fn reader(bytes: &[u8]) -> BitReader<'_> {
        BitReader::new(bytes, ByteOffset::new(0))
    }

    fn base_filter() -> CoreSeqFilterView {
        CoreSeqFilterView {
            enable_cdef: true,
            enable_gdf: true,
            gdf_unit_matches_sb_size: false,
            disable_loopfilters_across_tiles: false,
            cdef_on_skip_txfm: CdefOnSkipTxfm::Adaptive,
            df_par_bits_minus_2: 0,
            enable_df_sub_pu: false,
            single_picture_header_flag: false,
        }
    }

    // ===== deblocking (§ 5.18.5.2) =====

    /// Parse the hand-built bits, write the parsed model back, assert byte-exact round-trip
    /// against the parser-consumed bit length, then reparse the written bytes to the same
    /// model.
    fn roundtrip_deblocking(
        bits: Bits,
        coded_lossless: bool,
        num_planes: u8,
        df_par_bits_minus_2: u8,
        mfh: Option<&MfhDeblockingView>,
    ) {
        let data = bits.into_bytes();
        let mut rd = reader(&data);
        // The writer inverts the intra deblocking arm (FrameType != INTER_FRAME), so the
        // round-trip drives parse with `read_allow_df_sub_pu == false` (no inter bit).
        let params = parse_deblocking_filter_params(
            &mut rd,
            coded_lossless,
            num_planes,
            df_par_bits_minus_2,
            false,
            mfh,
        )
        .unwrap();
        let consumed = rd.consumed_bits();
        let mut writer = BitWriter::new();
        write_deblocking_filter_params(
            &mut writer,
            &params,
            coded_lossless,
            num_planes,
            df_par_bits_minus_2,
            mfh,
        )
        .unwrap();
        assert_eq!(writer.bit_len(), consumed, "bit length matches parser");
        let bytes = writer.into_bytes();
        let reparsed = parse_deblocking_filter_params(
            &mut reader(&bytes),
            coded_lossless,
            num_planes,
            df_par_bits_minus_2,
            false,
            mfh,
        )
        .unwrap();
        assert_eq!(reparsed, params);
    }

    #[test]
    fn deblocking_coded_lossless_round_trips() {
        roundtrip_deblocking(Bits::default(), true, 3, 0, None);
    }

    #[test]
    fn deblocking_direct_full_chroma_round_trips() {
        let mut bits = Bits::default();
        bits.bit(1); // apply[0]
        bits.bit(0); // apply[1]
        bits.bit(1); // apply[2]
        bits.bit(0); // apply[3]
        bits.bit(1); // df_delta_q_present[0]
        bits.f(3, 2); // df_delta_q[0] f(2) == 3 -> 1
        bits.bit(1); // df_delta_q_present[2]
        bits.f(0, 2); // df_delta_q[2] == 0 -> -2
        roundtrip_deblocking(bits, false, 3, 0, None);
    }

    #[test]
    fn deblocking_index_one_inherits_round_trips() {
        let mut bits = Bits::default();
        bits.bit(1); // apply[0]
        bits.bit(1); // apply[1]
        bits.bit(1); // df_delta_q_present[0]
        bits.f(3, 2); // df_delta_q[0] -> 1
        bits.bit(0); // df_delta_q_present[1] -> inherits DfDeltaQ[0]
        roundtrip_deblocking(bits, false, 1, 0, None);
    }

    #[test]
    fn deblocking_monochrome_round_trips() {
        let mut bits = Bits::default();
        bits.bit(1); // apply[0]
        bits.bit(1); // apply[1]
        bits.bit(0); // df_delta_q_present[0]
        bits.bit(0); // df_delta_q_present[1]
        roundtrip_deblocking(bits, false, 1, 0, None);
    }

    #[test]
    fn deblocking_wide_df_par_bits_round_trips() {
        let mut bits = Bits::default();
        bits.bit(1); // apply[0]
        bits.bit(0); // apply[1]
        bits.bit(1); // df_delta_q_present[0]
        bits.f(20, 5); // df_delta_q[0] f(5) == 20 -> 4
        roundtrip_deblocking(bits, false, 1, 3, None);
    }

    #[test]
    fn deblocking_mfh_update_round_trips() {
        let mfh = MfhDeblockingView {
            mfh_deblocking_filter_update: true,
            mfh_apply_deblocking_filter: [true, false, true, true],
        };
        let mut bits = Bits::default();
        bits.bit(0); // df_delta_q_present[0]
        bits.bit(0); // df_delta_q_present[2]
        bits.bit(0); // df_delta_q_present[3]
        roundtrip_deblocking(bits, false, 3, 0, Some(&mfh));
    }

    #[test]
    fn deblocking_mfh_update_zero_reads_apply_round_trips() {
        let mfh = MfhDeblockingView {
            mfh_deblocking_filter_update: false,
            mfh_apply_deblocking_filter: [true, true, true, true],
        };
        let mut bits = Bits::default();
        bits.bit(0); // apply[0]
        bits.bit(0); // apply[1]
        roundtrip_deblocking(bits, false, 3, 0, Some(&mfh));
    }

    #[test]
    fn deblocking_coded_lossless_nonzero_rejected() {
        let params = DeblockingFilterParams {
            apply_deblocking_filter: [true, false, false, false],
            df_delta_q_present: [false; 4],
            df_delta_q: [0; 4],
        };
        let mut writer = BitWriter::new();
        let err =
            write_deblocking_filter_params(&mut writer, &params, true, 3, 0, None).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "deblocking_coded_lossless"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn deblocking_oversized_df_par_bits_rejected() {
        let params = DeblockingFilterParams {
            apply_deblocking_filter: [false; 4],
            df_delta_q_present: [false; 4],
            df_delta_q: [0; 4],
        };
        let mut writer = BitWriter::new();
        // df_par_bits_minus_2 == 31 -> dfParBits = 33 > 32.
        let err =
            write_deblocking_filter_params(&mut writer, &params, false, 1, 31, None).unwrap_err();
        assert_eq!(
            err,
            WriteError::BitWidthTooLarge {
                requested: 33,
                max: 32
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn deblocking_coded_lossless_ignores_oversized_df_par_bits() {
        // Regression (#4f adversarial review): on the coded_lossless path the parser returns
        // the all-default structure before deriving dfParBits, so it never raises
        // BitWidthTooLarge there. The writer must match — an out-of-range df_par_bits_minus_2
        // (a non-conformant § 5.4.10 field) must NOT reject a model the parser accepted; it
        // round-trips with zero bits.
        for df_par_bits_minus_2 in [31u8, 255] {
            roundtrip_deblocking(Bits::default(), true, 1, df_par_bits_minus_2, None);
        }
    }

    #[test]
    fn deblocking_mfh_apply_mismatch_rejected() {
        let mfh = MfhDeblockingView {
            mfh_deblocking_filter_update: true,
            mfh_apply_deblocking_filter: [true, false, false, false],
        };
        // Derived array is [true, false, false, false]; a model with apply[1] set disagrees.
        let params = DeblockingFilterParams {
            apply_deblocking_filter: [true, true, false, false],
            df_delta_q_present: [false; 4],
            df_delta_q: [0; 4],
        };
        let mut writer = BitWriter::new();
        let err =
            write_deblocking_filter_params(&mut writer, &params, false, 3, 0, Some(&mfh)).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "apply_deblocking_filter"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn deblocking_uncoded_chroma_apply_rejected() {
        // Direct arm, monochrome (num_planes 1): apply[2] has no bitstream home.
        let params = DeblockingFilterParams {
            apply_deblocking_filter: [true, false, true, false],
            df_delta_q_present: [false; 4],
            df_delta_q: [0; 4],
        };
        let mut writer = BitWriter::new();
        let err =
            write_deblocking_filter_params(&mut writer, &params, false, 1, 0, None).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "apply_deblocking_filter_chroma"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn deblocking_gated_off_delta_q_rejected() {
        // apply[0] false -> df_delta_q_present[0]/df_delta_q[0] must be default.
        let params = DeblockingFilterParams {
            apply_deblocking_filter: [false; 4],
            df_delta_q_present: [true, false, false, false],
            df_delta_q: [0; 4],
        };
        let mut writer = BitWriter::new();
        let err =
            write_deblocking_filter_params(&mut writer, &params, false, 1, 0, None).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader { what: "df_delta_q" }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn deblocking_inferred_index_one_mismatch_rejected() {
        // apply[1] set, absent present[1] -> DfDeltaQ[1] must equal DfDeltaQ[0]; mismatch bad.
        let params = DeblockingFilterParams {
            apply_deblocking_filter: [true, true, false, false],
            df_delta_q_present: [true, false, false, false],
            df_delta_q: [1, 2, 0, 0], // [1] should be 1 (== [0])
        };
        let mut writer = BitWriter::new();
        let err =
            write_deblocking_filter_params(&mut writer, &params, false, 1, 0, None).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader { what: "df_delta_q" }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn deblocking_coded_delta_q_out_of_domain_rejected() {
        // dfParBits = 2 -> raw = DfDeltaQ + 2 must be in 0..=3; DfDeltaQ = 5 -> raw 7 > 3.
        let params = DeblockingFilterParams {
            apply_deblocking_filter: [true, false, false, false],
            df_delta_q_present: [true, false, false, false],
            df_delta_q: [5, 0, 0, 0],
        };
        let mut writer = BitWriter::new();
        let err =
            write_deblocking_filter_params(&mut writer, &params, false, 1, 0, None).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader { what: "df_delta_q" }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    // ===== gdf (§ 5.18.7.9) =====

    fn roundtrip_gdf(bits: Bits, coded_lossless: bool, filter: CoreSeqFilterView, geometry: GdfGeometry<'_>) {
        let data = bits.into_bytes();
        let mut rd = reader(&data);
        let params = parse_gdf_params(&mut rd, coded_lossless, &filter, geometry).unwrap();
        let consumed = rd.consumed_bits();
        let mut writer = BitWriter::new();
        write_gdf_params(&mut writer, &params, coded_lossless, &filter, geometry).unwrap();
        assert_eq!(writer.bit_len(), consumed, "bit length matches parser");
        let bytes = writer.into_bytes();
        let reparsed = parse_gdf_params(&mut reader(&bytes), coded_lossless, &filter, geometry).unwrap();
        assert_eq!(reparsed, params);
    }

    #[test]
    fn gdf_coded_lossless_round_trips() {
        roundtrip_gdf(Bits::default(), true, base_filter(), base_geometry());
    }

    #[test]
    fn gdf_disabled_seq_round_trips() {
        let mut filter = base_filter();
        filter.enable_gdf = false;
        roundtrip_gdf(Bits::default(), false, filter, base_geometry());
    }

    #[test]
    fn gdf_single_picture_per_block_coded_round_trips() {
        let mut filter = base_filter();
        filter.single_picture_header_flag = true;
        let mut bits = Bits::default();
        bits.bit(1); // gdf_per_block (coded: frame exceeds block)
        bits.f(2, 2); // gdf_pic_qc_idx
        bits.f(1, 2); // gdf_pic_scale_idx
        roundtrip_gdf(bits, false, filter, base_geometry());
    }

    #[test]
    fn gdf_frame_disabled_round_trips() {
        let mut bits = Bits::default();
        bits.bit(0); // gdf_frame_enable = 0
        roundtrip_gdf(bits, false, base_filter(), base_geometry());
    }

    #[test]
    fn gdf_per_block_inferred_round_trips() {
        // A small single-tile frame at gdfBlkSize -> gdf_per_block inferred 0, not coded.
        let geom = GdfGeometry {
            sb_size: SuperblockSize::Block128x128,
            mi_cols: 32, // 32*4 == 128 == gdfBlkSize, not exceeded
            mi_rows: 32,
            tile_cols: 1,
            tile_rows: 1,
            mi_col_starts: &[0],
            mi_row_starts: &[0],
        };
        let mut bits = Bits::default();
        bits.bit(1); // gdf_frame_enable (read)
        bits.f(0, 2); // gdf_pic_qc_idx
        bits.f(0, 2); // gdf_pic_scale_idx
        roundtrip_gdf(bits, false, base_filter(), geom);
    }

    #[test]
    fn gdf_disabled_nonzero_model_rejected() {
        let params = GdfParams {
            gdf_frame_enable: true,
            gdf_per_block: None,
            gdf_pic_qc_idx: None,
            gdf_pic_scale_idx: None,
        };
        let mut writer = BitWriter::new();
        let err = write_gdf_params(&mut writer, &params, true, &base_filter(), base_geometry())
            .unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "gdf_disabled"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn gdf_single_picture_disabled_frame_rejected() {
        let mut filter = base_filter();
        filter.single_picture_header_flag = true;
        let params = GdfParams {
            gdf_frame_enable: false,
            gdf_per_block: None,
            gdf_pic_qc_idx: None,
            gdf_pic_scale_idx: None,
        };
        let mut writer = BitWriter::new();
        let err =
            write_gdf_params(&mut writer, &params, false, &filter, base_geometry()).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "gdf_frame_enable"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn gdf_frame_disabled_with_some_option_rejected() {
        let params = GdfParams {
            gdf_frame_enable: false,
            gdf_per_block: Some(false),
            gdf_pic_qc_idx: None,
            gdf_pic_scale_idx: None,
        };
        let mut writer = BitWriter::new();
        let err = write_gdf_params(&mut writer, &params, false, &base_filter(), base_geometry())
            .unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "gdf_frame_disabled"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn gdf_enabled_with_none_option_rejected() {
        let params = GdfParams {
            gdf_frame_enable: true,
            gdf_per_block: Some(true),
            gdf_pic_qc_idx: None, // should be Some when enabled
            gdf_pic_scale_idx: Some(0),
        };
        let mut writer = BitWriter::new();
        let err = write_gdf_params(&mut writer, &params, false, &base_filter(), base_geometry())
            .unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "gdf_frame_enabled"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn gdf_per_block_inferred_true_rejected() {
        // Frame at gdfBlkSize -> per-block bit inferred 0; Some(true) could not be produced.
        let geom = GdfGeometry {
            sb_size: SuperblockSize::Block128x128,
            mi_cols: 32,
            mi_rows: 32,
            tile_cols: 1,
            tile_rows: 1,
            mi_col_starts: &[0],
            mi_row_starts: &[0],
        };
        let params = GdfParams {
            gdf_frame_enable: true,
            gdf_per_block: Some(true),
            gdf_pic_qc_idx: Some(0),
            gdf_pic_scale_idx: Some(0),
        };
        let mut writer = BitWriter::new();
        let err = write_gdf_params(&mut writer, &params, false, &base_filter(), geom).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "gdf_per_block"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn gdf_qc_idx_too_wide_rejected() {
        let params = GdfParams {
            gdf_frame_enable: true,
            gdf_per_block: Some(false),
            gdf_pic_qc_idx: Some(4), // f(2) max is 3
            gdf_pic_scale_idx: Some(0),
        };
        let mut writer = BitWriter::new();
        let err = write_gdf_params(&mut writer, &params, false, &base_filter(), base_geometry())
            .unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "gdf_pic_qc_idx"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn gdf_scale_idx_too_wide_rejected() {
        let params = GdfParams {
            gdf_frame_enable: true,
            gdf_per_block: Some(true),
            gdf_pic_qc_idx: Some(0),
            gdf_pic_scale_idx: Some(7), // f(2) max is 3
        };
        let mut writer = BitWriter::new();
        let err = write_gdf_params(&mut writer, &params, false, &base_filter(), base_geometry())
            .unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "gdf_pic_scale_idx"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    // ===== cdef (§ 5.18.7.10) =====

    fn roundtrip_cdef(bits: Bits, coded_lossless: bool, num_planes: u8, filter: CoreSeqFilterView) {
        let data = bits.into_bytes();
        let mut rd = reader(&data);
        let params = parse_cdef_params(&mut rd, coded_lossless, num_planes, &filter).unwrap();
        let consumed = rd.consumed_bits();
        let mut writer = BitWriter::new();
        write_cdef_params(&mut writer, &params, coded_lossless, num_planes, &filter).unwrap();
        assert_eq!(writer.bit_len(), consumed, "bit length matches parser");
        let bytes = writer.into_bytes();
        let reparsed = parse_cdef_params(&mut reader(&bytes), coded_lossless, num_planes, &filter).unwrap();
        assert_eq!(reparsed, params);
    }

    #[test]
    fn cdef_coded_lossless_round_trips() {
        roundtrip_cdef(Bits::default(), true, 3, base_filter());
    }

    #[test]
    fn cdef_disabled_seq_round_trips() {
        let mut filter = base_filter();
        filter.enable_cdef = false;
        roundtrip_cdef(Bits::default(), false, 3, filter);
    }

    #[test]
    fn cdef_frame_disabled_round_trips() {
        let mut bits = Bits::default();
        bits.bit(0); // cdef_frame_enable = 0
        roundtrip_cdef(bits, false, 3, base_filter());
    }

    #[test]
    fn cdef_multiple_strengths_round_trips() {
        let mut bits = Bits::default();
        bits.bit(1); // cdef_frame_enable
        bits.f(1, 2); // cdef_damping_minus_3 -> 4
        bits.f(1, 3); // cdef_strengths_minus_1 -> 2
        bits.bit(1); // cdef_on_skip_txfm (adaptive)
        // strength 0
        bits.bit(0); // y_pri_zero == 0
        bits.f(9, 4); // y_pri_strength
        bits.f(3, 2); // y_sec_strength 3 -> 4 (the remap edge)
        bits.bit(1); // uv_pri_zero == 1 -> 0
        bits.f(2, 2); // uv_sec_strength
        // strength 1
        bits.bit(1); // y_pri_zero == 1 -> 0
        bits.f(1, 2); // y_sec_strength
        bits.bit(0); // uv_pri_zero == 0
        bits.f(5, 4); // uv_pri_strength
        bits.f(3, 2); // uv_sec_strength 3 -> 4
        roundtrip_cdef(bits, false, 3, base_filter());
    }

    #[test]
    fn cdef_monochrome_round_trips() {
        let mut bits = Bits::default();
        bits.bit(1); // cdef_frame_enable
        bits.f(0, 2); // damping
        bits.f(0, 3); // strengths -> 1
        bits.bit(1); // cdef_on_skip_txfm
        bits.bit(0); // y_pri_zero
        bits.f(7, 4); // y_pri_strength
        bits.f(1, 2); // y_sec_strength
        roundtrip_cdef(bits, false, 1, base_filter());
    }

    #[test]
    fn cdef_skip_txfm_always_on_round_trips() {
        let mut filter = base_filter();
        filter.cdef_on_skip_txfm = CdefOnSkipTxfm::AlwaysOn;
        let mut bits = Bits::default();
        bits.bit(1); // cdef_frame_enable
        bits.f(0, 2); // damping
        bits.f(0, 3); // strengths -> 1
        // no skip-txfm bit
        bits.bit(1); // y_pri_zero
        bits.f(0, 2); // y_sec_strength
        bits.bit(1); // uv_pri_zero
        bits.f(0, 2); // uv_sec_strength
        roundtrip_cdef(bits, false, 3, filter);
    }

    #[test]
    fn cdef_skip_txfm_disabled_round_trips() {
        let mut filter = base_filter();
        filter.cdef_on_skip_txfm = CdefOnSkipTxfm::Disabled;
        let mut bits = Bits::default();
        bits.bit(1); // cdef_frame_enable
        bits.f(0, 2); // damping
        bits.f(0, 3); // strengths -> 1
        bits.bit(1); // y_pri_zero
        bits.f(0, 2); // y_sec_strength
        bits.bit(1); // uv_pri_zero
        bits.f(0, 2); // uv_sec_strength
        roundtrip_cdef(bits, false, 3, filter);
    }

    #[test]
    fn cdef_single_picture_round_trips() {
        // No cdef_frame_enable bit (inferred 1); CdefDamping = 6 (max), CdefStrengths = 8
        // (max), one cdef_on_skip_txfm bit, then 8 strength sets.
        let mut filter = base_filter();
        filter.single_picture_header_flag = true;
        let mut bits = Bits::default();
        bits.f(3, 2); // cdef_damping_minus_3 -> CdefDamping = 6
        bits.f(7, 3); // cdef_strengths_minus_1 -> CdefStrengths = 8
        bits.bit(1); // cdef_on_skip_txfm (adaptive -> read once)
        for _ in 0..8 {
            bits.bit(1); // y_pri_zero
            bits.f(0, 2); // y_sec
            bits.bit(1); // uv_pri_zero
            bits.f(0, 2); // uv_sec
        }
        roundtrip_cdef(bits, false, 3, filter);
    }

    #[test]
    fn cdef_disabled_nonzero_model_rejected() {
        let params = CdefParams {
            cdef_frame_enable: true,
            cdef_damping: None,
            cdef_strengths: None,
            cdef_on_skip_txfm_frame_enable: None,
            strengths: Vec::new(),
        };
        let mut writer = BitWriter::new();
        let err = write_cdef_params(&mut writer, &params, true, 3, &base_filter()).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "cdef_disabled"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn cdef_single_picture_disabled_frame_rejected() {
        let mut filter = base_filter();
        filter.single_picture_header_flag = true;
        let params = CdefParams {
            cdef_frame_enable: false,
            cdef_damping: None,
            cdef_strengths: None,
            cdef_on_skip_txfm_frame_enable: None,
            strengths: Vec::new(),
        };
        let mut writer = BitWriter::new();
        let err = write_cdef_params(&mut writer, &params, false, 3, &filter).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "cdef_frame_enable"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn cdef_frame_disabled_with_some_rejected() {
        let params = CdefParams {
            cdef_frame_enable: false,
            cdef_damping: Some(4),
            cdef_strengths: None,
            cdef_on_skip_txfm_frame_enable: None,
            strengths: Vec::new(),
        };
        let mut writer = BitWriter::new();
        let err = write_cdef_params(&mut writer, &params, false, 3, &base_filter()).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "cdef_frame_disabled"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn cdef_enabled_with_none_rejected() {
        let params = CdefParams {
            cdef_frame_enable: true,
            cdef_damping: Some(4),
            cdef_strengths: None, // should be Some
            cdef_on_skip_txfm_frame_enable: Some(true),
            strengths: Vec::new(),
        };
        let mut writer = BitWriter::new();
        let err = write_cdef_params(&mut writer, &params, false, 3, &base_filter()).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "cdef_frame_enabled"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    fn one_set() -> CdefStrengthSet {
        CdefStrengthSet {
            y_pri_strength: 0,
            y_sec_strength: 0,
            uv_pri_strength: 0,
            uv_sec_strength: 0,
        }
    }

    fn enabled_cdef(strengths: u8, sets: Vec<CdefStrengthSet>) -> CdefParams {
        CdefParams {
            cdef_frame_enable: true,
            cdef_damping: Some(4),
            cdef_strengths: Some(strengths),
            cdef_on_skip_txfm_frame_enable: Some(true),
            strengths: sets,
        }
    }

    #[test]
    fn cdef_damping_below_min_rejected() {
        let mut params = enabled_cdef(1, vec![one_set()]);
        params.cdef_damping = Some(2); // < 3
        let mut writer = BitWriter::new();
        let err = write_cdef_params(&mut writer, &params, false, 3, &base_filter()).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "cdef_damping"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn cdef_damping_above_max_rejected() {
        let mut params = enabled_cdef(1, vec![one_set()]);
        params.cdef_damping = Some(7); // > 6
        let mut writer = BitWriter::new();
        let err = write_cdef_params(&mut writer, &params, false, 3, &base_filter()).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "cdef_damping"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn cdef_strengths_zero_rejected() {
        let params = enabled_cdef(0, Vec::new()); // CdefStrengths 0 < 1
        let mut writer = BitWriter::new();
        let err = write_cdef_params(&mut writer, &params, false, 3, &base_filter()).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "cdef_strengths"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn cdef_strengths_above_max_rejected() {
        let params = enabled_cdef(9, vec![one_set(); 9]); // > 8
        let mut writer = BitWriter::new();
        let err = write_cdef_params(&mut writer, &params, false, 3, &base_filter()).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "cdef_strengths"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn cdef_strengths_len_mismatch_rejected() {
        let params = enabled_cdef(2, vec![one_set()]); // says 2, has 1
        let mut writer = BitWriter::new();
        let err = write_cdef_params(&mut writer, &params, false, 3, &base_filter()).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "cdef_strengths_len"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn cdef_skip_txfm_always_on_false_rejected() {
        let mut filter = base_filter();
        filter.cdef_on_skip_txfm = CdefOnSkipTxfm::AlwaysOn;
        let mut params = enabled_cdef(1, vec![one_set()]);
        params.cdef_on_skip_txfm_frame_enable = Some(false); // AlwaysOn infers true
        let mut writer = BitWriter::new();
        let err = write_cdef_params(&mut writer, &params, false, 3, &filter).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "cdef_on_skip_txfm_frame_enable"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn cdef_skip_txfm_disabled_true_rejected() {
        let mut filter = base_filter();
        filter.cdef_on_skip_txfm = CdefOnSkipTxfm::Disabled;
        let mut params = enabled_cdef(1, vec![one_set()]);
        params.cdef_on_skip_txfm_frame_enable = Some(true); // Disabled infers false
        let mut writer = BitWriter::new();
        let err = write_cdef_params(&mut writer, &params, false, 3, &filter).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "cdef_on_skip_txfm_frame_enable"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn cdef_pri_strength_too_wide_rejected() {
        let mut set = one_set();
        set.y_pri_strength = 16; // f(4) max is 15
        let params = enabled_cdef(1, vec![set]);
        let mut writer = BitWriter::new();
        let err = write_cdef_params(&mut writer, &params, false, 3, &base_filter()).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "cdef_pri_strength"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn cdef_sec_strength_three_rejected() {
        let mut set = one_set();
        set.y_sec_strength = 3; // impossible: 3 is remapped to 4 on parse
        let params = enabled_cdef(1, vec![set]);
        let mut writer = BitWriter::new();
        let err = write_cdef_params(&mut writer, &params, false, 3, &base_filter()).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "cdef_sec_strength"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn cdef_sec_strength_above_four_rejected() {
        let mut set = one_set();
        set.y_sec_strength = 5; // not in {0,1,2,4}
        let params = enabled_cdef(1, vec![set]);
        let mut writer = BitWriter::new();
        let err = write_cdef_params(&mut writer, &params, false, 3, &base_filter()).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "cdef_sec_strength"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn cdef_monochrome_nonzero_uv_rejected() {
        let mut set = one_set();
        set.uv_pri_strength = 5; // monochrome never codes UV
        let params = enabled_cdef(1, vec![set]);
        let mut writer = BitWriter::new();
        let err = write_cdef_params(&mut writer, &params, false, 1, &base_filter()).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "cdef_uv_monochrome"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn cdef_y_pri_zero_canonicalization_collapses_explicit_form() {
        // A parser fed the explicit y_pri_zero == 0 with a coded f(4) zero produces
        // y_pri_strength == 0; the writer re-emits the shorter y_pri_zero == 1 form (semantic
        // round-trip), which is fewer bits but reparses identically.
        let mut bits = Bits::default();
        bits.bit(1); // cdef_frame_enable
        bits.f(0, 2); // damping
        bits.f(0, 3); // strengths -> 1
        bits.bit(1); // cdef_on_skip_txfm
        bits.bit(0); // y_pri_zero == 0 (explicit, redundant)
        bits.f(0, 4); // y_pri_strength = 0
        bits.f(0, 2); // y_sec_strength
        bits.bit(1); // uv_pri_zero
        bits.f(0, 2); // uv_sec_strength
        let data = bits.into_bytes();
        let params = parse_cdef_params(&mut reader(&data), false, 3, &base_filter()).unwrap();
        assert_eq!(params.strengths[0].y_pri_strength, 0);
        let mut writer = BitWriter::new();
        write_cdef_params(&mut writer, &params, false, 3, &base_filter()).unwrap();
        // The canonical form drops the explicit f(4): 1 (zero flag) instead of 1 + 4 bits.
        // enable(1) + damping(2) + strengths(3) + onskip(1) + y_pri_zero(1) + y_sec(2)
        //   + uv_pri_zero(1) + uv_sec(2) = 13 bits.
        assert_eq!(writer.bit_len(), 13);
        let bytes = writer.into_bytes();
        let reparsed = parse_cdef_params(&mut reader(&bytes), false, 3, &base_filter()).unwrap();
        assert_eq!(reparsed, params);
    }
}

// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

// Property tests for the §5.18.5.2 / §5.18.7.9 / §5.18.7.10 frame loop-filter writers: each
// parser is driven on random bits + gating, then the parsed model is re-emitted and reparsed
// to assert the universal semantic round-trip; plus a "never panics" property over arbitrary
// (possibly invalid) constructed models.

// `include!`d into `crate::write::frame_filters` so `super::*` resolves to its writers and
// private helpers (the unit/reject tests live in the sibling `frame_filters_tests.rs`).

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod proptests {
    use super::*;
    use crate::bitio::BitReader;
    use crate::headers::frame::{
        CdefStrengthSet, parse_cdef_params, parse_deblocking_filter_params, parse_gdf_params,
    };
    use crate::headers::sequence::SuperblockSize;
    use crate::span::ByteOffset;
    use proptest::prelude::*;

    fn pack(bits: &[bool]) -> Vec<u8> {
        let mut out = Vec::new();
        for chunk in bits.chunks(8) {
            let mut byte = 0u8;
            for (i, b) in chunk.iter().enumerate() {
                byte |= u8::from(*b) << (7 - i);
            }
            out.push(byte);
        }
        out.extend_from_slice(&[0u8; 8]); // pad so the parser never hits EOF mid-field
        out
    }

    fn reader(bytes: &[u8]) -> BitReader<'_> {
        BitReader::new(bytes, ByteOffset::new(0))
    }

    fn arbitrary_skip_txfm() -> impl Strategy<Value = CdefOnSkipTxfm> {
        prop_oneof![
            Just(CdefOnSkipTxfm::Adaptive),
            Just(CdefOnSkipTxfm::AlwaysOn),
            Just(CdefOnSkipTxfm::Disabled),
        ]
    }

    fn arbitrary_sb_size() -> impl Strategy<Value = SuperblockSize> {
        prop_oneof![
            Just(SuperblockSize::Block64x64),
            Just(SuperblockSize::Block128x128),
            Just(SuperblockSize::Block256x256),
        ]
    }

    proptest! {
        // ===== deblocking (§ 5.18.5.2) =====

        /// Every parser-reachable deblocking_filter_params round-trips: parse random bits +
        /// gating, then re-emit and reparse to the same model.
        #[test]
        fn deblocking_round_trips(
            coded_lossless in any::<bool>(),
            num_planes in prop_oneof![Just(1u8), Just(3u8)],
            // Mostly the conformant 0..=3 range (dfParBits 2..=5); occasionally an out-of-range
            // value so the coded_lossless path (which never codes dfParBits) still round-trips,
            // while the non-lossless over-wide path is parser-rejected and skipped by the
            // `if let Ok` guard below.
            df_par_bits_minus_2 in prop_oneof![0u8..=3, Just(30u8), Just(31u8), Just(255u8)],
            mfh in proptest::option::of((any::<bool>(), any::<[bool; 4]>())),
            bits in proptest::collection::vec(any::<bool>(), 0..40),
        ) {
            let view = mfh.map(|(update, apply)| MfhDeblockingView {
                mfh_deblocking_filter_update: update,
                mfh_apply_deblocking_filter: apply,
            });
            let packed = pack(&bits);
            if let Ok(params) = parse_deblocking_filter_params(
                &mut reader(&packed),
                coded_lossless,
                num_planes,
                df_par_bits_minus_2,
                view.as_ref(),
            ) {
                let mut writer = BitWriter::new();
                write_deblocking_filter_params(
                    &mut writer,
                    &params,
                    coded_lossless,
                    num_planes,
                    df_par_bits_minus_2,
                    view.as_ref(),
                )
                .unwrap();
                let written = writer.into_bytes();
                let reparsed = parse_deblocking_filter_params(
                    &mut reader(&written),
                    coded_lossless,
                    num_planes,
                    df_par_bits_minus_2,
                    view.as_ref(),
                )
                .unwrap();
                prop_assert_eq!(reparsed, params);
            }
        }

        /// The deblocking writer never panics on an arbitrary (possibly invalid) model + gating,
        /// and on Err leaves the writer empty.
        #[test]
        fn deblocking_writer_never_panics_on_constructed_models(
            apply in any::<[bool; 4]>(),
            present in any::<[bool; 4]>(),
            df_delta_q in any::<[i32; 4]>(),
            coded_lossless in any::<bool>(),
            num_planes in prop_oneof![Just(1u8), Just(3u8)],
            df_par_bits_minus_2 in any::<u8>(),
            mfh in proptest::option::of((any::<bool>(), any::<[bool; 4]>())),
        ) {
            let view = mfh.map(|(update, a)| MfhDeblockingView {
                mfh_deblocking_filter_update: update,
                mfh_apply_deblocking_filter: a,
            });
            let params = DeblockingFilterParams {
                apply_deblocking_filter: apply,
                df_delta_q_present: present,
                df_delta_q,
            };
            let mut writer = BitWriter::new();
            let result = write_deblocking_filter_params(
                &mut writer,
                &params,
                coded_lossless,
                num_planes,
                df_par_bits_minus_2,
                view.as_ref(),
            );
            if result.is_err() {
                prop_assert_eq!(writer.bit_len(), 0);
            }
        }

        // ===== gdf (§ 5.18.7.9) =====

        /// Every parser-reachable gdf_params round-trips.
        #[test]
        fn gdf_round_trips(
            coded_lossless in any::<bool>(),
            enable_gdf in any::<bool>(),
            gdf_unit_matches_sb_size in any::<bool>(),
            disable_loopfilters_across_tiles in any::<bool>(),
            single_picture_header_flag in any::<bool>(),
            sb_size in arbitrary_sb_size(),
            mi_cols in 0u32..=65536,
            mi_rows in 0u32..=65536,
            tile_cols in 1u32..=64,
            tile_rows in 1u32..=64,
            col_starts in proptest::collection::vec(0u32..=65536, 0..=64),
            row_starts in proptest::collection::vec(0u32..=65536, 0..=64),
            bits in proptest::collection::vec(any::<bool>(), 0..16),
        ) {
            let filter = CoreSeqFilterView {
                enable_cdef: true,
                enable_gdf,
                gdf_unit_matches_sb_size,
                disable_loopfilters_across_tiles,
                cdef_on_skip_txfm: CdefOnSkipTxfm::Adaptive,
                df_par_bits_minus_2: 0,
                single_picture_header_flag,
            };
            let geometry = GdfGeometry {
                sb_size,
                mi_cols,
                mi_rows,
                tile_cols,
                tile_rows,
                mi_col_starts: &col_starts,
                mi_row_starts: &row_starts,
            };
            let packed = pack(&bits);
            if let Ok(params) = parse_gdf_params(&mut reader(&packed), coded_lossless, &filter, geometry) {
                let mut writer = BitWriter::new();
                write_gdf_params(&mut writer, &params, coded_lossless, &filter, geometry).unwrap();
                let written = writer.into_bytes();
                let reparsed =
                    parse_gdf_params(&mut reader(&written), coded_lossless, &filter, geometry).unwrap();
                prop_assert_eq!(reparsed, params);
            }
        }

        /// The GDF writer never panics on an arbitrary (possibly invalid) model + gating, and on
        /// Err leaves the writer empty.
        #[test]
        fn gdf_writer_never_panics_on_constructed_models(
            gdf_frame_enable in any::<bool>(),
            gdf_per_block in proptest::option::of(any::<bool>()),
            gdf_pic_qc_idx in proptest::option::of(any::<u8>()),
            gdf_pic_scale_idx in proptest::option::of(any::<u8>()),
            coded_lossless in any::<bool>(),
            enable_gdf in any::<bool>(),
            gdf_unit_matches_sb_size in any::<bool>(),
            disable_loopfilters_across_tiles in any::<bool>(),
            single_picture_header_flag in any::<bool>(),
            sb_size in arbitrary_sb_size(),
            mi_cols in 0u32..=65536,
            mi_rows in 0u32..=65536,
            tile_cols in 1u32..=64,
            tile_rows in 1u32..=64,
        ) {
            let filter = CoreSeqFilterView {
                enable_cdef: true,
                enable_gdf,
                gdf_unit_matches_sb_size,
                disable_loopfilters_across_tiles,
                cdef_on_skip_txfm: CdefOnSkipTxfm::Adaptive,
                df_par_bits_minus_2: 0,
                single_picture_header_flag,
            };
            let geometry = GdfGeometry {
                sb_size,
                mi_cols,
                mi_rows,
                tile_cols,
                tile_rows,
                mi_col_starts: &[0],
                mi_row_starts: &[0],
            };
            let params = GdfParams {
                gdf_frame_enable,
                gdf_per_block,
                gdf_pic_qc_idx,
                gdf_pic_scale_idx,
            };
            let mut writer = BitWriter::new();
            let result = write_gdf_params(&mut writer, &params, coded_lossless, &filter, geometry);
            if result.is_err() {
                prop_assert_eq!(writer.bit_len(), 0);
            }
        }

        // ===== cdef (§ 5.18.7.10) =====

        /// Every parser-reachable cdef_params round-trips.
        #[test]
        fn cdef_round_trips(
            coded_lossless in any::<bool>(),
            enable_cdef in any::<bool>(),
            num_planes in prop_oneof![Just(1u8), Just(3u8)],
            single_picture_header_flag in any::<bool>(),
            cdef_on_skip_txfm in arbitrary_skip_txfm(),
            bits in proptest::collection::vec(any::<bool>(), 0..80),
        ) {
            let filter = CoreSeqFilterView {
                enable_cdef,
                enable_gdf: true,
                gdf_unit_matches_sb_size: false,
                disable_loopfilters_across_tiles: false,
                cdef_on_skip_txfm,
                df_par_bits_minus_2: 0,
                single_picture_header_flag,
            };
            let packed = pack(&bits);
            if let Ok(params) = parse_cdef_params(&mut reader(&packed), coded_lossless, num_planes, &filter) {
                let mut writer = BitWriter::new();
                write_cdef_params(&mut writer, &params, coded_lossless, num_planes, &filter).unwrap();
                let written = writer.into_bytes();
                let reparsed =
                    parse_cdef_params(&mut reader(&written), coded_lossless, num_planes, &filter).unwrap();
                prop_assert_eq!(reparsed, params);
            }
        }

        /// The CDEF writer never panics on an arbitrary (possibly invalid) model + gating, and
        /// on Err leaves the writer empty.
        #[test]
        fn cdef_writer_never_panics_on_constructed_models(
            cdef_frame_enable in any::<bool>(),
            cdef_damping in proptest::option::of(any::<u8>()),
            cdef_strengths in proptest::option::of(any::<u8>()),
            cdef_on_skip_txfm_frame_enable in proptest::option::of(any::<bool>()),
            sets in proptest::collection::vec(
                (any::<u8>(), any::<u8>(), any::<u8>(), any::<u8>()),
                0..12,
            ),
            coded_lossless in any::<bool>(),
            enable_cdef in any::<bool>(),
            num_planes in prop_oneof![Just(1u8), Just(3u8)],
            single_picture_header_flag in any::<bool>(),
            cdef_on_skip_txfm in arbitrary_skip_txfm(),
        ) {
            let filter = CoreSeqFilterView {
                enable_cdef,
                enable_gdf: true,
                gdf_unit_matches_sb_size: false,
                disable_loopfilters_across_tiles: false,
                cdef_on_skip_txfm,
                df_par_bits_minus_2: 0,
                single_picture_header_flag,
            };
            let strengths = sets
                .into_iter()
                .map(|(y_pri, y_sec, uv_pri, uv_sec)| CdefStrengthSet {
                    y_pri_strength: y_pri,
                    y_sec_strength: y_sec,
                    uv_pri_strength: uv_pri,
                    uv_sec_strength: uv_sec,
                })
                .collect();
            let params = CdefParams {
                cdef_frame_enable,
                cdef_damping,
                cdef_strengths,
                cdef_on_skip_txfm_frame_enable,
                strengths,
            };
            let mut writer = BitWriter::new();
            let result =
                write_cdef_params(&mut writer, &params, coded_lossless, num_planes, &filter);
            if result.is_err() {
                prop_assert_eq!(writer.bit_len(), 0);
            }
        }
    }
}

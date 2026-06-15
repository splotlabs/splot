// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

// Unit, byte-exact, and rejection tests for the § 5.18.7.2 frame tile_info writer.

// `include!`d into `crate::write::frame_tiling` so `super::*` resolves to its writers and
// private helpers. The property tests live in the sibling `frame_tiling_proptests.rs`.

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::too_many_lines)]
mod tests {
    use super::*;
    use crate::headers::frame::parse_tile_info;
    use crate::headers::sequence::{LevelIdx, Tier};

    /// MSB-first bit builder mirroring the `Bits` helper in the parser's own `tiling`
    /// tests, so this module reuses the same hand-built, spec-grounded fixtures.
    #[derive(Default)]
    struct Bits {
        bits: Vec<u8>,
    }

    impl Bits {
        fn bit(&mut self, bit: u8) -> &mut Self {
            self.bits.push(bit & 1);
            self
        }

        fn f(&mut self, value: u32, width: u32) -> &mut Self {
            for shift in (0..width).rev() {
                self.bit(((value >> shift) & 1) as u8);
            }
            self
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

    /// A 64x64-superblock sequence with no sequence tile info, level 0 Main tier, and
    /// frame-level CDF context updates enabled (mirrors the parser-test `base_view`).
    fn base_view() -> CoreSeqTileView {
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

    /// A stored uniform 2x2 sequence tile layout for a 4x4-superblock frame.
    fn uniform_2x2_seq_params() -> TileParams {
        TileParams {
            tile_cols: 2,
            tile_rows: 2,
            tile_cols_log2: 1,
            tile_rows_log2: 1,
            sb_cols: 4,
            sb_rows: 4,
            uniform_spacing: true,
            covers_cols: true,
            covers_rows: true,
        }
    }

    /// Parses `tile_info()` from `data` with the same fixed flags the round-trip uses.
    fn parse(
        view: &CoreSeqTileView,
        data: &[u8],
        frame: FrameSize,
        is_bridge: bool,
        tip: bool,
    ) -> TileInfo {
        let mut reader = BitReader::new(data, ByteOffset::new(0));
        parse_tile_info(&mut reader, view, frame, true, is_bridge, tip).unwrap()
    }

    /// `tile_info()` bits for a single uniform tile (just `uniform_tile_spacing_flag = 1`).
    fn single_uniform_tile_bits() -> Vec<u8> {
        let mut bits = Bits::default();
        bits.bit(1);
        bits.into_bytes()
    }

    /// `tile_info()` bits for the explicit uniform 2-column layout on a 256x256 frame:
    /// uniform=1, one column increment, no row increment, then context_update_tile_id f(1)
    /// = 1 and tile_size_bytes_minus_1 f(2) = 3.
    fn explicit_multi_tile_bits() -> Vec<u8> {
        let mut bits = Bits::default();
        bits.bit(1).bit(1).bit(0).bit(0).f(1, 1).f(3, 2);
        bits.into_bytes()
    }

    /// Writes `info`, reparses, and asserts byte-exact semantic round-trip.
    fn assert_round_trip(
        info: &TileInfo,
        view: &CoreSeqTileView,
        frame: FrameSize,
        is_bridge: bool,
        tip: bool,
    ) {
        let mut writer = BitWriter::new();
        write_tile_info(&mut writer, info, view, frame, true, is_bridge, tip).unwrap();
        let bytes = writer.into_bytes();
        let mut reader = BitReader::new(&bytes, ByteOffset::new(0));
        let reparsed = parse_tile_info(&mut reader, view, frame, true, is_bridge, tip).unwrap();
        assert_eq!(&reparsed, info);
    }

    #[test]
    fn explicit_uniform_single_tile_round_trips() {
        // 16x8 frame, no sequence tile info -> explicit tile_params() reads only
        // uniform_tile_spacing_flag; single tile, no trailing fields.
        let mut bits = Bits::default();
        bits.bit(1); // uniform_tile_spacing_flag = 1
        let info = parse(&base_view(), &bits.into_bytes(), FrameSize::new(16, 8), false, false);
        assert!(!info.reuse_tile_info);
        assert_eq!(info.tile_cols, 1);
        assert_eq!(info.tile_size_bytes, None);
        assert!(info.tile_params.is_some());
        assert_round_trip(&info, &base_view(), FrameSize::new(16, 8), false, false);
    }

    #[test]
    fn explicit_uniform_multi_tile_round_trips() {
        // 256x256 frame: uniform with one column increment -> 2 columns, trailing
        // context_update_tile_id f(1) + tile_size_bytes_minus_1 f(2).
        let mut bits = Bits::default();
        bits.bit(1) // uniform_tile_spacing_flag
            .bit(1) // increment_tile_cols_log2 = 1
            .bit(0) // increment_tile_cols_log2 = 0
            .bit(0) // increment_tile_rows_log2 = 0
            .f(1, 1) // context_update_tile_id
            .f(3, 2); // tile_size_bytes_minus_1 -> TileSizeBytes = 4
        let info = parse(
            &base_view(),
            &bits.into_bytes(),
            FrameSize::new(256, 256),
            false,
            false,
        );
        assert!(!info.reuse_tile_info);
        assert_eq!(info.tile_cols, 2);
        assert_eq!(info.tile_rows, 1);
        assert_eq!(info.context_update_tile_id, 1);
        assert_eq!(info.tile_size_bytes, Some(4));
        assert_round_trip(&info, &base_view(), FrameSize::new(256, 256), false, false);
    }

    #[test]
    fn explicit_non_uniform_round_trips() {
        // 128x8 frame, BLOCK_64X64 -> sbCols = 2, sbRows = 1. uniform=0 then ns(2)=0 for
        // the first column; two 1-superblock columns, multi-tile trailing fields.
        let mut bits = Bits::default();
        bits.bit(0) // uniform_tile_spacing_flag = 0
            .bit(0) // ns(2) width_in_sbs_minus_1 = 0
            .f(0, 1) // context_update_tile_id (n = TileRowsLog2 + TileColsLog2 = 1)
            .f(0, 2); // tile_size_bytes_minus_1 -> TileSizeBytes = 1
        let info = parse(
            &base_view(),
            &bits.into_bytes(),
            FrameSize::new(128, 8),
            false,
            false,
        );
        assert!(!info.reuse_tile_info);
        assert!(!info.tile_params.as_ref().unwrap().uniform_spacing);
        assert_eq!(info.tile_cols, 2);
        assert_eq!(info.tile_rows, 1);
        assert_round_trip(&info, &base_view(), FrameSize::new(128, 8), false, false);
    }

    #[test]
    fn reuse_uniform_round_trips() {
        // Eligible uniform sequence layout with allow_tile_info_change -> reuse_tile_info
        // f(1) = 1, then reuse_tile_params() recomputes a 2x2 layout; trailing fields.
        let mut view = base_view();
        view.seq_tile_info_present_flag = true;
        view.allow_tile_info_change = true;
        view.seq_tile_params = Some(uniform_2x2_seq_params());
        let mut bits = Bits::default();
        bits.bit(1) // reuse_tile_info
            .f(2, 2) // context_update_tile_id (n = 1 + 1)
            .f(1, 2); // tile_size_bytes_minus_1 -> TileSizeBytes = 2
        let info = parse(&view, &bits.into_bytes(), FrameSize::new(256, 256), false, false);
        assert!(info.reuse_tile_info);
        assert!(info.tile_params.is_none());
        assert_eq!(info.tile_cols, 2);
        assert_eq!(info.tile_rows, 2);
        assert_round_trip(&info, &view, FrameSize::new(256, 256), false, false);
    }

    #[test]
    fn reuse_uniform_inferred_no_bit_round_trips() {
        // Eligible layout without allow_tile_info_change: reuse_tile_info = 1 inferred (no
        // bit). The writer must also emit no reuse bit.
        let mut view = base_view();
        view.seq_tile_info_present_flag = true;
        view.seq_tile_params = Some(uniform_2x2_seq_params());
        // (no reuse bit — inferred) context_update_tile_id f(2) = 0, tile_size_bytes f(2) = 0.
        let mut bits = Bits::default();
        bits.f(0, 2).f(0, 2);
        let info = parse(&view, &bits.into_bytes(), FrameSize::new(256, 256), false, false);
        assert!(info.reuse_tile_info);
        assert_round_trip(&info, &view, FrameSize::new(256, 256), false, false);
    }

    #[test]
    fn reuse_non_uniform_round_trips() {
        // Stored non-uniform layout with matching SeqSbCols/SeqSbRows is eligible; reuse is
        // inferred (no allow_tile_info_change) and the recorded starts pass through.
        let mut view = base_view();
        view.seq_tile_info_present_flag = true;
        let mut params = uniform_2x2_seq_params();
        params.uniform_spacing = false;
        view.seq_tile_params = Some(params);
        view.seq_sb_col_starts = vec![0, 2];
        view.seq_sb_row_starts = vec![0, 2];
        // (no reuse bit — inferred) context_update_tile_id f(2) = 2, tile_size_bytes f(2) = 1.
        let mut bits = Bits::default();
        bits.f(2, 2).f(1, 2);
        let info = parse(&view, &bits.into_bytes(), FrameSize::new(256, 256), false, false);
        assert!(info.reuse_tile_info);
        assert!(info.tile_params.is_none());
        assert_eq!(info.tile_cols, 2);
        assert_eq!(info.tile_rows, 2);
        assert_round_trip(&info, &view, FrameSize::new(256, 256), false, false);
    }

    #[test]
    fn multi_tile_avg_cdf_gate_round_trips() {
        // enable_avg_cdf && avg_cdf_type skip the context_update_tile_id read; only
        // tile_size_bytes_minus_1 f(2) is read. Writer must also skip the ctx bits.
        let mut view = base_view();
        view.enable_avg_cdf = true;
        view.avg_cdf_type = 1;
        let mut bits = Bits::default();
        bits.bit(1) // uniform_tile_spacing_flag
            .bit(1) // increment_tile_cols_log2 = 1
            .bit(0) // increment_tile_cols_log2 = 0
            .bit(0) // increment_tile_rows_log2 = 0
            .f(2, 2); // tile_size_bytes_minus_1 -> TileSizeBytes = 3 (no context_update bits)
        let info = parse(&view, &bits.into_bytes(), FrameSize::new(256, 256), false, false);
        assert_eq!(info.tile_cols, 2);
        assert_eq!(info.context_update_tile_id, 0);
        assert_eq!(info.tile_size_bytes, Some(3));
        assert_round_trip(&info, &view, FrameSize::new(256, 256), false, false);
    }

    #[test]
    fn bridge_minimal_layout_round_trips() {
        // IsBridge: haveTileParams = 0, tile_params() infers uniform=1 with no increments
        // and no bits; trailing fields gated off. Writer emits nothing.
        let mut view = base_view();
        view.seq_tile_info_present_flag = true;
        view.allow_tile_info_change = true;
        view.seq_tile_params = Some(uniform_2x2_seq_params());
        let info = parse(&view, &[], FrameSize::new(256, 256), true, false);
        assert!(!info.reuse_tile_info);
        assert_eq!(info.tile_cols, 1);
        let mut writer = BitWriter::new();
        write_tile_info(
            &mut writer,
            &info,
            &view,
            FrameSize::new(256, 256),
            true,
            true,
            false,
        )
        .unwrap();
        assert_eq!(writer.bit_len(), 0);
        assert_round_trip(&info, &view, FrameSize::new(256, 256), true, false);
    }

    #[test]
    fn not_eligible_inferred_reuse_zero_round_trips() {
        // 16x8 frame with a stored 2x2 uniform layout: uniform_eligible(1, 1) fails, so
        // reuse_tile_info is inferred 0 and tile_params() is read directly. No reuse bit.
        let mut view = base_view();
        view.seq_tile_info_present_flag = true;
        view.allow_tile_info_change = true;
        view.seq_tile_params = Some(uniform_2x2_seq_params());
        // uniform_tile_spacing_flag = 1 (no reuse bit since not eligible).
        let mut bits = Bits::default();
        bits.bit(1);
        let info = parse(&view, &bits.into_bytes(), FrameSize::new(16, 8), false, false);
        assert!(!info.reuse_tile_info);
        assert_eq!(info.tile_cols, 1);
        assert_round_trip(&info, &view, FrameSize::new(16, 8), false, false);
    }

    #[test]
    fn tip_frame_as_output_skips_trailing_round_trips() {
        // Multi-tile layout with TipFrameMode == TIP_FRAME_AS_OUTPUT: trailing fields gated
        // off. No context/tile-size bits on either side.
        let mut bits = Bits::default();
        bits.bit(1) // uniform_tile_spacing_flag
            .bit(1) // increment_tile_cols_log2 = 1
            .bit(0) // increment_tile_cols_log2 = 0
            .bit(0); // increment_tile_rows_log2 = 0
        let info = parse(&base_view(), &bits.into_bytes(), FrameSize::new(256, 256), false, true);
        assert_eq!(info.tile_cols, 2);
        assert_eq!(info.context_update_tile_id, 0);
        assert_eq!(info.tile_size_bytes, None);
        assert_round_trip(&info, &base_view(), FrameSize::new(256, 256), false, true);
    }

    // ---- reject tests (every reject leaves bit_len() == 0) ----

    fn reject(
        info: &TileInfo,
        view: &CoreSeqTileView,
        frame: FrameSize,
        is_bridge: bool,
        tip: bool,
    ) -> WriteError {
        let mut writer = BitWriter::new();
        let err = write_tile_info(&mut writer, info, view, frame, true, is_bridge, tip).unwrap_err();
        assert_eq!(writer.bit_len(), 0);
        err
    }

    #[test]
    fn reserved_level_present_flag_rejected() {
        // seq_tile_info_present_flag set without a stored layout (reserved seq_level_idx):
        // the parser stops, so the writer cannot emit the bits.
        let mut view = base_view();
        view.seq_tile_info_present_flag = true;
        // A minimal TileInfo (its field values are irrelevant — the grid check fires first).
        let info = TileInfo {
            reuse_tile_info: false,
            tile_cols: 1,
            tile_rows: 1,
            tile_cols_log2: 0,
            tile_rows_log2: 0,
            mi_col_starts: vec![0, 4],
            mi_row_starts: vec![0, 2],
            context_update_tile_id: 0,
            tile_size_bytes: None,
            tile_params: None,
        };
        let err = reject(&info, &view, FrameSize::new(16, 8), false, false);
        assert_eq!(
            err,
            WriteError::UnwritableSequenceHeader {
                feature: "AV2-5.18.7-SEGMENTATION-TILING"
            }
        );
    }

    #[test]
    fn inferred_reuse_mismatch_rejected() {
        // Not eligible (no sequence tile info) -> reuse_tile_info is inferred 0, but the
        // stored model claims reuse = true.
        let mut info = parse(
            &base_view(),
            &single_uniform_tile_bits(),
            FrameSize::new(16, 8),
            false,
            false,
        );
        info.reuse_tile_info = true;
        let err = reject(&info, &base_view(), FrameSize::new(16, 8), false, false);
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "reuse_tile_info"
            }
        );
    }

    #[test]
    fn reuse_layout_mismatch_rejected() {
        // Eligible reuse layout, but the stored counts disagree with reuse_tile_params().
        let mut view = base_view();
        view.seq_tile_info_present_flag = true;
        view.seq_tile_params = Some(uniform_2x2_seq_params());
        // (no reuse bit — inferred) context_update_tile_id f(2) = 0, tile_size_bytes f(2) = 0.
        let mut bits = Bits::default();
        bits.f(0, 2).f(0, 2);
        let mut info = parse(&view, &bits.into_bytes(), FrameSize::new(256, 256), false, false);
        info.tile_cols = 3; // re-derivation produces 2
        let err = reject(&info, &view, FrameSize::new(256, 256), false, false);
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "reuse_tile_params"
            }
        );
    }

    #[test]
    fn explicit_summary_mismatch_rejected() {
        // Explicit layout whose top-level tile_cols disagrees with the surfaced
        // tile_params() (which drives the re-derivation). The mismatch surfaces at the
        // final summary cross-check rather than in the forward replay.
        let mut info = parse(
            &base_view(),
            &explicit_multi_tile_bits(),
            FrameSize::new(256, 256),
            false,
            false,
        );
        info.tile_cols = 3; // the re-derived layout has 2 columns
        let err = reject(&info, &base_view(), FrameSize::new(256, 256), false, false);
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "tile_params_summary"
            }
        );
    }

    #[test]
    fn reuse_branch_tile_params_some_rejected() {
        // The reuse branch writes no tile_params() bits, so the parser leaves tile_params
        // None; a stored Some would reparse to None (round-trip break).
        let mut view = base_view();
        view.seq_tile_info_present_flag = true;
        view.seq_tile_params = Some(uniform_2x2_seq_params());
        let mut bits = Bits::default();
        bits.f(0, 2).f(0, 2); // inferred reuse; context f(2)=0, tile_size f(2)=0
        let mut info = parse(&view, &bits.into_bytes(), FrameSize::new(256, 256), false, false);
        assert!(info.reuse_tile_info);
        assert!(info.tile_params.is_none());
        // Graft a tile_params the reuse branch never carries.
        let explicit = parse(
            &base_view(),
            &explicit_multi_tile_bits(),
            FrameSize::new(256, 256),
            false,
            false,
        );
        info.tile_params = explicit.tile_params;
        assert!(info.tile_params.is_some());
        let err = reject(&info, &view, FrameSize::new(256, 256), false, false);
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "tile_params"
            }
        );
    }

    #[test]
    fn explicit_non_monotonic_starts_rejected() {
        // A non-canonical explicit model with non-monotonic mi_col_starts: the writer must
        // reject (not panic in write_tile_params' consecutive-start subtraction). frame
        // 256x256 -> MiCols 64; recovered sb starts (>> sbShift2) of [0,48,16] are not strictly
        // increasing.
        let explicit = parse(
            &base_view(),
            &explicit_multi_tile_bits(),
            FrameSize::new(256, 256),
            false,
            false,
        );
        let info = TileInfo {
            reuse_tile_info: false,
            tile_cols: 3,
            tile_rows: 1,
            tile_cols_log2: 2,
            tile_rows_log2: 0,
            mi_col_starts: vec![0, 48, 16, 64],
            mi_row_starts: vec![0, 64],
            context_update_tile_id: 0,
            tile_size_bytes: Some(1),
            tile_params: explicit.tile_params,
        };
        let err = reject(&info, &base_view(), FrameSize::new(256, 256), false, false);
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "mi_col_starts"
            }
        );
    }

    #[test]
    fn context_update_width_above_32_rejected() {
        // tile_rows_log2 + tile_cols_log2 > 32 with the context field enabled exceeds the f(n)
        // descriptor max. The layout check bounds the log2s in write_tile_info, so the guard is
        // exercised directly on check_trailing_fields.
        let mut view = base_view();
        view.enable_avg_cdf = false; // context_update_tile_id field is signalled
        let info = TileInfo {
            reuse_tile_info: false,
            tile_cols: 4,
            tile_rows: 4,
            tile_cols_log2: 20,
            tile_rows_log2: 20, // sum 40 > 32
            mi_col_starts: vec![0, 16, 32, 48, 64],
            mi_row_starts: vec![0, 16, 32, 48, 64],
            context_update_tile_id: 0,
            tile_size_bytes: Some(1),
            tile_params: None,
        };
        let err = check_trailing_fields(&info, &view, false, false).unwrap_err();
        assert_eq!(err, WriteError::BitWidthTooLarge { requested: 40, max: 32 });
    }

    #[test]
    fn missing_explicit_tile_params_rejected() {
        // Explicit branch with tile_params() surfaced as None: the writer cannot replay.
        let mut info = parse(
            &base_view(),
            &single_uniform_tile_bits(),
            FrameSize::new(16, 8),
            false,
            false,
        );
        info.tile_params = None;
        let err = reject(&info, &base_view(), FrameSize::new(16, 8), false, false);
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "tile_params"
            }
        );
    }

    #[test]
    fn gated_off_context_update_tile_id_rejected() {
        // avg-CDF gate off the context_update_tile_id read, but a non-zero value is stored.
        let mut view = base_view();
        view.enable_avg_cdf = true;
        view.avg_cdf_type = 1;
        // uniform=1, one column increment, no row increment, tile_size_bytes f(2) (no ctx).
        let mut bits = Bits::default();
        bits.bit(1).bit(1).bit(0).bit(0).f(2, 2);
        let mut info = parse(&view, &bits.into_bytes(), FrameSize::new(256, 256), false, false);
        info.context_update_tile_id = 1;
        let err = reject(&info, &view, FrameSize::new(256, 256), false, false);
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "context_update_tile_id"
            }
        );
    }

    #[test]
    fn tile_size_bytes_none_when_tail_present_rejected() {
        // Multi-tile layout reads tile_size_bytes; storing None is non-canonical.
        let mut info = parse(
            &base_view(),
            &explicit_multi_tile_bits(),
            FrameSize::new(256, 256),
            false,
            false,
        );
        info.tile_size_bytes = None;
        let err = reject(&info, &base_view(), FrameSize::new(256, 256), false, false);
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "tile_size_bytes"
            }
        );
    }

    #[test]
    fn tile_size_bytes_some_when_tail_absent_rejected() {
        // Single tile reads no tile_size_bytes; storing Some is non-canonical.
        let mut info = parse(
            &base_view(),
            &single_uniform_tile_bits(),
            FrameSize::new(16, 8),
            false,
            false,
        );
        info.tile_size_bytes = Some(2);
        let err = reject(&info, &base_view(), FrameSize::new(16, 8), false, false);
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "tile_size_bytes"
            }
        );
    }

    #[test]
    fn context_update_present_when_tail_absent_rejected() {
        // Single tile: context_update_tile_id is inferred 0; storing non-zero is bad.
        let mut info = parse(
            &base_view(),
            &single_uniform_tile_bits(),
            FrameSize::new(16, 8),
            false,
            false,
        );
        info.context_update_tile_id = 1;
        let err = reject(&info, &base_view(), FrameSize::new(16, 8), false, false);
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "context_update_tile_id"
            }
        );
    }
}

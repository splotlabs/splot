// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

// End-to-end round-trip and reject tests for the composing first-tile-group `tile_group_obu()`
// writer (§ 5.19). The round-trip test builds a valid `FrameHeaderCore` + `CoreSeqView` by PARSING a
// real intra frame header (the same fixture-building approach as `frame_header_core_tests.rs`),
// composes a whole OBU payload with `write_tile_group_obu`, then reparses it STAGE BY STAGE from a
// single `BitReader` (prefix -> frame-header core -> structure -> framing) and asserts each stage's
// syntax fields equal the inputs. The reject tests confirm reject-before-write (`bit_len() == 0`)
// for the `continuation_unsupported` form and for a sub-writer reject propagating through the
// scratch buffer.

// `include!`d into `crate::write::tile_group` so `super::*` resolves to the composer + helpers.

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]
mod obu_tests {
    use super::*;
    use crate::bitio::BitReader;
    use crate::headers::frame::{
        CoreSeqCcsoView, CoreSeqFilterView, CoreSeqInterView, CoreSeqQuantView,
        CoreSeqRestorationView, CoreSeqSegView, CoreSeqTileView, FrameHeaderParseStatus, FrameType,
        FrameReferenceStateView, init_core_from_prefix, parse_core_body, parse_frame_header_prefix,
    };
    use crate::headers::sequence::{CdefOnSkipTxfm, ChromaFormatIdc, LevelIdx, SuperblockSize, Tier};
    use crate::headers::tile_group::{
        TileFraming, parse_tile_group_framing, parse_tile_group_prefix,
        parse_tile_group_structure,
    };
    use crate::span::ByteOffset;
    use crate::types::ObuType;

    /// MSB-first bit builder mirroring the parser/writer test helpers.
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

    // ---- sub-view builders (mirror frame_header_core_tests.rs::base_*) -----------------

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

    /// The canonical CLK, cur_mfh_id == 0, single-tile, non-lossless intra `frame_header()` body
    /// bits (the exact fixture from `frame_header_core_tests.rs::clk_direct_reference_bits`). A
    /// 1920x1080 @ 128x128 frame is a single tile (`NumTiles == 1`).
    fn clk_frame_header_bits() -> Bits {
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

    /// Parses a frame-header body (activation prefix + `parse_core_body`) against the directly built
    /// `seq`, the same approach `frame_header_core_tests.rs::parse_core_body_for_test` uses. Reads
    /// from `reader` (so the caller can continue the stage-by-stage reparse afterward).
    fn parse_core_from(
        reader: &mut BitReader<'_>,
        obu_type: ObuType,
        first_pic: bool,
        seq: &CoreSeqView,
    ) -> FrameHeaderCore {
        let prefix = parse_frame_header_prefix(reader, obu_type, Some(first_pic)).unwrap();
        let mut core = init_core_from_prefix(&prefix, obu_type, first_pic);
        parse_core_body(
            reader,
            &mut core,
            seq,
            None,
            &FrameReferenceStateView::unknown(),
        )
        .unwrap();
        core
    }

    /// Builds a valid single-tile `FrameHeaderCore` + `CoreSeqView` by parsing the CLK fixture.
    fn valid_core() -> (FrameHeaderCore, CoreSeqView) {
        let seq = base_seq();
        let data = clk_frame_header_bits().into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let core = parse_core_from(&mut reader, ObuType::ClosedLoopKey, true, &seq);
        assert_eq!(
            core.status,
            FrameHeaderParseStatus::IntraHeaderComplete,
            "fixture must parse to IntraHeaderComplete"
        );
        (core, seq)
    }

    /// A `Complete` single-tile structure (no flag, inferred range 0..=0).
    fn single_tile_structure() -> TileGroupStructure {
        TileGroupStructure {
            tile_start_and_end_present_flag: false,
            tg_start: 0,
            tg_end: 0,
            outcome: TileGroupStructureOutcome::Complete,
            header_bytes: None,
            payload_size: None,
        }
    }

    #[test]
    fn whole_obu_round_trips_stage_by_stage() {
        // Build a valid frame-header core by parsing the CLK fixture (the same fixture-building
        // approach the frame_header_core writer tests use), then a single-tile structure + a
        // single-tile framing whose lone tile is the (last) remainder tile carrying its coded bytes.
        let (core, seq) = valid_core();
        let layout = TileGroupLayout::new(1, 1, 0, 0);
        assert_eq!(layout.num_tiles, 1);
        let structure = single_tile_structure();

        // A single-tile framing: tg_start == tg_end == 0, the lone tile reads no size field and
        // takes the whole region. Build it parser-driven (as the payload tests do) over a 5-byte
        // coded-tile region with a distinct marker per byte.
        let tile_bytes: Vec<u8> = (0u8..5).map(|b| b.wrapping_mul(37)).collect();
        let framing = parse_tile_group_framing(&tile_bytes, 0, 0, 1, false);
        assert_eq!(framing.defect, None);
        assert_eq!(framing.tiles.len(), 1);
        assert_eq!(framing.tiles[0].tile_size, 5);
        let tile_data: &[&[u8]] = &[&tile_bytes];

        // Compose the whole first-tile-group OBU payload.
        let mut writer = BitWriter::new();
        write_tile_group_obu(
            &mut writer,
            &core,
            &seq,
            None,
            true,
            &structure,
            &framing,
            tile_data,
            true,
        )
        .unwrap();
        let bytes = writer.into_bytes();

        // --- Stage 0: the prefix round-trips is_first / frame_header_present on a fresh reader.
        // parse_tile_group_prefix reads is_first_tile_group then the frame_header PREFIX (the
        // activation fields), so confirm the flags here.
        {
            let mut reader = BitReader::new(&bytes, ByteOffset::new(0));
            let prefix =
                parse_tile_group_prefix(&mut reader, ObuType::ClosedLoopKey, Some(true)).unwrap();
            assert!(prefix.is_first_tile_group);
            assert!(prefix.frame_header_present_flag);
            assert!(prefix.frame_header.is_some());
        }

        // --- Stage-by-stage reparse from a single BitReader: read the is_first bit, then the FULL
        // frame-header core, then the § 5.19 structure, then the § 5.20.1 framing.
        let mut reader = BitReader::new(&bytes, ByteOffset::new(0));

        // Stage 1: is_first_tile_group f(1) == 1.
        assert_eq!(reader.read_bit().unwrap(), 1, "is_first_tile_group == 1");

        // Stage 2: the embedded frame_header() — reparse to a core and assert it equals the input.
        let reparsed_core = parse_core_from(&mut reader, ObuType::ClosedLoopKey, true, &seq);
        assert_eq!(
            reparsed_core.status,
            FrameHeaderParseStatus::IntraHeaderComplete
        );
        // Compare the structural fields (consumed_bits differs: the original was parsed standalone,
        // here the reader spans the is_first bit + the trailing tile group). Clear it on both.
        {
            let mut a = reparsed_core.clone();
            let mut b = core.clone();
            a.consumed_bits = 0;
            b.consumed_bits = 0;
            assert_eq!(a, b, "reparsed frame-header core != input core");
        }
        assert_eq!(reparsed_core.frame_type, Some(FrameType::Key));

        // The frame_header() ended with byte_alignment() inside write_tile_group_structure, NOT
        // here: the reader is now positioned right after frame_header(), mid-byte, exactly where the
        // § 5.19 structure begins. parse_tile_group_structure consumes the (empty, single-tile)
        // tg-range bits then byte_alignment(); sz is the remaining bytes from the OBU payload start.
        let sz = bytes.len() as u64;
        let parsed_structure = parse_tile_group_structure(&mut reader, layout, sz).unwrap();
        assert_eq!(parsed_structure.outcome, TileGroupStructureOutcome::Complete);
        assert_eq!(
            parsed_structure.tile_start_and_end_present_flag,
            structure.tile_start_and_end_present_flag
        );
        assert_eq!(parsed_structure.tg_start, structure.tg_start);
        assert_eq!(parsed_structure.tg_end, structure.tg_end);

        // Stage 4: the § 5.20.1 framing over the payload region (payload_size bytes after the
        // structure's byte_alignment(), at headerBytes into the OBU payload).
        let header_bytes = parsed_structure.header_bytes.unwrap() as usize;
        let payload_size = parsed_structure.payload_size.unwrap() as usize;
        let region = &bytes[header_bytes..header_bytes + payload_size];
        let parsed_framing = parse_tile_group_framing(region, 0, 0, 1, false);
        assert_eq!(parsed_framing.defect, None);
        assert_eq!(parsed_framing, framing, "reparsed framing != input framing");
        // The lone tile's coded bytes round-trip byte-exact.
        let t = parsed_framing.tiles[0];
        let start = t.tile_data_offset as usize;
        let end = start + t.tile_size as usize;
        assert_eq!(&region[start..end], tile_bytes.as_slice());
    }

    #[test]
    fn reject_continuation_unsupported() {
        // is_first_tile_group == false is the non-first frame_header_copy() continuation, out of
        // scope: rejected before any bit, leaving the caller's writer untouched.
        let (core, seq) = valid_core();
        let structure = single_tile_structure();
        let tile_bytes = vec![0u8; 4];
        let framing = parse_tile_group_framing(&tile_bytes, 0, 0, 1, false);
        let tile_data: &[&[u8]] = &[&tile_bytes];

        let mut writer = BitWriter::new();
        let err = write_tile_group_obu(
            &mut writer,
            &core,
            &seq,
            None,
            true,
            &structure,
            &framing,
            tile_data,
            false, // is_first_tile_group == false
        )
        .unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalTileGroup {
                what: "continuation_unsupported"
            }
        );
        assert_eq!(writer.bit_len(), 0, "no bit written on continuation reject");
    }

    #[test]
    fn reject_unaligned_writer() {
        // An OBU payload begins byte-aligned; a mid-byte writer would shift the is_first bit and
        // every following byte. The composer rejects before any draft, leaving the caller's stray
        // bit untouched (bit_len() unchanged, nothing appended).
        let (core, seq) = valid_core();
        let structure = single_tile_structure();
        let tile_bytes = vec![0u8; 4];
        let framing = parse_tile_group_framing(&tile_bytes, 0, 0, 1, false);
        let tile_data: &[&[u8]] = &[&tile_bytes];

        let mut writer = BitWriter::new();
        writer.write_bit(1).unwrap(); // one stray bit -> not byte-aligned
        let err = write_tile_group_obu(
            &mut writer,
            &core,
            &seq,
            None,
            true,
            &structure,
            &framing,
            tile_data,
            true,
        )
        .unwrap_err();
        assert_eq!(err, WriteError::WriterNotByteAligned);
        assert_eq!(writer.bit_len(), 1, "stray bit unchanged; composer wrote nothing");
    }

    #[test]
    fn reject_not_tile_group_obu() {
        // Only a tile-group-carrying OBU type frames a tile_group_obu(). A SEF single-picture header
        // is not a tile-group carrier, so the composer rejects it before any bit (write_frame_header_core
        // would otherwise accept it as a frame-header OBU).
        let (mut core, seq) = valid_core();
        core.obu_type = ObuType::RegularSef; // is_tile_group() == false
        let structure = single_tile_structure();
        let tile_bytes = vec![0u8; 4];
        let framing = parse_tile_group_framing(&tile_bytes, 0, 0, 1, false);
        let tile_data: &[&[u8]] = &[&tile_bytes];

        let mut writer = BitWriter::new();
        let err = write_tile_group_obu(
            &mut writer,
            &core,
            &seq,
            None,
            true,
            &structure,
            &framing,
            tile_data,
            true,
        )
        .unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalTileGroup {
                what: "not_tile_group_obu"
            }
        );
        assert_eq!(writer.bit_len(), 0, "no bit written on not-tile-group reject");
    }

    #[test]
    fn reject_first_tg_start_not_zero() {
        // § 6.18: the first tile group's tg_start must be 0. A non-zero first-group tg_start is a
        // conformance violation the composer refuses before any bit.
        let (core, seq) = valid_core();
        let mut structure = single_tile_structure();
        structure.tg_start = 1;
        structure.tg_end = 1;
        let tile_bytes = vec![0u8; 4];
        let framing = parse_tile_group_framing(&tile_bytes, 0, 0, 1, false);
        let tile_data: &[&[u8]] = &[&tile_bytes];

        let mut writer = BitWriter::new();
        let err = write_tile_group_obu(
            &mut writer,
            &core,
            &seq,
            None,
            true,
            &structure,
            &framing,
            tile_data,
            true,
        )
        .unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalTileGroup {
                what: "first_tg_start_not_zero"
            }
        );
        assert_eq!(
            writer.bit_len(),
            0,
            "no bit written on first-tg-start reject"
        );
    }

    #[test]
    fn reject_missing_tile_info() {
        // The § 5.19 layout and § 5.20.1 TileSizeBytes are derived from core.tile_info; with it
        // absent the composer cannot derive them and rejects before any bit.
        let (mut core, seq) = valid_core();
        core.tile_info = None;
        let structure = single_tile_structure();
        let tile_bytes = vec![0u8; 4];
        let framing = parse_tile_group_framing(&tile_bytes, 0, 0, 1, false);
        let tile_data: &[&[u8]] = &[&tile_bytes];

        let mut writer = BitWriter::new();
        let err = write_tile_group_obu(
            &mut writer,
            &core,
            &seq,
            None,
            true,
            &structure,
            &framing,
            tile_data,
            true,
        )
        .unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalTileGroup {
                what: "missing_tile_info"
            }
        );
        assert_eq!(
            writer.bit_len(),
            0,
            "no bit written on missing-tile-info reject"
        );
    }

    #[test]
    fn reject_framing_range_mismatch() {
        // The structure's tg range (single tile: tg_end == 0 -> 1 tile) must match
        // framing.tiles.len(); a two-record framing is rejected before any bit, because a reparse
        // frames the payload from the emitted single-tile range and would treat the first tile's
        // size field as tile data.
        let (core, seq) = valid_core();
        let structure = single_tile_structure(); // tg_start == tg_end == 0 -> expects 1 tile
        // A conformant 2-tile region: tile0 has a 1-byte size field (tile_size 3), tile1 the
        // remainder (tile_size 4).
        let region: Vec<u8> = vec![0x02, 10, 11, 12, 20, 21, 22, 23];
        let framing = parse_tile_group_framing(&region, 0, 1, 1, false);
        assert_eq!(framing.tiles.len(), 2);
        let tile_data: &[&[u8]] = &[&region[1..4], &region[4..8]];

        let mut writer = BitWriter::new();
        let err = write_tile_group_obu(
            &mut writer,
            &core,
            &seq,
            None,
            true,
            &structure,
            &framing,
            tile_data,
            true,
        )
        .unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalTileGroup {
                what: "framing_range_mismatch"
            }
        );
        assert_eq!(
            writer.bit_len(),
            0,
            "no bit written on framing-range-mismatch reject"
        );
    }

    #[test]
    fn frame_header_sub_writer_reject_propagates() {
        // A non-canonical frame-header core (mutated to a non-IntraHeaderComplete status) is
        // rejected by write_frame_header_core; that reject must propagate through the scratch buffer
        // and leave the caller's writer untouched (bit_len() == 0), even though the is_first bit was
        // already drafted into the scratch.
        let (mut core, seq) = valid_core();
        core.status = FrameHeaderParseStatus::ActivationFieldsOnly;
        let structure = single_tile_structure();
        let tile_bytes = vec![0u8; 4];
        let framing = parse_tile_group_framing(&tile_bytes, 0, 0, 1, false);
        let tile_data: &[&[u8]] = &[&tile_bytes];

        let mut writer = BitWriter::new();
        let err = write_tile_group_obu(
            &mut writer,
            &core,
            &seq,
            None,
            true,
            &structure,
            &framing,
            tile_data,
            true,
        )
        .unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader { what: "status" }
        );
        assert_eq!(writer.bit_len(), 0, "no bit written on frame-header reject");
    }

    #[test]
    fn structure_sub_writer_reject_propagates() {
        // A Truncated structure is rejected by write_tile_group_structure; the reject propagates
        // through the scratch (which already holds the is_first bit + the whole frame_header()) and
        // the caller's writer stays untouched.
        let (core, seq) = valid_core();
        let mut structure = single_tile_structure();
        structure.outcome = TileGroupStructureOutcome::Truncated;
        let tile_bytes = vec![0u8; 4];
        let framing = parse_tile_group_framing(&tile_bytes, 0, 0, 1, false);
        let tile_data: &[&[u8]] = &[&tile_bytes];

        let mut writer = BitWriter::new();
        let err = write_tile_group_obu(
            &mut writer,
            &core,
            &seq,
            None,
            true,
            &structure,
            &framing,
            tile_data,
            true,
        )
        .unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalTileGroup {
                what: "incomplete_structure"
            }
        );
        assert_eq!(writer.bit_len(), 0, "no bit written on structure reject");
    }

    #[test]
    fn payload_sub_writer_reject_propagates() {
        // A framing whose lone tile records tile_size == 0 is rejected by write_tile_group_payload
        // (zero_size_tile); the reject propagates through the scratch (holding the is_first bit, the
        // frame_header(), and the structure) and the caller's writer stays untouched.
        let (core, seq) = valid_core();
        let structure = single_tile_structure();
        let framing = TileGroupFraming {
            tiles: vec![TileFraming {
                tile_num: 0,
                size_field_offset: None,
                tile_data_offset: 0,
                tile_size: 0,
            }],
            defect: None,
        };
        let empty: &[u8] = &[];
        let tile_data: &[&[u8]] = &[empty];

        let mut writer = BitWriter::new();
        let err = write_tile_group_obu(
            &mut writer,
            &core,
            &seq,
            None,
            true,
            &structure,
            &framing,
            tile_data,
            true,
        )
        .unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalTileGroup {
                what: "zero_size_tile"
            }
        );
        assert_eq!(writer.bit_len(), 0, "no bit written on payload reject");
    }
}

// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>


#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::bitio::BitReader;
    use crate::headers::atlas_segment::parse_atlas_segment;
    use crate::span::ByteOffset;

    use crate::test_bits::Bits;

    /// Parses a hand-built atlas-segment body into a guaranteed-parser-producible model.
    fn parse(bits: Bits) -> AtlasSegment {
        let bytes = bits.into_bytes();
        let mut reader = BitReader::new(&bytes, ByteOffset::new(0));
        parse_atlas_segment(&mut reader).unwrap()
    }

    /// Writes an atlas-segment body and reparses it, asserting model equality. The body is
    /// variable-width; the parser reads exactly the body bits and ignores the zero byte-padding
    /// `into_bytes` adds after a byte-aligned payload.
    fn round_trip(atlas: &AtlasSegment) {
        let mut writer = BitWriter::new();
        write_atlas_segment(&mut writer, atlas).unwrap();
        let bytes = writer.into_bytes();
        let mut reader = BitReader::new(&bytes, ByteOffset::new(0));
        let reparsed = parse_atlas_segment(&mut reader).unwrap();
        assert_eq!(&reparsed, atlas);
    }

    /// Asserts `write_atlas_segment` rejects `atlas` with the given `what` and writes nothing.
    fn reject(atlas: &AtlasSegment, what: &str) {
        let mut writer = BitWriter::new();
        let err = write_atlas_segment(&mut writer, atlas).unwrap_err();
        assert!(
            matches!(err, WriteError::NonCanonicalAtlasSegment { what: w } if w == what),
            "expected {what} reject, got {err:?}"
        );
        assert_eq!(writer.bit_len(), 0, "reject left bits in the writer");
    }

    // ===================================================================================
    // Fixture builders (each yields a parser-producible model)
    // ===================================================================================

    /// SINGLE_ATLAS (mode 2), unsignaled label.
    fn single_atlas() -> AtlasSegment {
        let mut bits = Bits::default();
        bits.f(3, 3); // atlas_segment_id
        bits.uvlc(2); // mode_idc = SINGLE_ATLAS
        bits.uvlc(1919); // nominal_width_minus_1
        bits.uvlc(1079); // nominal_height_minus_1
        bits.bit(0); // signaled_atlas_segment_ids_flag = 0
        parse(bits)
    }

    /// BASIC_ATLAS (mode 1) with two segments, a stream id, and signaled label ids.
    fn basic_atlas_signaled() -> AtlasSegment {
        let mut bits = Bits::default();
        bits.f(0, 3); // atlas_segment_id
        bits.uvlc(1); // mode_idc = BASIC_ATLAS
        bits.bit(1); // stream_id_present
        bits.uvlc(640); // width
        bits.uvlc(480); // height
        bits.uvlc(1); // num_atlas_segments_minus_1 = 1 -> 2 segments
        for _ in 0..2 {
            bits.f(5, 5); // input_stream_id
            bits.uvlc(0); // top_left_pos_x
            bits.uvlc(0); // top_left_pos_y
            bits.uvlc(100); // width
            bits.uvlc(100); // height
        }
        bits.bit(1); // signaled_atlas_segment_ids_flag = 1
        bits.f(10, 8); // ats_atlas_segment_id[0]
        bits.f(20, 8); // ats_atlas_segment_id[1]
        parse(bits)
    }

    /// BASIC_ATLAS (mode 1) without stream ids and with an unsignaled label.
    fn basic_atlas_unsignaled_no_stream_id() -> AtlasSegment {
        let mut bits = Bits::default();
        bits.f(1, 3); // atlas_segment_id
        bits.uvlc(1); // mode_idc = BASIC_ATLAS
        bits.bit(0); // stream_id_present = 0
        bits.uvlc(320); // width
        bits.uvlc(240); // height
        bits.uvlc(0); // num_atlas_segments_minus_1 = 0 -> 1 segment
        bits.uvlc(0); // top_left_pos_x
        bits.uvlc(0); // top_left_pos_y
        bits.uvlc(50); // width
        bits.uvlc(50); // height
        bits.bit(0); // signaled_atlas_segment_ids_flag = 0
        parse(bits)
    }

    /// ENHANCED_ATLAS (mode 0), uniform spacing, single-region-per-segment.
    fn enhanced_uniform_single_region() -> AtlasSegment {
        let mut bits = Bits::default();
        bits.f(1, 3); // atlas_segment_id
        bits.uvlc(0); // mode_idc = ENHANCED_ATLAS
        bits.uvlc(0); // num_region_columns_minus_1 = 0
        bits.uvlc(0); // num_region_rows_minus_1 = 0 -> NumRegionsInAtlas = 1
        bits.bit(1); // uniform_spacing_flag
        bits.uvlc(63); // region_width_minus_1
        bits.uvlc(63); // region_height_minus_1
        bits.bit(1); // single_region_per_atlas_segment_flag -> numSegments = 1
        bits.bit(0); // signaled_atlas_segment_ids_flag
        parse(bits)
    }

    /// ENHANCED_ATLAS (mode 0), explicit (non-uniform) region dims, explicit mapping.
    fn enhanced_explicit_mapping() -> AtlasSegment {
        let mut bits = Bits::default();
        bits.f(2, 3); // atlas_segment_id
        bits.uvlc(0); // mode_idc = ENHANCED_ATLAS
        bits.uvlc(1); // num_region_columns_minus_1 = 1 -> 2 columns
        bits.uvlc(0); // num_region_rows_minus_1 = 0 -> 1 row, NumRegionsInAtlas = 2
        bits.bit(0); // uniform_spacing_flag = 0 (explicit)
        bits.uvlc(31); // column_width_minus_1[0]
        bits.uvlc(95); // column_width_minus_1[1]
        bits.uvlc(63); // row_height_minus_1[0]
        bits.bit(0); // single_region_per_atlas_segment_flag = 0 (explicit mapping)
        bits.uvlc(1); // num_atlas_segments_minus_1 = 1 -> 2 segments
        for _ in 0..2 {
            bits.uvlc(0); // top_left_region_column
            bits.uvlc(0); // top_left_region_row
            bits.uvlc(0); // bottom_right_region_column_off
            bits.uvlc(0); // bottom_right_region_row_off
        }
        bits.bit(0); // signaled_atlas_segment_ids_flag
        parse(bits)
    }

    /// MULTISTREAM_ATLAS (mode 3), one segment, no background.
    fn multistream_atlas() -> AtlasSegment {
        let mut bits = Bits::default();
        bits.f(2, 3); // atlas_segment_id
        bits.uvlc(3); // mode_idc = MULTISTREAM_ATLAS
        bits.uvlc(3840); // msi_width
        bits.uvlc(2160); // msi_height
        bits.uvlc(0); // num_atlas_segments_minus_1 = 0 -> 1 segment
        bits.bit(0); // background_info_present_flag = 0
        bits.f(1, 5); // input_stream_id
        bits.uvlc(0); // pos_x
        bits.uvlc(0); // pos_y
        bits.uvlc(1920); // width
        bits.uvlc(1080); // height
        bits.bit(0); // signaled_atlas_segment_ids_flag
        parse(bits)
    }

    /// MULTISTREAM_ALPHA_ATLAS (mode 4), two segments, alpha flags coded, background present.
    fn multistream_alpha_atlas() -> AtlasSegment {
        let mut bits = Bits::default();
        bits.f(0, 3); // atlas_segment_id
        bits.uvlc(4); // mode_idc = MULTISTREAM_ALPHA_ATLAS
        bits.uvlc(100); // msi_width
        bits.uvlc(100); // msi_height
        bits.uvlc(1); // num_atlas_segments_minus_1 = 1 -> 2 segments
        bits.bit(1); // alpha_segments_present_flag
        bits.bit(1); // background_info_present_flag
        bits.f(255, 8); // red
        bits.f(0, 8); // green
        bits.f(0, 8); // blue
        // segment 0 (not last -> alpha flag coded):
        bits.f(0, 5); // input_stream_id
        bits.uvlc(0);
        bits.uvlc(0);
        bits.uvlc(50);
        bits.uvlc(50);
        bits.bit(1); // alpha_segment_flag[0]
        // segment 1 (last -> alpha flag inferred 0):
        bits.f(1, 5);
        bits.uvlc(50);
        bits.uvlc(0);
        bits.uvlc(50);
        bits.uvlc(50);
        bits.bit(0); // signaled_atlas_segment_ids_flag
        parse(bits)
    }

    // ===================================================================================
    // Round-trips (one per mode + label / region-dim form)
    // ===================================================================================

    #[test]
    fn single_atlas_round_trips() {
        round_trip(&single_atlas());
    }

    #[test]
    fn basic_atlas_signaled_ids_round_trips() {
        round_trip(&basic_atlas_signaled());
    }

    #[test]
    fn basic_atlas_unsignaled_no_stream_id_round_trips() {
        round_trip(&basic_atlas_unsignaled_no_stream_id());
    }

    #[test]
    fn enhanced_uniform_single_region_round_trips() {
        round_trip(&enhanced_uniform_single_region());
    }

    #[test]
    fn enhanced_explicit_mapping_round_trips() {
        round_trip(&enhanced_explicit_mapping());
    }

    #[test]
    fn multistream_atlas_round_trips() {
        round_trip(&multistream_atlas());
    }

    #[test]
    fn multistream_alpha_atlas_round_trips() {
        round_trip(&multistream_alpha_atlas());
    }

    #[test]
    fn signaled_label_ids_preserved_verbatim() {
        // §6.9.2: signaled ats_atlas_segment_id values are descriptive id assignments with no
        // conformance requirement; arbitrary values round-trip verbatim.
        let mut atlas = basic_atlas_signaled();
        let AtlasModeInfo::Basic(_) = &atlas.mode_info else {
            panic!("expected basic mode info");
        };
        atlas.label.segment_ids = vec![200, 7];
        round_trip(&atlas);
    }


    #[test]
    fn rejects_unaligned_writer() {
        let mut writer = BitWriter::new();
        writer.write_bit(1).unwrap();
        let err = write_atlas_segment(&mut writer, &single_atlas()).unwrap_err();
        assert!(matches!(err, WriteError::WriterNotByteAligned));
        assert_eq!(writer.bit_len(), 1, "unaligned reject left the writer untouched");
    }

    #[test]
    fn rejects_mode_vs_mode_info_mismatch() {
        let basic = basic_atlas_signaled();
        let mut atlas = single_atlas();
        atlas.mode_info = basic.mode_info;
        reject(&atlas, "mode_info_variant");
    }

    #[test]
    fn rejects_num_segments_disagreeing_with_derivation() {
        let mut atlas = basic_atlas_signaled();
        atlas.num_segments = 3; // mode body derives 2
        reject(&atlas, "num_segments");
    }

    #[test]
    fn rejects_basic_segment_count_vs_len_mismatch() {
        let mut atlas = basic_atlas_signaled();
        let AtlasModeInfo::Basic(basic) = &mut atlas.mode_info else {
            panic!("expected basic mode info");
        };
        basic.segments.pop(); // now 1 segment but num_atlas_segments_minus_1 still 1
        reject(&atlas, "segment_count");
    }

    #[test]
    fn rejects_basic_stream_id_gate_present_without_flag() {
        let mut atlas = basic_atlas_unsignaled_no_stream_id();
        let AtlasModeInfo::Basic(basic) = &mut atlas.mode_info else {
            panic!("expected basic mode info");
        };
        assert!(!basic.stream_id_present);
        basic.segments[0].input_stream_id = Some(3);
        reject(&atlas, "stream_id_gate");
    }

    #[test]
    fn rejects_basic_stream_id_gate_absent_with_flag() {
        let mut atlas = basic_atlas_signaled();
        let AtlasModeInfo::Basic(basic) = &mut atlas.mode_info else {
            panic!("expected basic mode info");
        };
        assert!(basic.stream_id_present);
        basic.segments[0].input_stream_id = None;
        reject(&atlas, "stream_id_gate");
    }

    #[test]
    fn rejects_region_dimension_out_of_range() {
        let mut atlas = enhanced_explicit_mapping();
        let AtlasModeInfo::Enhanced(enhanced) = &mut atlas.mode_info else {
            panic!("expected enhanced mode info");
        };
        enhanced.region.num_region_columns_minus_1 = 64; // == MAX_ATLAS_COLS
        reject(&atlas, "region_dimension");
    }

    #[test]
    fn rejects_num_regions_in_atlas_disagreeing() {
        let mut atlas = enhanced_explicit_mapping();
        let AtlasModeInfo::Enhanced(enhanced) = &mut atlas.mode_info else {
            panic!("expected enhanced mode info");
        };
        enhanced.region.num_regions_in_atlas = 5; // counts derive 2
        reject(&atlas, "num_regions_in_atlas");
    }

    #[test]
    fn rejects_uniform_flag_with_explicit_lists() {
        let mut atlas = enhanced_uniform_single_region();
        let AtlasModeInfo::Enhanced(enhanced) = &mut atlas.mode_info else {
            panic!("expected enhanced mode info");
        };
        assert!(enhanced.region.uniform_spacing);
        enhanced.region.column_widths_minus_1 = vec![1]; // a list while uniform
        reject(&atlas, "region_uniform_dims");
    }

    #[test]
    fn rejects_explicit_column_list_length_mismatch() {
        let mut atlas = enhanced_explicit_mapping();
        let AtlasModeInfo::Enhanced(enhanced) = &mut atlas.mode_info else {
            panic!("expected enhanced mode info");
        };
        assert!(!enhanced.region.uniform_spacing);
        enhanced.region.column_widths_minus_1.pop(); // now 1, count expects 2
        reject(&atlas, "region_uniform_dims");
    }

    #[test]
    fn rejects_single_region_mapping_with_explicit_segments() {
        let mut atlas = enhanced_uniform_single_region();
        let AtlasModeInfo::Enhanced(enhanced) = &mut atlas.mode_info else {
            panic!("expected enhanced mode info");
        };
        assert!(enhanced.mapping.single_region_per_atlas_segment);
        enhanced.mapping.segments.push(AtlasSegmentRegion {
            top_left_region_column: 0,
            top_left_region_row: 0,
            bottom_right_region_column_off: 0,
            bottom_right_region_row_off: 0,
        });
        reject(&atlas, "single_region_segments");
    }

    #[test]
    fn rejects_explicit_mapping_segment_count_vs_len() {
        let mut atlas = enhanced_explicit_mapping();
        let AtlasModeInfo::Enhanced(enhanced) = &mut atlas.mode_info else {
            panic!("expected enhanced mode info");
        };
        enhanced.mapping.segments.pop(); // now 1, count expects 2
        reject(&atlas, "segment_count");
    }

    #[test]
    fn rejects_multistream_alpha_present_on_non_alpha_mode() {
        let mut atlas = multistream_atlas();
        let AtlasModeInfo::Multistream(msi) = &mut atlas.mode_info else {
            panic!("expected multistream mode info");
        };
        msi.alpha_segments_present = Some(true);
        reject(&atlas, "alpha_segments_gate");
    }

    #[test]
    fn rejects_multistream_last_segment_alpha_flag_set() {
        let mut atlas = multistream_alpha_atlas();
        let AtlasModeInfo::MultistreamAlpha(msi) = &mut atlas.mode_info else {
            panic!("expected multistream-alpha mode info");
        };
        let last = msi.segments.len() - 1;
        msi.segments[last].alpha_segment_flag = true;
        reject(&atlas, "alpha_segments_gate");
    }

    #[test]
    fn rejects_label_segment_count_mismatch() {
        let mut atlas = basic_atlas_signaled();
        atlas.label.segment_ids.pop(); // now 1, numSegments is 2
        reject(&atlas, "label_segment_count");
    }

    #[test]
    fn rejects_unsignaled_label_non_identity_ids() {
        let mut atlas = basic_atlas_unsignaled_no_stream_id();
        assert!(!atlas.label.signaled_atlas_segment_ids);
        atlas.label.segment_ids = vec![9]; // identity would be [0]
        reject(&atlas, "label_unsignaled_ids");
    }
}

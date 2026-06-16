// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

// Round-trip and reject tests for the §5.9 atlas_segment_info_obu() writer. `include!`d into
// `crate::write::atlas_segment` so `super::*` resolves to `write_atlas_segment` and the model
// imports.
//
// Every round-trip starts from a model produced by `parse_atlas_segment` over a hand-built,
// spec-grounded byte payload (so the model is guaranteed parser-producible), then writes it,
// reparses the emitted bytes, and asserts model equality. Reject tests mutate such a model into a
// shape the parser could never produce and assert the typed `NonCanonicalAtlasSegment { what }`
// reject with `bit_len() == 0`.

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::bitio::BitReader;
    use crate::headers::atlas_segment::parse_atlas_segment;
    use crate::span::ByteOffset;

    /// MSB-first bit writer for building atlas payloads, mirroring the parser's own `Bits` test
    /// helper at the bottom of `headers/atlas_segment.rs`.
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

    // ===================================================================================
    // Reject tests (decidable invariants; bit_len() == 0)
    // ===================================================================================

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
        // The parser builds the variant from the mode; a Single mode carrying a Basic body is
        // parser-unproducible.
        let basic = basic_atlas_signaled();
        let mut atlas = single_atlas();
        atlas.mode_info = basic.mode_info;
        reject(&atlas, "mode_info_variant");
    }

    #[test]
    fn rejects_num_segments_disagreeing_with_derivation() {
        // num_segments is derived from the mode body; a stored value that disagrees could not
        // round-trip (a reparse re-derives it).
        let mut atlas = basic_atlas_signaled();
        atlas.num_segments = 3; // mode body derives 2
        reject(&atlas, "num_segments");
    }

    #[test]
    fn rejects_basic_segment_count_vs_len_mismatch() {
        // num_atlas_segments_minus_1 + 1 must equal segments.len().
        let mut atlas = basic_atlas_signaled();
        let AtlasModeInfo::Basic(basic) = &mut atlas.mode_info else {
            panic!("expected basic mode info");
        };
        basic.segments.pop(); // now 1 segment but num_atlas_segments_minus_1 still 1
        reject(&atlas, "segment_count");
    }

    #[test]
    fn rejects_basic_stream_id_gate_present_without_flag() {
        // stream_id_present == false but a segment carries an input_stream_id.
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
        // stream_id_present == true but a segment is missing its input_stream_id.
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
        // §6.9.3.1: num_region_columns_minus_1 >= MAX_ATLAS_COLS is parser-unproducible.
        let mut atlas = enhanced_explicit_mapping();
        let AtlasModeInfo::Enhanced(enhanced) = &mut atlas.mode_info else {
            panic!("expected enhanced mode info");
        };
        enhanced.region.num_region_columns_minus_1 = 64; // == MAX_ATLAS_COLS
        reject(&atlas, "region_dimension");
    }

    #[test]
    fn rejects_num_regions_in_atlas_disagreeing() {
        // NumRegionsInAtlas is derived from the column/row counts.
        let mut atlas = enhanced_explicit_mapping();
        let AtlasModeInfo::Enhanced(enhanced) = &mut atlas.mode_info else {
            panic!("expected enhanced mode info");
        };
        enhanced.region.num_regions_in_atlas = 5; // counts derive 2
        reject(&atlas, "num_regions_in_atlas");
    }

    #[test]
    fn rejects_uniform_flag_with_explicit_lists() {
        // uniform_spacing == true must carry the Some width/height pair and empty explicit lists.
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
        // Non-uniform: column_widths_minus_1.len() must equal num_region_columns_minus_1 + 1.
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
        // single_region_per_atlas_segment leaves `segments` empty (the count is derived).
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
        // num_segments now disagrees with the (still 1) derivation, but the segments-non-empty
        // check fires first in write_region_to_segment_mapping. Keep num_segments consistent so the
        // earlier num_segments guard does not pre-empt this one.
        reject(&atlas, "single_region_segments");
    }

    #[test]
    fn rejects_explicit_mapping_segment_count_vs_len() {
        // Non-single-region mapping: segments.len() must equal num_atlas_segments_minus_1 + 1.
        let mut atlas = enhanced_explicit_mapping();
        let AtlasModeInfo::Enhanced(enhanced) = &mut atlas.mode_info else {
            panic!("expected enhanced mode info");
        };
        enhanced.mapping.segments.pop(); // now 1, count expects 2
        // num_segments is derived from num_atlas_segments_minus_1 (still 1 -> 2), so it still
        // matches; the segments length check is what fires.
        reject(&atlas, "segment_count");
    }

    #[test]
    fn rejects_multistream_alpha_present_on_non_alpha_mode() {
        // alpha_segments_present is Some only for the MULTISTREAM_ALPHA variant.
        let mut atlas = multistream_atlas();
        let AtlasModeInfo::Multistream(msi) = &mut atlas.mode_info else {
            panic!("expected multistream mode info");
        };
        msi.alpha_segments_present = Some(true);
        reject(&atlas, "alpha_segments_gate");
    }

    #[test]
    fn rejects_multistream_last_segment_alpha_flag_set() {
        // §6.9.5: the last alpha segment's flag is inferred 0; a stored true is parser-unproducible.
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
        // label.segment_ids.len() must equal the derived numSegments.
        let mut atlas = basic_atlas_signaled();
        atlas.label.segment_ids.pop(); // now 1, numSegments is 2
        reject(&atlas, "label_segment_count");
    }

    #[test]
    fn rejects_unsignaled_label_non_identity_ids() {
        // An unsignaled label must carry the identity indices (segment_ids[i] == i).
        let mut atlas = basic_atlas_unsignaled_no_stream_id();
        assert!(!atlas.label.signaled_atlas_segment_ids);
        atlas.label.segment_ids = vec![9]; // identity would be [0]
        reject(&atlas, "label_unsignaled_ids");
    }
}

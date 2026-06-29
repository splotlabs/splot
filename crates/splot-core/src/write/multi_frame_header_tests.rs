// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>


#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::bitio::BitReader;
    use crate::hls::parse_multi_frame_header;
    use crate::segment::SegmentFeature;
    use crate::span::ByteOffset;

    use crate::test_bits::Bits;

    /// Parses a hand-built MFH payload into a (guaranteed parser-producible) model.
    fn parse(bytes: &[u8]) -> MultiFrameHeader {
        let mut reader = BitReader::new(bytes, ByteOffset::new(0));
        parse_multi_frame_header(&mut reader).unwrap()
    }

    /// Writes the model and reparses the bytes, asserting the semantic round-trip
    /// `parse(write(mfh)) == mfh`.
    fn round_trip(mfh: &MultiFrameHeader) {
        let mut writer = BitWriter::new();
        write_multi_frame_header(&mut writer, mfh).unwrap();
        let bytes = writer.into_bytes();
        let reparsed = parse(&bytes);
        assert_eq!(&reparsed, mfh, "parse(write(mfh)) != mfh");
    }


    #[test]
    fn minimal_round_trips() {
        let mut bits = Bits::default();
        bits.uvlc(0); // mfh_seq_header_id
        bits.uvlc(0); // mfh_id_minus_1
        bits.bit(0); // mfh_frame_size_present_flag
        bits.bit(0); // mfh_deblocking_filter_update
        bits.bit(0); // mfh_seg_info_present_flag
        let mfh = parse(&bits.into_bytes());
        assert_eq!(mfh.mfh_frame_size, None);
        assert!(!mfh.mfh_deblocking_filter_update);
        assert_eq!(mfh.segment_info, None);
        round_trip(&mfh);
    }

    #[test]
    fn out_of_range_ids_reproduced_verbatim() {
        let mut bits = Bits::default();
        bits.uvlc(16); // mfh_seq_header_id (== MAX_SEQ_NUM, out of range)
        bits.uvlc(16); // mfh_id_minus_1 -> mfhId = 17 (>= MAX_MFH_NUM)
        bits.bit(0);
        bits.bit(0);
        bits.bit(0);
        let mfh = parse(&bits.into_bytes());
        assert!(!mfh.seq_header_id_in_range());
        assert!(!mfh.mfh_id_in_range());
        round_trip(&mfh);
    }

    #[test]
    fn frame_size_small_round_trips() {
        let mut bits = Bits::default();
        bits.uvlc(3); // mfh_seq_header_id
        bits.uvlc(2); // mfh_id_minus_1
        bits.bit(1); // mfh_frame_size_present_flag
        bits.f(3, 4); // mfh_frame_width_bits_minus_1 -> width_bits = 4
        bits.f(3, 4); // mfh_frame_height_bits_minus_1 -> height_bits = 4
        bits.f(15, 4); // mfh_frame_width_minus_1
        bits.f(7, 4); // mfh_frame_height_minus_1
        bits.bit(0); // mfh_deblocking_filter_update
        bits.bit(0); // mfh_seg_info_present_flag
        let mfh = parse(&bits.into_bytes());
        let size = mfh.mfh_frame_size.unwrap();
        assert_eq!(size.width_bits, 4);
        assert_eq!(size.width_minus_1, 15);
        assert_eq!(size.height_minus_1, 7);
        round_trip(&mfh);
    }

    #[test]
    fn frame_size_max_width_height_bits_round_trips() {
        let mut bits = Bits::default();
        bits.uvlc(0);
        bits.uvlc(0);
        bits.bit(1); // mfh_frame_size_present_flag
        bits.f(15, 4); // width_bits_minus_1 -> 16
        bits.f(15, 4); // height_bits_minus_1 -> 16
        bits.f(0xABCD, 16); // mfh_frame_width_minus_1
        bits.f(0xFFFF, 16); // mfh_frame_height_minus_1 (the max 16-bit value)
        bits.bit(0);
        bits.bit(0);
        let mfh = parse(&bits.into_bytes());
        let size = mfh.mfh_frame_size.unwrap();
        assert_eq!(size.width_bits, 16);
        assert_eq!(size.height_bits, 16);
        assert_eq!(size.width_minus_1, 0xABCD);
        assert_eq!(size.height_minus_1, 0xFFFF);
        round_trip(&mfh);
    }

    #[test]
    fn deblocking_update_various_flag_combos_round_trip() {
        for combo in [
            [false, false, false, false],
            [true, false, true, false],
            [false, true, false, true],
            [true, true, true, true],
        ] {
            let mut bits = Bits::default();
            bits.uvlc(0);
            bits.uvlc(0);
            bits.bit(0); // mfh_frame_size_present_flag
            bits.bit(1); // mfh_deblocking_filter_update
            for &flag in &combo {
                bits.bit(u8::from(flag));
            }
            bits.bit(0); // mfh_seg_info_present_flag
            let mfh = parse(&bits.into_bytes());
            assert!(mfh.mfh_deblocking_filter_update);
            assert_eq!(mfh.mfh_apply_deblocking_filter, combo);
            round_trip(&mfh);
        }
    }

    #[test]
    fn seg_info_present_ext_false_uses_eight_segments() {
        let mut bits = Bits::default();
        bits.uvlc(0);
        bits.uvlc(0);
        bits.bit(0); // mfh_frame_size_present_flag
        bits.bit(0); // mfh_deblocking_filter_update
        bits.bit(1); // mfh_seg_info_present_flag
        bits.bit(0); // mfh_ext_seg_flag -> seg_info(8)
        bits.bit(1); // mfh_allow_seg_info_change
        for _ in 0..(8 * 3) {
            bits.bit(0); // seg_info(8): all features disabled
        }
        let mfh = parse(&bits.into_bytes());
        assert_eq!(mfh.mfh_ext_seg_flag, Some(false));
        assert_eq!(mfh.mfh_allow_seg_info_change, Some(true));
        assert_eq!(mfh.segment_info.unwrap().num_segments, 8);
        round_trip(&mfh);
    }

    #[test]
    fn seg_info_present_ext_true_uses_sixteen_segments() {
        let mut bits = Bits::default();
        bits.uvlc(0);
        bits.uvlc(0);
        bits.bit(0); // mfh_frame_size_present_flag
        bits.bit(0); // mfh_deblocking_filter_update
        bits.bit(1); // mfh_seg_info_present_flag
        bits.bit(1); // mfh_ext_seg_flag -> seg_info(16)
        bits.bit(0); // mfh_allow_seg_info_change
        for _ in 0..(16 * 3) {
            bits.bit(0); // seg_info(16): all features disabled
        }
        let mfh = parse(&bits.into_bytes());
        assert_eq!(mfh.mfh_ext_seg_flag, Some(true));
        assert_eq!(mfh.segment_info.unwrap().num_segments, 16);
        round_trip(&mfh);
    }

    #[test]
    fn seg_info_with_enabled_feature_round_trips() {
        let mut bits = Bits::default();
        bits.uvlc(0);
        bits.uvlc(0);
        bits.bit(0); // mfh_frame_size_present_flag
        bits.bit(0); // mfh_deblocking_filter_update
        bits.bit(1); // mfh_seg_info_present_flag
        bits.bit(0); // mfh_ext_seg_flag -> seg_info(8)
        bits.bit(0); // mfh_allow_seg_info_change
        bits.bit(1); // feature_enabled[0][0]
        bits.f(100, 10); // su(10) value 100 (in the +/-351 clip window)
        bits.bit(0); // [0][1]
        bits.bit(0); // [0][2]
        for _ in 0..(7 * 3) {
            bits.bit(0);
        }
        let mfh = parse(&bits.into_bytes());
        let info = mfh.segment_info.as_ref().unwrap();
        assert!(info.features[0][0].enabled);
        assert_eq!(info.features[0][0].data, 100);
        round_trip(&mfh);
    }

    #[test]
    fn everything_present_round_trips() {
        let mut bits = Bits::default();
        bits.uvlc(5);
        bits.uvlc(4);
        bits.bit(1); // mfh_frame_size_present_flag
        bits.f(7, 4); // width_bits_minus_1 -> 8
        bits.f(7, 4); // height_bits_minus_1 -> 8
        bits.f(200, 8); // mfh_frame_width_minus_1
        bits.f(120, 8); // mfh_frame_height_minus_1
        bits.bit(1); // mfh_deblocking_filter_update
        bits.bit(1);
        bits.bit(0);
        bits.bit(1);
        bits.bit(1);
        bits.bit(1); // mfh_seg_info_present_flag
        bits.bit(1); // mfh_ext_seg_flag -> seg_info(16)
        bits.bit(1); // mfh_allow_seg_info_change
        for _ in 0..(16 * 3) {
            bits.bit(0);
        }
        let mfh = parse(&bits.into_bytes());
        assert!(mfh.mfh_frame_size.is_some());
        assert!(mfh.mfh_deblocking_filter_update);
        assert!(mfh.segment_info.is_some());
        round_trip(&mfh);
    }


    /// A minimal all-default parsed model to mutate into parser-unproducible states.
    fn minimal_model() -> MultiFrameHeader {
        let mut bits = Bits::default();
        bits.uvlc(0);
        bits.uvlc(0);
        bits.bit(0);
        bits.bit(0);
        bits.bit(0);
        parse(&bits.into_bytes())
    }

    #[test]
    fn apply_deblocking_non_false_without_update_rejects() {
        let mut mfh = minimal_model();
        mfh.mfh_deblocking_filter_update = false;
        mfh.mfh_apply_deblocking_filter = [false, false, true, false];
        let mut writer = BitWriter::new();
        let err = write_multi_frame_header(&mut writer, &mfh).unwrap_err();
        assert!(
            matches!(err, WriteError::NonCanonicalMultiFrameHeader { what }
                if what == "deblocking_apply_forced_false"),
            "expected deblocking_apply_forced_false, got {err:?}"
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn seg_info_options_disagree_with_flag_rejects() {
        let mut mfh = minimal_model();
        mfh.mfh_seg_info_present_flag = false;
        mfh.mfh_ext_seg_flag = Some(true);
        let mut w1 = BitWriter::new();
        let e1 = write_multi_frame_header(&mut w1, &mfh).unwrap_err();
        assert!(
            matches!(e1, WriteError::NonCanonicalMultiFrameHeader { what }
                if what == "seg_info_present_flag"),
            "expected seg_info_present_flag (flag clear, ext_seg Some), got {e1:?}"
        );
        assert_eq!(w1.bit_len(), 0);

        let mut mfh2 = minimal_model();
        mfh2.mfh_seg_info_present_flag = true; // but ext_seg/allow_change/segment_info are None
        let mut w2 = BitWriter::new();
        let e2 = write_multi_frame_header(&mut w2, &mfh2).unwrap_err();
        assert!(
            matches!(e2, WriteError::NonCanonicalMultiFrameHeader { what }
                if what == "seg_info_present_flag"),
            "expected seg_info_present_flag (flag set, Options None), got {e2:?}"
        );
        assert_eq!(w2.bit_len(), 0);
    }

    #[test]
    fn out_of_range_width_bits_rejects() {
        let mut bits = Bits::default();
        bits.uvlc(0);
        bits.uvlc(0);
        bits.bit(1); // mfh_frame_size_present_flag
        bits.f(3, 4); // width_bits_minus_1 -> width_bits = 4
        bits.f(3, 4); // height_bits_minus_1 -> height_bits = 4
        bits.f(0, 4);
        bits.f(0, 4);
        bits.bit(0);
        bits.bit(0);
        let mut mfh = parse(&bits.into_bytes());
        mfh.mfh_frame_size = Some(MfhFrameSize {
            width_bits: 0, // out of the 1..=16 range
            height_bits: 4,
            width_minus_1: 0,
            height_minus_1: 0,
        });
        let mut writer = BitWriter::new();
        let err = write_multi_frame_header(&mut writer, &mfh).unwrap_err();
        assert!(
            matches!(err, WriteError::NonCanonicalMultiFrameHeader { what }
                if what == "frame_width_bits"),
            "expected frame_width_bits, got {err:?}"
        );
        assert_eq!(writer.bit_len(), 0);

        mfh.mfh_frame_size = Some(MfhFrameSize {
            width_bits: 4,
            height_bits: 17,
            width_minus_1: 0,
            height_minus_1: 0,
        });
        let mut w2 = BitWriter::new();
        let e2 = write_multi_frame_header(&mut w2, &mfh).unwrap_err();
        assert!(
            matches!(e2, WriteError::NonCanonicalMultiFrameHeader { what }
                if what == "frame_height_bits"),
            "expected frame_height_bits, got {e2:?}"
        );
        assert_eq!(w2.bit_len(), 0);
    }

    #[test]
    fn nested_seg_info_reject_propagates() {
        let mut bits = Bits::default();
        bits.uvlc(0);
        bits.uvlc(0);
        bits.bit(0);
        bits.bit(0);
        bits.bit(1); // mfh_seg_info_present_flag
        bits.bit(0); // mfh_ext_seg_flag -> seg_info(8)
        bits.bit(0); // mfh_allow_seg_info_change
        for _ in 0..(8 * 3) {
            bits.bit(0);
        }
        let mut mfh = parse(&bits.into_bytes());
        let info = mfh.segment_info.as_mut().unwrap();
        info.features[0][0] = SegmentFeature {
            enabled: false,
            data: 5, // disabled must be 0
        };
        let mut writer = BitWriter::new();
        let err = write_multi_frame_header(&mut writer, &mfh).unwrap_err();
        assert!(
            matches!(err, WriteError::NonCanonicalSequenceValue { what }
                if what == "seg_info_disabled_data"),
            "expected propagated seg_info_disabled_data, got {err:?}"
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn unaligned_writer_rejects() {
        let mut writer = BitWriter::new();
        writer.write_bit(1).unwrap();
        let mfh = minimal_model();
        let err = write_multi_frame_header(&mut writer, &mfh).unwrap_err();
        assert!(matches!(err, WriteError::WriterNotByteAligned));
        assert_eq!(writer.bit_len(), 1);
    }
}

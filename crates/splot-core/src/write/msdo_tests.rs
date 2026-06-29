// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>


#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::bitio::BitReader;
    use crate::headers::sequence::ProfileIdc;
    use crate::hls::parse_msdo;
    use crate::span::ByteOffset;

    fn sub(
        sub_xlayer_id: u8,
        sub_stream_max_profile: u8,
        sub_stream_max_level: u8,
        sub_stream_max_tier: u8,
    ) -> SubStreamConfig {
        SubStreamConfig {
            sub_xlayer_id,
            sub_stream_max_profile,
            sub_stream_max_level,
            sub_stream_max_tier,
        }
    }

    /// Builds a canonical MSDO model with `subs` used entries (the rest of the fixed array zero), the
    /// derived `sub_stream_count`, and the chosen allocation form.
    fn msdo(even: bool, large_picture_idc: Option<u8>, subs: &[SubStreamConfig]) -> MultistreamDecoderOperation {
        let mut sub_streams = [sub(0, 0, 0, 0); 9];
        sub_streams[..subs.len()].copy_from_slice(subs);
        MultistreamDecoderOperation {
            num_streams_minus_2: (subs.len() - 2) as u8,
            multistream_profile_idc: ProfileIdc::from_bits(5),
            multistream_level_idx: 10,
            multistream_tier: 1,
            multistream_even_allocation_flag: even,
            multistream_large_picture_idc: large_picture_idc,
            sub_stream_count: subs.len() as u8,
            sub_streams,
            multistream_doh_constraint_flag: true,
        }
    }

    fn round_trip(model: &MultistreamDecoderOperation) {
        let mut writer = BitWriter::new();
        write_msdo(&mut writer, model).unwrap();
        let bytes = writer.into_bytes();
        let mut reader = BitReader::new(&bytes, ByteOffset::new(0));
        let reparsed = parse_msdo(&mut reader).unwrap();
        assert_eq!(&reparsed, model);
    }

    #[test]
    fn even_allocation_round_trips() {
        round_trip(&msdo(true, None, &[sub(1, 4, 3, 0), sub(2, 3, 4, 1)]));
    }

    #[test]
    fn uneven_allocation_round_trips() {
        let subs: Vec<SubStreamConfig> = (0..9).map(|i| sub(i, 31, 30, i & 1)).collect();
        round_trip(&msdo(false, Some(5), &subs));
    }

    #[test]
    fn large_picture_idc_flag_mismatch_rejects() {
        let model = msdo(true, Some(3), &[sub(1, 4, 3, 0), sub(2, 3, 4, 1)]);
        let mut writer = BitWriter::new();
        let err = write_msdo(&mut writer, &model).unwrap_err();
        assert!(
            matches!(err, WriteError::NonCanonicalMsdo { what } if what == "large_picture_idc_flag"),
            "expected large_picture_idc_flag, got {err:?}"
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn sub_stream_count_mismatch_rejects() {
        let mut model = msdo(true, None, &[sub(1, 4, 3, 0), sub(2, 3, 4, 1)]);
        model.sub_stream_count = 3; // disagrees with num_streams_minus_2 (0) + 2 == 2
        let mut writer = BitWriter::new();
        let err = write_msdo(&mut writer, &model).unwrap_err();
        assert!(
            matches!(err, WriteError::NonCanonicalMsdo { what } if what == "sub_stream_count"),
            "expected sub_stream_count, got {err:?}"
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn count_overflowing_the_array_rejects_without_panicking() {
        let mut model = msdo(true, None, &[sub(1, 4, 3, 0), sub(2, 3, 4, 1)]);
        model.num_streams_minus_2 = 200;
        model.sub_stream_count = 202;
        let mut writer = BitWriter::new();
        let err = write_msdo(&mut writer, &model).unwrap_err();
        assert!(
            matches!(err, WriteError::NonCanonicalMsdo { what } if what == "sub_stream_count"),
            "expected sub_stream_count, got {err:?}"
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn non_zero_unused_sub_stream_rejects() {
        let mut model = msdo(true, None, &[sub(1, 4, 3, 0), sub(2, 3, 4, 1)]);
        model.sub_streams[2] = sub(7, 0, 0, 0); // a used-looking value in an unused slot
        let mut writer = BitWriter::new();
        let err = write_msdo(&mut writer, &model).unwrap_err();
        assert!(
            matches!(err, WriteError::NonCanonicalMsdo { what } if what == "unused_sub_stream"),
            "expected unused_sub_stream, got {err:?}"
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn out_of_field_sub_stream_value_rejects() {
        let model = msdo(true, None, &[sub(1, 32, 3, 0), sub(2, 3, 4, 1)]);
        let mut writer = BitWriter::new();
        let err = write_msdo(&mut writer, &model).unwrap_err();
        assert!(
            matches!(err, WriteError::ValueTooWide { width_bits: 5, .. }),
            "expected a 5-bit ValueTooWide, got {err:?}"
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn unaligned_writer_rejects() {
        let mut writer = BitWriter::new();
        writer.write_bit(1).unwrap();
        let model = msdo(true, None, &[sub(1, 4, 3, 0), sub(2, 3, 4, 1)]);
        let err = write_msdo(&mut writer, &model).unwrap_err();
        assert!(matches!(err, WriteError::WriterNotByteAligned));
        assert_eq!(writer.bit_len(), 1);
    }
}

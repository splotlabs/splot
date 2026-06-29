// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>


#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::bitio::BitReader;
    use crate::headers::buffer_removal_timing::{BufferRemovalOpTiming, parse_buffer_removal_timing};
    use crate::span::ByteOffset;

    /// Writes a BRT body and reparses it, asserting model equality (the body is variable-width; the
    /// parser reads exactly the body bits and ignores the byte-padding `into_bytes` adds).
    fn round_trip(brt: &BufferRemovalTiming) {
        let mut writer = BitWriter::new();
        write_buffer_removal_timing(&mut writer, brt).unwrap();
        let bytes = writer.into_bytes();
        let mut reader = BitReader::new(&bytes, ByteOffset::new(0));
        let reparsed = parse_buffer_removal_timing(&mut reader).unwrap();
        assert_eq!(&reparsed, brt);
    }

    /// A per-operating-point entry with `br_time_op` present iff `Some`, `index` = `i` (canonical).
    fn op(index: u8, br_time_op: Option<u32>) -> BufferRemovalOpTiming {
        BufferRemovalOpTiming {
            index,
            decoder_model_present: br_time_op.is_some(),
            br_time_op,
        }
    }

    #[test]
    fn extended_layer_round_trips() {
        round_trip(&BufferRemovalTiming::ExtendedLayer { br_time: 0 });
        round_trip(&BufferRemovalTiming::ExtendedLayer { br_time: 42 });
        round_trip(&BufferRemovalTiming::ExtendedLayer { br_time: 511 });
    }

    #[test]
    fn ops_dependent_round_trips() {
        round_trip(&BufferRemovalTiming::OperatingPointSet {
            br_ops_id: 3,
            br_ops_cnt: 2,
            op_times: vec![op(0, Some(7)), op(1, None)],
        });
        round_trip(&BufferRemovalTiming::OperatingPointSet {
            br_ops_id: 15,
            br_ops_cnt: 0,
            op_times: vec![],
        });
    }

    #[test]
    fn op_count_mismatch_rejects_without_writing() {
        let brt = BufferRemovalTiming::OperatingPointSet {
            br_ops_id: 0,
            br_ops_cnt: 2,
            op_times: vec![op(0, None)],
        };
        let mut writer = BitWriter::new();
        let err = write_buffer_removal_timing(&mut writer, &brt).unwrap_err();
        assert!(
            matches!(err, WriteError::NonCanonicalBufferRemovalTiming { what } if what == "op_count"),
            "expected op_count, got {err:?}"
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn non_canonical_op_index_rejects_without_writing() {
        let brt = BufferRemovalTiming::OperatingPointSet {
            br_ops_id: 0,
            br_ops_cnt: 1,
            op_times: vec![op(5, None)],
        };
        let mut writer = BitWriter::new();
        let err = write_buffer_removal_timing(&mut writer, &brt).unwrap_err();
        assert!(
            matches!(err, WriteError::NonCanonicalBufferRemovalTiming { what } if what == "op_index"),
            "expected op_index, got {err:?}"
        );
        assert_eq!(writer.bit_len(), 0, "scratch reject left bits in the caller's writer");
    }

    #[test]
    fn gated_br_time_op_mismatch_rejects() {
        // decoder_model_present set but br_time_op absent — the parser ties them.
        let brt = BufferRemovalTiming::OperatingPointSet {
            br_ops_id: 0,
            br_ops_cnt: 1,
            op_times: vec![BufferRemovalOpTiming {
                index: 0,
                decoder_model_present: true,
                br_time_op: None,
            }],
        };
        let mut writer = BitWriter::new();
        let err = write_buffer_removal_timing(&mut writer, &brt).unwrap_err();
        assert!(
            matches!(err, WriteError::NonCanonicalBufferRemovalTiming { what } if what == "op_decoder_model_flag"),
            "expected op_decoder_model_flag, got {err:?}"
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn out_of_range_br_time_rejects() {
        let brt = BufferRemovalTiming::ExtendedLayer { br_time: 512 };
        let mut writer = BitWriter::new();
        let err = write_buffer_removal_timing(&mut writer, &brt).unwrap_err();
        assert!(
            matches!(err, WriteError::ValueOutOfRange { descriptor: "rg", .. }),
            "expected rg ValueOutOfRange, got {err:?}"
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn out_of_field_br_ops_id_rejects() {
        let brt = BufferRemovalTiming::OperatingPointSet {
            br_ops_id: 16,
            br_ops_cnt: 0,
            op_times: vec![],
        };
        let mut writer = BitWriter::new();
        let err = write_buffer_removal_timing(&mut writer, &brt).unwrap_err();
        assert!(
            matches!(err, WriteError::ValueTooWide { width_bits: 4, .. }),
            "expected a 4-bit ValueTooWide, got {err:?}"
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn unaligned_writer_rejects() {
        let mut writer = BitWriter::new();
        writer.write_bit(1).unwrap(); // leave the writer mid-byte
        let err =
            write_buffer_removal_timing(&mut writer, &BufferRemovalTiming::ExtendedLayer { br_time: 0 })
                .unwrap_err();
        assert!(matches!(err, WriteError::WriterNotByteAligned));
        assert_eq!(writer.bit_len(), 1, "only the pre-existing stray bit remains");
    }
}

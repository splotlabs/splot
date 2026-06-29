// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>


#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::annexb::parse_annex_b_obus;
    use crate::headers::metadata::{
        MetadataGroupObu, MetadataGroupUnit, MetadataPayload, MetadataShortObu, MetadataType,
        MetadataUnit, MetadataUnknownRaw,
    };
    use crate::headers::padding::PaddingObu;
    use crate::obu::{ParsedObu, PayloadStatus, read_obu_header_from_slice};
    use crate::span::ByteOffset;

    /// Parses one Annex B OBU and round-trips its parsed payload through the harness.
    fn roundtrip_first(bytes: &[u8]) -> RoundtripOutcome {
        let obus = parse_annex_b_obus(bytes).expect("fixture parses as Annex B");
        let env = obus.first().expect("one OBU");
        match env.payload_status().expect("payload parses") {
            PayloadStatus::Parsed(parsed) => roundtrip_obu(&env.header, env.payload, &parsed),
            other => panic!("expected a parsed payload, got {other:?}"),
        }
    }


    #[test]
    fn temporal_delimiter_round_trips() {
        assert_eq!(roundtrip_first(&[0x01, 0x08]), RoundtripOutcome::RoundTripped);
    }

    #[test]
    fn padding_round_trips_with_recovered_run() {
        assert_eq!(
            roundtrip_first(&[0x04, 0x64, 0xDE, 0xAD, 0x80]),
            RoundtripOutcome::RoundTripped
        );
    }

    #[test]
    fn cancelled_metadata_short_round_trips() {
        assert_eq!(
            roundtrip_first(&[0x04, 0x20, 0x08, 0x04, 0x80]),
            RoundtripOutcome::RoundTripped
        );
    }

    #[test]
    fn local_multi_unit_metadata_group_round_trips() {
        assert_eq!(
            roundtrip_first(&[0x08, 0x24, 0x00, 0x01, 0x04, 0x01, 0x04, 0x01, 0x80]),
            RoundtripOutcome::RoundTripped
        );
    }

    #[test]
    fn global_xlayer_metadata_group_round_trips() {
        assert_eq!(
            roundtrip_first(&[0x07, 0xA4, 0x1F, 0x00, 0x00, 0x04, 0x01, 0x80]),
            RoundtripOutcome::RoundTripped
        );
    }

    #[test]
    fn film_grain_round_trips() {
        assert_eq!(
            roundtrip_first(&[0x03, 0x5C, 0x00, 0xC0]),
            RoundtripOutcome::RoundTripped
        );
    }

    #[test]
    fn quantization_matrix_round_trips() {
        assert_eq!(
            roundtrip_first(&[0x04, 0x58, 0x00, 0x00, 0x80]),
            RoundtripOutcome::RoundTripped
        );
    }

    #[test]
    fn header_payload_mismatch_is_a_failure_not_a_panic() {
        let header = read_obu_header_from_slice(&[0x04], ByteOffset::new(0)).unwrap();
        let outcome = roundtrip_obu(&header, &[], &ParsedObu::TemporalDelimiter);
        assert_eq!(
            outcome,
            RoundtripOutcome::Failed {
                reason: "write_rejected"
            },
            "expected Failed(write_rejected), got {outcome:?}"
        );
    }


    #[test]
    fn recover_padding_returns_the_real_run() {
        let parsed = ParsedObu::Padding(PaddingObu {
            padding_len: 2,
            trailing_len: 1,
        });
        let got = recover_roundtrip_passthrough(&[0xDE, 0xAD, 0x80], &parsed).unwrap();
        assert_eq!(got, vec![0xDE, 0xAD], "the obu_padding_byte run, byte-exact");
    }

    #[test]
    fn recover_padding_run_longer_than_payload_is_rejected() {
        let parsed = ParsedObu::Padding(PaddingObu {
            padding_len: 5,
            trailing_len: 1,
        });
        assert!(
            recover_roundtrip_passthrough(&[0x00, 0x00, 0x80], &parsed).is_err(),
            "a run longer than the payload cannot be recovered"
        );
    }

    #[test]
    fn recover_metadata_blob_is_a_zero_fill_of_the_modeled_length() {
        let parsed = ParsedObu::MetadataShort(Box::new(MetadataShortObu {
            metadata_is_suffix: false,
            muh_layer_idc: 0,
            muh_cancel_flag: false,
            muh_persistence_idc: 0,
            metadata_type: MetadataType::Reserved(0),
            metadata_type_leb128_bytes: 1,
            unit: Some(MetadataUnit {
                metadata_type: MetadataType::Reserved(0),
                payload_size: 3,
                payload: MetadataPayload::UnknownRaw(MetadataUnknownRaw { raw_len: 3 }),
            }),
        }));
        let got = recover_roundtrip_passthrough(&[0u8; 4], &parsed).unwrap();
        assert_eq!(got, vec![0u8, 0, 0], "zero-fill of the modeled blob length");
        assert!(
            recover_roundtrip_passthrough(&[0u8; 2], &parsed).is_err(),
            "a blob longer than the payload cannot be recovered"
        );
    }

    #[test]
    fn recover_metadata_group_overflowing_unit_lengths_reject_without_panicking() {
        let unit = |raw_len: usize| MetadataGroupUnit {
            metadata_type: MetadataType::Reserved(0),
            muh_header_size: 3,
            muh_cancel_flag: false,
            muh_payload_size: Some(0),
            muh_layer_idc: Some(0),
            muh_persistence_idc: Some(0),
            muh_priority: Some(0),
            muh_reserved_zero_2bits: Some(0),
            muh_xlayer_map: None,
            muh_mlayer_maps: vec![],
            header_extension_len: 0,
            unit: Some(MetadataUnit {
                metadata_type: MetadataType::Reserved(0),
                payload_size: raw_len,
                payload: MetadataPayload::UnknownRaw(MetadataUnknownRaw { raw_len }),
            }),
        };
        let parsed = ParsedObu::MetadataGroup(Box::new(MetadataGroupObu {
            metadata_is_suffix: false,
            metadata_necessity_idc: 0,
            metadata_application_id: 0,
            units: vec![unit(usize::MAX), unit(1)],
        }));
        assert!(
            recover_roundtrip_passthrough(&[], &parsed).is_err(),
            "an overflowing per-unit blob-length sum must reject, not panic"
        );
    }

    #[test]
    fn recover_temporal_delimiter_is_empty() {
        let got = recover_roundtrip_passthrough(&[], &ParsedObu::TemporalDelimiter).unwrap();
        assert!(got.is_empty());
    }
}

// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

// `include!`d into `crate::write::roundtrip` so `super::*` resolves to its harness functions.

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

    // ===================================================================================
    // roundtrip_obu end-to-end (parse a real OBU, then write -> reparse -> compare)
    // ===================================================================================

    #[test]
    fn temporal_delimiter_round_trips() {
        // leb128(1) + TD header (obu_type 2 -> 0x08), empty payload.
        assert_eq!(roundtrip_first(&[0x01, 0x08]), RoundtripOutcome::RoundTripped);
    }

    #[test]
    fn padding_round_trips_with_recovered_run() {
        // leb128(4) + padding header (obu_type 25 -> 0x64) + 2 obu_padding_byte + 1 trailing byte.
        // The harness must recover the real [0xDE, 0xAD] run (its values drive the parser's split).
        assert_eq!(
            roundtrip_first(&[0x04, 0x64, 0xDE, 0xAD, 0x80]),
            RoundtripOutcome::RoundTripped
        );
    }

    #[test]
    fn cancelled_metadata_short_round_trips() {
        // leb128(4) + metadata-short header (obu_type 8 -> 0x20) + byte0 (cancel bit 0x08) +
        // metadata_type leb (0x04) + trailing 0x80. A cancelled unit needs an empty passthrough.
        assert_eq!(
            roundtrip_first(&[0x04, 0x20, 0x08, 0x04, 0x80]),
            RoundtripOutcome::RoundTripped
        );
    }

    #[test]
    fn local_multi_unit_metadata_group_round_trips() {
        // leb128(8) + metadata-group header (obu_type 9 -> 0x24, local) + group header 0x00 +
        // metadata_unit_cnt_minus_1 0x01 (2 units) + two cancelled units [type 0x04, cancel 0x01] +
        // trailing 0x80. Exercises the harness's per-unit flat-passthrough split (both empty).
        assert_eq!(
            roundtrip_first(&[0x08, 0x24, 0x00, 0x01, 0x04, 0x01, 0x04, 0x01, 0x80]),
            RoundtripOutcome::RoundTripped
        );
    }

    #[test]
    fn global_xlayer_metadata_group_round_trips() {
        // leb128(7) + extension metadata-group header (0xA4 0x1F: obu_type 9, obu_xlayer_id 31) +
        // group header + cnt 0 (one cancelled unit) + trailing. The §6.16.3 global layer-map branch
        // only round-trips because the harness writes via write_complete_obu (threading the header
        // xlayer), not write_obu_payload (which defaults to the local branch).
        assert_eq!(
            roundtrip_first(&[0x07, 0xA4, 0x1F, 0x00, 0x00, 0x04, 0x01, 0x80]),
            RoundtripOutcome::RoundTripped
        );
    }

    #[test]
    fn film_grain_round_trips() {
        // leb128(3) + film-grain header (obu_type 23 -> 0x5C) + minimal 2-byte payload
        // (fgm_update_flags 0, fgm_chroma_idc uvlc 0, no models) + trailing. §5.14 / §5.18.10.2 now
        // has a body writer, so the harness round-trips it (the model is lossy versus the wire, but
        // the semantic round-trip holds).
        assert_eq!(
            roundtrip_first(&[0x03, 0x5C, 0x00, 0xC0]),
            RoundtripOutcome::RoundTripped
        );
    }

    #[test]
    fn quantization_matrix_round_trips() {
        // leb128(4) + quantization-matrix header (obu_type 22 -> 0x58) + a minimal parsable payload
        // (qm_bit_map(15) == 0 reset + chroma flag(1) + trailing). §5.13 now has a body writer (the
        // last OBU type), so the harness round-trips it; no OBU type is Unwritable anymore.
        assert_eq!(
            roundtrip_first(&[0x04, 0x58, 0x00, 0x00, 0x80]),
            RoundtripOutcome::RoundTripped
        );
    }

    #[test]
    fn header_payload_mismatch_is_a_failure_not_a_panic() {
        // A sequence-header header (0x04) paired with a temporal-delimiter payload is a pair the
        // dispatch rejects (ObuTypePayloadMismatch); the harness reports Failed, never panics.
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

    // ===================================================================================
    // recover_roundtrip_passthrough
    // ===================================================================================

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
        // A non-cancel UnknownRaw unit declares a 3-byte blob; the values are not modeled, so the
        // harness returns 3 zero bytes (semantic round-trip), bounded by payload.len().
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
        // Bounded by the source payload: a blob longer than the payload is rejected (OOM guard).
        assert!(
            recover_roundtrip_passthrough(&[0u8; 2], &parsed).is_err(),
            "a blob longer than the payload cannot be recovered"
        );
    }

    #[test]
    fn recover_metadata_group_overflowing_unit_lengths_reject_without_panicking() {
        // Two non-cancel UnknownRaw units whose modeled blob lengths sum past usize::MAX must
        // REJECT (not panic under overflow-checks) — the checked fold runs before any allocation.
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

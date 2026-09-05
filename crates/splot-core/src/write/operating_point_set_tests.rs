// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::bitio::BitReader;
    use crate::headers::operating_point_set::parse_operating_point_set;
    use crate::headers::sequence::ProfileIdc;
    use crate::span::ByteOffset;
    use crate::types::{ExtendedLayerId, GLOBAL_XLAYER_ID};

    /// Writes an OPS body and reparses it with the matching `xlayer_id`, asserting model equality.
    /// The body is variable-width; the parser reads exactly the body bits and ignores the
    /// byte-padding `into_bytes` adds after a byte-aligned payload.
    fn round_trip(ops: &OperatingPointSet) {
        let mut writer = BitWriter::new();
        write_operating_point_set(&mut writer, ops, ops.xlayer_id).unwrap();
        let bytes = writer.into_bytes();
        let mut reader = BitReader::new(&bytes, ByteOffset::new(0));
        let reparsed = parse_operating_point_set(&mut reader, ops.xlayer_id).unwrap();
        assert_eq!(&reparsed, ops);
    }

    /// A reset (`ops_cnt == 0`) OPS for the given scope, with every gated field absent.
    fn reset_ops(xlayer_id: ExtendedLayerId, reset_flag: bool, ops_id: u8) -> OperatingPointSet {
        OperatingPointSet {
            xlayer_id,
            reset_flag,
            ops_id,
            ops_cnt: 0,
            priority: None,
            intent: None,
            intent_present: false,
            ptl_present: false,
            color_info_present: false,
            mlayer_info_idc: None,
            local_reserved_2bits: None,
            payloads: vec![],
        }
    }

    /// Makes an OPS fixture canonical by setting every payload's
    /// `declared_size_bytes`/`computed_size_bytes` to the real `opsBytes`, derived by replaying the
    /// payload body (after `ops_data_size`, through the closing `byte_alignment()`) with the same
    /// private body writers the production code uses, in parse order. The writer re-derives the same
    /// `opsBytes`, so a canonical fixture round-trips; a deliberate-mismatch reject test does not run
    /// through this helper.
    fn canonicalize_sizes(ops: &mut OperatingPointSet) {
        let snapshot = ops.clone();
        for payload in &mut ops.payloads {
            let size = measure_payload_body(&snapshot, payload);
            payload.declared_size_bytes = size;
            payload.computed_size_bytes = size;
        }
    }

    /// Replays a payload body in parse order (§ 5.11): `ops_op_intent`, the top-level PTL
    /// (`ops_aggregate_info()` global / `ops_seq_profile_tier_level_info()` local, before color),
    /// `ops_color_info()`, the decoder-model and initial-display-delay flags + optionals, then the
    /// xlayer section, then `byte_alignment()`. Returns the byte length, i.e. `opsBytes`.
    fn measure_payload_body(ops: &OperatingPointSet, payload: &OperatingPointPayload) -> u32 {
        let is_global = ops.xlayer_id.is_global();
        let mut body = BitWriter::new();
        if ops.intent_present {
            body.write_bits_u8(payload.op_intent.unwrap(), 7).unwrap();
        }
        if is_global {
            if let Some(agg) = &payload.aggregate_info {
                write_ops_aggregate_info(&mut body, agg).unwrap();
            }
        } else if ops.ptl_present {
            write_ops_seq_profile_tier_level_info(
                &mut body,
                payload.xlayer_entries[0].ptl_info.as_ref().unwrap(),
            )
            .unwrap();
        }
        if let Some(color) = &payload.color_info {
            write_ops_color_info(&mut body, color).unwrap();
        }
        body.write_flag(payload.decoder_model_info.is_some())
            .unwrap();
        if let Some(dm) = &payload.decoder_model_info {
            write_ops_decoder_model_info(&mut body, dm).unwrap();
        }
        body.write_flag(payload.initial_display_delay_minus_1.is_some())
            .unwrap();
        if let Some(d) = payload.initial_display_delay_minus_1 {
            body.write_bits_u8(d, 4).unwrap();
        }
        if is_global {
            let map = payload.xlayer_map.unwrap();
            body.write_bits(map, 31).unwrap();
            for entry in &payload.xlayer_entries {
                if ops.ptl_present {
                    write_ops_seq_profile_tier_level_info(
                        &mut body,
                        entry.ptl_info.as_ref().unwrap(),
                    )
                    .unwrap();
                }
                write_global_mlayer_source(&mut body, ops.mlayer_info_idc, &entry.mlayer).unwrap();
            }
        } else {
            match &payload.xlayer_entries[0].mlayer {
                OpsMlayerSource::Explicit(m) => write_ops_mlayer_info(&mut body, m).unwrap(),
                _ => panic!("local entry must be explicit"),
            }
        }
        body.align_to_byte();
        u32::try_from(body.bit_len() / 8).unwrap()
    }

    #[test]
    fn reset_local_round_trips() {
        round_trip(&reset_ops(ExtendedLayerId::from_bits(2), false, 3));
        round_trip(&reset_ops(ExtendedLayerId::from_bits(0), true, 15));
    }

    #[test]
    fn reset_global_round_trips() {
        round_trip(&reset_ops(GLOBAL_XLAYER_ID, false, 0));
    }

    #[test]
    fn local_minimal_round_trips() {
        round_trip(&local_one_payload());
    }

    #[test]
    fn local_full_body_round_trips() {
        let xlayer = ExtendedLayerId::from_bits(1);
        let mut ops = OperatingPointSet {
            xlayer_id: xlayer,
            reset_flag: true,
            ops_id: 2,
            ops_cnt: 1,
            priority: Some(9),
            intent: Some(42),
            intent_present: true,
            ptl_present: true,
            color_info_present: true,
            mlayer_info_idc: None,
            local_reserved_2bits: Some(0),
            payloads: vec![OperatingPointPayload {
                index: 0,
                declared_size_bytes: 0,
                computed_size_bytes: 0,
                op_intent: Some(7),
                aggregate_info: None,
                color_info: Some(OpsColorInfo {
                    color_description_idc: 0,
                    color_primaries: Some(1),
                    transfer_characteristics: Some(13),
                    matrix_coefficients: Some(6),
                    full_range_flag: true,
                }),
                decoder_model_info: Some(OpsDecoderModelInfo {
                    decoder_buffer_delay: 10,
                    encoder_buffer_delay: 20,
                    low_delay_mode_flag: true,
                }),
                initial_display_delay_minus_1: Some(3),
                xlayer_map: None,
                xlayer_entries: vec![OpsXlayerEntry {
                    xlayer_id: xlayer,
                    ptl_info: Some(OpsSeqProfileTierLevelInfo {
                        target_xlayer_id: xlayer,
                        seq_profile_idc: ProfileIdc::from_bits(0),
                        level_idx: 0,
                        tier_flag: false,
                        mlayer_count: 0,
                        reserved_2bits: 0,
                    }),
                    mlayer: OpsMlayerSource::Explicit(OpsMlayerInfo {
                        mlayer_map: 0b1,
                        tlayer_maps: vec![(0, 0b101)],
                    }),
                }],
            }],
        };
        canonicalize_sizes(&mut ops);
        round_trip(&ops);
    }

    #[test]
    fn global_idc0_single_layer_round_trips() {
        let mut ops = global_ops_one_layer(0, false, OpsMlayerSource::Absent, None);
        canonicalize_sizes(&mut ops);
        round_trip(&ops);
    }

    #[test]
    fn global_idc1_with_ptl_round_trips() {
        let mut ops = OperatingPointSet {
            xlayer_id: GLOBAL_XLAYER_ID,
            reset_flag: false,
            ops_id: 5,
            ops_cnt: 1,
            priority: Some(1),
            intent: Some(2),
            intent_present: false,
            ptl_present: true,
            color_info_present: false,
            mlayer_info_idc: Some(1),
            local_reserved_2bits: None,
            payloads: vec![OperatingPointPayload {
                index: 0,
                declared_size_bytes: 0,
                computed_size_bytes: 0,
                op_intent: None,
                aggregate_info: Some(OpsAggregateInfo {
                    config_idc: 5,
                    aggregate_level_idx: 3,
                    max_tier_flag: true,
                    max_interop: 2,
                }),
                color_info: None,
                decoder_model_info: None,
                initial_display_delay_minus_1: None,
                xlayer_map: Some(0b101), // layers 0 and 2
                xlayer_entries: vec![
                    global_entry(0, Some(0), OpsMlayerSource::Explicit(OpsMlayerInfo {
                        mlayer_map: 0,
                        tlayer_maps: vec![],
                    })),
                    global_entry(2, Some(0), OpsMlayerSource::Explicit(OpsMlayerInfo {
                        mlayer_map: 0b11,
                        tlayer_maps: vec![(0, 1), (1, 2)],
                    })),
                ],
            }],
        };
        canonicalize_sizes(&mut ops);
        round_trip(&ops);
    }

    #[test]
    fn global_idc2_inherited_and_explicit_round_trips() {
        let mut ops = OperatingPointSet {
            xlayer_id: GLOBAL_XLAYER_ID,
            reset_flag: false,
            ops_id: 0,
            ops_cnt: 1,
            priority: Some(0),
            intent: Some(0),
            intent_present: false,
            ptl_present: false,
            color_info_present: false,
            mlayer_info_idc: Some(2),
            local_reserved_2bits: None,
            payloads: vec![OperatingPointPayload {
                index: 0,
                declared_size_bytes: 0,
                computed_size_bytes: 0,
                op_intent: None,
                aggregate_info: None,
                color_info: None,
                decoder_model_info: None,
                initial_display_delay_minus_1: None,
                xlayer_map: Some(0b11), // layers 0 and 1
                xlayer_entries: vec![
                    global_entry(0, None, OpsMlayerSource::Explicit(OpsMlayerInfo {
                        mlayer_map: 0,
                        tlayer_maps: vec![],
                    })),
                    global_entry(1, None, OpsMlayerSource::Inherited {
                        embedded_ops_id: 0,
                        embedded_op_index: 5,
                    }),
                ],
            }],
        };
        canonicalize_sizes(&mut ops);
        round_trip(&ops);
    }

    #[test]
    fn global_idc3_reserved_round_trips() {
        let mut ops = global_ops_one_layer(3, false, OpsMlayerSource::Absent, None);
        canonicalize_sizes(&mut ops);
        round_trip(&ops);
    }

    #[test]
    fn global_multi_payload_round_trips() {
        let payload = |index: u8| OperatingPointPayload {
            index,
            declared_size_bytes: 0,
            computed_size_bytes: 0,
            op_intent: None,
            aggregate_info: None,
            color_info: None,
            decoder_model_info: None,
            initial_display_delay_minus_1: None,
            xlayer_map: Some(0b1),
            xlayer_entries: vec![global_entry(0, None, OpsMlayerSource::Absent)],
        };
        let mut ops = OperatingPointSet {
            xlayer_id: GLOBAL_XLAYER_ID,
            reset_flag: false,
            ops_id: 1,
            ops_cnt: 2,
            priority: Some(0),
            intent: Some(0),
            intent_present: false,
            ptl_present: false,
            color_info_present: false,
            mlayer_info_idc: Some(0),
            local_reserved_2bits: None,
            payloads: vec![payload(0), payload(1)],
        };
        canonicalize_sizes(&mut ops);
        round_trip(&ops);
    }

    #[test]
    fn color_implicit_idc_round_trips() {
        let xlayer = ExtendedLayerId::from_bits(2);
        let mut ops = OperatingPointSet {
            xlayer_id: xlayer,
            reset_flag: false,
            ops_id: 0,
            ops_cnt: 1,
            priority: Some(0),
            intent: Some(0),
            intent_present: false,
            ptl_present: false,
            color_info_present: true,
            mlayer_info_idc: None,
            local_reserved_2bits: Some(0),
            payloads: vec![OperatingPointPayload {
                index: 0,
                declared_size_bytes: 0,
                computed_size_bytes: 0,
                op_intent: None,
                aggregate_info: None,
                color_info: Some(OpsColorInfo {
                    color_description_idc: 2,
                    color_primaries: None,
                    transfer_characteristics: None,
                    matrix_coefficients: None,
                    full_range_flag: false,
                }),
                decoder_model_info: None,
                initial_display_delay_minus_1: None,
                xlayer_map: None,
                xlayer_entries: vec![OpsXlayerEntry {
                    xlayer_id: xlayer,
                    ptl_info: None,
                    mlayer: OpsMlayerSource::Explicit(OpsMlayerInfo {
                        mlayer_map: 0,
                        tlayer_maps: vec![],
                    }),
                }],
            }],
        };
        canonicalize_sizes(&mut ops);
        round_trip(&ops);
    }

    fn global_entry(
        layer_bit: u8,
        profile: Option<u8>,
        mlayer: OpsMlayerSource,
    ) -> OpsXlayerEntry {
        let xlayer = ExtendedLayerId::from_bits(layer_bit);
        OpsXlayerEntry {
            xlayer_id: xlayer,
            ptl_info: profile.map(|p| OpsSeqProfileTierLevelInfo {
                target_xlayer_id: xlayer,
                seq_profile_idc: ProfileIdc::from_bits(p),
                level_idx: 0,
                tier_flag: false,
                mlayer_count: 0,
                reserved_2bits: 0,
            }),
            mlayer,
        }
    }

    fn global_ops_one_layer(
        idc: u8,
        ptl_present: bool,
        mlayer: OpsMlayerSource,
        profile: Option<u8>,
    ) -> OperatingPointSet {
        OperatingPointSet {
            xlayer_id: GLOBAL_XLAYER_ID,
            reset_flag: false,
            ops_id: 0,
            ops_cnt: 1,
            priority: Some(0),
            intent: Some(0),
            intent_present: false,
            ptl_present,
            color_info_present: false,
            mlayer_info_idc: Some(idc),
            local_reserved_2bits: None,
            payloads: vec![OperatingPointPayload {
                index: 0,
                declared_size_bytes: 0,
                computed_size_bytes: 0,
                op_intent: None,
                aggregate_info: None,
                color_info: None,
                decoder_model_info: None,
                initial_display_delay_minus_1: None,
                xlayer_map: Some(0b1),
                xlayer_entries: vec![global_entry(0, profile, mlayer)],
            }],
        }
    }

    /// Asserts the writer rejects `ops` with `NonCanonicalOperatingPointSet { what }` and writes
    /// no bit.
    fn assert_reject(ops: &OperatingPointSet, expect_what: &str) {
        let mut writer = BitWriter::new();
        let err = write_operating_point_set(&mut writer, ops, ops.xlayer_id).unwrap_err();
        assert!(
            matches!(err, WriteError::NonCanonicalOperatingPointSet { what } if what == expect_what),
            "expected {expect_what}, got {err:?}"
        );
        assert_eq!(writer.bit_len(), 0, "reject left bits in the writer");
    }

    #[test]
    fn xlayer_id_mismatch_rejects() {
        // The obu_xlayer_id argument disagrees with the stored xlayer_id.
        let ops = reset_ops(ExtendedLayerId::from_bits(2), false, 0);
        let mut writer = BitWriter::new();
        let err =
            write_operating_point_set(&mut writer, &ops, ExtendedLayerId::from_bits(3)).unwrap_err();
        assert!(
            matches!(err, WriteError::NonCanonicalOperatingPointSet { what } if what == "xlayer_id"),
            "expected xlayer_id, got {err:?}"
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn reset_with_priority_rejects() {
        let mut ops = reset_ops(ExtendedLayerId::from_bits(1), false, 0);
        ops.priority = Some(0);
        assert_reject(&ops, "reset_branch_field");
    }

    #[test]
    fn reset_with_intent_present_rejects() {
        let mut ops = reset_ops(ExtendedLayerId::from_bits(1), false, 0);
        ops.intent_present = true;
        assert_reject(&ops, "intent_present_reset");
    }

    #[test]
    fn reset_with_ptl_present_rejects() {
        let mut ops = reset_ops(ExtendedLayerId::from_bits(1), false, 0);
        ops.ptl_present = true;
        assert_reject(&ops, "ptl_present_reset");
    }

    #[test]
    fn reset_with_color_present_rejects() {
        let mut ops = reset_ops(ExtendedLayerId::from_bits(1), false, 0);
        ops.color_info_present = true;
        assert_reject(&ops, "color_present_reset");
    }

    #[test]
    fn reset_with_mlayer_idc_rejects() {
        let mut ops = reset_ops(GLOBAL_XLAYER_ID, false, 0);
        ops.mlayer_info_idc = Some(0);
        assert_reject(&ops, "mlayer_info_idc_scope");
    }

    #[test]
    fn reset_with_local_reserved_rejects() {
        let mut ops = reset_ops(ExtendedLayerId::from_bits(1), false, 0);
        ops.local_reserved_2bits = Some(0);
        assert_reject(&ops, "local_reserved_scope");
    }

    #[test]
    fn reset_with_payload_rejects() {
        let mut ops = reset_ops(ExtendedLayerId::from_bits(1), false, 0);
        ops.payloads.push(OperatingPointPayload {
            index: 0,
            declared_size_bytes: 0,
            computed_size_bytes: 0,
            op_intent: None,
            aggregate_info: None,
            color_info: None,
            decoder_model_info: None,
            initial_display_delay_minus_1: None,
            xlayer_map: None,
            xlayer_entries: vec![],
        });
        assert_reject(&ops, "payload_count");
    }

    #[test]
    fn active_missing_priority_rejects() {
        let mut ops = global_ops_one_layer(0, false, OpsMlayerSource::Absent, None);
        canonicalize_sizes(&mut ops);
        ops.priority = None;
        assert_reject(&ops, "reset_branch_field");
    }

    #[test]
    fn global_missing_mlayer_idc_rejects() {
        let mut ops = global_ops_one_layer(0, false, OpsMlayerSource::Absent, None);
        canonicalize_sizes(&mut ops);
        ops.mlayer_info_idc = None;
        assert_reject(&ops, "mlayer_info_idc_scope");
    }

    #[test]
    fn global_with_local_reserved_rejects() {
        let mut ops = global_ops_one_layer(0, false, OpsMlayerSource::Absent, None);
        canonicalize_sizes(&mut ops);
        ops.local_reserved_2bits = Some(0);
        assert_reject(&ops, "local_reserved_scope");
    }

    #[test]
    fn local_missing_reserved_rejects() {
        let mut ops = local_one_payload();
        ops.local_reserved_2bits = None;
        assert_reject(&ops, "local_reserved_scope");
    }

    #[test]
    fn local_with_mlayer_idc_rejects() {
        let mut ops = local_one_payload();
        ops.mlayer_info_idc = Some(0);
        assert_reject(&ops, "mlayer_info_idc_scope");
    }

    #[test]
    fn payload_count_mismatch_rejects() {
        let mut ops = local_one_payload();
        ops.ops_cnt = 2; // disagrees with payloads.len() == 1
        assert_reject(&ops, "payload_count");
    }

    #[test]
    fn payload_index_mismatch_rejects() {
        let mut ops = local_one_payload();
        ops.payloads[0].index = 7;
        assert_reject(&ops, "payload_index");
    }

    #[test]
    fn op_intent_gate_mismatch_rejects() {
        let mut ops = local_one_payload();
        ops.payloads[0].op_intent = Some(1);
        assert_reject(&ops, "op_intent_gate");
    }

    #[test]
    fn op_intent_missing_when_present_rejects() {
        let mut ops = local_one_payload();
        ops.intent_present = true;
        ops.payloads[0].op_intent = None;
        assert_reject(&ops, "op_intent_gate");
    }

    #[test]
    fn aggregate_info_gate_mismatch_rejects() {
        let mut ops = global_ops_one_layer(0, false, OpsMlayerSource::Absent, None);
        canonicalize_sizes(&mut ops);
        ops.payloads[0].aggregate_info = Some(OpsAggregateInfo {
            config_idc: 0,
            aggregate_level_idx: 0,
            max_tier_flag: false,
            max_interop: 0,
        });
        assert_reject(&ops, "aggregate_info_gate");
    }

    #[test]
    fn aggregate_info_missing_when_ptl_present_rejects() {
        let mut ops = global_ops_one_layer(0, true, OpsMlayerSource::Absent, Some(0));
        canonicalize_sizes(&mut ops);
        ops.payloads[0].aggregate_info = None;
        assert_reject(&ops, "aggregate_info_gate");
    }

    #[test]
    fn local_aggregate_info_rejects() {
        let mut ops = local_one_payload();
        ops.payloads[0].aggregate_info = Some(OpsAggregateInfo {
            config_idc: 0,
            aggregate_level_idx: 0,
            max_tier_flag: false,
            max_interop: 0,
        });
        assert_reject(&ops, "aggregate_info_gate");
    }

    #[test]
    fn color_info_gate_mismatch_rejects() {
        let mut ops = local_one_payload();
        ops.payloads[0].color_info = Some(OpsColorInfo {
            color_description_idc: 2,
            color_primaries: None,
            transfer_characteristics: None,
            matrix_coefficients: None,
            full_range_flag: false,
        });
        assert_reject(&ops, "color_info_gate");
    }

    #[test]
    fn color_triple_gate_mismatch_rejects() {
        let mut ops = local_one_payload();
        ops.color_info_present = true;
        ops.payloads[0].color_info = Some(OpsColorInfo {
            color_description_idc: 0,
            color_primaries: None,
            transfer_characteristics: None,
            matrix_coefficients: None,
            full_range_flag: false,
        });
        assert_reject(&ops, "color_triple_gate");
    }

    #[test]
    fn color_triple_present_when_implicit_rejects() {
        let mut ops = local_one_payload();
        ops.color_info_present = true;
        ops.payloads[0].color_info = Some(OpsColorInfo {
            color_description_idc: 1,
            color_primaries: Some(1),
            transfer_characteristics: None,
            matrix_coefficients: None,
            full_range_flag: false,
        });
        assert_reject(&ops, "color_triple_gate");
    }

    #[test]
    fn global_xlayer_map_absent_rejects() {
        let mut ops = global_ops_one_layer(0, false, OpsMlayerSource::Absent, None);
        canonicalize_sizes(&mut ops);
        ops.payloads[0].xlayer_map = None;
        assert_reject(&ops, "xlayer_map_scope");
    }

    #[test]
    fn local_xlayer_map_present_rejects() {
        let mut ops = local_one_payload();
        ops.payloads[0].xlayer_map = Some(0b1);
        assert_reject(&ops, "xlayer_map_scope");
    }

    #[test]
    fn global_xlayer_entries_count_mismatch_rejects() {
        let mut ops = global_ops_one_layer(0, false, OpsMlayerSource::Absent, None);
        canonicalize_sizes(&mut ops);
        ops.payloads[0]
            .xlayer_entries
            .push(global_entry(1, None, OpsMlayerSource::Absent));
        assert_reject(&ops, "xlayer_entries");
    }

    #[test]
    fn global_xlayer_entry_wrong_layer_rejects() {
        let mut ops = global_ops_one_layer(0, false, OpsMlayerSource::Absent, None);
        canonicalize_sizes(&mut ops);
        ops.payloads[0].xlayer_entries[0] = global_entry(1, None, OpsMlayerSource::Absent);
        assert_reject(&ops, "xlayer_entries");
    }

    #[test]
    fn local_two_entries_rejects() {
        let mut ops = local_one_payload();
        ops.payloads[0].xlayer_entries.push(OpsXlayerEntry {
            xlayer_id: ExtendedLayerId::from_bits(5),
            ptl_info: None,
            mlayer: OpsMlayerSource::Explicit(OpsMlayerInfo {
                mlayer_map: 0,
                tlayer_maps: vec![],
            }),
        });
        assert_reject(&ops, "xlayer_entries");
    }

    #[test]
    fn local_entry_wrong_layer_rejects() {
        let mut ops = local_one_payload();
        ops.payloads[0].xlayer_entries[0].xlayer_id = ExtendedLayerId::from_bits(9);
        assert_reject(&ops, "xlayer_entries");
    }

    #[test]
    fn entry_ptl_gate_mismatch_rejects() {
        let mut ops = local_one_payload();
        ops.ptl_present = true;
        ops.payloads[0].xlayer_entries[0].ptl_info = None;
        assert_reject(&ops, "entry_ptl_gate");
    }

    #[test]
    fn entry_ptl_wrong_target_rejects() {
        let mut ops = local_one_payload();
        ops.ptl_present = true;
        ops.payloads[0].xlayer_entries[0].ptl_info = Some(OpsSeqProfileTierLevelInfo {
            target_xlayer_id: ExtendedLayerId::from_bits(7), // != the OBU's layer (4)
            seq_profile_idc: ProfileIdc::from_bits(0),
            level_idx: 0,
            tier_flag: false,
            mlayer_count: 0,
            reserved_2bits: 0,
        });
        assert_reject(&ops, "entry_ptl_gate");
    }

    #[test]
    fn entry_ptl_present_when_absent_flag_rejects() {
        let mut ops = global_ops_one_layer(0, false, OpsMlayerSource::Absent, None);
        canonicalize_sizes(&mut ops);
        let xlayer = ExtendedLayerId::from_bits(0);
        ops.payloads[0].xlayer_entries[0].ptl_info = Some(OpsSeqProfileTierLevelInfo {
            target_xlayer_id: xlayer,
            seq_profile_idc: ProfileIdc::from_bits(0),
            level_idx: 0,
            tier_flag: false,
            mlayer_count: 0,
            reserved_2bits: 0,
        });
        assert_reject(&ops, "entry_ptl_gate");
    }

    #[test]
    fn local_entry_not_explicit_rejects() {
        let mut ops = local_one_payload();
        ops.payloads[0].xlayer_entries[0].mlayer = OpsMlayerSource::Absent;
        assert_reject(&ops, "local_entry_mlayer");
    }

    #[test]
    fn global_mlayer_source_disagrees_with_idc_rejects() {
        // idc=0 codes nothing, so a non-Absent source is non-canonical. The writer rejects while
        // drafting the body (before the ops_data_size check), so the placeholder size is irrelevant
        // and the fixture is not canonicalized (canonicalize would itself trip the same reject).
        let ops = global_ops_one_layer(
            0,
            false,
            OpsMlayerSource::Explicit(OpsMlayerInfo {
                mlayer_map: 0,
                tlayer_maps: vec![],
            }),
            None,
        );
        assert_reject(&ops, "global_mlayer_source");
    }

    #[test]
    fn global_idc1_inherited_source_rejects() {
        let ops = global_ops_one_layer(
            1,
            false,
            OpsMlayerSource::Inherited {
                embedded_ops_id: 0,
                embedded_op_index: 0,
            },
            None,
        );
        assert_reject(&ops, "global_mlayer_source");
    }

    #[test]
    fn global_idc2_absent_source_rejects() {
        let ops = global_ops_one_layer(2, false, OpsMlayerSource::Absent, None);
        assert_reject(&ops, "global_mlayer_source");
    }

    #[test]
    fn mlayer_tlayer_maps_mismatch_rejects() {
        let mut ops = local_one_payload();
        if let OpsMlayerSource::Explicit(mlayer) = &mut ops.payloads[0].xlayer_entries[0].mlayer {
            mlayer.mlayer_map = 0;
            mlayer.tlayer_maps = vec![(0, 1)];
        }
        assert_reject(&ops, "mlayer_tlayer_maps");
    }

    #[test]
    fn mlayer_tlayer_maps_wrong_layer_rejects() {
        let mut ops = local_one_payload();
        if let OpsMlayerSource::Explicit(mlayer) = &mut ops.payloads[0].xlayer_entries[0].mlayer {
            mlayer.mlayer_map = 0b1;
            mlayer.tlayer_maps = vec![(3, 1)];
        }
        assert_reject(&ops, "mlayer_tlayer_maps");
    }

    #[test]
    fn declared_ops_data_size_mismatch_round_trips() {
        let mut ops = local_one_payload();
        ops.payloads[0].declared_size_bytes += 1;
        round_trip(&ops);
    }

    #[test]
    fn computed_size_bytes_mismatch_rejects() {
        let mut ops = local_one_payload();
        ops.payloads[0].computed_size_bytes += 1;
        assert_reject(&ops, "ops_computed_size");
    }

    /// A canonical local one-payload OPS (no intent/ptl/color, one explicit empty-map entry),
    /// used as a base for reject mutations.
    fn local_one_payload() -> OperatingPointSet {
        let xlayer = ExtendedLayerId::from_bits(4);
        let mut ops = OperatingPointSet {
            xlayer_id: xlayer,
            reset_flag: false,
            ops_id: 1,
            ops_cnt: 1,
            priority: Some(0),
            intent: Some(0),
            intent_present: false,
            ptl_present: false,
            color_info_present: false,
            mlayer_info_idc: None,
            local_reserved_2bits: Some(0),
            payloads: vec![OperatingPointPayload {
                index: 0,
                declared_size_bytes: 0,
                computed_size_bytes: 0,
                op_intent: None,
                aggregate_info: None,
                color_info: None,
                decoder_model_info: None,
                initial_display_delay_minus_1: None,
                xlayer_map: None,
                xlayer_entries: vec![OpsXlayerEntry {
                    xlayer_id: xlayer,
                    ptl_info: None,
                    mlayer: OpsMlayerSource::Explicit(OpsMlayerInfo {
                        mlayer_map: 0,
                        tlayer_maps: vec![],
                    }),
                }],
            }],
        };
        canonicalize_sizes(&mut ops);
        ops
    }

    #[test]
    fn out_of_field_ops_id_rejects() {
        let ops = reset_ops(ExtendedLayerId::from_bits(1), false, 16);
        let mut writer = BitWriter::new();
        let err = write_operating_point_set(&mut writer, &ops, ops.xlayer_id).unwrap_err();
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
        let ops = reset_ops(ExtendedLayerId::from_bits(1), false, 0);
        let err = write_operating_point_set(&mut writer, &ops, ops.xlayer_id).unwrap_err();
        assert!(matches!(err, WriteError::WriterNotByteAligned));
        assert_eq!(writer.bit_len(), 1, "only the pre-existing stray bit remains");
    }
}

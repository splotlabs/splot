// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::bitio::BitReader;
    use crate::headers::layer_config_record::{LcrXlayerAtlasRef, parse_layer_config_record};
    use crate::span::ByteOffset;
    use crate::types::{ExtendedLayerId, GLOBAL_XLAYER_ID};

    use crate::test_bits::Bits;

    /// The fixed `lcr_global_info()` prefix with every optional section absent.
    fn global_prefix(
        global_id: u32,
        xlayer_map: u32,
        agg: bool,
        ptl: bool,
        payload: bool,
        dependent: bool,
        atlas: bool,
    ) -> Bits {
        let mut bits = Bits::default();
        bits.f(global_id, 3); // lcr_global_config_record_id
        bits.f(xlayer_map, 31); // lcr_xlayer_map
        bits.bit(u8::from(agg)); // lcr_aggregate_info_present_flag
        bits.bit(u8::from(ptl)); // lcr_seq_profile_tier_level_info_present_flag
        bits.bit(u8::from(payload)); // lcr_global_payload_present_flag
        bits.bit(u8::from(dependent)); // lcr_dependent_xlayers_flag
        bits.bit(u8::from(atlas)); // lcr_global_atlas_id_present_flag
        bits.f(0, 7); // lcr_global_purpose_id
        bits.bit(0); // lcr_doh_constraint_flag
        bits.bit(0); // lcr_enforce_tile_alignment_flag
        bits.f(0, 3); // lcr_global_atlas_id OR lcr_global_reserved_zero_3bits
        bits.f(0, 5); // lcr_global_reserved_zero_5bits
        bits
    }

    /// A minimal `lcr_xlayer_info()` with all present flags clear (four flag bits + alignment). When
    /// `global_atlas` is set, the else-branch `f(8)` atlas triple follows.
    fn minimal_xlayer_info(global_atlas: bool) -> Bits {
        let mut bits = Bits::default();
        bits.f(0, 4); // four present flags clear
        bits.align(); // byte_alignment()
        if global_atlas {
            bits.f(0, 8); // lcr_xlayer_atlas_segment_id
            bits.f(0, 8); // lcr_xlayer_priority_order
            bits.f(0, 8); // lcr_xlayer_rendering_method
        }
        bits
    }

    fn parse(body: &[u8], xlayer: ExtendedLayerId) -> LayerConfigurationRecord {
        let mut reader = BitReader::new(body, ByteOffset::new(0));
        parse_layer_config_record(&mut reader, xlayer).unwrap()
    }

    /// Parses a hand-built body, writes the model back (under the same header `xlayer`), asserts
    /// byte-exact + semantic round-trip, and returns the parsed model.
    fn round_trip(body: &[u8], xlayer: ExtendedLayerId) -> LayerConfigurationRecord {
        let model = parse(body, xlayer);
        let mut writer = BitWriter::new();
        write_layer_config_record(&mut writer, &model, xlayer).unwrap();
        let bytes = writer.into_bytes();
        assert_eq!(bytes, body, "byte-exact round-trip");
        let reparsed = parse(&bytes, xlayer);
        assert_eq!(model, reparsed, "semantic round-trip");
        model
    }

    /// The header `obu_xlayer_id` that *agrees* with a model's scope — `GLOBAL_XLAYER_ID` for a
    /// global record, the stored `xlayer_id` for a local one — so a reject test exercises the
    /// intended invariant rather than the scope guard.
    fn matching_xlayer(model: &LayerConfigurationRecord) -> ExtendedLayerId {
        match model {
            LayerConfigurationRecord::Global(_) => GLOBAL_XLAYER_ID,
            LayerConfigurationRecord::Local(info) => ExtendedLayerId::from_bits(info.xlayer_id),
        }
    }

    /// Writes a (deliberately non-canonical) model under the scope-matching header and returns the
    /// reject, asserting no bit reached the destination writer.
    fn write_err(model: &LayerConfigurationRecord) -> WriteError {
        write_err_xlayer(model, matching_xlayer(model))
    }

    /// Like [`write_err`] but with an explicit header `obu_xlayer_id` (for the scope / local-id
    /// disagreement tests).
    fn write_err_xlayer(model: &LayerConfigurationRecord, xlayer: ExtendedLayerId) -> WriteError {
        let mut writer = BitWriter::new();
        let err = write_layer_config_record(&mut writer, model, xlayer).unwrap_err();
        assert_eq!(writer.bit_len(), 0, "a rejected model writes no bit");
        err
    }

    fn global(record: &mut LayerConfigurationRecord) -> &mut LcrGlobalInfo {
        match record {
            LayerConfigurationRecord::Global(info) => info,
            LayerConfigurationRecord::Local(_) => panic!("expected a global record"),
        }
    }

    fn local(record: &mut LayerConfigurationRecord) -> &mut LcrLocalInfo {
        match record {
            LayerConfigurationRecord::Local(info) => info,
            LayerConfigurationRecord::Global(_) => panic!("expected a local record"),
        }
    }

    fn what(err: &WriteError) -> &'static str {
        match err {
            WriteError::NonCanonicalLayerConfigRecord { what } => what,
            other => panic!("expected NonCanonicalLayerConfigRecord, got {other:?}"),
        }
    }

    #[test]
    fn minimal_global_round_trips() {
        let body = global_prefix(1, 0b1, false, false, false, false, false).into_bytes();
        round_trip(&body, GLOBAL_XLAYER_ID);
    }

    #[test]
    fn global_with_aggregate_round_trips() {
        let mut bits = global_prefix(7, 0b101, true, false, false, false, false);
        bits.f(0b10_1010, 6); // lcr_config_idc
        bits.f(0b1_0101, 5); // lcr_aggregate_level_idx
        bits.bit(1); // lcr_max_tier_flag
        bits.f(0b1001, 4); // lcr_max_interop
        round_trip(&bits.into_bytes(), GLOBAL_XLAYER_ID);
    }

    #[test]
    fn global_with_seq_ptl_round_trips() {
        let mut bits = global_prefix(2, 0b101, false, true, false, false, false);
        bits.ptl(4, 7, 1, 3, 0); // xId = 0
        bits.ptl(31, 12, 0, 1, 0); // xId = 2 (Configurable profile)
        round_trip(&bits.into_bytes(), GLOBAL_XLAYER_ID);
    }

    #[test]
    fn global_with_atlas_id_round_trips() {
        let mut bits = Bits::default();
        bits.f(3, 3); // id
        bits.f(0b1, 31); // map
        bits.f(0, 4); // agg/ptl/payload/dependent flags = 0
        bits.bit(1); // lcr_global_atlas_id_present_flag
        bits.f(0, 7); // purpose
        bits.f(0, 2); // doh / tile-alignment
        bits.f(5, 3); // lcr_global_atlas_id = 5
        bits.f(0, 5); // reserved_zero_5bits
        let record = round_trip(&bits.into_bytes(), GLOBAL_XLAYER_ID);
        let LayerConfigurationRecord::Global(info) = record else {
            panic!("expected global");
        };
        assert_eq!(info.global_atlas_id, Some(5));
    }

    #[test]
    fn global_payload_exact_size_round_trips() {
        let mut bits = global_prefix(2, 0b1, false, false, true, false, false);
        bits.leb128_byte(1); // lcr_data_size = 1 byte == the minimal xlayer_info
        bits.extend(minimal_xlayer_info(false));
        round_trip(&bits.into_bytes(), GLOBAL_XLAYER_ID);
    }

    #[test]
    fn global_payload_with_remaining_filler_round_trips() {
        let mut bits = global_prefix(2, 0b1, false, false, true, false, false);
        bits.leb128_byte(2); // lcr_data_size = 2 bytes -> 8 remaining filler bits
        bits.extend(minimal_xlayer_info(false));
        bits.f(0, 8); // 8 lcr_remaining_payload_bit
        let record = round_trip(&bits.into_bytes(), GLOBAL_XLAYER_ID);
        let LayerConfigurationRecord::Global(info) = record else {
            panic!("expected global");
        };
        assert_eq!(info.payloads[0].remaining_payload_bits, 8);
    }

    #[test]
    fn global_payload_with_dependent_map_round_trips() {
        let mut bits = global_prefix(1, 0b10, false, false, true, true, false);
        bits.leb128_byte(2); // lcr_data_size = 2 bytes
        bits.bit(1); // lcr_num_dependent_xlayer_map f(1) = 1
        bits.extend(minimal_xlayer_info(false)); // 1 byte
        bits.f(0, 7); // remaining filler to fill 2 bytes (1 + 8 + 7 = 16)
        let record = round_trip(&bits.into_bytes(), GLOBAL_XLAYER_ID);
        let LayerConfigurationRecord::Global(info) = record else {
            panic!("expected global");
        };
        assert_eq!(info.payloads[0].num_dependent_xlayer_map, Some(1));
    }

    #[test]
    fn global_payload_with_atlas_else_branch_round_trips() {
        let mut bits = Bits::default();
        bits.f(1, 3); // id
        bits.f(0b1, 31); // map
        bits.bit(0); // agg
        bits.bit(0); // ptl
        bits.bit(1); // payload present
        bits.bit(0); // dependent
        bits.bit(1); // lcr_global_atlas_id_present_flag
        bits.f(0, 7); // purpose
        bits.f(0, 2); // doh / tile
        bits.f(2, 3); // lcr_global_atlas_id
        bits.f(0, 5); // reserved_zero_5bits
        bits.leb128_byte(4); // lcr_data_size = 4 bytes
        bits.f(0, 4); // four xlayer_info present flags clear
        bits.align(); // byte_alignment()
        bits.f(9, 8); // lcr_xlayer_atlas_segment_id
        bits.f(3, 8); // lcr_xlayer_priority_order
        bits.f(1, 8); // lcr_xlayer_rendering_method
        let record = round_trip(&bits.into_bytes(), GLOBAL_XLAYER_ID);
        let LayerConfigurationRecord::Global(info) = record else {
            panic!("expected global");
        };
        let atlas = info.payloads[0].xlayer_info.xlayer_atlas.unwrap();
        assert_eq!(atlas.atlas_segment_id, 9);
        assert_eq!(atlas.priority_order, 3);
        assert_eq!(atlas.rendering_method, 1);
    }

    #[test]
    fn minimal_local_round_trips() {
        round_trip(&minimal_local_body(), ExtendedLayerId::from_bits(2));
    }

    #[test]
    fn local_with_ptl_and_rep_info_round_trips() {
        let mut bits = Bits::default();
        bits.f(0, 3); // lcr_global_id
        bits.f(2, 3); // lcr_local_id
        bits.bit(1); // ptl present
        bits.bit(0); // local atlas present
        bits.ptl(0, 5, 0, 2, 0); // lcr_seq_profile_tier_level_info(xId)
        bits.f(0, 3); // reserved_zero_3bits
        bits.f(0, 5); // reserved_zero_5bits
        bits.bit(1); // rep_info present
        bits.bit(0); // purpose present
        bits.bit(0); // color present
        bits.bit(0); // embedded present
        bits.uvlc(1920); // lcr_max_pic_width
        bits.uvlc(1080); // lcr_max_pic_height
        bits.bit(1); // lcr_format_info_present_flag
        bits.bit(1); // lcr_cropping_window_present_flag
        bits.uvlc(10); // lcr_bit_depth_idc
        bits.uvlc(1); // lcr_chroma_format_idc
        bits.uvlc(2); // left
        bits.uvlc(3); // right
        bits.uvlc(4); // top
        bits.uvlc(5); // bottom
        bits.align(); // byte_alignment()
        let record = round_trip(&bits.into_bytes(), ExtendedLayerId::from_bits(1));
        let LayerConfigurationRecord::Local(info) = record else {
            panic!("expected local");
        };
        let rep = info.xlayer_info.rep_info.unwrap();
        assert_eq!(rep.max_pic_width, 1920);
        assert_eq!(rep.format_info.unwrap().bit_depth_idc, 10);
        assert_eq!(rep.cropping_window.unwrap().bottom_offset, 5);
    }

    /// A local LCR whose xlayer_info carries color info and a two-layer embedded map (bits 0 and 1),
    /// the local atlas making the per-layer atlas triple present, with AUX / explicit-view / dependent
    /// map / max-expected fields exercised. Returned for reuse as a reject-test base.
    fn local_embedded_kitchen_sink_body() -> Vec<u8> {
        let mut bits = Bits::default();
        bits.f(0, 3); // lcr_global_id
        bits.f(1, 3); // lcr_local_id
        bits.bit(0); // ptl present
        bits.bit(1); // local atlas present
        bits.f(1, 3); // lcr_local_atlas_id
        bits.f(0, 5); // reserved_zero_5bits
        bits.bit(0); // rep_info present
        bits.bit(0); // purpose present
        bits.bit(1); // color info present
        bits.bit(1); // embedded layer info present
        bits.rg(0, 2); // layer_color_description_idc = 0 -> primaries present
        bits.f(1, 8); // primaries
        bits.f(13, 8); // transfer
        bits.f(6, 8); // matrix
        bits.bit(1); // full_range_flag
        bits.align(); // byte_alignment()
        bits.f(0b0000_0011, 8); // lcr_mlayer_map -> j = 0 and j = 1
        bits.f(0b0101, 4); // lcr_tlayer_map
        bits.f(7, 8); // atlas_segment_id
        bits.f(2, 8); // priority_order
        bits.f(0, 8); // rendering_method
        bits.f(u32::from(AUX_LAYER), 8); // lcr_layer_type = AUX_LAYER
        bits.f(9, 8); // lcr_auxiliary_type
        bits.f(u32::from(VIEW_EXPLICIT), 8); // lcr_view_type = VIEW_EXPLICIT
        bits.f(3, 8); // lcr_view_id
        bits.bit(0); // same_sh_max_resolution_flag = 0 -> max expected follows
        bits.uvlc(1920); // max_expected_width
        bits.uvlc(1080); // max_expected_height
        bits.align(); // per-iteration byte_alignment()
        bits.f(0b0001, 4); // lcr_tlayer_map
        bits.f(8, 8); // atlas_segment_id
        bits.f(1, 8); // priority_order
        bits.f(0, 8); // rendering_method
        bits.f(0, 8); // lcr_layer_type (not AUX) -> no auxiliary_type
        bits.f(0, 8); // lcr_view_type (not explicit) -> no view_id
        bits.f(1, 1); // lcr_dependent_layer_map f(j=1)
        bits.bit(1); // same_sh_max_resolution_flag = 1 -> no max expected
        bits.align(); // per-iteration byte_alignment()
        bits.into_bytes()
    }

    #[test]
    fn local_embedded_kitchen_sink_round_trips() {
        let record = round_trip(&local_embedded_kitchen_sink_body(), ExtendedLayerId::from_bits(1));
        let LayerConfigurationRecord::Local(info) = record else {
            panic!("expected local");
        };
        let color = info.xlayer_info.color_info.unwrap();
        assert_eq!(color.primaries, Some((1, 13, 6)));
        let embedded = info.xlayer_info.embedded_layer_info.unwrap();
        assert_eq!(embedded.layers.len(), 2);
        assert_eq!(embedded.layers[0].auxiliary_type, Some(9));
        assert_eq!(embedded.layers[0].view_id, Some(3));
        assert_eq!(embedded.layers[0].max_expected_width, Some(1920));
        assert_eq!(embedded.layers[1].mlayer_index, 1);
        assert_eq!(embedded.layers[1].dependent_layer_map, Some(1));
        assert!(embedded.layers[1].same_sh_max_resolution_flag);
    }

    #[test]
    fn rejects_aggregate_info_gate() {
        let mut record = parse(
            &global_prefix(1, 0b1, false, false, false, false, false).into_bytes(),
            GLOBAL_XLAYER_ID,
        );
        global(&mut record).aggregate_info_present = true; // aggregate_info stays None
        assert_eq!(what(&write_err(&record)), "aggregate_info_gate");
    }

    #[test]
    fn rejects_global_atlas_id_gate() {
        let mut record = parse(
            &global_prefix(1, 0b1, false, false, false, false, false).into_bytes(),
            GLOBAL_XLAYER_ID,
        );
        global(&mut record).global_atlas_id_present = true; // global_atlas_id stays None
        assert_eq!(what(&write_err(&record)), "global_atlas_id_gate");
    }

    #[test]
    fn rejects_atlas_reserved_3bits_nonzero() {
        let mut bits = Bits::default();
        bits.f(1, 3); // id
        bits.f(0b1, 31); // map
        bits.f(0, 4); // agg/ptl/payload/dependent
        bits.bit(1); // atlas present
        bits.f(0, 7); // purpose
        bits.f(0, 2); // doh/tile
        bits.f(4, 3); // lcr_global_atlas_id = 4
        bits.f(0, 5); // reserved_zero_5bits
        let mut record = parse(&bits.into_bytes(), GLOBAL_XLAYER_ID);
        global(&mut record).reserved_zero_3bits = 1; // non-zero with atlas present
        assert_eq!(what(&write_err(&record)), "atlas_reserved_3bits");
    }

    #[test]
    fn rejects_seq_ptl_info_count() {
        let mut record = parse(
            &global_prefix(1, 0b1, false, false, false, false, false).into_bytes(),
            GLOBAL_XLAYER_ID,
        );
        global(&mut record).seq_ptl_info_present = true; // but seq_ptl_infos is empty
        assert_eq!(what(&write_err(&record)), "seq_ptl_info_count");
    }

    #[test]
    fn rejects_seq_ptl_xlayer_id() {
        let mut bits = global_prefix(1, 0b1, false, true, false, false, false);
        bits.ptl(0, 0, 0, 0, 0);
        let mut record = parse(&bits.into_bytes(), GLOBAL_XLAYER_ID);
        global(&mut record).seq_ptl_infos[0].xlayer_id = 9; // != the map's set-bit id 0
        assert_eq!(what(&write_err(&record)), "seq_ptl_xlayer_id");
    }

    #[test]
    fn rejects_payload_count() {
        let mut record = parse(
            &global_prefix(1, 0b1, false, false, false, false, false).into_bytes(),
            GLOBAL_XLAYER_ID,
        );
        global(&mut record).global_payload_present = true; // but payloads is empty
        assert_eq!(what(&write_err(&record)), "payload_count");
    }

    #[test]
    fn rejects_payload_xlayer_id() {
        let mut bits = global_prefix(2, 0b1, false, false, true, false, false);
        bits.leb128_byte(1);
        bits.extend(minimal_xlayer_info(false));
        let mut record = parse(&bits.into_bytes(), GLOBAL_XLAYER_ID);
        global(&mut record).payloads[0].xlayer_id = 7; // != the map's set-bit id 0
        assert_eq!(what(&write_err(&record)), "payload_xlayer_id");
    }

    #[test]
    fn rejects_payload_size_mismatch() {
        let mut bits = global_prefix(2, 0b1, false, false, true, false, false);
        bits.leb128_byte(1);
        bits.extend(minimal_xlayer_info(false));
        let mut record = parse(&bits.into_bytes(), GLOBAL_XLAYER_ID);
        global(&mut record).payloads[0].remaining_payload_bits = 8; // content already fills data_size*8
        assert_eq!(what(&write_err(&record)), "payload_size");
    }

    #[test]
    fn rejects_num_dependent_gate() {
        let mut bits = global_prefix(1, 0b10, false, false, true, false, false);
        bits.leb128_byte(1);
        bits.extend(minimal_xlayer_info(false));
        let mut record = parse(&bits.into_bytes(), GLOBAL_XLAYER_ID);
        global(&mut record).dependent_xlayers_flag = true; // now n>0 demands a map, but it is None
        assert_eq!(what(&write_err(&record)), "num_dependent_gate");
    }

    #[test]
    fn rejects_local_ptl_gate() {
        let mut record = parse(&minimal_local_body(), ExtendedLayerId::from_bits(2));
        local(&mut record).profile_tier_level_info_present = true; // seq_ptl_info stays None
        assert_eq!(what(&write_err(&record)), "local_ptl_gate");
    }

    #[test]
    fn rejects_embedded_atlas_exclusive() {
        let mut record = parse(&local_embedded_kitchen_sink_body(), ExtendedLayerId::from_bits(1));
        local(&mut record).xlayer_info.xlayer_atlas = Some(LcrXlayerAtlasRef {
            atlas_segment_id: 1,
            priority_order: 1,
            rendering_method: 1,
        });
        assert_eq!(what(&write_err(&record)), "embedded_atlas_exclusive");
    }

    #[test]
    fn rejects_global_record_under_local_header() {
        let record = parse(
            &global_prefix(1, 0b1, false, false, false, false, false).into_bytes(),
            GLOBAL_XLAYER_ID,
        );
        let err = write_err_xlayer(&record, ExtendedLayerId::from_bits(0));
        assert_eq!(what(&err), "xlayer_scope");
    }

    #[test]
    fn rejects_local_record_under_global_header() {
        let record = parse(&minimal_local_body(), ExtendedLayerId::from_bits(2));
        let err = write_err_xlayer(&record, GLOBAL_XLAYER_ID);
        assert_eq!(what(&err), "xlayer_scope");
    }

    #[test]
    fn rejects_local_xlayer_id_disagreeing_with_header() {
        let record = parse(&minimal_local_body(), ExtendedLayerId::from_bits(2));
        let err = write_err_xlayer(&record, ExtendedLayerId::from_bits(5));
        assert_eq!(what(&err), "local_xlayer_id");
    }

    #[test]
    fn rejects_local_ptl_xlayer_id() {
        let mut bits = Bits::default();
        bits.f(0, 3); // lcr_global_id
        bits.f(2, 3); // lcr_local_id
        bits.bit(1); // ptl present
        bits.bit(0); // local atlas present
        bits.ptl(0, 0, 0, 0, 0); // lcr_seq_profile_tier_level_info(xId)
        bits.f(0, 3); // reserved_zero_3bits
        bits.f(0, 5); // reserved_zero_5bits
        bits.extend(minimal_xlayer_info(false));
        let mut record = parse(&bits.into_bytes(), ExtendedLayerId::from_bits(3));
        local(&mut record).seq_ptl_info.as_mut().unwrap().xlayer_id = 9; // != the record's xlayer_id 3
        assert_eq!(what(&write_err(&record)), "local_ptl_xlayer_id");
    }

    #[test]
    fn rejects_xlayer_atlas_gate() {
        let mut bits = Bits::default();
        bits.f(1, 3); // id
        bits.f(0b1, 31); // map
        bits.f(0, 2); // agg/ptl
        bits.bit(1); // payload present
        bits.bit(0); // dependent
        bits.bit(1); // atlas present
        bits.f(0, 7); // purpose
        bits.f(0, 2); // doh/tile
        bits.f(0, 3); // atlas id
        bits.f(0, 5); // reserved
        bits.leb128_byte(4);
        bits.f(0, 4); // xlayer_info flags
        bits.align();
        bits.f(0, 24); // atlas triple
        let mut record = parse(&bits.into_bytes(), GLOBAL_XLAYER_ID);
        global(&mut record).payloads[0].xlayer_info.xlayer_atlas = None;
        assert_eq!(what(&write_err(&record)), "xlayer_atlas_gate");
    }

    #[test]
    fn rejects_mlayer_layer_count() {
        let mut record = parse(&local_embedded_kitchen_sink_body(), ExtendedLayerId::from_bits(1));
        local(&mut record)
            .xlayer_info
            .embedded_layer_info
            .as_mut()
            .unwrap()
            .layers
            .clear(); // map has two set bits, layers now empty
        assert_eq!(what(&write_err(&record)), "mlayer_layer_count");
    }

    #[test]
    fn rejects_mlayer_index() {
        let mut record = parse(&local_embedded_kitchen_sink_body(), ExtendedLayerId::from_bits(1));
        local(&mut record)
            .xlayer_info
            .embedded_layer_info
            .as_mut()
            .unwrap()
            .layers[0]
            .mlayer_index = 5; // != the first set bit (0)
        assert_eq!(what(&write_err(&record)), "mlayer_index");
    }

    #[test]
    fn rejects_embedded_atlas_gate() {
        let mut record = parse(&local_embedded_kitchen_sink_body(), ExtendedLayerId::from_bits(1));
        local(&mut record)
            .xlayer_info
            .embedded_layer_info
            .as_mut()
            .unwrap()
            .layers[0]
            .atlas_segment_id = None; // atlas present for the layer, but a field is missing
        assert_eq!(what(&write_err(&record)), "embedded_atlas_gate");
    }

    #[test]
    fn rejects_aux_type_gate() {
        let mut record = parse(&local_embedded_kitchen_sink_body(), ExtendedLayerId::from_bits(1));
        local(&mut record)
            .xlayer_info
            .embedded_layer_info
            .as_mut()
            .unwrap()
            .layers[0]
            .auxiliary_type = None; // layer_type == AUX_LAYER demands it
        assert_eq!(what(&write_err(&record)), "aux_type_gate");
    }

    #[test]
    fn rejects_view_id_gate() {
        let mut record = parse(&local_embedded_kitchen_sink_body(), ExtendedLayerId::from_bits(1));
        local(&mut record)
            .xlayer_info
            .embedded_layer_info
            .as_mut()
            .unwrap()
            .layers[0]
            .view_id = None; // view_type == VIEW_EXPLICIT demands it
        assert_eq!(what(&write_err(&record)), "view_id_gate");
    }

    #[test]
    fn rejects_dependent_layer_map_gate() {
        let mut record = parse(&local_embedded_kitchen_sink_body(), ExtendedLayerId::from_bits(1));
        local(&mut record)
            .xlayer_info
            .embedded_layer_info
            .as_mut()
            .unwrap()
            .layers[1]
            .dependent_layer_map = None; // j = 1 > 0 demands it
        assert_eq!(what(&write_err(&record)), "dependent_layer_map_gate");
    }

    #[test]
    fn rejects_max_expected_gate() {
        let mut record = parse(&local_embedded_kitchen_sink_body(), ExtendedLayerId::from_bits(1));
        local(&mut record)
            .xlayer_info
            .embedded_layer_info
            .as_mut()
            .unwrap()
            .layers[0]
            .max_expected_height = None; // same_sh flag is clear, so both must be present
        assert_eq!(what(&write_err(&record)), "max_expected_gate");
    }

    #[test]
    fn rejects_color_primaries_gate() {
        let mut record = parse(&local_embedded_kitchen_sink_body(), ExtendedLayerId::from_bits(1));
        local(&mut record).xlayer_info.color_info.as_mut().unwrap().primaries = None; // idc == 0 demands it
        assert_eq!(what(&write_err(&record)), "color_primaries_gate");
    }

    fn minimal_local_body() -> Vec<u8> {
        let mut bits = Bits::default();
        bits.f(3, 3); // lcr_global_id
        bits.f(1, 3); // lcr_local_id
        bits.bit(0); // ptl present
        bits.bit(0); // local atlas present
        bits.f(0, 3); // reserved_zero_3bits
        bits.f(0, 5); // reserved_zero_5bits
        bits.extend(minimal_xlayer_info(false));
        bits.into_bytes()
    }
}

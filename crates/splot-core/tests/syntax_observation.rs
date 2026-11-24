// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! End-to-end verification of `splot_core::syntax` observation across all AV2
//! OBU header parsers: independent bit-span oracles, conditional absence /
//! inferred defaults, truncated/malformed error reporting, and hostile-input
//! limits.

use splot_core::bitio::BitReader;
use splot_core::headers::frame::{
    FrameHeaderParseInput, FrameReferenceStateView, encode_minimal_intra_clk_tile_group_obu,
    encode_minimal_intra_sequence_header_obu,
};
use splot_core::headers::tile_group::{TileGroupLayout, parse_tile_group_structure_observed};
use splot_core::headers::{
    parse_atlas_segment_observed, parse_buffer_removal_timing_observed,
    parse_content_interpretation_observed, parse_film_grain_observed,
    parse_frame_header_core_observed, parse_layer_config_record_observed,
    parse_metadata_group_observed, parse_metadata_short_observed,
    parse_operating_point_set_observed, parse_padding_obu_observed,
    parse_quantizer_matrix_observed, parse_sequence_header_general_observed,
    parse_sequence_header_observed, parse_tile_group_prefix_observed, parse_timing_info_observed,
};
use splot_core::hls::{parse_msdo_observed, parse_multi_frame_header_observed};
use splot_core::obu::{
    ObuHeader, dispatch_obu_payload_observed, read_obu_header_from_slice_observed,
};
use splot_core::span::{BitSpan, ByteOffset};
use splot_core::syntax::{
    SyntaxDescriptor, SyntaxEvent, SyntaxLimits, SyntaxRecorder, SyntaxTrace, SyntaxValue,
};
use splot_core::types::{ExtendedLayerId, GLOBAL_XLAYER_ID, ObuType};

fn assert_scopes_balanced(trace: &SyntaxTrace) {
    let mut depth: isize = 0;
    for event in &trace.events {
        match event {
            SyntaxEvent::EnterScope(_) => depth += 1,
            SyntaxEvent::ExitScope => {
                depth -= 1;
                assert!(depth >= 0, "scope depth went negative");
            }
            _ => {}
        }
    }
    assert_eq!(depth, 0, "unclosed scope in trace");
}

fn assert_contiguous_reads(trace: &SyntaxTrace, expected_total_bits: u64) {
    let mut cursor = 0u64;
    for field in trace.fields_read() {
        assert_eq!(
            field.span.start_bit, cursor,
            "field {} started at bit {}, expected {}",
            field.name, field.span.start_bit, cursor
        );
        cursor = field.span.end_bit();
    }
    assert_eq!(cursor, expected_total_bits);
}

#[test]
fn obu_header_without_and_with_extension_reports_exact_spans_and_inferred_layers() {
    let mut rec = SyntaxRecorder::with_default_limits();
    let hdr = read_obu_header_from_slice_observed(&[0x04], ByteOffset::new(128), &mut rec)
        .expect("valid 1-byte OBU header");
    assert_eq!(hdr.obu_type, ObuType::SequenceHeader);
    let trace = rec.finish();
    assert_scopes_balanced(&trace);
    assert_contiguous_reads(&trace, 8);

    let reads: Vec<_> = trace
        .fields_read()
        .map(|f| (f.name.as_ref(), f.descriptor, f.value, f.span))
        .collect();
    assert_eq!(
        reads,
        vec![
            (
                "obu_header_extension_flag",
                SyntaxDescriptor::F(1),
                SyntaxValue::Bool(false),
                BitSpan::new(0, 1)
            ),
            (
                "obu_type",
                SyntaxDescriptor::F(5),
                SyntaxValue::Unsigned(1),
                BitSpan::new(1, 5)
            ),
            (
                "obu_tlayer_id",
                SyntaxDescriptor::F(2),
                SyntaxValue::Unsigned(0),
                BitSpan::new(6, 2)
            ),
        ]
    );
    let absent: Vec<_> = trace
        .fields_absent()
        .map(|f| (f.name.as_ref(), f.inferred))
        .collect();
    assert_eq!(
        absent,
        vec![
            ("obu_mlayer_id", Some(SyntaxValue::Unsigned(0))),
            ("obu_xlayer_id", Some(SyntaxValue::Unsigned(0))),
        ]
    );

    let mut rec2 = SyntaxRecorder::with_default_limits();
    let hdr2 = read_obu_header_from_slice_observed(&[0x99, 0x65], ByteOffset::new(512), &mut rec2)
        .expect("valid 2-byte OBU header");
    assert!(hdr2.has_header_extension);
    let trace2 = rec2.finish();
    assert_scopes_balanced(&trace2);
    assert_contiguous_reads(&trace2, 16);
    assert_eq!(trace2.fields_absent().count(), 0);
}

#[test]
fn buffer_removal_timing_reports_exact_independent_spans_and_absence() {
    let payload = [0x9A, 0x9C];
    let mut reader = BitReader::new(&payload, ByteOffset::new(4096));
    let mut rec = SyntaxRecorder::with_default_limits();
    let brt = parse_buffer_removal_timing_observed(&mut reader, &mut rec).expect("valid BRT");
    assert!(brt.is_ops_dependent());
    let trace = rec.finish();
    assert_scopes_balanced(&trace);
    assert_contiguous_reads(&trace, 15);

    let reads: Vec<_> = trace
        .fields_read()
        .map(|f| (f.name.as_ref(), f.descriptor, f.value, f.span))
        .collect();
    assert_eq!(
        reads,
        vec![
            (
                "br_ops_dependent_flag",
                SyntaxDescriptor::F(1),
                SyntaxValue::Bool(true),
                BitSpan::new(0, 1)
            ),
            (
                "br_ops_id",
                SyntaxDescriptor::F(4),
                SyntaxValue::Unsigned(3),
                BitSpan::new(1, 4)
            ),
            (
                "br_ops_cnt",
                SyntaxDescriptor::F(3),
                SyntaxValue::Unsigned(2),
                BitSpan::new(5, 3)
            ),
            (
                "br_decoder_model_present_flag",
                SyntaxDescriptor::F(1),
                SyntaxValue::Bool(true),
                BitSpan::new(8, 1)
            ),
            (
                "br_time_op",
                SyntaxDescriptor::Rg(4),
                SyntaxValue::Unsigned(7),
                BitSpan::new(9, 5)
            ),
            (
                "br_decoder_model_present_flag",
                SyntaxDescriptor::F(1),
                SyntaxValue::Bool(false),
                BitSpan::new(14, 1)
            ),
        ]
    );
    let absent: Vec<_> = trace
        .fields_absent()
        .map(|f| (f.name.as_ref(), f.inferred))
        .collect();
    assert!(absent.contains(&("br_time_op", None)));
    assert!(absent.contains(&("br_time", None)));
}

#[test]
fn metadata_short_sub_reader_preserves_obu_payload_relative_bit_spans() {
    let payload = [0x01, 0x01, 0x03, 0xE8, 0x01, 0x90, 0x80];
    let mut reader = BitReader::new(&payload, ByteOffset::new(8192));
    let mut rec = SyntaxRecorder::with_default_limits();
    let parsed = parse_metadata_short_observed(&mut reader, payload.len(), &mut rec)
        .expect("valid metadata");
    assert_eq!(parsed.muh_persistence_idc, 1);
    let trace = rec.finish();
    assert_scopes_balanced(&trace);
    assert_contiguous_reads(&trace, 48);

    let max_cll = trace
        .fields_read()
        .find(|f| f.name == "max_cll")
        .expect("max_cll present");
    assert_eq!(max_cll.span, BitSpan::new(16, 16));
    assert_eq!(max_cll.value, SyntaxValue::Unsigned(1000));
    assert_eq!(max_cll.descriptor, SyntaxDescriptor::F(16));

    let max_fall = trace
        .fields_read()
        .find(|f| f.name == "max_fall")
        .expect("max_fall present");
    assert_eq!(max_fall.span, BitSpan::new(32, 16));
    assert_eq!(max_fall.value, SyntaxValue::Unsigned(400));
}

#[test]
fn sequence_and_frame_headers_observe_full_syntax_and_inferred_fields() {
    let seq_obu = encode_minimal_intra_sequence_header_obu().expect("encode seq obu");
    let seq_header = read_obu_header_from_slice_observed(
        &seq_obu[1..],
        ByteOffset::new(1),
        &mut SyntaxRecorder::with_default_limits(),
    )
    .expect("seq obu header");
    let seq_bytes = &seq_obu[1 + usize::from(seq_header.header_size_bytes)..];
    let mut reader = BitReader::new(seq_bytes, ByteOffset::new(100));
    let mut rec = SyntaxRecorder::with_default_limits();
    let parsed_seq = parse_sequence_header_observed(&mut reader, &mut rec).expect("parse seq");
    assert_eq!(parsed_seq.general.max_frame_width.get(), 64);
    let seq_trace = rec.finish();
    assert_scopes_balanced(&seq_trace);
    assert_contiguous_reads(&seq_trace, reader.consumed_bits());
    assert!(seq_trace.fields_read().any(|f| f.name == "seq_header_id"));
    assert!(
        seq_trace
            .fields_absent()
            .any(|f| f.name == "seq_tier" && f.inferred == Some(SyntaxValue::Unsigned(0)))
    );

    let tg_payload = encode_minimal_intra_clk_tile_group_obu(&[0x00, 0x80]).expect("encode tg");

    let mut prefix_reader = BitReader::new(&tg_payload, ByteOffset::new(0));
    let mut prefix_rec = SyntaxRecorder::with_default_limits();
    let prefix = parse_tile_group_prefix_observed(
        &mut prefix_reader,
        ObuType::ClosedLoopKey,
        Some(true),
        &mut prefix_rec,
    )
    .expect("tg prefix");
    assert!(prefix.is_first_tile_group);
    let prefix_trace = prefix_rec.finish();
    assert_scopes_balanced(&prefix_trace);
    assert_contiguous_reads(&prefix_trace, prefix_reader.consumed_bits());
    assert!(prefix_trace.fields_absent().any(|f| {
        f.name == "frame_header_present_flag" && f.inferred == Some(SyntaxValue::Bool(true))
    }));

    let mut fh_reader = BitReader::new(&tg_payload, ByteOffset::new(0));
    assert!(fh_reader.read_flag().expect("first tg bit"));
    let mut fh_rec = SyntaxRecorder::with_default_limits();
    let input = FrameHeaderParseInput {
        obu_type: ObuType::ClosedLoopKey,
        obu_tlayer_id: splot_core::types::TemporalLayerId::BASE,
        first_picture_in_tu: true,
        active_sequence: Some(&parsed_seq),
        mfh_record: None,
        reference_state: FrameReferenceStateView::unknown(),
    };
    let core = parse_frame_header_core_observed(&mut fh_reader, &input, &mut fh_rec)
        .expect("parse frame header core");
    assert_eq!(core.frame_size.map(|s| (s.width, s.height)), Some((64, 64)));
    let fh_trace = fh_rec.finish();
    assert_scopes_balanced(&fh_trace);
    assert_contiguous_reads(&fh_trace, core.consumed_bits);
    assert!(fh_trace.fields_read().any(|f| f.name == "base_q_idx"));

    let tile_info = core.tile_info.as_ref().expect("tile info");
    let layout = TileGroupLayout::new(
        tile_info.tile_cols,
        tile_info.tile_rows,
        tile_info.tile_cols_log2,
        tile_info.tile_rows_log2,
    );
    let mut tg_struct_rec = SyntaxRecorder::with_default_limits();
    let tg_struct = parse_tile_group_structure_observed(
        &mut fh_reader,
        layout,
        tg_payload.len() as u64,
        &mut tg_struct_rec,
    )
    .expect("tg structure");
    assert!(tg_struct.header_bytes.is_some());
    let tg_struct_trace = tg_struct_rec.finish();
    assert_scopes_balanced(&tg_struct_trace);
    assert!(tg_struct_trace.fields_absent().any(|f| {
        f.name == "tile_start_and_end_present_flag" && f.inferred == Some(SyntaxValue::Bool(false))
    }));
}

#[test]
fn truncated_and_malformed_headers_report_prefix_fields_and_located_error() {
    let truncated = [0b1000_0000];
    let mut reader = BitReader::new(&truncated, ByteOffset::new(0));
    let mut rec = SyntaxRecorder::with_default_limits();
    let res = parse_sequence_header_general_observed(&mut reader, &mut rec);
    assert!(res.is_err());
    let trace = rec.finish();
    assert_scopes_balanced(&trace);

    let read_names: Vec<_> = trace.fields_read().map(|f| f.name.as_ref()).collect();
    assert_eq!(
        read_names,
        vec![
            "seq_header_id",
            "seq_profile_idc",
            "single_picture_header_flag"
        ]
    );
    let err = trace.error.as_ref().expect("located error recorded");
    assert_eq!(err.bit_offset, 7);
    assert_eq!(err.field.as_deref(), Some("seq_level_idx"));

    let zero_tick = [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00];
    let mut t_reader = BitReader::new(&zero_tick, ByteOffset::new(0));
    let mut t_rec = SyntaxRecorder::with_default_limits();
    assert!(parse_timing_info_observed(&mut t_reader, &mut t_rec).is_err());
    let t_trace = t_rec.finish();
    assert_scopes_balanced(&t_trace);
    assert_eq!(t_trace.fields_read().count(), 1);
    let t_err = t_trace.error.as_ref().expect("semantic error recorded");
    assert_eq!(t_err.bit_offset, 0);
    assert_eq!(t_err.field.as_deref(), Some("num_units_in_display_tick"));

    let mut p_rec = SyntaxRecorder::with_default_limits();
    assert!(parse_padding_obu_observed(&[0x00, 0x00], ByteOffset::new(0), &mut p_rec).is_err());
    let p_trace = p_rec.finish();
    assert_scopes_balanced(&p_trace);
    assert_eq!(p_trace.fields_read().count(), 0);
    assert_eq!(p_trace.error.as_ref().map(|e| e.bit_offset), Some(0));
}

#[test]
fn hostile_input_limits_hold_and_still_capture_errors() {
    let seq_obu = encode_minimal_intra_sequence_header_obu().expect("encode seq obu");
    let seq_header = read_obu_header_from_slice_observed(
        &seq_obu[1..],
        ByteOffset::new(1),
        &mut SyntaxRecorder::with_default_limits(),
    )
    .expect("seq obu header");
    let seq_bytes = &seq_obu[1 + usize::from(seq_header.header_size_bytes)..];

    let mut reader = BitReader::new(seq_bytes, ByteOffset::new(0));
    let mut rec = SyntaxRecorder::new(SyntaxLimits::new(5, 1));
    let _ = parse_sequence_header_observed(&mut reader, &mut rec).expect("parse succeeds");
    let trace = rec.finish();
    assert_scopes_balanced(&trace);
    assert!(trace.is_truncated());
    assert!(trace.elements_truncated);
    assert!(trace.depth_truncated);
    assert!(trace.fields_read().count() + trace.fields_absent().count() <= 5);

    let mut bad_reader = BitReader::new(&[0b1000_0000], ByteOffset::new(0));
    let mut zero_rec = SyntaxRecorder::new(SyntaxLimits::new(0, 0));
    assert!(parse_sequence_header_general_observed(&mut bad_reader, &mut zero_rec).is_err());
    let zero_trace = zero_rec.finish();
    assert!(zero_trace.events.is_empty());
    assert!(zero_trace.elements_truncated);
    assert!(zero_trace.error.is_some());
}

#[test]
fn all_other_obu_header_parsers_emit_balanced_traces() {
    let mut ops_r = BitReader::new(&[0x18], ByteOffset::new(0));
    let mut ops_rec = SyntaxRecorder::with_default_limits();
    let _ =
        parse_operating_point_set_observed(&mut ops_r, ExtendedLayerId::from_bits(1), &mut ops_rec)
            .expect("valid OPS reset");
    let ops_trace = ops_rec.finish();
    assert_scopes_balanced(&ops_trace);
    assert_contiguous_reads(&ops_trace, 8);
    assert!(ops_trace.fields_absent().any(|f| f.name == "ops_priority"));

    let lcr_bytes = [0b0000_0100, 0x00, 0x00];
    let mut lcr_r = BitReader::new(&lcr_bytes, ByteOffset::new(0));
    let mut lcr_rec = SyntaxRecorder::with_default_limits();
    let _ =
        parse_layer_config_record_observed(&mut lcr_r, ExtendedLayerId::from_bits(0), &mut lcr_rec)
            .expect("valid local LCR");
    let lcr_trace = lcr_rec.finish();
    assert_scopes_balanced(&lcr_trace);
    assert_contiguous_reads(&lcr_trace, 24);

    let mut ci_r = BitReader::new(&[0b0100_0000], ByteOffset::new(0));
    let mut ci_rec = SyntaxRecorder::with_default_limits();
    let _ = parse_content_interpretation_observed(&mut ci_r, &mut ci_rec).expect("valid CI");
    let ci_trace = ci_rec.finish();
    assert_scopes_balanced(&ci_trace);
    assert_contiguous_reads(&ci_trace, 8);

    let mut qm_r = BitReader::new(&[0x00, 0x00], ByteOffset::new(0));
    let mut qm_rec = SyntaxRecorder::with_default_limits();
    let _ = parse_quantizer_matrix_observed(&mut qm_r, &mut qm_rec).expect("valid QM");
    let qm_trace = qm_rec.finish();
    assert_scopes_balanced(&qm_trace);
    assert_contiguous_reads(&qm_trace, 16);

    let mut fg_r = BitReader::new(&[0x00, 0x80], ByteOffset::new(0));
    let mut fg_rec = SyntaxRecorder::with_default_limits();
    let _ = parse_film_grain_observed(&mut fg_r, &mut fg_rec).expect("valid FG");
    let fg_trace = fg_rec.finish();
    assert_scopes_balanced(&fg_trace);
    assert_contiguous_reads(&fg_trace, 9);

    let mut ats_r = BitReader::new(&[0b0000_1111, 0b0000_0000], ByteOffset::new(0));
    let mut ats_rec = SyntaxRecorder::with_default_limits();
    let _ = parse_atlas_segment_observed(&mut ats_r, &mut ats_rec).expect("valid Atlas");
    let ats_trace = ats_rec.finish();
    assert_scopes_balanced(&ats_trace);
    assert_contiguous_reads(&ats_trace, 9);

    let mut pad_rec = SyntaxRecorder::with_default_limits();
    let _ = parse_padding_obu_observed(&[0xAA, 0xBB, 0x80], ByteOffset::new(0), &mut pad_rec)
        .expect("valid padding");
    let pad_trace = pad_rec.finish();
    assert_scopes_balanced(&pad_trace);
    assert_contiguous_reads(&pad_trace, 24);

    let msdo_bytes = [0x00, 0b0000_0010, 0x00, 0x00, 0x00, 0x00];
    let mut msdo_r = BitReader::new(&msdo_bytes, ByteOffset::new(0));
    let mut msdo_rec = SyntaxRecorder::with_default_limits();
    let _ = parse_msdo_observed(&mut msdo_r, &mut msdo_rec).expect("valid MSDO");
    let msdo_trace = msdo_rec.finish();
    assert_scopes_balanced(&msdo_trace);
    assert_contiguous_reads(&msdo_trace, 48);

    let mut mfh_r = BitReader::new(&[0b1100_0000], ByteOffset::new(0));
    let mut mfh_rec = SyntaxRecorder::with_default_limits();
    let _ = parse_multi_frame_header_observed(&mut mfh_r, &mut mfh_rec).expect("valid MFH");
    let mfh_trace = mfh_rec.finish();
    assert_scopes_balanced(&mfh_trace);
    assert_contiguous_reads(&mfh_trace, 5);

    let mg_bytes = [0x00, 0x00, 0x02, 0x01];
    let mut mg_r = BitReader::new(&mg_bytes, ByteOffset::new(0));
    let mut mg_rec = SyntaxRecorder::with_default_limits();
    let _ = parse_metadata_group_observed(&mut mg_r, GLOBAL_XLAYER_ID, &mut mg_rec)
        .expect("valid metadata group");
    let mg_trace = mg_rec.finish();
    assert_scopes_balanced(&mg_trace);
    assert_contiguous_reads(&mg_trace, 32);

    let hdr = ObuHeader {
        has_header_extension: false,
        obu_type: ObuType::ContentInterpretation,
        temporal_layer_id: splot_core::types::TemporalLayerId::from_bits(0),
        embedded_layer_id: splot_core::types::EmbeddedLayerId::from_bits(0),
        extended_layer_id: ExtendedLayerId::from_bits(0),
        header_size_bytes: 1,
    };
    let mut disp_rec = SyntaxRecorder::with_default_limits();
    let _ = dispatch_obu_payload_observed(
        hdr,
        &[0b0100_0000, 0b0100_0000],
        ByteOffset::new(16),
        &mut disp_rec,
    )
    .expect("dispatch CI");
    let disp_trace = disp_rec.finish();
    assert_scopes_balanced(&disp_trace);
    assert!(
        disp_trace
            .fields_read()
            .any(|f| f.name == "trailing_one_bit")
    );
}

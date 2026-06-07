// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `splot inspect` — print OBUs and headers from a bitstream.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use clap::Args;
use serde::Serialize;
use splot_core::annexb::{ObuEnvelope, parse_annex_b_obus_partial};
use splot_core::bitio::BitReader;
use splot_core::headers::buffer_removal_timing::{
    BufferRemovalTiming, parse_buffer_removal_timing,
};
use splot_core::headers::content_interpretation::{
    ContentInterpretation, parse_content_interpretation,
};
use splot_core::headers::frame::{FrameHeaderPrefix, parse_frame_header_prefix};
use splot_core::headers::operating_point_set::{OperatingPointSet, parse_operating_point_set};
use splot_core::headers::sequence::{SequenceHeader, parse_sequence_header};
use splot_core::headers::tile_group::parse_tile_group_prefix;
use splot_core::obu::{ObuHeader, PayloadStatus};
use splot_core::types::ObuType;

use crate::commands::read_input;

/// Arguments for `splot inspect`.
#[derive(Args, Debug)]
pub struct InspectArgs {
    /// Path to the AV2 length-delimited bitstream.
    pub input: PathBuf,
    /// Emit the inspection as JSON.
    #[arg(long)]
    pub json: bool,
    /// Print only OBU headers (omit payload sizes in text output).
    #[arg(long)]
    pub headers: bool,
}

/// A serializable summary of one OBU for `--json` output.
#[derive(Serialize)]
struct InspectRecord {
    index: usize,
    byte_offset: u64,
    size: u32,
    payload_len: usize,
    payload_status: InspectPayloadStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    sequence_header: Option<SequenceHeaderView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_interpretation: Option<ContentInterpretationView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    operating_point_set: Option<OperatingPointSetView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    buffer_removal_timing: Option<BufferRemovalTimingView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frame_header_prefix: Option<FrameHeaderPrefixView>,
    header: ObuHeader,
}

impl InspectRecord {
    fn new(index: usize, obu: &ObuEnvelope<'_>) -> Self {
        Self {
            index,
            byte_offset: obu.offset.get(),
            size: obu.size,
            payload_len: obu.payload.len(),
            payload_status: InspectPayloadStatus::new(obu),
            sequence_header: sequence_header_view(obu),
            content_interpretation: content_interpretation_view(obu),
            operating_point_set: operating_point_set_view(obu),
            buffer_removal_timing: buffer_removal_timing_view(obu),
            frame_header_prefix: frame_header_prefix_view(obu),
            header: obu.header,
        }
    }
}

/// Re-parses an `operating_point_set_obu()` so `--json` can expose its key fields.
fn operating_point_set_view(obu: &ObuEnvelope<'_>) -> Option<OperatingPointSetView> {
    if obu.header.obu_type != ObuType::OperatingPointSet {
        return None;
    }
    let mut reader = BitReader::new(obu.payload, obu.payload_offset());
    parse_operating_point_set(&mut reader, obu.header.extended_layer_id)
        .ok()
        .map(|ops| OperatingPointSetView::new(&ops))
}

/// A compact, machine-readable view of a parsed `operating_point_set_obu()`.
#[derive(Serialize)]
struct OperatingPointSetView {
    xlayer_id: u8,
    is_global: bool,
    reset_flag: bool,
    ops_id: u8,
    ops_cnt: u8,
    payload_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    mlayer_info_idc: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    local_reserved_2bits: Option<u8>,
}

impl OperatingPointSetView {
    fn new(ops: &OperatingPointSet) -> Self {
        Self {
            xlayer_id: ops.xlayer_id.get(),
            is_global: ops.is_global(),
            reset_flag: ops.reset_flag,
            ops_id: ops.ops_id,
            ops_cnt: ops.ops_cnt,
            payload_count: ops.payloads.len(),
            mlayer_info_idc: ops.mlayer_info_idc,
            local_reserved_2bits: ops.local_reserved_2bits,
        }
    }
}

/// Re-parses a `buffer_removal_timing_obu()` so `--json` can expose its key fields.
fn buffer_removal_timing_view(obu: &ObuEnvelope<'_>) -> Option<BufferRemovalTimingView> {
    if obu.header.obu_type != ObuType::BufferRemovalTiming {
        return None;
    }
    let mut reader = BitReader::new(obu.payload, obu.payload_offset());
    parse_buffer_removal_timing(&mut reader)
        .ok()
        .map(|brt| BufferRemovalTimingView::new(&brt))
}

/// A compact, machine-readable view of a parsed `buffer_removal_timing_obu()`.
#[derive(Serialize)]
struct BufferRemovalTimingView {
    ops_dependent: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    br_ops_id: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    br_ops_cnt: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    op_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    br_time: Option<u32>,
}

impl BufferRemovalTimingView {
    fn new(brt: &BufferRemovalTiming) -> Self {
        let ops_reference = brt.ops_reference();
        Self {
            ops_dependent: brt.is_ops_dependent(),
            br_ops_id: ops_reference.map(|(id, _)| id),
            br_ops_cnt: ops_reference.map(|(_, cnt)| cnt),
            op_count: ops_reference.map(|_| brt.op_timings().len()),
            br_time: brt.extended_layer_time(),
        }
    }
}

/// Re-parses a frame-bearing OBU's prefix so `--json` can expose the
/// activation/reference fields. This is **prefix-only** data, never a complete frame
/// header. The inspector does not model temporal-unit state, so `FirstPictureInTU` is
/// passed as `false` and `startCVS` is not surfaced.
fn frame_header_prefix_view(obu: &ObuEnvelope<'_>) -> Option<FrameHeaderPrefixView> {
    let obu_type = obu.header.obu_type;
    let mut reader = BitReader::new(obu.payload, obu.payload_offset());
    let prefix = if obu_type.is_tile_group() {
        parse_tile_group_prefix(&mut reader, obu_type, false)
            .ok()
            .and_then(|tile_group| tile_group.frame_header)?
    } else if obu_type.is_sef() || obu_type.is_tip_frame() || obu_type == ObuType::BridgeFrame {
        parse_frame_header_prefix(&mut reader, obu_type, false).ok()?
    } else {
        return None;
    };
    Some(FrameHeaderPrefixView::new(&prefix))
}

/// A prefix-only view of a parsed `frame_header_info()` for `--json`. The
/// `payload_kind` / `prefix_status` labels make explicit that this is not a complete
/// frame header (AV2 § 5.18 is only prefix-parsed).
#[derive(Serialize)]
struct FrameHeaderPrefixView {
    payload_kind: &'static str,
    prefix_status: &'static str,
    cur_mfh_id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    seq_header_id_in_frame_header: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    referenced_sequence_header_id: Option<u8>,
    is_key_frame: bool,
    is_bridge: bool,
    is_regular: bool,
}

impl FrameHeaderPrefixView {
    fn new(prefix: &FrameHeaderPrefix) -> Self {
        Self {
            payload_kind: "frame_header_prefix",
            prefix_status: prefix.status.label(),
            cur_mfh_id: prefix.cur_mfh_id.get(),
            seq_header_id_in_frame_header: prefix.seq_header_id_in_frame_header,
            referenced_sequence_header_id: prefix.referenced_sequence_header_id.map(|id| id.get()),
            is_key_frame: prefix.is_key_frame,
            is_bridge: prefix.is_bridge,
            is_regular: prefix.is_regular,
        }
    }
}

/// Re-parses a content-interpretation OBU so `--json` can expose its parsed flags
/// and timing status.
fn content_interpretation_view(obu: &ObuEnvelope<'_>) -> Option<ContentInterpretationView> {
    if obu.header.obu_type != ObuType::ContentInterpretation {
        return None;
    }
    let mut reader = BitReader::new(obu.payload, obu.payload_offset());
    parse_content_interpretation(&mut reader)
        .ok()
        .map(|ci| ContentInterpretationView::new(&ci))
}

/// A compact, machine-readable view of a parsed `content_interpretation_obu()`.
#[derive(Serialize)]
struct ContentInterpretationView {
    scan_type_idc: u8,
    color_description_present: bool,
    chroma_sample_position_present: bool,
    aspect_ratio_info_present: bool,
    timing_info_present: bool,
    reserved_2bit: u8,
}

impl ContentInterpretationView {
    fn new(ci: &ContentInterpretation) -> Self {
        Self {
            scan_type_idc: ci.scan_type_idc.get(),
            color_description_present: ci.color_description.is_some(),
            chroma_sample_position_present: ci.chroma_sample_position.is_some(),
            aspect_ratio_info_present: ci.aspect_ratio.is_some(),
            timing_info_present: ci.timing_info.is_some(),
            reserved_2bit: ci.reserved_2bit,
        }
    }
}

/// Re-parses a sequence-header OBU so `--json` can expose which `§5.4` child
/// sections were parsed and which (if any) are bounded as unimplemented.
fn sequence_header_view(obu: &ObuEnvelope<'_>) -> Option<SequenceHeaderView> {
    if obu.header.obu_type != ObuType::SequenceHeader {
        return None;
    }
    let mut reader = BitReader::new(obu.payload, obu.payload_offset());
    parse_sequence_header(&mut reader)
        .ok()
        .map(|header| SequenceHeaderView::new(&header))
}

/// A compact, machine-readable view of a parsed `sequence_header_obu()`.
#[derive(Serialize)]
struct SequenceHeaderView {
    seq_header_id: u8,
    seq_profile_idc: u8,
    single_picture_header_flag: bool,
    chroma_format_idc: u8,
    bit_depth: u8,
    max_tlayer_id: u8,
    max_mlayer_id: u8,
    fully_parsed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    unimplemented_at: Option<&'static str>,
    children: SequenceChildrenView,
}

impl SequenceHeaderView {
    fn new(header: &SequenceHeader) -> Self {
        let general = &header.general;
        Self {
            seq_header_id: general.seq_header_id.get(),
            seq_profile_idc: general.seq_profile_idc.get(),
            single_picture_header_flag: general.single_picture_header_flag,
            chroma_format_idc: general.chroma_format_idc.get(),
            bit_depth: general.bit_depth_idc.bit_depth(),
            max_tlayer_id: general.max_tlayer_id.get(),
            max_mlayer_id: general.max_mlayer_id.get(),
            fully_parsed: header.is_fully_parsed(),
            unimplemented_at: header.unimplemented_at,
            children: SequenceChildrenView {
                partition: header.partition.is_some(),
                segment: header.segment.is_some(),
                intra: header.intra.is_some(),
                inter: header.inter.is_some(),
                screen_content: header.screen_content.is_some(),
                transform_quant_entropy: header.transform_quant_entropy.is_some(),
                filter: header.filter.is_some(),
                tile: header.tile.is_some(),
                film_grain_params_present: header.film_grain_params_present.is_some(),
            },
        }
    }
}

/// Which `§5.4` child sections of a sequence header were parsed.
#[derive(Serialize)]
struct SequenceChildrenView {
    partition: bool,
    segment: bool,
    intra: bool,
    inter: bool,
    screen_content: bool,
    transform_quant_entropy: bool,
    filter: bool,
    tile: bool,
    film_grain_params_present: bool,
}

/// A serializable summary of how much OBU payload syntax is currently parsed.
#[derive(Serialize)]
struct InspectPayloadStatus {
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    syntax: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    feature: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl InspectPayloadStatus {
    fn new(obu: &ObuEnvelope<'_>) -> Self {
        match obu.payload_status() {
            Ok(PayloadStatus::Parsed(parsed)) => Self {
                status: "parsed",
                syntax: Some(parsed.syntax_name()),
                feature: Some(parsed.feature_id()),
                error: None,
            },
            Ok(PayloadStatus::Opaque(_)) => Self {
                status: "opaque",
                syntax: None,
                feature: None,
                error: None,
            },
            Ok(PayloadStatus::Unimplemented { feature, .. }) => Self {
                status: "unimplemented",
                syntax: None,
                feature: Some(feature),
                error: None,
            },
            Err(error) => Self {
                status: "invalid",
                syntax: None,
                feature: Some("AV2-5.2.1-OBU-DISPATCH"),
                error: Some(error.to_string()),
            },
        }
    }
}

/// Runs `splot inspect`.
///
/// Exit codes: `0` on success, `1` if a structural parse error is hit (the OBUs
/// parsed before it are still printed), and `2` (via an `Err`) for I/O failures.
///
/// # Errors
/// Returns an error if the input file cannot be read or the output cannot be
/// serialized.
pub fn run(args: &InspectArgs) -> Result<ExitCode> {
    let data = read_input(&args.input)?;
    // Use the partial parse so the OBUs before a malformed tail are still shown.
    let parsed = parse_annex_b_obus_partial(&data);

    if args.json {
        let records: Vec<InspectRecord> = parsed
            .obus
            .iter()
            .enumerate()
            .map(|(index, obu)| InspectRecord::new(index, obu))
            .collect();
        let json =
            serde_json::to_string_pretty(&records).context("failed to serialize inspection")?;
        println!("{json}");
    } else {
        print_human(&parsed.obus, args.headers);
    }

    if let Some(error) = parsed.error {
        eprintln!("error: failed to parse remainder of bitstream: {error}");
        Ok(ExitCode::from(1))
    } else {
        Ok(ExitCode::from(0))
    }
}

fn print_human(obus: &[ObuEnvelope<'_>], headers_only: bool) {
    println!("{} OBU(s)", obus.len());
    for (index, obu) in obus.iter().enumerate() {
        let header = &obu.header;
        println!(
            "OBU #{index}  @byte {}  size={}  type={}({})  ext={}  tlayer={} mlayer={} xlayer={}",
            obu.offset,
            obu.size,
            header.obu_type.spec_name(),
            header.obu_type.raw(),
            header.has_header_extension,
            header.temporal_layer_id.get(),
            header.embedded_layer_id.get(),
            header.extended_layer_id.get(),
        );
        if !headers_only {
            println!("        payload: {} byte(s)", obu.payload.len());
        }
    }
}

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
use splot_core::headers::content_interpretation::{
    ContentInterpretation, parse_content_interpretation,
};
use splot_core::headers::sequence::{SequenceHeader, parse_sequence_header};
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
            header: obu.header,
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

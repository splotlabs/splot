// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `splot decode` — experimental decoder for the currently supported AV2 subset.

use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read as _, Write as _};
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Context as _, Result};
use clap::{Args, ValueEnum};
use serde::Serialize;
use splot_core::simd::SimdBackend;
use splot_core::span::ByteOffset;
use splot_core::stream_reader::DEFAULT_MAX_UNIT_BYTES;
use splot_decode::{
    DecodeContext, DecodeDiagnostic, DecodeDiagnosticDetails, DecodeDiagnosticReport, DecodeError,
    DecodeHashEntry, DecodeHashFrame, DecodeHashReport, DecodeOptions, DecodeOutputError,
    DecodeOutputOperation, DecodeRuntimeConfig, DecodeScope, EmbeddedLayerId,
    EmbeddedLayerSelection, ExtendedLayerId, ExtendedLayerSelection, ExternalAtlases,
    OperatingPointSelection, TemporalLayerId, TemporalLayerSelection, TypedDecodeContext,
    Y4mFrameRate,
};
use splot_parallel::{FrameDelay, ThreadCount};

use super::atlas::PictureAtlasView;

static OUTPUT_TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Output artifact selected for `splot decode`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
#[value(rename_all = "kebab-case")]
enum DecodeOutputFormat {
    /// Runtime Y4M decoded-video output.
    Y4m,
    /// Headerless raw decoded sample output.
    Raw,
    /// Deterministic decoded-frame hash output.
    Hash,
    /// Decode all requested frames without producing an output artifact.
    Null,
}

impl DecodeOutputFormat {
    fn as_str(self) -> &'static str {
        match self {
            Self::Y4m => "y4m",
            Self::Raw => "raw",
            Self::Hash => "hash",
            Self::Null => "null",
        }
    }
}

/// Arguments for `splot decode`.
#[derive(Args, Debug)]
pub(crate) struct DecodeArgs {
    /// Emit the unsupported decode diagnostic as JSON.
    #[arg(long)]
    json: bool,
    /// Select the decoded output mode.
    #[arg(
        long = "output-format",
        value_enum,
        requires_if("y4m", "output"),
        requires_if("raw", "output")
    )]
    output_format: Option<DecodeOutputFormat>,
    /// Supply one length-framed atlas OBU from this file. Repeat for additional atlases.
    #[arg(long, value_name = "FILE")]
    external_atlas: Vec<PathBuf>,
    /// Input AV2 bitstream.
    input: PathBuf,
    /// Output path for the selected artifact; `-` writes to stdout.
    #[arg(short = 'o', long, required_unless_present = "output_format")]
    output: Option<PathBuf>,
    /// Override the Y4M frame rate as `NUM:DEN`; required for raw Annex B Y4M output.
    #[arg(long, value_name = "NUM:DEN", value_parser = parse_y4m_frame_rate)]
    frame_rate: Option<Y4mFrameRate>,
    /// Worker-thread policy: `auto` (default), a positive integer, or `0` (alias for auto).
    #[arg(long, default_value_t = ThreadCount::Auto)]
    threads: ThreadCount,
    /// Frame-pipelining depth: `auto` (default) resolves to the worker count, while a positive
    /// integer requests an explicit depth. Every depth uses the same admission/task path; the
    /// effective capacity is the requested depth capped by the worker count. `0` aliases auto.
    /// At low worker counts, each in-flight single-tile inter frame may temporarily retain about one extra decoded-frame-sized buffer plus parsed data for the full frame.
    #[arg(long, default_value_t = FrameDelay::Auto)]
    frame_delay: FrameDelay,
    /// Stop after emitting this many output frames.
    #[arg(long)]
    limit: Option<NonZeroU64>,
    /// Output frames without film grain synthesis. Hash output never has it.
    #[arg(long)]
    skip_film_grain: bool,
    /// Temporal enhancement layers to decode and emit: `all` (default) or `0..=3`
    /// (layers `0..=LAYER`).
    #[arg(
        long,
        value_name = "LAYER",
        value_parser = parse_temporal_layer,
        conflicts_with = "temporal_output_mask"
    )]
    temporal_layer: Option<TemporalLayerSelection>,
    /// Bitmask (`1..=15` or `0x1..=0xf`) of temporal layers to emit; transitive
    /// dependencies still decode.
    #[arg(
        long,
        value_name = "MASK",
        value_parser = parse_temporal_output_mask,
        conflicts_with = "temporal_layer"
    )]
    temporal_output_mask: Option<TemporalLayerSelection>,
    /// Embedded layer to decode and emit: `all` (default) or `0..=7`.
    #[arg(
        long,
        value_name = "LAYER",
        value_parser = parse_embedded_layer,
        conflicts_with = "embedded_output_mask"
    )]
    embedded_layer: Option<EmbeddedLayerSelection>,
    /// Bitmask (`1..=255` or `0x1..=0xff`) of embedded layers to emit;
    /// transitive dependencies still decode.
    #[arg(
        long,
        value_name = "MASK",
        value_parser = parse_embedded_output_mask,
        conflicts_with = "embedded_layer"
    )]
    embedded_output_mask: Option<EmbeddedLayerSelection>,
    /// Extended layer to decode and emit: `all` (default) or `0..=30`.
    #[arg(
        long,
        value_name = "LAYER",
        value_parser = parse_extended_layer,
        conflicts_with = "extended_output_mask"
    )]
    extended_layer: Option<ExtendedLayerSelection>,
    /// Bitmask (`1..=2147483647` or `0x1..=0x7fffffff`) of extended layers
    /// to emit.
    #[arg(
        long,
        value_name = "MASK",
        value_parser = parse_extended_output_mask,
        conflicts_with = "extended_layer"
    )]
    extended_output_mask: Option<ExtendedLayerSelection>,
    /// Operating point to decode: `global:OPS_ID:OP_INDEX` or
    /// `local:XLAYER:OPS_ID:OP_INDEX`.
    #[arg(
        long,
        value_name = "SPEC",
        value_parser = parse_operating_point,
        conflicts_with_all = [
            "temporal_layer",
            "temporal_output_mask",
            "embedded_layer",
            "embedded_output_mask",
            "extended_layer",
            "extended_output_mask",
        ]
    )]
    operating_point: Option<OperatingPointSelection>,
}

fn parse_layer_arg(value: &str, max: u8, name: &str) -> core::result::Result<Option<u8>, String> {
    if value.eq_ignore_ascii_case("all") {
        return Ok(None);
    }
    match value.parse::<u8>() {
        Ok(id) if id <= max => Ok(Some(id)),
        _ => Err(format!(
            "{name} layer must be 'all' or an integer in 0..={max}"
        )),
    }
}

fn parse_temporal_layer(value: &str) -> core::result::Result<TemporalLayerSelection, String> {
    Ok(
        parse_layer_arg(value, 3, "temporal")?.map_or(TemporalLayerSelection::All, |id| {
            TemporalLayerSelection::Target(TemporalLayerId::from_bits(id))
        }),
    )
}

fn parse_embedded_layer(value: &str) -> core::result::Result<EmbeddedLayerSelection, String> {
    Ok(
        parse_layer_arg(value, 7, "embedded")?.map_or(EmbeddedLayerSelection::All, |id| {
            EmbeddedLayerSelection::Target(EmbeddedLayerId::from_bits(id))
        }),
    )
}

fn parse_extended_layer(value: &str) -> core::result::Result<ExtendedLayerSelection, String> {
    Ok(
        parse_layer_arg(value, 30, "extended")?.map_or(ExtendedLayerSelection::All, |id| {
            ExtendedLayerSelection::Target(ExtendedLayerId::from_bits(id))
        }),
    )
}

fn parse_output_mask(value: &str, max: u32, name: &str) -> core::result::Result<u32, String> {
    let mask = if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        u32::from_str_radix(hex, 16)
    } else {
        value.parse::<u32>()
    }
    .map_err(|_| format!("{name} output mask must be an integer in 1..={max}"))?;
    if mask == 0 || mask > max {
        return Err(format!(
            "{name} output mask must be non-zero within 1..={max}"
        ));
    }
    Ok(mask)
}

fn parse_temporal_output_mask(value: &str) -> core::result::Result<TemporalLayerSelection, String> {
    Ok(TemporalLayerSelection::OutputMask(
        parse_output_mask(value, 0x0F, "temporal")? as u8,
    ))
}

fn parse_embedded_output_mask(value: &str) -> core::result::Result<EmbeddedLayerSelection, String> {
    Ok(EmbeddedLayerSelection::OutputMask(
        parse_output_mask(value, u32::from(u8::MAX), "embedded")? as u8,
    ))
}

fn parse_extended_output_mask(value: &str) -> core::result::Result<ExtendedLayerSelection, String> {
    Ok(ExtendedLayerSelection::OutputMask(parse_output_mask(
        value,
        0x7FFF_FFFF,
        "extended",
    )?))
}

fn parse_u8_bounded(value: &str, max: u8, name: &str) -> core::result::Result<u8, String> {
    match value.parse::<u8>() {
        Ok(parsed) if parsed <= max => Ok(parsed),
        _ => Err(format!("{name} must be an integer in 0..={max}")),
    }
}

fn parse_operating_point(value: &str) -> core::result::Result<OperatingPointSelection, String> {
    let parts: Vec<&str> = value.split(':').collect();
    match parts.as_slice() {
        [scope, ops_id, op_index] if scope.eq_ignore_ascii_case("global") => {
            Ok(OperatingPointSelection::Global {
                ops_id: parse_u8_bounded(ops_id, 15, "ops_id")?,
                op_index: parse_u8_bounded(op_index, 6, "op_index")?,
            })
        }
        [scope, xlayer, ops_id, op_index] if scope.eq_ignore_ascii_case("local") => {
            let xlayer_id = parse_u8_bounded(xlayer, 30, "extended layer")?;
            Ok(OperatingPointSelection::Local {
                xlayer_id: ExtendedLayerId::from_bits(xlayer_id),
                ops_id: parse_u8_bounded(ops_id, 15, "ops_id")?,
                op_index: parse_u8_bounded(op_index, 6, "op_index")?,
            })
        }
        _ => Err(
            "operating point must be 'global:OPS_ID:OP_INDEX' or 'local:XLAYER:OPS_ID:OP_INDEX'"
                .to_owned(),
        ),
    }
}

fn parse_y4m_frame_rate(value: &str) -> core::result::Result<Y4mFrameRate, String> {
    let (numerator, denominator) = value
        .split_once(':')
        .ok_or_else(|| "frame rate must use NUM:DEN syntax".to_owned())?;
    let numerator = numerator
        .parse::<u32>()
        .map_err(|_| "frame-rate numerator must be a 32-bit unsigned integer".to_owned())?;
    let denominator = denominator
        .parse::<u32>()
        .map_err(|_| "frame-rate denominator must be a 32-bit unsigned integer".to_owned())?;
    Y4mFrameRate::new(numerator, denominator).map_err(|error| error.to_string())
}

#[derive(Debug)]
enum DecodeOutputTarget<'a> {
    Y4m { path: &'a Path },
    Raw { path: &'a Path },
    Hash,
    Null,
}

impl DecodeOutputTarget<'_> {
    fn format(&self) -> DecodeOutputFormat {
        match self {
            Self::Y4m { .. } => DecodeOutputFormat::Y4m,
            Self::Raw { .. } => DecodeOutputFormat::Raw,
            Self::Hash => DecodeOutputFormat::Hash,
            Self::Null => DecodeOutputFormat::Null,
        }
    }
}

impl DecodeArgs {
    fn output_target(&self) -> Option<DecodeOutputTarget<'_>> {
        match (
            self.output_format.unwrap_or(DecodeOutputFormat::Y4m),
            self.output.as_deref(),
        ) {
            (DecodeOutputFormat::Y4m, Some(path)) => Some(DecodeOutputTarget::Y4m { path }),
            (DecodeOutputFormat::Raw, Some(path)) => Some(DecodeOutputTarget::Raw { path }),
            (DecodeOutputFormat::Y4m | DecodeOutputFormat::Raw, None) => None,
            (DecodeOutputFormat::Hash, _) => Some(DecodeOutputTarget::Hash),
            (DecodeOutputFormat::Null, _) => Some(DecodeOutputTarget::Null),
        }
    }
}

fn render_text_diagnostic(report: &DecodeDiagnosticReport, output_format: DecodeOutputFormat) {
    let diagnostic = &report.diagnostic;
    render_text_base(diagnostic);
    eprintln!("output_format: {}", output_format.as_str());
    match &report.details {
        DecodeDiagnosticDetails::MalformedSource(details) => {
            eprintln!("detail_kind: malformed_source");
            eprintln!("source_issue_kind: {}", details.source_issue_kind);
            eprintln!(
                "parser_rule_id: {}",
                details.parser_rule_id.unwrap_or_default()
            );
            eprintln!("byte_offset: {}", option_u64_text(details.byte_offset));
            eprintln!(
                "ivf_frame_index: {}",
                option_usize_text(details.frame_index)
            );
            eprintln!("parser_message: {}", details.parser_message);
        }
        DecodeDiagnosticDetails::ResourceLimit(details) => {
            eprintln!("detail_kind: resource_limit");
            eprintln!("limit_name: {}", details.limit_name);
            eprintln!("limit: {}", option_u64_text(details.limit));
            eprintln!("actual: {}", option_u64_text(details.actual));
            eprintln!("unit: {}", details.unit);
            eprintln!("byte_offset: {}", option_u64_text(details.byte_offset));
            eprintln!("bit_offset: {}", option_u64_text(details.bit_offset));
        }
        DecodeDiagnosticDetails::UnsupportedStructure(details) => {
            eprintln!("detail_kind: unsupported_structure");
            eprintln!("unsupported_reason: {}", details.unsupported_reason);
            eprintln!("obu_type: {}", details.obu_type);
            eprintln!("byte_offset: {}", details.byte_offset);
        }
        DecodeDiagnosticDetails::UnsupportedFeature(details) => {
            eprintln!("detail_kind: unsupported_feature");
            eprintln!("unsupported_reason: {}", details.unsupported_reason);
            eprintln!("byte_offset: {}", option_u64_text(details.byte_offset));
        }
        DecodeDiagnosticDetails::OutputError(details) => {
            eprintln!("detail_kind: output_error");
            eprintln!("output_operation: {}", details.operation);
            eprintln!("output_source_kind: {}", details.source_kind);
            eprintln!("output_source_message: {}", details.source_message);
        }
        _ => {
            eprintln!("detail_kind: unknown");
        }
    }
}

fn render_text_base(diagnostic: &DecodeDiagnostic) {
    eprintln!("rule_id: {}", diagnostic.rule_id);
    eprintln!("severity: {}", diagnostic.severity);
    eprintln!("spec_section: {}", spec_section_text(diagnostic));
    eprintln!("message: {}", diagnostic.message);
}

fn spec_section_text(diagnostic: &DecodeDiagnostic) -> &'static str {
    diagnostic.spec_section.unwrap_or_default()
}

fn option_u64_text(value: Option<u64>) -> String {
    value.map(|value| value.to_string()).unwrap_or_default()
}

fn option_usize_text(value: Option<usize>) -> String {
    value.map(|value| value.to_string()).unwrap_or_default()
}

#[derive(Serialize)]
struct DecodeDiagnosticJson<'a> {
    rule_id: &'a str,
    severity: &'a str,
    spec_section: &'a str,
    message: &'a str,
    output_format: &'a str,
    detail_kind: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_issue_kind: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parser_rule_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parser_message: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    limit_name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    limit: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    actual: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    unit: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    unsupported_reason: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_operation: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_source_kind: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_source_message: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    obu_type: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    byte_offset: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bit_offset: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ivf_frame_index: Option<usize>,
}

impl<'a> DecodeDiagnosticJson<'a> {
    fn new(report: &'a DecodeDiagnosticReport, output_format: DecodeOutputFormat) -> Self {
        let diagnostic = &report.diagnostic;
        let mut json = Self {
            rule_id: diagnostic.rule_id,
            severity: diagnostic.severity.as_str(),
            spec_section: spec_section_text(diagnostic),
            message: diagnostic.message,
            output_format: output_format.as_str(),
            detail_kind: "",
            source_issue_kind: None,
            parser_rule_id: None,
            parser_message: None,
            limit_name: None,
            limit: None,
            actual: None,
            unit: None,
            unsupported_reason: None,
            output_operation: None,
            output_source_kind: None,
            output_source_message: None,
            obu_type: None,
            byte_offset: None,
            bit_offset: None,
            ivf_frame_index: None,
        };

        match &report.details {
            DecodeDiagnosticDetails::MalformedSource(details) => {
                json.detail_kind = "malformed_source";
                json.source_issue_kind = Some(details.source_issue_kind);
                json.parser_rule_id = details.parser_rule_id;
                json.parser_message = Some(&details.parser_message);
                json.byte_offset = details.byte_offset;
                json.ivf_frame_index = details.frame_index;
            }
            DecodeDiagnosticDetails::ResourceLimit(details) => {
                json.detail_kind = "resource_limit";
                json.limit_name = Some(details.limit_name);
                json.limit = details.limit;
                json.actual = details.actual;
                json.unit = Some(details.unit);
                json.byte_offset = details.byte_offset;
                json.bit_offset = details.bit_offset;
            }
            DecodeDiagnosticDetails::UnsupportedStructure(details) => {
                json.detail_kind = "unsupported_structure";
                json.unsupported_reason = Some(details.unsupported_reason);
                json.obu_type = Some(details.obu_type);
                json.byte_offset = Some(details.byte_offset);
            }
            DecodeDiagnosticDetails::UnsupportedFeature(details) => {
                json.detail_kind = "unsupported_feature";
                json.unsupported_reason = Some(details.unsupported_reason);
                json.byte_offset = details.byte_offset;
            }
            DecodeDiagnosticDetails::OutputError(details) => {
                json.detail_kind = "output_error";
                json.output_operation = Some(details.operation);
                json.output_source_kind = Some(details.source_kind);
                json.output_source_message = Some(&details.source_message);
            }
            _ => {
                json.detail_kind = "unknown";
            }
        }

        json
    }
}

#[derive(Serialize)]
struct DecodeHashReportJson<'a> {
    contract_id: &'a str,
    contract_version: u32,
    selected_output_variants: Vec<&'static str>,
    selected_thread_policy: &'a str,
    frames: Vec<DecodeHashFrameJson<'a>>,
}

impl<'a> DecodeHashReportJson<'a> {
    fn new(report: &'a DecodeHashReport) -> Self {
        Self {
            contract_id: report.contract_id,
            contract_version: report.contract_version,
            selected_output_variants: report
                .selected_output_variants
                .iter()
                .map(|variant| variant.as_str())
                .collect(),
            selected_thread_policy: &report.selected_thread_policy,
            frames: report.frames.iter().map(DecodeHashFrameJson::new).collect(),
        }
    }
}

#[derive(Serialize)]
struct DecodeHashFrameJson<'a> {
    output_index: u64,
    visible_luma_left: u32,
    visible_luma_top: u32,
    visible_luma_width: u32,
    visible_luma_height: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    chroma_left: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    chroma_top: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    chroma_width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    chroma_height: Option<u32>,
    bit_depth: u8,
    pixel_format: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    atlas: Option<PictureAtlasView<'a>>,
    hashes: Vec<DecodeHashEntryJson<'a>>,
}

impl<'a> DecodeHashFrameJson<'a> {
    fn new(frame: &'a DecodeHashFrame) -> Self {
        Self {
            output_index: frame.output_index,
            visible_luma_left: frame.visible_luma_left,
            visible_luma_top: frame.visible_luma_top,
            visible_luma_width: frame.visible_luma_width,
            visible_luma_height: frame.visible_luma_height,
            chroma_left: frame.chroma_left,
            chroma_top: frame.chroma_top,
            chroma_width: frame.chroma_width,
            chroma_height: frame.chroma_height,
            bit_depth: frame.bit_depth,
            pixel_format: frame.pixel_format.as_str(),
            atlas: PictureAtlasView::of(frame),
            hashes: frame.hashes.iter().map(DecodeHashEntryJson::new).collect(),
        }
    }
}

#[derive(Serialize)]
struct DecodeHashEntryJson<'a> {
    variant: &'static str,
    algorithm_id: &'static str,
    byte_stream_id: &'static str,
    digest_hex: &'a str,
}

impl<'a> DecodeHashEntryJson<'a> {
    fn new(entry: &'a DecodeHashEntry) -> Self {
        Self {
            variant: entry.variant.as_str(),
            algorithm_id: entry.algorithm_id,
            byte_stream_id: entry.byte_stream_id,
            digest_hex: &entry.digest_hex,
        }
    }
}

fn render_hash_report(report: &DecodeHashReport, json: bool) -> Result<()> {
    if json {
        let json = serde_json::to_string_pretty(&DecodeHashReportJson::new(report))
            .context("failed to serialize decode hash report")?;
        println!("{json}");
        return Ok(());
    }

    println!("contract_id: {}", report.contract_id);
    println!("contract_version: {}", report.contract_version);
    println!("selected_thread_policy: {}", report.selected_thread_policy);
    for variant in &report.selected_output_variants {
        println!("selected_output_variant: {}", variant.as_str());
    }
    for frame in &report.frames {
        println!("frame.output_index: {}", frame.output_index);
        println!(
            "frame.visible_luma: {},{},{}x{}",
            frame.visible_luma_left,
            frame.visible_luma_top,
            frame.visible_luma_width,
            frame.visible_luma_height
        );
        println!("frame.pixel_format: {}", frame.pixel_format.as_str());
        println!("frame.bit_depth: {}", frame.bit_depth);
        if let Some(atlas) = PictureAtlasView::of(frame) {
            println!("frame.atlas: {}", serde_json::to_string(&atlas)?);
        }
        for hash in &frame.hashes {
            println!(
                "frame.hash: {} {} {} {}",
                hash.variant.as_str(),
                hash.algorithm_id,
                hash.byte_stream_id,
                hash.digest_hex
            );
        }
    }
    Ok(())
}

/// Runs `splot decode` through the byte-stream decode handoff.
///
/// Null, hash, raw, and Y4M modes decode streams the runtime supports; unsupported
/// streams surface structured diagnostics instead of output.
///
/// # Errors
/// Returns an error if input cannot be read, the decode context cannot be
/// constructed, the worker pool fails, or JSON serialization fails.
pub(crate) fn run(args: &DecodeArgs) -> Result<ExitCode> {
    let target = args
        .output_target()
        .context("decode output target was not resolved")?;
    let output_format = target.format();
    if args.frame_rate.is_some() && output_format != DecodeOutputFormat::Y4m {
        anyhow::bail!("--frame-rate is only valid with Y4M output");
    }

    let temporal_layer_selection = args
        .temporal_layer
        .or(args.temporal_output_mask)
        .unwrap_or_default();
    let embedded_layer_selection = args
        .embedded_layer
        .or(args.embedded_output_mask)
        .unwrap_or_default();
    let extended_layer_selection = args
        .extended_layer
        .or(args.extended_output_mask)
        .unwrap_or_default();
    let options = DecodeOptions::default()
        .with_output_frame_limit(args.limit)
        .with_y4m_frame_rate_override(args.frame_rate)
        .with_skip_film_grain(args.skip_film_grain)
        .with_temporal_layer_selection(temporal_layer_selection)
        .with_embedded_layer_selection(embedded_layer_selection)
        .with_extended_layer_selection(extended_layer_selection)
        .with_operating_point_selection(args.operating_point.unwrap_or_default());
    let input = open_decode_input(&args.input)
        .with_context(|| format!("failed to read input file: {}", args.input.display()))?;
    let context = DecodeContext::new(
        DecodeRuntimeConfig::new(args.threads).with_frame_delay(args.frame_delay),
    )?;
    let context = if args.external_atlas.is_empty() {
        context
    } else {
        context.with_external_atlases(read_external_atlases(&args.external_atlas)?)
    };
    let stdout_carries_output = matches!(
        target,
        DecodeOutputTarget::Y4m { path } | DecodeOutputTarget::Raw { path } if is_stdout_path(path)
    );
    let Some(report) = context.with_typed_backend(CliDecodeScope {
        target,
        input,
        input_path: &args.input,
        options,
        json: args.json,
    })?
    else {
        return Ok(ExitCode::SUCCESS);
    };

    if args.json {
        let json = serde_json::to_string_pretty(&DecodeDiagnosticJson::new(&report, output_format))
            .context("failed to serialize decode diagnostic")?;
        if stdout_carries_output {
            eprintln!("{json}");
        } else {
            println!("{json}");
        }
    } else {
        render_text_diagnostic(&report, output_format);
    }

    Ok(ExitCode::from(1))
}

struct CliDecodeScope<'a> {
    target: DecodeOutputTarget<'a>,
    input: BufReader<File>,
    input_path: &'a Path,
    options: DecodeOptions,
    json: bool,
}

impl DecodeScope<Result<Option<DecodeDiagnosticReport>>> for CliDecodeScope<'_> {
    fn run<B: SimdBackend>(
        self,
        context: &TypedDecodeContext<'_, B>,
    ) -> Result<Option<DecodeDiagnosticReport>> {
        match self.target {
            DecodeOutputTarget::Null => {
                match context.decode_discard_reader(self.input, self.options) {
                    Ok(()) => Ok(None),
                    Err(error) => decode_report_from_error(&error, self.input_path).map(Some),
                }
            }
            DecodeOutputTarget::Hash => {
                match context.decode_hash_report_reader(self.input, self.options) {
                    Ok(report) => {
                        render_hash_report(&report, self.json)?;
                        Ok(None)
                    }
                    Err(error) => decode_report_from_error(&error, self.input_path).map(Some),
                }
            }
            DecodeOutputTarget::Y4m { path } => {
                match decode_y4m_to_file(context, self.input, &self.options, path) {
                    Ok(()) => Ok(None),
                    Err(error) => decode_report_from_error(&error, self.input_path).map(Some),
                }
            }
            DecodeOutputTarget::Raw { path } => {
                match decode_raw_to_file(context, self.input, &self.options, path) {
                    Ok(()) => Ok(None),
                    Err(error) => decode_report_from_error(&error, self.input_path).map(Some),
                }
            }
        }
    }
}

fn decode_y4m_to_file<B: SimdBackend>(
    context: &TypedDecodeContext<'_, B>,
    input: BufReader<File>,
    options: &DecodeOptions,
    path: &Path,
) -> core::result::Result<(), DecodeError> {
    publish_output(path, Y4M_OUTPUT, |writer| {
        context.decode_y4m_reader(input, *options, writer)
    })
}

fn decode_raw_to_file<B: SimdBackend>(
    context: &TypedDecodeContext<'_, B>,
    input: BufReader<File>,
    options: &DecodeOptions,
    path: &Path,
) -> core::result::Result<(), DecodeError> {
    if cfg!(unix) && path == Path::new("/dev/null") {
        return context.decode_discard_reader(input, *options);
    }
    publish_output(path, RAW_OUTPUT, |writer| {
        context.decode_raw_reader(input, *options, writer)
    })
}

#[derive(Clone, Copy)]
struct OutputArtifact {
    write_stream_operation: DecodeOutputOperation,
    resolve_operation: DecodeOutputOperation,
    create_temp_operation: DecodeOutputOperation,
    write_temp_operation: DecodeOutputOperation,
    flush_temp_operation: DecodeOutputOperation,
    sync_temp_operation: DecodeOutputOperation,
    rename_operation: DecodeOutputOperation,
    cleanup_temp_operation: DecodeOutputOperation,
    temp_label: &'static str,
    path_name: &'static str,
}

const Y4M_OUTPUT: OutputArtifact = OutputArtifact {
    write_stream_operation: DecodeOutputOperation::WriteY4mStream,
    resolve_operation: DecodeOutputOperation::ResolveY4mOutputPath,
    create_temp_operation: DecodeOutputOperation::CreateY4mTempFile,
    write_temp_operation: DecodeOutputOperation::WriteY4mTempFile,
    flush_temp_operation: DecodeOutputOperation::FlushY4mTempFile,
    sync_temp_operation: DecodeOutputOperation::SyncY4mTempFile,
    rename_operation: DecodeOutputOperation::RenameY4mOutput,
    cleanup_temp_operation: DecodeOutputOperation::CleanupY4mTempFile,
    temp_label: "y4m",
    path_name: "Y4M",
};

const RAW_OUTPUT: OutputArtifact = OutputArtifact {
    write_stream_operation: DecodeOutputOperation::WriteRawStream,
    resolve_operation: DecodeOutputOperation::ResolveRawOutputPath,
    create_temp_operation: DecodeOutputOperation::CreateRawTempFile,
    write_temp_operation: DecodeOutputOperation::WriteRawTempFile,
    flush_temp_operation: DecodeOutputOperation::FlushRawTempFile,
    sync_temp_operation: DecodeOutputOperation::SyncRawTempFile,
    rename_operation: DecodeOutputOperation::RenameRawOutput,
    cleanup_temp_operation: DecodeOutputOperation::CleanupRawTempFile,
    temp_label: "raw",
    path_name: "raw",
};

fn publish_output(
    path: &Path,
    artifact: OutputArtifact,
    write: impl FnOnce(&mut (dyn io::Write + Send)) -> core::result::Result<(), DecodeError>,
) -> core::result::Result<(), DecodeError> {
    publish_output_with(path, artifact, write, |error| {
        matches!(error, DecodeError::Output {
            source: DecodeOutputError::Io { source, .. },
        } if source.kind() == io::ErrorKind::BrokenPipe)
    })
}

pub(super) fn publish_composite(
    path: &Path,
    write: impl FnOnce(&mut (dyn io::Write + Send)) -> Result<()>,
) -> Result<()> {
    publish_output_with(path, RAW_OUTPUT, write, |error| {
        error.chain().any(|cause| {
            cause
                .downcast_ref::<io::Error>()
                .is_some_and(|io| io.kind() == io::ErrorKind::BrokenPipe)
        })
    })
}

fn publish_output_with<E: From<DecodeError>>(
    path: &Path,
    artifact: OutputArtifact,
    write: impl FnOnce(&mut (dyn io::Write + Send)) -> Result<(), E>,
    ignore_stream_error: impl FnOnce(&E) -> bool,
) -> Result<(), E> {
    if is_stdout_path(path) {
        return write_stream_output(io::stdout(), artifact, write, ignore_stream_error);
    }
    if fs::metadata(path).is_ok_and(|metadata| !metadata.is_file() && !metadata.is_dir()) {
        let file = OpenOptions::new()
            .write(true)
            .open(path)
            .map_err(|source| output_io(artifact.write_stream_operation, source))?;
        return write_stream_output(file, artifact, write, ignore_stream_error);
    }

    let (parent, final_name) = output_parent_and_name(path, artifact)?;
    let mut output = AtomicOutput::new(parent, final_name, artifact);

    if let Err(error) = write(&mut output) {
        let error = output.take_write_error().map_or(error, E::from);
        return Err(output.cleanup(error));
    }
    output.finish().map_err(E::from)
}

struct AtomicOutput<'a> {
    parent: &'a Path,
    final_name: &'a OsStr,
    artifact: OutputArtifact,
    file: Option<BufWriter<File>>,
    temp_path: Option<PathBuf>,
    write_error: Option<DecodeError>,
}

impl<'a> AtomicOutput<'a> {
    fn new(parent: &'a Path, final_name: &'a OsStr, artifact: OutputArtifact) -> Self {
        Self {
            parent,
            final_name,
            artifact,
            file: None,
            temp_path: None,
            write_error: None,
        }
    }

    fn ensure_file(&mut self) -> core::result::Result<(), DecodeError> {
        if self.file.is_none() {
            let (file, temp_path) = create_temp_file(self.parent, self.final_name, self.artifact)?;
            self.file = Some(BufWriter::new(file));
            self.temp_path = Some(temp_path);
        }
        Ok(())
    }

    fn remember_write_error(&mut self, error: DecodeError) -> io::Error {
        let message = error.to_string();
        self.write_error = Some(error);
        io::Error::other(message)
    }

    fn take_write_error(&mut self) -> Option<DecodeError> {
        self.write_error.take()
    }

    fn cleanup<E: From<DecodeError>>(mut self, error: E) -> E {
        self.file.take();
        match self.temp_path.take() {
            Some(path) => cleanup_temp_file(&path, self.artifact, error),
            None => error,
        }
    }

    fn finish(mut self) -> core::result::Result<(), DecodeError> {
        if let Err(error) = self.ensure_file() {
            return Err(self.cleanup(error));
        }
        let flush_result = self.file.as_mut().map(BufWriter::flush);
        if let Some(Err(source)) = flush_result {
            let error = output_io(self.artifact.flush_temp_operation, source);
            return Err(self.cleanup(error));
        }
        let sync_result = self.file.as_ref().map(|file| file.get_ref().sync_all());
        if let Some(Err(source)) = sync_result {
            let error = output_io(self.artifact.sync_temp_operation, source);
            return Err(self.cleanup(error));
        }
        self.file.take();

        let temp_path = self.temp_path.take().ok_or_else(|| {
            output_io(
                self.artifact.create_temp_operation,
                io::Error::other("temporary output path is unavailable"),
            )
        })?;
        let final_path = self.parent.join(self.final_name);
        replace_output(&temp_path, &final_path, self.artifact)?;
        sync_parent_directory_best_effort(self.parent);
        Ok(())
    }
}

impl io::Write for AtomicOutput<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if let Err(error) = self.ensure_file() {
            return Err(self.remember_write_error(error));
        }
        let result = self
            .file
            .as_mut()
            .ok_or_else(|| io::Error::other("temporary output file is unavailable"))?
            .write(bytes);
        result.map_err(|source| {
            let error = output_io(self.artifact.write_temp_operation, source);
            self.remember_write_error(error)
        })
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Err(error) = self.ensure_file() {
            return Err(self.remember_write_error(error));
        }
        let result = self
            .file
            .as_mut()
            .ok_or_else(|| io::Error::other("temporary output file is unavailable"))?
            .flush();
        result.map_err(|source| {
            let error = output_io(self.artifact.flush_temp_operation, source);
            self.remember_write_error(error)
        })
    }
}

fn write_stream_output<E: From<DecodeError>>(
    output: impl io::Write + Send,
    artifact: OutputArtifact,
    write: impl FnOnce(&mut (dyn io::Write + Send)) -> Result<(), E>,
    ignore_error: impl FnOnce(&E) -> bool,
) -> Result<(), E> {
    let mut output = BufWriter::new(output);
    let result = write(&mut output).and_then(|()| {
        output
            .flush()
            .map_err(|source| E::from(output_io(artifact.write_stream_operation, source)))
    });
    match result {
        Err(error) if ignore_error(&error) => Ok(()),
        result => result,
    }
}

fn is_stdout_path(path: &Path) -> bool {
    matches!(path.to_str(), Some("-" | "/dev/stdout"))
}

fn replace_output(
    temp_path: &Path,
    final_path: &Path,
    artifact: OutputArtifact,
) -> core::result::Result<(), DecodeError> {
    fs::rename(temp_path, final_path)
        .map_err(|source| output_io(artifact.rename_operation, source))
        .map_err(|error| cleanup_temp_file(temp_path, artifact, error))
}

fn output_parent_and_name(
    path: &Path,
    artifact: OutputArtifact,
) -> core::result::Result<(&Path, &OsStr), DecodeError> {
    let file_name = path
        .file_name()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| {
            output_io(
                artifact.resolve_operation,
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "{} output path must include a file name",
                        artifact.path_name
                    ),
                ),
            )
        })?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    Ok((parent, file_name))
}

fn create_temp_file(
    parent: &Path,
    final_name: &OsStr,
    artifact: OutputArtifact,
) -> core::result::Result<(File, PathBuf), DecodeError> {
    let nonce = OUTPUT_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut last_collision = None;
    for attempt in 0..64 {
        let temp_name = temp_file_name(artifact, nonce, attempt);
        if temp_name == final_name {
            continue;
        }
        let temp_path = parent.join(temp_name);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
        {
            Ok(file) => return Ok((file, temp_path)),
            Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {
                last_collision = Some(source);
            }
            Err(source) => {
                return Err(output_io(artifact.create_temp_operation, source));
            }
        }
    }

    Err(output_io(
        artifact.create_temp_operation,
        last_collision.unwrap_or_else(|| {
            io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!(
                    "could not allocate a unique {} temporary output name",
                    artifact.path_name
                ),
            )
        }),
    ))
}

fn temp_file_name(artifact: OutputArtifact, nonce: usize, attempt: usize) -> OsString {
    format!(
        ".splot-decode-{}-{}-{nonce}-{attempt}.tmp",
        artifact.temp_label,
        std::process::id()
    )
    .into()
}

fn cleanup_temp_file<E: From<DecodeError>>(path: &Path, artifact: OutputArtifact, error: E) -> E {
    match fs::remove_file(path) {
        Ok(()) => error,
        Err(source) if source.kind() == io::ErrorKind::NotFound => error,
        Err(source) => output_io(artifact.cleanup_temp_operation, source).into(),
    }
}

fn sync_parent_directory_best_effort(parent: &Path) {
    let Ok(directory) = File::open(parent) else {
        return;
    };
    let _ = directory.sync_all();
}

fn output_io(operation: DecodeOutputOperation, source: io::Error) -> DecodeError {
    DecodeOutputError::io(operation, source).into()
}

pub(super) fn read_external_atlases(paths: &[PathBuf]) -> Result<ExternalAtlases> {
    let mut atlases = ExternalAtlases::default();
    let mut bytes = Vec::new();
    for path in paths {
        bytes.clear();
        File::open(path)
            .and_then(|file| {
                file.take(DEFAULT_MAX_UNIT_BYTES as u64 + 1)
                    .read_to_end(&mut bytes)
            })
            .with_context(|| format!("Cannot read external atlas {}.", path.display()))?;
        anyhow::ensure!(
            bytes.len() <= DEFAULT_MAX_UNIT_BYTES,
            "External atlas {} exceeds the unit size limit of {DEFAULT_MAX_UNIT_BYTES} bytes.",
            path.display(),
        );
        atlases
            .insert(&bytes, ByteOffset::new(0))
            .with_context(|| format!("Invalid external atlas {}.", path.display()))?;
    }
    Ok(atlases)
}

/// Read the input once in stream order, including pipes and FIFOs.
fn open_decode_input(path: &Path) -> io::Result<BufReader<File>> {
    Ok(BufReader::new(File::open(path)?))
}

fn decode_report_from_error(error: &DecodeError, input: &Path) -> Result<DecodeDiagnosticReport> {
    if let DecodeError::Input { source } = error {
        anyhow::bail!("failed to read input file: {}: {source}", input.display());
    }
    DecodeDiagnosticReport::from_decode_error(error)
        .ok_or_else(|| anyhow::anyhow!("failed to plan decode input: {error}"))
}

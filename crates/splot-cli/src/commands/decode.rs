// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `splot decode` — future reference-style decode / round-trip entry point.

use std::fs::File;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use clap::{Args, ValueEnum};
use serde::Serialize;
use splot_decode::{
    DecodeContext, DecodeDiagnostic, DecodeDiagnosticDetails, DecodeDiagnosticReport, DecodeError,
    DecodeHashEntry, DecodeHashFrame, DecodeHashReport, DecodeLimitError, DecodeLimitName,
    DecodeOptions, DecodeRuntimeConfig,
};
use splot_parallel::ThreadCount;

/// Output artifact selected for future `splot decode` success.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
#[value(rename_all = "kebab-case")]
pub enum DecodeOutputFormat {
    /// Future Y4M decoded-video output.
    Y4m,
    /// Future deterministic decoded-frame hash output.
    Hash,
}

impl DecodeOutputFormat {
    fn as_str(self) -> &'static str {
        match self {
            Self::Y4m => "y4m",
            Self::Hash => "hash",
        }
    }
}

/// Arguments for `splot decode`.
#[derive(Args, Debug)]
pub struct DecodeArgs {
    /// Emit the unsupported decode diagnostic as JSON.
    #[arg(long)]
    pub json: bool,
    /// Select the future decode output artifact.
    #[arg(long = "output-format", value_enum, requires_if("y4m", "output"))]
    pub output_format: Option<DecodeOutputFormat>,
    /// Input AV2 bitstream.
    pub input: PathBuf,
    /// Output path for the selected artifact.
    #[arg(short = 'o', long, required_unless_present = "output_format")]
    pub output: Option<PathBuf>,
    /// Worker-thread policy: `auto` (default), a positive integer, or `0` (alias for auto).
    #[arg(long, default_value_t = ThreadCount::Auto)]
    pub threads: ThreadCount,
}

#[derive(Debug)]
enum DecodeOutputTarget<'a> {
    Y4m { path: &'a Path },
    Hash { path: Option<&'a Path> },
}

impl<'a> DecodeOutputTarget<'a> {
    fn path(&self) -> Option<&'a Path> {
        match self {
            Self::Y4m { path } => Some(path),
            Self::Hash { path } => *path,
        }
    }

    fn format(&self) -> DecodeOutputFormat {
        match self {
            Self::Y4m { .. } => DecodeOutputFormat::Y4m,
            Self::Hash { .. } => DecodeOutputFormat::Hash,
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
            (DecodeOutputFormat::Y4m, None) => None,
            (DecodeOutputFormat::Hash, path) => Some(DecodeOutputTarget::Hash { path }),
        }
    }
}

#[derive(Debug)]
enum DecodeInputRead {
    Bytes(Vec<u8>),
    Limit(DecodeLimitError),
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
            eprintln!("tier_id: {}", details.tier_id);
            eprintln!("byte_offset: {}", option_u64_text(details.byte_offset));
        }
        DecodeDiagnosticDetails::RuntimeUnsupported(summary) => {
            eprintln!("detail_kind: runtime_unsupported");
            eprintln!("bitstream_format: {}", summary.bitstream_format);
            eprintln!("input_len_bytes: {}", summary.input_len_bytes);
            eprintln!("obu_count: {}", summary.obu_count);
            eprintln!("frame_candidate_count: {}", summary.frame_candidate_count);
            eprintln!("source_warning_count: {}", summary.source_warning_count);
            eprintln!(
                "selected_temporal_layer_id: {}",
                summary.selected_temporal_layer_id
            );
            eprintln!(
                "selected_embedded_layer_id: {}",
                summary.selected_embedded_layer_id
            );
            eprintln!(
                "selected_extended_layer_id: {}",
                summary.selected_extended_layer_id
            );
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
    eprintln!("matrix_row: {}", diagnostic.matrix_row);
    eprintln!("feature_id: {}", diagnostic.feature_id);
    eprintln!("message: {}", diagnostic.message);
    eprintln!("remediation: {}", diagnostic.remediation);
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
    matrix_row: &'a str,
    feature_id: &'a str,
    message: &'a str,
    remediation: &'a str,
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
    tier_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    obu_type: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bitstream_format: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    input_len_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    obu_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frame_candidate_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_warning_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    selected_temporal_layer_id: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    selected_embedded_layer_id: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    selected_extended_layer_id: Option<u8>,
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
            matrix_row: diagnostic.matrix_row,
            feature_id: diagnostic.feature_id,
            message: diagnostic.message,
            remediation: diagnostic.remediation,
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
            tier_id: None,
            obu_type: None,
            bitstream_format: None,
            input_len_bytes: None,
            obu_count: None,
            frame_candidate_count: None,
            source_warning_count: None,
            selected_temporal_layer_id: None,
            selected_embedded_layer_id: None,
            selected_extended_layer_id: None,
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
                json.tier_id = Some(details.tier_id);
                json.byte_offset = details.byte_offset;
            }
            DecodeDiagnosticDetails::RuntimeUnsupported(summary) => {
                json.detail_kind = "runtime_unsupported";
                json.bitstream_format = Some(summary.bitstream_format);
                json.input_len_bytes = Some(summary.input_len_bytes);
                json.obu_count = Some(summary.obu_count);
                json.frame_candidate_count = Some(summary.frame_candidate_count);
                json.source_warning_count = Some(summary.source_warning_count);
                json.selected_temporal_layer_id = Some(summary.selected_temporal_layer_id);
                json.selected_embedded_layer_id = Some(summary.selected_embedded_layer_id);
                json.selected_extended_layer_id = Some(summary.selected_extended_layer_id);
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

/// Runs `splot decode` through the byte-stream decode handoff. Hash mode has a
/// narrow minimal-tier runtime success path; other runtime outputs remain
/// diagnostic-only until later decoder milestones.
///
/// # Errors
/// Returns an error if input cannot be read, the decode context cannot be
/// constructed, the worker pool fails, or JSON serialization fails.
pub fn run(args: &DecodeArgs) -> Result<ExitCode> {
    let target = args
        .output_target()
        .context("decode output target was not resolved")?;
    // Resolve, but intentionally do not write, the future output artifact path.
    let _target_path = target.path();
    let output_format = target.format();

    let options = DecodeOptions::default();
    let report = match read_decode_input(&args.input, options)? {
        DecodeInputRead::Bytes(bytes) => {
            let context = DecodeContext::new(DecodeRuntimeConfig::new(args.threads))?;
            match output_format {
                DecodeOutputFormat::Hash => match context.decode_hash_report_bytes(&bytes, options)
                {
                    Ok(report) => {
                        render_hash_report(&report, args.json)?;
                        return Ok(ExitCode::SUCCESS);
                    }
                    Err(error) => decode_report_from_error(&error)?,
                },
                DecodeOutputFormat::Y4m => match context.plan_bytes(&bytes, options) {
                    Ok(plan) => DecodeDiagnosticReport::runtime_unsupported(&plan),
                    Err(error) => decode_report_from_error(&error)?,
                },
            }
        }
        DecodeInputRead::Limit(source) => {
            let error = DecodeError::Limit { source };
            decode_report_from_error(&error)?
        }
    };

    if args.json {
        let json = serde_json::to_string_pretty(&DecodeDiagnosticJson::new(&report, output_format))
            .context("failed to serialize decode diagnostic")?;
        println!("{json}");
    } else {
        render_text_diagnostic(&report, output_format);
    }

    Ok(ExitCode::from(1))
}

fn read_decode_input(path: &Path, options: DecodeOptions) -> Result<DecodeInputRead> {
    let mut file = File::open(path)
        .with_context(|| format!("failed to read input file: {}", path.display()))?;

    if let Some(max_input_bytes) = options.limits().max_input_bytes().max_value() {
        if let Ok(metadata) = file.metadata()
            && let Some(error) = input_byte_limit_error(options, metadata.len())
        {
            return Ok(DecodeInputRead::Limit(error));
        }

        let read_limit = max_input_bytes.checked_add(1).unwrap_or(max_input_bytes);
        let mut bytes = Vec::new();
        file.take(read_limit)
            .read_to_end(&mut bytes)
            .with_context(|| format!("failed to read input file: {}", path.display()))?;
        let actual = bytes.len() as u64;
        if let Some(error) = input_byte_limit_error(options, actual) {
            return Ok(DecodeInputRead::Limit(error));
        }

        return Ok(DecodeInputRead::Bytes(bytes));
    }

    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .with_context(|| format!("failed to read input file: {}", path.display()))?;
    Ok(DecodeInputRead::Bytes(bytes))
}

fn input_byte_limit_error(options: DecodeOptions, actual: u64) -> Option<DecodeLimitError> {
    options
        .limits()
        .ensure(DecodeLimitName::MaxInputBytes, actual)
        .err()
}

fn decode_report_from_error(error: &DecodeError) -> Result<DecodeDiagnosticReport> {
    DecodeDiagnosticReport::from_decode_error(error)
        .ok_or_else(|| anyhow::anyhow!("failed to plan decode input: {error}"))
}

// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `splot decode` — future reference-style decode / round-trip entry point.

use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read as _, Write as _};
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Context as _, Result};
use clap::{Args, ValueEnum};
use serde::Serialize;
use splot_decode::{
    DecodeContext, DecodeDiagnostic, DecodeDiagnosticDetails, DecodeDiagnosticReport, DecodeError,
    DecodeHashEntry, DecodeHashFrame, DecodeHashReport, DecodeLimitError, DecodeLimitName,
    DecodeOptions, DecodeOutputError, DecodeOutputOperation, DecodeRuntimeConfig,
};
use splot_parallel::ThreadCount;

static OUTPUT_TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Starts an env-gated (`SPLOT_DECODE_TIMING`) phase timer.
fn timing_start() -> Option<std::time::Instant> {
    std::env::var_os("SPLOT_DECODE_TIMING").map(|_| std::time::Instant::now())
}

/// Emits one `splot.decode_timing` stderr line for a phase started via
/// [`timing_start`].
fn timing_report(phase: &str, started: Option<std::time::Instant>) {
    if let Some(started) = started {
        eprintln!(
            "splot.decode_timing {phase}_ms={:.3}",
            started.elapsed().as_secs_f64() * 1000.0
        );
    }
}

/// Output artifact selected for future `splot decode` success.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
#[value(rename_all = "kebab-case")]
pub enum DecodeOutputFormat {
    /// Runtime Y4M decoded-video output for the documented minimal tier.
    Y4m,
    /// Headerless raw decoded sample output for the documented minimal tier.
    Raw,
    /// Deterministic decoded-frame hash output for the documented minimal tier.
    Hash,
}

impl DecodeOutputFormat {
    fn as_str(self) -> &'static str {
        match self {
            Self::Y4m => "y4m",
            Self::Raw => "raw",
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
    #[arg(
        long = "output-format",
        value_enum,
        requires_if("y4m", "output"),
        requires_if("raw", "output")
    )]
    pub output_format: Option<DecodeOutputFormat>,
    /// Input AV2 bitstream.
    pub input: PathBuf,
    /// Output path for the selected artifact.
    #[arg(short = 'o', long, required_unless_present = "output_format")]
    pub output: Option<PathBuf>,
    /// Worker-thread policy: `auto` (default), a positive integer, or `0` (alias for auto).
    #[arg(long, default_value_t = ThreadCount::Auto)]
    pub threads: ThreadCount,
    /// Stop after emitting this many output frames.
    #[arg(long)]
    pub limit: Option<NonZeroU64>,
}

#[derive(Debug)]
enum DecodeOutputTarget<'a> {
    Y4m { path: &'a Path },
    Raw { path: &'a Path },
    Hash { path: Option<&'a Path> },
}

impl DecodeOutputTarget<'_> {
    fn format(&self) -> DecodeOutputFormat {
        match self {
            Self::Y4m { .. } => DecodeOutputFormat::Y4m,
            Self::Raw { .. } => DecodeOutputFormat::Raw,
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
            (DecodeOutputFormat::Raw, Some(path)) => Some(DecodeOutputTarget::Raw { path }),
            (DecodeOutputFormat::Y4m | DecodeOutputFormat::Raw, None) => None,
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
    tier_id: Option<&'a str>,
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
            tier_id: None,
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

/// Runs `splot decode` through the byte-stream decode handoff.
///
/// Hash, raw, and Y4M modes have narrow minimal tier success paths;
/// broader runtime outputs remain diagnostic-only until later decoder
/// milestones.
///
/// # Errors
/// Returns an error if input cannot be read, the decode context cannot be
/// constructed, the worker pool fails, or JSON serialization fails.
pub fn run(args: &DecodeArgs) -> Result<ExitCode> {
    let target = args
        .output_target()
        .context("decode output target was not resolved")?;
    let output_format = target.format();

    let total_started = timing_start();
    let options = DecodeOptions::default().with_output_frame_limit(args.limit);
    let input_read_started = timing_start();
    let input = read_decode_input(&args.input, &options)?;
    timing_report("input_read", input_read_started);
    let report = match input {
        DecodeInputRead::Bytes(bytes) => {
            let context_started = timing_start();
            let context = DecodeContext::new(DecodeRuntimeConfig::new(args.threads))?;
            timing_report("context_new", context_started);
            match target {
                DecodeOutputTarget::Hash { path } => {
                    let _ = path;
                    match context.decode_hash_report_bytes(&bytes, options) {
                        Ok(report) => {
                            render_hash_report(&report, args.json)?;
                            timing_report("total", total_started);
                            return Ok(ExitCode::SUCCESS);
                        }
                        Err(error) => decode_report_from_error(&error)?,
                    }
                }
                DecodeOutputTarget::Y4m { path } => {
                    match decode_y4m_to_file(&context, &bytes, &options, path) {
                        Ok(()) => {
                            timing_report("total", total_started);
                            return Ok(ExitCode::SUCCESS);
                        }
                        Err(error) => decode_report_from_error(&error)?,
                    }
                }
                DecodeOutputTarget::Raw { path } => {
                    match decode_raw_to_file(&context, &bytes, &options, path) {
                        Ok(()) => {
                            timing_report("total", total_started);
                            return Ok(ExitCode::SUCCESS);
                        }
                        Err(error) => decode_report_from_error(&error)?,
                    }
                }
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

fn decode_y4m_to_file(
    context: &DecodeContext,
    bytes: &[u8],
    options: &DecodeOptions,
    path: &Path,
) -> core::result::Result<(), DecodeError> {
    let mut y4m = Vec::new();
    context.decode_y4m_bytes(bytes, *options, &mut y4m)?;
    publish_output(path, &y4m, Y4M_OUTPUT)
}

fn decode_raw_to_file(
    context: &DecodeContext,
    bytes: &[u8],
    options: &DecodeOptions,
    path: &Path,
) -> core::result::Result<(), DecodeError> {
    let mut raw = Vec::new();
    context.decode_raw_bytes(bytes, *options, &mut raw)?;
    let publish_started = timing_start();
    let published = publish_output(path, &raw, RAW_OUTPUT);
    timing_report("output_publish", publish_started);
    published
}

#[derive(Clone, Copy)]
struct OutputArtifact {
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
    bytes: &[u8],
    artifact: OutputArtifact,
) -> core::result::Result<(), DecodeError> {
    let (parent, final_name) = output_parent_and_name(path, artifact)?;
    let (mut temp_file, temp_path) = create_temp_file(parent, final_name, artifact)?;

    if let Err(source) = temp_file.write_all(bytes) {
        let error = output_io(artifact.write_temp_operation, source);
        return Err(close_and_cleanup_temp_file(
            temp_file, &temp_path, artifact, error,
        ));
    }
    if let Err(source) = temp_file.flush() {
        let error = output_io(artifact.flush_temp_operation, source);
        return Err(close_and_cleanup_temp_file(
            temp_file, &temp_path, artifact, error,
        ));
    }
    if let Err(source) = temp_file.sync_all() {
        let error = output_io(artifact.sync_temp_operation, source);
        return Err(close_and_cleanup_temp_file(
            temp_file, &temp_path, artifact, error,
        ));
    }
    drop(temp_file);

    let final_path = parent.join(final_name);
    replace_output(&temp_path, &final_path, artifact)?;
    sync_parent_directory_best_effort(parent);

    Ok(())
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
    let mut name = OsString::from(OsStr::new(".splot-decode-"));
    name.push(artifact.temp_label);
    name.push("-");
    name.push(std::process::id().to_string());
    name.push("-");
    name.push(nonce.to_string());
    name.push("-");
    name.push(attempt.to_string());
    name.push(".tmp");
    name
}

fn close_and_cleanup_temp_file(
    file: File,
    path: &Path,
    artifact: OutputArtifact,
    error: DecodeError,
) -> DecodeError {
    drop(file);
    cleanup_temp_file(path, artifact, error)
}

fn cleanup_temp_file(path: &Path, artifact: OutputArtifact, error: DecodeError) -> DecodeError {
    match fs::remove_file(path) {
        Ok(()) => error,
        Err(source) if source.kind() == io::ErrorKind::NotFound => error,
        Err(source) => output_io(artifact.cleanup_temp_operation, source),
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

fn read_decode_input(path: &Path, options: &DecodeOptions) -> Result<DecodeInputRead> {
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

fn input_byte_limit_error(options: &DecodeOptions, actual: u64) -> Option<DecodeLimitError> {
    options
        .limits()
        .ensure(DecodeLimitName::MaxInputBytes, actual)
        .err()
}

fn decode_report_from_error(error: &DecodeError) -> Result<DecodeDiagnosticReport> {
    DecodeDiagnosticReport::from_decode_error(error)
        .ok_or_else(|| anyhow::anyhow!("failed to plan decode input: {error}"))
}

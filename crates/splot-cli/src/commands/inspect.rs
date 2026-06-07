// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `splot inspect` — print OBUs and headers from a bitstream.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use clap::Args;
use serde::Serialize;
use splot_core::annexb::{ObuEnvelope, parse_annex_b_obus_partial};
use splot_core::obu::{ObuHeader, PayloadStatus};

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
            header: obu.header,
        }
    }
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

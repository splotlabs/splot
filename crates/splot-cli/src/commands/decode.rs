// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `splot decode` — future reference-style decode / round-trip entry point.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::Args;

/// Arguments for `splot decode`.
#[derive(Args, Debug)]
pub struct DecodeArgs {
    /// Input AV2 bitstream.
    pub input: PathBuf,
    /// Output decoded video file (Y4M).
    #[arg(short = 'o', long)]
    pub output: PathBuf,
}

/// Runs `splot decode`. Reference-style decode / round-trip testing is a future
/// milestone, so this exits non-zero.
///
/// # Errors
/// Does not currently fail; returns `Ok` with a non-zero exit code.
pub fn run(args: &DecodeArgs) -> Result<ExitCode> {
    let _ = args;
    eprintln!("error: `splot decode` is not yet implemented.");
    eprintln!("note: reference-style decode / round-trip testing is a planned milestone.");
    Ok(ExitCode::from(1))
}

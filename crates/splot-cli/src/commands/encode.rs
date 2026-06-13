// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `splot encode` — future AV2 encoder entry point (not yet implemented).

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::Args;
use splot_encode::{Context, EncoderConfig, EncoderRuntimeConfig, Frame};

/// Arguments for `splot encode`.
#[derive(Args, Debug)]
pub struct EncodeArgs {
    /// Input raw video file (Y4M).
    pub input: PathBuf,
    /// Output AV2 bitstream path.
    #[arg(short = 'o', long)]
    pub output: PathBuf,
    /// Encoder speed preset.
    #[arg(long)]
    pub speed: Option<u8>,
    /// Quantizer parameter.
    #[arg(long)]
    pub qp: Option<u32>,
}

/// Runs `splot encode`. The AV2 encoder is a future milestone, so this exercises
/// the (stub) encoder API and exits non-zero.
///
/// # Errors
/// Returns an error only if the encoder context cannot be created.
pub fn run(args: &EncodeArgs) -> Result<ExitCode> {
    let _ = args;
    let mut context = Context::new(EncoderConfig::default(), EncoderRuntimeConfig::default())?;
    match context.send_frame(Frame::default()) {
        Ok(()) => Ok(ExitCode::from(0)),
        Err(error) => {
            eprintln!("error: `splot encode` is not yet implemented ({error}).");
            eprintln!("note: the AV2 encoder is a planned milestone.");
            Ok(ExitCode::from(1))
        }
    }
}

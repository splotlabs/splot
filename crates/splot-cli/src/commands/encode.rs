// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `splot encode` — future AV2 encoder entry point (not yet implemented).

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::Args;
use splot_encode::{
    Context, EncoderConfig, EncoderRuntimeConfig, Frame, FrameId, FrameInfo, FramePlaneInput,
    FramePlanesInput, PlaneRect, PlaneSize,
};
use splot_parallel::ThreadCount;

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
    /// Worker-thread policy: `auto` (default), a positive integer, or `0` (alias for auto).
    #[arg(long, default_value_t = ThreadCount::Auto)]
    pub threads: ThreadCount,
}

/// Runs `splot encode`. The AV2 encoder is a future milestone, so this exercises
/// the (stub) encoder API and exits non-zero.
///
/// # Errors
/// Returns an error if the encoder context or its temporary input probe cannot
/// be created.
pub fn run(args: &EncodeArgs) -> Result<ExitCode> {
    let _ = (&args.input, &args.output, &args.speed, &args.qp);
    let runtime = EncoderRuntimeConfig::new(args.threads);
    let mut context = Context::new(EncoderConfig::default(), runtime)?;
    let y = [0_u8; 1];
    let u = [0_u8; 1];
    let v = [0_u8; 1];
    let frame = Frame::from_planes(
        FrameInfo::yuv420_8bit(FrameId::new(0), PlaneSize::new(1, 1)?),
        FramePlanesInput::yuv(
            FramePlaneInput::new(&y, 1, PlaneRect::new(0, 0, 1, 1)?),
            FramePlaneInput::new(&u, 1, PlaneRect::new(0, 0, 1, 1)?),
            FramePlaneInput::new(&v, 1, PlaneRect::new(0, 0, 1, 1)?),
        ),
    )?;
    match context.send_frame(frame) {
        Ok(()) => Ok(ExitCode::from(0)),
        Err(error) => {
            eprintln!("error: `splot encode` is not yet implemented ({error}).");
            eprintln!("note: the AV2 encoder is a planned milestone.");
            Ok(ExitCode::from(1))
        }
    }
}

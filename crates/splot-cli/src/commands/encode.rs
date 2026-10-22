// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `splot encode` — encode a Y4M file to an AV2 IVF stream.

use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::num::{NonZeroU32, NonZeroU64};
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context as _, Result, bail};
use clap::Args;
use splot_encode::ivf::IvfWriter;
use splot_encode::y4m::Y4mReader;
use splot_encode::{BitDepth, Config, Encoder, Preset, RateControlMode, SourceFrame, ThreadCount};

/// Arguments for `splot encode`.
#[derive(Args, Debug)]
pub(crate) struct EncodeArgs {
    /// Input Y4M file, or `-` for standard input.
    input: PathBuf,
    /// Output IVF file.
    #[arg(short = 'o', long)]
    output: PathBuf,
    /// Constant quantizer (`base_q_idx`), 1..=255.
    #[arg(long, default_value_t = Config::DEFAULT_QP)]
    qp: u8,
    /// Speed preset: 0 is the slowest and best, 10 the fastest.
    #[arg(long, default_value_t = Preset::default().get())]
    preset: u8,
    /// Worker-thread policy: `auto` (default), a positive integer, or `0` (alias for auto).
    #[arg(long, default_value_t = ThreadCount::Auto)]
    threads: ThreadCount,
    /// Stop after this many input frames.
    #[arg(long)]
    limit: Option<NonZeroU64>,
    /// Distance between key frames, in frames.
    #[arg(long, default_value_t = Config::DEFAULT_KEYINT)]
    keyint: NonZeroU32,
    /// Print `frame,pts,qp,bits,psnr_y,psnr_u,psnr_v` CSV lines to stdout.
    #[arg(long)]
    stats: bool,
}

/// Runs `splot encode`.
///
/// # Errors
/// Returns an error for unreadable or malformed input, an unsupported input
/// format or setting, an encoder failure, or an output write failure.
pub(crate) fn run(args: &EncodeArgs) -> Result<ExitCode> {
    let input: Box<dyn Read> = if args.input.as_os_str() == "-" {
        Box::new(io::stdin().lock())
    } else {
        Box::new(
            File::open(&args.input)
                .with_context(|| format!("failed to open input file: {}", args.input.display()))?,
        )
    };
    let mut reader = Y4mReader::new(BufReader::new(input)).context("failed to read Y4M input")?;
    let header = *reader.header();

    let mut config = Config::new(header.width, header.height);
    config.bit_depth = match header.bit_depth {
        8 => BitDepth::Eight,
        10 => BitDepth::Ten,
        bits => bail!("{bits}-bit input is not supported; use 8-bit or 10-bit input"),
    };
    config.chroma = header.format;
    config.frame_rate = header.frame_rate;
    config.rate_control = RateControlMode::ConstantQp { qp: args.qp };
    config.preset = Preset::new(args.preset)?;
    config.threads = args.threads;
    config.keyint = args.keyint;
    if u16::try_from(header.width).is_err() || u16::try_from(header.height).is_err() {
        bail!("IVF output supports at most 65535 samples per side");
    }
    let mut encoder = Encoder::new(config.clone()).context("failed to create the encoder")?;

    // The output file is created at the first packet, so bad input never truncates it.
    let mut writer = None;
    let mut stdout = io::stdout().lock();
    if args.stats {
        writeln!(stdout, "frame,pts,qp,bits,psnr_y,psnr_u,psnr_v")?;
    }
    let mut drain = |encoder: &mut Encoder| -> Result<()> {
        while let Some(packet) = encoder.receive_packet()? {
            if writer.is_none() {
                let output = File::create(&args.output).with_context(|| {
                    format!("failed to create output file: {}", args.output.display())
                })?;
                writer = Some(IvfWriter::new(BufWriter::new(output), &config)?);
            }
            if let Some(writer) = &mut writer {
                writer.write_packet(&packet)?;
            }
            if args.stats {
                for s in &packet.stats {
                    writeln!(
                        stdout,
                        "{},{},{},{},{:.4},{:.4},{:.4}",
                        s.frame_index, s.pts, s.qp, s.bits, s.psnr_y, s.psnr_u, s.psnr_v
                    )?;
                }
            }
        }
        Ok(())
    };

    let limit = args.limit.map_or(u64::MAX, NonZeroU64::get);
    for pts in 0..limit {
        let Some(planes) = reader.read_frame().context("failed to read Y4M input")? else {
            break;
        };
        encoder.send_frame(SourceFrame::new(&config, pts, planes)?)?;
        drain(&mut encoder)?;
    }
    encoder.flush()?;
    drain(&mut encoder)?;
    match writer {
        Some(writer) => {
            writer.finish()?;
        }
        None => bail!("the input has no frames"),
    }
    Ok(ExitCode::SUCCESS)
}

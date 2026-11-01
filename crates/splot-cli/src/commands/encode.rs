// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `splot encode` — encode a Y4M file to an AV2 IVF stream.

use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::num::{NonZeroU32, NonZeroU64};
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context as _, Result, bail};
use clap::{Args, ValueEnum};
use splot_encode::ivf::IvfWriter;
use splot_encode::y4m::Y4mReader;
use splot_encode::{
    BitDepth, Config, Encoder, Preset, RateControlMode, SourceFrame, ThreadCount, TileLayout,
};

/// Arguments for `splot encode`.
#[derive(Args, Debug)]
pub(crate) struct EncodeArgs {
    /// Input Y4M file, or `-` for standard input.
    input: PathBuf,
    /// Output IVF file.
    #[arg(short = 'o', long)]
    output: PathBuf,
    /// Constant quantizer (`base_q_idx`): 1..=255 at 8 bits, 1..=303 at 10 bits; the default mode, at 80.
    #[arg(long, conflicts_with_all = ["crf", "bitrate"])]
    qp: Option<u16>,
    /// Constant quality with lookahead offsets: rate factor 1..=63, higher is smaller.
    #[arg(long, conflicts_with = "bitrate")]
    crf: Option<u8>,
    /// Target bitrate in kilobits per second.
    #[arg(long)]
    bitrate: Option<u32>,
    /// Bitrate mode for `--bitrate`: `vbr` (default) or `cbr` (one-second buffer).
    #[arg(long, value_enum, requires = "bitrate")]
    rc: Option<RcMode>,
    /// Speed preset: 0 is the slowest and best, 10 the fastest.
    #[arg(long, default_value_t = Preset::default().get())]
    preset: u8,
    /// Overrides one field of the preset, as FIELD=VALUE; repeat for more
    /// fields. It wins over the screen-content switch. Flags take 0, 1,
    /// false or true; a per-frame-kind field (`intra_modes`, `tcq`,
    /// `tf_frames`, `tf_strength`) takes one value or four comma-separated
    /// values for key, base, mid and leaf frames; `mvd_tools` takes three
    /// comma-separated flags. An unknown FIELD lists the valid names.
    #[arg(long, value_name = "FIELD=VALUE", value_parser = parse_preset_set)]
    preset_set: Vec<(String, String)>,
    /// Low delay: one packet per input frame, in display order. Sets
    /// `mini_gop_size=1` (IPPP, every frame shown) and `lookahead_frames=1`
    /// before `--preset-set`; off by default.
    #[arg(long)]
    low_delay: bool,
    /// Worker-thread policy: `auto` (default), a positive integer, or `0` (alias for auto).
    #[arg(long, default_value_t = ThreadCount::Auto)]
    threads: ThreadCount,
    /// Tile columns x rows, each a power of two up to 64. More tiles code faster
    /// on more threads and cost some compression.
    #[arg(long, value_parser = parse_tiles, default_value = "1x1")]
    tiles: TileLayout,
    /// Stop after this many input frames.
    #[arg(long)]
    limit: Option<NonZeroU64>,
    /// Fixed-ratio resize: codes every frame at 8/D of the input size per
    /// axis, D in 9..=16; off by default. The IVF, its decode and the
    /// `--stats` PSNR (against the resized source) are at the coded size.
    #[arg(long, value_name = "D")]
    resize_denominator: Option<u8>,
    /// Film grain: each key frame codes a grain model fitted to the noise of
    /// its source, and every output frame applies it at decode; off by
    /// default. The encoder does not denoise, so the stats PSNR does not
    /// include the grain.
    #[arg(long)]
    film_grain: bool,
    /// Distance between key frames, in frames.
    #[arg(long, default_value_t = Config::DEFAULT_KEYINT)]
    keyint: NonZeroU32,
    /// Print `frame,pts,qp,bits,psnr_y,psnr_u,psnr_v` CSV lines to stdout.
    #[arg(long)]
    stats: bool,
}

/// Parses `--tiles COLSxROWS`.
fn parse_tiles(value: &str) -> Result<TileLayout, String> {
    let log2 = |count: &str| match count.parse::<u32>() {
        Ok(count)
            if count.is_power_of_two() && count.ilog2() <= u32::from(TileLayout::MAX_LOG2) =>
        {
            Ok(count.ilog2() as u8)
        }
        _ => Err(format!(
            "tile count `{count}` is not a power of two in 1..=64"
        )),
    };
    let (cols, rows) = value
        .split_once('x')
        .ok_or_else(|| format!("`{value}` is not COLSxROWS"))?;
    Ok(TileLayout {
        cols_log2: log2(cols)?,
        rows_log2: log2(rows)?,
    })
}

/// Parses `--preset-set FIELD=VALUE`.
fn parse_preset_set(value: &str) -> Result<(String, String), String> {
    let (field, value) = value
        .split_once('=')
        .ok_or_else(|| format!("`{value}` is not FIELD=VALUE"))?;
    Ok((field.to_owned(), value.to_owned()))
}

/// Bitrate modes of `splot encode --bitrate`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum RcMode {
    Vbr,
    Cbr,
}

/// The preset overrides of `--low-delay`.
const LOW_DELAY: [(&str, &str); 2] = [("mini_gop_size", "1"), ("lookahead_frames", "1")];

/// Buffer of `--rc cbr`, in milliseconds.
const CBR_BUFFER_MS: u32 = 1000;

impl EncodeArgs {
    fn rate_control(&self) -> RateControlMode {
        match (self.qp, self.crf, self.bitrate) {
            (_, Some(crf), _) => RateControlMode::Crf { crf },
            (_, _, Some(bitrate_kbps)) if self.rc == Some(RcMode::Cbr) => RateControlMode::Cbr {
                bitrate_kbps,
                buffer_ms: CBR_BUFFER_MS,
            },
            (_, _, Some(bitrate_kbps)) => RateControlMode::Vbr { bitrate_kbps },
            (qp, None, None) => RateControlMode::ConstantQp {
                qp: qp.unwrap_or(Config::DEFAULT_QP),
            },
        }
    }
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
    config.rate_control = args.rate_control();
    config.preset = Preset::new(args.preset)?;
    if args.low_delay {
        config.preset_overrides = LOW_DELAY
            .iter()
            .map(|&(field, value)| (field.to_owned(), value.to_owned()))
            .collect();
    }
    config
        .preset_overrides
        .extend(args.preset_set.iter().cloned());
    config.threads = args.threads;
    config.tiles = args.tiles;
    config.keyint = args.keyint;
    config.resize_denominator = args.resize_denominator;
    config.film_grain = args.film_grain;
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

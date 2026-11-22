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
    BitDepth, ChromaSamplePosition, Config, ContentLightLevel, CroppingWindow, Encoder, FirstPass,
    MasteringDisplay, Preset, RateControlMode, SourceFrame, ThreadCount, TileLayout, Tune,
};

/// Arguments for `splot encode`.
#[derive(Args, Debug)]
#[command(group(clap::ArgGroup::new("buffered").multiple(true).args(["rc", "max_bitrate"])))]
pub(crate) struct EncodeArgs {
    /// Input Y4M file, or `-` for standard input.
    input: PathBuf,
    /// Output IVF file.
    #[arg(short = 'o', long)]
    output: PathBuf,
    /// Constant quantizer (`base_q_idx`) for every frame: 1..=255 at 8 bits, 1..=303 at 10 bits; `--lossless` codes 0.
    #[arg(long, conflicts_with_all = ["crf", "bitrate"], value_parser = clap::value_parser!(u16).range(1..=303))]
    qp: Option<u16>,
    /// Lossless: every frame at `base_q_idx` 0, so the decoder output equals the input samples.
    #[arg(long, conflicts_with_all = ["qp", "crf", "bitrate", "max_bitrate", "film_grain"])]
    lossless: bool,
    /// Constant quality with lookahead offsets: rate factor 1..=63, higher is smaller; the default mode, at 20.
    #[arg(long, conflicts_with = "bitrate")]
    crf: Option<u8>,
    /// Maximum bitrate of constant quality (`--crf` or the default) in kilobits per second: a frame that would underflow the decoder buffer of `--buffer` is coded again more coarsely.
    #[arg(long, value_name = "KBPS", conflicts_with_all = ["qp", "bitrate"])]
    max_bitrate: Option<u32>,
    /// Target bitrate in kilobits per second.
    #[arg(long)]
    bitrate: Option<u32>,
    /// Bitrate mode for `--bitrate`: `vbr` (default) or `cbr` (decoder buffer of `--buffer`).
    #[arg(long, value_enum, requires = "bitrate")]
    rc: Option<RcMode>,
    /// Decoder buffer of `--rc cbr` and `--max-bitrate`, in milliseconds at the bitrate [default: 1000].
    #[arg(long, value_name = "MS", requires = "buffered")]
    buffer: Option<u32>,
    /// Decoder buffer level before the first frame, in milliseconds at the bitrate [default: half of `--buffer`].
    #[arg(long, value_name = "MS", requires = "buffered")]
    initial_buffer: Option<u32>,
    /// Two-pass VBR: a first pass at half size and constant qindex reads the input once more and sets each frame's share of the budget. Needs a file input.
    #[arg(long, requires = "bitrate")]
    two_pass: bool,
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
    /// `mini_gop_size=1` and `bitrate_mini_gop_size=1` (IPPP, every frame
    /// shown) and `lookahead_frames=1` before `--preset-set`; off by default.
    #[arg(long)]
    low_delay: bool,
    /// The metric the encoder tunes for: `psnr` (default), `ssim`
    /// (superblock lambda and qindex follow the luma variance) or `vq`
    /// (trials keep the AC energy of the source); before `--preset-set`.
    #[arg(long, value_enum, default_value_t = TuneArg::Psnr)]
    tune: TuneArg,
    /// Fast decode: 64x64 superblocks, no Wiener-NS or GDF, and at most two
    /// CDEF strength sets, before `--preset-set`; off by default.
    #[arg(long)]
    fast_decode: bool,
    /// Worker-thread policy: `auto` (default), a positive integer, or `0` (alias for auto).
    #[arg(long, default_value_t = ThreadCount::Auto)]
    threads: ThreadCount,
    /// Tile columns x rows, each a power of two up to 64. More tiles code faster
    /// on more threads and cost some compression.
    #[arg(long, value_parser = parse_tiles, default_value = "1x1")]
    tiles: TileLayout,
    /// Tile groups per frame: the tiles split in raster order into N tile
    /// group OBUs; N above the tile count codes one tile per group.
    #[arg(long, value_name = "N", default_value = "1")]
    tile_groups: NonZeroU32,
    /// Stop after this many input frames.
    #[arg(long)]
    limit: Option<NonZeroU64>,
    /// Fixed-ratio resize: codes every frame at 8/D of the input size per
    /// axis, D in 9..=16; off by default. The IVF, its decode and the
    /// `--stats` PSNR (against the resized source) are at the coded size.
    #[arg(long, value_name = "D")]
    resize_denominator: Option<u8>,
    /// Cropping window: cut L, R, T and B luma samples off the left, right,
    /// top and bottom of the coded frame (after --resize-denominator); the
    /// stream decodes to the rest. L and T fall on a chroma sample (even for
    /// 4:2:0, L even for 4:2:2). Off by default. The `--stats` PSNR covers the
    /// full coded frame.
    #[arg(long, value_name = "L,R,T,B", value_parser = parse_crop)]
    crop: Option<CroppingWindow>,
    /// Film grain: each key frame codes a grain model fitted to the noise of
    /// its source, and every output frame applies it at decode; off by
    /// default. The encoder does not denoise, so the stats PSNR does not
    /// include the grain.
    #[arg(long)]
    film_grain: bool,
    /// CBR filler: a temporal unit that would overflow the decoder buffer
    /// ends with a padding OBU of the overflow, so the stream keeps the
    /// target bitrate. Only with `--rc cbr`; off by default.
    #[arg(long)]
    cbr_padding: bool,
    /// H.273 colour primaries code point (1 BT.709, 9 BT.2020), written to
    /// the stream as declared; the samples are not converted. Without the
    /// colour flags the stream declares no colour.
    #[arg(long, value_name = "N")]
    color_primaries: Option<u8>,
    /// H.273 transfer characteristics code point (1 BT.709, 16 PQ, 18 HLG).
    #[arg(long, value_name = "N")]
    transfer_characteristics: Option<u8>,
    /// H.273 matrix coefficients code point (1 BT.709, 9 BT.2020 NCL).
    #[arg(long, value_name = "N")]
    matrix_coefficients: Option<u8>,
    /// Sample range; the default is the Y4M `XCOLORRANGE`, else limited.
    #[arg(long, value_enum)]
    color_range: Option<ColorRangeArg>,
    /// Chroma sample position of 4:2:0 or 4:2:2 input; the default is the
    /// one of a `C420mpeg2` (left) or `C420paldv` (top-left) Y4M header.
    #[arg(long, value_enum)]
    chroma_sample_position: Option<ChromaPositionArg>,
    /// Sample aspect ratio; the default is the Y4M `A` tag.
    #[arg(long, value_name = "W:H", value_parser = parse_sar)]
    sar: Option<(u32, u32)>,
    /// Write the frame rate to the stream as timing info; off by default.
    #[arg(long)]
    timing_info: bool,
    /// HDR mastering display as `G(x,y)B(x,y)R(x,y)WP(x,y)L(max,min)`:
    /// chromaticities in units of 0.00002, luminance in units of 0.0001
    /// cd/m2 (the SMPTE ST 2086 integers).
    #[arg(long, value_name = "G(x,y)B(x,y)R(x,y)WP(x,y)L(max,min)", value_parser = parse_mastering_display)]
    mastering_display: Option<MasteringDisplay>,
    /// HDR content light level, MaxCLL and MaxFALL in cd/m2.
    #[arg(long, value_name = "CLL,FALL", value_parser = parse_content_light)]
    content_light: Option<ContentLightLevel>,
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

/// Parses `--sar W:H`.
fn parse_sar(value: &str) -> Result<(u32, u32), String> {
    value
        .split_once(':')
        .and_then(|(width, height)| Some((width.parse().ok()?, height.parse().ok()?)))
        .ok_or_else(|| format!("`{value}` is not W:H"))
}

/// Parses `--crop L,R,T,B`.
fn parse_crop(value: &str) -> Result<CroppingWindow, String> {
    let offsets: Vec<u32> = value
        .split(',')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .map_err(|_| format!("`{value}` is not L,R,T,B"))?;
    match offsets[..] {
        [left, right, top, bottom] => Ok(CroppingWindow {
            left,
            right,
            top,
            bottom,
        }),
        _ => Err(format!("`{value}` is not L,R,T,B")),
    }
}

/// Parses `--mastering-display G(x,y)B(x,y)R(x,y)WP(x,y)L(max,min)`.
fn parse_mastering_display(value: &str) -> Result<MasteringDisplay, String> {
    let invalid = || {
        format!("`{value}` is not G(x,y)B(x,y)R(x,y)WP(x,y)L(max,min), in that order, in integers")
    };
    let mut rest = value;
    let mut pair = |name: &str| {
        let (x, y) = rest
            .strip_prefix(name)?
            .strip_prefix('(')?
            .split_once(')')
            .and_then(|(pair, tail)| {
                rest = tail;
                pair.split_once(',')
            })?;
        Some((x.parse::<u32>().ok()?, y.parse::<u32>().ok()?))
    };
    let mut chromaticity = |name: &str| {
        let (x, y) = pair(name)?;
        Some((u16::try_from(x).ok()?, u16::try_from(y).ok()?))
    };
    let primaries = [chromaticity("G"), chromaticity("B"), chromaticity("R")];
    let white_point = chromaticity("WP");
    let luminance = pair("L");
    match (primaries, white_point, luminance, rest) {
        ([Some(green), Some(blue), Some(red)], Some(white_point), Some((max, min)), "") => {
            Ok(MasteringDisplay {
                primaries: [green, blue, red],
                white_point,
                luminance_max: max,
                luminance_min: min,
            })
        }
        _ => Err(invalid()),
    }
}

/// Parses `--content-light CLL,FALL`.
fn parse_content_light(value: &str) -> Result<ContentLightLevel, String> {
    value
        .split_once(',')
        .and_then(|(cll, fall)| {
            Some(ContentLightLevel {
                max_cll: cll.parse().ok()?,
                max_fall: fall.parse().ok()?,
            })
        })
        .ok_or_else(|| format!("`{value}` is not CLL,FALL"))
}

/// Ranges of `splot encode --color-range`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum ColorRangeArg {
    Limited,
    Full,
}

/// Positions of `splot encode --chroma-sample-position`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum ChromaPositionArg {
    Left,
    Center,
    TopLeft,
    Top,
    BottomLeft,
    Bottom,
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

/// Metrics of `splot encode --tune`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum TuneArg {
    Psnr,
    Ssim,
    Vq,
}

/// The preset overrides of `--low-delay`.
const LOW_DELAY: [(&str, &str); 3] = [
    ("mini_gop_size", "1"),
    ("bitrate_mini_gop_size", "1"),
    ("lookahead_frames", "1"),
];

/// Default decoder buffer of `--rc cbr` and `--max-bitrate`, in milliseconds.
const CBR_BUFFER_MS: u32 = 1000;

impl EncodeArgs {
    /// `--buffer` and `--initial-buffer` with their defaults.
    fn buffer_ms(&self) -> (u32, u32) {
        let buffer_ms = self.buffer.unwrap_or(CBR_BUFFER_MS);
        (buffer_ms, self.initial_buffer.unwrap_or(buffer_ms / 2))
    }

    fn rate_control(&self) -> RateControlMode {
        let (buffer_ms, initial_ms) = self.buffer_ms();
        let qp = self.lossless.then_some(0).or(self.qp);
        match (qp, self.crf, self.bitrate) {
            (_, _, Some(bitrate_kbps)) if self.rc == Some(RcMode::Cbr) => RateControlMode::Cbr {
                bitrate_kbps,
                buffer_ms,
                initial_ms,
            },
            (_, _, Some(bitrate_kbps)) => RateControlMode::Vbr { bitrate_kbps },
            (Some(qp), None, None) => RateControlMode::ConstantQp { qp },
            (_, crf, None) => {
                let crf = crf.unwrap_or(Config::DEFAULT_CRF);
                match self.max_bitrate {
                    Some(max_bitrate_kbps) => RateControlMode::CappedCrf {
                        crf,
                        max_bitrate_kbps,
                        buffer_ms,
                        initial_ms,
                    },
                    None => RateControlMode::Crf { crf },
                }
            }
        }
    }
}

/// Runs `splot encode`.
///
/// # Errors
/// Returns an error for unreadable or malformed input, an unsupported input
/// format or setting, an encoder failure, or an output write failure.
pub(crate) fn run(args: &EncodeArgs) -> Result<ExitCode> {
    if args.rc != Some(RcMode::Cbr)
        && args.max_bitrate.is_none()
        && (args.buffer.is_some() || args.initial_buffer.is_some())
    {
        bail!("--buffer and --initial-buffer need --rc cbr or --max-bitrate");
    }
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
    config.content = header.content_description();
    let content = &mut config.content;
    if let Some(code_point) = args.color_primaries {
        content.color_primaries = code_point;
    }
    if let Some(code_point) = args.transfer_characteristics {
        content.transfer_characteristics = code_point;
    }
    if let Some(code_point) = args.matrix_coefficients {
        content.matrix_coefficients = code_point;
    }
    if let Some(range) = args.color_range {
        content.full_range = range == ColorRangeArg::Full;
    }
    if let Some(position) = args.chroma_sample_position {
        content.chroma_sample_position = Some(match position {
            ChromaPositionArg::Left => ChromaSamplePosition::Left,
            ChromaPositionArg::Center => ChromaSamplePosition::Center,
            ChromaPositionArg::TopLeft => ChromaSamplePosition::TopLeft,
            ChromaPositionArg::Top => ChromaSamplePosition::Top,
            ChromaPositionArg::BottomLeft => ChromaSamplePosition::BottomLeft,
            ChromaPositionArg::Bottom => ChromaSamplePosition::Bottom,
        });
    }
    content.sample_aspect_ratio = args.sar.or(content.sample_aspect_ratio);
    content.timing_info = args.timing_info;
    content.mastering_display = args.mastering_display;
    content.content_light_level = args.content_light;
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
    config.tile_groups = args.tile_groups;
    config.keyint = args.keyint;
    config.resize_denominator = args.resize_denominator;
    config.crop = args.crop;
    config.film_grain = args.film_grain;
    config.cbr_padding = args.cbr_padding;
    config.tune = match args.tune {
        TuneArg::Psnr => Tune::Psnr,
        TuneArg::Ssim => Tune::Ssim,
        TuneArg::Vq => Tune::Vq,
    };
    config.fast_decode = args.fast_decode;
    if u16::try_from(header.width).is_err() || u16::try_from(header.height).is_err() {
        bail!("IVF output supports at most 65535 samples per side");
    }
    if args.two_pass {
        config.first_pass = first_pass(args, &config)?;
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

/// The first-pass bits of each input frame of `args` for `config`, reading
/// the input file once more.
fn first_pass(args: &EncodeArgs, config: &Config) -> Result<Vec<u64>> {
    if args.rc == Some(RcMode::Cbr) {
        bail!("--two-pass needs VBR, not --rc cbr");
    }
    if args.input.as_os_str() == "-" {
        bail!("--two-pass reads the input twice and needs a file, not stdin");
    }
    let input = File::open(&args.input)
        .with_context(|| format!("failed to open input file: {}", args.input.display()))?;
    let mut reader = Y4mReader::new(BufReader::new(input)).context("failed to read Y4M input")?;
    let mut first = FirstPass::new(config).context("failed to create the first pass")?;
    for pts in 0..args.limit.map_or(u64::MAX, NonZeroU64::get) {
        let Some(planes) = reader.read_frame().context("failed to read Y4M input")? else {
            break;
        };
        first.send_frame(SourceFrame::new(config, pts, planes)?)?;
    }
    Ok(first.finish()?)
}

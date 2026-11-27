// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Streams with the decoder model of [`Config::decoder_model`], checked by a
//! model of Annex E written from E.3, E.5.1 to E.5.7, E.7 and the Annex A.4
//! limits that need removal times. The model shares no derivation with the
//! encoder: it reads the coded sizes, the `br_time` of each buffer removal
//! timing OBU, the sequence header and `timing_info()` of the stream, and
//! the level tables of `splot_core::level`.

use super::*;
use crate::Packet;
use core::num::NonZeroU32;
use splot_core::annexb::parse_annex_b_obus_partial;
use splot_core::bitio::BitReader;
use splot_core::headers::buffer_removal_timing::{
    BufferRemovalTiming, parse_buffer_removal_timing,
};
use splot_core::headers::content_interpretation::parse_content_interpretation;
use splot_core::headers::sequence::{
    LevelIdx, SequenceDecoderModelInfo, SequenceHeader, Tier, parse_sequence_header,
};
use splot_core::level::{level_limits, level_rate_limits};
use splot_core::span::ByteOffset;
use splot_core::types::ObuType;

/// One decodable frame group (Annex E.3) of a stream with one tile group
/// per frame.
#[derive(Debug, Clone, Copy)]
struct Group {
    /// `CodedBits`.
    bits: i128,
    /// `BufferRemovalTime`: the `br_time` of its frame unit.
    br_time: i128,
    /// A closed-loop key, open-loop key or RAS frame: a random access point.
    random_access: bool,
    /// `FrameIsIntra`: a key frame. The streams code no intra-only frame.
    intra: bool,
    /// The output index (`pts`) of its temporal unit; a hidden frame of the
    /// unit is output later.
    output: i128,
}

/// The decoder model inputs of a stream.
#[derive(Debug, Clone)]
struct Stream {
    sequence: SequenceHeader,
    /// `time_scale` and `num_units_in_display_tick` of `timing_info()`.
    timing: (i128, i128),
    /// `BitRate` in bits per second (Annex E.3).
    bitrate: i128,
    groups: Vec<Group>,
}

/// The decoder model inputs of `packets`, one temporal unit per output
/// frame, whose `pts` is its output index.
fn stream(packets: &[Packet]) -> Stream {
    let (mut sequence, mut timing, mut groups) = (None::<SequenceHeader>, None, Vec::new());
    let (mut base, mut start, mut br_time) = (0, 0, None);
    for packet in packets {
        let (unit, output) = (&packet.data, i128::from(packet.pts));
        let parsed = parse_annex_b_obus_partial(unit);
        assert!(parsed.error.is_none());
        for obu in &parsed.obus {
            let mut reader = BitReader::new(obu.payload, ByteOffset::new(0));
            let end = base + i128::from(obu.offset.get()) + i128::from(obu.size);
            match obu.header.obu_type {
                ObuType::SequenceHeader => {
                    let header = parse_sequence_header(&mut reader).unwrap();
                    assert!(sequence.as_ref().is_none_or(|s| *s == header));
                    sequence = Some(header);
                }
                ObuType::ContentInterpretation => {
                    let content = parse_content_interpretation(&mut reader).unwrap();
                    let info = content.timing_info.unwrap();
                    let ts = i128::from(info.time_scale);
                    timing = Some((ts, i128::from(info.num_units_in_display_tick)));
                }
                ObuType::BufferRemovalTiming => {
                    let BufferRemovalTiming::ExtendedLayer { br_time: time } =
                        parse_buffer_removal_timing(&mut reader).unwrap()
                    else {
                        panic!("an operating-point removal time");
                    };
                    assert!(br_time.replace(i128::from(time)).is_none());
                }
                kind if kind.is_tile_group() || kind.is_tip_frame() => {
                    let random_access = matches!(
                        kind,
                        ObuType::ClosedLoopKey | ObuType::OpenLoopKey | ObuType::RasFrame
                    );
                    groups.push(Group {
                        bits: (end - start) * 8,
                        br_time: br_time.take().expect("a decoded frame without br_time"),
                        random_access,
                        intra: matches!(kind, ObuType::ClosedLoopKey | ObuType::OpenLoopKey),
                        output,
                    });
                    start = end;
                }
                _ => {}
            }
        }
        base += unit.len() as i128;
    }
    let sequence = sequence.unwrap();
    let general = &sequence.general;
    let rates = level_rate_limits(general.seq_level_idx.get()).unwrap();
    let kbps = if general.seq_tier == Tier::High {
        rates.high_kbps.unwrap()
    } else {
        rates.main_kbps
    };
    let factor = general.seq_profile_idc.bitrate_factor_milli().unwrap();
    Stream {
        bitrate: i128::from(kbps) * i128::from(factor),
        timing: timing.unwrap(),
        sequence,
        groups,
    }
}

/// `PicSizeProfileFactor` (Annex A.2 Table A.2) and `MinCompBasis` (Annex
/// A.4 Table A.9, `MainCR` or `HighCR`) of `general`.
fn compression_basis(
    general: &splot_core::headers::sequence::SequenceHeaderGeneral,
) -> (i128, i128) {
    let pic_size_factor = match general.seq_profile_idc.get() {
        0..=2 => 15,
        3 => 20,
        _ => 30,
    };
    let basis = match (general.seq_tier, general.seq_level_idx.get()) {
        (Tier::High, _) | (_, 4 | 5) => 4,
        (_, 0..=3) => 2,
        (_, 6) => 6,
        _ => 8,
    };
    (pic_size_factor, basis)
}

/// The constraints of Annex E.7 and the removal-time limits of Annex A.4
/// that `stream` breaks, in decoding schedule mode with `LowDelayMode` 0.
/// Times count in units of `1 / second` seconds, so every quotient is exact.
/// `TimeToDecode` takes `DecodeCount` 2 for a key frame where that makes the
/// check stricter and 1 where 1 does.
fn violations(stream: &Stream) -> Vec<String> {
    let general = &stream.sequence.general;
    let level = general.seq_level_idx.get();
    let (frame, rates) = (
        level_limits(level).unwrap(),
        level_rate_limits(level).unwrap(),
    );
    let info = general.decoder_model_info.unwrap();
    let delay = i128::from(info.decoder_buffer_delay);
    let both = delay + i128::from(info.encoder_buffer_delay);
    let tick = i128::from(general.num_units_in_decoding_tick.unwrap());
    let shown = usize::from(general.seq_initial_display_delay_minus_1.unwrap());
    let ((ts, display_tick), bitrate) = (stream.timing, stream.bitrate);
    let (decode_rate, display_rate) = (
        i128::from(rates.max_decode_rate),
        i128::from(rates.max_display_rate),
    );
    let second = 90_000 * ts * bitrate * decode_rate;
    let luma = i128::from(general.max_frame_width.get() * general.max_frame_height.get());
    let decode_count = |group: &Group, upper: bool| 1 + i128::from(group.intra && upper);
    let decode =
        |group: &Group, upper: bool| luma * decode_count(group, upper) * second / decode_rate;
    let headers =
        i128::from(frame.max_header_rate) * (1 + 2 * i128::from(general.seq_tier == Tier::High));
    let (pic_size_factor, basis) = compression_basis(general);
    let mut found = Vec::new();
    let mut check = |holds: bool, what: String| {
        if !holds {
            found.push(what);
        }
    };
    check(
        (1..=90_000).contains(&delay),
        format!("E.7.8: DecoderBufferDelay {delay}"),
    );
    check(
        display_tick * display_rate >= luma * ts
            && display_tick * headers * display_rate >= decode_rate * ts,
        "E.7.6: presentation interval".to_owned(),
    );
    let groups = &stream.groups;
    let (mut removal, mut first, mut last) = (Vec::new(), Vec::new(), Vec::new());
    let mut previous_access = 0;
    for (i, group) in groups.iter().enumerate() {
        let time = if i == 0 {
            delay * second / 90_000
        } else {
            removal[previous_access] + group.br_time * tick * second / ts
        };
        if group.random_access {
            previous_access = i;
        }
        let begin = match last.last() {
            None => 0,
            Some(&end) => (time - both * second / 90_000).max(end),
        };
        removal.push(time);
        first.push(begin);
        last.push(begin + group.bits * second / bitrate);
    }
    let presented = removal[shown] + decode(&groups[shown], false);
    let gaps: Vec<i128> = removal.windows(2).map(|pair| pair[1] - pair[0]).collect();
    for (i, group) in groups.iter().enumerate() {
        check(
            removal[i] >= last[i],
            format!("E.7.4: underflow of group {i}"),
        );
        let arrived: i128 = groups
            .iter()
            .zip(&first)
            .map(|(other, &begin)| {
                (other.bits * second).min(((removal[i] - begin) * bitrate).max(0))
            })
            .sum();
        let removed: i128 = groups[..i].iter().map(|other| other.bits * second).sum();
        check(
            arrived - removed <= bitrate * second,
            format!("E.7.3: overflow before group {i}"),
        );
        if i > 0 && group.random_access {
            let delta = (removal[i] - last[i - 1]) * 90_000;
            check(
                (delay - 1) * second < delta,
                format!("E.7.2: delay at group {i}"),
            );
        }
        let gap = gaps.get(i).or(gaps.last()).copied().unwrap_or(second);
        if i + 1 < groups.len() {
            check(
                gap >= decode(group, true) && gap * headers >= second,
                format!("E.7.5: decode time of group {i}"),
            );
        }
        let bytes = group.bits / 8 - 128;
        check(
            bytes * 4 <= ((luma * pic_size_factor) >> 3) * 5
                && bytes * 8 * basis * second * decode_count(group, true)
                    <= gap * decode_rate * pic_size_factor,
            format!("A.4: CompressedSize of group {i}"),
        );
        let output = presented + group.output * display_tick * second / ts;
        check(
            removal[i] + decode(group, true) <= output,
            format!("E.7.7: group {i} decodes after its output"),
        );
    }
    found
}

/// `(chroma, bit depth, level, tier, extra preset overrides)` of a stream.
type Case = (
    PixelFormat,
    BitDepth,
    u8,
    Tier,
    &'static [(&'static str, &'static str)],
);

/// A 4:2:0 and a 4:4:4 stream, temporal layers with implicit output, and
/// RAS switch frames on every second mini-GOP.
const CASES: [Case; 4] = [
    (PixelFormat::Yuv420, BitDepth::Eight, 0, Tier::Main, &[]),
    (PixelFormat::Yuv444, BitDepth::Ten, 4, Tier::High, &[]),
    (
        PixelFormat::Yuv420,
        BitDepth::Eight,
        1,
        Tier::Main,
        &[("max_tlayer_id", "2"), ("implicit_output", "true")],
    ),
    (
        PixelFormat::Yuv420,
        BitDepth::Eight,
        0,
        Tier::Main,
        &[
            ("switch_frames", "2"),
            ("switch_type", "1"),
            ("long_term_id_bits", "3"),
        ],
    ),
];

/// 26 panning 72x40 frames at preset 6 in CBR at 200 kbit/s (mini-GOPs of
/// 8, key frames at 0 and 16) with the decoder model on `threads` threads:
/// the packets and the encoder recon in display order.
fn model_stream(
    (chroma, bit_depth, level, tier, extra): Case,
    threads: usize,
) -> (Vec<Packet>, Vec<u8>) {
    let mut config = config(72, 40, bit_depth, 6, 150);
    config.chroma = chroma;
    config.level = Some(LevelIdx::from_bits(level));
    config.tier = Some(tier);
    config.content.timing_info = true;
    config.decoder_model = true;
    config.keyint = NonZeroU32::new(16).unwrap();
    config.rate_control = RateControlMode::Cbr {
        bitrate_kbps: 200,
        buffer_ms: 1000,
        initial_ms: 500,
    };
    config.preset_overrides = [("bitrate_mini_gop_size", "8"), ("intra_only", "0")]
        .iter()
        .chain(extra)
        .map(|&(field, value)| (field.to_owned(), value.to_owned()))
        .collect();
    config.threads = splot_parallel::ThreadCount::from(threads);
    carry::encoder_packets(&config, 26, panning_frame)
}

/// Each decoded frame has one removal time; the sequence header codes a
/// display delay of 4 (a mini-GOP of 8 codes frame 1 after frames 8, 4 and
/// 2), one decoding tick per frame and delays that sum to one second. The
/// streams keep every checked Annex E constraint, decode to the encoder
/// recon, and have the same bytes on 1, 2 and 8 threads.
#[test]
fn decoder_model_streams_keep_annex_e() {
    let info = SequenceDecoderModelInfo {
        decoder_buffer_delay: 45_000,
        encoder_buffer_delay: 45_000,
        low_delay_mode_flag: false,
    };
    for (index, case) in CASES.into_iter().enumerate() {
        let name = format!("{case:?}");
        let (packets, recon) = model_stream(case, 1);
        let stream = stream(&packets);
        let units: Vec<u8> = packets.iter().flat_map(|p| p.data.clone()).collect();
        let general = &stream.sequence.general;
        assert_eq!(general.seq_initial_display_delay_minus_1, Some(3), "{name}");
        assert_eq!(general.num_units_in_decoding_tick, Some(1), "{name}");
        assert_eq!(general.decoder_model_info, Some(info), "{name}");
        assert_eq!(stream.groups.len(), 26, "{name}");
        let access = stream.groups.iter().filter(|g| g.random_access).count();
        assert!(access >= 2 + usize::from(index == 3), "{name}: {access}");
        assert_eq!(violations(&stream), Vec::<String>::new(), "{name}");
        assert_same_samples(&decode(&units), &recon, &name);
        if index == 0 {
            for threads in [2, 8] {
                assert!(
                    model_stream(case, threads).0 == packets,
                    "{threads} threads"
                );
            }
        }
    }
}

/// The model finds the underflow of a smoothing buffer that fills slower
/// than the stream and of a first removal before the key frame arrives, and
/// a frame decoded after its output with one display delay frame less.
#[test]
fn annex_e_model_finds_violations() {
    let (packets, _) = model_stream(CASES[0], 1);
    let mut slow = stream(&packets);
    slow.bitrate = 20_000;
    let found = violations(&slow);
    assert!(found.iter().any(|v| v.starts_with("E.7.4")), "{found:?}");
    let mut early = stream(&packets);
    let info = early.sequence.general.decoder_model_info.as_mut().unwrap();
    info.decoder_buffer_delay = 1;
    assert!(violations(&early).contains(&"E.7.4: underflow of group 0".to_owned()));
    let mut short = stream(&packets);
    short.sequence.general.seq_initial_display_delay_minus_1 = Some(2);
    let found = violations(&short);
    assert!(found.iter().any(|v| v.starts_with("E.7.7")), "{found:?}");
}

/// A named change of a configuration.
type Change = (&'static str, fn(&mut Config));

/// The decoder model refuses each form it does not cover, and a level whose
/// decode rate or tile count does not hold at one frame per decoding tick.
#[test]
fn decoder_model_refusals() {
    let mut base = config(72, 40, BitDepth::Eight, 6, 150);
    base.level = Some(LevelIdx::from_bits(0));
    base.content.timing_info = true;
    base.decoder_model = true;
    base.rate_control = RateControlMode::Cbr {
        bitrate_kbps: 200,
        buffer_ms: 1000,
        initial_ms: 500,
    };
    assert!(IntraFrameCoder::new(&base).is_ok());
    base.validate().unwrap();
    let changes: [Change; 8] = [
        ("single picture", |c| c.single_picture = true),
        ("timing_info", |c| c.content.timing_info = false),
        ("rate control", |c| {
            c.rate_control = RateControlMode::Crf { crf: 30 }
        }),
        ("initial 0", |c| {
            c.rate_control = RateControlMode::Cbr {
                bitrate_kbps: 200,
                buffer_ms: 2000,
                initial_ms: 0,
            };
        }),
        ("initial 1500", |c| {
            c.rate_control = RateControlMode::CappedCrf {
                crf: 30,
                max_bitrate_kbps: 200,
                buffer_ms: 2000,
                initial_ms: 1500,
            };
        }),
        ("keyint", |c| c.keyint = NonZeroU32::new(512).unwrap()),
        ("open_loop_key", |c| {
            c.preset_overrides = vec![("open_loop_key".to_owned(), "1".to_owned())];
        }),
        ("sef_repeat", |c| {
            c.preset_overrides = vec![("sef_repeat".to_owned(), "true".to_owned())];
        }),
    ];
    for (what, change) in changes {
        let mut config = base.clone();
        change(&mut config);
        let refusal = config.validate();
        assert!(
            matches!(refusal, Err(Error::DecoderModelWith { .. })),
            "{what}: {refusal:?}"
        );
    }
    let mut maximum = base.clone();
    maximum.level = Some(LevelIdx::from_bits(31));
    assert!(matches!(
        IntraFrameCoder::new(&maximum),
        Err(Error::InvalidLevel { reason, .. }) if reason.contains("decoder model")
    ));
    maximum.level = None;
    let coder = IntraFrameCoder::new(&maximum).unwrap();
    assert_eq!(coder.sequence.general.seq_level_idx.get(), 0);

    let exceeded = |config: &Config| match IntraFrameCoder::new(config) {
        Err(Error::LevelExceeded { limit, .. }) => Some(limit),
        Ok(_) => None,
        Err(other) => panic!("{other}"),
    };
    let mut decode_rate = base.clone();
    (
        decode_rate.width,
        decode_rate.height,
        decode_rate.frame_rate,
    ) = (160, 144, (150, 1));
    decode_rate.preset_overrides = vec![("intrabc".to_owned(), "true".to_owned())];
    let mut tiles = base.clone();
    (tiles.width, tiles.height, tiles.frame_rate) = (208, 128, (150, 1));
    tiles.tiles = TileLayout::Uniform {
        cols_log2: 2,
        rows_log2: 1,
    };
    tiles.preset_overrides = [
        ("intrabc", "false"),
        ("screen_detect", "false"),
        ("max_block_log2", "6"),
    ]
    .map(|(field, value)| (field.to_owned(), value.to_owned()))
    .to_vec();
    for (mut config, limit) in [
        (decode_rate, "LumaSampleCount * DecodeCount per DecCT"),
        (tiles, "NumTiles at FrameParsingTime"),
    ] {
        assert_eq!(exceeded(&config), Some(limit));
        config.decoder_model = false;
        assert_eq!(exceeded(&config), None, "{limit} without the decoder model");
    }
}

/// AVM decodes the decoder model streams like splot.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_decoder_model_streams_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for (index, case) in CASES.into_iter().enumerate() {
        let (packets, recon) = model_stream(case, 1);
        let units: Vec<Vec<u8>> = packets.into_iter().map(|p| p.data).collect();
        let name = format!("decoder-model{index}");
        let avm = avm_decode(&avmdec, &units, (72, 40), case.1, &name);
        assert!(avm == recon, "{name}: AVM output differs");
    }
}

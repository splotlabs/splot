// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Streams with a requested `seq_level_idx` and `seq_tier` (§ 5.4.1,
//! Annex A.4).

use super::*;
use crate::config::TileLayout;
use splot_core::annexb::parse_annex_b_obus_partial;
use splot_core::bitio::BitReader;
use splot_core::headers::sequence::{LevelIdx, ProfileIdc, Tier, parse_sequence_header};
use splot_core::span::ByteOffset;
use splot_core::types::ObuType;

/// The level and tier that the coder of `config` codes.
fn chosen(config: &Config) -> Result<(u8, Tier)> {
    let coder = IntraFrameCoder::new(config)?;
    let general = &coder.sequence.general;
    Ok((general.seq_level_idx.get(), general.seq_tier))
}

fn level(index: u8) -> Option<LevelIdx> {
    Some(LevelIdx::from_bits(index))
}

/// The automatic level is the lowest level whose checked limits hold; the
/// default stays 31 and the main tier.
#[test]
fn automatic_levels_are_the_lowest_that_hold() {
    let mut config = config(64, 64, BitDepth::Eight, 6, 150);
    assert_eq!(chosen(&config).unwrap(), (31, Tier::Main));
    config.level = None;
    assert_eq!(chosen(&config).unwrap(), (0, Tier::Main));
    for ((width, height, fps), expected) in [
        ((640, 360, 30), 1),
        ((1280, 720, 30), 3),
        ((1920, 1080, 30), 4),
        ((1920, 1080, 60), 5),
        ((3840, 2160, 30), 6),
        ((64, 64, 400), 14),
    ] {
        let mut config = config.clone();
        (config.width, config.height, config.frame_rate) = (width, height, (fps, 1));
        assert_eq!(
            chosen(&config).unwrap(),
            (expected, Tier::Main),
            "{width}x{height}@{fps}"
        );
    }
    config.rate_control = RateControlMode::Vbr {
        bitrate_kbps: 20_000,
    };
    assert_eq!(
        chosen(&config).unwrap(),
        (5, Tier::Main),
        "20 Mbit/s main tier"
    );
    config.tier = None;
    assert_eq!(
        chosen(&config).unwrap(),
        (4, Tier::High),
        "20 Mbit/s high tier"
    );
    config.width = 8;
    assert_eq!(chosen(&config).unwrap().0, 31, "under 16x16");
    config.width = 64;
    config.chroma = PixelFormat::Yuv422;
    config.tier = Some(Tier::Main);
    config.rate_control = RateControlMode::Vbr {
        bitrate_kbps: 2_000,
    };
    assert_eq!(
        chosen(&config).unwrap().0,
        0,
        "profile 3 BitrateProfileFactor 1.667"
    );
    config.profile = Some(ProfileIdc::Configurable);
    assert_eq!(chosen(&config).unwrap().0, 31, "profile 31");
}

/// A requested level refuses each checked limit that the stream breaks.
#[test]
fn requested_levels_refuse_the_limits_the_stream_breaks() {
    let exceeded = |config: &Config| match chosen(config) {
        Err(Error::LevelExceeded { limit, .. }) => limit,
        other => panic!("{other:?}"),
    };
    let base = {
        let mut config = config(64, 64, BitDepth::Eight, 6, 150);
        config.level = level(0);
        config
    };
    assert_eq!(chosen(&base).unwrap(), (0, Tier::Main));
    let mut config = base.clone();
    (config.width, config.height) = (704, 64);
    assert_eq!(exceeded(&config), "FrameWidth");
    (config.width, config.height) = (512, 512);
    assert_eq!(exceeded(&config), "FrameWidth * FrameHeight");
    (config.width, config.height) = (640, 64);
    config.tiles = TileLayout::Uniform {
        cols_log2: 3,
        rows_log2: 0,
    };
    assert_eq!(exceeded(&config), "TileCols");
    let mut config = base.clone();
    (config.width, config.height, config.frame_rate) = (426, 240, (60, 1));
    assert_eq!(exceeded(&config), "TotalDisplayLumaSampleRate");
    let mut config = base.clone();
    config.frame_rate = (151, 1);
    assert_eq!(exceeded(&config), "NumFrameHeadersPerSec");
    let mut config = base.clone();
    (config.width, config.height) = (640, 224);
    config.preset_overrides = vec![("num_ref_frames".into(), "9".into())];
    assert_eq!(exceeded(&config), "NumRefFrames");
    let mut config = base.clone();
    config.rate_control = RateControlMode::Cbr {
        bitrate_kbps: 1_501,
        buffer_ms: 1_000,
        initial_ms: 500,
    };
    assert_eq!(exceeded(&config), "BitRate");
    config.rate_control = RateControlMode::Cbr {
        bitrate_kbps: 1_500,
        buffer_ms: 1_000,
        initial_ms: 500,
    };
    assert!(chosen(&config).is_ok());
    let mut config = base;
    config.rate_control = RateControlMode::Crf { crf: 1 };
    assert!(
        chosen(&config).is_ok(),
        "the CRF bitrate is the caller's limit"
    );
}

/// Reserved levels, the high tier below level 4.0 and a level with profile
/// 31 are refused at configuration.
#[test]
fn uncodable_levels_are_refused() {
    let mut config = config(64, 64, BitDepth::Eight, 6, 150);
    for (index, tier, profile) in [
        (22, Tier::Main, None),
        (30, Tier::Main, None),
        (3, Tier::High, None),
        (4, Tier::Main, Some(ProfileIdc::Configurable)),
    ] {
        (config.level, config.tier, config.profile) = (level(index), Some(tier), profile);
        assert!(
            matches!(config.validate(), Err(Error::InvalidLevel { level, .. }) if level.get() == index),
            "{index} {tier:?}"
        );
    }
    (config.level, config.tier, config.profile) = (level(31), Some(Tier::High), None);
    assert!(config.validate().is_ok());
    config.width = 8;
    config.level = level(0);
    assert!(matches!(chosen(&config), Err(Error::InvalidLevel { .. })));
}

/// A 72x40 panning stream at preset 6 (mini-GOPs of 8, filters on).
fn level_stream(
    (chroma, bit_depth): (PixelFormat, BitDepth),
    (index, tier): (Option<u8>, Tier),
    threads: usize,
) -> (Vec<Vec<u8>>, Vec<u8>) {
    let mut config = config(72, 40, bit_depth, 6, 150);
    config.chroma = chroma;
    config.level = index.map(LevelIdx::from_bits);
    config.tier = Some(tier);
    config.threads = splot_parallel::ThreadCount::from(threads);
    encode_frames(&config, (0..9).map(|index| panning_frame(&config, index)))
}

/// The level and tier of every sequence header in `units`, the payloads of
/// the other OBUs, the number of frame headers that count for the header
/// rate (§ 5.18.1) and the number of show-existing frames.
fn split_level(units: &[Vec<u8>]) -> (Vec<(u8, Tier)>, Vec<Vec<u8>>, usize, usize) {
    let stream = units.concat();
    let parsed = parse_annex_b_obus_partial(&stream);
    assert!(parsed.error.is_none());
    let (mut levels, mut rest, mut counted, mut shown) = (Vec::new(), Vec::new(), 0, 0);
    for obu in &parsed.obus {
        match obu.header.obu_type {
            ObuType::SequenceHeader => {
                let mut reader = BitReader::new(obu.payload, ByteOffset::new(0));
                let general = parse_sequence_header(&mut reader).unwrap().general;
                levels.push((general.seq_level_idx.get(), general.seq_tier));
                continue;
            }
            ObuType::ClosedLoopKey
            | ObuType::OpenLoopKey
            | ObuType::LeadingTileGroup
            | ObuType::RegularTileGroup
                if obu.payload[0] & 0x80 != 0 =>
            {
                counted += 1;
            }
            ObuType::RegularSef => shown += 1,
            _ => {}
        }
        rest.push(obu.payload.to_vec());
    }
    (levels, rest, counted, shown)
}

/// Level 2.0 (no `seq_tier` bit) and 4.0 high tier reach every sequence
/// header of 4:2:0 8-bit and 4:4:4 10-bit streams, which decode to the
/// encoder recon with the same bytes on 1, 2 and 8 threads and change no
/// other OBU. Show-existing frames code no counted frame header: each input
/// frame codes one.
#[test]
fn requested_levels_decode_to_the_encoder_recon() {
    for format in [
        (PixelFormat::Yuv420, BitDepth::Eight),
        (PixelFormat::Yuv444, BitDepth::Ten),
    ] {
        for (index, tier) in [(0, Tier::Main), (4, Tier::High)] {
            let case = format!("{format:?} level {index} {tier:?}");
            let (units, recon) = level_stream(format, (Some(index), tier), 1);
            let (levels, rest, counted, shown) = split_level(&units);
            assert!(
                !levels.is_empty() && levels.iter().all(|&l| l == (index, tier)),
                "{case}"
            );
            assert_eq!(counted, 9, "{case}: counted frame headers");
            assert!(shown > 0, "{case}: no show-existing frame");
            assert_same_samples(&decode(&units.concat()), &recon, &case);
            for threads in [2, 8] {
                let other = level_stream(format, (Some(index), tier), threads).0;
                assert!(other == units, "{case}: {threads} threads");
            }
            let (plain, plain_recon) = level_stream(format, (Some(31), Tier::Main), 1);
            assert_eq!(plain_recon, recon, "{case}: the level changes the recon");
            assert!(
                split_level(&plain).1 == rest,
                "{case}: the level changes another OBU"
            );
        }
    }
}

/// AVM decodes the requested-level streams like splot.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_requested_levels_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for format in [
        (PixelFormat::Yuv420, BitDepth::Eight),
        (PixelFormat::Yuv444, BitDepth::Ten),
    ] {
        for (index, tier) in [(0, Tier::Main), (4, Tier::High), (21, Tier::High)] {
            let (units, recon) = level_stream(format, (Some(index), tier), 1);
            let name = format!("level{:?}{:?}{index}{tier:?}", format.0, format.1);
            let avm = avm_decode(&avmdec, &units, (72, 40), format.1, &name);
            assert!(avm == recon, "{name}: AVM output differs");
        }
    }
}

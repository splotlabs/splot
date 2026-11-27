// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The `seq_level_idx` and `seq_tier` of the stream (§ 5.4.1, Annex A.4).
//!
//! A level other than 31 holds only when the stream keeps the Annex A.4
//! limits that the encoder can check before the first frame; see
//! [`Config::tier`] for the list.

use splot_core::headers::frame::{
    EncoderStreamParams, FrameFilterParams, KeyFrameTxParams, build_key_frame_header,
};
use splot_core::headers::sequence::{LevelIdx, Tier};
use splot_core::level::{level_limits, level_rate_limits};

use crate::config::{Config, RateControlMode};
use crate::error::{Error, Result};
use crate::intra_frame::{coding, frame_tiles, sequence_view};
use crate::preset::PresetParams;

/// `seq_level_idx == 31`: maximum parameters, without level constraints.
const MAX_PARAMETERS: LevelIdx = LevelIdx::from_bits(31);

/// The table-mapped levels 2.0 to 8.3 (Annex A.4 Table A.7), lowest first.
const TABLE_LEVELS: core::ops::RangeInclusive<u8> = 0..=21;

/// Sets the level and tier of `params` from [`Config::level`] and
/// [`Config::tier`]. An automatic level is the lowest one that holds, and 31
/// when none holds (Annex A.4: 31 is for streams that conform to no level).
///
/// # Errors
/// [`Error::LevelExceeded`] or [`Error::InvalidLevel`] when a requested level
/// does not hold, and the tile or header errors of the stream setup.
pub(crate) fn choose_level(
    config: &Config,
    preset: &PresetParams,
    params: &mut EncoderStreamParams,
) -> Result<()> {
    let tiers: &[Tier] = match config.tier {
        Some(Tier::Main) => &[Tier::Main],
        Some(Tier::High) => &[Tier::High],
        None => &[Tier::Main, Tier::High],
    };
    let maximum = |params: &mut EncoderStreamParams| {
        params.tools.level = Some(MAX_PARAMETERS);
        params.tools.tier = tiers[0];
    };
    let codable = |level: LevelIdx, tier: Tier| tier == Tier::Main || level.codes_tier(false);
    match config.level {
        Some(level) if level == MAX_PARAMETERS => maximum(params),
        Some(level) => {
            let mut refusal = None;
            for &tier in tiers.iter().filter(|&&tier| codable(level, tier)) {
                match check(config, preset, params, (level, tier)) {
                    Err(error) => refusal = refusal.or(Some(error)),
                    ok => return ok,
                }
            }
            return Err(refusal.unwrap_or(Error::InvalidLevel {
                level,
                tier: tiers[0],
                reason: "the high tier needs level 4.0 or above (§ 5.4.1)",
            }));
        }
        None => {
            for level in TABLE_LEVELS.map(LevelIdx::from_bits) {
                for &tier in tiers.iter().filter(|&&tier| codable(level, tier)) {
                    match check(config, preset, params, (level, tier)) {
                        Err(Error::LevelExceeded { .. } | Error::InvalidTiles { .. }) => {}
                        Err(Error::InvalidLevel { .. }) => {
                            maximum(params);
                            return Ok(());
                        }
                        result => return result,
                    }
                }
            }
            maximum(params);
        }
    }
    Ok(())
}

/// Sets `level` and `tier` in `params` when the stream holds their limits.
fn check(
    config: &Config,
    preset: &PresetParams,
    params: &mut EncoderStreamParams,
    (level, tier): (LevelIdx, Tier),
) -> Result<()> {
    let exceeded = |limit, value: u64, max: u64| {
        if value > max {
            Err(Error::LevelExceeded {
                level,
                tier,
                limit,
                value,
                max,
            })
        } else {
            Ok(())
        }
    };
    let refused = |reason| Error::InvalidLevel {
        level,
        tier,
        reason,
    };
    let (Some(frame), Some(rates)) = (level_limits(level.get()), level_rate_limits(level.get()))
    else {
        return Err(refused("the level is reserved (Annex A.4 Table A.7)"));
    };
    let (width, height) = (u64::from(params.width), u64::from(params.height));
    if width.min(height) < 16 {
        return Err(refused("a frame under 16x16 holds only level 31"));
    }
    exceeded("FrameWidth", width, u64::from(frame.max_width))?;
    exceeded("FrameHeight", height, u64::from(frame.max_height))?;
    exceeded(
        "FrameWidth * FrameHeight",
        width * height,
        frame.max_pic_size,
    )?;

    let mut probe = *params;
    probe.tools.level = Some(level);
    probe.tools.tier = tier;
    let (sequence, view) = sequence_view(&probe)?;
    let profile = sequence.general.seq_profile_idc;
    let bitrate_factor = profile.bitrate_factor_milli().ok_or(refused(
        "profile 31 defines no Annex A.2 Table A.2 factors for a level",
    ))?;
    let tiles = frame_tiles(config, preset, (&probe, &view))?;
    let key = build_key_frame_header(
        &view,
        1,
        0,
        &FrameFilterParams::default(),
        KeyFrameTxParams::default(),
        tiles,
    )
    .map_err(coding("level tile check"))?;
    let info = key.tile_info.as_ref().ok_or(Error::Unsupported {
        what: "a key frame without tile_info()",
    })?;
    exceeded(
        "TileCols",
        u64::from(info.tile_cols),
        u64::from(frame.max_tile_cols),
    )?;
    exceeded(
        "NumTiles",
        u64::from(info.tile_cols) * u64::from(info.tile_rows),
        u64::from(frame.max_tiles),
    )?;

    // Each input frame is one output frame and one counted frame header:
    // show-existing and TIP output frames count none (§ 5.18.1).
    let (num, den) = (
        u128::from(config.frame_rate.0),
        u128::from(config.frame_rate.1),
    );
    let per_second = |count: u128| u64::try_from((count * num).div_ceil(den)).unwrap_or(u64::MAX);
    exceeded(
        "TotalDisplayLumaSampleRate",
        per_second(u128::from(width * height)),
        rates.max_display_rate,
    )?;
    let high = u64::from(tier == Tier::High);
    exceeded(
        "NumFrameHeadersPerSec",
        per_second(1),
        u64::from(frame.max_header_rate) * (1 + 2 * high),
    )?;
    // shortcut: the screen-content row may turn IntraBC on after setup, so
    // screen detection counts as IntraBC; upgrade when setup knows the row.
    let decode_twice = (preset.intrabc || preset.screen_detect) && preset.open_loop_key > 0;
    if let (Some(inter), Some(slots)) = (
        sequence.inter.as_ref(),
        frame.ref_frame_slots(&sequence, decode_twice),
    ) {
        exceeded("NumRefFrames", u64::from(inter.num_ref_frames), slots)?;
    }
    let kbps = match config.rate_control {
        RateControlMode::Cbr { bitrate_kbps, .. } | RateControlMode::Vbr { bitrate_kbps } => {
            Some(bitrate_kbps)
        }
        RateControlMode::CappedCrf {
            max_bitrate_kbps, ..
        } => Some(max_bitrate_kbps),
        _ => None,
    };
    let max_kbps = if tier == Tier::High {
        rates.high_kbps
    } else {
        Some(rates.main_kbps)
    };
    if let (Some(kbps), Some(max_kbps)) = (kbps, max_kbps) {
        let max_bitrate = u64::from(max_kbps) * u64::from(bitrate_factor);
        exceeded("BitRate", u64::from(kbps) * 1000, max_bitrate)?;
    }
    *params = probe;
    Ok(())
}

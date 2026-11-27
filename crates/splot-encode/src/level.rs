// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The `seq_level_idx` and `seq_tier` of the stream (§ 5.4.1, Annex A.4).
//!
//! A level other than 31 holds only when the stream keeps the Annex A.4
//! limits that the encoder can check before the first frame; see
//! [`Config::tier`] for the list. The decoder model of [`Config::decoder_model`]
//! (Annex E) needs such a level.

use splot_core::headers::frame::{
    EncoderStreamParams, EncoderToolFlags, FrameFilterParams, KeyFrameTxParams,
    build_key_frame_header, encode_buffer_removal_timing_obu,
};
use splot_core::headers::sequence::{LevelIdx, SequenceDecoderModelInfo, Tier};
use splot_core::level::{level_limits, level_rate_limits};

use crate::config::{Config, RateControlMode};
use crate::error::{Error, Result};
use crate::gop::{GopFrame, GopStep};
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
/// With [`Config::decoder_model`] the level is not 31, and `params` takes
/// the decoder model of the stream ([`decoder_model`]).
///
/// # Errors
/// [`Error::LevelExceeded`] or [`Error::InvalidLevel`] when a requested level
/// does not hold, and the tile or header errors of the stream setup.
pub(crate) fn choose_level(
    config: &Config,
    preset: &PresetParams,
    params: &mut EncoderStreamParams,
) -> Result<()> {
    resolve_level(config, preset, params)?;
    if !config.decoder_model {
        return Ok(());
    }
    if params.tools.level == Some(MAX_PARAMETERS) {
        return Err(Error::InvalidLevel {
            level: MAX_PARAMETERS,
            tier: params.tools.tier,
            reason: "the decoder model needs a level other than 31 (Annex E.7.1)",
        });
    }
    decoder_model(config, preset, params)
}

fn resolve_level(
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
    let codable =
        |level: LevelIdx, tier: Tier| tier == Tier::Main || level.codes_tier(config.single_picture);
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
///
/// Each input frame is one output frame and one counted frame header:
/// show-existing and TIP output frames count none (§ 5.18.1). The decoder
/// model removes one frame per frame period, its `FrameParsingTime` (Annex
/// A.4), and counts `DecodeCount` 2 when a key frame may use IntraBC.
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
    let decode_twice = decodes_twice(preset);
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
    if config.decoder_model {
        let decode_count = 1 + u128::from(preset.intrabc || preset.screen_detect);
        exceeded(
            "LumaSampleCount * DecodeCount per DecCT",
            per_second(u128::from(width * height) * decode_count),
            rates.max_decode_rate,
        )?;
        let parsing_tiles = u128::from(frame.max_tiles) * 120 * den / (num * decode_count);
        exceeded(
            "NumTiles at FrameParsingTime",
            u64::from(info.tile_cols) * u64::from(info.tile_rows),
            u64::try_from(parsing_tiles.max(1))
                .map_or(u64::MAX, |tiles| tiles.min(u64::from(frame.max_tiles))),
        )?;
    }
    *params = probe;
    Ok(())
}

/// Whether a frame that is not a closed-loop key frame can have
/// `DecodeCount` 2: an open-loop key frame with IntraBC and in-loop filters.
///
/// shortcut: the screen-content row can turn IntraBC on after setup, so
/// screen detection counts as IntraBC; upgrade when setup knows the row.
fn decodes_twice(preset: &PresetParams) -> bool {
    (preset.intrabc || preset.screen_detect) && preset.open_loop_key > 0
}

/// Sets the [`Config::decoder_model`] fields of `params`: one decoding tick
/// per frame period, the delays of Annex E.7.8 from the initial buffer of
/// the rate control, and the initial display delay of the coding order.
///
/// # Errors
/// [`Error::Unsupported`] for an initial buffer outside 1 to 1000 ms or a
/// reorder depth above 15 frames, and the errors of the coding-order plan.
fn decoder_model(
    config: &Config,
    preset: &PresetParams,
    params: &mut EncoderStreamParams,
) -> Result<()> {
    let initial_ms = config
        .initial_buffer_ms()
        .filter(|ms| (1..=1000).contains(ms))
        .ok_or(Error::Unsupported {
            what: "a decoder model without an initial buffer of 1 to 1000 ms",
        })?;
    let decoder_buffer_delay = initial_ms * 90;
    let encoder_buffer_delay = 90_000 - decoder_buffer_delay;
    let tools = &mut params.tools;
    tools.num_units_in_decoding_tick = Some(config.frame_rate.1);
    tools.decoder_model_info = Some(SequenceDecoderModelInfo {
        decoder_buffer_delay,
        encoder_buffer_delay,
        low_delay_mode_flag: false,
    });
    let mut planner = crate::encoder::gop_planner(config, preset);
    let mut depth = 0;
    while planner.next_display() <= u64::from(config.keyint.get()) {
        let Some(len) = planner.next_len(usize::MAX, true) else {
            break;
        };
        for step in planner.plan(len)? {
            if let GopStep::Code(frame) = step {
                depth = depth.max(frame.frame_index.saturating_sub(frame.display_index));
            }
        }
    }
    tools.initial_display_delay_minus_1 = Some(
        u8::try_from(depth)
            .ok()
            .filter(|&depth| depth < 16)
            .ok_or(Error::Unsupported {
                what: "a decoder model with a reorder depth above 15 frames",
            })?,
    );
    Ok(())
}

/// The buffer removal timing OBU of `plan` in a stream of `tools` with the
/// decoder model, whose last random access point before `plan` has coding
/// index `random_access`: `br_time` counts the frames coded since it
/// (§ 6.11). Nothing without the decoder model.
///
/// # Errors
/// [`Error::Unsupported`] for a frame coded before `random_access`, and the
/// OBU error of a `br_time` above 511.
pub(crate) fn removal_timing_obu(
    tools: &EncoderToolFlags,
    plan: &GopFrame,
    random_access: u64,
) -> Result<Vec<u8>> {
    if tools.decoder_model_info.is_none() {
        return Ok(Vec::new());
    }
    let br_time = plan
        .frame_index
        .checked_sub(random_access)
        .and_then(|ticks| u32::try_from(ticks).ok())
        .ok_or(Error::Unsupported {
            what: "a frame coded before its random access point",
        })?;
    encode_buffer_removal_timing_obu(br_time, plan.tlayer).map_err(coding("removal timing"))
}

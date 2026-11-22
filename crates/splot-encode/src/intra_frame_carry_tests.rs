// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Cross-frame CDF carry (§ 5.18.2 `set_primary_ref_frame_and_ctx`, § 7.23).

use super::*;
use crate::gop::{GopPlanner, GopStep};
use crate::preset::PresetParams;
use splot_core::headers::frame::FrameHeaderCore;

/// `PRIMARY_REF_NONE` (§ 3).
const PRIMARY_REF_NONE: u8 = 7;

/// A panning mini-GOP stream coded with `preset`: the units in coding order,
/// the recon in display order, and the header of every frame.
pub(super) fn planned_stream(
    config: &Config,
    preset: &'static PresetParams,
) -> (Vec<u8>, Vec<u8>, Vec<FrameHeaderCore>, IntraFrameCoder) {
    let coder = IntraFrameCoder::with_preset(config, preset).unwrap();
    planned_stream_of(config, coder, panning_frame, 110)
}

/// [`planned_stream`] with `coder` of the frames `frame(config,
/// display_index)` at `qp`.
pub(super) fn planned_stream_of(
    config: &Config,
    mut coder: IntraFrameCoder,
    frame: fn(&Config, u32) -> SourceFrame,
    qp: u16,
) -> (Vec<u8>, Vec<u8>, Vec<FrameHeaderCore>, IntraFrameCoder) {
    let mini_gop = coder.preset.mini_gop_size.min(4);
    let mut planner = GopPlanner::new(config.keyint, mini_gop);
    let (mut units, mut recons, mut cores) = (Vec::new(), BTreeMap::new(), Vec::new());
    let mut queued = 9;
    while let Some(len) = planner.next_len(queued, true) {
        queued -= len;
        for step in planner.plan(len).unwrap() {
            match step {
                GopStep::Code(plan) => {
                    let source = frame(config, plan.display_index as u32);
                    let mut coded = coder.code_frame(&source, &plan, qp, &[]).unwrap();
                    cores.push(
                        coded
                            .tip_output
                            .clone()
                            .unwrap_or_else(|| coded.reference.as_ref().unwrap().0.clone()),
                    );
                    coder.commit(&mut coded).unwrap();
                    recons.insert(plan.display_index, recon_bytes(&coded, config.bit_depth));
                    units.extend(coded.temporal_unit);
                }
                GopStep::ShowExisting { slot, .. } => {
                    units.extend(coder.show_existing_frame(slot).unwrap());
                }
            }
        }
    }
    (
        units,
        recons.into_values().flatten().collect(),
        cores,
        coder,
    )
}

/// With carry, every inter frame codes a primary reference and the sequence
/// averages CDFs, so the decoder loads the saved CDFs the encoder coded with;
/// without carry, frames code `PRIMARY_REF_NONE` and start from the defaults.
#[test]
fn inter_frames_start_from_the_primary_reference_cdfs() {
    let config = config(72, 24, BitDepth::Ten, 7, 110);
    let on = Preset::new(7).unwrap().params();
    let off: &'static PresetParams = Box::leak(Box::new(PresetParams {
        cdf_carry: false,
        ..*on
    }));
    let mut streams = Vec::new();
    for (preset, carry) in [(on, true), (off, false)] {
        let (units, expected, cores, coder) = planned_stream(&config, preset);
        let tq = coder.sequence.transform_quant_entropy.unwrap();
        assert_eq!(tq.enable_avg_cdf, carry);
        let inter: Vec<_> = cores
            .iter()
            .filter_map(|core| core.inter.as_ref())
            .collect();
        assert!(
            inter.len() >= 7,
            "carry {carry}: {} inter frames",
            inter.len()
        );
        for header in inter {
            assert_eq!(header.disable_cross_frame_cdf_init, Some(!carry));
            assert_eq!(
                header.primary_ref_frame != Some(PRIMARY_REF_NONE),
                carry,
                "carry {carry}: primary_ref_frame {:?}",
                header.primary_ref_frame
            );
        }
        assert_same_samples(&decode(&units), &expected, &format!("carry {carry}"));
        streams.push(units);
    }
    assert!(
        streams[0].len() < streams[1].len(),
        "carried CDFs cost {} bytes, default CDFs {}",
        streams[0].len(),
        streams[1].len()
    );
}

/// A mini-GOP stream in 2x2 tiles, coded through the [`Encoder`] on `threads`.
fn tiled_mini_gop(threads: usize) -> (Config, Vec<Vec<u8>>, Vec<u8>) {
    let mut config = config(72, 72, BitDepth::Eight, 7, 100);
    config.tiles = TileLayout {
        cols_log2: 1,
        rows_log2: 1,
    };
    config.threads = splot_parallel::ThreadCount::from(threads);
    let (units, expected) = encode_stream(&config, 9);
    (config, units, expected)
}

#[test]
fn tiled_mini_gops_decode_alike_on_any_thread_count() {
    let (_, one, expected) = tiled_mini_gop(1);
    assert_same_samples(&decode(&one.concat()), &expected, "2x2 tiles");
    assert!(tiled_mini_gop(3).1 == one, "3 threads");
}

/// AVM decodes a carried 2x2-tile mini-GOP stream to the encoder recon.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_tiled_mini_gops_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    let (config, units, expected) = tiled_mini_gop(4);
    let size = (config.width, config.height);
    let avm = avm_decode(&avmdec, &units, size, config.bit_depth, "carry");
    assert!(avm == expected, "AVM output differs");
}

/// `disable_cdf_update`, `primary_default_cdfs` and `cdf_update_tile` each
/// reach the frame headers (§ 5.18.2, § 5.18.7.2); a 2x2-tile mini-GOP stream
/// with any one decodes to the encoder recon, differs from the stream with
/// all off and codes the same bytes on 1, 2 and 8 threads.
#[test]
fn cdf_control_streams_decode_alike_on_any_thread_count() {
    let base = *Preset::new(6).unwrap().params();
    let stream = |preset: PresetParams, threads: usize| {
        let preset: &'static PresetParams = Box::leak(Box::new(preset));
        let mut config = config(136, 136, BitDepth::Eight, 6, 150);
        config.tiles = TileLayout {
            cols_log2: 1,
            rows_log2: 1,
        };
        config.threads = splot_parallel::ThreadCount::from(threads);
        let coder = IntraFrameCoder::with_preset(&config, preset).unwrap();
        planned_stream_of(&config, coder, panning_frame, 150)
    };
    let plain = stream(base, 1).0;
    for (fixed, default_cdfs, tile) in [(true, false, 0), (false, true, 0), (false, false, 1)] {
        let preset = PresetParams {
            disable_cdf_update: fixed,
            primary_default_cdfs: default_cdfs,
            cdf_update_tile: tile,
            ..base
        };
        let (one, expected, cores, _) = stream(preset, 1);
        let fixed_frames = cores
            .iter()
            .filter(|core| core.disable_cdf_update == Some(fixed))
            .count();
        assert!(
            cores
                .iter()
                .all(|core| core.disable_cdf_update != Some(!fixed))
        );
        assert!(fixed_frames >= 8, "{fixed_frames} frames code {fixed}");
        let tiled: Vec<_> = cores
            .iter()
            .filter_map(|core| core.tile_info.as_ref())
            .filter(|info| info.tile_cols * info.tile_rows > 1)
            .collect();
        assert!(tiled.len() >= 8, "{} frames of several tiles", tiled.len());
        for info in tiled {
            let middle = info.tile_cols * info.tile_rows / 2;
            assert_eq!(info.context_update_tile_id, middle * u32::from(tile));
        }
        let carried: Vec<_> = cores
            .iter()
            .filter_map(|core| core.inter.as_ref())
            .filter(|inter| {
                inter
                    .primary_ref_frame
                    .is_some_and(|p| p != PRIMARY_REF_NONE)
            })
            .collect();
        assert!(
            carried.len() >= 4,
            "{} frames keep a primary",
            carried.len()
        );
        for inter in carried {
            assert_eq!(inter.disable_cross_frame_cdf_init, Some(default_cdfs));
        }
        assert!(one != plain, "case {fixed} {default_cdfs} {tile}");
        assert_same_samples(&decode(&one), &expected, "CDF controls");
        for threads in [2, 8] {
            assert!(stream(preset, threads).0 == one, "{threads} threads");
        }
    }
}

/// With `primary_ref_choose` an inter frame codes `PRIMARY_REF_CHOOSE` when
/// the decoder derives its primary reference (§ 5.18.2) and keeps the
/// explicit signal when it derives another one. The recon equals the recon
/// without the field (the same CDFs give the same decisions), the stream
/// decodes to it, and 1, 2 and 8 threads write the same bytes.
#[test]
fn chosen_primary_references_keep_the_recon_on_any_thread_count() {
    let base = *Preset::new(6).unwrap().params();
    let stream = |preset: PresetParams, threads: usize| {
        let preset: &'static PresetParams = Box::leak(Box::new(preset));
        let mut config = config(136, 72, BitDepth::Eight, 6, 150);
        config.threads = splot_parallel::ThreadCount::from(threads);
        let coder = IntraFrameCoder::with_preset(&config, preset).unwrap();
        planned_stream_of(&config, coder, panning_frame, 150)
    };
    let (plain, plain_recon, _, _) = stream(base, 1);
    let choose = PresetParams {
        primary_ref_choose: true,
        ..base
    };
    let (one, expected, cores, _) = stream(choose, 1);
    let signals: Vec<bool> = cores
        .iter()
        .filter_map(|core| core.inter.as_ref())
        .filter(|inter| inter.disable_cross_frame_cdf_init.is_some())
        .filter_map(|inter| inter.signal_primary_ref_frame)
        .collect();
    let chosen = signals.iter().filter(|&&signal| !signal).count();
    assert!(chosen > 0 && chosen < signals.len(), "{signals:?}");
    assert!(expected == plain_recon, "the recon moved");
    assert!(
        one.len() <= plain.len(),
        "{} > {} bytes",
        one.len(),
        plain.len()
    );
    assert_same_samples(&decode(&one), &expected, "chosen primary");
    for threads in [2, 8] {
        assert!(stream(choose, threads).0 == one, "{threads} threads");
    }
}

/// With `seq_tile_info` the sequence codes the 2x2 layout and every frame
/// reuses it (1, 2) or codes `reuse_tile_info = 0` and its own (3); with 256x256
/// superblocks the intra frames reuse it over 128x128 superblocks. The
/// streams decode to the encoder recon with the same bytes on 1, 2 and 8
/// threads.
#[test]
fn sequence_tile_info_streams_decode_alike_on_any_thread_count() {
    let base = *Preset::new(6).unwrap().params();
    let large = PresetParams {
        max_block_log2: 8,
        sb256_min_height: 0,
        ..base
    };
    for (mode, params) in [(1, base), (2, base), (3, base), (1, large)] {
        let stream = |threads: usize| {
            let preset: &'static PresetParams = Box::leak(Box::new(PresetParams {
                seq_tile_info: mode,
                ..params
            }));
            let mut config = config(136, 136, BitDepth::Eight, 6, 150);
            config.tiles = TileLayout {
                cols_log2: 1,
                rows_log2: 1,
            };
            config.threads = splot_parallel::ThreadCount::from(threads);
            let coder = IntraFrameCoder::with_preset(&config, preset).unwrap();
            planned_stream_of(&config, coder, panning_frame, 150)
        };
        let (units, expected, cores, coder) = stream(1);
        let tile = coder.sequence.tile.unwrap();
        assert!(tile.seq_tile_info_present_flag);
        assert_eq!(tile.allow_tile_info_change, Some(mode > 1));
        let name = format!("mode {mode} sb {}", params.max_block_log2);
        for core in &cores {
            let info = core.tile_info.as_ref().unwrap();
            assert_eq!(info.reuse_tile_info, mode < 3, "{name}");
            let tiles = if params.max_block_log2 == 8 { 1 } else { 4 };
            assert_eq!(info.tile_cols * info.tile_rows, tiles, "{name}");
        }
        assert_same_samples(&decode(&units), &expected, &name);
        for threads in [2, 8] {
            assert!(stream(threads).0 == units, "{name}: {threads} threads");
        }
    }
}

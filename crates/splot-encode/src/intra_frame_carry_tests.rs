// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Cross-frame CDF carry (§ 5.18.2 `set_primary_ref_frame_and_ctx`, § 7.23).

use super::grain_streams::with_noise;
use super::*;
use crate::Packet;
use crate::gop::{GopPlanner, GopStep};
use crate::preset::PresetParams;
use splot_core::headers::frame::{FrameHeaderCore, FrameType as CoreFrameType};

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

/// [`planned_stream`] with `coder` of 5 frames `frame(config, display_index)`
/// (one key frame and one mini-GOP of 4) at `qp`.
pub(super) fn planned_stream_of(
    config: &Config,
    coder: IntraFrameCoder,
    frame: fn(&Config, u32) -> SourceFrame,
    qp: u16,
) -> (Vec<u8>, Vec<u8>, Vec<FrameHeaderCore>, IntraFrameCoder) {
    planned_stream_frames(config, coder, frame, qp, 5)
}

/// [`planned_stream_of`] for `frames` frames.
pub(super) fn planned_stream_frames(
    config: &Config,
    mut coder: IntraFrameCoder,
    frame: fn(&Config, u32) -> SourceFrame,
    qp: u16,
    mut queued: usize,
) -> (Vec<u8>, Vec<u8>, Vec<FrameHeaderCore>, IntraFrameCoder) {
    let mini_gop = coder.preset.mini_gop_size.min(4);
    let mut planner =
        GopPlanner::new(config.keyint, mini_gop).with_intra_only(coder.preset.intra_only);
    let (mut units, mut recons, mut cores) = (Vec::new(), BTreeMap::new(), Vec::new());
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
                    units.extend(coder.show_existing_frame(slot, None, false).unwrap());
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
    let config = config(72, 24, BitDepth::Ten, 9, 110);
    let on = Preset::new(9).unwrap().params();
    let off: &'static PresetParams = Box::leak(Box::new(PresetParams {
        cdf_carry: false,
        ..*on
    }));
    let mut streams = Vec::new();
    for (preset, carry) in [(on, true), (off, false)] {
        let coder = IntraFrameCoder::with_preset(&config, preset).unwrap();
        let (units, expected, cores, coder) =
            planned_stream_frames(&config, coder, panning_frame, 110, 9);
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
    let mut config = config(72, 72, BitDepth::Eight, 9, 100);
    config.tiles = TileLayout::Uniform {
        cols_log2: 1,
        rows_log2: 1,
    };
    config.threads = splot_parallel::ThreadCount::from(threads);
    let (units, expected) = encode_stream(&config, 5);
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
    let base = *Preset::new(9).unwrap().params();
    let stream = |preset: PresetParams, threads: usize| {
        let preset: &'static PresetParams = Box::leak(Box::new(preset));
        let mut config = config(136, 136, BitDepth::Eight, 9, 150);
        config.tiles = TileLayout::Uniform {
            cols_log2: 1,
            rows_log2: 1,
        };
        config.threads = splot_parallel::ThreadCount::from(threads);
        let coder = IntraFrameCoder::with_preset(&config, preset).unwrap();
        planned_stream_of(&config, coder, panning_frame, 150)
    };
    let plain = stream(base, 1).0;
    for (index, (fixed, default_cdfs, tile)) in
        [(true, false, 0), (false, true, 0), (false, false, 1)]
            .into_iter()
            .enumerate()
    {
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
        assert!(fixed_frames >= 5, "{fixed_frames} frames code {fixed}");
        let tiled: Vec<_> = cores
            .iter()
            .filter_map(|core| core.tile_info.as_ref())
            .filter(|info| info.tile_cols * info.tile_rows > 1)
            .collect();
        assert!(tiled.len() >= 5, "{} frames of several tiles", tiled.len());
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
            carried.len() >= 3,
            "{} frames keep a primary",
            carried.len()
        );
        for inter in carried {
            assert_eq!(inter.disable_cross_frame_cdf_init, Some(default_cdfs));
        }
        assert!(one != plain, "case {fixed} {default_cdfs} {tile}");
        assert_same_samples(&decode(&one), &expected, "CDF controls");
        if index == 0 {
            assert!(stream(preset, 2).0 == one, "2 threads");
        }
    }
}

/// With `primary_ref_choose` an inter frame codes `PRIMARY_REF_CHOOSE` when
/// the decoder derives its primary reference (§ 5.18.2) and keeps the
/// explicit signal when it derives another one. The recon equals the recon
/// without the field (the same CDFs give the same decisions), the stream
/// decodes to it, and multiple threads write the same bytes.
#[test]
fn chosen_primary_references_keep_the_recon_on_any_thread_count() {
    let base = *Preset::new(9).unwrap().params();
    let stream = |preset: PresetParams, threads: usize| {
        let preset: &'static PresetParams = Box::leak(Box::new(preset));
        let mut config = config(136, 72, BitDepth::Eight, 9, 150);
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
    assert!(stream(choose, 2).0 == one, "2 threads");
}

/// With `seq_tile_info` the sequence codes the 2x2 layout and every frame
/// reuses it (1, 2) or codes `reuse_tile_info = 0` and its own (3); with 256x256
/// superblocks the intra frames reuse it over 128x128 superblocks. The
/// streams decode to the encoder recon with the same bytes on any thread count.
#[test]
fn sequence_tile_info_streams_decode_alike_on_any_thread_count() {
    let base = fast_p6();
    let large = PresetParams {
        max_block_log2: 8,
        sb256_min_height: 0,
        ..base
    };
    for (index, (mode, params)) in [(1, base), (2, base), (3, base), (1, large)]
        .into_iter()
        .enumerate()
    {
        let stream = |threads: usize| {
            let preset: &'static PresetParams = Box::leak(Box::new(PresetParams {
                seq_tile_info: mode,
                ..params
            }));
            let mut config = config(136, 136, BitDepth::Eight, 9, 150);
            config.tiles = TileLayout::Uniform {
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
        if index == 0 {
            assert!(stream(2).0 == units, "{name}: 2 threads");
        }
    }
}

/// With `explicit_refs` the sequence codes `explicit_ref_frame_map`: at `1`
/// inter frames keep the default map, above it they code an explicit map of
/// at most `explicit_refs - 1` references. The mini-GOP streams decode to the
/// encoder recon with the same bytes on any thread count.
#[test]
fn explicit_reference_map_streams_decode_alike_on_any_thread_count() {
    let base = *Preset::new(9).unwrap().params();
    let mut most = 0;
    for explicit_refs in [1, 2, 3, 8] {
        let stream = |threads: usize| {
            let preset: &'static PresetParams = Box::leak(Box::new(PresetParams {
                explicit_refs,
                ..base
            }));
            let mut config = config(72, 40, BitDepth::Eight, 9, 150);
            config.threads = splot_parallel::ThreadCount::from(threads);
            let coder = IntraFrameCoder::with_preset(&config, preset).unwrap();
            planned_stream_of(&config, coder, panning_frame, 150)
        };
        let (units, expected, cores, coder) = stream(1);
        assert!(coder.sequence.inter.unwrap().explicit_ref_frame_map);
        for inter in cores.iter().filter_map(|core| core.inter.as_ref()) {
            let refs = inter.num_total_refs.unwrap();
            assert_eq!(inter.explicit_ref_frame_map, Some(explicit_refs > 1));
            assert!(explicit_refs == 1 || refs < u32::from(explicit_refs));
            most = most.max(refs);
        }
        let name = format!("explicit_refs {explicit_refs}");
        assert_same_samples(&decode(&units), &expected, &name);
        if explicit_refs == 3 {
            assert!(stream(2).0 == units, "{name}: 2 threads");
        }
    }
    assert!(most > 2, "no frame with more than two references");
}

/// With `num_ref_frames` the sequence codes that many slots and the mini-GOP
/// of 8 plans over them: streams of 4, 5 and 16 slots decode to the encoder
/// recon with the same bytes on any thread count. Too few slots for the
/// mini-GOP fail validation.
#[test]
fn explicit_slot_count_streams_decode_alike_on_any_thread_count() {
    let stream = |slots: u8, threads: usize| {
        let mut config = config(72, 40, BitDepth::Eight, 9, 150);
        config.preset_overrides = vec![("num_ref_frames".to_owned(), slots.to_string())];
        config.threads = splot_parallel::ThreadCount::from(threads);
        encode_frames(&config, (0..18).map(|index| panning_frame(&config, index)))
    };
    for slots in [4, 5, 16] {
        let (units, expected) = stream(slots, 1);
        let units = units.concat();
        assert_same_samples(&decode(&units), &expected, &format!("{slots} slots"));
        if slots == 4 {
            assert!(stream(slots, 2).0.concat() == units, "{slots}: 2 threads");
        }
    }
    let mut config = config(72, 40, BitDepth::Eight, 9, 150);
    config.preset_overrides = vec![("num_ref_frames".to_owned(), "2".to_owned())];
    assert!(config.validate().is_err());
}

/// With `intra_only = 1` the first frame of each mini-GOP is an intra-only
/// frame (§ 5.18.2) that refreshes one slot, and a frame coded after it
/// still references a frame from before it. The 4:2:0 and 4:4:4 streams
/// decode to the encoder recon with the same bytes on any thread count.
#[test]
fn intra_only_streams_decode_alike_on_any_thread_count() {
    for chroma in [PixelFormat::Yuv420, PixelFormat::Yuv444] {
        let (units, expected, cores) = intra_only_stream(chroma, 1);
        let intra: Vec<_> = cores
            .iter()
            .enumerate()
            .filter(|(_, core)| core.frame_type == Some(CoreFrameType::IntraOnly))
            .collect();
        assert!(
            intra.len() >= 2,
            "{chroma:?}: {} intra-only frames",
            intra.len()
        );
        for (index, core) in intra {
            assert_eq!(core.refresh_frame_flags.map(u32::count_ones), Some(1));
            assert!(core.inter.is_none());
            let hint = core.display_order_hint().unwrap();
            let older = cores[index + 1..]
                .iter()
                .filter_map(|later| later.inter.as_ref())
                .any(|inter| inter.order_hints.iter().any(|&h| h < hint));
            assert!(older, "{chroma:?}: no reference across the frame at {hint}");
        }
        assert_same_samples(&decode(&units), &expected, &format!("{chroma:?}"));
        if chroma == PixelFormat::Yuv420 {
            let other = intra_only_stream(chroma, 2).0;
            assert!(other == units, "{chroma:?}: 2 threads");
        }
    }
}

/// A panning mini-GOP stream in `chroma` with `intra_only = 1` on `threads`
/// threads: its units, the encoder recon and the frame headers.
fn intra_only_stream(
    chroma: PixelFormat,
    threads: usize,
) -> (Vec<u8>, Vec<u8>, Vec<FrameHeaderCore>) {
    let preset: &'static PresetParams = Box::leak(Box::new(PresetParams {
        intra_only: 1,
        ..*Preset::new(9).unwrap().params()
    }));
    let mut config = config(72, 40, BitDepth::Eight, 9, 150);
    config.chroma = chroma;
    config.threads = splot_parallel::ThreadCount::from(threads);
    let coder = IntraFrameCoder::with_preset(&config, preset).unwrap();
    let (units, expected, cores, _) = planned_stream_frames(&config, coder, panning_frame, 150, 9);
    (units, expected, cores)
}

/// CRF and VBR streams with intra-only frames decode to the encoder recon.
#[test]
fn rate_controlled_intra_only_streams_decode_to_the_encoder_recon() {
    for mode in [
        RateControlMode::Crf { crf: 32 },
        RateControlMode::Vbr { bitrate_kbps: 200 },
    ] {
        let mut config = config(72, 40, BitDepth::Eight, 9, 150);
        config.rate_control = mode;
        config.preset_overrides = vec![("intra_only".to_owned(), "1".to_owned())];
        let (units, expected) =
            encode_frames(&config, (0..18).map(|index| panning_frame(&config, index)));
        assert_same_samples(&decode(&units.concat()), &expected, &format!("{mode:?}"));
    }
}

#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_intra_only_streams_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for chroma in [PixelFormat::Yuv420, PixelFormat::Yuv444] {
        let (units, expected, _) = intra_only_stream(chroma, 1);
        let name = format!("intraonly{chroma:?}");
        let avm = avm_decode(&avmdec, &[units], (72, 40), BitDepth::Eight, &name);
        assert!(avm == expected, "{chroma:?}: AVM output differs");
    }
}

/// The packets of `frames` panning frames coded through the [`Encoder`] with
/// `implicit_output` on `threads`, and the encoder recon in display order.
fn implicit_output_stream(frames: u32, threads: usize) -> (Config, Vec<Packet>, Vec<u8>) {
    let mut config = config(72, 40, BitDepth::Eight, 9, 150);
    config.preset_overrides = vec![("implicit_output".to_owned(), "true".to_owned())];
    config.threads = splot_parallel::ThreadCount::from(threads);
    let (packets, expected) = encoder_packets(&config, frames, panning_frame);
    (config, packets, expected)
}

/// The packets of `frames` source frames `frame(config, index)` coded through
/// the [`Encoder`] with `config`, and the encoder recon in display order.
pub(super) fn encoder_packets(
    config: &Config,
    frames: u32,
    frame: fn(&Config, u32) -> SourceFrame,
) -> (Vec<Packet>, Vec<u8>) {
    let recons = Arc::default();
    let coder = Recording {
        coder: IntraFrameCoder::new(config).unwrap(),
        bit_depth: config.bit_depth,
        recons: Arc::clone(&recons),
    };
    let mut encoder = Encoder::with_coder(config.clone(), Box::new(coder)).unwrap();
    let mut packets = Vec::new();
    for index in (0..frames).map(Some).chain([None]) {
        match index {
            Some(index) => encoder.send_frame(frame(config, index)).unwrap(),
            None => encoder.flush().unwrap(),
        }
        while let Some(packet) = encoder.receive_packet().unwrap() {
            packets.push(packet);
        }
    }
    let expected = recons.lock().unwrap().values().flatten().copied().collect();
    (packets, expected)
}

/// With `implicit_output` a stream of 11 frames, which ends inside its second
/// mini-GOP of 8, codes one temporal unit per frame and no show-existing
/// frame; the packets come in coding order, so their pts is not monotonic,
/// and cover every input frame once. `splot decode` outputs every frame, in
/// display order, equal to the encoder recon (§ 7.21.3, the § 7.21.5 flush
/// at the end), and the bytes are the same on 1, 2 and 8 threads.
#[test]
fn implicit_output_streams_decode_in_display_order() {
    let (_, packets, expected) = implicit_output_stream(11, 1);
    let mut pts: Vec<u64> = packets.iter().map(|packet| packet.pts).collect();
    assert!(!pts.is_sorted(), "{pts:?}");
    pts.sort_unstable();
    assert_eq!(pts, (0..11).collect::<Vec<_>>());
    assert!(packets.iter().all(|packet| packet.stats.len() == 1));
    let units: Vec<u8> = packets
        .iter()
        .flat_map(|packet| packet.data.clone())
        .collect();
    assert_same_samples(&decode(&units), &expected, "implicit output");
    let other: Vec<u8> = implicit_output_stream(11, 2)
        .1
        .iter()
        .flat_map(|packet| packet.data.clone())
        .collect();
    assert!(other == units, "2 threads");
}

#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_implicit_output_streams_in_display_order() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    let (config, packets, expected) = implicit_output_stream(11, 1);
    let units: Vec<Vec<u8>> = packets.into_iter().map(|packet| packet.data).collect();
    let size = (config.width, config.height);
    let avm = avm_decode(&avmdec, &units, size, config.bit_depth, "implicit");
    assert!(avm == expected, "AVM output differs");
}

/// With `long_term_id_bits` 2 the sequence codes 2 bits and the key frames of
/// a stream with a key frame every 2 frames take the long-term ids 0, 1, 2,
/// 0, 1 (§ 5.18.2); the stream decodes to the encoder recon with the same bytes
/// on 1, 2 and 8 threads.
#[test]
fn key_frames_cycle_their_long_term_ids() {
    let stream = |threads: usize| {
        let preset: &'static PresetParams = Box::leak(Box::new(PresetParams {
            long_term_id_bits: 2,
            ..*Preset::new(9).unwrap().params()
        }));
        let mut config = config(72, 40, BitDepth::Eight, 9, 150);
        config.keyint = core::num::NonZeroU32::new(2).unwrap();
        config.threads = splot_parallel::ThreadCount::from(threads);
        let coder = IntraFrameCoder::with_preset(&config, preset).unwrap();
        planned_stream_frames(&config, coder, panning_frame, 150, 9)
    };
    let (units, expected, cores, coder) = stream(1);
    assert_eq!(coder.sequence.inter.unwrap().long_term_frame_id_bits, 2);
    let ids: Vec<Option<i32>> = cores
        .iter()
        .filter(|core| core.frame_type == Some(splot_core::headers::frame::FrameType::Key))
        .map(|core| core.long_term_id)
        .collect();
    assert_eq!(ids, [Some(0), Some(1), Some(2), Some(0), Some(1)]);
    assert_same_samples(&decode(&units), &expected, "long-term ids");
    assert!(stream(2).0 == units, "2 threads");
}

/// `(chroma, bit depth, implicit output, short refresh)` of the open-loop
/// key streams.
const OPEN_LOOP_CASES: [(PixelFormat, BitDepth, bool, bool); 4] = [
    (PixelFormat::Yuv420, BitDepth::Eight, false, true),
    (PixelFormat::Yuv444, BitDepth::Ten, false, true),
    (PixelFormat::Yuv420, BitDepth::Eight, true, true),
    (PixelFormat::Yuv420, BitDepth::Eight, false, false),
];

/// 26 panning frames at preset 6 (mini-GOPs of 8) with a key frame every 12
/// frames and `open_loop_key = 1` in `mode`, with noise and film grain when
/// `grain`: the config, the packets and the encoder recon in display order.
fn open_loop_stream(
    (chroma, bit_depth, implicit, short_refresh): (PixelFormat, BitDepth, bool, bool),
    mode: RateControlMode,
    threads: usize,
    grain: bool,
) -> (Config, Vec<Packet>, Vec<u8>) {
    let mut config = config(72, 40, bit_depth, 9, 150);
    config.chroma = chroma;
    config.film_grain = grain;
    config.rate_control = mode;
    config.keyint = core::num::NonZeroU32::new(12).unwrap();
    config.preset_overrides = vec![
        ("open_loop_key".to_owned(), "1".to_owned()),
        ("implicit_output".to_owned(), implicit.to_string()),
        ("short_refresh".to_owned(), short_refresh.to_string()),
    ];
    config.threads = splot_parallel::ThreadCount::from(threads);
    let frame: fn(&Config, u32) -> SourceFrame = if grain {
        |config, index| with_noise(config, &panning_frame(config, index), index)
    } else {
        panning_frame
    };
    let (packets, expected) = encoder_packets(&config, 26, frame);
    let mut pts: Vec<u64> = packets.iter().map(|packet| packet.pts).collect();
    pts.sort_unstable();
    assert_eq!(
        pts,
        (0..26).collect::<Vec<_>>(),
        "one packet per input frame"
    );
    let coded: usize = packets.iter().map(|packet| packet.stats.len()).sum();
    assert_eq!(coded, 26, "one coded frame per input frame");
    (config, packets, expected)
}

/// The OBU types of `packets` in stream order.
pub(super) fn obu_types(packets: &[Packet]) -> Vec<splot_core::types::ObuType> {
    let units: Vec<u8> = packets
        .iter()
        .flat_map(|packet| packet.data.clone())
        .collect();
    let parsed = splot_core::annexb::parse_annex_b_obus_partial(&units);
    assert!(parsed.error.is_none());
    parsed.obus.iter().map(|obu| obu.header.obu_type).collect()
}

/// With `open_loop_key = 1` the key frames at 12 and 24 are open-loop key
/// frames (§ 7.4.4) with leading tile groups and, without implicit output,
/// leading show-existing frames. The 4:2:0 8-bit, 4:4:4 10-bit and
/// implicit-output streams decode to the encoder recon with the same bytes on
/// 1, 2 and 8 threads.
#[test]
fn open_loop_key_streams_decode_alike_on_any_thread_count() {
    use splot_core::types::ObuType;
    each_case(OPEN_LOOP_CASES, |case| {
        let qp = RateControlMode::ConstantQp { qp: 150 };
        let (_, packets, expected) = open_loop_stream(case, qp, 1, false);
        let types = obu_types(&packets);
        let count = |kind| types.iter().filter(|&&obu| obu == kind).count();
        assert_eq!(count(ObuType::OpenLoopKey), 2, "{case:?}");
        assert_eq!(count(ObuType::ClosedLoopKey), 1, "{case:?}");
        assert!(count(ObuType::LeadingTileGroup) >= 6, "{case:?}");
        assert_eq!(count(ObuType::LeadingSef) > 0, !case.2, "{case:?}");
        let units: Vec<u8> = packets
            .iter()
            .flat_map(|packet| packet.data.clone())
            .collect();
        assert_same_samples(&decode(&units), &expected, &format!("{case:?}"));
        let start = packets
            .iter()
            .position(|packet| {
                obu_types(core::slice::from_ref(packet)).contains(&ObuType::OpenLoopKey)
            })
            .unwrap();
        let ra_units: Vec<u8> = packets[start..]
            .iter()
            .flat_map(|packet| packet.data.clone())
            .collect();
        let frame_bytes = expected.len() / 26;
        assert_same_samples(
            &decode(&ra_units),
            &expected[12 * frame_bytes..],
            &format!("{case:?}: random access"),
        );
        if case == OPEN_LOOP_CASES[0] {
            let other: Vec<u8> = open_loop_stream(case, qp, 2, false)
                .1
                .iter()
                .flat_map(|packet| packet.data.clone())
                .collect();
            assert!(other == units, "{case:?}: 2 threads");
        }
    });
}

/// CRF, VBR and CBR streams with open-loop key frames, with and without
/// implicit output, decode to the encoder recon with one packet per input
/// frame.
#[test]
fn rate_controlled_open_loop_key_streams_decode_to_the_encoder_recon() {
    let modes = [
        RateControlMode::Crf { crf: 32 },
        RateControlMode::Vbr { bitrate_kbps: 200 },
        RateControlMode::Cbr {
            bitrate_kbps: 200,
            buffer_ms: 1000,
            initial_ms: 500,
        },
    ];
    let cases = modes
        .into_iter()
        .flat_map(|mode| [false, true].map(|implicit| (mode, implicit)));
    each_case(cases, |(mode, implicit)| {
        let case = (PixelFormat::Yuv420, BitDepth::Eight, implicit, true);
        let (_, packets, expected) = open_loop_stream(case, mode, 1, false);
        let units: Vec<u8> = packets
            .iter()
            .flat_map(|packet| packet.data.clone())
            .collect();
        assert_same_samples(&decode(&units), &expected, &format!("{mode:?} {implicit}"));
    });
}

/// An open-loop key frame needs `long_term_id_bits = 0`.
#[test]
fn open_loop_keys_refuse_long_term_ids() {
    let mut config = config(72, 40, BitDepth::Eight, 6, 150);
    config.preset_overrides = vec![
        ("open_loop_key".to_owned(), "1".to_owned()),
        ("long_term_id_bits".to_owned(), "2".to_owned()),
    ];
    assert!(config.validate().is_err());
    config.preset_overrides.truncate(1);
    assert!(config.validate().is_ok());
}

/// AVM decodes the open-loop key streams like splot, and a decode that starts
/// at the temporal unit of the first open-loop key frame (§ 7.4.4) outputs the
/// frames from it on, equal to the sequential decode; the leading frames are
/// discarded.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_open_loop_key_streams_from_the_start_and_from_the_key() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for case in OPEN_LOOP_CASES {
        let qp = RateControlMode::ConstantQp { qp: 150 };
        let (config, packets, expected) = open_loop_stream(case, qp, 1, false);
        let size = (config.width, config.height);
        let units: Vec<Vec<u8>> = packets.iter().map(|packet| packet.data.clone()).collect();
        let name = format!("olk{}{}{}", case.1 as u8, case.2, case.3);
        let avm = avm_decode(&avmdec, &units, size, config.bit_depth, &name);
        assert!(avm == expected, "{case:?}: AVM output differs");
        let start = packets
            .iter()
            .position(|packet| {
                obu_types(core::slice::from_ref(packet))
                    .contains(&splot_core::types::ObuType::OpenLoopKey)
            })
            .unwrap();
        let frame_bytes = expected.len() / 26;
        let avm = avm_decode(&avmdec, &units[start..], size, config.bit_depth, &name);
        assert!(
            avm == expected[12 * frame_bytes..],
            "{case:?}: random access differs"
        );
    }
}

/// The raw `splot decode` output of `packets`, with film grain unless `skip`.
fn decode_packets(packets: &[Packet], skip: bool) -> Vec<u8> {
    let units: Vec<u8> = packets
        .iter()
        .flat_map(|packet| packet.data.clone())
        .collect();
    let threads = splot_parallel::ThreadCount::from(1);
    let ctx = splot_decode::DecodeContext::new(splot_decode::DecodeRuntimeConfig::new(threads));
    let options = splot_decode::DecodeOptions::default().with_skip_film_grain(skip);
    let mut out = Vec::new();
    ctx.unwrap()
        .decode_raw_bytes(&units, options, &mut out)
        .unwrap();
    out
}

/// With film grain and `open_loop_key = 1` every key frame sends a grain
/// model, and every output frame after the closed-loop key frame and after
/// each of the two hidden open-loop key frames carries grain with its own
/// seed (§ 7.21.7). Without grain synthesis the stream decodes to the encoder
/// recon, and the bytes are the same on 1, 2 and 8 threads.
#[test]
fn open_loop_key_streams_carry_film_grain() {
    use splot_core::types::ObuType;
    each_case([OPEN_LOOP_CASES[0], OPEN_LOOP_CASES[1]], |case| {
        let qp = RateControlMode::ConstantQp { qp: 150 };
        let (_, packets, expected) = open_loop_stream(case, qp, 1, true);
        let types = obu_types(&packets);
        let count = |kind| types.iter().filter(|&&obu| obu == kind).count();
        assert_eq!(count(ObuType::OpenLoopKey), 2, "{case:?}");
        assert_eq!(count(ObuType::FilmGrain), 3, "{case:?}");
        assert_same_samples(
            &decode_packets(&packets, true),
            &expected,
            &format!("{case:?}"),
        );
        let grained = decode_packets(&packets, false);
        assert_eq!(grained.len(), expected.len(), "{case:?}");
        let frame_bytes = expected.len() / 26;
        let mut noise: Vec<Vec<i16>> = Vec::new();
        for (index, (frame, recon)) in grained
            .chunks(frame_bytes)
            .zip(expected.chunks(frame_bytes))
            .enumerate()
        {
            let diff: Vec<i16> = frame
                .iter()
                .zip(recon)
                .map(|(&a, &b)| i16::from(a) - i16::from(b))
                .collect();
            assert!(
                diff.iter().any(|&d| d != 0),
                "{case:?}: frame {index} has no grain"
            );
            assert!(
                !noise.contains(&diff),
                "{case:?}: frame {index} repeats a seed"
            );
            noise.push(diff);
        }
        if case == OPEN_LOOP_CASES[0] {
            let other = open_loop_stream(case, qp, 2, true).1;
            assert!(
                other
                    .iter()
                    .map(|packet| &packet.data)
                    .eq(packets.iter().map(|packet| &packet.data)),
                "{case:?}: 2 threads"
            );
        }
    });
}

/// AVM synthesizes the same grain as splot on the open-loop key streams with
/// film grain, from the start and from the first open-loop key frame.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_open_loop_key_grain_streams_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for case in [OPEN_LOOP_CASES[0], OPEN_LOOP_CASES[1]] {
        let qp = RateControlMode::ConstantQp { qp: 150 };
        let (config, packets, _) = open_loop_stream(case, qp, 1, true);
        let grained = decode_packets(&packets, false);
        let size = (config.width, config.height);
        let units: Vec<Vec<u8>> = packets.iter().map(|packet| packet.data.clone()).collect();
        let name = format!("olk-grain{}", case.1 as u8);
        let avm = avm_decode(&avmdec, &units, size, config.bit_depth, &name);
        assert!(avm == grained, "{case:?}: AVM output differs");
        let start = packets
            .iter()
            .position(|packet| {
                obu_types(core::slice::from_ref(packet))
                    .contains(&splot_core::types::ObuType::OpenLoopKey)
            })
            .unwrap();
        let avm = avm_decode(&avmdec, &units[start..], size, config.bit_depth, &name);
        assert!(
            avm == grained[12 * (grained.len() / 26)..],
            "{case:?}: random access differs"
        );
    }
}

// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Intra coding tools in coded streams: MRL (§ 7.13.2), IST (§ 7.15.3), CCTX
//! (§ 7.14.6), MHCCP (§ 5.20.7.32), FSC (§ 5.20.7.27), DIP (§ 5.20.7.9),
//! palette (§ 5.20.8.1) and the CfL down-sampling filter (§ 7.13.5).

use super::*;

/// A key frame and two inter frames of a two-tile `chroma` pan at `bit_depth`
/// on `threads` threads: the units, the encoder recon, and the sequence.
fn tool_stream(
    case: (usize, PixelFormat, BitDepth),
    preset: &'static PresetParams,
) -> (Vec<Vec<u8>>, Vec<u8>, SequenceHeader) {
    cfl_stream(case, preset, 0, panning_frame)
}

/// [`tool_stream`] with `cfl_ds_filter_index` `cfl_filter`.
fn cfl_stream(
    (threads, chroma, bit_depth): (usize, PixelFormat, BitDepth),
    preset: &'static PresetParams,
    cfl_filter: u8,
    frame: fn(&Config, u32) -> SourceFrame,
) -> (Vec<Vec<u8>>, Vec<u8>, SequenceHeader) {
    let mut config = config(136, 72, bit_depth, 9, 120);
    config.chroma = chroma;
    config.tiles = TileLayout::Uniform {
        cols_log2: 1,
        rows_log2: 0,
    };
    config.threads = splot_parallel::ThreadCount::from(threads);
    let mut coder = IntraFrameCoder::with_preset(&config, preset).unwrap();
    coder.set_cfl_filter(cfl_filter).unwrap();
    let (mut units, mut recon) = (Vec::new(), Vec::new());
    for index in 0..3 {
        let plan = crate::gop::plan(u64::from(index), config.keyint);
        let coded = coder
            .encode_frame(&frame(&config, index), &plan, 120)
            .unwrap();
        recon.extend(recon_bytes(&coded, bit_depth));
        units.push(coded.temporal_unit);
    }
    (units, recon, coder.sequence)
}

/// Every `cfl_ds_filter_index` reaches the sequence header, and its CfL and
/// chroma Wiener-NS samples decode to the encoder recon in every chroma
/// format with chroma and both bit depths.
#[test]
fn cfl_filters_decode_to_the_encoder_recon() {
    for chroma in CHROMAS.into_iter().filter(|chroma| !chroma.is_monochrome()) {
        for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
            for index in 1..3 {
                let (units, recon, seq) =
                    cfl_stream((2, chroma, bit_depth), &PRESETS[9], index, panning_frame);
                assert_eq!(seq.intra.unwrap().cfl_ds_filter_index, index);
                let name = format!("{chroma:?} {bit_depth:?} filter {index}");
                assert_same_samples(&decode(&units.concat()), &recon, &name);
            }
        }
    }
}

/// AVM decodes the CfL and chroma Wiener-NS samples of every filter like splot.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_cfl_filters_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for (index, bit_depth) in [(1, BitDepth::Eight), (2, BitDepth::Ten)] {
        let (units, recon, _) = cfl_stream(
            (2, PixelFormat::Yuv420, bit_depth),
            &PRESETS[9],
            index,
            panning_frame,
        );
        let avm = avm_decode(&avmdec, &units, (136, 72), bit_depth, "cfl filter");
        assert!(avm == recon, "filter {index}: AVM output differs");
    }
}

/// Preset 9, fast enough for debug tests, with `set` applied.
fn preset_with(set: fn(&mut PresetParams)) -> &'static PresetParams {
    let mut preset = PRESETS[9];
    set(&mut preset);
    Box::leak(Box::new(preset))
}

/// Preset 9 with the MRL trials of preset 6.
fn mrl_preset() -> &'static PresetParams {
    preset_with(|p| p.mrl_trials = PRESETS[6].mrl_trials)
}

/// Preset 9 with every IST set.
fn ist_preset() -> &'static PresetParams {
    preset_with(|p| p.ist_sets = splot_decode::encoder_syntax::IST_SETS)
}

/// Streams of `on` in every chroma format and both bit depths decode to the
/// encoder recon with `enabled` set, and 1, 2 and 8 threads write the same
/// bytes; `off` clears `enabled` and changes the stream.
fn assert_tool_streams(
    on: &'static PresetParams,
    off: &'static PresetParams,
    enabled: fn(&SequenceHeader) -> bool,
) {
    for chroma in CHROMAS {
        for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
            let (units, recon, seq) = tool_stream((2, chroma, bit_depth), on);
            assert!(enabled(&seq));
            let name = format!("{chroma:?} {bit_depth:?}");
            assert_same_samples(&decode(&units.concat()), &recon, &name);
        }
    }
    let case = (1, PixelFormat::Yuv420, BitDepth::Ten);
    let (one, ..) = tool_stream(case, on);
    {
        let threads = 2;
        assert!(
            tool_stream((threads, case.1, case.2), on).0 == one,
            "{threads} threads"
        );
    }
    let (units, recon, seq) = tool_stream(case, off);
    assert!(!enabled(&seq));
    assert_same_samples(&decode(&units.concat()), &recon, "off");
    assert_ne!(units, one);
}

/// AVM decodes the key and inter frames of `preset` to the encoder recon.
fn assert_avm_decodes(preset: &'static PresetParams, name: &str) {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
        let (units, recon, _) = tool_stream((2, PixelFormat::Yuv420, bit_depth), preset);
        let avm = avm_decode(&avmdec, &units, (136, 72), bit_depth, name);
        assert!(avm == recon, "{name} {bit_depth:?}: AVM output differs");
    }
}

/// MRL streams decode to the encoder recon on any thread count.
#[test]
fn mrl_frames_decode_to_the_encoder_recon_on_any_thread_count() {
    assert_tool_streams(mrl_preset(), preset_with(|p| p.mrl_trials = 0), |seq| {
        seq.intra.unwrap().enable_mrls
    });
}

/// The key-frame search codes leaves on every MRL line.
#[test]
fn search_picks_mrl_lines() {
    let config = config(136, 72, BitDepth::Ten, 9, 90);
    let coder = IntraFrameCoder::with_preset(&config, mrl_preset()).unwrap();
    let plan = crate::gop::plan(0, config.keyint);
    let trees = search_frame(
        &coder,
        &plan,
        &synthetic_frame(&config, 3),
        config.bit_depth,
    );
    for index in 1..=3 {
        let line = |choice: &LeafChoice| choice.inter.is_none() && choice.modes.mrl.index == index;
        let count: usize = trees.iter().map(|tree| leaves(tree, line)).sum();
        assert!(count > 0, "no leaf on MRL line {index}");
    }
}

/// AVM decodes the MRL key and inter frames to the encoder recon.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_mrl_frames_like_splot() {
    assert_avm_decodes(mrl_preset(), "mrl");
}

/// IST streams decode to the encoder recon on any thread count.
#[test]
fn ist_frames_decode_to_the_encoder_recon_on_any_thread_count() {
    assert_tool_streams(ist_preset(), preset_with(|p| p.ist_sets = 0), |seq| {
        seq.transform_quant_entropy.unwrap().enable_intra_ist
    });
}

/// CCTX streams decode to the encoder recon on any thread count; a
/// monochrome sequence has no chroma to code it for.
#[test]
fn cctx_frames_decode_to_the_encoder_recon_on_any_thread_count() {
    crate::mode_decision::CCTX_RECORDED.set(0);
    let (units, recon, _) = tool_stream(
        (1, PixelFormat::Yuv420, BitDepth::Ten),
        preset_with(|p| p.cctx = true),
    );
    assert!(
        crate::mode_decision::CCTX_RECORDED.get() > 0,
        "the final write coded no CCTX pair"
    );
    assert_same_samples(&decode(&units.concat()), &recon, "cctx");
    assert_tool_streams(preset_with(|p| p.cctx = true), preset_with(|_| {}), |seq| {
        seq.transform_quant_entropy.unwrap().enable_cctx
            || seq.general.chroma_format_idc.is_monochrome()
    });
}

/// MHCCP streams decode to the encoder recon on any thread count.
#[test]
fn mhccp_frames_decode_to_the_encoder_recon_on_any_thread_count() {
    assert_tool_streams(
        preset_with(|p| p.mhccp = true),
        preset_with(|_| {}),
        |seq| seq.intra.unwrap().enable_mhccp,
    );
}

/// FSC streams decode to the encoder recon on any thread count.
#[test]
fn fsc_frames_decode_to_the_encoder_recon_on_any_thread_count() {
    assert_tool_streams(
        preset_with(|p| p.fsc = true),
        preset_with(|p| p.fsc = false),
        |seq| seq.transform_quant_entropy.unwrap().enable_fsc,
    );
}

/// The key-frame search codes FSC leaves.
#[test]
fn search_picks_fsc_leaves() {
    let config = config(136, 72, BitDepth::Eight, 9, 90);
    let coder = IntraFrameCoder::with_preset(&config, preset_with(|p| p.fsc = true)).unwrap();
    let plan = crate::gop::plan(0, config.keyint);
    let trees = search_frame(
        &coder,
        &plan,
        &synthetic_frame(&config, 3),
        config.bit_depth,
    );
    let fsc = |choice: &LeafChoice| choice.inter.is_none() && choice.modes.fsc;
    let count: usize = trees.iter().map(|tree| leaves(tree, fsc)).sum();
    assert!(count > 0);
}

/// AVM decodes the FSC key and inter frames to the encoder recon.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_fsc_frames_like_splot() {
    assert_avm_decodes(preset_with(|p| p.fsc = true), "fsc");
}

/// IDTX-intra streams (`enable_idtx_intra` without FSC) decode to the
/// encoder recon on any thread count.
#[test]
fn idtx_intra_frames_decode_to_the_encoder_recon_on_any_thread_count() {
    assert_tool_streams(
        preset_with(|p| p.idtx_intra = true),
        preset_with(|_| {}),
        |seq| {
            let tq = seq.transform_quant_entropy.unwrap();
            tq.enable_idtx_intra && !tq.enable_fsc
        },
    );
}

/// Without FSC, the key-frame search codes IDTX-intra leaves.
#[test]
fn search_picks_idtx_intra_leaves() {
    let config = config(136, 72, BitDepth::Eight, 9, 90);
    let preset = preset_with(|p| p.idtx_intra = true);
    let coder = IntraFrameCoder::with_preset(&config, preset).unwrap();
    let plan = crate::gop::plan(0, config.keyint);
    let trees = search_frame(
        &coder,
        &plan,
        &synthetic_frame(&config, 3),
        config.bit_depth,
    );
    let idtx = |choice: &LeafChoice| choice.inter.is_none() && choice.modes.fsc;
    let count: usize = trees.iter().map(|tree| leaves(tree, idtx)).sum();
    assert!(count > 0);
}

/// AVM decodes the IDTX-intra key and inter frames to the encoder recon.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_idtx_intra_frames_like_splot() {
    assert_avm_decodes(preset_with(|p| p.idtx_intra = true), "idtx-intra");
}

/// Frame `index` of drawn text panning by `(4, 2)` luma samples per frame:
/// two luma levels with an edge level between, on flat chroma.
pub(super) fn screen_frame(config: &Config, index: u32) -> SourceFrame {
    drawn_text(config, index, 7)
}

/// [`screen_frame`] whose glyph pattern advances `stride` levels per column.
fn drawn_text(config: &Config, index: u32, stride: u32) -> SourceFrame {
    let max = u32::from(config.bit_depth.max_sample());
    let sizes = config.plane_sizes().unwrap();
    let planes = core::array::from_fn(|plane| {
        let Some(&size) = sizes.get(plane) else {
            return Vec::new();
        };
        let mut samples = Vec::with_capacity(size.width() * size.height());
        for y in 0..size.height() as u32 {
            for x in 0..size.width() as u32 {
                let (x, y) = (x + 4 * index, y + 2 * index);
                let luma = match (x * stride + y * 3 + x / 5 * (y / 3)) % 13 {
                    0..=2 => max / 10,
                    3 => max / 2 + 3,
                    _ => max * 9 / 10,
                };
                samples.push(if plane == 0 {
                    luma
                } else {
                    max / 2 + 7 * plane as u32
                } as u16);
            }
        }
        samples
    });
    SourceFrame::new(config, u64::from(index), planes).unwrap()
}

/// Palette streams decode to the encoder recon on any thread count.
#[test]
fn palette_frames_decode_to_the_encoder_recon_on_any_thread_count() {
    assert_tool_streams(
        preset_with(|p| p.palette = true),
        preset_with(|p| p.palette = false),
        |seq| seq.screen_content.unwrap().seq_force_screen_content_tools == 2,
    );
}

/// Drawn text codes palette leaves in key frames and, over new text, in
/// inter frames, at both bit depths and in 4:2:0 and 4:4:4. Its streams
/// decode to the encoder recon in every chroma format and both bit depths,
/// and 1, 2 and 8 threads write the same bytes.
#[test]
fn screen_streams_code_palette_leaves() {
    let preset = preset_with(|p| p.palette = true);
    let palette = |choice: &LeafChoice| choice.inter.is_none() && choice.modes.palette.is_some();
    for (chroma, bit_depth) in [
        (PixelFormat::Yuv420, BitDepth::Ten),
        (PixelFormat::Yuv420, BitDepth::Eight),
        (PixelFormat::Yuv444, BitDepth::Eight),
    ] {
        let mut config = config(136, 72, bit_depth, 9, 90);
        config.chroma = chroma;
        let mut coder = IntraFrameCoder::with_preset(&config, preset).unwrap();
        let key = crate::gop::plan(0, config.keyint);
        let trees = search_frame(&coder, &key, &screen_frame(&config, 0), bit_depth);
        let count: usize = trees.iter().map(|tree| leaves(tree, palette)).sum();
        assert!(count > 0, "key {chroma:?} {bit_depth:?}");
        coder
            .encode_frame(&screen_frame(&config, 0), &key, 90)
            .unwrap();
        let plan = crate::gop::plan(1, config.keyint);
        let trees = search_frame(&coder, &plan, &drawn_text(&config, 1, 5), bit_depth);
        let count: usize = trees.iter().map(|tree| leaves(tree, palette)).sum();
        assert!(count > 0, "inter {chroma:?} {bit_depth:?}");
    }
    for chroma in CHROMAS {
        for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
            let (units, recon, _) = cfl_stream((2, chroma, bit_depth), preset, 0, screen_frame);
            let name = format!("{chroma:?} {bit_depth:?}");
            assert_same_samples(&decode(&units.concat()), &recon, &name);
        }
    }
    let case = |threads| (threads, PixelFormat::Yuv420, BitDepth::Ten);
    let (one, ..) = cfl_stream(case(1), preset, 0, screen_frame);
    {
        let threads = 2;
        let (units, ..) = cfl_stream(case(threads), preset, 0, screen_frame);
        assert!(units == one, "{threads} threads");
    }
}

/// Frame `index` of a 24x16 glyph tile repeated over the frame, panning by
/// `(4, 2)` luma samples per frame, on flat chroma.
pub(super) fn tiled_text(config: &Config, index: u32) -> SourceFrame {
    let max = u32::from(config.bit_depth.max_sample());
    let sizes = config.plane_sizes().unwrap();
    let planes = core::array::from_fn(|plane| {
        let Some(&size) = sizes.get(plane) else {
            return Vec::new();
        };
        let mut samples = Vec::with_capacity(size.width() * size.height());
        for y in 0..size.height() as u32 {
            for x in 0..size.width() as u32 {
                let (x, y) = ((x + 4 * index) % 24, (y + 2 * index) % 16);
                let ink = (x * 5 + y * y * 3 + x * y) % 7 < 2;
                let luma = if ink { max / 8 } else { max * 7 / 8 };
                samples.push(if plane == 0 {
                    luma
                } else {
                    max / 2 + 5 * plane as u32
                } as u16);
            }
        }
        samples
    });
    SourceFrame::new(config, u64::from(index), planes).unwrap()
}

/// Two key frames of [`tiled_text`] at 256x128 coded with `preset` on
/// `threads` threads: the units and the encoder recon.
fn intrabc_stream(
    case: (usize, PixelFormat, BitDepth),
    preset: &'static PresetParams,
) -> (Vec<u8>, Vec<u8>) {
    source_stream(case, preset, (256, tiled_text, 0))
}

/// [`intrabc_stream`] of `source` frames `width` samples wide, in
/// `1 << tiles_log2` tile rows and columns.
fn source_stream(
    (threads, chroma, bit_depth): (usize, PixelFormat, BitDepth),
    preset: &'static PresetParams,
    (width, source, tiles_log2): (u32, fn(&Config, u32) -> SourceFrame, u8),
) -> (Vec<u8>, Vec<u8>) {
    let mut config = config(width, 128, bit_depth, 9, 90);
    config.tiles = TileLayout::Uniform {
        cols_log2: tiles_log2,
        rows_log2: tiles_log2,
    };
    config.chroma = chroma;
    config.keyint = core::num::NonZeroU32::MIN;
    config.threads = splot_parallel::ThreadCount::from(threads);
    let mut coder = IntraFrameCoder::with_preset(&config, preset).unwrap();
    let (mut units, mut recon) = (Vec::new(), Vec::new());
    let plan = crate::gop::plan(0, config.keyint);
    let coded = coder.encode_frame(&source(&config, 0), &plan, 120).unwrap();
    recon.extend(recon_bytes(&coded, bit_depth));
    units.push(coded.temporal_unit);
    (units.concat(), recon)
}

/// Preset 9 with local-area IntraBC on leaves down to 4x4 and SDP off (as
/// the screen-content rows), and `intrabc_small` as `small` says.
fn small_intrabc_preset(small: bool) -> &'static PresetParams {
    let mut preset = PRESETS[9];
    (preset.intrabc, preset.intrabc_local, preset.intrabc_small) = (true, true, small);
    preset.sdp = false;
    preset.min_block_log2 = crate::preset::Levels::all(2);
    Box::leak(Box::new(preset))
}

/// [`small_intrabc_preset`] with `intrabc_small` and leaves of at most 8x8.
fn tiny_intrabc_preset() -> &'static PresetParams {
    let mut preset = *small_intrabc_preset(true);
    preset.max_block_log2 = 3;
    Box::leak(Box::new(preset))
}

/// With `intrabc_small`, the final passes code IntraBC leaves under 8
/// samples on a side, and none without it. With leaves of at most 8x8 and
/// IntraBC the only candidate where it has one, some of them code chroma (chroma-offset
/// leaves in 4:2:0) and decode to the encoder recon. The streams decode to
/// the encoder recon in every chroma format and both bit depths, and 1, 2
/// and 8 threads write the same bytes.
#[test]
fn small_intrabc_codes_small_and_chroma_offset_leaves() {
    let coded = |preset, case| {
        crate::tile_writer::SMALL_INTRABC.set([0; 2]);
        let (units, recon) = intrabc_stream(case, preset);
        (units, recon, crate::tile_writer::SMALL_INTRABC.get())
    };
    let case = (1, PixelFormat::Yuv420, BitDepth::Eight);
    assert_eq!(coded(small_intrabc_preset(false), case).2, [0, 0]);
    assert!(coded(small_intrabc_preset(true), case).2[0] > 0);
    crate::mode_decision::FORCE_INTRABC.set(true);
    let (units, recon, [small, chroma]) = coded(tiny_intrabc_preset(), case);
    crate::mode_decision::FORCE_INTRABC.set(false);
    assert!(small > chroma && chroma > 0, "{small} {chroma}");
    assert_same_samples(&decode(&units), &recon, "forced IntraBC");
    let preset = small_intrabc_preset(true);
    each_case(
        [
            (PixelFormat::Yuv420, BitDepth::Eight),
            (PixelFormat::Yuv422, BitDepth::Ten),
            (PixelFormat::Yuv444, BitDepth::Eight),
            (PixelFormat::Monochrome, BitDepth::Ten),
        ],
        |(chroma, bit_depth)| {
            let (units, recon, _) = coded(preset, (2, chroma, bit_depth));
            let name = format!("{chroma:?} {bit_depth:?}");
            assert_same_samples(&decode(&units), &recon, &name);
        },
    );
    let (one, ..) = coded(preset, (1, PixelFormat::Yuv420, BitDepth::Ten));
    each_case([2], |threads| {
        let (bytes, ..) = coded(preset, (threads, PixelFormat::Yuv420, BitDepth::Ten));
        assert!(bytes == one, "{threads} threads");
    });
}

/// AVM decodes key frames with small and chroma-offset IntraBC leaves (leaves
/// of at most 8x8, IntraBC the only candidate where it has one) to the
/// encoder recon.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_small_intrabc_frames_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for (chroma, bit_depth) in [
        (PixelFormat::Yuv420, BitDepth::Eight),
        (PixelFormat::Yuv422, BitDepth::Ten),
    ] {
        crate::mode_decision::FORCE_INTRABC.set(true);
        let (units, recon) = intrabc_stream((1, chroma, bit_depth), tiny_intrabc_preset());
        crate::mode_decision::FORCE_INTRABC.set(false);
        let avm = avm_decode(&avmdec, &[units], (256, 128), bit_depth, "small-intrabc");
        assert!(
            avm == recon,
            "small intrabc {chroma:?} {bit_depth:?}: AVM output differs"
        );
    }
}

/// AVM decodes the IntraBC key frames of repeated text to the encoder recon.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_intrabc_frames_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    let preset = preset_with(|p| p.intrabc = true);
    for (chroma, bit_depth) in [
        (PixelFormat::Yuv420, BitDepth::Eight),
        (PixelFormat::Yuv420, BitDepth::Ten),
    ] {
        let (units, recon) = intrabc_stream((2, chroma, bit_depth), preset);
        let avm = avm_decode(&avmdec, &[units], (256, 128), bit_depth, "intrabc");
        assert!(avm == recon, "intrabc {bit_depth:?}: AVM output differs");
    }
}

/// Two key frames of [`half_sample_copy`] with IntraBC, `intrabc_subpel`
/// as `subpel` says, and IntraBC the only candidate where it has one: the
/// units, the encoder recon and the count of final-pass leaves with a
/// fractional vector.
fn subpel_stream(subpel: bool, case: (usize, PixelFormat, BitDepth)) -> (Vec<u8>, Vec<u8>, usize) {
    let preset = if subpel {
        preset_with(|p| (p.intrabc, p.intrabc_subpel) = (true, true))
    } else {
        preset_with(|p| p.intrabc = true)
    };
    crate::mode_decision::FORCE_INTRABC.set(true);
    crate::tile_writer::QUARTER_INTRABC.set(0);
    let (units, recon) = source_stream(case, preset, (640, half_sample_copy, 0));
    crate::mode_decision::FORCE_INTRABC.set(false);
    (units, recon, crate::tile_writer::QUARTER_INTRABC.get())
}

/// A 640x128 frame of noise with two 32x32 blocks at (64, 64) and
/// (448, 64): both start with the same 8x8 luma ramp, the first goes on
/// with fine vertical stripes and the second with the half-sample average
/// of the first block's samples, so its best copy is fractional.
fn half_sample_copy(config: &Config, index: u32) -> SourceFrame {
    let max = f64::from(config.bit_depth.max_sample());
    let sizes = config.plane_sizes().unwrap();
    let planes = core::array::from_fn(|plane| {
        let Some(&size) = sizes.get(plane) else {
            return Vec::new();
        };
        let (w, h) = (size.width(), size.height());
        if plane > 0 {
            return vec![(max / 2.0) as u16 + 5 * plane as u16; w * h];
        }
        let noise = |x: usize, y: usize| {
            let v = (x * 7919 + y * 104_729 + index as usize * 31).wrapping_mul(2_654_435_761);
            max * (0.2 + 0.6 * f64::from((v >> 7) as u32 % 1000) / 1000.0)
        };
        let block = |xl: usize, yl: usize| {
            if xl < 8 && yl < 8 {
                max * (0.4 + xl as f64 / 32.0 + yl as f64 / 64.0)
            } else {
                max * (0.5 + 0.35 * (xl as f64 * 1.25).sin())
            }
        };
        let mut samples: Vec<f64> = (0..w * h).map(|i| noise(i % w, i / w)).collect();
        for yl in 0..32 {
            for xl in 0..32 {
                samples[(64 + yl) * w + 64 + xl] = block(xl, yl);
            }
        }
        for yl in 0..32 {
            for xl in 0..32 {
                let a = |x: usize| samples[(64 + yl) * w + 64 + x].round();
                samples[(64 + yl) * w + 448 + xl] = if xl < 8 && yl < 8 {
                    a(xl)
                } else {
                    ((a(xl) + a(xl + 1)) / 2.0).ceil()
                };
            }
        }
        samples.into_iter().map(|v| v.round() as u16).collect()
    });
    SourceFrame::new(config, u64::from(index), planes).unwrap()
}

/// With `intrabc_subpel`, the quarter-sample refinement finds the
/// half-sample copy of [`half_sample_copy`] (IntraBC the only candidate
/// where it has one) in both bit depths, and none without the field. The
/// streams decode to the encoder recon in every chroma format and both bit
/// depths, and 1, 2 and 8 threads write the same bytes.
#[test]
fn subpel_intrabc_codes_fractional_vectors() {
    let coded = subpel_stream;
    for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
        let case = (1, PixelFormat::Yuv420, bit_depth);
        assert_eq!(coded(false, case).2, 0, "{bit_depth:?}");
        let (units, recon, quarter) = coded(true, case);
        assert!(quarter > 0, "{bit_depth:?}");
        assert_same_samples(&decode(&units), &recon, "forced IntraBC");
    }
    each_case(
        [
            (PixelFormat::Yuv422, BitDepth::Eight),
            (PixelFormat::Yuv444, BitDepth::Ten),
            (PixelFormat::Monochrome, BitDepth::Eight),
        ],
        |(chroma, bit_depth)| {
            let (units, recon, _) = coded(true, (2, chroma, bit_depth));
            let name = format!("{chroma:?} {bit_depth:?}");
            assert_same_samples(&decode(&units), &recon, &name);
        },
    );
    let (one, ..) = coded(true, (1, PixelFormat::Yuv420, BitDepth::Ten));
    each_case([2], |threads| {
        let (bytes, ..) = coded(true, (threads, PixelFormat::Yuv420, BitDepth::Ten));
        assert!(bytes == one, "{threads} threads");
    });
}

/// AVM decodes the [`subpel_stream`] key frames to the encoder recon.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_subpel_intrabc_frames_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for (chroma, bit_depth) in [
        (PixelFormat::Yuv420, BitDepth::Eight),
        (PixelFormat::Yuv422, BitDepth::Ten),
    ] {
        let (units, recon, quarter) = subpel_stream(true, (1, chroma, bit_depth));
        assert!(quarter > 0, "{chroma:?} {bit_depth:?}");
        let name = format!("subpel-intrabc-{chroma:?}");
        let avm = avm_decode(&avmdec, &[units], (640, 128), bit_depth, &name);
        assert!(avm == recon, "{chroma:?} {bit_depth:?}: AVM output differs");
    }
}

/// [`intrabc_stream`] with IntraBC the only candidate where it has one and
/// each vector moved by a quarter sample where it can: the units, the
/// encoder recon and the count of final-pass leaves with a fractional
/// vector.
fn quarter_intrabc_stream(
    case: (usize, PixelFormat, BitDepth),
    preset: &'static PresetParams,
) -> (Vec<u8>, Vec<u8>, usize) {
    crate::mode_decision::FORCE_INTRABC.set(true);
    crate::intrabc::QUARTER_BV.set(true);
    crate::tile_writer::QUARTER_INTRABC.set(0);
    let (units, recon) = intrabc_stream(case, preset);
    crate::intrabc::QUARTER_BV.set(false);
    crate::mode_decision::FORCE_INTRABC.set(false);
    (units, recon, crate::tile_writer::QUARTER_INTRABC.get())
}

/// The global and the local-area presets with their quarter-sample cases.
fn quarter_intrabc_cases() -> [(&'static PresetParams, PixelFormat, BitDepth); 2] {
    [
        (
            preset_with(|p| p.intrabc = true),
            PixelFormat::Yuv420,
            BitDepth::Eight,
        ),
        (
            small_intrabc_preset(true),
            PixelFormat::Yuv444,
            BitDepth::Ten,
        ),
    ]
}

/// Key frames with quarter-sample IntraBC vectors (§ 5.20.5.4
/// `intrabc_precision = 1`, the § 7.13.3 BILINEAR filter) decode to the
/// encoder recon, for the global and the local area.
#[test]
fn quarter_sample_intrabc_frames_decode_to_the_encoder_recon() {
    for (preset, chroma, bit_depth) in quarter_intrabc_cases() {
        let (units, recon, quarter) = quarter_intrabc_stream((1, chroma, bit_depth), preset);
        let name = format!("{chroma:?} {bit_depth:?}");
        assert!(quarter > 0, "{name}");
        assert_same_samples(&decode(&units), &recon, &name);
    }
}

/// AVM decodes key frames with quarter-sample IntraBC vectors to the
/// encoder recon.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_quarter_sample_intrabc_frames_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for (preset, chroma, bit_depth) in quarter_intrabc_cases() {
        let (units, recon, _) = quarter_intrabc_stream((1, chroma, bit_depth), preset);
        let name = format!("quarter-intrabc-{chroma:?}");
        let avm = avm_decode(&avmdec, &[units], (256, 128), bit_depth, &name);
        assert!(avm == recon, "{chroma:?} {bit_depth:?}: AVM output differs");
    }
}

/// Preset 9 with IntraBC, the local area as `local` says, and superblocks
/// of `1 << sb_log2`.
fn local_intrabc_preset(local: bool, sb_log2: u8) -> &'static PresetParams {
    let mut preset = PRESETS[9];
    (preset.intrabc, preset.intrabc_local, preset.max_block_log2) = (true, local, sb_log2);
    Box::leak(Box::new(preset))
}

/// In the first 64 rows of a 256-wide key frame no global-area vector is
/// valid (§ 6.19.7.12), so its IntraBC leaves there copy from the local
/// area: some with `intrabc_local`, none without, with 64x64 and 128x128
/// superblocks. With the local area the search does not replay memoized
/// trees with IntraBC leaves; it searches their nodes again.
#[test]
fn local_intrabc_codes_copies_in_the_first_superblock_row() {
    let intrabc = |choice: &LeafChoice| choice.intrabc.is_some();
    let first_row = |local, sb_log2| {
        let config = config(256, 128, BitDepth::Eight, 9, 90);
        let preset = local_intrabc_preset(local, sb_log2);
        let coder = IntraFrameCoder::with_preset(&config, preset).unwrap();
        let key = crate::gop::plan(0, config.keyint);
        let trees = search_frame(&coder, &key, &tiled_text(&config, 0), BitDepth::Eight);
        let per_row = 256 >> sb_log2;
        let count: usize = trees[..per_row].iter().map(|t| leaves(t, intrabc)).sum();
        count
    };
    for sb_log2 in [6, 7] {
        assert_eq!(first_row(false, sb_log2), 0, "global only, sb {sb_log2}");
        crate::mode_decision::SKIPPED_REPLAYS.set(0);
        assert!(first_row(true, sb_log2) > 0, "local, sb {sb_log2}");
        assert!(
            crate::mode_decision::SKIPPED_REPLAYS.get() > 0,
            "sb {sb_log2}"
        );
    }
}

/// Local-area IntraBC streams decode to the encoder recon in every chroma
/// format and both bit depths, and 1, 2 and 8 threads write the same bytes.
#[test]
fn local_intrabc_streams_decode_to_the_encoder_recon_on_any_thread_count() {
    for sb_log2 in [6, 7] {
        let stream = |case| intrabc_stream(case, local_intrabc_preset(true, sb_log2));
        each_case(CHROMAS, |chroma| {
            for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
                let (units, recon) = stream((2, chroma, bit_depth));
                let name = format!("{chroma:?} {bit_depth:?} sb {sb_log2}");
                assert_same_samples(&decode(&units), &recon, &name);
            }
        });
        let (one, _) = stream((1, PixelFormat::Yuv420, BitDepth::Ten));
        each_case([2], |threads| {
            let (bytes, _) = stream((threads, PixelFormat::Yuv420, BitDepth::Ten));
            assert!(bytes == one, "{threads} threads, sb {sb_log2}");
        });
    }
}

/// AVM decodes local-area IntraBC key frames to the encoder recon.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_local_intrabc_frames_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for (sb_log2, bit_depth) in [(6, BitDepth::Eight), (7, BitDepth::Ten)] {
        let preset = local_intrabc_preset(true, sb_log2);
        let (units, recon) = intrabc_stream((2, PixelFormat::Yuv420, bit_depth), preset);
        let avm = avm_decode(&avmdec, &[units], (256, 128), bit_depth, "local-intrabc");
        assert!(
            avm == recon,
            "local intrabc sb {sb_log2}: AVM output differs"
        );
    }
}

/// With `intrabc_drl = 2` the sequence codes `seq_max_bvp_drl_bits_minus_1 =
/// 2`, some IntraBC leaves code `RefMvIdx` 2 or 3, and the streams decode
/// to the encoder recon in every chroma format and both bit depths; 1, 2
/// and 8 threads write the same bytes.
#[test]
fn intrabc_drl_streams_code_far_reference_vectors() {
    let preset = preset_with(|p| (p.intrabc, p.intrabc_drl) = (true, 2));
    let far = |choice: &LeafChoice| choice.intrabc.is_some_and(|modes| modes.drl >= 2);
    let config = config(256, 128, BitDepth::Eight, 9, 90);
    let coder = IntraFrameCoder::with_preset(&config, preset).unwrap();
    assert_eq!(
        coder
            .sequence
            .inter
            .as_ref()
            .unwrap()
            .seq_max_bvp_drl_bits_minus_1,
        2
    );
    let key = crate::gop::plan(0, config.keyint);
    let trees = search_frame(&coder, &key, &tiled_text(&config, 0), BitDepth::Eight);
    assert!(trees.iter().map(|tree| leaves(tree, far)).sum::<usize>() > 0);
    let stream = |case| intrabc_stream(case, preset);
    each_case(CHROMAS, |chroma| {
        for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
            let (units, recon) = stream((2, chroma, bit_depth));
            assert_same_samples(
                &decode(&units),
                &recon,
                &format!("{chroma:?} {bit_depth:?}"),
            );
        }
    });
    let (one, _) = stream((1, PixelFormat::Yuv420, BitDepth::Ten));
    each_case([2], |threads| {
        assert!(
            stream((threads, PixelFormat::Yuv420, BitDepth::Ten)).0 == one,
            "{threads} threads"
        );
    });
}

/// AVM decodes key frames whose IntraBC leaves pick among four reference
/// block vectors to the encoder recon.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_intrabc_drl_frames_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    let preset = preset_with(|p| (p.intrabc, p.intrabc_drl) = (true, 2));
    for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
        let (units, recon) = intrabc_stream((2, PixelFormat::Yuv420, bit_depth), preset);
        let avm = avm_decode(&avmdec, &[units], (256, 128), bit_depth, "intrabc-drl");
        assert!(
            avm == recon,
            "intrabc drl {bit_depth:?}: AVM output differs"
        );
    }
}

/// Repeated text codes IntraBC leaves in key frames at both bit depths and
/// in 4:2:0 and 4:4:4; its streams decode to the encoder recon in every
/// chroma format and both bit depths, and 1, 2 and 8 threads write the same
/// bytes.
#[test]
fn screen_streams_code_intrabc_leaves() {
    let preset = preset_with(|p| p.intrabc = true);
    let intrabc = |choice: &LeafChoice| choice.intrabc.is_some();
    let stream = |case| intrabc_stream(case, preset);
    each_case(
        [
            (PixelFormat::Yuv420, BitDepth::Ten),
            (PixelFormat::Yuv444, BitDepth::Eight),
        ],
        |(chroma, bit_depth)| {
            let mut config = config(256, 128, bit_depth, 9, 90);
            config.chroma = chroma;
            let coder = IntraFrameCoder::with_preset(&config, preset).unwrap();
            let key = crate::gop::plan(0, config.keyint);
            let trees = search_frame(&coder, &key, &tiled_text(&config, 0), bit_depth);
            let count: usize = trees.iter().map(|tree| leaves(tree, intrabc)).sum();
            assert!(count > 0, "{chroma:?} {bit_depth:?}");
        },
    );
    each_case(CHROMAS, |chroma| {
        for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
            let (units, recon) = stream((2, chroma, bit_depth));
            let name = format!("{chroma:?} {bit_depth:?}");
            assert_same_samples(&decode(&units), &recon, &name);
        }
    });
    let case = |threads| (threads, PixelFormat::Yuv420, BitDepth::Ten);
    let (one, _) = stream(case(1));
    each_case([2], |threads| {
        assert!(stream(case(threads)).0 == one, "{threads} threads");
    });
}

/// Integer-MV streams decode to the encoder recon on any thread count.
#[test]
fn integer_mv_frames_decode_to_the_encoder_recon_on_any_thread_count() {
    assert_tool_streams(
        preset_with(|p| p.integer_mv = true),
        preset_with(|p| p.integer_mv = false),
        |seq| seq.screen_content.unwrap().seq_force_screen_content_tools == 2,
    );
}

/// An inter frame with `force_integer_mv` codes inter leaves whose vectors
/// are all whole samples.
#[test]
fn integer_mv_frames_code_whole_sample_vectors() {
    let config = config(64, 40, BitDepth::Ten, 9, 90);
    let preset = preset_with(|p| p.integer_mv = true);
    let mut coder = IntraFrameCoder::with_preset(&config, preset).unwrap();
    let key = crate::gop::plan(0, config.keyint);
    coder
        .encode_frame(&panning_frame(&config, 0), &key, 90)
        .unwrap();
    let plan = crate::gop::plan(1, config.keyint);
    let trees = search_frame(&coder, &plan, &panning_frame(&config, 1), config.bit_depth);
    let inter = |choice: &LeafChoice| choice.inter.is_some();
    assert!(trees.iter().map(|tree| leaves(tree, inter)).sum::<usize>() > 0);
    let fractional = |choice: &LeafChoice| match choice.inter {
        Some(InterPrediction::Single(modes)) => modes.mv.row % 8 != 0 || modes.mv.col % 8 != 0,
        Some(InterPrediction::Compound(modes)) => modes
            .mvs
            .iter()
            .any(|mv| mv.row % 8 != 0 || mv.col % 8 != 0),
        _ => false,
    };
    assert_eq!(
        trees
            .iter()
            .map(|tree| leaves(tree, fractional))
            .sum::<usize>(),
        0
    );
}

/// AVM decodes the integer-MV key and inter frames to the encoder recon.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_integer_mv_frames_like_splot() {
    assert_avm_decodes(preset_with(|p| p.integer_mv = true), "integer mv");
}

/// AVM decodes the palette key and inter frames of drawn text to the
/// encoder recon.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_palette_frames_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    let preset = preset_with(|p| p.palette = true);
    for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
        let case = (2, PixelFormat::Yuv420, bit_depth);
        let (units, recon, _) = cfl_stream(case, preset, 0, screen_frame);
        let avm = avm_decode(&avmdec, &units, (136, 72), bit_depth, "palette");
        assert!(avm == recon, "palette {bit_depth:?}: AVM output differs");
    }
}

/// DIP streams decode to the encoder recon on any thread count.
#[test]
fn dip_frames_decode_to_the_encoder_recon_on_any_thread_count() {
    assert_tool_streams(
        preset_with(|p| p.dip = true),
        preset_with(|p| p.dip = false),
        |seq| seq.intra.unwrap().enable_dip,
    );
}

/// The key-frame search codes DIP leaves.
#[test]
fn search_picks_dip_leaves() {
    let config = config(136, 72, BitDepth::Ten, 9, 90);
    let coder = IntraFrameCoder::with_preset(&config, preset_with(|p| p.dip = true)).unwrap();
    let plan = crate::gop::plan(0, config.keyint);
    let trees = search_frame(
        &coder,
        &plan,
        &synthetic_frame(&config, 3),
        config.bit_depth,
    );
    let dip = |choice: &LeafChoice| choice.inter.is_none() && choice.modes.dip.is_some();
    let count: usize = trees.iter().map(|tree| leaves(tree, dip)).sum();
    assert!(count > 0);
}

/// AVM decodes the DIP key and inter frames to the encoder recon.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_dip_frames_like_splot() {
    assert_avm_decodes(preset_with(|p| p.dip = true), "dip");
}

/// The key-frame search codes CCTX pairs and MHCCP leaves.
#[test]
fn search_picks_cctx_pairs_and_mhccp_leaves() {
    let config = config(136, 72, BitDepth::Ten, 9, 90);
    let preset = preset_with(|p| (p.cctx, p.mhccp) = (true, true));
    let coder = IntraFrameCoder::with_preset(&config, preset).unwrap();
    let plan = crate::gop::plan(0, config.keyint);
    let source = wave_frame(&config);
    let (trees, [_, cctx]) = search_frame_counting(&coder, &plan, &source, config.bit_depth);
    let mhccp = |choice: &LeafChoice| {
        choice.inter.is_none()
            && matches!(
                choice.modes.uv_mode,
                splot_decode::encoder_syntax::ChromaMode::Mhccp { .. }
            )
    };
    let leaves: usize = trees.iter().map(|tree| leaves(tree, mhccp)).sum();
    assert!(cctx > 0 && leaves > 0, "cctx {cctx} mhccp {leaves}");
}

/// AVM decodes CCTX and MHCCP key and inter frames to the encoder recon.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_cctx_and_mhccp_frames_like_splot() {
    assert_avm_decodes(
        preset_with(|p| (p.cctx, p.mhccp) = (true, true)),
        "cctx-mhccp",
    );
}

/// An oblique sine wave: directional residual that IST compacts.
fn wave_frame(config: &Config) -> SourceFrame {
    let max = f64::from(config.bit_depth.max_sample());
    let sizes = config.plane_sizes().unwrap();
    let planes = core::array::from_fn(|plane| {
        let Some(&size) = sizes.get(plane) else {
            return Vec::new();
        };
        (0..size.height())
            .flat_map(|y| (0..size.width()).map(move |x| (x as f64, y as f64)))
            .map(|(x, y)| (max * (0.5 + 0.3 * ((x * 0.8 + y * 0.6) / 7.0).sin())) as u16)
            .collect()
    });
    SourceFrame::new(config, 0, planes).unwrap()
}

/// The key-frame search without SDP and without the early exit, whose
/// shared tree splits this frame finer, codes luma blocks with a secondary
/// transform.
#[test]
fn search_picks_ist_blocks() {
    let config = config(64, 64, BitDepth::Eight, 9, 90);
    let shared = preset_with(|p| {
        (p.ist_sets, p.sdp) = (splot_decode::encoder_syntax::IST_SETS, false);
        p.partition_early_exit = crate::preset::Levels::all(false);
    });
    let coder = IntraFrameCoder::with_preset(&config, shared).unwrap();
    let plan = crate::gop::plan(0, config.keyint);
    let (_, [ist, _]) =
        search_frame_counting(&coder, &plan, &wave_frame(&config), config.bit_depth);
    assert!(ist > 0);
}

/// AVM decodes the IST key and inter frames to the encoder recon.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_ist_frames_like_splot() {
    assert_avm_decodes(ist_preset(), "ist");
}

/// The AC-energy loss of a flat recon of noise is about its squared error at
/// 100% on 8x8 and on 4x4 Hadamard tiles alike, scales with `psy_rd`, and is
/// zero for an exact recon.
#[test]
fn ac_energy_loss_charges_lost_texture() {
    use crate::intra_frame::ac_energy_loss;
    for (w, h) in [(16, 8), (4, 32), (32, 4)] {
        let dims = crate::kernels::BlockDims::new(w, h).unwrap();
        let mut state = 0x2545_f491_u32;
        let source: Vec<u16> = (0..w * h)
            .map(|i| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                (300 + i % 7) as u16 + (state % 41) as u16
            })
            .collect();
        let mean = (source.iter().map(|&s| u32::from(s)).sum::<u32>() / (w * h) as u32) as u16;
        let flat = vec![mean; w * h];
        let sse: u64 = source
            .iter()
            .map(|&s| u64::from(s.abs_diff(mean)).pow(2))
            .sum();
        let full = ac_energy_loss((&source, w), &flat, dims, 100);
        assert!(
            full > sse * 2 / 3 && full < sse * 3 / 2,
            "{w}x{h}: {full} vs {sse}"
        );
        assert_eq!(ac_energy_loss((&source, w), &flat, dims, 50), full / 2);
        assert_eq!(ac_energy_loss((&source, w), &source, dims, 100), 0);
        assert_eq!(ac_energy_loss((&source, w), &flat, dims, 0), 0);
    }
}

/// `Tune::Vq` streams (`ssim_aq` and `psy_rd`) decode to the encoder recon
/// with the same bytes on 1, 2 and 8 threads; `psy_rd` changes the stream.
#[test]
fn vq_tuned_frames_decode_to_the_encoder_recon_on_any_thread_count() {
    let vq = preset_with(|p| (p.ssim_aq, p.psy_rd) = (30, 50));
    let case = (1, PixelFormat::Yuv420, BitDepth::Ten);
    let (units, recon, _) = tool_stream(case, vq);
    assert_same_samples(&decode(&units.concat()), &recon, "vq");
    {
        let threads = 2;
        assert!(
            tool_stream((threads, case.1, case.2), vq).0 == units,
            "{threads}"
        );
    }
    let aq_only = preset_with(|p| p.ssim_aq = 30);
    assert_ne!(tool_stream(case, aq_only).0, units);
}

/// AVM decodes the `Tune::Vq` key and inter frames to the encoder recon.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_vq_tuned_frames_like_splot() {
    assert_avm_decodes(preset_with(|p| (p.ssim_aq, p.psy_rd) = (30, 50)), "vq");
}

/// The units, the encoder recon and the frame headers of a key frame and
/// three inter frames of panning content at preset 6 and qp 220 in two
/// tiles, with
/// `fast_decode`, on `threads` threads.
fn fast_decode_stream(
    fast_decode: bool,
    threads: usize,
) -> (Vec<Vec<u8>>, Vec<u8>, SequenceHeader, Vec<FrameHeaderCore>) {
    let mut config = config(136, 72, BitDepth::Ten, 6, 220);
    config.fast_decode = fast_decode;
    config.tiles = TileLayout::Uniform {
        cols_log2: 1,
        rows_log2: 0,
    };
    config.threads = splot_parallel::ThreadCount::from(threads);
    let mut coder = IntraFrameCoder::new(&config).unwrap();
    let (mut units, mut recon, mut cores) = (Vec::new(), Vec::new(), Vec::new());
    for index in 0..4 {
        let plan = crate::gop::plan(u64::from(index), config.keyint);
        let mut coded = coder
            .code_frame(&panning_frame(&config, index), &plan, 220, &[])
            .unwrap();
        cores.push(coded.reference.as_ref().unwrap().0.clone());
        coder.commit(&mut coded).unwrap();
        recon.extend(recon_bytes(&coded, config.bit_depth));
        units.push(coded.temporal_unit);
    }
    (units, recon, coder.sequence.clone(), cores)
}

/// Fast decode codes 64x64 superblocks, no GDF, no Wiener-NS unit and at
/// most two CDEF strength sets; its key frame and the inter frames that
/// reference it decode to the encoder recon with deblocking, CDEF, CCSO and
/// PC-Wiener on, and
/// 1, 2 and 8 threads write the same bytes. Off codes another stream.
#[test]
fn fast_decode_streams_decode_to_the_encoder_recon_on_any_thread_count() {
    use splot_core::headers::frame::FrameRestorationType;
    let (units, recon, seq, cores) = fast_decode_stream(true, 1);
    let partition = seq.partition.unwrap();
    assert!(!partition.use_256x256_superblock && !partition.use_128x128_superblock);
    assert!(!seq.filter.unwrap().enable_gdf);
    let cdef = |core: &FrameHeaderCore| core.cdef_params.as_ref().and_then(|c| c.cdef_strengths);
    let restoration = |core: &FrameHeaderCore| {
        core.lr_params.as_ref().map_or(Vec::new(), |lr| {
            lr.planes.iter().map(|p| p.restoration_type).collect()
        })
    };
    assert!(cores.iter().all(|core| cdef(core).unwrap_or(0) <= 2));
    assert!(cores.iter().any(|core| cdef(core).is_some()));
    assert!(cores.iter().any(|core| {
        core.deblocking_filter_params
            .as_ref()
            .is_some_and(|df| df.apply_deblocking_filter.iter().any(|&on| on))
    }));
    assert!(cores.iter().any(|core| {
        core.ccso_params
            .as_ref()
            .is_some_and(|ccso| ccso.ccso_frame_flag == Some(true))
    }));
    let types: Vec<_> = cores.iter().flat_map(restoration).collect();
    assert!(types.contains(&FrameRestorationType::PcWiener), "{types:?}");
    assert!(!types.iter().any(|t| matches!(
        t,
        FrameRestorationType::WienerNonsep | FrameRestorationType::Switchable
    )));
    assert_same_samples(&decode(&units.concat()), &recon, "fast decode");
    {
        let threads = 2;
        assert!(
            fast_decode_stream(true, threads).0 == units,
            "{threads} threads"
        );
    }
    let (off, ..) = fast_decode_stream(false, 1);
    assert_ne!(off, units);
}

/// AVM decodes the fast-decode key and inter frames to the encoder recon.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_fast_decode_frames_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    let (units, recon, ..) = fast_decode_stream(true, 2);
    let avm = avm_decode(&avmdec, &units, (136, 72), BitDepth::Ten, "fast-decode");
    assert!(avm == recon, "fast decode: AVM output differs");
}

/// A 512x128 frame of smooth luma with two 32x32 blocks of noise at
/// (288, 96) and (416, 96), in the lower-right tile of a 2x2 tile layout:
/// the second block and its 8-sample top and left margin are the first ones a quarter
/// of the sample range brighter, except their first 8x8 luma samples,
/// which are equal, so only a brightness-adapted copy matches.
fn brighter_copy(config: &Config, index: u32) -> SourceFrame {
    let max = f64::from(config.bit_depth.max_sample());
    let sizes = config.plane_sizes().unwrap();
    let planes = core::array::from_fn(|plane| {
        let Some(&size) = sizes.get(plane) else {
            return Vec::new();
        };
        let (w, h) = (size.width(), size.height());
        if plane > 0 {
            return vec![(max / 2.0) as u16 + 5 * plane as u16; w * h];
        }
        let sample = |x: usize, y: usize| {
            let v = (x * 7919 + y * 104_729 + index as usize * 31).wrapping_mul(2_654_435_761);
            let noise = f64::from((v >> 7) as u32 % 1000) / 1000.0;
            let (fx, fy) = (x as f64, y as f64);
            let smooth = 0.4 + 0.1 * (fx * 0.21 + fy * 0.13).sin();
            let block = (96..128).contains(&y) && (288..320).contains(&x);
            (max * if block { 0.2 + 0.4 * noise } else { smooth }).round() as u16
        };
        let mut samples: Vec<u16> = (0..w * h).map(|i| sample(i % w, i / w)).collect();
        let offset = (max / 4.0) as u16;
        for y in 88..128 {
            for x in 0..40 {
                let first = (96..104).contains(&y) && (8..16).contains(&x);
                let a = samples[y * w + 280 + x];
                samples[y * w + 408 + x] = if first { a } else { a + offset };
            }
        }
        samples
    });
    SourceFrame::new(config, u64::from(index), planes).unwrap()
}

/// Two key frames of [`brighter_copy`] in 2x2 tiles with local-area
/// IntraBC, `bawp`, `integer_mv` (for `allow_screen_content_tools`) and
/// `intrabc_morph` as `morph` says; `forced` makes IntraBC the only
/// candidate where it has one and keeps only the `morph_pred = 1` twin.
/// The units, the encoder recon and the count of final-pass leaves with
/// `morph_pred = 1`.
fn morph_stream(
    (morph, forced): (bool, bool),
    case: (usize, PixelFormat, BitDepth),
) -> (Vec<u8>, Vec<u8>, usize) {
    let mut preset = *small_intrabc_preset(false);
    (preset.bawp, preset.integer_mv, preset.intrabc_morph) = (true, true, morph);
    let preset = Box::leak(Box::new(preset));
    crate::mode_decision::FORCE_INTRABC.set(forced);
    crate::intrabc::FORCE_MORPH.set(forced);
    crate::tile_writer::MORPH_INTRABC.set(0);
    let (units, recon) = source_stream(case, preset, (512, brighter_copy, 1));
    crate::mode_decision::FORCE_INTRABC.set(false);
    crate::intrabc::FORCE_MORPH.set(false);
    (units, recon, crate::tile_writer::MORPH_INTRABC.get())
}

/// With `intrabc_morph` and only the morph twins kept (one thread), the
/// IntraBC leaves of [`brighter_copy`], which lie in a tile that does not
/// start at (0, 0), code `morph_pred = 1` (§ 7.13.3.26) in both bit depths
/// and decode to the encoder recon; without the field none do. The
/// searched streams decode to the encoder recon in every chroma format and
/// both bit depths, and 1, 2 and 8 threads write the same bytes.
#[test]
fn morph_intrabc_frames_decode_to_the_encoder_recon() {
    for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
        let case = (1, PixelFormat::Yuv420, bit_depth);
        assert_eq!(morph_stream((false, true), case).2, 0, "{bit_depth:?}");
        let (units, recon, morph) = morph_stream((true, true), case);
        assert!(morph > 0, "{bit_depth:?}");
        assert_same_samples(&decode(&units), &recon, "forced morph");
    }
    each_case(
        [
            (PixelFormat::Yuv420, BitDepth::Eight),
            (PixelFormat::Yuv422, BitDepth::Ten),
            (PixelFormat::Yuv444, BitDepth::Eight),
            (PixelFormat::Monochrome, BitDepth::Ten),
        ],
        |(chroma, bit_depth)| {
            let (units, recon, _) = morph_stream((true, false), (2, chroma, bit_depth));
            let name = format!("{chroma:?} {bit_depth:?}");
            assert_same_samples(&decode(&units), &recon, &name);
        },
    );
    let case = |threads| (threads, PixelFormat::Yuv420, BitDepth::Ten);
    let (one, ..) = morph_stream((true, false), case(1));
    each_case([2], |threads| {
        let (bytes, ..) = morph_stream((true, false), case(threads));
        assert!(bytes == one, "{threads} threads");
    });
}

/// AVM decodes key frames with forced morph twins ([`morph_stream`]) to
/// the encoder recon.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_morph_intrabc_frames_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for (chroma, bit_depth) in [
        (PixelFormat::Yuv420, BitDepth::Eight),
        (PixelFormat::Yuv444, BitDepth::Ten),
    ] {
        let (units, recon, morph) = morph_stream((true, true), (1, chroma, bit_depth));
        assert!(morph > 0, "{chroma:?} {bit_depth:?}");
        let name = format!("morph-intrabc-{chroma:?}");
        let avm = avm_decode(&avmdec, &[units], (512, 128), bit_depth, &name);
        assert!(avm == recon, "{chroma:?} {bit_depth:?}: AVM output differs");
    }
}

// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use super::*;
use crate::config::TileLayout;
use crate::preset::{Preset, PresetError};

/// Emits the frame index and qp as the frame's OBUs (after `0xd0` when
/// the frame opens a unit) and a recon that is the source plus one; fails
/// on the frame index it was told to.
struct FakeCoder {
    fail_at: Option<u64>,
}

impl FrameCoder for FakeCoder {
    fn encode_wave(&self, wave: &[(&SourceFrame, GopFrame, u16)]) -> Result<Vec<CodedFrame>> {
        wave.iter()
            .map(|&(source, plan, qp)| self.encode_frame(source, &plan, qp))
            .collect()
    }

    fn commit(&mut self, _frame: &mut CodedFrame) -> Result<()> {
        Ok(())
    }

    fn show_existing_frame(&mut self, slot: u32, order_hint: Option<u32>) -> Result<Vec<u8>> {
        let mut unit = vec![0xd0, 0x5e, slot as u8];
        unit.extend(order_hint.map(|hint| hint as u8));
        Ok(unit)
    }
}

impl FakeCoder {
    fn encode_frame(&self, source: &SourceFrame, plan: &GopFrame, qp: u16) -> Result<CodedFrame> {
        if self.fail_at == Some(plan.frame_index) {
            return Err(Error::Unsupported { what: "test" });
        }
        let recon = source
            .planes()
            .clone()
            .map(|plane| plane.into_iter().map(|s| s + 1).collect());
        let mut temporal_unit = if plan.opens_temporal_unit {
            vec![0xd0]
        } else {
            Vec::new()
        };
        temporal_unit.extend([plan.frame_index as u8, qp as u8]);
        Ok(CodedFrame {
            temporal_unit,
            recon,
            frame_type: plan.frame_type,
            reference: None,
            tip_output: None,
            warp: None,
            intra_cells: None,
            global_leaves: 0,
        })
    }
}

fn encoder(fail_at: Option<u64>, preset: u8) -> Result<Encoder> {
    let mut config = Config::new(4, 4);
    config.keyint = core::num::NonZeroU32::MIN.saturating_add(1);
    config.rate_control = RateControlMode::ConstantQp { qp: 40 };
    config.preset = Preset::new(preset).unwrap_or_default();
    Encoder::with_coder(config, Box::new(FakeCoder { fail_at }))
}

fn frame(encoder: &Encoder, pts: u64) -> Result<SourceFrame> {
    SourceFrame::new(
        encoder.config(),
        pts,
        [vec![10; 16], vec![20; 4], vec![30; 4]],
    )
}

fn drain(encoder: &mut Encoder) -> Result<Vec<Packet>> {
    let mut packets = Vec::new();
    while let Some(packet) = encoder.receive_packet()? {
        packets.push(packet);
    }
    Ok(packets)
}

#[test]
fn encodes_flat_frames_through_the_public_api() -> Result<()> {
    assert!(matches!(
        Encoder::new(Config::new(0, 64)),
        Err(Error::InvalidDimensions { .. })
    ));
    let config = Config::new(40, 24);
    let mut encoder = Encoder::new(config.clone())?;
    for pts in 0..2 {
        let planes = [vec![90; 40 * 24], vec![128; 20 * 12], vec![160; 20 * 12]];
        encoder.send_frame(SourceFrame::new(&config, pts, planes)?)?;
    }
    encoder.flush()?;
    let packets = drain(&mut encoder)?;
    assert_eq!(packets.len(), 2);
    for (pts, packet) in packets.iter().enumerate() {
        assert_eq!(packet.pts, pts as u64);
        assert!(packet.stats[0].psnr_y > 40.0, "{:?}", packet.stats);
    }
    let types: Vec<_> = packets.iter().map(|packet| packet.frame_type).collect();
    assert_eq!(types, [FrameType::Key, FrameType::Inter]);
    Ok(())
}

/// The low-delay overrides: every frame is coded and shown as soon as it
/// is sent, in display order.
#[test]
fn mini_gop_and_lookahead_of_one_emit_each_frame_at_once() -> Result<()> {
    let mut config = encoder(None, 6)?.config().clone();
    config.keyint = Config::DEFAULT_KEYINT;
    config.preset_overrides = vec![
        ("mini_gop_size".into(), "1".into()),
        ("bitrate_mini_gop_size".into(), "1".into()),
        ("lookahead_frames".into(), "1".into()),
    ];
    let mut encoder = Encoder::with_coder(config, Box::new(FakeCoder { fail_at: None }))?;
    for pts in 0..6 {
        encoder.send_frame(frame(&encoder, pts)?)?;
        let packet = encoder.receive_packet()?.ok_or(Error::Failed)?;
        assert_eq!((packet.pts, packet.data[1]), (pts, pts as u8));
        assert!(encoder.receive_packet()?.is_none());
    }
    Ok(())
}

/// A resized stream takes input-size frames, codes them at 8/16 of the
/// size with every loop filter, and decodes to the coded size; the stats
/// compare the recon with the resized source.
#[test]
fn resized_streams_code_input_frames_at_the_scaled_size() -> Result<()> {
    let mut config = Config::new(67, 48);
    config.resize_denominator = Some(16);
    config.keyint = core::num::NonZeroU32::MIN.saturating_add(1);
    let coded = config.coded();
    assert_eq!((coded.width, coded.height), (34, 24));
    let mut encoder = Encoder::new(config.clone())?;
    let small = SourceFrame::new(
        &coded,
        0,
        [vec![0; 34 * 24], vec![0; 17 * 12], vec![0; 17 * 12]],
    )?;
    assert!(matches!(
        encoder.send_frame(small),
        Err(Error::FrameConfigMismatch)
    ));
    let sizes = config.plane_sizes()?;
    for pts in 0..3u16 {
        let planes = core::array::from_fn(|plane| {
            let (w, h) = (sizes[plane].width(), sizes[plane].height());
            (0..w * h)
                .map(|i| (60 + i % w + (i / w) * 2 + usize::from(pts) * 5) as u16)
                .collect()
        });
        encoder.send_frame(SourceFrame::new(&config, u64::from(pts), planes)?)?;
    }
    encoder.flush()?;
    let packets = drain(&mut encoder)?;
    let stream: Vec<u8> = packets.iter().flat_map(|p| p.data.clone()).collect();
    assert_eq!(
        crate::test_support::decode(&stream).len(),
        3 * (34 * 24 + 2 * 17 * 12)
    );
    assert!(
        packets.iter().all(|p| p.stats[0].psnr_y > 35.0),
        "{packets:?}"
    );
    Ok(())
}

/// The stream of three flat 40x24 frames at preset 6 with `overrides`.
fn stream_with(overrides: &[(&str, &str)]) -> Result<Vec<u8>> {
    let mut config = Config::new(40, 24);
    config.preset_overrides = overrides
        .iter()
        .map(|&(field, value)| (field.into(), value.into()))
        .collect();
    let mut encoder = Encoder::new(config.clone())?;
    for pts in 0..3 {
        let planes = [
            vec![90 + pts as u16; 40 * 24],
            vec![128; 20 * 12],
            vec![160; 20 * 12],
        ];
        encoder.send_frame(SourceFrame::new(&config, pts, planes)?)?;
    }
    encoder.flush()?;
    Ok(drain(&mut encoder)?
        .into_iter()
        .flat_map(|packet| packet.data)
        .collect())
}

#[test]
fn preset_overrides_change_the_stream() -> Result<()> {
    let plain = stream_with(&[])?;
    assert_eq!(
        stream_with(&[("max_block_log2", "7"), ("mini_gop_size", "4")])?,
        plain
    );
    assert_ne!(stream_with(&[("max_block_log2", "6")])?, plain);
    let ippp = [("mini_gop_size", "1"), ("bitrate_mini_gop_size", "1")];
    assert_ne!(stream_with(&ippp)?, plain);
    assert!(matches!(
        stream_with(&[("mini_gop", "1")]),
        Err(Error::Preset(PresetError::UnknownField { .. }))
    ));
    assert!(matches!(
        stream_with(&[("mini_gop_size", "x")]),
        Err(Error::Preset(PresetError::InvalidValue { .. }))
    ));
    Ok(())
}

#[test]
fn codes_frames_in_order_with_stats() -> Result<()> {
    let mut encoder = encoder(None, Preset::MAX)?;
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink_seen = Arc::clone(&seen);
    encoder.set_stats_sink(Box::new(move |stats| {
        if let Ok(mut seen) = sink_seen.lock() {
            seen.push(stats.frame_index);
        }
    }));
    assert_eq!(encoder.receive_packet()?, None);
    for pts in [5, 6, 7] {
        let frame = frame(&encoder, pts)?;
        encoder.send_frame(frame)?;
    }
    encoder.flush()?;
    let packets = drain(&mut encoder)?;
    let types: Vec<_> = packets.iter().map(|p| (p.pts, p.frame_type)).collect();
    assert_eq!(
        types,
        [
            (5, FrameType::Key),
            (6, FrameType::Inter),
            (7, FrameType::Key)
        ]
    );
    let stats = packets[1].stats[0];
    assert_eq!((stats.frame_index, stats.pts), (1, 6));
    assert_eq!((stats.qp, stats.bits), (40, 24));
    assert!((stats.psnr_y - 48.130_803_608).abs() < 1e-6);
    assert_eq!(packets[2].data, [0xd0, 2, 40]);
    assert_eq!(seen.lock().map(|s| s.clone()).ok(), Some(vec![0, 1, 2]));
    Ok(())
}

/// A mini-GOP of 4 waits for its frames and packs hidden frames with the
/// next output: coding order 4 2 1 [2] 3 [4], with 4 and 2 hidden in slots 1 and 2.
#[test]
fn packs_hidden_frames_into_the_next_output_unit() -> Result<()> {
    let mut encoder = encoder(None, 6)?;
    encoder.config.keyint = core::num::NonZeroU32::new(10).unwrap_or(encoder.config.keyint);
    encoder.planner = GopPlanner::new(encoder.config.keyint, 4);
    encoder.lookahead_frames = 4;
    for pts in 0..4 {
        let frame = frame(&encoder, pts)?;
        encoder.send_frame(frame)?;
    }
    let key = drain(&mut encoder)?;
    assert_eq!(key.len(), 1, "the mini-GOP after the key frame waits");
    let frame = frame(&encoder, 4)?;
    encoder.send_frame(frame)?;
    let packets = drain(&mut encoder)?;
    let data: Vec<_> = packets.iter().map(|p| p.data.clone()).collect();
    assert_eq!(
        data,
        [
            vec![0xd0, 1, 40, 2, 40, 3, 40],
            vec![0xd0, 0x5e, 2],
            vec![0xd0, 4, 40],
            vec![0xd0, 0x5e, 1],
        ]
    );
    let pts: Vec<_> = packets.iter().map(|p| p.pts).collect();
    assert_eq!(pts, [1, 2, 3, 4]);
    let stats: Vec<Vec<u64>> = packets
        .iter()
        .map(|p| p.stats.iter().map(|s| s.pts).collect())
        .collect();
    assert_eq!(stats, [vec![4, 2, 1], vec![], vec![3], vec![]]);
    Ok(())
}

/// A call codes the waves its packet needs and no more: of a key frame
/// and a mini-GOP of 8, the last wave (frames 5 and 7) waits for the call
/// after the units of frames 1 to 4.
#[test]
fn a_call_codes_only_the_waves_of_its_packet() -> Result<()> {
    let mut config = encoder(None, 6)?.config().clone();
    config.keyint = Config::DEFAULT_KEYINT;
    let mut encoder = Encoder::with_coder(config, Box::new(FakeCoder { fail_at: None }))?;
    let progress = encoder.progress();
    for pts in 0..9 {
        encoder.send_frame(frame(&encoder, pts)?)?;
    }
    encoder.flush()?;
    let mut coded = Vec::new();
    while let Some(packet) = encoder.receive_packet()? {
        coded.push((packet.pts, progress.frames() as u64));
    }
    let expected = [0, 1, 2, 3, 4, 5, 6, 7, 8].map(|pts| match pts {
        0 => (0, 1),
        1..=4 => (pts, 7),
        _ => (pts, 9),
    });
    assert_eq!(coded, expected);
    Ok(())
}

/// The superblocks of a frame add up to one frame before its commit, and
/// the count is whole after it.
#[test]
fn progress_counts_the_superblocks_of_a_frame() -> Result<()> {
    let mut config = Config::new(320, 192);
    config.preset = Preset::new(Preset::MAX)?;
    let mut encoder = Encoder::new(config.clone())?;
    let progress = encoder.progress();
    let at_commit = Arc::new(Mutex::new(Vec::new()));
    let (seen, sink_progress) = (Arc::clone(&at_commit), progress.clone());
    encoder.set_stats_sink(Box::new(move |_| {
        if let Ok(mut seen) = seen.lock() {
            seen.push(sink_progress.frames());
        }
    }));
    let planes = [
        vec![90; 320 * 192],
        vec![128; 160 * 96],
        vec![160; 160 * 96],
    ];
    encoder.send_frame(SourceFrame::new(&config, 0, planes)?)?;
    encoder.flush()?;
    assert_eq!(progress.frames().to_bits(), 0f64.to_bits());
    assert_eq!(drain(&mut encoder)?.len(), 1);
    let at_commit = at_commit.lock().map(|seen| seen.clone()).ok();
    let [searched] = at_commit.as_deref().unwrap_or(&[]) else {
        return Err(Error::Failed);
    };
    assert!((0.999..1.0).contains(searched), "{searched}");
    assert_eq!(progress.frames().to_bits(), 1f64.to_bits());
    Ok(())
}

/// Codes `frames` frames of a 64x64 texture panning by `shift` samples a
/// frame, with a new texture from frame `cut`, through the fake coder.
fn textured(rate_control: RateControlMode, shift: usize, cut: u64) -> Result<Vec<Packet>> {
    let mut config = Config::new(64, 64);
    config.rate_control = rate_control;
    let mut encoder = Encoder::with_coder(config.clone(), Box::new(FakeCoder { fail_at: None }))?;
    for pts in 0..12 {
        let scene = u32::from(pts >= cut);
        let frame = crate::lookahead::tests::frame(&config, pts, shift * pts as usize, scene);
        encoder.send_frame(frame)?;
    }
    encoder.flush()?;
    drain(&mut encoder)
}

#[test]
fn a_scene_cut_starts_a_key_frame() -> Result<()> {
    let packets = textured(RateControlMode::ConstantQp { qp: 40 }, 2, 9)?;
    let keys: Vec<u64> = packets
        .iter()
        .filter(|packet| packet.frame_type == FrameType::Key)
        .map(|packet| packet.pts)
        .collect();
    assert_eq!(keys, [0, 9]);
    Ok(())
}

/// Fresh noise in every frame predicts badly from the frame before, like
/// a cut, but no worse than the frame before did: only frame 0 is a key.
#[test]
fn noise_makes_no_scene_cuts() -> Result<()> {
    let config = Config::new(64, 64);
    let mut encoder = Encoder::with_coder(config.clone(), Box::new(FakeCoder { fail_at: None }))?;
    let mut random = xorshift(5);
    for pts in 0..12 {
        let sizes = config.plane_sizes()?;
        let planes = core::array::from_fn(|plane| {
            (0..sizes[plane].width() * sizes[plane].height())
                .map(|_| 16 + 37 * random() as u16)
                .collect()
        });
        encoder.send_frame(SourceFrame::new(&config, pts, planes)?)?;
    }
    encoder.flush()?;
    let keys = drain(&mut encoder)?
        .iter()
        .filter(|packet| packet.frame_type == FrameType::Key)
        .count();
    assert_eq!(keys, 1);
    Ok(())
}

/// On static content the alt-ref of each mini-GOP of 8 gets a finer
/// quantizer than its middle frame, that one than the quarter frames, and
/// those than the leaves.
#[test]
fn crf_gives_referenced_frames_lower_qindex_on_static_content() -> Result<()> {
    let packets = textured(RateControlMode::Crf { crf: 30 }, 0, u64::MAX)?;
    let qp: BTreeMap<u64, u16> = packets
        .iter()
        .flat_map(|packet| &packet.stats)
        .map(|stats| (stats.pts, stats.qp))
        .collect();
    let layers = [8, 4, 2, 1].map(|display| qp[&display]);
    assert!(layers.is_sorted_by(|a, b| a < b), "{qp:?}");
    assert_eq!(qp[&1], qp[&3], "{qp:?}");
    assert!(qp[&0] < qp[&2], "{qp:?}");
    Ok(())
}

/// The motion search of a window reads the last coded frame, the past
/// reference of the window's first coded frame. A window of six frames
/// over mini-GOPs of four plans frame 11 against frame 8 only after 8 is
/// coded; on a pan frame 11 then costs far less than intra.
#[test]
fn a_window_searches_against_the_last_coded_frame() -> Result<()> {
    let mut config = Config::new(64, 64);
    config.preset_overrides = [("lookahead_frames", "6"), ("mini_gop_size", "4")]
        .map(|(field, value)| (field.to_owned(), value.to_owned()))
        .into();
    let mut encoder = Encoder::with_coder(config.clone(), Box::new(FakeCoder { fail_at: None }))?;
    for pts in 0..12 {
        encoder.send_frame(crate::lookahead::tests::frame(
            &config,
            pts,
            2 * pts as usize,
            0,
        ))?;
    }
    encoder.flush()?;
    drain(&mut encoder)?;
    let last = encoder
        .window
        .iter()
        .max_by_key(|info| info.display_index)
        .copied();
    let last = last.ok_or(Error::Failed)?;
    assert_eq!(
        (last.display_index, last.frame_type),
        (11, FrameType::Inter)
    );
    let intra = encoder.lookahead.intra(11).ok_or(Error::Failed)?;
    let intra: u64 = intra.iter().map(|&cost| u64::from(cost.max(128))).sum();
    assert!(
        last.complexity * 2 < intra,
        "{} vs intra {intra}",
        last.complexity
    );
    Ok(())
}

/// Encodes 20 panning frames `width` wide in `tiles` with a cut at 11,
/// sending them `sends` at a time and then receiving up to `receives`
/// packets, until the input ends.
fn interleaved(
    (rate_control, width, tiles): (RateControlMode, u32, TileLayout),
    mut sends: impl FnMut() -> usize,
    mut receives: impl FnMut() -> usize,
) -> Result<Vec<u8>> {
    let mut config = Config::new(width, 16);
    config.rate_control = rate_control;
    config.tiles = tiles;
    config.preset = Preset::new(9).unwrap_or_default();
    let mut encoder = Encoder::new(config.clone())?;
    let mut bytes = Vec::new();
    let mut next = 0;
    while next < 20 {
        for _ in 0..sends() {
            if next < 20 {
                let scene = u32::from(next >= 11);
                let frame = crate::lookahead::tests::frame(&config, next, 2 * next as usize, scene);
                encoder.send_frame(frame)?;
                next += 1;
            }
        }
        for _ in 0..receives() {
            let Some(packet) = encoder.receive_packet()? else {
                break;
            };
            bytes.extend(packet.data);
        }
    }
    encoder.flush()?;
    for packet in drain(&mut encoder)? {
        bytes.extend(packet.data);
    }
    assert_eq!(encoder.progress().frames().to_bits(), 20f64.to_bits());
    Ok(bytes)
}

/// Pseudo-random counts in `0..6`.
fn xorshift(mut state: u32) -> impl FnMut() -> usize {
    move || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        (state % 6) as usize
    }
}

/// One send and a full drain per frame, every send before the flush, and
/// random interleavings all give the same bytes; the VBR case codes two
/// tiles in parallel.
#[test]
fn output_does_not_depend_on_the_call_pattern() -> Result<()> {
    let one = TileLayout::default();
    let two = TileLayout::Uniform {
        cols_log2: 1,
        rows_log2: 0,
    };
    let cbr = RateControlMode::Cbr {
        bitrate_kbps: 60,
        buffer_ms: 1000,
        initial_ms: 500,
    };
    for mode in [
        (RateControlMode::Crf { crf: 30 }, 48, one.clone()),
        (RateControlMode::Vbr { bitrate_kbps: 90 }, 72, two),
        (cbr, 48, one),
    ] {
        let per_frame = interleaved(mode.clone(), || 1, || usize::MAX)?;
        assert!(!per_frame.is_empty());
        assert!(
            per_frame == interleaved(mode.clone(), || 20, || 0)?,
            "{mode:?}"
        );
        for seed in [1, 7, 12_345] {
            let random = interleaved(mode.clone(), xorshift(seed), xorshift(seed + 99))?;
            assert!(per_frame == random, "{mode:?} seed {seed}");
        }
    }
    Ok(())
}

/// Asks for a qindex above `MaxQ` and records the show-existing bits.
struct Recorder(Arc<Mutex<Vec<u64>>>);

impl RateController for Recorder {
    fn frame_qp(&mut self, _info: &FramePlanInfo) -> u16 {
        u16::MAX
    }

    fn update(&mut self, _stats: &FrameStats) {}

    fn update_show_existing(&mut self, bits: u64) {
        if let Ok(mut seen) = self.0.lock() {
            seen.push(bits);
        }
    }
}

/// A caller's rate controller gets every show-existing unit's bits, and
/// its qindex is clamped to `MaxQ`.
#[test]
fn custom_rate_controller_sees_show_existing_bits() -> Result<()> {
    let mut config = Config::new(16, 16);
    config.preset = Preset::new(6)?;
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut encoder =
        Encoder::with_rate_controller(config.clone(), Box::new(Recorder(Arc::clone(&seen))))?;
    for pts in 0..5 {
        let planes = [vec![90; 256], vec![128; 64], vec![160; 64]];
        encoder.send_frame(SourceFrame::new(&config, pts, planes)?)?;
    }
    encoder.flush()?;
    let packets = drain(&mut encoder)?;
    let shown: Vec<u64> = packets
        .iter()
        .filter(|packet| packet.stats.is_empty())
        .map(|packet| packet.data.len() as u64 * 8)
        .collect();
    assert_eq!(shown.len(), 2);
    assert_eq!(seen.lock().map(|seen| seen.clone()).ok(), Some(shown));
    assert!(packets.iter().flat_map(|p| &p.stats).all(|s| s.qp == 255));
    Ok(())
}

#[test]
fn state_machine_rejects_invalid_operations() -> Result<()> {
    let mut encoder = encoder(Some(0), Preset::MAX)?;
    let mut other = Config::new(8, 4);
    other.rate_control = RateControlMode::ConstantQp { qp: 40 };
    let wrong = SourceFrame::new(&other, 0, [vec![0; 32], vec![0; 8], vec![0; 8]])?;
    assert!(matches!(
        encoder.send_frame(wrong),
        Err(Error::FrameConfigMismatch)
    ));
    let first = frame(&encoder, 0)?;
    encoder.send_frame(first)?;
    encoder.flush()?;
    let late = frame(&encoder, 1)?;
    assert!(matches!(
        encoder.send_frame(late),
        Err(Error::SendAfterFlush)
    ));
    assert!(matches!(
        encoder.receive_packet(),
        Err(Error::Unsupported { .. })
    ));
    assert!(matches!(encoder.receive_packet(), Err(Error::Failed)));
    assert!(matches!(encoder.flush(), Err(Error::Failed)));
    Ok(())
}

// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use splot_recon::BitDepth;

use super::*;
use crate::config::{Config, RateControlMode};
use crate::encoder::{CodedFrame, Encoder, FrameCoder};
use crate::error::Result;
use crate::frame::SourceFrame;
use crate::gop::GopFrame;
use crate::intra_frame::IntraFrameCoder;
use crate::lookahead::tests::frame;
use crate::preset::Preset;
use crate::test_support::{
    assert_same_samples, avm_decode, avm_decoder, decode, each_case, raw_bytes,
};

fn plan(frame_index: u64, frame_type: FrameType, offset: i16) -> FramePlanInfo {
    FramePlanInfo {
        frame_index,
        display_index: frame_index,
        frame_type,
        layer: 0,
        complexity: 50_000,
        min_complexity: 1_000,
        qindex_offset: offset,
        bit_depth: BitDepth::Eight,
    }
}

#[test]
fn crf_adds_the_offset_and_clamps() {
    let mut crf = Crf { crf: 30 };
    assert_eq!(crf.frame_qp(&plan(0, FrameType::Inter, -20)), 100);
    assert_eq!(crf.frame_qp(&plan(0, FrameType::Inter, 200)), 255);
    assert_eq!(Crf { crf: 1 }.frame_qp(&plan(0, FrameType::Key, -20)), 1);
    let ten = FramePlanInfo {
        bit_depth: BitDepth::Ten,
        ..plan(0, FrameType::Inter, -20)
    };
    assert_eq!(crf.frame_qp(&ten), 148);
    assert_eq!(Crf { crf: Crf::MAX }.base_qindex(BitDepth::Ten), 300);
    assert_eq!(
        crf.frame_qp(&FramePlanInfo {
            qindex_offset: 200,
            ..ten
        }),
        303
    );
}

/// A coder whose frames cost `k * complexity / step` bits, with `k` far from
/// the initial model: the controllers meet the target within a few windows.
#[test]
fn bitrate_controllers_meet_a_simulated_target() {
    for cbr in [false, true] {
        let mut control = if cbr {
            Box::new(Cbr::new(200, (500, 250), (30, 1)).unwrap()) as Box<dyn RateController>
        } else {
            Box::new(Vbr::new(200, (30, 1)).unwrap())
        };
        let sizes = simulate(control.as_mut(), 10, |index| {
            let kind = if index % 20 == 0 {
                FrameType::Key
            } else {
                FrameType::Inter
            };
            plan(index, kind, -4 * (index % 4) as i16)
        });
        let (total, largest) = (sizes.iter().sum::<u64>(), sizes.iter().max());
        let target = 200_000 * 40 / 30;
        assert!(
            total.abs_diff(target) * 20 < target,
            "cbr {cbr}: {total} vs {target}"
        );
        if cbr {
            assert!(
                largest < Some(&(200 * 500 / 2 + 200_000 / 30)),
                "{largest:?}"
            );
        }
    }
}

/// Records the raw decoder output of every coded frame by display index.
struct Recording {
    coder: IntraFrameCoder,
    bit_depth: BitDepth,
    recons: Arc<Mutex<BTreeMap<u64, Vec<u8>>>>,
    codings: Arc<Mutex<usize>>,
}

impl FrameCoder for Recording {
    fn encode_wave(&self, wave: &[(&SourceFrame, GopFrame, u16)]) -> Result<Vec<CodedFrame>> {
        let coded = self.coder.encode_wave(wave)?;
        *self.codings.lock().unwrap() += coded.len();
        for ((_, plan, _), frame) in wave.iter().zip(&coded) {
            let recon = raw_bytes(frame.recon.iter().map(Vec::as_slice), self.bit_depth);
            self.recons
                .lock()
                .unwrap()
                .insert(plan.display_index, recon);
        }
        Ok(coded)
    }

    fn commit(&mut self, frame: &mut CodedFrame) -> Result<()> {
        self.coder.commit(frame)
    }

    fn show_existing_frame(&mut self, slot: u32) -> Result<Vec<u8>> {
        self.coder.show_existing_frame(slot)
    }
}

/// A coded stream: its temporal units, the encoder reconstruction in display
/// order, each packet's frame type, the stats of each coded frame, and the
/// codings past the first of a frame.
struct Stream {
    units: Vec<Vec<u8>>,
    expected: Vec<u8>,
    types: Vec<FrameType>,
    stats: Vec<FrameStats>,
    recodes: usize,
}

impl Stream {
    fn kbps(&self) -> u64 {
        let bits: u64 = self.units.iter().map(|unit| unit.len() as u64 * 8).sum();
        bits * 30 / self.units.len() as u64 / 1000
    }
}

const SIZE: (u32, u32) = (48, 32);
const FRAMES: u64 = 30;

/// Thirty frames of a texture panning by two samples a frame, with a new
/// texture from frame 17, coded at preset 9 (mini-GOPs of 2); the first frame
/// is black when `black_first` is set.
fn encode(rate_control: RateControlMode, black_first: bool) -> Stream {
    encode_overriding(rate_control, black_first, &[])
}

/// [`encode`] with the preset fields `overrides`.
fn encode_overriding(
    rate_control: RateControlMode,
    black_first: bool,
    overrides: &[(&str, &str)],
) -> Stream {
    let mut config = Config::new(SIZE.0, SIZE.1);
    config.preset = Preset::new(9).unwrap();
    config.rate_control = rate_control;
    config.preset_overrides = overrides
        .iter()
        .map(|&(name, value)| (name.to_owned(), value.to_owned()))
        .collect();
    let (recons, codings) = (Arc::default(), Arc::default());
    let coder = Recording {
        coder: IntraFrameCoder::new(&config).unwrap(),
        bit_depth: config.bit_depth,
        recons: Arc::clone(&recons),
        codings: Arc::clone(&codings),
    };
    let mut encoder = Encoder::with_coder(config.clone(), Box::new(coder)).unwrap();
    let mut packets = Vec::new();
    for index in 0..=FRAMES {
        if index == FRAMES {
            encoder.flush().unwrap();
        } else {
            let scene = u32::from(index >= 17);
            let mut source = frame(&config, index, 2 * index as usize, scene);
            if black_first && index == 0 {
                let planes = source.planes().clone().map(|plane| vec![16; plane.len()]);
                source = SourceFrame::new(&config, 0, planes).unwrap();
            }
            encoder.send_frame(source).unwrap();
        }
        while let Some(packet) = encoder.receive_packet().unwrap() {
            packets.push(packet);
        }
    }
    let expected = recons.lock().unwrap().values().flatten().copied().collect();
    let stats: Vec<FrameStats> = packets.iter().flat_map(|p| p.stats.clone()).collect();
    Stream {
        types: packets.iter().map(|packet| packet.frame_type).collect(),
        recodes: *codings.lock().unwrap() - stats.len(),
        stats,
        units: packets.into_iter().map(|packet| packet.data).collect(),
        expected,
    }
}

/// `key_qindex_boost` lowers the key frames' qindex in CRF mode by its
/// cell and leaves the other frames and constant-qp streams as they are.
#[test]
fn key_qindex_boost_lowers_only_key_frames_in_crf() {
    let boost = [("key_qindex_boost", "6,0,0,0")];
    let crf = RateControlMode::Crf { crf: 30 };
    let (plain, boosted) = (encode(crf, false), encode_overriding(crf, false, &boost));
    let qps = |stream: &Stream| stream.stats.iter().map(|s| s.qp).collect::<Vec<_>>();
    let (plain_qps, boosted_qps) = (qps(&plain), qps(&boosted));
    assert_eq!(plain_qps.len(), boosted_qps.len());
    for (index, (&p, &b)) in plain_qps.iter().zip(&boosted_qps).enumerate() {
        let key = plain.types[index] == FrameType::Key;
        assert_eq!(
            b + if key { 6 } else { 0 },
            p,
            "frame {index}: {plain_qps:?} {boosted_qps:?}"
        );
    }
    assert_same_samples(
        &decode(&boosted.units.concat()),
        &boosted.expected,
        "boosted",
    );
    let flat = RateControlMode::ConstantQp { qp: 120 };
    assert_eq!(
        encode(flat, false).units,
        encode_overriding(flat, false, &boost).units
    );
}

fn cases() -> [RateControlMode; 3] {
    [
        RateControlMode::Crf { crf: 30 },
        RateControlMode::Vbr { bitrate_kbps: 60 },
        RateControlMode::Cbr {
            bitrate_kbps: 60,
            buffer_ms: 1000,
            initial_ms: 500,
        },
    ]
}

/// VBR and CBR land within 15% of the target, CRF spends fewer bits as the
/// rate factor rises, and every stream decodes to the encoder recon with a
/// key frame at the scene cut.
#[test]
fn rate_controlled_streams_meet_targets_and_decode_to_the_recon() {
    let mut crf_kbps = Vec::new();
    for mode in cases() {
        let stream = encode(mode, false);
        if matches!(mode, RateControlMode::Crf { .. }) {
            crf_kbps.push(stream.kbps());
        }
        assert_same_samples(
            &decode(&stream.units.concat()),
            &stream.expected,
            &format!("{mode:?}"),
        );
        let keys: Vec<usize> = (0..stream.types.len())
            .filter(|&i| stream.types[i] == FrameType::Key)
            .collect();
        assert_eq!(keys, [0, 17], "{mode:?}");
        let qps: std::collections::BTreeSet<u16> = stream.stats.iter().map(|s| s.qp).collect();
        assert!(qps.len() > 1, "{mode:?}: one qindex {qps:?}");
        if let RateControlMode::Vbr { bitrate_kbps } | RateControlMode::Cbr { bitrate_kbps, .. } =
            mode
        {
            let target = u64::from(bitrate_kbps);
            assert!(
                stream.kbps().abs_diff(target) * 100 <= target * 15,
                "{mode:?}: {} kbps",
                stream.kbps()
            );
        }
    }
    let kbps = [20, 40].map(|crf| encode(RateControlMode::Crf { crf }, false).kbps());
    assert!(
        kbps[0] > crf_kbps[0] && crf_kbps[0] > kbps[1],
        "{kbps:?} {crf_kbps:?}"
    );
}

/// CRF, VBR, capped CRF with a cap that codes frames again, and CBR with a
/// 250 ms buffer: streams of 24 panning frames in two tiles are the same
/// bytes at 1, 2 and 8 threads.
#[test]
fn rate_controlled_streams_do_not_depend_on_the_thread_count() {
    let small_cbr = RateControlMode::Cbr {
        bitrate_kbps: 150,
        buffer_ms: 250,
        initial_ms: 125,
    };
    let modes = [
        cases()[0],
        cases()[1],
        capped(30, 100, (500, 250)),
        small_cbr,
    ];
    let mut streams = Vec::new();
    for mode in &modes {
        let stream = |threads: usize| {
            let mut config = Config::new(144, 32);
            config.preset = Preset::new(9).unwrap();
            config.rate_control = *mode;
            config.tiles.cols_log2 = 1;
            config.threads = splot_parallel::ThreadCount::from(threads);
            let mut encoder = Encoder::new(config.clone()).unwrap();
            let mut bytes = Vec::new();
            for index in 0..24 {
                let source = frame(&config, index, 2 * index as usize, u32::from(index >= 17));
                encoder.send_frame(source).unwrap();
                while let Some(packet) = encoder.receive_packet().unwrap() {
                    bytes.extend(packet.data);
                }
            }
            encoder.flush().unwrap();
            while let Some(packet) = encoder.receive_packet().unwrap() {
                bytes.extend(packet.data);
            }
            bytes
        };
        let one = stream(1);
        assert!(!one.is_empty());
        for threads in [2, 8] {
            assert!(stream(threads) == one, "{mode:?} at {threads} threads");
        }
        streams.push(one);
    }
    assert!(streams[2] != streams[0], "the cap does not bind");
}

/// VBR meets three targets within 3% over 60 frames of a 96x64 pan, key
/// frames every 30.
#[test]
fn vbr_meets_targets_within_three_percent() {
    each_case([40, 100, 250], |target| {
        let mut config = Config::new(96, 64);
        config.preset = Preset::new(9).unwrap();
        config.rate_control = RateControlMode::Vbr {
            bitrate_kbps: target,
        };
        config.keyint = core::num::NonZeroU32::new(30).unwrap();
        let mut encoder = Encoder::new(config.clone()).unwrap();
        let mut bits = 0;
        for index in 0..=60u64 {
            if index == 60 {
                encoder.flush().unwrap();
            } else {
                encoder
                    .send_frame(frame(&config, index, 2 * index as usize, 0))
                    .unwrap();
            }
            while let Some(packet) = encoder.receive_packet().unwrap() {
                bits += packet.data.len() as u64 * 8;
            }
        }
        let budget = u64::from(target) * 2000;
        assert!(
            bits.abs_diff(budget) * 100 <= budget * 3,
            "{target}: {bits} bits"
        );
    });
}

/// AVM decodes the rate-controlled streams to the encoder recon.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_rate_controlled_streams_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    let small_cbr = RateControlMode::Cbr {
        bitrate_kbps: 60,
        buffer_ms: 250,
        initial_ms: 125,
    };
    let modes = cases()
        .into_iter()
        .chain([capped(20, 40, (500, 250)), small_cbr]);
    for (index, mode) in modes.enumerate() {
        let stream = encode(mode, false);
        let name = format!("rc{index}");
        let avm = avm_decode(&avmdec, &stream.units, SIZE, BitDepth::Eight, &name);
        assert!(avm == stream.expected, "{mode:?}: AVM output differs");
    }
}

fn stats(frame_index: u64, qp: u16, bits: u64) -> FrameStats {
    FrameStats {
        frame_index,
        pts: frame_index,
        qp,
        bits,
        psnr_y: 0.0,
        psnr_u: 0.0,
        psnr_v: 0.0,
        ssim_y: None,
    }
}

/// Codes `windows` windows of four frames, `frame(index)` each, through a
/// simulated coder whose key frames cost `120 * complexity / step` bits and
/// inter frames `20 * complexity / step`.
fn simulate(
    control: &mut dyn RateController,
    windows: u64,
    frame: impl Fn(u64) -> FramePlanInfo,
) -> Vec<u64> {
    let mut sizes = Vec::new();
    for window in 0..windows {
        let frames: Vec<FramePlanInfo> = (0..4).map(|i| frame(window * 4 + i)).collect();
        control.plan_window(&frames);
        for info in &frames {
            let qp = control.frame_qp(info);
            let k = if info.frame_type == FrameType::Key {
                120
            } else {
                20
            };
            let bits = k * info.complexity / step(qp, info.bit_depth);
            control.update(&stats(info.frame_index, qp, bits));
            sizes.push(bits);
        }
    }
    sizes
}

/// Ten seconds of static content underspend; the complex content after them
/// keeps CBR inside its buffer, and both modes spend at most their bounded
/// credit in the first second and the target, within 5%, after it.
#[test]
fn static_content_banks_no_unbounded_credit() {
    let frame = |index| FramePlanInfo {
        complexity: if index < 300 { 500 } else { 1_000_000 },
        ..plan(index, FrameType::Inter, 0)
    };
    let frame_bits = 200_000 / 30;
    let buffer = 200 * 1000;
    let cbr = simulate(
        &mut Cbr::new(200, (1000, 500), (30, 1)).unwrap(),
        100,
        frame,
    );
    let mut level = buffer / 2;
    for (index, &bits) in cbr.iter().enumerate() {
        level = (level + bits).saturating_sub(frame_bits);
        assert!(
            bits <= buffer && level <= buffer,
            "frame {index}: {bits} bits, level {level}"
        );
    }
    let vbr = simulate(&mut Vbr::new(200, (30, 1)).unwrap(), 100, frame);
    for (sizes, credit) in [(cbr, buffer / 2), (vbr, 8 * frame_bits)] {
        for (second, sizes) in sizes[300..].chunks(30).enumerate() {
            let bits: u64 = sizes.iter().sum();
            let target = frame_bits * sizes.len() as u64;
            let allowed = if second == 0 { credit } else { target / 20 };
            assert!(
                bits.abs_diff(target) <= allowed,
                "second {second}: {bits} vs {target}"
            );
        }
    }
}

/// Settings out of range are rejected, and extreme settings, complexities
/// and frame sizes neither overflow nor panic.
#[test]
fn extreme_inputs_do_not_overflow() {
    assert!(matches!(
        Vbr::new(MAX_BITRATE_KBPS + 1, (30, 1)),
        Err(Error::InvalidBitrate { .. })
    ));
    assert!(matches!(
        Cbr::new(100, (MAX_BUFFER_MS + 1, 0), (30, 1)),
        Err(Error::InvalidBitrate { .. })
    ));
    assert!(matches!(
        Vbr::new(100, (30, 0)),
        Err(Error::InvalidFrameRate { .. })
    ));
    let controllers: [Box<dyn RateController>; 4] = [
        Box::new(Vbr::new(MAX_BITRATE_KBPS, (1, u32::MAX)).unwrap()),
        Box::new(Vbr::new(1, (u32::MAX, 1)).unwrap()),
        Box::new(
            Cbr::new(
                MAX_BITRATE_KBPS,
                (MAX_BUFFER_MS, MAX_BUFFER_MS),
                (1, u32::MAX),
            )
            .unwrap(),
        ),
        Box::new(Cbr::new(1, (1, 0), (u32::MAX, 1)).unwrap()),
    ];
    for mut control in controllers {
        for complexity in [0, u64::MAX] {
            let window = [
                FramePlanInfo {
                    complexity,
                    ..plan(0, FrameType::Key, i16::MIN)
                },
                FramePlanInfo {
                    complexity,
                    ..plan(1, FrameType::Inter, i16::MAX)
                },
            ];
            control.plan_window(&window);
            for info in &window {
                let qp = control.frame_qp(info);
                assert!((1..=255).contains(&qp), "{qp}");
                for bits in [0, u64::MAX] {
                    control.update(&stats(info.frame_index, qp, bits));
                    control.update_show_existing(bits);
                }
            }
        }
    }
}

/// A black first frame tells the bitrate model nothing about the size of
/// later key frames: they get about the qindex they get without it.
#[test]
fn a_black_first_frame_keeps_key_frame_qindex_sane() {
    for mode in &cases()[1..] {
        let key_qps = |black_first| {
            let stream = encode(*mode, black_first);
            stream
                .stats
                .iter()
                .filter(|s| s.pts > 0 && stream.types[s.pts as usize] == FrameType::Key)
                .map(|s| s.qp)
                .collect::<Vec<u16>>()
        };
        let reference = key_qps(false);
        assert_eq!(reference.len(), 1, "{mode:?}");
        let black = key_qps(true);
        assert!(
            black.iter().all(|&qp| qp <= reference[0] + 16)
                && black
                    .last()
                    .is_some_and(|qp| qp.abs_diff(reference[0]) <= 16),
            "{mode:?}: {black:?} vs {reference:?}"
        );
    }
}

/// The frames of one wave take their qindex before any of them is coded.
/// After an underspend each frame alone could fill the buffer; CBR counts
/// the predicted size of each for the next, so the predicted level stays
/// inside the buffer after every frame of the wave.
#[test]
fn cbr_keeps_a_whole_wave_inside_the_buffer() {
    let buffer = 200 * 500;
    for step in 0..64 {
        let frame = FramePlanInfo {
            complexity: (1 << 16) + step * (1 << 18),
            ..plan(1, FrameType::Inter, 0)
        };
        let mut cbr = Cbr::new(200, (500, 250), (30, 1)).unwrap();
        for index in 0..30 {
            cbr.update(&stats(index, 100, 0));
        }
        cbr.plan_window(&[frame; 4]);
        for _ in 0..4 {
            let qindex = cbr.frame_qp(&frame);
            let level = buffer / 2 + cbr.vbr.drift + cbr.pending;
            assert!(
                qindex == 255 || level <= buffer,
                "step {step}: qindex {qindex}, level {level}"
            );
        }
    }
}

/// A CBR decoder buffer of half a second that starts at a fifth, half,
/// four fifths or all of its size: simulated frames, with a key frame every 30,
/// never underflow or overflow it, and the first second spends more the
/// fuller the buffer starts.
#[test]
fn cbr_keeps_the_decoder_buffer_from_any_initial_level() {
    let (frame_bits, buffer) = (200_000 / 30, 200 * 500);
    let mut first_second = Vec::new();
    for initial_ms in [100, 250, 400, 500] {
        let mut cbr = Cbr::new(200, (500, initial_ms), (30, 1)).unwrap();
        let sizes = simulate(&mut cbr, 30, |index| {
            let kind = if index % 30 == 0 {
                FrameType::Key
            } else {
                FrameType::Inter
            };
            plan(index, kind, -4 * (index % 4) as i16)
        });
        let mut level = 200 * u64::from(initial_ms);
        for (index, &bits) in sizes.iter().enumerate() {
            assert!(bits <= level, "{initial_ms} ms: frame {index} underflows");
            level = level - bits + frame_bits;
            assert!(level <= buffer, "{initial_ms} ms: frame {index} overflows");
        }
        first_second.push(sizes[..30].iter().sum::<u64>());
    }
    assert!(first_second.is_sorted_by(|a, b| a < b), "{first_second:?}");
}

/// Sixty frames of a 96x64 texture that holds still for 30 frames and then
/// pans 6 samples a frame, through VBR at preset 9 with `first_pass`, and
/// the bits of each output packet.
fn still_then_pan(config: &Config) -> Vec<u64> {
    let mut encoder = Encoder::new(config.clone()).unwrap();
    let mut bits = Vec::new();
    for index in 0..=60u64 {
        if index == 60 {
            encoder.flush().unwrap();
        } else {
            let shift = 6 * index.saturating_sub(30) as usize;
            encoder.send_frame(frame(config, index, shift, 0)).unwrap();
        }
        while let Some(packet) = encoder.receive_packet().unwrap() {
            bits.push(packet.data.len() as u64 * 8);
        }
    }
    bits
}

/// The first pass codes the frames at half size in the second pass's
/// mini-GOPs, gives the same bits on 1 and 8 threads, and costs the panning
/// half more than the still half; two-pass VBR then spends a larger share of
/// the same budget on the pan than one pass does.
#[test]
fn two_pass_vbr_follows_the_first_pass() {
    let mut config = Config::new(96, 64);
    config.preset = Preset::new(9).unwrap();
    config.rate_control = RateControlMode::Vbr { bitrate_kbps: 100 };
    let first = config.first_pass_config().unwrap();
    assert_eq!(first.coded().width, 48);
    assert_eq!(
        first.preset_overrides.last().unwrap(),
        &("mini_gop_size".to_owned(), "2".to_owned())
    );
    let pass = |threads: usize| {
        let mut config = config.clone();
        config.threads = splot_parallel::ThreadCount::from(threads);
        let mut pass = FirstPass::new(&config).unwrap();
        for index in 0..60u64 {
            let shift = 6 * index.saturating_sub(30) as usize;
            pass.send_frame(frame(&config, index, shift, 0)).unwrap();
        }
        pass.finish().unwrap()
    };
    let bits = pass(1);
    assert_eq!(bits, pass(8));
    let half = |bits: &[u64]| -> (u64, u64) { (bits[..30].iter().sum(), bits[30..].iter().sum()) };
    let (still, pan) = half(&bits);
    assert!(pan > 2 * still, "{still} {pan}");
    let one = still_then_pan(&config);
    config.first_pass = bits;
    let two = still_then_pan(&config);
    let share = |bits: &[u64]| half(bits).1 * 1000 / bits.iter().sum::<u64>();
    assert!(
        share(&two) > share(&one),
        "{} vs {}",
        share(&two),
        share(&one)
    );
}

fn capped(crf: u8, max_bitrate_kbps: u32, (buffer_ms, initial_ms): (u32, u32)) -> RateControlMode {
    RateControlMode::CappedCrf {
        crf,
        max_bitrate_kbps,
        buffer_ms,
        initial_ms,
    }
}

/// The units of `units` that underflow a decoder buffer of `buffer_ms` at
/// `kbps` that holds `initial_ms` first: each unit leaves, then one frame
/// period at 30 fps arrives, up to the buffer size.
fn underflows(units: &[Vec<u8>], kbps: u64, (buffer_ms, initial_ms): (u64, u64)) -> Vec<usize> {
    let (size, period) = (kbps * buffer_ms, kbps * 1000 / 30);
    let mut level = kbps * initial_ms;
    let mut late = Vec::new();
    for (index, unit) in units.iter().enumerate() {
        let bits = unit.len() as u64 * 8;
        if bits > level {
            late.push(index);
        }
        level = (level.saturating_sub(bits) + period).min(size);
    }
    late
}

/// A cap above every unit codes the CRF stream byte for byte, with no frame
/// coded twice.
#[test]
fn capped_crf_with_a_loose_cap_is_the_crf_stream() {
    let plain = encode(RateControlMode::Crf { crf: 20 }, false);
    let loose = encode(capped(20, MAX_BITRATE_KBPS, (1000, 500)), false);
    assert_eq!(loose.units, plain.units);
    assert_eq!((plain.recodes, loose.recodes), (0, 0));
}

/// A cap at 60% of the CRF rate underflows the CRF stream; capped CRF codes
/// frames again until no unit underflows, spends less, and still decodes to
/// the encoder recon.
#[test]
fn capped_crf_keeps_the_decoder_buffer_and_decodes_to_the_recon() {
    let plain = encode(RateControlMode::Crf { crf: 20 }, false);
    let cap = plain.kbps() * 6 / 10;
    let buffer = (500, 250);
    assert!(!underflows(&plain.units, cap, buffer).is_empty());
    let mode = capped(20, cap as u32, (500, 250));
    let stream = encode(mode, false);
    assert_eq!(underflows(&stream.units, cap, buffer), [] as [usize; 0]);
    assert!(stream.recodes > 0);
    assert!(stream.kbps() < plain.kbps());
    assert_same_samples(&decode(&stream.units.concat()), &stream.expected, "capped");
}

/// A 250 ms CBR buffer that starts half full: the planned qindex of a frame
/// can still overflow it, and the frame is coded again so that no unit
/// underflows the decoder buffer.
#[test]
fn cbr_codes_a_frame_again_instead_of_underflowing_a_small_buffer() {
    let mode = RateControlMode::Cbr {
        bitrate_kbps: 60,
        buffer_ms: 250,
        initial_ms: 125,
    };
    let stream = encode(mode, false);
    assert_eq!(underflows(&stream.units, 60, (250, 125)), [] as [usize; 0]);
    assert!(stream.recodes > 0);
    assert_same_samples(&decode(&stream.units.concat()), &stream.expected, "cbr");
}

/// The tries of a frame first coded at `first` and coded again while
/// [`DecoderBuffer::coarser`] asks, where qindex `q` costs `bits(q)`.
fn fit(buffer: &DecoderBuffer, first: u16, bits: impl Fn(u16) -> u64) -> Vec<(u16, u64)> {
    let mut tries = vec![(first, bits(first))];
    while let Some(next) = buffer.coarser(&tries, (0, 0), BitDepth::Eight) {
        assert!(next > tries.last().unwrap().0, "{tries:?} then {next}");
        tries.push((next, bits(next)));
    }
    tries
}

/// Bits that fall slower than `1 / step`, that rise between two tries, that
/// close in on the room from above, or that never fit: the last try fits the
/// room or is at `MaxQ`, within `MAX_RECODES + 8` retries.
#[test]
fn the_buffer_re_code_ends_fitting_or_at_the_maximum_qindex() {
    let buffer = DecoderBuffer {
        size: 30_000,
        drain: 1_000,
        level: 25_320,
    };
    let room = 4_680;
    let max = max_qindex(BitDepth::Eight);
    let flat = |q: u16| 60_000 / step(q, BitDepth::Eight).isqrt();
    let curves: [(&str, &dyn Fn(u16) -> u64); 4] = [
        ("flat", &flat),
        ("noisy", &|q| flat(q) + u64::from(q % 5) * 400),
        ("closing", &|q| {
            room + 4_000_000 / (u64::from(q) * u64::from(q))
        }),
        ("never", &|_| 10_000),
    ];
    for (name, bits) in curves {
        for first in [20, 69, 120, 200] {
            let tries = fit(&buffer, first, bits);
            let &(qindex, last) = tries.last().unwrap();
            assert!(last <= room || qindex == max, "{name}: {tries:?}");
            assert!(tries.len() <= MAX_RECODES + 9, "{name}: {tries:?}");
        }
    }
}

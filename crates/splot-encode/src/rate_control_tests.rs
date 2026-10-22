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
use crate::test_support::{assert_same_samples, avm_decode, avm_decoder, decode, raw_bytes};

fn plan(frame_index: u64, frame_type: FrameType, offset: i16) -> FramePlanInfo {
    FramePlanInfo {
        frame_index,
        display_index: frame_index,
        frame_type,
        layer: 0,
        complexity: 50_000,
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
            BitrateController::cbr(200, 500, (30, 1))
        } else {
            BitrateController::vbr(200, (30, 1))
        };
        let (mut total, mut largest) = (0, 0);
        for window in 0..10u64 {
            let frames: Vec<FramePlanInfo> = (0..4)
                .map(|i| {
                    let index = window * 4 + i;
                    let kind = if index % 20 == 0 {
                        FrameType::Key
                    } else {
                        FrameType::Inter
                    };
                    plan(index, kind, -4 * i as i16)
                })
                .collect();
            control.plan_window(&frames);
            for info in &frames {
                let qp = control.frame_qp(info);
                let k = if info.frame_type == FrameType::Key {
                    120
                } else {
                    20
                };
                let bits = k * info.complexity / step(qp, info.bit_depth);
                control.update(&FrameStats {
                    frame_index: info.frame_index,
                    pts: info.frame_index,
                    qp,
                    bits,
                    psnr_y: 0.0,
                    psnr_u: 0.0,
                    psnr_v: 0.0,
                    ssim_y: None,
                });
                total += bits;
                largest = largest.max(bits);
            }
        }
        let target = 200_000 * 40 / 30;
        assert!(
            total.abs_diff(target) * 20 < target,
            "cbr {cbr}: {total} vs {target}"
        );
        if cbr {
            assert!(largest < 200 * 500 / 2 + 200_000 / 30, "{largest}");
        }
    }
}

/// Records the raw decoder output of every coded frame by display index.
struct Recording {
    coder: IntraFrameCoder,
    bit_depth: BitDepth,
    recons: Arc<Mutex<BTreeMap<u64, Vec<u8>>>>,
}

impl FrameCoder for Recording {
    fn encode_frame(
        &mut self,
        source: &SourceFrame,
        plan: &GopFrame,
        qp: u16,
    ) -> Result<CodedFrame> {
        let coded = self.coder.encode_frame(source, plan, qp)?;
        let recon = raw_bytes(coded.recon.iter().map(Vec::as_slice), self.bit_depth);
        self.recons
            .lock()
            .unwrap()
            .insert(plan.display_index, recon);
        Ok(coded)
    }

    fn show_existing_frame(&mut self, slot: u32) -> Result<Vec<u8>> {
        self.coder.show_existing_frame(slot)
    }
}

/// A coded stream: its temporal units, the encoder reconstruction in display
/// order, each packet's frame type, and the stats of each coded frame.
struct Stream {
    units: Vec<Vec<u8>>,
    expected: Vec<u8>,
    types: Vec<FrameType>,
    stats: Vec<FrameStats>,
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
/// texture from frame 17, coded at preset 9 (mini-GOPs of 2).
fn encode(rate_control: RateControlMode) -> Stream {
    let mut config = Config::new(SIZE.0, SIZE.1);
    config.preset = Preset::new(9).unwrap();
    config.rate_control = rate_control;
    let recons = Arc::default();
    let coder = Recording {
        coder: IntraFrameCoder::new(&config).unwrap(),
        bit_depth: config.bit_depth,
        recons: Arc::clone(&recons),
    };
    let mut encoder = Encoder::with_coder(config.clone(), Box::new(coder));
    let mut packets = Vec::new();
    for index in 0..=FRAMES {
        if index == FRAMES {
            encoder.flush().unwrap();
        } else {
            let scene = u32::from(index >= 17);
            encoder
                .send_frame(frame(&config, index, 2 * index as usize, scene))
                .unwrap();
        }
        while let Some(packet) = encoder.receive_packet().unwrap() {
            packets.push(packet);
        }
    }
    let expected = recons.lock().unwrap().values().flatten().copied().collect();
    Stream {
        types: packets.iter().map(|packet| packet.frame_type).collect(),
        stats: packets.iter().flat_map(|p| p.stats.clone()).collect(),
        units: packets.into_iter().map(|packet| packet.data).collect(),
        expected,
    }
}

fn cases() -> [RateControlMode; 3] {
    [
        RateControlMode::Crf { crf: 30 },
        RateControlMode::Vbr { bitrate_kbps: 60 },
        RateControlMode::Cbr {
            bitrate_kbps: 60,
            buffer_ms: 1000,
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
        let stream = encode(mode);
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
    let kbps = [20, 40].map(|crf| encode(RateControlMode::Crf { crf }).kbps());
    assert!(
        kbps[0] > crf_kbps[0] && crf_kbps[0] > kbps[1],
        "{kbps:?} {crf_kbps:?}"
    );
}

/// AVM decodes the rate-controlled streams to the encoder recon.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_rate_controlled_streams_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for (index, mode) in cases().into_iter().enumerate() {
        let stream = encode(mode);
        let name = format!("rc{index}");
        let avm = avm_decode(&avmdec, &stream.units, SIZE, BitDepth::Eight, &name);
        assert!(avm == stream.expected, "{mode:?}: AVM output differs");
    }
}

// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Switch and RAS frames (§ 5.18.2, § 7.4.5) through the encoder.

use super::carry::{encoder_packets, obu_types};
use super::*;
use crate::Packet;
use crate::gop::{GopPlanner, GopStep, SwitchKind};
use splot_core::headers::frame::{FrameHeaderCore, TipFrameMode};
use splot_core::types::ObuType;

/// `(chroma, bit depth, switch_type)` of the switch streams.
const SWITCH_CASES: [(PixelFormat, BitDepth, u8); 4] = [
    (PixelFormat::Yuv420, BitDepth::Eight, 0),
    (PixelFormat::Yuv444, BitDepth::Ten, 0),
    (PixelFormat::Yuv420, BitDepth::Eight, 1),
    (PixelFormat::Yuv444, BitDepth::Ten, 1),
];

/// The preset overrides of `switch_type` on every second mini-GOP; a RAS
/// frame needs long-term ids.
fn switch_overrides(switch_type: u8) -> Vec<(String, String)> {
    let mut overrides = vec![
        ("switch_frames".to_owned(), "2".to_owned()),
        ("switch_type".to_owned(), switch_type.to_string()),
    ];
    if switch_type == 1 {
        overrides.push(("long_term_id_bits".to_owned(), "3".to_owned()));
    }
    overrides
}

/// 26 panning frames at preset 6 (mini-GOPs of 8) with a key frame every 24
/// frames and the switch frames of [`switch_overrides`], plus `extra`
/// overrides, in `mode` on `threads`: the config, the packets and the
/// encoder recon in display order. The anchors 16 (hidden) and 25 (shown)
/// are the switch frames.
fn switch_stream(
    (chroma, bit_depth, switch_type): (PixelFormat, BitDepth, u8),
    mode: RateControlMode,
    threads: usize,
    extra: &[(&str, &str)],
) -> (Config, Vec<Packet>, Vec<u8>) {
    let mut config = config(72, 40, bit_depth, 6, 150);
    config.chroma = chroma;
    config.rate_control = mode;
    config.keyint = core::num::NonZeroU32::new(24).unwrap();
    config.preset_overrides = switch_overrides(switch_type);
    for &(name, value) in extra {
        config
            .preset_overrides
            .push((name.to_owned(), value.to_owned()));
    }
    config.threads = splot_parallel::ThreadCount::from(threads);
    let (packets, expected) = encoder_packets(&config, 26);
    (config, packets, expected)
}

/// The bytes of `packets` in stream order.
fn stream_bytes(packets: &[Packet]) -> Vec<u8> {
    packets
        .iter()
        .flat_map(|packet| packet.data.clone())
        .collect()
}

/// The OBU type of the switch frames of `switch_type`.
const fn switch_obu(switch_type: u8) -> ObuType {
    if switch_type == 0 {
        ObuType::Switch
    } else {
        ObuType::RasFrame
    }
}

/// With `switch_frames = 2` the layer-0 frames 16 and 25 are `OBU_SWITCH` or
/// `OBU_RAS_FRAME` frames. The 4:2:0 8-bit and 4:4:4 10-bit streams decode to
/// the encoder recon with the same bytes on 1, 2 and 8 threads.
#[test]
fn switch_streams_decode_alike_on_any_thread_count() {
    each_case(SWITCH_CASES, |case| {
        let qp = RateControlMode::ConstantQp { qp: 150 };
        let (_, packets, expected) = switch_stream(case, qp, 1, &[]);
        let types = obu_types(&packets);
        let count = types
            .iter()
            .filter(|&&obu| obu == switch_obu(case.2))
            .count();
        assert_eq!(count, 2, "{case:?}");
        let units = stream_bytes(&packets);
        assert_same_samples(&decode(&units), &expected, &format!("{case:?}"));
        for threads in [2, 8] {
            let other = stream_bytes(&switch_stream(case, qp, threads, &[]).1);
            assert!(other == units, "{case:?}: {threads} threads");
        }
    });
}

/// CRF, VBR and CBR streams with switch and RAS frames decode to the
/// encoder recon.
#[test]
fn rate_controlled_switch_streams_decode_to_the_encoder_recon() {
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
        .flat_map(|mode| [0, 1].map(|switch_type| (mode, switch_type)));
    each_case(cases, |(mode, switch_type)| {
        let case = (PixelFormat::Yuv420, BitDepth::Eight, switch_type);
        let (_, packets, expected) = switch_stream(case, mode, 1, &[]);
        let name = format!("{mode:?} {switch_type}");
        assert_same_samples(&decode(&stream_bytes(&packets)), &expected, &name);
    });
}

/// `OBU_SWITCH` frames combine with open-loop key frames: the mini-GOP of
/// the open-loop key frame 24 codes no switch frame, and the stream decodes
/// to the encoder recon.
#[test]
fn switch_frames_combine_with_open_loop_keys() {
    let case = (PixelFormat::Yuv420, BitDepth::Eight, 0);
    let qp = RateControlMode::ConstantQp { qp: 150 };
    let (_, packets, expected) = switch_stream(case, qp, 1, &[("open_loop_key", "1")]);
    let types = obu_types(&packets);
    let count = |kind| types.iter().filter(|&&obu| obu == kind).count();
    assert_eq!(
        (count(ObuType::OpenLoopKey), count(ObuType::Switch)),
        (1, 2)
    );
    assert_same_samples(&decode(&stream_bytes(&packets)), &expected, "olk");
}

/// `switch_type` needs `switch_frames > 0`; a RAS frame needs
/// `long_term_id_bits > 0`, so it cannot combine with open-loop key frames,
/// and a keyint of at most 256 (§ 7.4.5).
#[test]
fn switch_types_refuse_what_they_cannot_code() {
    let validates = |overrides: &[(&str, &str)], keyint: u32| {
        let mut config = config(72, 40, BitDepth::Eight, 6, 150);
        config.keyint = core::num::NonZeroU32::new(keyint).unwrap();
        config.preset_overrides = overrides
            .iter()
            .map(|&(name, value)| (name.to_owned(), value.to_owned()))
            .collect();
        config.validate().is_ok()
    };
    let ras = [
        ("switch_frames", "2"),
        ("switch_type", "1"),
        ("long_term_id_bits", "3"),
    ];
    assert!(validates(&ras, 256));
    assert!(!validates(&ras, 257));
    assert!(!validates(&ras[..2], 30));
    assert!(!validates(&[("switch_type", "1"), ras[2]], 30));
    assert!(!validates(&[ras[0], ("switch_type", "2"), ras[2]], 30));
    assert!(!validates(
        &[ras[0], ras[1], ras[2], ("open_loop_key", "1")],
        30
    ));
    assert!(validates(&[ras[0], ("open_loop_key", "1")], 30));
}

/// One coded frame of [`planned_switch_stream`]: its plan, its header, and
/// the `RefValid` of the decoder's § 7.23 buffer after it.
type PlannedFrame = (crate::gop::GopFrame, FrameHeaderCore, Vec<bool>);

/// 26 panning frames of `switch_type` through the planner and the frame
/// coder at preset 6 with a key frame every 24 frames: the units, the recon
/// in display order and every coded frame.
fn planned_switch_stream(switch_type: u8) -> (Vec<u8>, Vec<u8>, Vec<PlannedFrame>) {
    let mut config = config(72, 40, BitDepth::Eight, 6, 150);
    config.keyint = core::num::NonZeroU32::new(24).unwrap();
    config.preset_overrides = switch_overrides(switch_type);
    let preset = config.override_preset(config.preset.params()).unwrap();
    let mut coder = IntraFrameCoder::with_preset(&config, &preset).unwrap();
    let mut planner = GopPlanner::new(config.keyint, preset.mini_gop_size)
        .with_switch_frames(preset.switch_frames, preset.switch_type);
    let (mut units, mut recons, mut frames) = (Vec::new(), BTreeMap::new(), Vec::new());
    let mut queued = 26;
    while let Some(len) = planner.next_len(queued, true) {
        queued -= len;
        for step in planner.plan(len).unwrap() {
            match step {
                GopStep::Code(plan) => {
                    let source = panning_frame(&config, plan.display_index as u32);
                    let mut coded = coder.code_frame(&source, &plan, 150, &[]).unwrap();
                    let core = coded.reference.as_ref().unwrap().0.clone();
                    coder.commit(&mut coded).unwrap();
                    let valid = coder.references.header_view().ref_valid.unwrap().to_vec();
                    frames.push((plan, core, valid));
                    recons.insert(plan.display_index, recon_bytes(&coded, config.bit_depth));
                    units.extend(coded.temporal_unit);
                }
                GopStep::ShowExisting { slot, .. } => {
                    units.extend(coder.show_existing_frame(slot, None, false).unwrap());
                }
            }
        }
    }
    (units, recons.into_values().flatten().collect(), frames)
}

/// After a RAS frame the decoder's buffer holds only the RAS frame and the
/// key frame (§ 7.23), the RAS frame lists the key frame's long-term id, and
/// the frames up to the next key frame reference only those two slots.
#[test]
fn frames_after_a_ras_frame_reference_only_it_and_the_key_frame() {
    let (units, expected, frames) = planned_switch_stream(1);
    assert_same_samples(&decode(&units), &expected, "ras");
    let ras: Vec<usize> = (0..frames.len())
        .filter(|&at| frames[at].0.switch == Some(SwitchKind::Ras))
        .collect();
    assert_eq!(ras.len(), 2);
    for &at in &ras {
        let (plan, core, valid) = &frames[at];
        assert_eq!(core.obu_type, ObuType::RasFrame);
        let key = frames[..at]
            .iter()
            .rfind(|(plan, ..)| plan.frame_type == FrameType::Key)
            .unwrap();
        let id = key.1.long_term_id.and_then(|id| u32::try_from(id).ok());
        assert!(
            id.is_some(),
            "key frame {} has no long-term id",
            key.0.display_index
        );
        assert_eq!(core.ref_long_term_ids, id.into_iter().collect::<Vec<_>>());
        let held: Vec<usize> = (0..valid.len()).filter(|&slot| valid[slot]).collect();
        assert_eq!(held, [0, 1], "after {}", plan.display_index);
        for (later, core, _) in frames[at + 1..]
            .iter()
            .take_while(|(later, ..)| later.frame_type != FrameType::Key)
        {
            let inter = core.inter.as_ref().unwrap();
            let count = inter.num_total_refs.unwrap() as usize;
            assert!(
                inter.ref_frame_idx[..count].iter().all(|&slot| slot < 2),
                "{}: {:?}",
                later.display_index,
                &inter.ref_frame_idx[..count]
            );
        }
    }
}

/// The frames between the key frame's mini-GOP and a switch frame that
/// follows them in display order reference it as a future frame and code
/// TIP (§ 7.12.2): the switch frame is an inter frame for
/// `ref_is_intra_frame`. The stream decodes to the encoder recon.
#[test]
fn frames_before_a_switch_frame_code_tip() {
    let (units, expected, frames) = planned_switch_stream(0);
    assert_same_samples(&decode(&units), &expected, "switch");
    let switch = frames
        .iter()
        .find(|(plan, ..)| plan.switch.is_some())
        .map(|(plan, ..)| *plan)
        .unwrap();
    let slot = switch.refresh_frame_flags.trailing_zeros();
    let tip = frames.iter().filter(|(plan, core, _)| {
        plan.frame_index > switch.frame_index
            && plan.display_index < switch.display_index
            && core.inter.as_ref().is_some_and(|inter| {
                inter.tip_frame_mode == Some(TipFrameMode::AsRef)
                    && inter.ref_frame_idx.contains(&slot)
            })
    });
    assert!(
        tip.count() > 0,
        "no TIP frame before {}",
        switch.display_index
    );
}

/// `units` with the `immediate_output_frame` bit of its key frame cleared:
/// a hidden copy of the key frame for a random access (§ 7.3.9.1), in a
/// sequence with `monotonic_output_order_flag = 1` and no film grain, where
/// the bit changes no other syntax element.
fn hidden_key_unit(units: &[u8]) -> Vec<u8> {
    let mut hidden = units.to_vec();
    let parsed = splot_core::annexb::parse_annex_b_obus_partial(units);
    let key = parsed
        .obus
        .iter()
        .find(|obu| obu.header.obu_type == ObuType::ClosedLoopKey)
        .unwrap();
    let start = key.payload.as_ptr() as usize - units.as_ptr() as usize;
    let mut reader =
        splot_core::bitio::BitReader::new(key.payload, splot_core::span::ByteOffset::new(0));
    assert_eq!(reader.read_bit().unwrap(), 1, "is_first_tile_group");
    if reader.read_uvlc().unwrap() == 0 {
        reader.read_uvlc().unwrap(); // seq_header_id_in_frame_header
    }
    reader.read_bits(3).unwrap(); // long_term_id_plus_1
    let bit = reader.consumed_bits() as usize;
    let mask = 0x80 >> (bit % 8);
    assert_ne!(hidden[start + bit / 8] & mask, 0, "immediate_output_frame");
    hidden[start + bit / 8] &= !mask;
    hidden
}

/// AVM decodes the switch streams like splot. A decode that starts at the
/// temporal unit of the first RAS frame, after a hidden copy of the key frame
/// (§ 7.3.9.1, § 7.4.5), outputs the frames of the RAS frame's mini-GOP and
/// all later frames, equal to the sequential decode.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_switch_streams_from_the_start_and_from_the_ras_frame() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for case in SWITCH_CASES {
        let qp = RateControlMode::ConstantQp { qp: 150 };
        let (config, packets, expected) = switch_stream(case, qp, 1, &[]);
        let size = (config.width, config.height);
        let units: Vec<Vec<u8>> = packets.iter().map(|packet| packet.data.clone()).collect();
        let name = format!("switch{}{}", case.1 as u8, case.2);
        let avm = avm_decode(&avmdec, &units, size, config.bit_depth, &name);
        assert!(avm == expected, "{case:?}: AVM output differs");
        if case.2 == 0 {
            continue;
        }
        let start = packets
            .iter()
            .position(|packet| {
                obu_types(core::slice::from_ref(packet)).contains(&ObuType::RasFrame)
            })
            .unwrap();
        let delimiter = splot_core::headers::frame::encode_temporal_delimiter_obu().unwrap();
        let mut access = vec![hidden_key_unit(&units[0])];
        access.push(units[start][delimiter.len()..].to_vec());
        access.extend_from_slice(&units[start + 1..]);
        let frame_bytes = expected.len() / 26;
        let avm = avm_decode(&avmdec, &access, size, config.bit_depth, &name);
        assert!(
            avm == expected[9 * frame_bytes..],
            "{case:?}: random access differs"
        );
        let splot = decode(&access.concat());
        assert!(
            splot == expected[9 * frame_bytes..],
            "{case:?}: splot random access differs"
        );
    }
}

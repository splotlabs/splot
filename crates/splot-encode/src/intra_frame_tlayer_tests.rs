// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;
use crate::Packet;
use splot_core::annexb::parse_annex_b_obus_partial;
use splot_core::types::{ObuType, TemporalLayerId};
use splot_decode::TemporalLayerSelection;

const FRAMES: u32 = 26;

/// A tile group, TIP or show-existing frame OBU: an OBU that codes a frame.
const fn codes_frame(obu_type: ObuType) -> bool {
    obu_type.is_tile_group() || obu_type.is_tip_frame() || obu_type.is_sef()
}

/// `(chroma, bit depth, max_tlayer_id)`: mini-GOPs of 8 have depths 0..=3.
const TLAYER_CASES: [(PixelFormat, BitDepth, u8); 3] = [
    (PixelFormat::Yuv420, BitDepth::Eight, 3),
    (PixelFormat::Yuv444, BitDepth::Ten, 2),
    (PixelFormat::Yuv420, BitDepth::Eight, 1),
];

/// 26 panning frames at preset 6 (mini-GOPs of 8, the loop filters on) with
/// `max_tlayer_id`, `implicit_output` and the `extra` preset overrides on `threads` threads: the
/// config, the packets and the encoder recon in display order.
fn tlayer_stream(
    (chroma, bit_depth, max_tlayer_id): (PixelFormat, BitDepth, u8),
    extra: &[(&str, &str)],
    threads: usize,
) -> (Config, Vec<Packet>, Vec<u8>) {
    let mut config = config(72, 40, bit_depth, 6, 150);
    config.chroma = chroma;
    let max_tlayer_id = max_tlayer_id.to_string();
    config.preset_overrides = [
        ("max_tlayer_id", max_tlayer_id.as_str()),
        ("implicit_output", "true"),
    ]
    .into_iter()
    .chain(extra.iter().copied())
    .map(|(field, value)| (field.to_owned(), value.to_owned()))
    .collect();
    config.threads = splot_parallel::ThreadCount::from(threads);
    config.frame_metadata.timecode = true;
    let (packets, expected) = carry::encoder_packets(&config, FRAMES, panning_frame);
    (config, packets, expected)
}

/// The temporal units of `packets` without the OBUs above temporal layer `n`
/// (the Annex F sub-bitstream of operating point `n`); a unit left without a
/// frame is dropped with its temporal delimiter.
fn sub_bitstream(packets: &[Packet], n: u8) -> Vec<Vec<u8>> {
    packets
        .iter()
        .filter_map(|packet| {
            let parsed = parse_annex_b_obus_partial(&packet.data);
            assert!(parsed.error.is_none());
            let kept: Vec<_> = parsed
                .obus
                .iter()
                .filter(|obu| obu.header.temporal_layer_id.get() <= n)
                .collect();
            kept.iter()
                .any(|obu| codes_frame(obu.header.obu_type))
                .then(|| {
                    let mut w = splot_core::write::BitWriter::new();
                    for obu in kept {
                        splot_core::write::write_annexb_obu(&mut w, &obu.header, obu.payload)
                            .unwrap();
                    }
                    w.into_bytes()
                })
        })
        .collect()
}

/// The temporal layer of each output frame in display order: the layer of the
/// tile group or show-existing frame that outputs it, the last frame OBU of
/// its packet.
fn output_tlayers(packets: &[Packet]) -> Vec<u8> {
    let mut by_pts: Vec<(u64, u8)> = packets
        .iter()
        .map(|packet| {
            let parsed = parse_annex_b_obus_partial(&packet.data);
            let last = parsed
                .obus
                .iter()
                .rfind(|obu| codes_frame(obu.header.obu_type))
                .unwrap();
            (packet.pts, last.header.temporal_layer_id.get())
        })
        .collect();
    by_pts.sort_unstable();
    by_pts.into_iter().map(|(_, tlayer)| tlayer).collect()
}

/// The raw splot output of `units` that decodes temporal layers `selection`.
fn decode_layers(units: &[u8], selection: TemporalLayerSelection) -> Vec<u8> {
    let threads = splot_parallel::ThreadCount::from(1);
    let ctx = splot_decode::DecodeContext::new(splot_decode::DecodeRuntimeConfig::new(threads));
    let options = splot_decode::DecodeOptions::default().with_temporal_layer_selection(selection);
    let mut out = Vec::new();
    ctx.unwrap()
        .decode_raw_bytes(units, options, &mut out)
        .unwrap();
    out
}

/// The frames of `expected` whose temporal layer is at most `n`.
fn frames_up_to(expected: &[u8], tlayers: &[u8], n: u8) -> Vec<u8> {
    let frame_bytes = expected.len() / tlayers.len();
    expected
        .chunks(frame_bytes)
        .zip(tlayers)
        .filter(|&(_, &tlayer)| tlayer <= n)
        .flat_map(|(frame, _)| frame.iter().copied())
        .collect()
}

/// With `max_tlayer_id` the frames code `obu_tlayer_id = min(depth,
/// max_tlayer_id)` (§ 5.2.2), and the sequence header, temporal delimiters
/// and key frames stay on layer 0 (§ 6.2.2); the timecode metadata of a
/// frame codes the frame's layer (§ 7.3.3). The stream decodes to the
/// encoder recon; for each lower operating point `n`, `--temporal-layer n`
/// and the sub-bitstream without the OBUs above `n` both output exactly the
/// frames of layers `0..=n`, equal to the full decode. The bytes are the
/// same on 1, 2 and 8 threads.
#[test]
fn temporal_layer_streams_decode_at_each_operating_point() {
    each_case(TLAYER_CASES, |case| {
        let (_, packets, expected) = tlayer_stream(case, &[], 1);
        let units: Vec<u8> = packets
            .iter()
            .flat_map(|packet| packet.data.clone())
            .collect();
        let parsed = parse_annex_b_obus_partial(&units);
        assert!(parsed.error.is_none(), "{case:?}");
        for packet in &packets {
            let parsed = parse_annex_b_obus_partial(&packet.data);
            for (index, obu) in parsed.obus.iter().enumerate() {
                if obu.header.obu_type != ObuType::MetadataShort {
                    continue;
                }
                let frame = parsed.obus[index..]
                    .iter()
                    .find(|obu| codes_frame(obu.header.obu_type))
                    .unwrap();
                assert_eq!(
                    obu.header.temporal_layer_id, frame.header.temporal_layer_id,
                    "{case:?}: frame metadata joins the unit of its frame (§ 7.3.3)"
                );
            }
        }
        let mut layers = [0usize; 4];
        for obu in &parsed.obus {
            let tlayer = obu.header.temporal_layer_id.get();
            layers[usize::from(tlayer)] += 1;
            let key = matches!(
                obu.header.obu_type,
                ObuType::ClosedLoopKey | ObuType::OpenLoopKey
            );
            if key
                || !(codes_frame(obu.header.obu_type)
                    || obu.header.obu_type == ObuType::MetadataShort)
            {
                assert_eq!(tlayer, 0, "{case:?}: {:?}", obu.header.obu_type);
            }
        }
        for (tlayer, &count) in layers.iter().enumerate() {
            assert_eq!(
                count > 0,
                tlayer <= usize::from(case.2),
                "{case:?}: layer {tlayer}"
            );
        }
        assert_same_samples(&decode(&units), &expected, &format!("{case:?}"));
        let tlayers = output_tlayers(&packets);
        for n in 0..case.2 {
            let frames = frames_up_to(&expected, &tlayers, n);
            assert!(frames.len() < expected.len(), "{case:?}: layer {n}");
            let target = TemporalLayerSelection::Target(TemporalLayerId::from_bits(n));
            let selected = decode_layers(&units, target);
            assert_same_samples(
                &selected,
                &frames,
                &format!("{case:?}: --temporal-layer {n}"),
            );
            let sub: Vec<u8> = sub_bitstream(&packets, n).concat();
            assert_same_samples(
                &decode(&sub),
                &frames,
                &format!("{case:?}: sub-bitstream {n}"),
            );
        }
        if case == TLAYER_CASES[0] {
            for threads in [2, 8] {
                let other: Vec<u8> = tlayer_stream(case, &[], threads)
                    .1
                    .iter()
                    .flat_map(|packet| packet.data.clone())
                    .collect();
                assert!(other == units, "{case:?}: {threads} threads");
            }
        }
    });
}

/// Temporal layers with open-loop key frames every 12 frames decode to the
/// encoder recon, and their layer 0 sub-bitstream outputs the layer 0 frames.
#[test]
fn temporal_layers_combine_with_open_loop_keys() {
    let mut config = config(72, 40, BitDepth::Eight, 6, 150);
    config.keyint = core::num::NonZeroU32::new(12).unwrap();
    config.preset_overrides = [
        ("max_tlayer_id", "3"),
        ("implicit_output", "true"),
        ("open_loop_key", "1"),
    ]
    .into_iter()
    .map(|(field, value)| (field.to_owned(), value.to_owned()))
    .collect();
    let (packets, expected) = carry::encoder_packets(&config, FRAMES, panning_frame);
    let units: Vec<u8> = packets
        .iter()
        .flat_map(|packet| packet.data.clone())
        .collect();
    assert_same_samples(&decode(&units), &expected, "open-loop keys");
    let tlayers = output_tlayers(&packets);
    let sub: Vec<u8> = sub_bitstream(&packets, 0).concat();
    let frames = frames_up_to(&expected, &tlayers, 0);
    assert_same_samples(&decode(&sub), &frames, "open-loop keys: sub-bitstream 0");
}

/// Temporal layers need `implicit_output`, and `Config::validate` refuses a
/// key frame interval above 255 with temporal layers and accepts 255.
#[test]
fn temporal_layers_refuse_long_key_frame_intervals() {
    let mut config = config(72, 40, BitDepth::Eight, 6, 150);
    config.preset_overrides = vec![("max_tlayer_id".to_owned(), "1".to_owned())];
    assert!(matches!(config.validate(), Err(Error::Preset(_))));
    config
        .preset_overrides
        .push(("implicit_output".to_owned(), "true".to_owned()));
    config.keyint = core::num::NonZeroU32::new(256).unwrap();
    assert!(matches!(
        config.validate(),
        Err(Error::KeyintWithTemporalLayers {
            keyint: 256,
            max: 255
        })
    ));
    config.keyint = core::num::NonZeroU32::new(255).unwrap();
    assert!(config.validate().is_ok());
    config.preset_overrides.clear();
    config.keyint = core::num::NonZeroU32::new(256).unwrap();
    assert!(config.validate().is_ok());
}

/// AVM decodes the temporal layer streams like splot, and each lower
/// operating point's sub-bitstream (Annex F) like `--temporal-layer n`.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_temporal_layer_streams_at_each_operating_point() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for case in TLAYER_CASES {
        let (config, packets, expected) = tlayer_stream(case, &[], 1);
        let size = (config.width, config.height);
        let units: Vec<Vec<u8>> = packets.iter().map(|packet| packet.data.clone()).collect();
        let name = format!("tlayer{}{}", case.1.bits(), case.2);
        let avm = avm_decode(&avmdec, &units, size, config.bit_depth, &name);
        assert!(avm == expected, "{case:?}: AVM output differs");
        let tlayers = output_tlayers(&packets);
        for n in 0..case.2 {
            let sub = sub_bitstream(&packets, n);
            let avm = avm_decode(
                &avmdec,
                &sub,
                size,
                config.bit_depth,
                &format!("{name}-{n}"),
            );
            let frames = frames_up_to(&expected, &tlayers, n);
            assert!(avm == frames, "{case:?}: AVM sub-bitstream {n} differs");
        }
    }
}

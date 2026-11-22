// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Input frames that repeat the frame before them, shown again with an
//! explicit `sef_order_hint` (§ 5.18.2) under the preset's `sef_repeat`.

use super::*;
use splot_core::annexb::parse_annex_b_obus_partial;
use splot_core::bitio::BitReader;
use splot_core::headers::frame::{
    FrameHeaderParseInput, FrameHeaderParseMode, FrameReferenceStateView, parse_frame_header_core,
};
use splot_core::headers::sequence::parse_sequence_header;
use splot_core::span::ByteOffset;
use splot_core::types::ObuType;
use splot_decode::{DataProps, Decoder, DecoderSettings, SendStatus};

/// Content indices with repeats right after the key frame, a run of two, a
/// repeat before the key frame at 8 and one at it, and one at the end.
const CONTENT: [u32; 16] = [0, 0, 1, 2, 3, 3, 4, 4, 4, 5, 6, 7, 8, 9, 10, 10];
/// The display indices of [`CONTENT`] that code a show-existing frame; 8 is
/// the scheduled key frame.
const REPEATS: [u64; 4] = [1, 5, 7, 15];

/// Makes the source frame of a content index.
type Source = fn(&Config, u32) -> SourceFrame;

/// `config` with `sef_repeat`, keyint 8 and `overrides`.
fn repeat_config(mut config: Config, overrides: &[(&str, &str)]) -> Config {
    config.keyint = core::num::NonZeroU32::new(8).unwrap();
    config.preset_overrides = [("sef_repeat", "true")]
        .iter()
        .chain(overrides)
        .map(|&(field, value)| (field.to_owned(), value.to_owned()))
        .collect();
    config
}

/// The panning frames of `content` through the [`Encoder`]: the temporal
/// units, one per input frame (in display order without `implicit_output`),
/// and the encoder recon of the frame each one
/// outputs, in display order. `frame` makes the source of a content index.
fn repeat_stream(config: &Config, content: &[u32], frame: Source) -> (Vec<Vec<u8>>, Vec<u8>) {
    let recons: Arc<Mutex<BTreeMap<u64, Vec<u8>>>> = Arc::default();
    let coder = Recording {
        coder: IntraFrameCoder::new(config).unwrap(),
        bit_depth: config.bit_depth,
        recons: Arc::clone(&recons),
    };
    let mut encoder = Encoder::with_coder(config.clone(), Box::new(coder)).unwrap();
    let mut packets = Vec::new();
    for (display, &index) in content.iter().enumerate() {
        let planes = frame(config, index).planes().clone();
        let source = SourceFrame::new(config, display as u64, planes).unwrap();
        encoder.send_frame(source).unwrap();
        packets.extend(core::iter::from_fn(|| encoder.receive_packet().unwrap()));
    }
    encoder.flush().unwrap();
    packets.extend(core::iter::from_fn(|| encoder.receive_packet().unwrap()));
    let mut pts: Vec<u64> = packets.iter().map(|packet| packet.pts).collect();
    pts.sort_unstable();
    assert_eq!(pts, (0..content.len() as u64).collect::<Vec<_>>());
    let recons = recons.lock().unwrap();
    let expected = (0..content.len() as u64)
        .flat_map(|display| recons.range(..=display).next_back().unwrap().1.clone())
        .collect();
    (
        packets.into_iter().map(|packet| packet.data).collect(),
        expected,
    )
}

/// The units of `units` that are a temporal delimiter and a show-existing
/// frame with `derive_sef_order_hint = 0`, with their `sef_order_hint`.
fn explicit_hints(units: &[Vec<u8>]) -> Vec<(u64, u32)> {
    let first = parse_annex_b_obus_partial(&units[0]);
    let seq_payload = first
        .obus
        .iter()
        .find(|obu| obu.header.obu_type == ObuType::SequenceHeader)
        .unwrap()
        .payload;
    let seq = parse_sequence_header(&mut BitReader::new(seq_payload, ByteOffset::new(0))).unwrap();
    let mut hints = Vec::new();
    for (display, unit) in units.iter().enumerate() {
        let obus = parse_annex_b_obus_partial(unit).obus;
        let Some(sef) = obus
            .iter()
            .find(|obu| obu.header.obu_type == ObuType::RegularSef)
        else {
            continue;
        };
        let input = FrameHeaderParseInput {
            obu_type: ObuType::RegularSef,
            first_picture_in_tu: true,
            active_sequence: Some(&seq),
            mfh_record: None,
            reference_state: FrameReferenceStateView::unknown(),
            mode: FrameHeaderParseMode::Core,
        };
        let mut reader = BitReader::new(sef.payload, ByteOffset::new(0));
        let core = parse_frame_header_core(&mut reader, &input).unwrap();
        if core.derive_sef_order_hint == Some(false) {
            let types: Vec<ObuType> = obus.iter().map(|obu| obu.header.obu_type).collect();
            assert_eq!(types, [ObuType::TemporalDelimiter, ObuType::RegularSef]);
            hints.push((display as u64, core.order_hint_lsb.unwrap()));
        }
    }
    hints
}

/// The repeats of [`CONTENT`] code only a show-existing frame with the
/// display order hint, and the 4:2:0 8-bit and 4:4:4 10-bit streams decode
/// to the frames they repeat, with the same bytes on 1, 2 and 8 threads;
/// the frames after them are filtered inter frames.
#[test]
fn repeats_decode_to_the_frames_they_show_on_any_thread_count() {
    for (chroma, bit_depth) in [
        (PixelFormat::Yuv420, BitDepth::Eight),
        (PixelFormat::Yuv444, BitDepth::Ten),
    ] {
        let mut config = repeat_config(config(72, 40, bit_depth, 6, 150), &[]);
        config.chroma = chroma;
        let case = format!("{chroma:?} {bit_depth:?}");
        let (units, expected) = repeat_stream(&config, &CONTENT, panning_frame);
        let hints = explicit_hints(&units);
        let displays: Vec<u64> = hints.iter().map(|&(display, _)| display).collect();
        assert_eq!(displays, REPEATS, "{case}");
        assert!(
            hints
                .iter()
                .all(|&(display, hint)| u64::from(hint) == display % 8)
        );
        assert_same_samples(&decode(&units.concat()), &expected, &case);
        for threads in [2, 8] {
            config.threads = splot_parallel::ThreadCount::from(threads);
            let other = repeat_stream(&config, &CONTENT, panning_frame).0;
            assert!(other == units, "{case}: {threads} threads");
        }
        config.preset_overrides.clear();
        let plain = repeat_stream(&config, &CONTENT, panning_frame).0;
        assert!(explicit_hints(&plain).is_empty(), "{case}: off");
    }
}

/// The repeats of a lossless stream code show-existing frames too, and the
/// stream decodes to the source of each shown frame.
#[test]
fn lossless_repeats_decode_to_the_source() {
    let config = repeat_config(config(72, 40, BitDepth::Eight, 6, 0), &[]);
    assert!(config.is_lossless());
    let (units, _) = repeat_stream(&config, &CONTENT, panning_frame);
    let displays: Vec<u64> = explicit_hints(&units).iter().map(|&(d, _)| d).collect();
    assert_eq!(displays, REPEATS);
    let source: Vec<u8> = CONTENT
        .iter()
        .flat_map(|&index| {
            let frame = panning_frame(&config, index);
            crate::test_support::raw_bytes(
                frame.planes().iter().map(Vec::as_slice),
                BitDepth::Eight,
            )
        })
        .collect();
    assert_same_samples(&decode(&units.concat()), &source, "lossless");
}

/// A run of repeats longer than the order-hint window allows codes its frame
/// 125 again; the stream decodes across the cut, and so do VBR, CBR and
/// `implicit_output` streams with repeats.
#[test]
fn capped_runs_and_rate_controlled_repeats_decode_to_the_shown_frames() {
    let base = repeat_config(
        config(72, 40, BitDepth::Eight, 6, 150),
        &[("mini_gop_size", "4")],
    );
    let mut long = base.clone();
    long.keyint = core::num::NonZeroU32::new(300).unwrap();
    let content: Vec<u32> = (0..135u32)
        .map(|display| display.saturating_sub(129))
        .collect();
    let (units, expected) = repeat_stream(&long, &content, panning_frame);
    let repeats = explicit_hints(&units);
    assert_eq!(
        repeats.len(),
        124 + 4,
        "1..=124 repeat 0, 126..=129 repeat 125"
    );
    assert!(repeats.iter().all(|&(display, _)| display != 125));
    assert_same_samples(&decode(&units.concat()), &expected, "capped run");
    let modes = [
        RateControlMode::Vbr { bitrate_kbps: 200 },
        RateControlMode::Cbr {
            bitrate_kbps: 200,
            buffer_ms: 1000,
            initial_ms: 500,
        },
    ];
    for (mode, implicit) in modes
        .into_iter()
        .map(|mode| (mode, false))
        .chain([(base.rate_control, true)])
    {
        let mut config = base.clone();
        config.rate_control = mode;
        if implicit {
            config
                .preset_overrides
                .push(("implicit_output".to_owned(), "true".to_owned()));
        }
        let (units, expected) = repeat_stream(&config, &CONTENT, panning_frame);
        assert_eq!(explicit_hints(&units).len(), REPEATS.len(), "{mode:?}");
        assert_same_samples(&decode(&units.concat()), &expected, &format!("{mode:?}"));
    }
}

/// Repeats with and without `implicit_output` refer to the multi-frame
/// header of each key frame unit (§ 5.18.2) and decode to the frames they
/// show.
#[test]
fn repeats_refer_to_the_multi_frame_header() {
    for overrides in [&[][..], &[("implicit_output", "true")]] {
        let mut config = repeat_config(config(72, 40, BitDepth::Eight, 6, 150), overrides);
        config.multi_frame_header = true;
        let case = format!("{overrides:?}");
        let (units, expected) = repeat_stream(&config, &CONTENT, panning_frame);
        let ids = super::mfh::mfh_ids(&units.concat());
        assert_eq!(super::mfh::assert_mfh_references(&ids, &case), 2, "{case}");
        assert!(
            ids.iter().any(|&(kind, _)| kind == ObuType::RegularSef),
            "{case}"
        );
        assert_same_samples(&decode(&units.concat()), &expected, &case);
    }
}

/// A noisy panning frame, so the film grain model has points.
fn grainy_frame(config: &Config, index: u32) -> SourceFrame {
    grain_streams::with_noise(config, &panning_frame(config, index), index)
}

/// The decoded pictures of `units`, with grain or without it.
fn pictures(units: &[Vec<u8>], skip_grain: bool) -> Vec<Vec<u8>> {
    let settings = DecoderSettings::default().with_skip_film_grain(skip_grain);
    let mut decoder = Decoder::new(settings).unwrap();
    let mut out = Vec::new();
    for unit in units {
        while decoder.send_data(unit, DataProps::default()).unwrap() == SendStatus::Full {
            out.extend(core::iter::from_fn(|| decoder.get_picture().unwrap()));
        }
    }
    decoder.drain();
    out.extend(core::iter::from_fn(|| decoder.get_picture().unwrap()));
    out.iter()
        .map(|picture| grain_streams::picture_bytes(core::slice::from_ref(picture)))
        .collect()
}

/// With film grain a repeat outputs the samples of the frame it shows and
/// the grain of its own `film_grain_config()`: § 5.18.2 skips
/// `save_grain_params` for `derive_sef_order_hint = 0`, and § 7.21.2 loads
/// the grain of the show-existing frame, so the repeat gets a new seed.
#[test]
fn repeats_with_film_grain_get_their_own_seed() {
    let mut config = repeat_config(config(72, 40, BitDepth::Eight, 6, 120), &[]);
    config.film_grain = true;
    let (units, expected) = repeat_stream(&config, &CONTENT, grainy_frame);
    assert_eq!(explicit_hints(&units).len(), REPEATS.len());
    let clean = pictures(&units, true);
    assert_same_samples(&clean.concat(), &expected, "no grain");
    let grained = pictures(&units, false);
    for display in REPEATS.map(|display| display as usize) {
        assert_eq!(clean[display], clean[display - 1]);
        assert_ne!(grained[display], grained[display - 1], "{display}");
        assert_ne!(grained[display], clean[display], "{display}");
    }
}

/// AVM decodes repeat streams like splot: the plain, grain, capped and
/// `implicit_output` streams.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_repeats_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    let base = repeat_config(config(72, 40, BitDepth::Eight, 6, 150), &[]);
    let mut grain = base.clone();
    grain.film_grain = true;
    let mut implicit = base.clone();
    implicit
        .preset_overrides
        .push(("implicit_output".to_owned(), "true".to_owned()));
    let mut long = base.clone();
    long.keyint = core::num::NonZeroU32::new(300).unwrap();
    let capped: Vec<u32> = (0..135u32)
        .map(|display| display.saturating_sub(129))
        .collect();
    let cases: [(&str, &Config, &[u32], Source); 4] = [
        ("repeat", &base, &CONTENT, panning_frame),
        ("repeatgrain", &grain, &CONTENT, grainy_frame),
        ("repeatimplicit", &implicit, &CONTENT, panning_frame),
        ("repeatcapped", &long, &capped, panning_frame),
    ];
    for (name, config, content, frame) in cases {
        let (units, _) = repeat_stream(config, content, frame);
        let splot = pictures(&units, false).concat();
        let avm = avm_decode(&avmdec, &units, (72, 40), BitDepth::Eight, name);
        assert!(avm == splot, "{name}: AVM output differs");
    }
}

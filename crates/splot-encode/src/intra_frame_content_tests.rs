// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The content interpretation OBU (§ 5.15) and metadata OBUs (§ 5.17)
//! of a described stream.

use super::*;
use crate::content::{
    ChromaSamplePosition, ContentDescription, ContentLightLevel, MasteringDisplay,
};
use splot_core::annexb::parse_annex_b_obus_partial;
use splot_core::bitio::BitReader;
use splot_core::headers::content_interpretation::{
    ContentInterpretation, parse_content_interpretation,
};
use splot_core::headers::metadata::{MetadataPayload, MetadataType, parse_metadata_short};
use splot_core::span::ByteOffset;
use splot_core::types::ObuType;

/// A BT.2020 mastering display of 1000 to 0.0001 cd/m² with a D65 white.
const DISPLAY: MasteringDisplay = MasteringDisplay {
    primaries: [(8500, 39850), (6550, 2300), (35400, 14600)],
    white_point: (15635, 16450),
    luminance_max: 10_000_000,
    luminance_min: 1,
};
const LIGHT: ContentLightLevel = ContentLightLevel {
    max_cll: 1000,
    max_fall: 400,
};

/// BT.2100 PQ at full range, left chroma, a 4:3 sample, 30000/1001 timing
/// and HDR metadata.
fn described(film_grain: bool) -> Config {
    let mut config = config(72, 40, BitDepth::Ten, 10, 120);
    config.keyint = core::num::NonZeroU32::new(2).unwrap();
    config.frame_rate = (30000, 1001);
    config.film_grain = film_grain;
    config.content.color_primaries = 9;
    config.content.transfer_characteristics = 16;
    config.content.matrix_coefficients = 9;
    config.content.full_range = true;
    config.content.chroma_sample_position = Some(ChromaSamplePosition::Left);
    config.content.sample_aspect_ratio = Some((8, 6));
    config.content.timing_info = true;
    config.content.mastering_display = Some(DISPLAY);
    config.content.content_light_level = Some(LIGHT);
    config
}

/// The payloads of the metadata short OBUs of `unit`, with their
/// `(muh_layer_idc, muh_persistence_idc, metadata_is_suffix)`.
fn metadata(unit: &[u8]) -> Vec<(MetadataPayload, (u8, u8, bool))> {
    parse_annex_b_obus_partial(unit)
        .obus
        .iter()
        .filter(|obu| obu.header.obu_type == ObuType::MetadataShort)
        .map(|obu| {
            let mut reader = BitReader::new(obu.payload, ByteOffset::new(0));
            let obu = parse_metadata_short(&mut reader, obu.payload.len()).unwrap();
            let header = (
                obu.muh_layer_idc,
                obu.muh_persistence_idc,
                obu.metadata_is_suffix,
            );
            (obu.unit.unwrap().payload, header)
        })
        .collect()
}

/// The OBU types of `unit` and its content interpretation OBUs.
fn obus(unit: &[u8]) -> (Vec<ObuType>, Vec<ContentInterpretation>) {
    let parsed = parse_annex_b_obus_partial(unit);
    assert!(parsed.error.is_none(), "{:?}", parsed.error);
    let types = parsed.obus.iter().map(|obu| obu.header.obu_type).collect();
    let content = parsed
        .obus
        .iter()
        .filter(|obu| obu.header.obu_type == ObuType::ContentInterpretation)
        .map(|obu| {
            let mut reader = BitReader::new(obu.payload, ByteOffset::new(0));
            parse_content_interpretation(&mut reader).unwrap()
        })
        .collect();
    (types, content)
}

/// Every key frame carries the same content interpretation, mastering display
/// and content light level OBUs between its sequence header and its film
/// grain OBU, the inter frames carry none, and the samples do not change.
#[test]
fn key_frames_carry_the_content_description() {
    for film_grain in [false, true] {
        let config = described(film_grain);
        let frames = code_stream(&config, &[3, 4, 5, 6]);
        let mut plain = config.clone();
        plain.content = ContentDescription::default();
        let plain_frames = code_stream(&plain, &[3, 4, 5, 6]);
        for (index, (frame, plain_frame)) in frames.iter().zip(&plain_frames).enumerate() {
            let (types, content) = obus(&frame.temporal_unit);
            let (plain_types, plain_content) = obus(&plain_frame.temporal_unit);
            assert!(plain_content.is_empty());
            assert_eq!(frame.recon, plain_frame.recon);
            if index % 2 == 1 {
                assert_eq!(types, plain_types);
                continue;
            }
            assert_eq!(
                types[..5],
                [
                    ObuType::TemporalDelimiter,
                    ObuType::SequenceHeader,
                    ObuType::ContentInterpretation,
                    ObuType::MetadataShort,
                    ObuType::MetadataShort
                ]
            );
            assert_eq!([&types[..2], &types[5..]].concat(), plain_types);
            let [
                (MetadataPayload::HdrMdcv(display), mdcv),
                (MetadataPayload::HdrCll(light), cll),
            ] = metadata(&frame.temporal_unit)[..]
            else {
                unreachable!("a key frame has a mastering display and a light level OBU");
            };
            assert_eq!((mdcv, cll), ((1, 0, false), (1, 0, false)));
            assert_eq!(display.primary_chromaticity_x, [8500, 6550, 35400]);
            assert_eq!(display.primary_chromaticity_y, [39850, 2300, 14600]);
            assert_eq!(
                (
                    display.white_point_chromaticity_x,
                    display.white_point_chromaticity_y
                ),
                DISPLAY.white_point
            );
            assert_eq!(
                (display.luminance_max, display.luminance_min),
                (10_000_000, 1)
            );
            assert_eq!((light.max_cll, light.max_fall), (1000, 400));
            assert_eq!(content.len(), 1);
            let content = content[0];
            assert!(content.scan_type_idc.is_progressive_frame());
            let color = content.color_description.unwrap();
            assert_eq!(
                (color.color_description_idc, color.full_range_flag),
                (2, true)
            );
            let chroma = content.chroma_sample_position.unwrap();
            assert_eq!((chroma.top, chroma.bottom), (0, 0));
            assert_eq!(content.aspect_ratio.unwrap().aspect_ratio_idc, 14);
            let timing = content.timing_info.unwrap();
            assert_eq!(
                (timing.num_units_in_display_tick, timing.time_scale),
                (1001, 30000)
            );
            assert_eq!(timing.num_ticks_per_picture_minus_1, Some(0));
        }
        let stream: Vec<u8> = frames
            .iter()
            .flat_map(|f| f.temporal_unit.clone())
            .collect();
        let plain: Vec<u8> = plain_frames
            .iter()
            .flat_map(|f| f.temporal_unit.clone())
            .collect();
        assert_same_samples(&decode(&stream), &decode(&plain), "described");
    }
}

#[test]
fn avm_decodes_a_described_stream_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    let units: Vec<Vec<u8>> = code_stream(&described(false), &[3, 4, 5, 6])
        .into_iter()
        .map(|frame| frame.temporal_unit)
        .collect();
    let avm = avm_decode(&avmdec, &units, (72, 40), BitDepth::Ten, "content");
    assert!(avm == decode(&units.concat()), "AVM output differs");
}

/// HDR metadata alone, with no content interpretation OBU, still decodes.
#[test]
fn hdr_metadata_needs_no_content_interpretation() {
    let mut config = config(72, 40, BitDepth::Ten, 10, 120);
    config.content.content_light_level = Some(LIGHT);
    let frame = code(&config, 3);
    let (types, _) = obus(&frame.temporal_unit);
    assert_eq!(
        types[..3],
        [
            ObuType::TemporalDelimiter,
            ObuType::SequenceHeader,
            ObuType::MetadataShort
        ]
    );
    assert_same_samples(
        &decode(&frame.temporal_unit),
        &recon_bytes(&frame, config.bit_depth),
        "light level",
    );
}

#[test]
fn rejects_a_description_the_input_cannot_have() {
    let invalid = |change: fn(&mut Config)| {
        let mut config = config(16, 16, BitDepth::Eight, 10, 120);
        change(&mut config);
        matches!(
            config.validate(),
            Err(Error::InvalidContentDescription { .. })
        )
    };
    assert!(invalid(|config| {
        config.chroma = PixelFormat::Yuv444;
        config.content.chroma_sample_position = Some(ChromaSamplePosition::Left);
    }));
    assert!(invalid(|config| {
        config.chroma = PixelFormat::Yuv422;
        config.content.chroma_sample_position = Some(ChromaSamplePosition::TopLeft);
    }));
    assert!(invalid(
        |config| config.content.sample_aspect_ratio = Some((4, 0))
    ));
    assert!(invalid(|config| {
        config.content.mastering_display = Some(MasteringDisplay {
            luminance_max: 50000,
            luminance_min: 50000,
            ..DISPLAY
        });
    }));
    assert!(!invalid(|config| {
        config.chroma = PixelFormat::Yuv422;
        config.content.chroma_sample_position = Some(ChromaSamplePosition::Center);
    }));
}

/// An ICC profile of a 128-byte header and four data bytes.
const ICC: &[u8] = &[0x49; 132];
/// Country code 0xFF, extension byte 0x01, then three payload bytes.
const T35: &[u8] = &[0xFF, 0x01, 0x00, 0x3C, 0x42];
const USER_DATA: &[u8] = b"0123456789abcdefuser data";

/// A 10-bit stream of `chroma` with key frames every second frame, a content
/// light level and, with `payloads`, the three key frame metadata payloads.
fn with_payloads(chroma: PixelFormat, payloads: bool) -> Config {
    let mut config = config(72, 40, BitDepth::Ten, 10, 120);
    config.keyint = core::num::NonZeroU32::new(2).unwrap();
    config.chroma = chroma;
    config.content.content_light_level = Some(LIGHT);
    if payloads {
        let metadata = &mut config.key_frame_metadata;
        metadata.icc_profile = Some(ICC.to_vec());
        metadata.itut_t35 = Some(T35.to_vec());
        metadata.user_data_unregistered = Some(USER_DATA.to_vec());
    }
    config
}

/// Every key frame carries an ICC profile, a T.35 and a user data OBU after
/// its HDR metadata, each with the bytes of its payload; the inter frames
/// carry none, and the samples do not change.
#[test]
fn key_frames_carry_the_metadata_payloads() {
    for chroma in [PixelFormat::Yuv420, PixelFormat::Yuv444] {
        let frames = code_stream(&with_payloads(chroma, true), &[3, 4, 5, 6]);
        let plain_frames = code_stream(&with_payloads(chroma, false), &[3, 4, 5, 6]);
        for (index, (frame, plain_frame)) in frames.iter().zip(&plain_frames).enumerate() {
            assert_eq!(frame.recon, plain_frame.recon);
            let (types, _) = obus(&frame.temporal_unit);
            let (plain_types, _) = obus(&plain_frame.temporal_unit);
            if index % 2 == 1 {
                assert_eq!(types, plain_types);
                continue;
            }
            let mut expected = plain_types.clone();
            expected.splice(3..3, [ObuType::MetadataShort; 3]);
            assert_eq!(types, expected);
            let parsed = parse_annex_b_obus_partial(&frame.temporal_unit);
            for (obu, (metadata_type, bytes)) in parsed.obus[3..6].iter().zip([
                (MetadataType::IccProfile, ICC),
                (MetadataType::ItutT35, T35),
                (MetadataType::UserDataUnregistered, USER_DATA),
            ]) {
                let header = [0x10, u8::try_from(metadata_type.value()).unwrap()];
                assert_eq!(obu.payload, [&header[..], bytes, &[0x80]].concat());
            }
            let [
                _,
                (MetadataPayload::IccProfile(_), _),
                (MetadataPayload::ItutT35(t35), _),
                (MetadataPayload::UserDataUnregistered(user), _),
            ] = metadata(&frame.temporal_unit)[..]
            else {
                unreachable!("a key frame has a light level, ICC, T.35 and user data OBU");
            };
            assert_eq!(
                (
                    t35.itu_t_t35_country_code,
                    t35.itu_t_t35_country_code_extension_byte
                ),
                (0xFF, Some(0x01))
            );
            assert_eq!(&user.uuid_iso_iec_11578, b"0123456789abcdef");
        }
        let stream: Vec<u8> = frames
            .iter()
            .flat_map(|f| f.temporal_unit.clone())
            .collect();
        let plain: Vec<u8> = plain_frames
            .iter()
            .flat_map(|f| f.temporal_unit.clone())
            .collect();
        assert_same_samples(&decode(&stream), &decode(&plain), "metadata payloads");
    }
}

#[test]
fn avm_decodes_a_stream_with_metadata_payloads_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    let units: Vec<Vec<u8>> = code_stream(&with_payloads(PixelFormat::Yuv420, true), &[3, 4, 5, 6])
        .into_iter()
        .map(|frame| frame.temporal_unit)
        .collect();
    let avm = avm_decode(&avmdec, &units, (72, 40), BitDepth::Ten, "payloads");
    assert!(avm == decode(&units.concat()), "AVM output differs");
}

#[test]
fn rejects_a_metadata_payload_its_syntax_cannot_carry() {
    let refused = |icc: &[u8], t35: &[u8], user: &[u8]| {
        let mut config = config(16, 16, BitDepth::Eight, 10, 120);
        let metadata = &mut config.key_frame_metadata;
        metadata.icc_profile = Some(icc.to_vec());
        metadata.itut_t35 = Some(t35.to_vec());
        metadata.user_data_unregistered = Some(user.to_vec());
        matches!(config.validate(), Err(Error::InvalidMetadata { .. }))
    };
    let uuid = [7; 16];
    let icc = [0; 128];
    assert!(!refused(&icc, &[0xB5, 0], &uuid));
    assert!(!refused(&icc, &[0xFF, 1, 0], &uuid));
    assert!(refused(&icc[..127], &[0xB5, 0], &uuid));
    for t35 in [&[][..], &[0xB5], &[0xFF], &[0xFF, 1]] {
        assert!(refused(&icc, t35, &uuid), "{t35:?}");
    }
    assert!(refused(&icc, &[0xB5, 0], &uuid[..15]));
}

/// `metadata_banding_hints()` of one component with a band width of 9 and a
/// step of 4, padded to two bytes.
const BANDING: [u8; 2] = [0xA8, 0xA6];

/// A 30000/1001 stream with timing info, a key frame every eighth frame,
/// the preset `overrides` and, with `metadata`, every per-frame metadata.
fn with_frame_metadata(overrides: &[&str], metadata: bool) -> Config {
    let mut config = config(72, 40, BitDepth::Eight, 10, 120);
    config.keyint = core::num::NonZeroU32::new(8).unwrap();
    config.frame_rate = (30000, 1001);
    config.content.timing_info = true;
    config.preset_overrides = overrides
        .iter()
        .map(|field| ((*field).to_owned(), "true".to_owned()))
        .collect();
    if metadata {
        let frame = &mut config.frame_metadata;
        frame.timecode = true;
        frame.temporal_point_info = true;
        frame.scan_type = true;
        frame.banding_hints = Some(BANDING.to_vec());
    }
    config
}

/// The per-frame metadata of `units` in stream order, one entry per
/// timecode: `(display index, presentation time, followed by a key frame)`.
fn frame_metadata(units: &[Vec<u8>]) -> Vec<(u64, u32, bool)> {
    let mut entries = Vec::new();
    for unit in units {
        let parsed = parse_annex_b_obus_partial(unit);
        for (index, obu) in parsed.obus.iter().enumerate() {
            if obu.header.obu_type != ObuType::MetadataShort {
                continue;
            }
            let mut reader = BitReader::new(obu.payload, ByteOffset::new(0));
            let short = parse_metadata_short(&mut reader, obu.payload.len()).unwrap();
            let Some(MetadataPayload::Timecode(timecode)) = short.unit.map(|unit| unit.payload)
            else {
                continue;
            };
            assert_eq!(short.muh_persistence_idc, 2);
            let [tpi, scan, banding] = &parsed.obus[index + 1..index + 4] else {
                unreachable!("four metadata OBUs per output frame");
            };
            let tpi = parse_metadata_short(
                &mut BitReader::new(tpi.payload, ByteOffset::new(0)),
                tpi.payload.len(),
            )
            .unwrap();
            let Some(MetadataPayload::TemporalPointInfo(point)) = tpi.unit.map(|unit| unit.payload)
            else {
                unreachable!("temporal point info follows the timecode");
            };
            assert_eq!(scan.payload, [0x12, 8, 0b0000_0010, 0x80]);
            assert_eq!(
                banding.payload,
                [&[0x12, 6][..], &BANDING, &[0x80]].concat()
            );
            let frame = parsed.obus[index + 4..]
                .iter()
                .map(|obu| obu.header.obu_type)
                .find(|&kind| kind != ObuType::MetadataShort && kind != ObuType::FilmGrain)
                .unwrap();
            assert!(
                matches!(
                    frame,
                    ObuType::ClosedLoopKey
                        | ObuType::RegularTileGroup
                        | ObuType::RegularSef
                        | ObuType::RegularTip
                ),
                "{frame:?}"
            );
            let seconds = u64::from(timecode.seconds_value.unwrap());
            assert_eq!(timecode.minutes_value, Some(0));
            entries.push((
                seconds * 30 + u64::from(timecode.n_frames),
                point.frame_presentation_time,
                frame == ObuType::ClosedLoopKey,
            ));
        }
    }
    entries
}

/// The temporal units of `frames` input frames through the [`Encoder`], in
/// coding order; with `repeats` every input frame comes twice.
fn encoded_units(config: &Config, frames: u32, repeats: bool) -> Vec<Vec<u8>> {
    let mut encoder = Encoder::new(config.clone()).unwrap();
    let mut units = Vec::new();
    for index in (0..frames).map(Some).chain([None]) {
        match index {
            Some(index) => {
                let seed = if repeats { index / 2 } else { index };
                encoder.send_frame(synthetic_frame(config, seed)).unwrap();
            }
            None => encoder.flush().unwrap(),
        }
        while let Some(packet) = encoder.receive_packet().unwrap() {
            units.push(packet.data);
        }
    }
    units
}

/// Every output frame, coded, implicit or shown again, carries one timecode,
/// temporal point info, scan type and banding hints OBU in front of its
/// frame OBU; the timecodes count every display index once; the
/// presentation time counts from the last key frame; key frames write a
/// progressive content interpretation OBU; the samples do not change, and
/// the stream is the same on 1 and 8 threads.
#[test]
fn output_frames_carry_the_frame_metadata() {
    for overrides in [&[][..], &["implicit_output"], &["sef_repeat"]] {
        let repeats = overrides == ["sef_repeat"];
        let config = with_frame_metadata(overrides, true);
        let units = encoded_units(&config, 34, repeats);
        let plain_units = encoded_units(&with_frame_metadata(overrides, false), 34, repeats);
        let mut entries = frame_metadata(&units);
        entries.sort_unstable();
        let indices: Vec<u64> = entries.iter().map(|entry| entry.0).collect();
        assert_eq!(indices, (0..34).collect::<Vec<_>>(), "{overrides:?}");
        let mut key = 0;
        for &(index, time, is_key) in &entries {
            if is_key {
                key = index;
            }
            assert_eq!(u64::from(time), index - key, "{overrides:?} frame {index}");
        }
        let shown_again = units
            .iter()
            .flat_map(|unit| obus(unit).0)
            .filter(|&kind| kind == ObuType::RegularSef)
            .count();
        assert_eq!(
            shown_again > 0,
            overrides != ["implicit_output"],
            "{overrides:?}"
        );
        let keys = entries.iter().filter(|entry| entry.2).count();
        let content = units.iter().map(|unit| obus(unit).1.len()).sum::<usize>();
        assert_eq!(content, keys);
        assert_same_samples(
            &decode(&units.concat()),
            &decode(&plain_units.concat()),
            "frame metadata",
        );
        if overrides.is_empty() {
            let mut threaded = config.clone();
            threaded.threads = splot_parallel::ThreadCount::from(8);
            assert_eq!(encoded_units(&threaded, 34, false), units);
        }
    }
}

#[test]
fn avm_decodes_a_stream_with_frame_metadata_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    let config = with_frame_metadata(&["implicit_output"], true);
    let units = encoded_units(&config, 18, false);
    let avm = avm_decode(&avmdec, &units, (72, 40), BitDepth::Eight, "frame-metadata");
    assert!(avm == decode(&units.concat()), "AVM output differs");
}

#[test]
fn rejects_frame_metadata_the_stream_cannot_carry() {
    let refused = |change: fn(&mut Config)| {
        let mut config = with_frame_metadata(&[], true);
        change(&mut config);
        matches!(config.validate(), Err(Error::InvalidMetadata { .. }))
    };
    assert!(!refused(|_| {}));
    assert!(refused(|config| config.content.timing_info = false));
    assert!(!refused(|config| {
        config.content.timing_info = false;
        config.frame_metadata.temporal_point_info = false;
    }));
    assert!(refused(|config| config.frame_rate = (513, 1)));
    assert!(!refused(|config| config.frame_rate = (512, 1)));
    assert!(refused(
        |config| config.frame_metadata.banding_hints = Some(vec![0xA8])
    ));
    assert!(refused(|config| {
        config.frame_metadata.banding_hints = Some(vec![0xA8, 0xA6, 0]);
    }));
}

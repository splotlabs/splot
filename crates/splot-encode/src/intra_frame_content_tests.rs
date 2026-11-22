// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The content interpretation OBU (§ 5.15) and HDR metadata OBUs (§ 5.17)
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
use splot_core::headers::metadata::{MetadataPayload, parse_metadata_short};
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

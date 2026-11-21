// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The content interpretation OBU (§ 5.15) of a described stream.

use super::*;
use crate::content::{ChromaSamplePosition, ContentDescription};
use splot_core::annexb::parse_annex_b_obus_partial;
use splot_core::bitio::BitReader;
use splot_core::headers::content_interpretation::{
    ContentInterpretation, parse_content_interpretation,
};
use splot_core::span::ByteOffset;
use splot_core::types::ObuType;

/// BT.2100 PQ at full range, left chroma, a 4:3 sample and 30000/1001 timing.
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
    config
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

/// Every key frame carries the same OBU between its sequence header and its
/// film grain OBU, the inter frames carry none, and the samples do not change.
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
                types[..3],
                [
                    ObuType::TemporalDelimiter,
                    ObuType::SequenceHeader,
                    ObuType::ContentInterpretation
                ]
            );
            assert_eq!([&types[..2], &types[3..]].concat(), plain_types);
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
    assert!(!invalid(|config| {
        config.chroma = PixelFormat::Yuv422;
        config.content.chroma_sample_position = Some(ChromaSamplePosition::Center);
    }));
}

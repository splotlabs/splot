// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Streaming `Decoder` tests: the pictures must match the batch raw output,
//! carry each temporal unit's caller data, and share the decoder's storage.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use splot_core::stream_reader::{StreamUnit, TemporalUnitReader};
use splot_decode::{
    DataProps, DecodeContext, DecodeError, DecodeOptions, DecodeRuntimeConfig, Decoder,
    DecoderSettings, DecoderStateError, FrameDelay, Picture, PictureSamples, PlaneId, SendStatus,
    ThreadCount,
};

const MULTIREF: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-3frame-multiref-64x64.ivf");
const EIGHT_FRAME: &[u8] = include_bytes!(
    "../../../tests/conformance/vectors/valid/syn-8frame-opfl-refine-all-64x64-q120.ivf"
);
const SEF_FAMILIES: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-frame-sef-families-64x64.ivf");
const TIP_FAMILIES: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-frame-tip-families-64x64.ivf");
const BRIDGE: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-bridge-celu-64x64.ivf");
const TEN_BIT_INTER: &[u8] = include_bytes!(
    "../../../tests/conformance/vectors/valid/syn-3frame-deblock-subpu-chroma-64x32-10bit-q90.ivf"
);
const CROP: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-crop-intra-64x64-q80.ivf");

const FIXTURES: &[&[u8]] = &[
    MULTIREF,
    EIGHT_FRAME,
    SEF_FAMILIES,
    TIP_FAMILIES,
    BRIDGE,
    TEN_BIT_INTER,
    CROP,
];

/// The payload of each non-empty IVF frame record: one temporal unit each.
fn units(ivf: &[u8]) -> Vec<Vec<u8>> {
    let mut reader = TemporalUnitReader::new(ivf);
    let mut units = Vec::new();
    while let Some(unit) = reader.next_unit().unwrap() {
        if let StreamUnit::IvfFrame { payload, .. } = unit
            && !payload.is_empty()
        {
            units.push(payload.to_vec());
        }
    }
    units
}

fn decoder(threads: usize, frame_delay: usize) -> Decoder {
    Decoder::new(
        DecoderSettings::default()
            .with_threads(ThreadCount::from(threads))
            .with_frame_delay(FrameDelay::from(frame_delay)),
    )
    .unwrap()
}

/// Sends every unit with `props(index)` and keeps every picture, the way a
/// host's send/receive loop does.
fn decode(
    decoder: &mut Decoder,
    units: &[Vec<u8>],
    props: impl Fn(usize) -> DataProps,
) -> Vec<Picture> {
    let mut pictures = Vec::new();
    for (index, unit) in units.iter().enumerate() {
        while decoder.send_data(unit, props(index)).unwrap() == SendStatus::Full {
            pictures.extend(core::iter::from_fn(|| decoder.get_picture().unwrap()));
        }
        pictures.extend(core::iter::from_fn(|| decoder.get_picture().unwrap()));
    }
    decoder.drain();
    pictures.extend(core::iter::from_fn(|| decoder.get_picture().unwrap()));
    pictures
}

/// The picture's visible samples in the raw output layout: Y, U, V rows, one
/// byte per eight-bit sample, two little-endian bytes per ten-bit sample.
fn raw(picture: &Picture, out: &mut Vec<u8>) {
    for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
        match picture.plane(plane) {
            Some(PictureSamples::Eight(plane)) => {
                plane
                    .visible_rows()
                    .for_each(|row| out.extend_from_slice(row));
            }
            Some(PictureSamples::Ten(plane)) => plane
                .visible_rows()
                .flatten()
                .for_each(|sample| out.extend_from_slice(&sample.to_le_bytes())),
            None => {}
        }
    }
}

fn raw_of(pictures: &[Picture]) -> Vec<u8> {
    let mut out = Vec::new();
    for picture in pictures {
        raw(picture, &mut out);
    }
    out
}

fn batch_raw(bytes: &[u8]) -> Vec<u8> {
    let context = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize))).unwrap();
    let mut out = Vec::new();
    context
        .decode_raw_bytes(bytes, DecodeOptions::default(), &mut out)
        .unwrap();
    out
}

#[test]
fn pictures_match_the_batch_raw_output_at_every_width_and_depth() {
    for fixture in FIXTURES {
        let expected = batch_raw(fixture);
        let units = units(fixture);
        for (threads, frame_delay) in [(1, 1), (2, 2), (4, 1), (4, 4), (8, 8)] {
            let pictures = decode(&mut decoder(threads, frame_delay), &units, |_| {
                DataProps::default()
            });
            let actual = raw_of(&pictures);
            assert!(
                actual == expected,
                "{threads} threads at depth {frame_delay} diverged from the batch output"
            );
        }
    }
}

#[test]
fn each_picture_carries_the_data_of_the_unit_that_coded_or_showed_it() {
    for fixture in [MULTIREF, EIGHT_FRAME, SEF_FAMILIES] {
        let units = units(fixture);
        let props = |index: usize| {
            DataProps::default()
                .with_pts(index as i64 * 1000)
                .with_duration(1000)
                .with_user_data(index as u64)
        };
        let pictures = decode(&mut decoder(4, 4), &units, props);
        assert!(!pictures.is_empty());
        for picture in &pictures {
            let index = usize::try_from(picture.props().user_data).unwrap();
            assert!(index < units.len());
            assert_eq!(picture.props(), props(index));
        }
    }
}

#[test]
fn held_pictures_neither_stall_the_decoder_nor_change_its_output() {
    let units: Vec<Vec<u8>> = (0..8).flat_map(|_| units(EIGHT_FRAME)).collect();
    let expected = raw_of(&decode(&mut decoder(1, 1), &units, |_| {
        DataProps::default()
    }));
    for (threads, frame_delay) in [(1, 1), (4, 4)] {
        let held = decode(&mut decoder(threads, frame_delay), &units, |_| {
            DataProps::default()
        });
        assert!(held.len() > 2 * 16 + frame_delay + 1);
        assert!(
            raw_of(&held) == expected,
            "held pictures changed the output"
        );
    }
}

#[test]
fn a_dropped_picture_gives_its_storage_back_to_the_decoder() {
    let units: Vec<Vec<u8>> = (0..4).flat_map(|_| units(EIGHT_FRAME)).collect();
    let mut decoder = decoder(1, 1);
    let mut seen = Vec::new();
    let mut reused = false;
    for unit in &units {
        assert_eq!(
            decoder.send_data(unit, DataProps::default()).unwrap(),
            SendStatus::Accepted
        );
        while let Some(picture) = decoder.get_picture().unwrap() {
            let Some(PictureSamples::Eight(luma)) = picture.plane(PlaneId::Y) else {
                panic!("an eight-bit luma plane");
            };
            let address = luma.samples().as_ptr() as usize;
            reused |= seen.contains(&address);
            seen.push(address);
        }
    }
    assert!(
        reused,
        "no decoded picture reused a dropped picture's storage"
    );
}

#[test]
fn a_full_decoder_takes_the_same_unit_once_pictures_are_taken() {
    let units = units(EIGHT_FRAME);
    let mut decoder = decoder(1, 1);
    let mut full = 0;
    let mut pictures = 0;
    for unit in &units {
        while decoder.send_data(unit, DataProps::default()).unwrap() == SendStatus::Full {
            full += 1;
            pictures += usize::from(decoder.get_picture().unwrap().is_some());
        }
    }
    decoder.drain();
    while decoder.get_picture().unwrap().is_some() {
        pictures += 1;
    }
    assert!(full > 0);
    assert_eq!(pictures, batch_raw(EIGHT_FRAME).len() / (64 * 64 * 3 / 2));
}

#[test]
fn a_decode_error_is_returned_once_then_the_session_reports_failure() {
    let mut decoder = decoder(2, 2);
    assert_eq!(
        decoder
            .send_data(&[0x01, 0x08, 0x05, 0x10], DataProps::default())
            .unwrap(),
        SendStatus::Accepted
    );
    decoder.drain();
    let error = decoder.get_picture().unwrap_err();
    assert!(!matches!(error, DecodeError::State { .. }), "{error}");
    assert!(matches!(
        decoder.get_picture(),
        Err(DecodeError::State {
            source: DecoderStateError::Failed
        })
    ));
    assert!(matches!(
        decoder.send_data(&[0x01, 0x08], DataProps::default()),
        Err(DecodeError::State { .. })
    ));
}

#[test]
fn a_decoder_dropped_mid_stream_stops_its_driver() {
    let units = units(EIGHT_FRAME);
    for threads in [1, 4] {
        let mut decoder = decoder(threads, 4);
        for unit in units.iter().take(3) {
            let _ = decoder.send_data(unit, DataProps::default()).unwrap();
        }
        drop(decoder);
    }
}

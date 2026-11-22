// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Streaming `Decoder` tests: the pictures must match the batch raw output,
//! carry each temporal unit's caller data, and share the decoder's storage.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use splot_core::stream_reader::{StreamUnit, TemporalUnitReader, is_random_access_point};
use splot_decode::{
    DataProps, DecodeContext, DecodeError, DecodeOptions, DecodeRuntimeConfig, Decoder,
    DecoderSettings, DecoderStateError, FrameDelay, FrameType, Picture, PictureSamples, PlaneId,
    SendStatus, ThreadCount,
};

const MULTIREF: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-3frame-multiref-64x64.ivf");
const EIGHT_FRAME: &[u8] = include_bytes!(
    "../../../tests/conformance/vectors/valid/syn-8frame-opfl-refine-all-64x64-q120.ivf"
);
const POOLED_FRAMES: &[u8] = include_bytes!(
    "../../../tests/conformance/vectors/valid/syn-4frame-inter-skip-chroma-256x128.ivf"
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
const FILM_GRAIN: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-filmgrain-intra-64x64.ivf");

const FIXTURES: &[&[u8]] = &[
    MULTIREF,
    EIGHT_FRAME,
    SEF_FAMILIES,
    TIP_FAMILIES,
    BRIDGE,
    TEN_BIT_INTER,
    CROP,
    FILM_GRAIN,
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
    batch_raw_with(bytes, &DecodeOptions::default())
}

fn batch_raw_with(bytes: &[u8], options: &DecodeOptions) -> Vec<u8> {
    let context = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize))).unwrap();
    let mut out = Vec::new();
    context.decode_raw_bytes(bytes, *options, &mut out).unwrap();
    out
}

/// The address of the picture's first stored luma sample.
fn luma_storage(picture: &Picture) -> usize {
    match picture.plane(PlaneId::Y) {
        Some(PictureSamples::Eight(luma)) => luma.samples().as_ptr() as usize,
        Some(PictureSamples::Ten(luma)) => luma.samples().as_ptr() as usize,
        None => panic!("a luma plane"),
    }
}

fn with_film_grain(pictures: &[Picture]) -> Vec<Picture> {
    pictures
        .iter()
        .map(|picture| picture.apply_film_grain().unwrap())
        .collect()
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
fn skipped_film_grain_is_left_out_and_applied_on_request() {
    let units = units(FILM_GRAIN);
    let grained = batch_raw(FILM_GRAIN);
    let clean = batch_raw_with(
        FILM_GRAIN,
        &DecodeOptions::default().with_skip_film_grain(true),
    );
    assert!(clean != grained, "the fixture's grain changes no sample");
    for threads in [1, 4] {
        let mut decoder = Decoder::new(
            DecoderSettings::default()
                .with_threads(ThreadCount::from(threads))
                .with_skip_film_grain(true),
        )
        .unwrap();
        let pictures = decode(&mut decoder, &units, |_| DataProps::default());
        drop(decoder);
        assert!(pictures.iter().all(Picture::has_film_grain));
        assert!(raw_of(&pictures) == clean);
        let (pictures, applied) = std::thread::spawn(move || {
            let applied = with_film_grain(&pictures);
            (pictures, applied)
        })
        .join()
        .unwrap();
        assert!(applied.iter().all(Picture::has_film_grain));
        assert!(raw_of(&applied) == grained);
        assert!(raw_of(&pictures) == clean, "the grain changed its source");
        assert!(raw_of(&with_film_grain(&applied)) == grained);
    }
}

#[test]
fn a_picture_with_nothing_to_apply_shares_its_samples() {
    for (fixture, has_film_grain) in [(FILM_GRAIN, true), (MULTIREF, false)] {
        let pictures = decode(&mut decoder(1, 1), &units(fixture), |_| {
            DataProps::default()
        });
        assert!(!pictures.is_empty());
        for (picture, applied) in pictures.iter().zip(with_film_grain(&pictures)) {
            assert_eq!(picture.has_film_grain(), has_film_grain);
            assert_eq!(applied.has_film_grain(), has_film_grain);
            assert_eq!(luma_storage(picture), luma_storage(&applied));
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
    let units: Vec<Vec<u8>> = (0..8).flat_map(|_| units(POOLED_FRAMES)).collect();
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

/// Sends every unit and takes pictures only with `wait_picture`, never
/// draining before the end. Returns the pictures and how many were out after
/// each unit.
fn decode_waiting(decoder: &mut Decoder, units: &[Vec<u8>]) -> (Vec<Picture>, Vec<usize>) {
    let mut pictures = Vec::new();
    let mut counts = Vec::new();
    for unit in units {
        while decoder.send_data(unit, DataProps::default()).unwrap() == SendStatus::Full {
            pictures.extend(decoder.wait_picture().unwrap());
        }
        pictures.extend(core::iter::from_fn(|| decoder.wait_picture().unwrap()));
        counts.push(pictures.len());
    }
    decoder.drain();
    pictures.extend(core::iter::from_fn(|| decoder.wait_picture().unwrap()));
    (pictures, counts)
}

#[test]
fn wait_picture_gives_the_pictures_of_the_units_sent_without_more_input() {
    for fixture in FIXTURES {
        let expected = batch_raw(fixture);
        let units = units(fixture);
        let (_, serial) = decode_waiting(&mut decoder(1, 1), &units);
        for (threads, frame_delay) in [(2, 2), (4, 1), (4, 4), (8, 8)] {
            let (pictures, counts) = decode_waiting(&mut decoder(threads, frame_delay), &units);
            assert_eq!(raw_of(&with_film_grain(&pictures)), expected);
            assert_eq!(
                counts, serial,
                "threads={threads} frame_delay={frame_delay}"
            );
        }
        assert!(serial[0] >= 1, "the key frame is out at once");
    }
}

#[test]
fn flush_starts_a_new_session_mid_stream_and_after_an_error() {
    let units = units(EIGHT_FRAME);
    let expected = batch_raw(EIGHT_FRAME);
    for (threads, frame_delay) in [(1, 1), (4, 4)] {
        let mut decoder = decoder(threads, frame_delay);
        for unit in &units[..4] {
            while decoder.send_data(unit, DataProps::default()).unwrap() == SendStatus::Full {
                decoder.get_picture().unwrap();
            }
        }
        decoder.flush();
        assert!(decoder.wait_picture().unwrap().is_none());
        assert_eq!(
            raw_of(&decode(&mut decoder, &units, |_| DataProps::default())),
            expected
        );

        decoder.flush();
        let _ = decoder.send_data(&units[1], DataProps::default()).unwrap();
        assert!(decoder.wait_picture().is_err());
        decoder.flush();
        assert_eq!(
            raw_of(&decode(&mut decoder, &units, |_| DataProps::default())),
            expected
        );
    }
}

#[test]
fn a_decoder_starts_at_exactly_the_random_access_points() {
    for fixture in FIXTURES {
        let units = units(fixture);
        assert!(is_random_access_point(&units[0]));
        for unit in &units {
            let mut decoder = decoder(1, 1);
            let _ = decoder.send_data(unit, DataProps::default()).unwrap();
            assert_eq!(decoder.wait_picture().is_ok(), is_random_access_point(unit));
        }
    }
    assert!(!is_random_access_point(&[]));
    assert!(!is_random_access_point(&[0xFF; 8]));
}

#[test]
fn a_picture_reports_the_type_of_the_frame_it_shows() {
    let pictures = decode(&mut decoder(1, 1), &units(MULTIREF), |_| {
        DataProps::default()
    });
    let types: Vec<_> = pictures.iter().map(Picture::frame_type).collect();
    assert_eq!(types[0], Some(FrameType::Key));
    assert!(
        types[1..]
            .iter()
            .all(|&kind| kind == Some(FrameType::Inter))
    );
}

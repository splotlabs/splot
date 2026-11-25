// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Streaming `Decoder` tests: the pictures must match the batch raw output,
//! carry each temporal unit's caller data, and share the decoder's storage.

#![expect(clippy::unwrap_used, clippy::panic)]

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
const BRU_INACTIVE: &[u8] = include_bytes!(
    "../../../tests/conformance/vectors/valid/syn-bru-inactive-frame-128x64-q220.ivf"
);
const BRU_INACTIVE_REFRESH_8BIT: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-bru-inactive-refresh-8bit.ivf");
const BRU_INACTIVE_REFRESH_10BIT: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-bru-inactive-refresh-10bit.ivf");
const BRU_INACTIVE_FILMGRAIN_8BIT: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-bru-inactive-filmgrain-8bit.ivf");
const BRU_INTER_CONTROL: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-bru-inter-128x64-q220.ivf");
const BRU_MULTITILE_CONTROL: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-bru-multitile-256x128-q220.ivf");
const BRU_LR_WIENERNS_8BIT: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-bru-lr-stripe-wienerns-8bit.ivf");
const BRU_LR_WIENERNS_10BIT: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-bru-lr-stripe-wienerns-10bit.ivf");
const BRU_LR_PCWIENER_8BIT: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-bru-lr-stripe-pcwiener-8bit.ivf");
const BRU_LR_PCWIENER_10BIT: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-bru-lr-stripe-pcwiener-10bit.ivf");
const BRU_LR_MULTITILE_CROSS_8BIT: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-bru-lr-multitile-cross-8bit.ivf");
const BRU_LR_MULTITILE_ISOLATED_8BIT: &[u8] = include_bytes!(
    "../../../tests/conformance/vectors/valid/syn-bru-lr-multitile-isolated-8bit.ivf"
);
const BRU_GDF_BLOCK_8BIT: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-bru-gdf-block-8bit.ivf");
const BRU_GDF_BLOCK_10BIT: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-bru-gdf-block-10bit.ivf");
const BRU_GDF_FRAME_8BIT: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-bru-gdf-frame-8bit.ivf");
const BRU_GDF_FRAME_10BIT: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-bru-gdf-frame-10bit.ivf");
const BRU_GDF_LR_10BIT: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-bru-gdf-lr-10bit.ivf");
const BRU_GDF_LR_8BIT: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-bru-gdf-lr-8bit.ivf");
const BRU_GDF_MULTITILE_8BIT: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-bru-gdf-multitile-8bit.ivf");
const BRU_LOCAL_INTRABC_8BIT: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-bru-local-intrabc-8bit.ivf");
const BRU_LOCAL_INTRABC_10BIT: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-bru-local-intrabc-10bit.ivf");
const BRU_LOCAL_INTRABC_MULTITILE_8BIT: &[u8] = include_bytes!(
    "../../../tests/conformance/vectors/valid/syn-bru-local-intrabc-multitile-8bit.ivf"
);
const BRU_LOCAL_INTRABC_LR_8BIT: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-bru-local-intrabc-lr-8bit.ivf");
const BRU_LOCAL_INTRABC_GDF_8BIT: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-bru-local-intrabc-gdf-8bit.ivf");

const FIXTURES: &[&[u8]] = &[
    MULTIREF,
    EIGHT_FRAME,
    SEF_FAMILIES,
    TIP_FAMILIES,
    BRIDGE,
    TEN_BIT_INTER,
    CROP,
    FILM_GRAIN,
    BRU_INACTIVE,
    BRU_INACTIVE_REFRESH_8BIT,
    BRU_INACTIVE_REFRESH_10BIT,
    BRU_INACTIVE_FILMGRAIN_8BIT,
    BRU_INTER_CONTROL,
    BRU_MULTITILE_CONTROL,
    BRU_LR_WIENERNS_8BIT,
    BRU_LR_WIENERNS_10BIT,
    BRU_LR_PCWIENER_8BIT,
    BRU_LR_PCWIENER_10BIT,
    BRU_LR_MULTITILE_CROSS_8BIT,
    BRU_LR_MULTITILE_ISOLATED_8BIT,
    BRU_GDF_BLOCK_8BIT,
    BRU_GDF_BLOCK_10BIT,
    BRU_GDF_LR_8BIT,
    BRU_GDF_MULTITILE_8BIT,
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

#[test]
fn decoder_validates_simd_policy_once_and_retains_it_across_flush() {
    use splot_decode::SelectedBackend;

    #[cfg(target_arch = "x86_64")]
    {
        use splot_decode::SimdPolicy;

        let err = Decoder::new(DecoderSettings::default().with_simd_policy(SimdPolicy::Neon))
            .unwrap_err();
        assert!(
            matches!(err, DecodeError::UnsupportedBackend { .. }),
            "{err}"
        );

        if !splot_core::simd::detect_capabilities().has_avx512() {
            let err = Decoder::new(DecoderSettings::default().with_simd_policy(SimdPolicy::Avx512))
                .unwrap_err();
            assert!(
                matches!(err, DecodeError::UnsupportedBackend { .. }),
                "{err}"
            );
        }
    }

    let expected_units = units(MULTIREF);
    let expected = batch_raw(MULTIREF);
    let mut decoder = Decoder::new(DecoderSettings::default()).unwrap();
    let initial = decoder.selected_backend();
    #[cfg(target_arch = "x86_64")]
    assert_eq!(
        initial,
        if splot_core::simd::detect_capabilities().has_avx512() {
            SelectedBackend::Avx512
        } else if splot_core::simd::detect_capabilities().has_avx2() {
            SelectedBackend::Avx2
        } else {
            SelectedBackend::Sse2
        }
    );
    #[cfg(target_arch = "aarch64")]
    assert_eq!(initial, SelectedBackend::Neon);

    let first = decode(&mut decoder, &expected_units, |_| DataProps::default());
    assert_eq!(raw_of(&with_film_grain(&first)), expected);
    assert_eq!(decoder.selected_backend(), initial);

    decoder.flush();
    assert_eq!(decoder.selected_backend(), initial);
    let second = decode(&mut decoder, &expected_units, |_| DataProps::default());
    assert_eq!(raw_of(&with_film_grain(&second)), expected);
    assert_eq!(decoder.selected_backend(), initial);
}

struct ChunkedSliceReader<'a> {
    bytes: &'a [u8],
    offset: usize,
    pattern: &'a [usize],
    step: usize,
}

impl std::io::Read for ChunkedSliceReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.offset >= self.bytes.len() || buf.is_empty() {
            return Ok(0);
        }
        let max_chunk = self.pattern[self.step % self.pattern.len()];
        self.step += 1;
        let count = (self.bytes.len() - self.offset)
            .min(buf.len())
            .min(max_chunk);
        buf[..count].copy_from_slice(&self.bytes[self.offset..self.offset + count]);
        self.offset += count;
        Ok(count)
    }
}

#[test]
fn bru_inactive_and_refresh_vectors_match_batch_and_chunked_streaming() {
    let bru_fixtures = [
        (
            "syn-bru-inactive-frame-128x64-q220",
            BRU_INACTIVE,
            5usize,
            Some((2usize, 4usize)),
        ),
        (
            "syn-bru-inactive-refresh-8bit",
            BRU_INACTIVE_REFRESH_8BIT,
            8,
            Some((3, 5)),
        ),
        (
            "syn-bru-inactive-refresh-10bit",
            BRU_INACTIVE_REFRESH_10BIT,
            8,
            Some((3, 5)),
        ),
        (
            "syn-bru-inactive-block-use-8bit",
            include_bytes!(
                "../../../tests/conformance/vectors/valid/syn-bru-inactive-block-use-8bit.ivf"
            ),
            8,
            Some((3, 5)),
        ),
        (
            "syn-bru-inactive-block-use-10bit",
            include_bytes!(
                "../../../tests/conformance/vectors/valid/syn-bru-inactive-block-use-10bit.ivf"
            ),
            8,
            Some((3, 5)),
        ),
        ("syn-bru-inter-128x64-q220", BRU_INTER_CONTROL, 5, None),
        (
            "syn-bru-multitile-256x128-q220",
            BRU_MULTITILE_CONTROL,
            6,
            None,
        ),
    ];

    for (name, bytes, expected_frames, inactive_pair) in bru_fixtures {
        let units = units(bytes);
        let expected = batch_raw(bytes);
        let frame_bytes = expected.len() / expected_frames;
        assert_eq!(frame_bytes * expected_frames, expected.len(), "{name}");
        if let Some((src_idx, inactive_idx)) = inactive_pair {
            let src = &expected[src_idx * frame_bytes..(src_idx + 1) * frame_bytes];
            let inactive = &expected[inactive_idx * frame_bytes..(inactive_idx + 1) * frame_bytes];
            assert_eq!(
                inactive, src,
                "{name}: bru_inactive frame {inactive_idx} must copy frame {src_idx}"
            );
            if expected_frames > inactive_idx + 1 {
                let next =
                    &expected[(inactive_idx + 1) * frame_bytes..(inactive_idx + 2) * frame_bytes];
                assert_ne!(
                    next, inactive,
                    "{name}: subsequent inter frame must differ from inactive"
                );
            }
        }

        for threads in [1, 3, 12] {
            let context =
                DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(threads))).unwrap();
            let mut batch_out = Vec::new();
            context
                .decode_raw_bytes(bytes, DecodeOptions::default(), &mut batch_out)
                .unwrap();
            assert_eq!(batch_out, expected, "{name} batch at {threads} threads");

            let mut chunked_out = Vec::new();
            let reader = ChunkedSliceReader {
                bytes,
                offset: 0,
                pattern: &[1, 3, 7, 13, 2],
                step: 0,
            };
            context
                .decode_raw_reader(reader, DecodeOptions::default(), &mut chunked_out)
                .unwrap();
            assert_eq!(chunked_out, expected, "{name} chunked at {threads} threads");

            let mut dec = decoder(threads, threads.min(4));
            let pics = decode(&mut dec, &units, |_| DataProps::default());
            assert_eq!(pics.len(), expected_frames, "{name}");
            assert_eq!(
                raw_of(&pics),
                expected,
                "{name} stream at {threads} threads"
            );

            dec.flush();
            dec.drain();
            assert!(dec.get_picture().unwrap().is_none());
            dec.flush();
            let pics_again = decode(&mut dec, &units, |_| DataProps::default());
            assert_eq!(
                raw_of(&pics_again),
                expected,
                "{name} repeated flush/drain at {threads} threads"
            );
        }
    }
}

#[test]
fn bru_inactive_film_grain_does_not_mutate_reference_or_double_apply() {
    let pre_grain_expected = batch_raw(BRU_INACTIVE_REFRESH_8BIT);
    let grained_expected = batch_raw(BRU_INACTIVE_FILMGRAIN_8BIT);
    let skipped = batch_raw_with(
        BRU_INACTIVE_FILMGRAIN_8BIT,
        &DecodeOptions::default().with_skip_film_grain(true),
    );
    assert_eq!(
        skipped, pre_grain_expected,
        "film-grain bru_inactive fixture pre-grain output must equal non-grain fixture"
    );
    assert_ne!(grained_expected, skipped);

    let frame_bytes = skipped.len() / 8;
    assert_eq!(frame_bytes * 8, skipped.len());
    assert_eq!(
        &skipped[5 * frame_bytes..6 * frame_bytes],
        &skipped[3 * frame_bytes..4 * frame_bytes],
        "pre-grain bru_inactive frame must still match its reference frame"
    );
    assert_ne!(
        &grained_expected[5 * frame_bytes..6 * frame_bytes],
        &skipped[5 * frame_bytes..6 * frame_bytes],
        "bru_inactive frame must apply its own output film grain"
    );

    let units = units(BRU_INACTIVE_FILMGRAIN_8BIT);
    for threads in [1, 3, 12] {
        let mut clean_dec = Decoder::new(
            DecoderSettings::default()
                .with_threads(ThreadCount::from(threads))
                .with_skip_film_grain(true),
        )
        .unwrap();
        let clean_pics = decode(&mut clean_dec, &units, |_| DataProps::default());
        assert_eq!(raw_of(&clean_pics), skipped);
        assert!(clean_pics.iter().all(Picture::has_film_grain));

        let applied_pics = with_film_grain(&clean_pics);
        assert_eq!(raw_of(&applied_pics), grained_expected);
        assert_eq!(
            raw_of(&clean_pics),
            skipped,
            "applying grain to emitted bru_inactive picture mutated its source samples"
        );
        assert_eq!(
            raw_of(&with_film_grain(&applied_pics)),
            grained_expected,
            "re-applying grain double-applied grain"
        );

        let mut grained_dec = decoder(threads, threads.min(4));
        let grained_pics = decode(&mut grained_dec, &units, |_| DataProps::default());
        assert_eq!(raw_of(&grained_pics), grained_expected);
    }
}

fn stream_decode_until_error(decoder: &mut Decoder, units: &[Vec<u8>]) -> bool {
    for (index, unit) in units.iter().enumerate() {
        let mut props = DataProps::default();
        props.pts = i64::try_from(index).unwrap_or_default();
        loop {
            match decoder.send_data(unit, props) {
                Ok(SendStatus::Full) => while let Ok(Some(_)) = decoder.get_picture() {},
                Ok(SendStatus::Accepted) => break,
                Err(_) => return true,
            }
            if decoder.get_picture().is_err() {
                return true;
            }
        }
    }
    decoder.drain();
    loop {
        match decoder.get_picture() {
            Ok(Some(_)) => {}
            Ok(None) => return false,
            Err(_) => return true,
        }
    }
}

fn assert_bru_batch_chunked_streaming(fixtures: &[(&str, &[u8])], kind: &str) {
    for &(name, bytes) in fixtures {
        let stream_units = units(bytes);
        let expected = batch_raw(bytes);
        for threads in [1, 3, 12] {
            let context =
                DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(threads))).unwrap();
            let mut batch_out = Vec::new();
            context
                .decode_raw_bytes(bytes, DecodeOptions::default(), &mut batch_out)
                .unwrap();
            assert_eq!(batch_out, expected, "{name} batch at {threads} threads");

            let mut chunked_out = Vec::new();
            let reader = ChunkedSliceReader {
                bytes,
                offset: 0,
                pattern: &[1, 3, 7, 13, 2],
                step: 0,
            };
            context
                .decode_raw_reader(reader, DecodeOptions::default(), &mut chunked_out)
                .unwrap();
            assert_eq!(chunked_out, expected, "{name} chunked at {threads} threads");

            let mut dec = decoder(threads, threads.min(4));
            let pics = decode(&mut dec, &stream_units, |_| DataProps::default());
            assert_eq!(pics.len(), 6, "{name}");
            assert_eq!(
                raw_of(&pics),
                expected,
                "{name} stream at {threads} threads"
            );

            dec.flush();
            dec.drain();
            assert!(dec.get_picture().unwrap().is_none());
            dec.flush();
            let pics_again = decode(&mut dec, &stream_units, |_| DataProps::default());
            assert_eq!(
                raw_of(&pics_again),
                expected,
                "{name} ({kind}) repeated flush/drain at {threads} threads"
            );
        }
    }
}

fn assert_bru_flush_recovery(cases: &[(&str, &[u8])], kind: &str) {
    for &(name, bytes) in cases {
        let mut corrupted_units = units(bytes);
        let cut = corrupted_units[3].len() / 2;
        corrupted_units[3].truncate(cut.max(4));
        for threads in [1, 3] {
            let mut dec = decoder(threads, threads.min(4));
            assert!(
                stream_decode_until_error(&mut dec, &corrupted_units),
                "{name}: truncated frame 3 {kind} payload must error at {threads} threads"
            );
            dec.flush();
            dec.drain();
            while matches!(dec.get_picture(), Ok(Some(_)) | Err(_)) {}
            dec.flush();
            let intact_units = units(bytes);
            let recovered = decode(&mut dec, &intact_units, |_| DataProps::default());
            assert_eq!(
                raw_of(&recovered),
                batch_raw(bytes),
                "{name}: decoder must recover cleanly after flush at {threads} threads"
            );
        }
    }
}

#[test]
fn bru_lr_vectors_match_batch_and_chunked_streaming_and_recover_on_flush() {
    let bru_lr_fixtures = [
        ("syn-bru-lr-stripe-wienerns-8bit", BRU_LR_WIENERNS_8BIT),
        ("syn-bru-lr-stripe-wienerns-10bit", BRU_LR_WIENERNS_10BIT),
        ("syn-bru-lr-stripe-pcwiener-8bit", BRU_LR_PCWIENER_8BIT),
        ("syn-bru-lr-stripe-pcwiener-10bit", BRU_LR_PCWIENER_10BIT),
        (
            "syn-bru-lr-multitile-cross-8bit",
            BRU_LR_MULTITILE_CROSS_8BIT,
        ),
        (
            "syn-bru-lr-multitile-isolated-8bit",
            BRU_LR_MULTITILE_ISOLATED_8BIT,
        ),
    ];

    assert_ne!(
        batch_raw(BRU_LR_MULTITILE_CROSS_8BIT),
        batch_raw(BRU_LR_MULTITILE_ISOLATED_8BIT),
        "multitile cross-tile and isolated BRU+LR streams must produce distinct pixels"
    );

    assert_bru_batch_chunked_streaming(&bru_lr_fixtures, "BRU+LR");
    assert_bru_flush_recovery(
        &[
            ("pcwiener-truncated", BRU_LR_PCWIENER_8BIT),
            ("wienerns-truncated", BRU_LR_WIENERNS_8BIT),
        ],
        "BRU+LR",
    );
}

#[test]
fn bru_gdf_vectors_match_batch_and_chunked_streaming_and_recover_on_flush() {
    let bru_gdf_fixtures = [
        ("syn-bru-gdf-block-8bit", BRU_GDF_BLOCK_8BIT),
        ("syn-bru-gdf-block-10bit", BRU_GDF_BLOCK_10BIT),
        ("syn-bru-gdf-frame-8bit", BRU_GDF_FRAME_8BIT),
        ("syn-bru-gdf-frame-10bit", BRU_GDF_FRAME_10BIT),
        ("syn-bru-gdf-lr-10bit", BRU_GDF_LR_10BIT),
        ("syn-bru-gdf-lr-8bit", BRU_GDF_LR_8BIT),
        ("syn-bru-gdf-multitile-8bit", BRU_GDF_MULTITILE_8BIT),
    ];

    assert_bru_batch_chunked_streaming(&bru_gdf_fixtures, "BRU+GDF");
    assert_bru_flush_recovery(
        &[
            ("gdf-block-8bit-truncated", BRU_GDF_BLOCK_8BIT),
            ("gdf-block-10bit-truncated", BRU_GDF_BLOCK_10BIT),
            ("gdf-lr-8bit-truncated", BRU_GDF_LR_8BIT),
        ],
        "BRU+GDF",
    );
}

#[test]
fn bru_local_intrabc_vectors_match_batch_and_chunked_streaming_and_recover_on_flush() {
    let bru_intrabc_fixtures = [
        ("syn-bru-local-intrabc-8bit", BRU_LOCAL_INTRABC_8BIT),
        ("syn-bru-local-intrabc-10bit", BRU_LOCAL_INTRABC_10BIT),
        (
            "syn-bru-local-intrabc-multitile-8bit",
            BRU_LOCAL_INTRABC_MULTITILE_8BIT,
        ),
        ("syn-bru-local-intrabc-lr-8bit", BRU_LOCAL_INTRABC_LR_8BIT),
        ("syn-bru-local-intrabc-gdf-8bit", BRU_LOCAL_INTRABC_GDF_8BIT),
    ];

    assert_bru_batch_chunked_streaming(&bru_intrabc_fixtures, "BRU+local-IntraBC");
    assert_bru_flush_recovery(
        &[
            ("local-intrabc-8bit-truncated", BRU_LOCAL_INTRABC_8BIT),
            ("local-intrabc-10bit-truncated", BRU_LOCAL_INTRABC_10BIT),
            (
                "local-intrabc-multitile-truncated",
                BRU_LOCAL_INTRABC_MULTITILE_8BIT,
            ),
            ("local-intrabc-lr-truncated", BRU_LOCAL_INTRABC_LR_8BIT),
            ("local-intrabc-gdf-truncated", BRU_LOCAL_INTRABC_GDF_8BIT),
        ],
        "BRU+local-IntraBC",
    );
}

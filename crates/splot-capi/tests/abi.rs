// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Drives the C ABI the way a C host does and checks the pictures against
//! the batch decoder's raw output.

#![allow(unsafe_code)]
#![expect(clippy::unwrap_used, clippy::panic)]

use core::ptr;

use splot_capi::*;
use splot_core::stream_reader::TemporalUnitReader;
use splot_decode::{DecodeContext, DecodeOptions, DecodeRuntimeConfig, ThreadCount};

const FIXTURES: &[&[u8]] = &[
    include_bytes!(
        "../../../tests/conformance/vectors/valid/syn-8frame-opfl-refine-all-64x64-q120.ivf"
    ),
    include_bytes!("../../../tests/conformance/vectors/valid/syn-frame-sef-families-64x64.ivf"),
    include_bytes!(
        "../../../tests/conformance/vectors/valid/syn-3frame-deblock-subpu-chroma-64x32-10bit-q90.ivf"
    ),
    include_bytes!("../../../tests/conformance/vectors/valid/syn-crop-intra-64x64-q80.ivf"),
];

const ATLAS: &[u8] = include_bytes!(
    "../../../tests/conformance/vectors/needs-external-hls/syn-atlas-global-local-id-scope-8bit.obu"
);

const FILM_GRAIN: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-filmgrain-intra-64x64.ivf");

fn units(stream: &[u8]) -> Vec<Vec<u8>> {
    let mut reader = TemporalUnitReader::new(stream);
    let mut units = Vec::new();
    while let Some(unit) = reader.next_temporal_unit().unwrap() {
        units.push(unit.bytes.to_vec());
    }
    units
}

/// Appends the picture's visible rows in the raw output layout.
fn append(picture: &SplotPicture, out: &mut Vec<u8>) {
    let bytes = (picture.bit_depth as usize).div_ceil(8);
    let (shift_x, shift_y) = match picture.layout {
        SplotPixelLayout::I420 => (1, 1),
        SplotPixelLayout::I422 => (1, 0),
        SplotPixelLayout::I400 | SplotPixelLayout::I444 => (0, 0),
    };
    for plane in 0..3 {
        if picture.data[plane].is_null() {
            continue;
        }
        let (shift_x, shift_y) = if plane == 0 {
            (0, 0)
        } else {
            (shift_x, shift_y)
        };
        let width = (picture.width as usize).div_ceil(1 << shift_x) * bytes;
        let height = (picture.height as usize).div_ceil(1 << shift_y);
        let top = (picture.crop_top >> shift_y) as isize;
        let left = (picture.crop_left >> shift_x) as usize * bytes;
        assert_eq!(picture.data[plane] as usize % 64, 0);
        assert_eq!(picture.stride[plane] % 64, 0);
        for row in 0..height {
            let row = unsafe {
                core::slice::from_raw_parts(
                    picture.data[plane]
                        .cast::<u8>()
                        .offset((top + row as isize) * picture.stride[plane])
                        .add(left),
                    width,
                )
            };
            out.extend_from_slice(row);
        }
    }
}

/// Decode the stream through the C ABI. The pictures survive decoder flush and close.
fn pictures(stream: &[u8], threads: u32, skip_film_grain: u8) -> Vec<SplotPicture> {
    let mut settings = SplotSettings {
        n_threads: 0,
        max_frame_delay: 0,
        skip_film_grain: 1,
        temporal_output_mask: 1,
        embedded_output_mask: 1,
        reserved0: 1,
        extended_output_mask: 1,
        operating_point_scope: 1,
        operating_point_xlayer_id: 1,
        operating_point_ops_id: 1,
        operating_point_index: 1,
        reserved: [1; 20],
    };
    let mut decoder = ptr::null_mut();
    unsafe {
        splot_default_settings(&raw mut settings);
        settings.n_threads = threads;
        settings.skip_film_grain = skip_film_grain;
        assert_eq!(splot_open(&raw mut decoder, &raw const settings), SPLOT_OK);
        let mut pictures = Vec::new();
        let mut take = |decoder| loop {
            let mut picture = core::mem::zeroed::<SplotPicture>();
            match splot_get_picture(decoder, &raw mut picture) {
                SPLOT_OK => pictures.push(picture),
                SPLOT_EAGAIN | SPLOT_EOF => break,
                error => panic!("splot_get_picture failed: {error}"),
            }
        };
        for (index, unit) in units(stream).iter().enumerate() {
            let props = SplotDataProps {
                pts: index as i64 * 100,
                duration: 100,
                user_data: index as u64,
            };
            loop {
                match splot_send_data(decoder, unit.as_ptr(), unit.len(), &raw const props) {
                    SPLOT_OK => break,
                    SPLOT_EAGAIN => take(decoder),
                    error => panic!("splot_send_data failed: {error}"),
                }
            }
            take(decoder);
        }
        splot_drain(decoder);
        take(decoder);
        let mut eof = core::mem::zeroed::<SplotPicture>();
        assert_eq!(splot_get_picture(decoder, &raw mut eof), SPLOT_EOF);
        splot_flush(decoder);
        splot_close(&raw mut decoder);
        assert!(decoder.is_null());
        pictures
    }
}

/// Decodes `ivf` through the C ABI, keeping every picture until the end.
fn decode(ivf: &[u8], threads: u32) -> (Vec<u8>, Vec<SplotDataProps>) {
    let mut raw = Vec::new();
    let mut props = Vec::new();
    for picture in pictures(ivf, threads, 0) {
        append(&picture, &mut raw);
        props.push(picture.props);
        assert_eq!(picture.has_film_grain, 0);
        assert_eq!(picture.atlas_count, 0);
        assert!(picture.atlas.is_null());
        inspect_and_release(picture, |_| {});
    }
    (raw, props)
}

fn inspect_and_release(
    mut picture: SplotPicture,
    inspect: impl FnOnce(Option<&SplotPictureAtlas>),
) {
    unsafe {
        inspect(picture.atlas.as_ref());
        splot_picture_unref(&raw mut picture);
    }
    assert!(picture.internal.is_null() && picture.data[0].is_null() && picture.atlas.is_null());
    assert_eq!(picture.atlas_count, 0);
}

fn batch_raw(ivf: &[u8], skip_film_grain: bool) -> Vec<u8> {
    let context = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize))).unwrap();
    let options = DecodeOptions::default().with_skip_film_grain(skip_film_grain);
    let mut raw = Vec::new();
    context.decode_raw_bytes(ivf, options, &mut raw).unwrap();
    raw
}

#[test]
fn pictures_from_the_c_abi_match_the_batch_decoder() {
    for fixture in FIXTURES {
        let expected = batch_raw(fixture, false);
        let unit_count = units(fixture).len() as u64;
        for threads in [1, 4] {
            let (raw, props) = decode(fixture, threads);
            assert!(raw == expected, "{threads}-thread C ABI output diverged");
            assert!(props.iter().all(|props| props.user_data < unit_count
                && props.pts == props.user_data as i64 * 100
                && props.duration == 100));
        }
    }
}

#[test]
fn skipped_film_grain_is_left_out_and_applied_on_request() {
    let grained = batch_raw(FILM_GRAIN, false);
    let clean = batch_raw(FILM_GRAIN, true);
    assert!(clean != grained);
    for (skip_film_grain, expected) in [(0, &grained), (1, &clean)] {
        let (mut decoded, mut applied) = (Vec::new(), Vec::new());
        for mut picture in pictures(FILM_GRAIN, 1, skip_film_grain) {
            assert_eq!(picture.has_film_grain, 1);
            append(&picture, &mut decoded);
            unsafe {
                assert_eq!(
                    splot_apply_grain(&raw mut picture, &raw const picture),
                    SPLOT_EINVAL
                );
                let mut with_grain = core::mem::zeroed::<SplotPicture>();
                assert_eq!(
                    splot_apply_grain(&raw mut with_grain, &raw const picture),
                    SPLOT_OK
                );
                splot_picture_unref(&raw mut picture);
                assert_eq!(with_grain.has_film_grain, 1);
                append(&with_grain, &mut applied);
                splot_picture_unref(&raw mut with_grain);
            }
        }
        assert!(&decoded == expected, "skip_film_grain={skip_film_grain}");
        assert!(applied == grained, "skip_film_grain={skip_film_grain}");
    }
}

#[test]
fn retained_atlas_metadata_survives_layout_replacement_flush_and_close() {
    let pictures = pictures(ATLAS, 3, 0);
    assert_eq!(pictures.len(), 8);
    for (n, picture) in pictures.into_iter().enumerate() {
        let xlayer = if n % 2 == 0 { 3 } else { 7 };
        let local = n >= 4 && xlayer == 3;
        assert_eq!(picture.extended_layer_id, xlayer);
        assert_eq!(picture.atlas_count, 1 + usize::from(local));
        inspect_and_release(picture, |snapshot| {
            let mut info = SplotAtlasInfo::default();
            let mut segment = SplotAtlasSegment::default();
            assert_eq!(splot_atlas_info(snapshot, 0, Some(&mut info)), SPLOT_OK);
            assert!(info.has_layout && !info.external && info.explicit_segment_ids);
            assert_eq!(
                (info.scope_xlayer_id, info.atlas_id, info.segment_count),
                (31, 5, 2)
            );
            assert_eq!(info.atlas_offset, if n < 4 { 37 } else { 1899 });
            assert_eq!(info.mode, if n < 4 { 3 } else { 4 });
            assert!(info.segment_id.present);
            assert_eq!(info.segment_id.value, if xlayer == 3 { 41 } else { 77 });
            assert_eq!(
                splot_atlas_segment(snapshot, 0, usize::from(xlayer == 7), Some(&mut segment)),
                SPLOT_OK
            );
            assert!(segment.input_stream_id.present);
            assert_eq!(segment.input_stream_id.value, xlayer);
            assert_eq!(
                splot_atlas_segment(snapshot, 0, 2, Some(&mut segment)),
                SPLOT_EINVAL
            );
            if local {
                assert_eq!(splot_atlas_info(snapshot, 1, Some(&mut info)), SPLOT_OK);
                assert!(info.has_layout && info.segment_id.present);
                assert_eq!((info.scope_xlayer_id, info.atlas_id, info.mode), (3, 5, 2));
                assert_eq!((info.atlas_offset, info.segment_id.value), (1932, 41));
            }
            assert_eq!(splot_atlas_info(snapshot, 2, Some(&mut info)), SPLOT_EINVAL);
        });
    }
    assert_eq!(
        splot_atlas_info(None, 0, Some(&mut SplotAtlasInfo::default())),
        SPLOT_EINVAL
    );
    assert_eq!(splot_atlas_info(None, 0, None), SPLOT_EINVAL);
}

/// `skip_film_grain` and `has_film_grain` each took the first reserved byte:
/// `reserved` was 32 bytes in the settings and 4 `usize` in the picture.
#[test]
fn the_new_fields_keep_the_struct_layouts() {
    use core::mem::offset_of;
    assert_eq!(size_of::<SplotSettings>(), 40);
    assert_eq!(offset_of!(SplotSettings, skip_film_grain), 8);
    assert_eq!(offset_of!(SplotSettings, temporal_output_mask), 9);
    assert_eq!(offset_of!(SplotSettings, embedded_output_mask), 10);
    assert_eq!(offset_of!(SplotSettings, reserved0), 11);
    assert_eq!(offset_of!(SplotSettings, extended_output_mask), 12);
    assert_eq!(offset_of!(SplotSettings, operating_point_scope), 16);
    assert_eq!(offset_of!(SplotSettings, operating_point_xlayer_id), 17);
    assert_eq!(offset_of!(SplotSettings, operating_point_ops_id), 18);
    assert_eq!(offset_of!(SplotSettings, operating_point_index), 19);
    assert_eq!(offset_of!(SplotSettings, reserved), 20);
    assert_eq!(
        offset_of!(SplotPicture, atlas_count),
        offset_of!(SplotPicture, atlas) + size_of::<usize>()
    );
    assert_eq!(
        offset_of!(SplotPicture, internal),
        offset_of!(SplotPicture, atlas) + 3 * size_of::<usize>()
    );
    assert_eq!(
        size_of::<SplotPicture>(),
        offset_of!(SplotPicture, internal) + size_of::<usize>()
    );
    assert_eq!(
        offset_of!(SplotPicture, internal),
        offset_of!(SplotPicture, has_film_grain) + 4 * size_of::<usize>()
    );
    assert_eq!(
        offset_of!(SplotPicture, frame_type),
        offset_of!(SplotPicture, has_film_grain) + 1
    );
    assert_eq!(
        offset_of!(SplotPicture, temporal_layer_id),
        offset_of!(SplotPicture, has_film_grain) + 2
    );
    assert_eq!(
        offset_of!(SplotPicture, embedded_layer_id),
        offset_of!(SplotPicture, has_film_grain) + 3
    );
    assert_eq!(
        offset_of!(SplotPicture, extended_layer_id),
        offset_of!(SplotPicture, has_film_grain) + 4
    );
}

/// Sends every unit and takes pictures with `splot_wait_picture` only, then
/// flushes and decodes the stream again on the same decoder.
#[test]
fn wait_picture_needs_no_more_input_and_flush_starts_again() {
    let expected = batch_raw(FIXTURES[0], false);
    let units = units(FIXTURES[0]);
    let mut settings = core::mem::MaybeUninit::<SplotSettings>::uninit();
    let mut decoder = ptr::null_mut();
    unsafe {
        splot_default_settings(settings.as_mut_ptr());
        assert_eq!(splot_open(&raw mut decoder, settings.as_ptr()), SPLOT_OK);
        for pass in 0..2 {
            let mut raw = Vec::new();
            let mut types = Vec::new();
            for unit in &units {
                assert_eq!(
                    splot_send_data(decoder, unit.as_ptr(), unit.len(), ptr::null()),
                    SPLOT_OK
                );
                let mut picture = core::mem::zeroed::<SplotPicture>();
                while splot_wait_picture(decoder, &raw mut picture) == SPLOT_OK {
                    append(&picture, &mut raw);
                    types.push(picture.frame_type);
                    splot_picture_unref(&raw mut picture);
                }
            }
            assert!(raw == expected, "pass {pass}");
            assert_eq!(types[0], SplotFrameType::Key as u8);
            assert!(!types[1..].contains(&(SplotFrameType::None as u8)));
            let mut idle = core::mem::zeroed::<SplotPicture>();
            assert_eq!(splot_wait_picture(decoder, &raw mut idle), SPLOT_EAGAIN);
            splot_drain(decoder);
            splot_flush(decoder);
        }
        splot_close(&raw mut decoder);
    }
}

#[test]
fn null_and_bad_arguments_are_refused() {
    let mut decoder = ptr::null_mut();
    unsafe {
        assert_eq!(splot_open(&raw mut decoder, ptr::null()), SPLOT_EINVAL);
        for (
            skip_film_grain,
            temporal_output_mask,
            embedded_output_mask,
            reserved0,
            extended_output_mask,
            reserved,
        ) in [
            (0, 0, 0, 0, 0, [1; 20]),
            (0, 0, 0, 1, 0, [0; 20]),
            (0, 0, 0, 0, 1u32 << 31, [0; 20]),
            (2, 0, 0, 0, 0, [0; 20]),
            (0, 0x10, 0, 0, 0, [0; 20]),
        ] {
            let settings = SplotSettings {
                n_threads: 1,
                max_frame_delay: 1,
                skip_film_grain,
                temporal_output_mask,
                embedded_output_mask,
                reserved0,
                extended_output_mask,
                operating_point_scope: 0,
                operating_point_xlayer_id: 0,
                operating_point_ops_id: 0,
                operating_point_index: 0,
                reserved,
            };
            assert_eq!(
                splot_open(&raw mut decoder, &raw const settings),
                SPLOT_EINVAL
            );
            assert!(decoder.is_null());
        }
        let mut picture = core::mem::zeroed::<SplotPicture>();
        let released = core::mem::zeroed::<SplotPicture>();
        assert_eq!(
            splot_apply_grain(&raw mut picture, &raw const released),
            SPLOT_EINVAL
        );
        assert_eq!(
            splot_apply_grain(ptr::null_mut(), &raw const released),
            SPLOT_EINVAL
        );
        assert_eq!(
            splot_apply_grain(&raw mut picture, ptr::null()),
            SPLOT_EINVAL
        );
        assert_eq!(
            splot_send_data(ptr::null_mut(), ptr::null(), 0, ptr::null()),
            SPLOT_EINVAL
        );
        assert_eq!(
            splot_get_picture(ptr::null_mut(), ptr::null_mut()),
            SPLOT_EINVAL
        );
        assert_eq!(
            splot_wait_picture(ptr::null_mut(), ptr::null_mut()),
            SPLOT_EINVAL
        );
        splot_flush(ptr::null_mut());
        splot_drain(ptr::null_mut());
        splot_picture_unref(ptr::null_mut());
        splot_close(ptr::null_mut());
        splot_close(&raw mut decoder);
    }
}

#[test]
fn a_decode_error_ends_the_session() {
    let mut settings = core::mem::MaybeUninit::<SplotSettings>::uninit();
    let mut decoder = ptr::null_mut();
    unsafe {
        splot_default_settings(settings.as_mut_ptr());
        assert_eq!(splot_open(&raw mut decoder, settings.as_ptr()), SPLOT_OK);
        let garbage = [0x01, 0x08, 0x05, 0x10];
        assert_eq!(
            splot_send_data(decoder, garbage.as_ptr(), garbage.len(), ptr::null()),
            SPLOT_OK
        );
        splot_drain(decoder);
        let mut picture = core::mem::zeroed::<SplotPicture>();
        assert_eq!(splot_get_picture(decoder, &raw mut picture), SPLOT_EDECODE);
        assert_eq!(splot_get_picture(decoder, &raw mut picture), SPLOT_ESTATE);
        splot_close(&raw mut decoder);
    }
}

#[test]
fn temporal_layer_and_output_mask_match_batch_decoder_and_report_layer_ids() {
    const DEP_8BIT: &[u8] =
        include_bytes!("../../../tests/conformance/vectors/valid/syn-tlayer2-dependent-8bit.ivf");
    let context = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize))).unwrap();

    for (configure, selection, expected_tlayers) in [
        (
            (|settings: &mut SplotSettings| {
                settings.temporal_output_mask = 1;
            }) as fn(&mut SplotSettings),
            splot_decode::TemporalLayerSelection::BASE_ONLY,
            [0u8, 0u8],
        ),
        (
            (|settings: &mut SplotSettings| {
                settings.temporal_output_mask = 2;
            }) as fn(&mut SplotSettings),
            splot_decode::TemporalLayerSelection::OutputMask(2),
            [1u8, 1u8],
        ),
    ] {
        let mut settings = core::mem::MaybeUninit::<SplotSettings>::uninit();
        let mut decoder = ptr::null_mut();
        let mut raw = Vec::new();
        let mut seen_tlayers = Vec::new();
        unsafe {
            splot_default_settings(settings.as_mut_ptr());
            let mut settings = settings.assume_init();
            settings.n_threads = 1;
            configure(&mut settings);
            assert_eq!(splot_open(&raw mut decoder, &raw const settings), SPLOT_OK);

            let mut take = |decoder: *mut SplotDecoder| loop {
                let mut picture = core::mem::zeroed::<SplotPicture>();
                match splot_get_picture(decoder, &raw mut picture) {
                    SPLOT_OK => {
                        append(&picture, &mut raw);
                        seen_tlayers.push(picture.temporal_layer_id);
                        splot_picture_unref(&raw mut picture);
                    }
                    SPLOT_EAGAIN | SPLOT_EOF => break,
                    error => panic!("splot_get_picture failed: {error}"),
                }
            };

            for unit in units(DEP_8BIT) {
                loop {
                    match splot_send_data(decoder, unit.as_ptr(), unit.len(), ptr::null()) {
                        SPLOT_OK => break,
                        SPLOT_EAGAIN => take(decoder),
                        error => panic!("splot_send_data failed: {error}"),
                    }
                }
                take(decoder);
            }
            splot_drain(decoder);
            take(decoder);
            splot_close(&raw mut decoder);
        }

        assert_eq!(seen_tlayers, expected_tlayers);
        let mut expected = Vec::new();
        context
            .decode_raw_bytes(
                DEP_8BIT,
                DecodeOptions::default().with_temporal_layer_selection(selection),
                &mut expected,
            )
            .unwrap();
        assert_eq!(raw, expected);
    }
}

#[test]
fn embedded_output_mask_matches_batch_decoder_and_reports_embedded_layer_ids() {
    const MLAYER_8BIT: &[u8] =
        include_bytes!("../../../tests/conformance/vectors/valid/syn-mlayer2-crossref-8bit.obu");
    let context = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize))).unwrap();
    let mut reader = TemporalUnitReader::new(MLAYER_8BIT);
    let mut stream_units = Vec::new();
    while let Some(unit) = reader.next_temporal_unit().unwrap() {
        stream_units.push(unit.bytes.to_vec());
    }

    for (mask, selection, expected_mlayers) in [
        (
            0u8,
            splot_decode::EmbeddedLayerSelection::All,
            vec![0u8, 1, 0, 1, 0, 1],
        ),
        (
            1u8,
            splot_decode::EmbeddedLayerSelection::BASE_ONLY,
            vec![0u8, 0, 0],
        ),
        (
            2u8,
            splot_decode::EmbeddedLayerSelection::OutputMask(2),
            vec![1u8, 1, 1],
        ),
        (
            3u8,
            splot_decode::EmbeddedLayerSelection::OutputMask(3),
            vec![0u8, 1, 0, 1, 0, 1],
        ),
    ] {
        let mut settings = core::mem::MaybeUninit::<SplotSettings>::uninit();
        let mut decoder = ptr::null_mut();
        let mut raw = Vec::new();
        let mut seen_mlayers = Vec::new();
        unsafe {
            splot_default_settings(settings.as_mut_ptr());
            let mut settings = settings.assume_init();
            settings.n_threads = 1;
            settings.embedded_output_mask = mask;
            assert_eq!(splot_open(&raw mut decoder, &raw const settings), SPLOT_OK);

            let mut take = |decoder: *mut SplotDecoder| loop {
                let mut picture = core::mem::zeroed::<SplotPicture>();
                match splot_get_picture(decoder, &raw mut picture) {
                    SPLOT_OK => {
                        append(&picture, &mut raw);
                        seen_mlayers.push(picture.embedded_layer_id);
                        splot_picture_unref(&raw mut picture);
                    }
                    SPLOT_EAGAIN | SPLOT_EOF => break,
                    error => panic!("splot_get_picture failed: {error}"),
                }
            };

            for unit in &stream_units {
                loop {
                    match splot_send_data(decoder, unit.as_ptr(), unit.len(), ptr::null()) {
                        SPLOT_OK => break,
                        SPLOT_EAGAIN => take(decoder),
                        error => panic!("splot_send_data failed: {error}"),
                    }
                }
                take(decoder);
            }
            splot_drain(decoder);
            take(decoder);
            splot_close(&raw mut decoder);
        }

        assert_eq!(seen_mlayers, expected_mlayers);
        let mut expected = Vec::new();
        context
            .decode_raw_bytes(
                MLAYER_8BIT,
                DecodeOptions::default().with_embedded_layer_selection(selection),
                &mut expected,
            )
            .unwrap();
        assert_eq!(raw, expected);
    }
}

#[test]
fn extended_output_mask_matches_batch_decoder_and_reports_extended_layer_ids() {
    const XLAYER_ISO: &[u8] = include_bytes!(
        "../../../tests/conformance/vectors/valid/syn-xlayer3-7-global-lcr-isolation.obu"
    );
    let context = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize))).unwrap();
    let mut reader = TemporalUnitReader::new(XLAYER_ISO);
    let mut stream_units = Vec::new();
    while let Some(unit) = reader.next_temporal_unit().unwrap() {
        stream_units.push(unit.bytes.to_vec());
    }

    let mut invalid_settings = core::mem::MaybeUninit::<SplotSettings>::uninit();
    unsafe {
        splot_default_settings(invalid_settings.as_mut_ptr());
        let mut invalid = invalid_settings.assume_init();
        invalid.extended_output_mask = 1u32 << 31;
        let mut bad_decoder = ptr::null_mut();
        assert_eq!(
            splot_open(&raw mut bad_decoder, &raw const invalid),
            SPLOT_EINVAL
        );
    }

    for (mask, expected_xlayers) in [
        (0u32, vec![3u8, 7, 3, 7, 3, 7, 3, 7]),
        (1u32 << 3, vec![3u8, 3, 3, 3]),
        (1u32 << 7, vec![7u8, 7, 7, 7]),
        ((1u32 << 3) | (1u32 << 7), vec![3u8, 7, 3, 7, 3, 7, 3, 7]),
    ] {
        let mut settings = core::mem::MaybeUninit::<SplotSettings>::uninit();
        let mut decoder = ptr::null_mut();
        let mut raw_x3 = Vec::new();
        let mut raw_x7 = Vec::new();
        let mut seen_xlayers = Vec::new();
        unsafe {
            splot_default_settings(settings.as_mut_ptr());
            let mut settings = settings.assume_init();
            settings.n_threads = 1;
            settings.extended_output_mask = mask;
            assert_eq!(splot_open(&raw mut decoder, &raw const settings), SPLOT_OK);

            let mut take = |decoder: *mut SplotDecoder| loop {
                let mut picture = core::mem::zeroed::<SplotPicture>();
                match splot_get_picture(decoder, &raw mut picture) {
                    SPLOT_OK => {
                        match picture.extended_layer_id {
                            3 => append(&picture, &mut raw_x3),
                            7 => append(&picture, &mut raw_x7),
                            other => panic!("unexpected extended_layer_id {other}"),
                        }
                        seen_xlayers.push(picture.extended_layer_id);
                        splot_picture_unref(&raw mut picture);
                    }
                    SPLOT_EAGAIN | SPLOT_EOF => break,
                    error => panic!("splot_get_picture failed: {error}"),
                }
            };

            for unit in &stream_units {
                loop {
                    match splot_send_data(decoder, unit.as_ptr(), unit.len(), ptr::null()) {
                        SPLOT_OK => break,
                        SPLOT_EAGAIN => take(decoder),
                        error => panic!("splot_send_data failed: {error}"),
                    }
                }
                take(decoder);
            }
            splot_drain(decoder);
            take(decoder);
            splot_close(&raw mut decoder);
        }

        assert_eq!(seen_xlayers, expected_xlayers);
        if expected_xlayers.contains(&3) {
            let mut expected = Vec::new();
            context
                .decode_raw_bytes(
                    XLAYER_ISO,
                    DecodeOptions::default().with_extended_layer_selection(
                        splot_decode::ExtendedLayerSelection::OutputMask(1 << 3),
                    ),
                    &mut expected,
                )
                .unwrap();
            assert_eq!(raw_x3, expected);
        } else {
            assert!(raw_x3.is_empty());
        }
        if expected_xlayers.contains(&7) {
            let mut expected = Vec::new();
            context
                .decode_raw_bytes(
                    XLAYER_ISO,
                    DecodeOptions::default().with_extended_layer_selection(
                        splot_decode::ExtendedLayerSelection::OutputMask(1 << 7),
                    ),
                    &mut expected,
                )
                .unwrap();
            assert_eq!(raw_x7, expected);
        } else {
            assert!(raw_x7.is_empty());
        }
    }
}

#[test]
fn operating_point_selection_matches_batch_decoder_and_rejects_invalid() {
    const MSDO_OPS: &[u8] =
        include_bytes!("../../../tests/conformance/vectors/valid/syn-msdo-ops-random-access.obu");
    let context = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize))).unwrap();
    let mut reader = TemporalUnitReader::new(MSDO_OPS);
    let mut stream_units = Vec::new();
    while let Some(unit) = reader.next_temporal_unit().unwrap() {
        stream_units.push(unit.bytes.to_vec());
    }

    for (scope, xlayer, ops_id, op_idx, ext_mask) in [
        (0u8, 1u8, 0u8, 0u8, 0u32),
        (3u8, 0u8, 0u8, 0u8, 0u32),
        (1u8, 1u8, 0u8, 0u8, 0u32),
        (1u8, 0u8, 16u8, 0u8, 0u32),
        (1u8, 0u8, 0u8, 7u8, 0u32),
        (2u8, 31u8, 0u8, 0u8, 0u32),
        (1u8, 0u8, 0u8, 1u8, 1u32 << 3),
    ] {
        let mut s_uninit = core::mem::MaybeUninit::<SplotSettings>::uninit();
        unsafe {
            splot_default_settings(s_uninit.as_mut_ptr());
            let mut s = s_uninit.assume_init();
            s.operating_point_scope = scope;
            s.operating_point_xlayer_id = xlayer;
            s.operating_point_ops_id = ops_id;
            s.operating_point_index = op_idx;
            s.extended_output_mask = ext_mask;
            let mut bad_decoder = ptr::null_mut();
            assert_eq!(splot_open(&raw mut bad_decoder, &raw const s), SPLOT_EINVAL);
            assert!(bad_decoder.is_null());
        }
    }

    for (scope, xlayer, ops_id, op_idx, selection, expected_xlayers) in [
        (
            1u8,
            0u8,
            0u8,
            1u8,
            splot_decode::OperatingPointSelection::Global {
                ops_id: 0,
                op_index: 1,
            },
            vec![3u8, 3, 3, 3, 3, 3],
        ),
        (
            1u8,
            0u8,
            1u8,
            0u8,
            splot_decode::OperatingPointSelection::Global {
                ops_id: 1,
                op_index: 0,
            },
            vec![3u8, 7, 3, 3, 7, 3, 3, 7, 3],
        ),
        (
            2u8,
            3u8,
            2u8,
            0u8,
            splot_decode::OperatingPointSelection::Local {
                xlayer_id: splot_core::types::ExtendedLayerId::from_bits(3),
                ops_id: 2,
                op_index: 0,
            },
            vec![3u8, 7, 7, 3, 7, 7, 3, 7, 7],
        ),
    ] {
        let mut s_uninit = core::mem::MaybeUninit::<SplotSettings>::uninit();
        let mut decoder = ptr::null_mut();
        let mut raw_x3 = Vec::new();
        let mut raw_x7 = Vec::new();
        let mut seen_xlayers = Vec::new();
        unsafe {
            splot_default_settings(s_uninit.as_mut_ptr());
            let mut settings = s_uninit.assume_init();
            settings.n_threads = 1;
            settings.operating_point_scope = scope;
            settings.operating_point_xlayer_id = xlayer;
            settings.operating_point_ops_id = ops_id;
            settings.operating_point_index = op_idx;
            assert_eq!(splot_open(&raw mut decoder, &raw const settings), SPLOT_OK);

            let mut take = |decoder: *mut SplotDecoder| loop {
                let mut picture = core::mem::zeroed::<SplotPicture>();
                match splot_get_picture(decoder, &raw mut picture) {
                    SPLOT_OK => {
                        match picture.extended_layer_id {
                            3 => append(&picture, &mut raw_x3),
                            7 => append(&picture, &mut raw_x7),
                            other => panic!("unexpected extended_layer_id {other}"),
                        }
                        seen_xlayers.push(picture.extended_layer_id);
                        splot_picture_unref(&raw mut picture);
                    }
                    SPLOT_EAGAIN | SPLOT_EOF => break,
                    error => panic!("splot_get_picture failed: {error}"),
                }
            };

            for unit in &stream_units {
                loop {
                    match splot_send_data(decoder, unit.as_ptr(), unit.len(), ptr::null()) {
                        SPLOT_OK => break,
                        SPLOT_EAGAIN => take(decoder),
                        error => panic!("splot_send_data failed: {error}"),
                    }
                }
                take(decoder);
            }
            splot_drain(decoder);
            take(decoder);
            splot_close(&raw mut decoder);
        }

        assert_eq!(seen_xlayers, expected_xlayers);
        let report = context
            .decode_hash_report_bytes(
                MSDO_OPS,
                DecodeOptions::default().with_operating_point_selection(selection),
            )
            .unwrap();
        assert_eq!(report.frames.len(), seen_xlayers.len());
        if !raw_x7.is_empty() {
            let x7_frames = seen_xlayers
                .iter()
                .fold(0usize, |acc, &x| acc + usize::from(x == 7));
            assert_eq!(raw_x7.len(), x7_frames * 80 * 64 * 3 / 2);
        }
        let x3_frames = seen_xlayers
            .iter()
            .fold(0usize, |acc, &x| acc + usize::from(x == 3));
        assert_eq!(raw_x3.len(), x3_frames * 64 * 64 * 3 / 2);
    }
}

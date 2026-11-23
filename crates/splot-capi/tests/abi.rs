// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Drives the C ABI the way a C host does and checks the pictures against
//! the batch decoder's raw output.

#![allow(unsafe_code)]
#![expect(clippy::unwrap_used, clippy::panic)]

use core::ptr;

use splot_capi::*;
use splot_core::stream_reader::{StreamUnit, TemporalUnitReader};
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

const FILM_GRAIN: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-filmgrain-intra-64x64.ivf");

fn units(ivf: &[u8]) -> Vec<Vec<u8>> {
    let mut reader = TemporalUnitReader::new(ivf);
    let mut units = Vec::new();
    while let Some(unit) = reader.next_unit().unwrap() {
        if let StreamUnit::IvfFrame { payload, .. } = unit {
            units.push(payload.to_vec());
        }
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

/// Decodes `ivf` through the C ABI and closes the decoder; the caller
/// releases the pictures.
fn pictures(ivf: &[u8], threads: u32, skip_film_grain: u8) -> Vec<SplotPicture> {
    let mut settings = SplotSettings {
        n_threads: 0,
        max_frame_delay: 0,
        skip_film_grain: 1,
        simd_policy: 99,
        reserved: [1; 30],
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
        for (index, unit) in units(ivf).iter().enumerate() {
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
        splot_close(&raw mut decoder);
        assert!(decoder.is_null());
        pictures
    }
}

/// Decodes `ivf` through the C ABI, keeping every picture until the end.
fn decode(ivf: &[u8], threads: u32) -> (Vec<u8>, Vec<SplotDataProps>) {
    let mut raw = Vec::new();
    let mut props = Vec::new();
    for mut picture in pictures(ivf, threads, 0) {
        append(&picture, &mut raw);
        props.push(picture.props);
        assert_eq!(picture.has_film_grain, 0);
        unsafe { splot_picture_unref(&raw mut picture) };
        assert!(picture.internal.is_null() && picture.data[0].is_null());
    }
    (raw, props)
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

/// `skip_film_grain` and `has_film_grain` each took the first reserved byte:
/// `reserved` was 32 bytes in the settings and 4 `usize` in the picture.
#[test]
fn the_new_fields_keep_the_struct_layouts() {
    use core::mem::offset_of;
    assert_eq!(size_of::<SplotSettings>(), 40);
    assert_eq!(offset_of!(SplotSettings, skip_film_grain), 8);
    assert_eq!(offset_of!(SplotSettings, simd_policy), 9);
    assert_eq!(
        offset_of!(SplotPicture, internal),
        offset_of!(SplotPicture, has_film_grain) + 4 * size_of::<usize>()
    );
    assert_eq!(
        offset_of!(SplotPicture, frame_type),
        offset_of!(SplotPicture, has_film_grain) + 1
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
        for (skip_film_grain, simd_policy, reserved) in
            [(0, 0, [1; 30]), (2, 0, [0; 30]), (0, 4, [0; 30])]
        {
            let settings = SplotSettings {
                n_threads: 1,
                max_frame_delay: 1,
                skip_film_grain,
                simd_policy,
                reserved,
            };
            assert_eq!(
                splot_open(&raw mut decoder, &raw const settings),
                SPLOT_EINVAL
            );
            assert!(decoder.is_null());
        }

        let mut neon_settings = core::mem::MaybeUninit::<SplotSettings>::uninit();
        splot_default_settings(neon_settings.as_mut_ptr());
        let mut neon_settings = neon_settings.assume_init();
        neon_settings.n_threads = 1;
        #[cfg(target_arch = "x86_64")]
        {
            neon_settings.simd_policy = SplotSimdPolicy::Neon as u8;
            assert_eq!(
                splot_open(&raw mut decoder, &raw const neon_settings),
                SPLOT_ENOTSUP
            );
            assert!(decoder.is_null());

            if !splot_core::simd::detect_capabilities().has_avx512() {
                neon_settings.simd_policy = SplotSimdPolicy::Avx512 as u8;
                assert_eq!(
                    splot_open(&raw mut decoder, &raw const neon_settings),
                    SPLOT_ENOTSUP
                );
                assert!(decoder.is_null());
            }

            neon_settings.simd_policy = SplotSimdPolicy::Avx2 as u8;
            assert_eq!(
                splot_open(&raw mut decoder, &raw const neon_settings),
                SPLOT_OK
            );
            assert!(!decoder.is_null());
            splot_close(&raw mut decoder);
        }
        #[cfg(target_arch = "aarch64")]
        {
            neon_settings.simd_policy = SplotSimdPolicy::Avx2 as u8;
            assert_eq!(
                splot_open(&raw mut decoder, &raw const neon_settings),
                SPLOT_ENOTSUP
            );
            assert!(decoder.is_null());

            neon_settings.simd_policy = SplotSimdPolicy::Neon as u8;
            assert_eq!(
                splot_open(&raw mut decoder, &raw const neon_settings),
                SPLOT_OK
            );
            assert!(!decoder.is_null());
            splot_close(&raw mut decoder);
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

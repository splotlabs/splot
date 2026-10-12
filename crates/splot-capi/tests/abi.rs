// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Drives the C ABI the way a C host does and checks the pictures against
//! the batch decoder's raw output.

#![allow(unsafe_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

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

/// Decodes `ivf` through the C ABI, keeping every picture until the end.
fn decode(ivf: &[u8], threads: u32) -> (Vec<u8>, Vec<SplotDataProps>) {
    let mut settings = SplotSettings {
        n_threads: 0,
        max_frame_delay: 0,
        reserved: [1; 32],
    };
    let mut decoder = ptr::null_mut();
    unsafe {
        splot_default_settings(&raw mut settings);
        settings.n_threads = threads;
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
        let mut raw = Vec::new();
        let mut props = Vec::new();
        for mut picture in pictures {
            append(&picture, &mut raw);
            props.push(picture.props);
            splot_picture_unref(&raw mut picture);
            assert!(picture.internal.is_null() && picture.data[0].is_null());
        }
        (raw, props)
    }
}

#[test]
fn pictures_from_the_c_abi_match_the_batch_decoder() {
    for fixture in FIXTURES {
        let context =
            DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize))).unwrap();
        let mut expected = Vec::new();
        context
            .decode_raw_bytes(fixture, DecodeOptions::default(), &mut expected)
            .unwrap();
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
fn null_and_bad_arguments_are_refused() {
    let mut decoder = ptr::null_mut();
    unsafe {
        assert_eq!(splot_open(&raw mut decoder, ptr::null()), SPLOT_EINVAL);
        let settings = SplotSettings {
            n_threads: 1,
            max_frame_delay: 1,
            reserved: [1; 32],
        };
        assert_eq!(
            splot_open(&raw mut decoder, &raw const settings),
            SPLOT_EINVAL
        );
        assert!(decoder.is_null());
        assert_eq!(
            splot_send_data(ptr::null_mut(), ptr::null(), 0, ptr::null()),
            SPLOT_EINVAL
        );
        assert_eq!(
            splot_get_picture(ptr::null_mut(), ptr::null_mut()),
            SPLOT_EINVAL
        );
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

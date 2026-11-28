// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use std::io::Write;

use splot_core::simd::{SelectedBackend, resolve_backend};
use splot_core::stream_reader::{StreamUnit, TemporalUnitReader};
use splot_decode::{
    DataProps, DecodeError, Decoder, DecoderSettings, PictureSamples, PlaneId, SendStatus,
    SimdPolicy, ThreadCount,
};

fn assert_unsupported(settings: DecoderSettings, policy: SimdPolicy) {
    let error = Decoder::new(settings.with_simd_policy(policy)).unwrap_err();
    assert!(
        matches!(error, DecodeError::UnsupportedBackend { source } if source.requested() == policy),
        "{policy}: {error}"
    );
}

fn write_ready_pictures(decoder: &mut Decoder, output: &mut impl Write) {
    while let Some(picture) = decoder.get_picture().unwrap() {
        for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
            match picture.plane(plane).unwrap() {
                PictureSamples::Eight(plane) => {
                    for row in plane.visible_rows() {
                        output.write_all(row).unwrap();
                    }
                }
                PictureSamples::Ten(plane) => {
                    for sample in plane.visible_rows().flatten() {
                        output.write_all(&sample.to_le_bytes()).unwrap();
                    }
                }
            }
        }
    }
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let [_, expected_backend, filename] = args.as_slice() else {
        panic!("expected backend and IVF filename");
    };
    assert!(
        !cfg!(target_feature = "avx2"),
        "consumer requires default compiler flags"
    );
    assert_eq!(
        std::arch::is_x86_feature_detected!("avx2"),
        expected_backend == "avx2",
        "unexpected emulated CPU"
    );
    if expected_backend == "sse2" {
        assert!(!std::arch::is_x86_feature_detected!("sse3"));
    }

    assert!(resolve_backend(SimdPolicy::Avx512).is_err());
    if let Ok(narrow) = resolve_backend(SimdPolicy::Avx2) {
        let backend = narrow.backend();
        assert_eq!(backend.kind(), SelectedBackend::Avx2);
        assert!(splot_core::with_simd!(backend, _simd => backend.avx2_token().is_some()));
    }
    let settings = DecoderSettings::default().with_threads(ThreadCount::from(3));
    let mut decoder = Decoder::new(settings).unwrap();
    assert_eq!(decoder.selected_backend().as_str(), expected_backend);
    assert_unsupported(settings, SimdPolicy::Neon);
    assert_unsupported(settings, SimdPolicy::Avx512);
    if expected_backend == "sse2" {
        assert_unsupported(settings, SimdPolicy::Avx2);
    }

    let input = std::fs::read(filename).unwrap();
    let mut reader = TemporalUnitReader::new(input.as_slice());
    let mut output = std::io::BufWriter::new(std::io::stdout().lock());
    while let Some(unit) = reader.next_unit().unwrap() {
        let StreamUnit::IvfFrame { payload, .. } = unit else {
            panic!("expected IVF frame");
        };
        while decoder.send_data(payload, DataProps::default()).unwrap() == SendStatus::Full {
            write_ready_pictures(&mut decoder, &mut output);
        }
        write_ready_pictures(&mut decoder, &mut output);
    }
    decoder.drain();
    write_ready_pictures(&mut decoder, &mut output);
    output.flush().unwrap();
}

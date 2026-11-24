// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! End-to-end forced-backend parity, typed-session, lifecycle, and fail-closed
//! CPU-safety integration tests for startup SIMD backend selection.

#![expect(clippy::unwrap_used)]

use splot_core::simd::{SimdBackend, detect_capabilities};
use splot_core::stream_reader::{StreamUnit, TemporalUnitReader};
use splot_decode::{
    DataProps, DecodeContext, DecodeError, DecodeHashReport, DecodeOptions, DecodeRuntimeConfig,
    DecodeScope, Decoder, DecoderSettings, FrameDelay, Picture, PictureSamples, PlaneId,
    SelectedBackend, SendStatus, SimdPolicy, ThreadCount, TypedDecodeContext,
};

const FIXTURES: &[(&str, &[u8])] = &[
    (
        "multiref",
        include_bytes!("../../../tests/conformance/vectors/valid/syn-3frame-multiref-64x64.ivf"),
    ),
    (
        "opfl_refine",
        include_bytes!(
            "../../../tests/conformance/vectors/valid/syn-8frame-opfl-refine-all-64x64-q120.ivf"
        ),
    ),
    (
        "ten_bit_deblock",
        include_bytes!(
            "../../../tests/conformance/vectors/valid/syn-3frame-deblock-subpu-chroma-64x32-10bit-q90.ivf"
        ),
    ),
    (
        "film_grain",
        include_bytes!("../../../tests/conformance/vectors/valid/syn-filmgrain-intra-64x64.ivf"),
    ),
    (
        "lr_wienerns_8bit",
        include_bytes!(
            "../../../tests/conformance/vectors/valid/syn-bru-lr-stripe-wienerns-8bit.ivf"
        ),
    ),
    (
        "lr_wienerns_10bit",
        include_bytes!(
            "../../../tests/conformance/vectors/valid/syn-bru-lr-stripe-wienerns-10bit.ivf"
        ),
    ),
    (
        "lr_pcwiener_8bit",
        include_bytes!(
            "../../../tests/conformance/vectors/valid/syn-bru-lr-stripe-pcwiener-8bit.ivf"
        ),
    ),
    (
        "lr_pcwiener_10bit",
        include_bytes!(
            "../../../tests/conformance/vectors/valid/syn-bru-lr-stripe-pcwiener-10bit.ivf"
        ),
    ),
    (
        "lr_multitile_cross_8bit",
        include_bytes!(
            "../../../tests/conformance/vectors/valid/syn-bru-lr-multitile-cross-8bit.ivf"
        ),
    ),
    (
        "gdf_10bit",
        include_bytes!("../../../tests/conformance/vectors/valid/syn-bru-gdf-block-10bit.ivf"),
    ),
];

struct ContextOutputs {
    hash: DecodeHashReport,
    raw: Vec<u8>,
    y4m: Vec<u8>,
}

struct TypedParityCheck<'a> {
    expected_backend: SelectedBackend,
    bytes: &'a [u8],
    expected: &'a ContextOutputs,
    label: &'a str,
}

impl DecodeScope<()> for TypedParityCheck<'_> {
    fn run<B: SimdBackend>(self, typed: &TypedDecodeContext<'_, B>) {
        assert_eq!(B::KIND, self.expected_backend, "{}", self.label);
        assert_eq!(
            typed.selected_backend(),
            self.expected_backend,
            "{}",
            self.label
        );
        let options = DecodeOptions::default();
        let hash = typed.decode_hash_report_bytes(self.bytes, options).unwrap();
        assert_eq!(hash, self.expected.hash, "typed hash {}", self.label);
        let mut raw = Vec::new();
        typed
            .decode_raw_bytes(self.bytes, options, &mut raw)
            .unwrap();
        assert_eq!(raw, self.expected.raw, "typed raw {}", self.label);
        let mut y4m = Vec::new();
        typed
            .decode_y4m_bytes(self.bytes, options, &mut y4m)
            .unwrap();
        assert_eq!(y4m, self.expected.y4m, "typed y4m {}", self.label);
        typed.decode_discard_reader(self.bytes, options).unwrap();
    }
}

fn supported_forced_policies() -> Vec<(SimdPolicy, SelectedBackend)> {
    let caps = detect_capabilities();
    let mut policies = Vec::new();
    #[cfg(target_arch = "x86_64")]
    {
        if caps.has_avx2() {
            policies.push((SimdPolicy::Avx2, SelectedBackend::Avx2));
        }
        if caps.has_avx512() {
            policies.push((SimdPolicy::Avx512, SelectedBackend::Avx512));
        }
    }
    #[cfg(target_arch = "aarch64")]
    if caps.has_neon() {
        policies.push((SimdPolicy::Neon, SelectedBackend::Neon));
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        let _ = caps;
        policies.push((SimdPolicy::Auto, SelectedBackend::Fallback));
    }
    policies
}

fn unsupported_policies() -> Vec<SimdPolicy> {
    let caps = detect_capabilities();
    let mut policies = Vec::new();
    #[cfg(target_arch = "x86_64")]
    {
        policies.push(SimdPolicy::Neon);
        if !caps.has_avx2() {
            policies.push(SimdPolicy::Avx2);
        }
        if !caps.has_avx512() {
            policies.push(SimdPolicy::Avx512);
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        policies.push(SimdPolicy::Avx2);
        policies.push(SimdPolicy::Avx512);
        if !caps.has_neon() {
            policies.push(SimdPolicy::Neon);
        }
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        let _ = caps;
        policies.extend([SimdPolicy::Avx2, SimdPolicy::Avx512, SimdPolicy::Neon]);
    }
    policies
}

fn extract_units(ivf: &[u8]) -> Vec<Vec<u8>> {
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

fn collect_context_outputs(ctx: &DecodeContext, bytes: &[u8]) -> ContextOutputs {
    let options = DecodeOptions::default();
    let hash = ctx.decode_hash_report_bytes(bytes, options).unwrap();
    let mut raw = Vec::new();
    ctx.decode_raw_bytes(bytes, options, &mut raw).unwrap();
    let mut y4m = Vec::new();
    ctx.decode_y4m_bytes(bytes, options, &mut y4m).unwrap();
    ctx.decode_discard_reader(bytes, options).unwrap();
    ContextOutputs { hash, raw, y4m }
}

fn raw_from_pictures(pictures: &[Picture]) -> Vec<u8> {
    let mut out = Vec::new();
    for picture in pictures {
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
    out
}

fn decode_streaming(decoder: &mut Decoder, units: &[Vec<u8>]) -> Vec<Picture> {
    let mut pictures = Vec::new();
    for unit in units {
        while decoder.send_data(unit, DataProps::default()).unwrap() == SendStatus::Full {
            pictures.extend(core::iter::from_fn(|| decoder.get_picture().unwrap()));
        }
        pictures.extend(core::iter::from_fn(|| decoder.get_picture().unwrap()));
    }
    decoder.drain();
    pictures.extend(core::iter::from_fn(|| decoder.get_picture().unwrap()));
    pictures
}

#[test]
fn forced_and_auto_policies_match_across_sync_streaming_and_worker_counts() {
    let forced = supported_forced_policies();
    assert!(!forced.is_empty());

    let auto_ctx = DecodeContext::new(DecodeRuntimeConfig::default()).unwrap();
    #[cfg(target_arch = "x86_64")]
    assert_eq!(auto_ctx.selected_backend(), SelectedBackend::Avx2);
    #[cfg(target_arch = "aarch64")]
    assert_eq!(auto_ctx.selected_backend(), SelectedBackend::Neon);
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    assert_eq!(auto_ctx.selected_backend(), SelectedBackend::Fallback);

    for &(name, bytes) in FIXTURES {
        let units = extract_units(bytes);
        let expected = collect_context_outputs(&auto_ctx, bytes);

        for &threads in &[1usize, 3, 8] {
            let delay = threads.min(4);
            let mut thread_expected: Option<ContextOutputs> = None;
            let mut policies = vec![(SimdPolicy::Auto, auto_ctx.selected_backend())];
            policies.extend_from_slice(&forced);

            for &(policy, expected_backend) in &policies {
                let label = format!("{name} threads={threads} policy={policy}");
                let ctx = DecodeContext::new(
                    DecodeRuntimeConfig::new(ThreadCount::from(threads))
                        .with_frame_delay(FrameDelay::from(delay))
                        .with_simd_policy(policy),
                )
                .unwrap();
                assert_eq!(ctx.selected_backend(), expected_backend, "{label}");
                let actual = collect_context_outputs(&ctx, bytes);
                assert_eq!(actual.hash.frames, expected.hash.frames, "hash {label}");
                assert_eq!(actual.raw, expected.raw, "raw {label}");
                assert_eq!(actual.y4m, expected.y4m, "y4m {label}");
                let baseline = thread_expected.get_or_insert_with(|| ContextOutputs {
                    hash: actual.hash.clone(),
                    raw: actual.raw.clone(),
                    y4m: actual.y4m.clone(),
                });
                assert_eq!(actual.hash, baseline.hash, "thread hash {label}");

                ctx.with_typed_backend(TypedParityCheck {
                    expected_backend,
                    bytes,
                    expected: baseline,
                    label: &label,
                });

                let mut decoder = Decoder::new(
                    DecoderSettings::default()
                        .with_threads(ThreadCount::from(threads))
                        .with_frame_delay(FrameDelay::from(delay))
                        .with_simd_policy(policy),
                )
                .unwrap();
                assert_eq!(decoder.selected_backend(), expected_backend, "{label}");
                let pass1 = decode_streaming(&mut decoder, &units);
                assert_eq!(raw_from_pictures(&pass1), expected.raw, "stream1 {label}");
                assert_eq!(decoder.selected_backend(), expected_backend, "{label}");

                decoder.flush();
                assert_eq!(decoder.selected_backend(), expected_backend, "{label}");
                let pass2 = decode_streaming(&mut decoder, &units);
                assert_eq!(raw_from_pictures(&pass2), expected.raw, "stream2 {label}");
                assert_eq!(decoder.selected_backend(), expected_backend, "{label}");

                if name == "film_grain" {
                    let mut no_grain_decoder = Decoder::new(
                        DecoderSettings::default()
                            .with_threads(ThreadCount::from(threads))
                            .with_frame_delay(FrameDelay::from(delay))
                            .with_skip_film_grain(true)
                            .with_simd_policy(policy),
                    )
                    .unwrap();
                    let skipped = decode_streaming(&mut no_grain_decoder, &units);
                    let applied: Vec<Picture> = skipped
                        .iter()
                        .map(|p| p.apply_film_grain().unwrap())
                        .collect();
                    assert_eq!(
                        raw_from_pictures(&applied),
                        expected.raw,
                        "reapplied grain {label}"
                    );
                }
            }
        }
    }
}

#[test]
fn unsupported_forced_policies_fail_closed_in_decoder_and_context() {
    for policy in unsupported_policies() {
        let ctx_err = DecodeContext::new(DecodeRuntimeConfig::default().with_simd_policy(policy))
            .unwrap_err();
        assert!(
            matches!(ctx_err, DecodeError::UnsupportedBackend { .. }),
            "{policy}: {ctx_err}"
        );
        let dec_err =
            Decoder::new(DecoderSettings::default().with_simd_policy(policy)).unwrap_err();
        assert!(
            matches!(dec_err, DecodeError::UnsupportedBackend { .. }),
            "{policy}: {dec_err}"
        );
    }
}

fn assert_malformed_decode_errors(decoder: &mut Decoder, malformed: &[u8], label: &str) {
    let mut saw_error = false;
    if decoder.send_data(malformed, DataProps::default()).is_err() {
        saw_error = true;
    } else {
        decoder.drain();
        loop {
            match decoder.get_picture() {
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(_) => {
                    saw_error = true;
                    break;
                }
            }
        }
    }
    assert!(saw_error, "expected decode failure for {label}");
}

struct ConcurrentCase<'a> {
    name: &'a str,
    bytes: &'a [u8],
    units: Vec<Vec<u8>>,
    expected: ContextOutputs,
}

#[test]
fn concurrent_multi_context_and_decoder_sessions_preserve_parity() {
    let auto_ctx = DecodeContext::new(DecodeRuntimeConfig::default()).unwrap();
    let mut policies = vec![(SimdPolicy::Auto, auto_ctx.selected_backend())];
    policies.extend(supported_forced_policies());

    let cases: Vec<ConcurrentCase<'_>> = ["multiref", "opfl_refine", "lr_wienerns_8bit"]
        .into_iter()
        .map(|target| {
            let &(_, bytes) = FIXTURES.iter().find(|&&(name, _)| name == target).unwrap();
            ConcurrentCase {
                name: target,
                bytes,
                units: extract_units(bytes),
                expected: collect_context_outputs(&auto_ctx, bytes),
            }
        })
        .collect();

    std::thread::scope(|scope| {
        for case in &cases {
            let (name, bytes, units, expected) =
                (case.name, case.bytes, &case.units, &case.expected);
            for &threads in &[1usize, 3, 4] {
                for &(policy, expected_backend) in &policies {
                    scope.spawn(move || {
                        let label = format!("{name} concurrent threads={threads} policy={policy}");
                        let delay = threads.min(4);
                        let ctx = DecodeContext::new(
                            DecodeRuntimeConfig::new(ThreadCount::from(threads))
                                .with_frame_delay(FrameDelay::from(delay))
                                .with_simd_policy(policy),
                        )
                        .unwrap();
                        assert_eq!(ctx.selected_backend(), expected_backend, "{label}");
                        let actual = collect_context_outputs(&ctx, bytes);
                        assert_eq!(actual.hash.frames, expected.hash.frames, "hash {label}");
                        assert_eq!(actual.raw, expected.raw, "raw {label}");
                        assert_eq!(actual.y4m, expected.y4m, "y4m {label}");

                        let mut decoder = Decoder::new(
                            DecoderSettings::default()
                                .with_threads(ThreadCount::from(threads))
                                .with_frame_delay(FrameDelay::from(delay))
                                .with_simd_policy(policy),
                        )
                        .unwrap();
                        assert_eq!(decoder.selected_backend(), expected_backend, "{label}");
                        let pics = decode_streaming(&mut decoder, units);
                        assert_eq!(raw_from_pictures(&pics), expected.raw, "stream {label}");
                        assert_eq!(decoder.selected_backend(), expected_backend, "{label}");
                    });
                }
            }
        }
    });
}

#[test]
fn malformed_input_and_early_drop_preserve_backend_and_recover() {
    let auto_ctx = DecodeContext::new(DecodeRuntimeConfig::default()).unwrap();
    let mut policies = vec![(SimdPolicy::Auto, auto_ctx.selected_backend())];
    policies.extend(supported_forced_policies());

    let &(_, multiref_bytes) = FIXTURES.iter().find(|&&(n, _)| n == "multiref").unwrap();
    let multiref_expected = collect_context_outputs(&auto_ctx, multiref_bytes);
    let multiref_units = extract_units(multiref_bytes);
    let truncated_unit = &multiref_units[0][..multiref_units[0].len() / 2];

    let &(_, opfl_bytes) = FIXTURES.iter().find(|&&(n, _)| n == "opfl_refine").unwrap();
    let opfl_expected = collect_context_outputs(&auto_ctx, opfl_bytes);
    let opfl_units = extract_units(opfl_bytes);

    let garbage: [u8; 4] = [0x01, 0x08, 0x05, 0x10];

    for &(policy, expected_backend) in &policies {
        for (case_name, bad_bytes) in [("garbage", &garbage[..]), ("truncated", truncated_unit)] {
            let label = format!("{case_name} policy={policy}");
            let mut decoder = Decoder::new(
                DecoderSettings::default()
                    .with_threads(ThreadCount::from(3))
                    .with_frame_delay(FrameDelay::from(2))
                    .with_simd_policy(policy),
            )
            .unwrap();
            assert_eq!(decoder.selected_backend(), expected_backend, "{label}");
            assert_malformed_decode_errors(&mut decoder, bad_bytes, &label);
            assert_eq!(decoder.selected_backend(), expected_backend, "{label}");
            decoder.flush();
            assert_eq!(decoder.selected_backend(), expected_backend, "{label}");
            let recovered = decode_streaming(&mut decoder, &multiref_units);
            assert_eq!(
                raw_from_pictures(&recovered),
                multiref_expected.raw,
                "recovered {label}"
            );
            assert_eq!(decoder.selected_backend(), expected_backend, "{label}");
        }

        {
            let mut partial = Decoder::new(
                DecoderSettings::default()
                    .with_threads(ThreadCount::from(4))
                    .with_frame_delay(FrameDelay::from(4))
                    .with_simd_policy(policy),
            )
            .unwrap();
            assert_eq!(partial.selected_backend(), expected_backend);
            let _ = partial.send_data(&opfl_units[0], DataProps::default());
            drop(partial);
        }

        let mut fresh = Decoder::new(
            DecoderSettings::default()
                .with_threads(ThreadCount::from(4))
                .with_frame_delay(FrameDelay::from(4))
                .with_simd_policy(policy),
        )
        .unwrap();
        assert_eq!(fresh.selected_backend(), expected_backend);
        let full = decode_streaming(&mut fresh, &opfl_units);
        assert_eq!(raw_from_pictures(&full), opfl_expected.raw);
        assert_eq!(fresh.selected_backend(), expected_backend);
    }
}

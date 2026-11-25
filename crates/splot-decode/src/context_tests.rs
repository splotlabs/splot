// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;
use crate::test_support::MINIMAL_FIXTURE;
use splot_parallel::ThreadCount;

#[test]
fn default_runtime_config_is_auto() {
    assert_eq!(
        DecodeRuntimeConfig::default().thread_count,
        ThreadCount::Auto
    );
}

#[test]
fn context_resolves_fixed_thread_count() {
    let ctx = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(4usize))).unwrap();
    assert_eq!(ctx.threads().get(), 4);
}

#[test]
fn zero_threads_maps_to_auto() {
    let ctx = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(0usize))).unwrap();
    assert!(ctx.threads().get() >= 1);
}

#[test]
fn discard_output_decodes_supported_fixture() {
    let ctx = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize))).unwrap();

    ctx.decode_discard_reader(MINIMAL_FIXTURE, DecodeOptions::default())
        .unwrap();
    ctx.decode_discard_reader(
        &include_bytes!("../../../tests/conformance/vectors/valid/syn-filmgrain-intra-64x64.ivf")[..],
        DecodeOptions::default(),
    )
    .unwrap();
}

#[test]
fn a_context_reused_across_streams_decodes_like_a_new_one() {
    const STREAMS: [&[u8]; 4] = [
        include_bytes!("../../../tests/conformance/vectors/valid/syn-3frame-multiref-64x64.ivf"),
        include_bytes!(
            "../../../tests/conformance/vectors/valid/syn-8frame-opfl-refine-all-64x64-q120.ivf"
        ),
        include_bytes!("../../../tests/conformance/vectors/valid/syn-frame-tip-families-64x64.ivf"),
        include_bytes!("../../../tests/conformance/vectors/valid/syn-warp-inter-128x128.ivf"),
    ];
    let context = || {
        DecodeContext::new(
            DecodeRuntimeConfig::new(ThreadCount::from(4usize))
                .with_frame_delay(splot_parallel::FrameDelay::from(4usize)),
        )
        .unwrap()
    };
    let decode = |context: &DecodeContext, bytes: &[u8]| {
        let mut output = Vec::new();
        context
            .decode_raw_bytes(bytes, DecodeOptions::default(), &mut output)
            .unwrap();
        output
    };
    let reused = context();
    for bytes in STREAMS.into_iter().chain(STREAMS) {
        assert_eq!(decode(&reused, bytes), decode(&context(), bytes));
    }
}

#[test]
fn context_validates_simd_policy_at_startup() {
    use crate::SelectedBackend;

    let ctx = DecodeContext::new(DecodeRuntimeConfig::default()).unwrap();
    #[cfg(target_arch = "x86_64")]
    assert_eq!(ctx.selected_backend(), SelectedBackend::Avx2);
    #[cfg(target_arch = "aarch64")]
    assert_eq!(ctx.selected_backend(), SelectedBackend::Neon);

    #[cfg(target_arch = "x86_64")]
    {
        use crate::{DecodeError, SimdPolicy};

        let err =
            DecodeContext::new(DecodeRuntimeConfig::default().with_simd_policy(SimdPolicy::Neon))
                .unwrap_err();
        assert!(
            matches!(err, DecodeError::UnsupportedBackend { .. }),
            "{err}"
        );

        if !splot_core::simd::detect_capabilities().has_avx512() {
            let err = DecodeContext::new(
                DecodeRuntimeConfig::default().with_simd_policy(SimdPolicy::Avx512),
            )
            .unwrap_err();
            assert!(
                matches!(err, DecodeError::UnsupportedBackend { .. }),
                "{err}"
            );
        }
    }
}

#[test]
fn with_typed_backend_matches_compatibility_routes_across_repeated_streams() {
    use crate::{DecodeScope, TypedDecodeContext};
    use splot_core::simd::SimdBackend;

    const STREAMS: [&[u8]; 3] = [
        include_bytes!("../../../tests/conformance/vectors/valid/syn-3frame-multiref-64x64.ivf"),
        include_bytes!(
            "../../../tests/conformance/vectors/valid/syn-3frame-deblock-subpu-chroma-64x32-10bit-q90.ivf"
        ),
        include_bytes!("../../../tests/conformance/vectors/valid/syn-filmgrain-intra-64x64.ivf"),
    ];

    struct BatchCheck<'a>(&'a DecodeContext, &'a [&'a [u8]]);

    impl DecodeScope<()> for BatchCheck<'_> {
        fn run<B: SimdBackend>(self, typed: &TypedDecodeContext<'_, B>) {
            assert_eq!(typed.selected_backend(), self.0.selected_backend());
            assert_eq!(B::KIND, typed.selected_backend());
            assert_eq!(typed.threads(), self.0.threads());
            assert_eq!(typed.frame_delay(), self.0.frame_delay());
            for &bytes in self.1 {
                let mut typed_raw = Vec::new();
                let mut compat_raw = Vec::new();
                typed
                    .decode_raw_bytes(bytes, DecodeOptions::default(), &mut typed_raw)
                    .unwrap();
                self.0
                    .decode_raw_bytes(bytes, DecodeOptions::default(), &mut compat_raw)
                    .unwrap();
                assert_eq!(typed_raw, compat_raw);

                let mut typed_y4m = Vec::new();
                let mut compat_y4m = Vec::new();
                typed
                    .decode_y4m_bytes(bytes, DecodeOptions::default(), &mut typed_y4m)
                    .unwrap();
                self.0
                    .decode_y4m_bytes(bytes, DecodeOptions::default(), &mut compat_y4m)
                    .unwrap();
                assert_eq!(typed_y4m, compat_y4m);

                assert_eq!(
                    typed
                        .decode_hash_report_bytes(bytes, DecodeOptions::default())
                        .unwrap(),
                    self.0
                        .decode_hash_report_bytes(bytes, DecodeOptions::default())
                        .unwrap()
                );

                typed
                    .decode_discard_reader(bytes, DecodeOptions::default())
                    .unwrap();
            }
        }
    }

    let ctx = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(2usize))).unwrap();
    ctx.with_typed_backend(BatchCheck(&ctx, &STREAMS));
}

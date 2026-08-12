// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use splot_core::headers::frame::GmType;

use super::tests::{decode_context, decode_fixture, frame_hashes, parse_inter_core_for_validation};
use crate::DecodeOptions;
use crate::error::DecodeError;

const SAME_REF_GLOBAL_GLOBAL_FIXTURE: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/syn-2frame-same-ref-global-global-64x64-q80.ivf"
);
const GLOBAL_TRANSLATION_FIXTURE: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/syn-2frame-global-translation-64x64-q80.ivf"
);
const GLOBAL_ROTZOOM_FIXTURE: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/syn-2frame-global-rotzoom-64x64-q80.ivf"
);
const GLOBAL_AFFINE_FIXTURE: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/syn-2frame-global-affine-64x64-q80.ivf"
);

#[test]
fn same_ref_global_global_fixture_decodes_bit_exact() {
    let (sequence, core, _) = decode_context()
        .pool()
        .install(|| parse_inter_core_for_validation(SAME_REF_GLOBAL_GLOBAL_FIXTURE))
        .expect("same-reference GLOBAL_GLOBALMV fixture inter header parses");
    assert_eq!(
        sequence
            .inter
            .as_ref()
            .expect("fixture has inter sequence configuration")
            .num_same_ref_compound,
        2
    );
    assert!(
        core.inter_tail
            .as_ref()
            .expect("fixture has inter tail")
            .reference_select
    );

    let frames = decode_fixture(SAME_REF_GLOBAL_GLOBAL_FIXTURE);
    assert_eq!(frames.len(), 2, "key frame + same-reference compound frame");
    assert_eq!(
        frame_hashes(&frames),
        [
            "ebf2ba02fa61281e66533bc142260d49971a96101442d7df7d099b1d2be3bad5",
            "ebf2ba02fa61281e66533bc142260d49971a96101442d7df7d099b1d2be3bad5",
        ],
        "frame hashes pinned from byte-identical AVM raw output"
    );
}

#[test]
fn same_ref_global_global_fixture_rejects_truncated_payload() {
    let truncated = &SAME_REF_GLOBAL_GLOBAL_FIXTURE[..SAME_REF_GLOBAL_GLOBAL_FIXTURE.len() - 1];
    let error = decode_context()
        .plan_bytes(truncated, DecodeOptions::default())
        .expect_err("truncating the coded compound block must fail closed");
    assert!(matches!(error, DecodeError::MalformedSource { .. }));
}

#[test]
fn global_motion_fixtures_match_reference_hashes() {
    for (name, fixture, gm_type, gm_params) in [
        (
            "translation",
            GLOBAL_TRANSLATION_FIXTURE,
            GmType::RotZoom,
            [131_072, 65_536, 65_536, 0, 0, 65_536],
        ),
        (
            "rotzoom",
            GLOBAL_ROTZOOM_FIXTURE,
            GmType::RotZoom,
            [131_072, 65_536, 65_600, 256, -256, 65_600],
        ),
        (
            "affine",
            GLOBAL_AFFINE_FIXTURE,
            GmType::Affine,
            [131_072, 65_536, 65_600, 256, -128, 65_728],
        ),
    ] {
        let (_, core, _) = decode_context()
            .pool()
            .install(|| parse_inter_core_for_validation(fixture))
            .unwrap_or_else(|error| panic!("{name} header parse failed: {error}"));
        let global_motion = &core
            .inter_tail
            .as_ref()
            .expect("fixture has inter tail")
            .global_motion;
        assert!(global_motion.use_global_motion, "{name}");
        assert_eq!(global_motion.stop, None, "{name}");
        assert_eq!(global_motion.references[0].gm_type, gm_type, "{name}");
        assert_eq!(global_motion.references[0].gm_params, gm_params, "{name}");

        let frames = decode_fixture(fixture);
        assert_eq!(frames.len(), 2, "{name}");
        assert_eq!(
            frame_hashes(&frames),
            [
                "f6d062ea309723896a05236c707b09d51f3d746dd223d529c8254a19753cdc77",
                "f6d062ea309723896a05236c707b09d51f3d746dd223d529c8254a19753cdc77",
            ],
            "{name} frame hashes pinned from byte-identical AVM raw output"
        );
    }
}

#[test]
fn global_candidate_fact_survives_force_integer_reconstruction_gating() {
    let (_, core, _) = decode_context()
        .pool()
        .install(|| parse_inter_core_for_validation(GLOBAL_AFFINE_FIXTURE))
        .expect("global-affine fixture inter header parses");

    assert!(super::block::is_global_mv_candidate(&core, 0, 2, 2));
    assert_eq!(super::block::global_motion_warp(&core, 0, true, 2, 2), None);
    assert!(!super::block::is_global_mv_candidate(&core, 0, 1, 2));
}

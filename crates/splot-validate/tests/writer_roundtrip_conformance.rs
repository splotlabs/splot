// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Cross-tool agreement (`ENC-BITSTREAM-WRITER`): when the `splot-core` WRITER re-emits an
//! already-conformant bitstream, the re-emission must still pass the `splot-validate` VALIDATOR with
//! zero error diagnostics. (This is *not* a claim that every writer output is conformant — the writer
//! serializes any encodable model, including a non-conformant one; it reproduces its input rather than
//! validating it.) This is the complement of the `parse → write → reparse` round-trip harness
//! (`splot-core` `write::roundtrip_obu`): there the writer is checked against the parser, here the
//! re-emission of a conformant stream is checked against the validator.
//!
//! `splot-validate` already depends on `splot-core`, so this integration test can drive both the
//! writer (`splot_core::write::*`) and the validator (`splot_validate::Validator`) in one process,
//! obeying the one-way crate dependency rule (nothing — including this test crate — makes the writer
//! depend on the validator).
//!
//! It reuses the three committed conformant fixtures whose OBUs are all *writable* types (temporal
//! delimiter + padding / metadata HDR_CLL — `tests/fixtures/{padding,metadata-short,metadata-group}.av2`,
//! all `expect = "clean"` in `MANIFEST.toml`): it re-emits each through the complete-OBU writer and
//! asserts the re-emission is byte-exact and still validator-clean.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use splot_core::annexb::parse_annex_b_obus;
use splot_core::obu::PayloadStatus;
use splot_core::write::{
    BitWriter, RoundtripOutcome, recover_roundtrip_passthrough, roundtrip_obu, write_complete_obu,
};
use splot_validate::Validator;

/// The committed conformant fixtures whose OBUs are all writable types, so the whole stream can be
/// re-emitted through the writer. (Frame-carrying fixtures are excluded: tile-group / SEF / TIP OBUs
/// have no body writer yet.)
const CONFORMANT_WRITABLE_FIXTURES: &[&str] =
    &["padding.av2", "metadata-short.av2", "metadata-group.av2"];

/// The repository `tests/fixtures/` directory, resolved from this crate's manifest dir so the test is
/// location-independent (mirrors `crates/splot-cli/tests/fixture_manifest.rs`).
fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .canonicalize()
        .expect("tests/fixtures/ exists")
}

/// Re-emits an Annex B stream by parsing every OBU and writing it back through the complete-OBU
/// writer (`write_complete_obu` + the `leb128(num_bytes_in_obu)` size prefix), asserting each OBU
/// also round-trips via `roundtrip_obu`. Panics (the test signal) if the stream holds a non-writable
/// OBU type.
fn rewrite_annexb(bytes: &[u8]) -> Vec<u8> {
    let obus = parse_annex_b_obus(bytes).expect("fixture parses as Annex B");
    let mut out = BitWriter::new();
    for env in &obus {
        let parsed = match env.payload_status().expect("payload parses") {
            PayloadStatus::Parsed(parsed) => parsed,
            other => panic!("fixture holds a non-writable OBU payload: {other:?}"),
        };
        assert_eq!(
            roundtrip_obu(&env.header, env.payload, &parsed),
            RoundtripOutcome::RoundTripped,
            "an OBU did not round-trip through the writer"
        );
        let passthrough =
            recover_roundtrip_passthrough(env.payload, &parsed).expect("recover passthrough");
        let mut complete = BitWriter::new();
        write_complete_obu(&mut complete, &env.header, &parsed, &passthrough)
            .expect("write complete OBU");
        let complete = complete.into_bytes();
        let total = u32::try_from(complete.len()).expect("OBU size fits u32");
        out.write_leb128(total).expect("write size prefix");
        out.write_le(&complete).expect("write OBU bytes");
    }
    out.into_bytes()
}

#[test]
fn writer_reemission_of_conformant_fixtures_stays_conformant() {
    let root = fixtures_root();
    // Non-strict matches the CLI default and the manifest's `expect = "clean"` declaration.
    let validator = Validator::new(false);

    for name in CONFORMANT_WRITABLE_FIXTURES {
        let original =
            std::fs::read(root.join(name)).unwrap_or_else(|e| panic!("read {name}: {e}"));

        // Sanity: the committed fixture is itself conformant (guards a stale or edited fixture).
        assert!(
            validator.validate_bytes(&original).is_conformant(),
            "fixture {name} is not validator-clean to begin with"
        );

        // Cross-tool agreement: the writer's re-emission is byte-exact (these fixtures are canonical
        // and carry no opaque non-zero blob) and still reports zero error diagnostics.
        let rewritten = rewrite_annexb(&original);
        assert_eq!(
            rewritten, original,
            "writer re-emission of {name} is not byte-exact to the canonical fixture"
        );
        let report = validator.validate_bytes(&rewritten);
        assert!(
            report.is_conformant(),
            "writer re-emission of {name} has error diagnostics: {:?}",
            report
                .errors()
                .map(|d| d.rule_id.as_str())
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn validator_clean_check_is_not_vacuous() {
    // Guard the positive test above: prove `is_conformant()` can be false, so a clean result is
    // meaningful and not an artifact of the validator accepting everything. A truncated leb128 size
    // prefix is a structural error the validator must flag (and never panic on).
    let validator = Validator::new(false);
    let report = validator.validate_bytes(&[0xFF]);
    assert!(
        !report.is_conformant(),
        "a structurally broken stream must produce at least one error diagnostic"
    );
}

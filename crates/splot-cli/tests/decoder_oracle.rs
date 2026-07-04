// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Committed decoder-output oracle runner (CONF-AVM-DECODE-ORACLE).
//!
//! Loads `tests/conformance/decoder-oracle.toml`, decodes each committed fixture
//! **in-process** through `splot_decode::DecodeContext` (the same library path the
//! CLI `decode` command uses), and asserts the manifest's expected outcome:
//!
//! - `must_pass`: `splot decode --output-format raw` output SHA-256 equals the
//!   recorded AVM oracle hash (`avmdec --i420 --rawvideo`, visible I420 samples).
//! - `xfail_splot`: decode fails closed with the recorded
//!   `decode/unsupported-feature` diagnostic (rule id, unsupported reason, matrix
//!   row). An unexpected success is reported as XPASS but does not fail normal CI.
//!
//! This is the CI gate: it runs under `cargo test`, hence `cargo xtask ci`. There
//! is NO AVM dependency — the runner compares `splot` against oracle hashes that
//! were recorded offline (see docs/decoder/AVM-FIXTURE-CORPUS.md). AVM is never
//! invoked and the network is never touched.
//!
//! Set `SPLOT_DECODER_ORACLE_STRICT_XPASS=1` to make an XPASS fail (so a fixture
//! that now decodes can be upgraded to `must_pass`).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};
use splot_decode::{
    DecodeContext, DecodeDiagnosticDetails, DecodeDiagnosticReport, DecodeOptions,
    DecodeRuntimeConfig,
};
use splot_parallel::ThreadCount;

/// The manifest root: an array of `[[fixture]]` entries.
#[derive(Debug, Deserialize)]
struct Manifest {
    #[serde(default)]
    fixture: Vec<Fixture>,
}

/// One decoder-oracle manifest entry (only the runner-relevant fields).
#[derive(Debug, Deserialize)]
struct Fixture {
    id: String,
    /// `.ivf` path relative to `tests/conformance/`.
    path: String,
    /// `must_pass` | `xfail_splot` | `avm_oracle_only` | `blocked`.
    status: String,
    hashes: Hashes,
    expected_splot: ExpectedSplot,
}

#[derive(Debug, Deserialize)]
struct Hashes {
    ivf_sha256: String,
    avm_raw_i420_sha256: String,
}

#[derive(Debug, Deserialize)]
struct ExpectedSplot {
    #[allow(dead_code)]
    kind: String,
    #[serde(default)]
    rule_id: Option<String>,
    #[serde(default)]
    unsupported_reason: Option<String>,
    #[serde(default)]
    matrix_row: Option<String>,
}

/// Locates `tests/conformance/` from the crate manifest dir (`crates/splot-cli`).
fn conformance_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/conformance")
        .canonicalize()
        .expect("tests/conformance/ exists")
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// Decodes `bytes` to the raw I420 visible-sample stream, serially and
/// deterministically, exactly as `splot decode --output-format raw` does.
fn decode_raw(bytes: &[u8]) -> splot_decode::Result<Vec<u8>> {
    let ctx = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize)))
        .expect("decode context");
    let mut out = Vec::new();
    ctx.decode_raw_bytes(bytes, DecodeOptions::default(), &mut out)?;
    Ok(out)
}

#[test]
fn decoder_oracle_corpus_matches_manifest() {
    let root = conformance_root();
    let manifest_path = root.join("decoder-oracle.toml");
    let manifest_text = std::fs::read_to_string(&manifest_path).expect("read decoder-oracle.toml");
    let manifest: Manifest = toml::from_str(&manifest_text).expect("parse decoder-oracle.toml");

    assert!(
        !manifest.fixture.is_empty(),
        "manifest {} has no [[fixture]] entries",
        manifest_path.display()
    );

    let mut saw_must_pass = false;
    let mut saw_xfail = false;
    let mut xpasses: Vec<String> = Vec::new();
    let mut manifest_ivf: BTreeSet<PathBuf> = BTreeSet::new();

    for fx in &manifest.fixture {
        let ivf_path = root.join(&fx.path);
        manifest_ivf.insert(ivf_path.clone());
        let bytes =
            std::fs::read(&ivf_path).unwrap_or_else(|e| panic!("read {}: {e}", ivf_path.display()));

        assert_eq!(
            sha256_hex(&bytes),
            fx.hashes.ivf_sha256,
            "fixture {} ivf bytes do not match recorded ivf_sha256",
            fx.id
        );

        match fx.status.as_str() {
            "must_pass" => {
                saw_must_pass = true;
                let raw = decode_raw(&bytes).unwrap_or_else(|e| {
                    panic!("must_pass fixture {} failed to decode: {e}", fx.id)
                });
                assert_eq!(
                    sha256_hex(&raw),
                    fx.hashes.avm_raw_i420_sha256,
                    "must_pass fixture {} raw output does not match the AVM oracle hash",
                    fx.id
                );
            }
            "xfail_splot" => {
                saw_xfail = true;
                match decode_raw(&bytes) {
                    Ok(raw) => {
                        let correct = sha256_hex(&raw) == fx.hashes.avm_raw_i420_sha256;
                        xpasses.push(format!(
                            "{} (output {} AVM oracle)",
                            fx.id,
                            if correct { "MATCHES" } else { "DIFFERS from" }
                        ));
                    }
                    Err(error) => {
                        let report = DecodeDiagnosticReport::from_decode_error(&error)
                            .unwrap_or_else(|| {
                                panic!(
                                    "xfail fixture {} produced a non-reportable error: {error}",
                                    fx.id
                                )
                            });
                        let want_rule =
                            fx.expected_splot.rule_id.as_deref().unwrap_or_else(|| {
                                panic!("xfail fixture {} missing rule_id", fx.id)
                            });
                        assert_eq!(
                            report.diagnostic.rule_id, want_rule,
                            "xfail fixture {} rule id mismatch",
                            fx.id
                        );
                        let reason = match &report.details {
                            DecodeDiagnosticDetails::UnsupportedFeature(d) => d.unsupported_reason,
                            DecodeDiagnosticDetails::UnsupportedStructure(d) => {
                                d.unsupported_reason
                            }
                            other => panic!(
                                "xfail fixture {} produced unexpected diagnostic details: {other:?}",
                                fx.id
                            ),
                        };
                        if let Some(want_reason) = fx.expected_splot.unsupported_reason.as_deref() {
                            assert_eq!(
                                reason, want_reason,
                                "xfail fixture {} unsupported_reason mismatch",
                                fx.id
                            );
                        }
                        if let Some(want_row) = fx.expected_splot.matrix_row.as_deref() {
                            assert_eq!(
                                report.diagnostic.matrix_row, want_row,
                                "xfail fixture {} matrix_row mismatch",
                                fx.id
                            );
                        }
                    }
                }
            }
            "avm_oracle_only" | "blocked" => {}
            other => panic!("fixture {} has unknown status {other:?}", fx.id),
        }
    }

    assert!(
        saw_must_pass,
        "manifest exercises no `must_pass` fixture; the oracle-compare arm would be vacuous"
    );
    assert!(
        saw_xfail,
        "manifest exercises no `xfail_splot` fixture; the fail-closed arm would be vacuous"
    );

    let mut committed = Vec::new();
    collect_ivf_files(&root.join("vectors/valid"), &mut committed);
    let orphans: Vec<String> = committed
        .iter()
        .filter(|p| !manifest_ivf.contains(*p))
        .map(|p| p.display().to_string())
        .collect();
    assert!(
        orphans.is_empty(),
        "committed valid .ivf vector(s) missing from decoder-oracle.toml (never differentially tested): {orphans:?}"
    );

    if !xpasses.is_empty() {
        let strict = std::env::var_os("SPLOT_DECODER_ORACLE_STRICT_XPASS").is_some();
        let msg = format!(
            "XPASS: {} `xfail_splot` fixture(s) now decode and should be upgraded to `must_pass`: {xpasses:?}",
            xpasses.len()
        );
        assert!(!strict, "{msg}");
        eprintln!("{msg}\n(set SPLOT_DECODER_ORACLE_STRICT_XPASS=1 to fail on XPASS locally)");
    }
}

/// Recursively collects committed `.ivf` files under `dir`.
fn collect_ivf_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_ivf_files(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("ivf") {
            out.push(path);
        }
    }
}

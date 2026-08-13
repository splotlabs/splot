// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Committed decoder-output oracle runner (CONF-AVM-DECODE-ORACLE).
//!
//! Decodes each committed fixture in-process through `splot_decode` and asserts
//! `tests/conformance/decoder-oracle.toml`: `must_pass` output SHA-256 equals the
//! recorded AVM oracle hash at every pool width in [`THREAD_LEGS`], so serial and
//! parallel decode arms are both differentially gated; `xfail_splot` fails closed
//! with the recorded `decode/unsupported-feature` reason/matrix row. CI gate, no
//! AVM, no network.
//! Set `SPLOT_DECODER_ORACLE_STRICT_XPASS=1` to fail on an unexpected pass.

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

const UNSUPPORTED_RULE: &str = "decode/unsupported-feature";

/// Serial arm plus a pool wide enough to select every parallel decode arm
/// (the widest gate today is `current_pool_width() >= 4`).
const THREAD_LEGS: [usize; 2] = [1, 8];

const SB256_INTRA_FIXTURE: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-sb256-intra-129x16-q180.ivf");
const SB256_INTRA_IVF_SHA256: &str =
    "bfdf8c13d29f022dfbc1abd03f69efd51d6361bb51ae600af7812a2df11fedaf";
const SB256_INTRA_RAW_SHA256: &str =
    "6304c67c4e126342e56bc55b26ef1750444fc3e55cde4416f7d385aba4226cc6";

#[derive(Deserialize)]
struct Manifest {
    vectors_dir: String,
    #[serde(default)]
    fixture: Vec<Fixture>,
}

#[derive(Deserialize)]
struct Fixture {
    id: String,
    status: String,
    ivf_sha256: String,
    avm_raw_sha256: String,
    #[serde(default)]
    unsupported_reason: Option<String>,
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut hex = String::with_capacity(64);
    for b in Sha256::digest(bytes) {
        let _ = write!(hex, "{b:02x}");
    }
    hex
}

fn decode_raw(bytes: &[u8], threads: usize) -> splot_decode::Result<Vec<u8>> {
    let ctx = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(threads)))
        .expect("decode context");
    let mut out = Vec::new();
    ctx.decode_raw_bytes(bytes, DecodeOptions::default(), &mut out)?;
    Ok(out)
}

#[test]
fn decoder_oracle_corpus_matches_manifest() {
    let root = repo_root();
    let manifest_text = std::fs::read_to_string(root.join("tests/conformance/decoder-oracle.toml"))
        .expect("read decoder-oracle.toml");
    let manifest: Manifest = toml::from_str(&manifest_text).expect("parse decoder-oracle.toml");
    let vectors = root.join(&manifest.vectors_dir);
    assert!(!manifest.fixture.is_empty(), "manifest has no fixtures");

    let mut saw_pass = false;
    let mut xpasses: Vec<String> = Vec::new();
    let mut ids: BTreeSet<&str> = BTreeSet::new();

    for fx in &manifest.fixture {
        assert!(ids.insert(&fx.id), "duplicate fixture id {}", fx.id);
        let bytes = std::fs::read(vectors.join(format!("{}.ivf", fx.id)))
            .unwrap_or_else(|e| panic!("read {}.ivf: {e}", fx.id));
        assert_eq!(sha256_hex(&bytes), fx.ivf_sha256, "{} ivf hash", fx.id);

        match fx.status.as_str() {
            "must_pass" => {
                saw_pass = true;
                for threads in THREAD_LEGS {
                    let raw = decode_raw(&bytes, threads).unwrap_or_else(|e| {
                        panic!(
                            "must_pass {} failed to decode at {threads} threads: {e}",
                            fx.id
                        )
                    });
                    assert_eq!(
                        sha256_hex(&raw),
                        fx.avm_raw_sha256,
                        "{} != AVM oracle at {threads} threads",
                        fx.id
                    );
                }
            }
            "xfail_splot" => match decode_raw(&bytes, 1) {
                Ok(raw) => {
                    assert_eq!(
                        sha256_hex(&raw),
                        fx.avm_raw_sha256,
                        "xfail {} decoded to output that DIFFERS from the AVM oracle: the \
                             decoder neither failed closed nor matched AVM (non-conforming output)",
                        fx.id
                    );
                    xpasses.push(fx.id.clone());
                }
                Err(error) => {
                    let report = DecodeDiagnosticReport::from_decode_error(&error)
                        .unwrap_or_else(|| panic!("xfail {} non-reportable: {error}", fx.id));
                    assert_eq!(
                        report.diagnostic.rule_id, UNSUPPORTED_RULE,
                        "{} rule",
                        fx.id
                    );
                    let reason = match &report.details {
                        DecodeDiagnosticDetails::UnsupportedFeature(d) => d.unsupported_reason,
                        DecodeDiagnosticDetails::UnsupportedStructure(d) => d.unsupported_reason,
                        other => panic!("xfail {} unexpected details: {other:?}", fx.id),
                    };
                    if let Some(want) = fx.unsupported_reason.as_deref() {
                        assert_eq!(reason, want, "{} reason", fx.id);
                    }
                }
            },
            other => panic!("{} unknown status {other:?}", fx.id),
        }
    }
    assert!(saw_pass, "corpus must exercise the must_pass arm");

    let orphans: Vec<String> = std::fs::read_dir(&vectors)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("ivf"))
        .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .filter(|stem| !ids.contains(stem.as_str()))
        .collect();
    assert!(
        orphans.is_empty(),
        "valid .ivf missing from manifest: {orphans:?}"
    );

    if !xpasses.is_empty() {
        let msg = format!("XPASS (upgrade to must_pass): {xpasses:?}");
        assert!(
            std::env::var_os("SPLOT_DECODER_ORACLE_STRICT_XPASS").is_none(),
            "{msg}"
        );
        eprintln!("{msg}");
    }
}

#[test]
fn sequence_sb256_intra_uses_effective_sb128_at_serial_and_parallel_widths() {
    assert_eq!(sha256_hex(SB256_INTRA_FIXTURE), SB256_INTRA_IVF_SHA256);
    for threads in THREAD_LEGS {
        let raw = decode_raw(SB256_INTRA_FIXTURE, threads)
            .unwrap_or_else(|error| panic!("SB256 intra failed at {threads} threads: {error}"));
        assert_eq!(sha256_hex(&raw), SB256_INTRA_RAW_SHA256);
    }

    let truncated = &SB256_INTRA_FIXTURE[..SB256_INTRA_FIXTURE.len() - 1];
    assert!(matches!(
        decode_raw(truncated, 8),
        Err(splot_decode::DecodeError::MalformedSource { .. })
    ));
}

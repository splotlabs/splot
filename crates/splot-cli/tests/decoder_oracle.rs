// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Committed decoder-output oracle runner (CONF-AVM-DECODE-ORACLE).
//!
//! Decodes each committed fixture in-process through `splot_decode` and asserts
//! `tests/conformance/decoder-oracle.toml`: output SHA-256 equals the recorded
//! AVM oracle hash at every pool width in [`THREAD_LEGS`], so serial and parallel
//! decode arms are both differentially gated. CI gate, no AVM, no network.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};
use splot_decode::{DecodeContext, DecodeOptions, DecodeRuntimeConfig};
use splot_parallel::ThreadCount;

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
    schema_version: u32,
    vectors_dir: String,
    #[serde(default)]
    fixture: Vec<Fixture>,
}

#[derive(Deserialize)]
struct Fixture {
    id: String,
    ivf_sha256: String,
    avm_raw_sha256: String,
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
    assert_eq!(manifest.schema_version, 2, "decoder oracle schema");
    let vectors = root.join(&manifest.vectors_dir);
    assert!(!manifest.fixture.is_empty(), "manifest has no fixtures");

    let mut ids: BTreeSet<&str> = BTreeSet::new();

    for fx in &manifest.fixture {
        assert!(ids.insert(&fx.id), "duplicate fixture id {}", fx.id);
        let bytes = std::fs::read(vectors.join(format!("{}.ivf", fx.id)))
            .unwrap_or_else(|e| panic!("read {}.ivf: {e}", fx.id));
        assert_eq!(sha256_hex(&bytes), fx.ivf_sha256, "{} ivf hash", fx.id);

        for threads in THREAD_LEGS {
            let raw = decode_raw(&bytes, threads)
                .unwrap_or_else(|e| panic!("{} failed to decode at {threads} threads: {e}", fx.id));
            assert_eq!(
                sha256_hex(&raw),
                fx.avm_raw_sha256,
                "{} != AVM oracle at {threads} threads",
                fx.id
            );
        }
    }

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

/// A pass that fails mid-tile must settle, not spin the § 7.12 resolve chain.
///
/// The failure drives the parse watermark past every threshold, so a resolve
/// step stalled on a unit the pass will never publish has to recognise the
/// failure instead of waiting on an already-satisfied condition.
#[test]
fn truncated_tile_payload_settles_at_every_width() {
    let root = repo_root();
    let bytes = std::fs::read(
        root.join("tests/conformance/vectors/valid/syn-4frame-mono-inter-256x128.ivf"),
    )
    .expect("read fixture");
    let full = decode_raw(&bytes, 1).expect("intact stream decodes");
    assert!(!full.is_empty(), "fixture produced no samples");

    for cut in [bytes.len() / 3, bytes.len() / 2, bytes.len() * 3 / 4] {
        for threads in THREAD_LEGS {
            let outcome = decode_raw(&bytes[..cut], threads);
            assert!(
                outcome.map_or(0, |raw| raw.len()) < full.len(),
                "truncation at {cut} bytes decoded a whole stream at {threads} threads",
            );
        }
    }
}

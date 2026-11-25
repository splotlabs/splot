// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Committed decoder-output oracle runner (CONF-AVM-DECODE-ORACLE).
//!
//! Decodes each committed fixture in-process through `splot_decode` and asserts
//! `tests/conformance/decoder-oracle.toml`: output SHA-256 equals the recorded
//! AVM oracle hash at every pool width in [`THREAD_LEGS`], ensuring output is
//! independent of worker count. CI gate, no AVM, no network.

#![expect(clippy::expect_used, clippy::panic)]

#[path = "decoder_oracle/atlas.rs"]
mod atlas;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde::Deserialize;
mod common;
use common::sha256_hex;
use splot_core::stream_reader::{StreamUnit, TemporalUnitReader};
use splot_decode::{
    DataProps, DecodeContext, DecodeOptions, DecodeRuntimeConfig, Decoder, DecoderSettings,
    Picture, PictureSamples, PlaneId, SendStatus,
};
use splot_parallel::ThreadCount;

/// Representative one-worker and multiworker pool widths.
const THREAD_LEGS: [usize; 2] = [1, 8];

const SB256_INTRA_FIXTURE: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-sb256-intra-129x16-q180.ivf");
const SB256_INTRA_IVF_SHA256: &str =
    "bfdf8c13d29f022dfbc1abd03f69efd51d6361bb51ae600af7812a2df11fedaf";
const SB256_INTRA_RAW_SHA256: &str =
    "6304c67c4e126342e56bc55b26ef1750444fc3e55cde4416f7d385aba4226cc6";

/// Film grain fixtures with the SHA-256 of their
/// `avmdec --i420 --rawvideo --skip-film-grain` output: an 8-bit key frame,
/// and a 10-bit key frame with two inter frames.
const FILM_GRAIN_SKIPPED: [(&[u8], &str); 3] = [
    (
        include_bytes!("../../../tests/conformance/vectors/valid/syn-filmgrain-intra-64x64.ivf"),
        "e73a3b0168597953992650452b153d6d316f649254b2493864fb6d320a3d8f53",
    ),
    (
        include_bytes!(
            "../../../tests/conformance/vectors/valid/syn-3frame-filmgrain-inter-64x64-10bit.ivf"
        ),
        "5e4da3a2658836d7f15696188ecf895b16992d6fd033d30e4c62f579b81cdf89",
    ),
    (
        include_bytes!(
            "../../../tests/conformance/vectors/valid/syn-bru-inactive-filmgrain-8bit.ivf"
        ),
        "6571db3ede2a79b9568e602483e1f38b266f7b4cfe3408e9e0f011c33c39791b",
    ),
];

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

fn decode_raw(bytes: &[u8], threads: usize) -> splot_decode::Result<Vec<u8>> {
    decode_raw_with(bytes, threads, &DecodeOptions::default())
}

fn decode_raw_with(
    bytes: &[u8],
    threads: usize,
    options: &DecodeOptions,
) -> splot_decode::Result<Vec<u8>> {
    let ctx = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(threads)))
        .expect("decode context");
    let mut out = Vec::new();
    ctx.decode_raw_bytes(bytes, *options, &mut out)?;
    Ok(out)
}

/// Decodes `bytes` through the streaming [`Decoder`], one IVF record per
/// temporal unit, and lays the pictures out as the raw output does.
fn decode_streaming(bytes: &[u8], threads: usize) -> splot_decode::Result<Vec<u8>> {
    let mut decoder =
        Decoder::new(DecoderSettings::default().with_threads(ThreadCount::from(threads)))?;
    let mut out = Vec::new();
    let mut reader = TemporalUnitReader::new(bytes);
    while let Some(unit) = reader.next_unit().expect("IVF record") {
        let StreamUnit::IvfFrame { payload, .. } = unit else {
            continue;
        };
        while decoder.send_data(payload, DataProps::default())? == SendStatus::Full {
            while let Some(picture) = decoder.get_picture()? {
                append_raw(&picture, &mut out);
            }
        }
        while let Some(picture) = decoder.get_picture()? {
            append_raw(&picture, &mut out);
        }
    }
    decoder.drain();
    while let Some(picture) = decoder.get_picture()? {
        append_raw(&picture, &mut out);
    }
    Ok(out)
}

fn append_raw(picture: &Picture, out: &mut Vec<u8>) {
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

fn check_fixture(vectors: &Path, fx: &Fixture, threads: usize) {
    let bytes = std::fs::read(vectors.join(format!("{}.ivf", fx.id)))
        .unwrap_or_else(|e| panic!("read {}.ivf: {e}", fx.id));
    assert_eq!(sha256_hex(&bytes), fx.ivf_sha256, "{} ivf hash", fx.id);
    let raw = decode_raw(&bytes, threads)
        .unwrap_or_else(|e| panic!("{} failed to decode at {threads} threads: {e}", fx.id));
    assert_eq!(
        sha256_hex(&raw),
        fx.avm_raw_sha256,
        "{} != AVM oracle at {threads} threads",
        fx.id
    );
    let streamed = decode_streaming(&bytes, threads)
        .unwrap_or_else(|e| panic!("{} failed to stream at {threads} threads: {e}", fx.id));
    assert_eq!(
        sha256_hex(&streamed),
        fx.avm_raw_sha256,
        "{} streamed != AVM oracle at {threads} threads",
        fx.id
    );
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
    }

    let cores = std::thread::available_parallelism().map_or(1, usize::from);
    for threads in THREAD_LEGS {
        let next = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..(cores / threads).max(1) {
                scope.spawn(|| {
                    while let Some(fx) = manifest.fixture.get(next.fetch_add(1, Ordering::Relaxed))
                    {
                        check_fixture(&vectors, fx, threads);
                    }
                });
            }
        });
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

#[test]
fn skipped_film_grain_output_matches_avmdec_skip_film_grain() {
    let options = DecodeOptions::default().with_skip_film_grain(true);
    for (fixture, skipped_sha256) in FILM_GRAIN_SKIPPED {
        for threads in THREAD_LEGS {
            let raw = decode_raw_with(fixture, threads, &options)
                .unwrap_or_else(|error| panic!("film grain failed at {threads} threads: {error}"));
            assert_eq!(sha256_hex(&raw), skipped_sha256);
        }
    }
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

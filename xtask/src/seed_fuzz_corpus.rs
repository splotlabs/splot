// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The fuzz-corpus seeding task (`cargo xtask seed-fuzz-corpus`).
//!
//! Materializes the curated `fuzz/corpus/<target>/` seed inputs that the CI
//! fuzz-smoke matrix relies on, from the committed `tests/fixtures/*.av2`
//! corpus and the `tests/conformance/vectors/**/*.ivf` AVM streams. This is the
//! Rust home of logic that previously lived as ~100 lines of embedded shell and
//! Python in `.github/workflows/ci.yml`; moving it into `xtask` makes every
//! byte layout unit-testable, runnable locally (`cargo xtask seed-fuzz-corpus`),
//! and keeps the workflow declarative.
//!
//! The seed bytes are byte-identical to the former inline script. Each fuzz
//! target consumes a small leading config prefix before the bitstream, so the
//! seeds prepend the matching prefix to a fixture or conformance vector, embed a
//! few hand-tuned minimal payloads, synthesize IVF wrappers, and de-wrap IVF
//! frames into raw OBU streams. Every helper documents the exact layout it
//! reproduces.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};

use crate::fuzz_targets;

/// `validate_bytes` strict-mode config prefix (a leading config byte before the
/// auto-detected bitstream).
const VALIDATE_STRICT_PREFIX: &[u8] = &[0x01];
/// `validate_bytes` strict + external-HLS prefix (config byte plus a key byte).
const VALIDATE_HLS_PREFIX: &[u8] = &[0x03, 0xFF];
/// `decode_plan_bytes` leading limit-policy byte; the fixture bytes follow intact.
const DECODE_PLAN_PREFIX: &[u8] = &[0xFF];
/// `decode_runtime_{hash,raw,y4m}_bytes` leading raw-mode byte; the fixture bytes
/// follow intact.
const RUNTIME_RAW_PREFIX: &[u8] = &[0x00];

/// The three minimal decode-runtime targets that take a leading mode/limit byte.
const RUNTIME_TARGETS: [&str; 3] = [
    "decode_runtime_hash_bytes",
    "decode_runtime_raw_bytes",
    "decode_runtime_y4m_bytes",
];

/// Seeds the entire `fuzz/corpus/` tree under `root`, returning a one-line
/// summary. Mirrors `.github/workflows/ci.yml`'s former seeding step exactly.
///
/// # Errors
/// Returns an error if the fuzz target set cannot be enumerated or any seed file
/// cannot be written.
pub(crate) fn run_seed_fuzz_corpus(root: &Path) -> Result<()> {
    let targets = fuzz_targets(root)?;
    let summary = seed_corpus(root, &targets)?;
    println!(
        "seed-fuzz-corpus: ok ({} target(s), {} fixture(s), {} conformance vector(s))",
        targets.len(),
        summary.fixtures,
        summary.conformance,
    );
    Ok(())
}

/// Counts of the curated inputs consumed, for the summary line.
struct SeedSummary {
    fixtures: usize,
    conformance: usize,
}

/// A committed input file plus the corpus-seed name derived from it.
struct Seed {
    /// The seed name: a fixture basename (`foo.av2`) or a conformance vector's
    /// repo-relative path with `/` replaced by `-`.
    name: String,
    bytes: Vec<u8>,
}

/// Seeds every `fuzz/corpus/<target>/` directory from the committed fixtures and
/// conformance vectors. `targets` is the full fuzz target set (every target's
/// directory receives a raw copy of each fixture, matching the former
/// `for target in $(cargo fuzz list)` loop).
fn seed_corpus(root: &Path, targets: &[String]) -> Result<SeedSummary> {
    let fixtures = load_fixtures(root)?;
    let conformance = load_conformance(root)?;

    for target in targets {
        let dir = corpus_dir(root, target);
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("failed to create {}", dir.display()))?;
        for fixture in &fixtures {
            write_seed(root, target, &fixture.name, &fixture.bytes)?;
        }
    }

    seed_static(root)?;
    seed_fixtures(root, &fixtures)?;
    seed_conformance(root, &conformance)?;

    Ok(SeedSummary {
        fixtures: fixtures.len(),
        conformance: conformance.len(),
    })
}

/// Writes the hand-tuned, input-independent seeds: minimal valid payloads for the
/// symbol-decoder, tile-payload, decode-runtime, and recon targets. The byte
/// layouts are transcribed verbatim from the former workflow `printf` seeds.
fn seed_static(root: &Path) -> Result<()> {
    write_seed(
        root,
        "symbol_decoder_bytes",
        "finish-valid-two-byte-payload",
        &[0x02, 0x02, 0x80, 0x00, 0x01, 0x05],
    )?;
    write_seed(
        root,
        "tile_payload_decode_bytes",
        "frontier-good-payload",
        &[0x06, 0x02, 0x7F, 0x00],
    )?;
    for target in RUNTIME_TARGETS {
        write_seed(root, target, "minimal-fixture-unmutated", &[0x80, 0x00])?;
    }
    write_seed(
        root,
        "recon_frame_hash_bytes",
        "mono-8bit-visible",
        &[
            0x00, 0x02, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x02, 0x03, 0x04,
        ],
    )?;
    write_seed(
        root,
        "recon_frame_plane_types_bytes",
        "mono-8bit-runtime-types",
        &[
            0x01, 0x00, 0x00, 0x03, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x02, 0x03, 0x04,
        ],
    )?;
    write_seed(
        root,
        "recon_reference_frame_store_bytes",
        "basic-state-machine",
        &[
            0x03, 0x03, 0x05, 0x04, 0x00, 0x01, 0x00, 0x02, 0x03, 0x04, 0x05, 0x06, 0x03, 0x00,
            0x07, 0x05, 0x00, 0x03, 0x00, 0x07,
        ],
    )?;
    Ok(())
}

/// Seeds the per-fixture derivatives: config-prefixed copies for the validator,
/// planner, and decode-runtime targets, plus a synthesized IVF wrapper for
/// `parse_ivf`.
fn seed_fixtures(root: &Path, fixtures: &[Seed]) -> Result<()> {
    for fixture in fixtures {
        let name = &fixture.name;
        let bytes = &fixture.bytes;
        write_seed(
            root,
            "validate_bytes",
            &format!("strict-{name}"),
            &prefixed(VALIDATE_STRICT_PREFIX, bytes),
        )?;
        write_seed(
            root,
            "validate_bytes",
            &format!("hls-{name}"),
            &prefixed(VALIDATE_HLS_PREFIX, bytes),
        )?;
        write_seed(
            root,
            "decode_plan_bytes",
            &format!("fixture-{name}"),
            &prefixed(DECODE_PLAN_PREFIX, bytes),
        )?;
        for target in RUNTIME_TARGETS {
            write_seed(
                root,
                target,
                &format!("raw-{name}"),
                &prefixed(RUNTIME_RAW_PREFIX, bytes),
            )?;
        }
        write_seed(root, "parse_ivf", &format!("ivf-{name}"), &ivf_wrap(bytes))?;
    }
    Ok(())
}

/// Seeds the conformance-vector derivatives: config-prefixed copies for the
/// validator, planner, and decode-runtime targets, plus de-wrapped raw OBU
/// streams for `parse_obu` and `parse_bitstream`.
fn seed_conformance(root: &Path, conformance: &[Seed]) -> Result<()> {
    for vector in conformance {
        let name = &vector.name;
        let bytes = &vector.bytes;
        write_seed(root, "parse_ivf", &format!("conf-{name}"), bytes)?;
        write_seed(
            root,
            "validate_bytes",
            &format!("conf-strict-{name}"),
            &prefixed(VALIDATE_STRICT_PREFIX, bytes),
        )?;
        write_seed(
            root,
            "validate_bytes",
            &format!("conf-hls-{name}"),
            &prefixed(VALIDATE_HLS_PREFIX, bytes),
        )?;
        write_seed(
            root,
            "decode_plan_bytes",
            &format!("conf-{name}"),
            &prefixed(DECODE_PLAN_PREFIX, bytes),
        )?;
        for target in RUNTIME_TARGETS {
            write_seed(
                root,
                target,
                &format!("conf-raw-{name}"),
                &prefixed(RUNTIME_RAW_PREFIX, bytes),
            )?;
        }
        if let Some(obu) = ivf_dewrap(bytes) {
            let tag = format!("conf-{name}.obu");
            write_seed(root, "parse_obu", &tag, &obu)?;
            write_seed(root, "parse_bitstream", &tag, &obu)?;
        }
    }
    Ok(())
}

/// Returns `prefix` concatenated with `body` (the seed = leading config bytes
/// then the bitstream).
fn prefixed(prefix: &[u8], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(prefix.len() + body.len());
    out.extend_from_slice(prefix);
    out.extend_from_slice(body);
    out
}

/// Wraps `data` in a minimal 32-byte IVF header plus one 12-byte frame header so
/// a raw Annex-B fixture exercises `parse_ivf`'s valid path. Reproduces the
/// former Python `struct.pack("<4sHH4sHHIIII", b"DKIF", 0, 32, b"AV02", 64, 64,
/// 30, 1, 1, 0)` header and `struct.pack("<IQ", len(data), 0)` frame header.
fn ivf_wrap(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(32 + 12 + data.len());
    out.extend_from_slice(b"DKIF"); // signature
    out.extend_from_slice(&0u16.to_le_bytes()); // version
    out.extend_from_slice(&32u16.to_le_bytes()); // header length
    out.extend_from_slice(b"AV02"); // codec fourcc
    out.extend_from_slice(&64u16.to_le_bytes()); // width
    out.extend_from_slice(&64u16.to_le_bytes()); // height
    out.extend_from_slice(&30u32.to_le_bytes()); // framerate numerator
    out.extend_from_slice(&1u32.to_le_bytes()); // framerate denominator
    out.extend_from_slice(&1u32.to_le_bytes()); // frame count
    out.extend_from_slice(&0u32.to_le_bytes()); // reserved
    out.extend_from_slice(&(data.len() as u32).to_le_bytes()); // frame size
    out.extend_from_slice(&0u64.to_le_bytes()); // timestamp
    out.extend_from_slice(data);
    out
}

/// De-wraps an IVF stream into the concatenation of its frame payloads (the raw
/// §5 low-overhead OBU bytes). Reproduces the former Python de-wrap loop: read
/// the header length from bytes `[6:8]`, then for each 12-byte frame header at
/// `off` take `size` (u32 LE at `[off..off+4]`) payload bytes after it. Returns
/// `None` for inputs shorter than the 32-byte IVF header (the Python
/// `if len(d) < 32: continue`).
fn ivf_dewrap(data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < 32 {
        return None;
    }
    let header_len = u16::from_le_bytes([data[6], data[7]]) as usize;
    let mut off = header_len;
    let mut obu = Vec::new();
    while off + 12 <= data.len() {
        let size =
            u32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]) as usize;
        off += 12;
        let end = off.saturating_add(size).min(data.len());
        obu.extend_from_slice(&data[off..end]);
        off = off.saturating_add(size);
    }
    Some(obu)
}

/// Loads `tests/fixtures/*.av2`, sorted by path, naming each seed by its
/// basename (matching the former `basename "$f"`).
fn load_fixtures(root: &Path) -> Result<Vec<Seed>> {
    let dir = root.join("tests").join("fixtures");
    let mut paths = list_files(&dir, "av2")?;
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let bytes = std::fs::read(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            Ok(Seed { name, bytes })
        })
        .collect()
}

/// Loads `tests/conformance/vectors/**/*.ivf`, sorted by path, naming each seed
/// by its repo-relative path with `/` replaced by `-` (matching the former
/// shell `tr '/' '-'` and Python `path.replace("/", "-")`).
fn load_conformance(root: &Path) -> Result<Vec<Seed>> {
    let dir = root.join("tests").join("conformance").join("vectors");
    let mut paths = Vec::new();
    list_files_recursive(&dir, "ivf", &mut paths)?;
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let rel = path.strip_prefix(root).unwrap_or(&path);
            let name = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("-");
            let bytes = std::fs::read(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            Ok(Seed { name, bytes })
        })
        .collect()
}

/// Returns `root/fuzz/corpus/<target>`.
fn corpus_dir(root: &Path, target: &str) -> PathBuf {
    root.join("fuzz").join("corpus").join(target)
}

/// Writes `bytes` to `root/fuzz/corpus/<target>/<name>`, creating the target's
/// corpus directory if needed.
fn write_seed(root: &Path, target: &str, name: &str, bytes: &[u8]) -> Result<()> {
    let dir = corpus_dir(root, target);
    std::fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
    let path = dir.join(name);
    std::fs::write(&path, bytes).with_context(|| format!("failed to write {}", path.display()))
}

/// Returns the immediate `*.ext` files under `dir`, or an empty vec if `dir` does
/// not exist.
fn list_files(dir: &Path, ext: &str) -> Result<Vec<PathBuf>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    for entry in
        std::fs::read_dir(dir).with_context(|| format!("failed to read {}", dir.display()))?
    {
        let entry = entry.with_context(|| format!("failed to read entry in {}", dir.display()))?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some(ext) {
            files.push(path);
        }
    }
    Ok(files)
}

/// Recursively collects `*.ext` files under `dir` into `out`; a missing `dir` is
/// a no-op.
fn list_files_recursive(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in
        std::fs::read_dir(dir).with_context(|| format!("failed to read {}", dir.display()))?
    {
        let entry = entry.with_context(|| format!("failed to read entry in {}", dir.display()))?;
        let path = entry.path();
        if path.is_dir() {
            list_files_recursive(&path, ext, out)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some(ext) {
            out.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::util::temp_root;

    #[test]
    fn prefixed_prepends_config_bytes() {
        assert_eq!(
            prefixed(&[0x03, 0xFF], &[0xAA, 0xBB]),
            vec![0x03, 0xFF, 0xAA, 0xBB]
        );
        assert_eq!(prefixed(&[], &[0x01]), vec![0x01]);
    }

    #[test]
    fn ivf_wrap_emits_32_byte_header_plus_12_byte_frame() {
        let data = [0xDE, 0xAD, 0xBE, 0xEF];
        let wrapped = ivf_wrap(&data);
        assert_eq!(wrapped.len(), 32 + 12 + data.len());
        assert_eq!(&wrapped[0..4], b"DKIF");
        assert_eq!(u16::from_le_bytes([wrapped[4], wrapped[5]]), 0); // version
        assert_eq!(u16::from_le_bytes([wrapped[6], wrapped[7]]), 32); // header length
        assert_eq!(&wrapped[8..12], b"AV02");
        assert_eq!(u16::from_le_bytes([wrapped[12], wrapped[13]]), 64); // width
        assert_eq!(u16::from_le_bytes([wrapped[14], wrapped[15]]), 64); // height
        assert_eq!(
            u32::from_le_bytes([wrapped[32], wrapped[33], wrapped[34], wrapped[35]]),
            data.len() as u32
        );
        assert_eq!(&wrapped[44..], &data);
    }

    #[test]
    fn ivf_dewrap_round_trips_a_single_frame() {
        let payload = [0x12, 0x34, 0x56];
        let wrapped = ivf_wrap(&payload);
        assert_eq!(ivf_dewrap(&wrapped).as_deref(), Some(&payload[..]));
    }

    #[test]
    fn ivf_dewrap_concatenates_multiple_frames_and_honors_header_len() {
        let mut ivf = Vec::new();
        ivf.extend_from_slice(b"DKIF");
        ivf.extend_from_slice(&0u16.to_le_bytes());
        ivf.extend_from_slice(&32u16.to_le_bytes());
        ivf.extend_from_slice(b"AV02");
        ivf.extend_from_slice(&[0u8; 20]); // remainder of the 32-byte header
        for payload in [&[0xA1u8, 0xA2][..], &[0xB1, 0xB2, 0xB3][..]] {
            ivf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            ivf.extend_from_slice(&0u64.to_le_bytes());
            ivf.extend_from_slice(payload);
        }
        assert_eq!(ivf_dewrap(&ivf), Some(vec![0xA1, 0xA2, 0xB1, 0xB2, 0xB3]));
    }

    #[test]
    fn ivf_dewrap_rejects_short_input() {
        assert_eq!(ivf_dewrap(&[0u8; 31]), None);
    }

    #[test]
    fn seed_corpus_writes_byte_exact_seeds() {
        let root = temp_root("xtask-seed-fuzz-corpus").unwrap();
        let fixture = [0x10, 0x20, 0x30];
        std::fs::create_dir_all(root.join("tests/fixtures")).unwrap();
        std::fs::write(root.join("tests/fixtures/sample.av2"), fixture).unwrap();
        let vector = [0u8; 44]; // 32-byte header + one zero-length frame header
        std::fs::create_dir_all(root.join("tests/conformance/vectors/valid")).unwrap();
        let mut ivf = Vec::new();
        ivf.extend_from_slice(b"DKIF");
        ivf.extend_from_slice(&0u16.to_le_bytes());
        ivf.extend_from_slice(&32u16.to_le_bytes());
        ivf.extend_from_slice(b"AV02");
        ivf.extend_from_slice(&[0u8; 20]);
        ivf.extend_from_slice(&3u32.to_le_bytes()); // frame size 3
        ivf.extend_from_slice(&0u64.to_le_bytes());
        ivf.extend_from_slice(&[0x77, 0x88, 0x99]);
        std::fs::write(root.join("tests/conformance/vectors/valid/v.ivf"), &ivf).unwrap();
        let _ = vector;

        let targets = vec!["parse_ivf".to_string(), "validate_bytes".to_string()];
        let summary = seed_corpus(&root, &targets).unwrap();
        assert_eq!(summary.fixtures, 1);
        assert_eq!(summary.conformance, 1);

        let read = |rel: &str| std::fs::read(root.join("fuzz/corpus").join(rel)).unwrap();

        assert_eq!(read("parse_ivf/sample.av2"), fixture);
        assert_eq!(read("validate_bytes/sample.av2"), fixture);
        assert_eq!(
            read("validate_bytes/strict-sample.av2"),
            [0x01, 0x10, 0x20, 0x30]
        );
        assert_eq!(
            read("validate_bytes/hls-sample.av2"),
            [0x03, 0xFF, 0x10, 0x20, 0x30]
        );
        assert_eq!(
            read("decode_plan_bytes/fixture-sample.av2"),
            [0xFF, 0x10, 0x20, 0x30]
        );
        assert_eq!(
            read("decode_runtime_hash_bytes/raw-sample.av2"),
            [0x00, 0x10, 0x20, 0x30]
        );
        assert_eq!(read("parse_ivf/ivf-sample.av2"), ivf_wrap(&fixture));
        assert_eq!(
            read("symbol_decoder_bytes/finish-valid-two-byte-payload"),
            [0x02, 0x02, 0x80, 0x00, 0x01, 0x05]
        );
        assert_eq!(
            read("decode_runtime_y4m_bytes/minimal-fixture-unmutated"),
            [0x80, 0x00]
        );
        let conf = "tests-conformance-vectors-valid-v.ivf";
        assert_eq!(read(&format!("parse_ivf/conf-{conf}")), ivf);
        assert_eq!(
            read(&format!("validate_bytes/conf-hls-{conf}")),
            prefixed(&[0x03, 0xFF], &ivf)
        );
        assert_eq!(
            read(&format!("parse_obu/conf-{conf}.obu")),
            [0x77, 0x88, 0x99]
        );
        assert_eq!(
            read(&format!("parse_bitstream/conf-{conf}.obu")),
            [0x77, 0x88, 0x99]
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}

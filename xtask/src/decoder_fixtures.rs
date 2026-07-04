// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `cargo xtask decoder-fixtures {verify,report,coverage}` — the decoder-output
//! oracle harness over the reused conformance corpus (CONF-AVM-DECODE-ORACLE).
//!
//! - `verify` is a metadata-integrity gate (no decode, no AVM): it validates the
//!   manifest/taxonomy shape, fixture hashes, feature ids, size limits, and that
//!   every committed valid `.ivf` has an oracle entry. Wired into `cargo xtask ci`.
//! - `report` decodes each fixture with the built `splot` binary and prints a
//!   PASS / XFAIL / XPASS summary (ergonomic manual entry; the CI gate is the
//!   in-process `crates/splot-cli/tests/decoder_oracle.rs` test).
//! - `coverage` generates `docs/decoder/DECODER-ORACLE-COVERAGE.md`; `--check`
//!   fails on drift and is wired into `cargo xtask ci`.
//!
//! There is NO AVM dependency: the committed oracle hashes were recorded offline
//! (see docs/decoder/AVM-FIXTURE-CORPUS.md). AVM is never invoked here or in CI.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use crate::conformance::build_splot_binary;
use crate::git_util::sha256_hex;

const MANIFEST_PATH: &str = "tests/conformance/decoder-oracle.toml";
const TAXONOMY_PATH: &str = "tests/conformance/decoder-oracle-coverage.toml";
const COVERAGE_DOC_PATH: &str = "docs/decoder/DECODER-ORACLE-COVERAGE.md";
const REGEN_COMMAND: &str = "cargo xtask decoder-fixtures coverage";
const VALID_STATUSES: &[&str] = &["must_pass", "xfail_splot", "avm_oracle_only", "blocked"];

/// Decoder-oracle manifest (`tests/conformance/decoder-oracle.toml`).
#[derive(Debug, Deserialize)]
struct Manifest {
    vectors_root: String,
    max_frames: u64,
    #[serde(default)]
    fixture: Vec<Fixture>,
}

#[derive(Debug, Deserialize)]
struct Fixture {
    id: String,
    path: String,
    status: String,
    #[allow(dead_code)]
    width: u32,
    #[allow(dead_code)]
    height: u32,
    coded_frames: u64,
    shown_frames: u64,
    #[allow(dead_code)]
    bytes_per_sample: u8,
    #[serde(default)]
    features: Vec<String>,
    #[serde(default)]
    #[allow(dead_code)]
    spec_sections: Vec<String>,
    hashes: Hashes,
    expected_splot: ExpectedSplot,
}

#[derive(Debug, Deserialize)]
#[allow(
    clippy::struct_field_names,
    reason = "serde field names bind to the manifest schema"
)]
struct Hashes {
    ivf_sha256: String,
    avm_raw_i420_sha256: String,
    #[serde(default)]
    avm_raw_i420_frame_sha256: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ExpectedSplot {
    kind: String,
    #[serde(default)]
    rule_id: Option<String>,
    #[serde(default)]
    unsupported_reason: Option<String>,
    #[serde(default)]
    matrix_row: Option<String>,
    #[serde(default)]
    feature_id: Option<String>,
    #[serde(default)]
    detail_kind: Option<String>,
}

/// Coverage taxonomy (`tests/conformance/decoder-oracle-coverage.toml`).
#[derive(Debug, Deserialize)]
struct Taxonomy {
    #[serde(default)]
    capability: Vec<Capability>,
}

#[derive(Debug, Deserialize)]
struct Capability {
    id: String,
    name: String,
    category: String,
    fixtureable: bool,
    #[serde(default)]
    spec_sections: Vec<String>,
}

fn load_toml<T: serde::de::DeserializeOwned>(root: &Path, rel: &str) -> Result<T> {
    crate::util::load_toml(&root.join(rel))
}

/// `cargo xtask decoder-fixtures verify` — metadata-integrity gate (no decode).
pub(crate) fn run_verify(root: &Path) -> Result<()> {
    let manifest: Manifest = load_toml(root, MANIFEST_PATH)?;
    let taxonomy: Taxonomy = load_toml(root, TAXONOMY_PATH)?;

    if manifest.fixture.is_empty() {
        bail!("{MANIFEST_PATH} has no [[fixture]] entries");
    }

    let mut capability_ids: BTreeSet<&str> = BTreeSet::new();
    for cap in &taxonomy.capability {
        if !capability_ids.insert(cap.id.as_str()) {
            bail!("{TAXONOMY_PATH}: duplicate capability id {:?}", cap.id);
        }
    }
    if capability_ids.is_empty() {
        bail!("{TAXONOMY_PATH} has no [[capability]] entries");
    }

    let vectors_root = root.join(&manifest.vectors_root);
    let mut ids: BTreeSet<&str> = BTreeSet::new();
    let mut manifest_ivf: BTreeSet<PathBuf> = BTreeSet::new();
    let (mut n_must, mut n_xfail, mut n_other) = (0u32, 0u32, 0u32);

    for fx in &manifest.fixture {
        if !ids.insert(fx.id.as_str()) {
            bail!("duplicate fixture id {:?}", fx.id);
        }
        if !VALID_STATUSES.contains(&fx.status.as_str()) {
            bail!("fixture {}: unknown status {:?}", fx.id, fx.status);
        }
        if fx.coded_frames > manifest.max_frames || fx.shown_frames > manifest.max_frames {
            bail!(
                "fixture {}: {} coded / {} shown frames exceed max_frames {}",
                fx.id,
                fx.coded_frames,
                fx.shown_frames,
                manifest.max_frames
            );
        }
        for feature in &fx.features {
            if !capability_ids.contains(feature.as_str()) {
                bail!(
                    "fixture {}: feature {:?} is not a capability id in {TAXONOMY_PATH}",
                    fx.id,
                    feature
                );
            }
        }
        check_hash_hex(&fx.id, "ivf_sha256", &fx.hashes.ivf_sha256)?;
        check_hash_hex(
            &fx.id,
            "avm_raw_i420_sha256",
            &fx.hashes.avm_raw_i420_sha256,
        )?;
        for (i, h) in fx.hashes.avm_raw_i420_frame_sha256.iter().enumerate() {
            check_hash_hex(&fx.id, &format!("avm_raw_i420_frame_sha256[{i}]"), h)?;
        }
        if !fx.hashes.avm_raw_i420_frame_sha256.is_empty()
            && fx.hashes.avm_raw_i420_frame_sha256.len() as u64 != fx.shown_frames
        {
            bail!(
                "fixture {}: {} per-frame hashes but shown_frames = {}",
                fx.id,
                fx.hashes.avm_raw_i420_frame_sha256.len(),
                fx.shown_frames
            );
        }

        match fx.status.as_str() {
            "must_pass" => {
                n_must += 1;
                if fx.expected_splot.kind != "raw_i420_equals_avm" {
                    bail!(
                        "fixture {}: must_pass requires expected_splot.kind = \"raw_i420_equals_avm\"",
                        fx.id
                    );
                }
            }
            "xfail_splot" => {
                n_xfail += 1;
                if fx.expected_splot.kind != "unsupported_feature" {
                    bail!(
                        "fixture {}: xfail_splot requires expected_splot.kind = \"unsupported_feature\"",
                        fx.id
                    );
                }
                for (field, value) in [
                    ("rule_id", &fx.expected_splot.rule_id),
                    ("unsupported_reason", &fx.expected_splot.unsupported_reason),
                    ("matrix_row", &fx.expected_splot.matrix_row),
                    ("feature_id", &fx.expected_splot.feature_id),
                    ("detail_kind", &fx.expected_splot.detail_kind),
                ] {
                    if value.is_none() {
                        bail!(
                            "fixture {}: xfail_splot missing expected_splot.{field}",
                            fx.id
                        );
                    }
                }
            }
            _ => n_other += 1,
        }

        let ivf_path = vectors_root.join(&fx.path);
        let bytes = std::fs::read(&ivf_path)
            .with_context(|| format!("fixture {}: read {}", fx.id, ivf_path.display()))?;
        let got = sha256_hex(&bytes);
        if got != fx.hashes.ivf_sha256 {
            bail!(
                "fixture {}: {} bytes hash to {got}, manifest records {}",
                fx.id,
                ivf_path.display(),
                fx.hashes.ivf_sha256
            );
        }
        if let Ok(canon) = ivf_path.canonicalize() {
            manifest_ivf.insert(canon);
        }
    }

    let mut committed = Vec::new();
    collect_ivf(&vectors_root.join("vectors/valid"), &mut committed);
    let orphans: Vec<String> = committed
        .iter()
        .filter_map(|p| p.canonicalize().ok())
        .filter(|p| !manifest_ivf.contains(p))
        .map(|p| p.display().to_string())
        .collect();
    if !orphans.is_empty() {
        bail!("committed valid .ivf without a {MANIFEST_PATH} entry: {orphans:?}");
    }

    eprintln!(
        "decoder-fixtures verify: ok ({} fixtures: {n_must} must_pass, {n_xfail} xfail_splot, {n_other} other; {} capabilities)",
        manifest.fixture.len(),
        capability_ids.len()
    );
    Ok(())
}

fn check_hash_hex(id: &str, field: &str, value: &str) -> Result<()> {
    if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("fixture {id}: {field} is not a 64-hex-digit sha256: {value:?}");
    }
    Ok(())
}

fn collect_ivf(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_ivf(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("ivf") {
            out.push(path);
        }
    }
}

/// `cargo xtask decoder-fixtures coverage [--check]`.
pub(crate) fn run_coverage(root: &Path, check: bool) -> Result<()> {
    let manifest: Manifest = load_toml(root, MANIFEST_PATH)?;
    let taxonomy: Taxonomy = load_toml(root, TAXONOMY_PATH)?;
    let rendered = render_coverage(&manifest, &taxonomy);
    let doc_path = root.join(COVERAGE_DOC_PATH);
    if check {
        let actual = std::fs::read_to_string(&doc_path)
            .with_context(|| format!("failed to read {}", doc_path.display()))?;
        if actual.trim_end() != rendered.trim_end() {
            bail!("{COVERAGE_DOC_PATH} is out of date; regenerate with `{REGEN_COMMAND}`");
        }
        eprintln!("decoder-fixtures coverage: ok (up to date)");
    } else {
        std::fs::write(&doc_path, &rendered)
            .with_context(|| format!("failed to write {}", doc_path.display()))?;
        eprintln!(
            "decoder-fixtures coverage: wrote {} capability row(s) to {COVERAGE_DOC_PATH}",
            taxonomy.capability.len()
        );
    }
    Ok(())
}

/// Coverage classification for one capability.
#[derive(PartialEq, Eq)]
enum Cover {
    MustPass,
    XfailOnly,
    NotCovered,
    NotFixtureable,
}

impl Cover {
    fn label(&self) -> &'static str {
        match self {
            Cover::MustPass => "covered (must_pass)",
            Cover::XfailOnly => "xfail only",
            Cover::NotCovered => "not covered",
            Cover::NotFixtureable => "not_fixtureable_with_avm_encoder",
        }
    }
}

fn render_coverage(manifest: &Manifest, taxonomy: &Taxonomy) -> String {
    let mut by_feature: BTreeMap<&str, (Vec<&str>, Vec<&str>)> = BTreeMap::new();
    for fx in &manifest.fixture {
        for feature in &fx.features {
            let entry = by_feature.entry(feature.as_str()).or_default();
            match fx.status.as_str() {
                "must_pass" => entry.0.push(&fx.id),
                "xfail_splot" => entry.1.push(&fx.id),
                _ => {}
            }
        }
    }

    let mut out = String::new();
    out.push_str("<!-- SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0 -->\n");
    out.push_str(
        "<!-- SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com> -->\n\n",
    );
    out.push_str("# Decoder-oracle coverage\n\n");
    let _ = writeln!(
        out,
        "Generated from `{MANIFEST_PATH}` and `{TAXONOMY_PATH}` by `{REGEN_COMMAND}`. Do not edit by hand.\n"
    );

    let n_must = manifest
        .fixture
        .iter()
        .filter(|f| f.status == "must_pass")
        .count();
    let n_xfail = manifest
        .fixture
        .iter()
        .filter(|f| f.status == "xfail_splot")
        .count();
    let _ = writeln!(
        out,
        "Corpus: {} fixtures ({n_must} `must_pass`, {n_xfail} `xfail_splot`). Taxonomy: {} capabilities.\n",
        manifest.fixture.len(),
        taxonomy.capability.len()
    );

    let mut counts: BTreeMap<&str, u32> = BTreeMap::new();
    out.push_str("## Capability coverage\n\n");
    out.push_str("| Category | Capability | Status | Spec | Fixtures |\n");
    out.push_str("|---|---|---|---|---|\n");
    for cap in &taxonomy.capability {
        let (must, xfail) = by_feature
            .get(cap.id.as_str())
            .map_or((&[][..], &[][..]), |(m, x)| (m.as_slice(), x.as_slice()));
        let cover = if !cap.fixtureable {
            Cover::NotFixtureable
        } else if !must.is_empty() {
            Cover::MustPass
        } else if !xfail.is_empty() {
            Cover::XfailOnly
        } else {
            Cover::NotCovered
        };
        *counts.entry(cover.label()).or_default() += 1;
        let mut fixtures: Vec<&str> = must.iter().chain(xfail.iter()).copied().collect();
        fixtures.sort_unstable();
        let fixtures_cell = if fixtures.is_empty() {
            "—".to_owned()
        } else {
            fixtures.join("<br>")
        };
        let spec = if cap.spec_sections.is_empty() {
            "—".to_owned()
        } else {
            cap.spec_sections.join(", ")
        };
        let _ = writeln!(
            out,
            "| `{}` | {} | {} | {} | {} |",
            cap.category,
            cap.name,
            cover.label(),
            spec,
            fixtures_cell
        );
    }

    out.push_str("\n## Coverage counts\n\n| Status | Capabilities |\n|---|---:|\n");
    for (label, count) in &counts {
        let _ = writeln!(out, "| {label} | {count} |");
    }

    let mut backlog: BTreeMap<(&str, &str), Vec<&str>> = BTreeMap::new();
    for fx in &manifest.fixture {
        if fx.status != "xfail_splot" {
            continue;
        }
        let reason = fx
            .expected_splot
            .unsupported_reason
            .as_deref()
            .unwrap_or("?");
        let row = fx.expected_splot.matrix_row.as_deref().unwrap_or("?");
        backlog.entry((reason, row)).or_default().push(&fx.id);
    }
    out.push_str("\n## Feature-unlock backlog (`xfail_splot` reasons)\n\n");
    out.push_str("Ordered by how many fixtures each unlock converts to `must_pass`.\n\n");
    out.push_str("| unsupported_reason | matrix_row | fixtures | ids |\n|---|---|---:|---|\n");
    let mut backlog_rows: Vec<((&str, &str), Vec<&str>)> = backlog.into_iter().collect();
    backlog_rows.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.0.cmp(b.0.0)));
    for ((reason, row), mut ids) in backlog_rows {
        ids.sort_unstable();
        let _ = writeln!(
            out,
            "| `{reason}` | `{row}` | {} | {} |",
            ids.len(),
            ids.join("<br>")
        );
    }

    out
}

/// `cargo xtask decoder-fixtures report [--strict-xpass]` — decodes each fixture
/// with the built `splot` binary and prints a PASS / XFAIL / XPASS summary.
pub(crate) fn run_report(root: &Path, strict_xpass: bool) -> Result<()> {
    let manifest: Manifest = load_toml(root, MANIFEST_PATH)?;
    let vectors_root = root.join(&manifest.vectors_root);
    let splot_bin = build_splot_binary(root)?;
    let tmp = std::env::temp_dir().join("splot-decoder-fixtures-report");
    std::fs::create_dir_all(&tmp).ok();

    let (mut pass, mut xfail, mut xpass, mut fail) = (0u32, 0u32, 0u32, 0u32);
    let mut problems: Vec<String> = Vec::new();
    eprintln!(
        "decoder-fixtures report: decoding {} fixture(s) with the built splot binary (no AVM)",
        manifest.fixture.len()
    );
    for fx in &manifest.fixture {
        if fx.status != "must_pass" && fx.status != "xfail_splot" {
            continue;
        }
        let ivf = vectors_root.join(&fx.path);
        let out_raw = tmp.join(format!("{}.raw", fx.id));
        let output = Command::new(&splot_bin)
            .args(["decode", "--output-format", "raw", "-o"])
            .arg(&out_raw)
            .arg(&ivf)
            .output()
            .with_context(|| format!("spawn splot decode for {}", fx.id))?;
        let ok = output.status.success();
        match (fx.status.as_str(), ok) {
            ("must_pass", true) => {
                let raw = std::fs::read(&out_raw).unwrap_or_default();
                if sha256_hex(&raw) == fx.hashes.avm_raw_i420_sha256 {
                    pass += 1;
                    eprintln!("  PASS   {}", fx.id);
                } else {
                    fail += 1;
                    problems.push(format!("{}: must_pass raw hash != AVM oracle", fx.id));
                    eprintln!("  FAIL   {}  (hash mismatch)", fx.id);
                }
            }
            ("must_pass", false) => {
                fail += 1;
                problems.push(format!("{}: must_pass failed to decode", fx.id));
                eprintln!("  FAIL   {}  (decode error)", fx.id);
            }
            ("xfail_splot", false) => {
                xfail += 1;
                eprintln!(
                    "  XFAIL  {}  ({})",
                    fx.id,
                    fx.expected_splot
                        .unsupported_reason
                        .as_deref()
                        .unwrap_or("?")
                );
            }
            ("xfail_splot", true) => {
                xpass += 1;
                let raw = std::fs::read(&out_raw).unwrap_or_default();
                let correct = sha256_hex(&raw) == fx.hashes.avm_raw_i420_sha256;
                problems.push(format!(
                    "{}: XPASS (output {} AVM oracle) — upgrade to must_pass",
                    fx.id,
                    if correct { "matches" } else { "differs from" }
                ));
                eprintln!("  XPASS  {}  (now decodes)", fx.id);
            }
            _ => {}
        }
    }
    eprintln!("decoder-fixtures report: {pass} pass, {xfail} xfail, {xpass} xpass, {fail} fail");
    if fail > 0 {
        bail!(
            "{fail} must_pass fixture(s) failed:\n  {}",
            problems.join("\n  ")
        );
    }
    if xpass > 0 && strict_xpass {
        bail!(
            "{xpass} xfail fixture(s) now decode (strict-xpass):\n  {}",
            problems.join("\n  ")
        );
    }
    for p in &problems {
        eprintln!("  note: {p}");
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn manifest(fixtures: &str) -> Manifest {
        let text = format!("vectors_root = \"tests/conformance\"\nmax_frames = 4\n{fixtures}");
        toml::from_str(&text).unwrap()
    }

    fn taxonomy() -> Taxonomy {
        toml::from_str(
            r#"
[[capability]]
id = "intra.dc"
name = "DC"
category = "intra"
fixtureable = true
spec_sections = ["7.11"]

[[capability]]
id = "intra.cfl"
name = "CfL"
category = "intra"
fixtureable = true

[[capability]]
id = "seq.chroma.444"
name = "4:4:4"
category = "sequence"
fixtureable = false
"#,
        )
        .unwrap()
    }

    const MP: &str = r#"
[[fixture]]
id = "a"
path = "vectors/valid/a.ivf"
status = "must_pass"
width = 64
height = 64
coded_frames = 1
shown_frames = 1
bytes_per_sample = 1
features = ["intra.dc"]
spec_sections = ["7.11"]
[fixture.hashes]
ivf_sha256 = "0000000000000000000000000000000000000000000000000000000000000000"
avm_raw_i420_sha256 = "1111111111111111111111111111111111111111111111111111111111111111"
[fixture.expected_splot]
kind = "raw_i420_equals_avm"
"#;

    const XF: &str = r#"
[[fixture]]
id = "b"
path = "vectors/valid/b.ivf"
status = "xfail_splot"
width = 64
height = 64
coded_frames = 1
shown_frames = 1
bytes_per_sample = 1
features = ["intra.cfl"]
spec_sections = ["5.20.5.6"]
[fixture.hashes]
ivf_sha256 = "0000000000000000000000000000000000000000000000000000000000000000"
avm_raw_i420_sha256 = "2222222222222222222222222222222222222222222222222222222222222222"
[fixture.expected_splot]
kind = "unsupported_feature"
rule_id = "decode/unsupported-feature"
unsupported_reason = "unsupported_cfl_intra"
matrix_row = "sequence-chroma-frontier"
feature_id = "DECODE-SEQUENCE-CHROMA-FRONTIER"
detail_kind = "unsupported_feature"
strict_xpass_should_fail = true
"#;

    #[test]
    fn coverage_classifies_capabilities() {
        let m = manifest(&format!("{MP}{XF}"));
        let rendered = render_coverage(&m, &taxonomy());
        assert!(rendered.contains("covered (must_pass)"));
        assert!(rendered.contains("xfail only"));
        assert!(rendered.contains("not_fixtureable_with_avm_encoder"));
        assert!(rendered.contains("unsupported_cfl_intra"));
    }

    #[test]
    fn hash_hex_validation() {
        assert!(check_hash_hex("x", "f", &"a".repeat(64)).is_ok());
        assert!(check_hash_hex("x", "f", "short").is_err());
        assert!(check_hash_hex("x", "f", &"g".repeat(64)).is_err());
    }

    #[test]
    fn coverage_is_deterministic() {
        let m = manifest(&format!("{MP}{XF}"));
        let t = taxonomy();
        assert_eq!(render_coverage(&m, &t), render_coverage(&m, &t));
    }
}

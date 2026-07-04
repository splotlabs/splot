// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `cargo xtask decoder-fixtures {verify,coverage}` — the decoder-output oracle
//! harness (CONF-AVM-DECODE-ORACLE), CI-safe, no AVM. `verify` checks the
//! manifest/taxonomy shape, fixture hashes, feature ids, and that every committed
//! valid `.ivf` has an entry. `coverage` (re)generates
//! `docs/decoder/DECODER-ORACLE-COVERAGE.md`. Both run in `cargo xtask ci`. The
//! bit-exact compare/xfail assertions live in `crates/splot-cli/tests/decoder_oracle.rs`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use crate::git_util::sha256_hex;
use crate::util::load_toml;

const MANIFEST: &str = "tests/conformance/decoder-oracle.toml";
const TAXONOMY: &str = "tests/conformance/decoder-oracle-coverage.toml";
const COVERAGE_DOC: &str = "docs/decoder/DECODER-ORACLE-COVERAGE.md";
const REGEN: &str = "cargo xtask decoder-fixtures coverage";

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
    #[serde(default)]
    features: Vec<String>,
    ivf_sha256: String,
    avm_raw_sha256: String,
    #[serde(default)]
    unsupported_reason: Option<String>,
    #[serde(default)]
    matrix_row: Option<String>,
}

#[derive(Deserialize)]
struct Taxonomy {
    #[serde(default)]
    capability: Vec<Capability>,
}

#[derive(Deserialize)]
struct Capability {
    id: String,
    name: String,
    category: String,
    fixtureable: bool,
    #[serde(default)]
    spec_sections: Vec<String>,
}

/// `cargo xtask decoder-fixtures verify` — metadata-integrity gate (no decode).
pub(crate) fn run_verify(root: &Path) -> Result<()> {
    let manifest: Manifest = load_toml(&root.join(MANIFEST))?;
    let taxonomy: Taxonomy = load_toml(&root.join(TAXONOMY))?;
    if manifest.fixture.is_empty() {
        bail!("{MANIFEST} has no fixtures");
    }
    let caps: BTreeSet<&str> = taxonomy.capability.iter().map(|c| c.id.as_str()).collect();
    if caps.len() != taxonomy.capability.len() {
        bail!("{TAXONOMY} has duplicate capability ids");
    }

    let vectors = root.join(&manifest.vectors_dir);
    let mut ids: BTreeSet<&str> = BTreeSet::new();
    let mut by_bytes: BTreeMap<&str, (&str, BTreeSet<&str>)> = BTreeMap::new();
    let (mut n_pass, mut n_xfail) = (0u32, 0u32);
    for fx in &manifest.fixture {
        if !ids.insert(&fx.id) {
            bail!("duplicate fixture id {}", fx.id);
        }
        let features: BTreeSet<&str> = fx.features.iter().map(String::as_str).collect();
        if let Some((other, other_features)) = by_bytes.get(fx.ivf_sha256.as_str())
            && *other_features != features
        {
            bail!(
                "fixtures {other} and {} are byte-identical but claim different features: a clone \
                 cannot exercise a distinct tool — regenerate so the tool is in the bytes, or drop \
                 the unearned feature",
                fx.id
            );
        }
        by_bytes
            .entry(fx.ivf_sha256.as_str())
            .or_insert((&fx.id, features));
        for h in [&fx.ivf_sha256, &fx.avm_raw_sha256] {
            if h.len() != 64 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
                bail!("fixture {}: bad sha256 {h:?}", fx.id);
            }
        }
        for feature in &fx.features {
            if !caps.contains(feature.as_str()) {
                bail!("fixture {}: unknown feature {feature:?}", fx.id);
            }
        }
        match fx.status.as_str() {
            "must_pass" => n_pass += 1,
            "xfail_splot" => {
                n_xfail += 1;
                if fx.unsupported_reason.is_none() || fx.matrix_row.is_none() {
                    bail!(
                        "xfail fixture {} needs unsupported_reason + matrix_row",
                        fx.id
                    );
                }
            }
            other => bail!("fixture {}: unknown status {other:?}", fx.id),
        }
        let bytes = std::fs::read(vectors.join(format!("{}.ivf", fx.id)))
            .with_context(|| format!("fixture {}: read .ivf", fx.id))?;
        if sha256_hex(&bytes) != fx.ivf_sha256 {
            bail!("fixture {}: committed bytes do not match ivf_sha256", fx.id);
        }
    }

    let mut committed: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&vectors) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("ivf")
                && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
                && !ids.contains(stem)
            {
                committed.push(stem.to_owned());
            }
        }
    }
    if !committed.is_empty() {
        bail!("valid .ivf missing from {MANIFEST}: {committed:?}");
    }

    eprintln!(
        "decoder-fixtures verify: ok ({} fixtures: {n_pass} must_pass, {n_xfail} xfail; {} capabilities)",
        manifest.fixture.len(),
        caps.len()
    );
    Ok(())
}

/// `cargo xtask decoder-fixtures coverage [--check]`.
pub(crate) fn run_coverage(root: &Path, check: bool) -> Result<()> {
    let manifest: Manifest = load_toml(&root.join(MANIFEST))?;
    let taxonomy: Taxonomy = load_toml(&root.join(TAXONOMY))?;
    let rendered = render_coverage(&manifest, &taxonomy);
    let path = root.join(COVERAGE_DOC);
    if check {
        let actual =
            std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        if actual.trim_end() != rendered.trim_end() {
            bail!("{COVERAGE_DOC} is out of date; regenerate with `{REGEN}`");
        }
        eprintln!("decoder-fixtures coverage: ok");
    } else {
        std::fs::write(&path, &rendered).with_context(|| format!("write {}", path.display()))?;
        eprintln!("decoder-fixtures coverage: wrote {COVERAGE_DOC}");
    }
    Ok(())
}

fn render_coverage(manifest: &Manifest, taxonomy: &Taxonomy) -> String {
    let mut by_feature: BTreeMap<&str, (Vec<&str>, Vec<&str>)> = BTreeMap::new();
    for fx in &manifest.fixture {
        for feature in &fx.features {
            let e = by_feature.entry(feature).or_default();
            match fx.status.as_str() {
                "must_pass" => e.0.push(&fx.id),
                "xfail_splot" => e.1.push(&fx.id),
                _ => {}
            }
        }
    }
    let n_pass = manifest
        .fixture
        .iter()
        .filter(|f| f.status == "must_pass")
        .count();

    let mut out = String::new();
    out.push_str("<!-- SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0 -->\n");
    out.push_str(
        "<!-- SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com> -->\n\n",
    );
    out.push_str("# Decoder-oracle coverage\n\n");
    let _ = writeln!(
        out,
        "Generated from `{MANIFEST}` + `{TAXONOMY}` by `{REGEN}`. Do not edit by hand.\n"
    );
    let _ = writeln!(
        out,
        "Corpus: {} fixtures ({n_pass} `must_pass`, {} `xfail_splot`). Taxonomy: {} capabilities.\n",
        manifest.fixture.len(),
        manifest.fixture.len() - n_pass,
        taxonomy.capability.len()
    );

    let mut counts: BTreeMap<&str, u32> = BTreeMap::new();
    out.push_str("## Capability coverage\n\n| Category | Capability | Status | Spec | Fixtures |\n|---|---|---|---|---|\n");
    for cap in &taxonomy.capability {
        let (must, xfail) = by_feature
            .get(cap.id.as_str())
            .map_or((&[][..], &[][..]), |(m, x)| (m.as_slice(), x.as_slice()));
        let status = if !cap.fixtureable {
            "not_fixtureable_with_avm_encoder"
        } else if !must.is_empty() {
            "covered (must_pass)"
        } else if !xfail.is_empty() {
            "xfail only"
        } else {
            "not covered"
        };
        *counts.entry(status).or_default() += 1;
        let mut fixtures: Vec<&str> = must.iter().chain(xfail).copied().collect();
        fixtures.sort_unstable();
        let cell = if fixtures.is_empty() {
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
            "| `{}` | {} | {status} | {spec} | {cell} |",
            cap.category, cap.name
        );
    }

    out.push_str("\n## Coverage counts\n\n| Status | Capabilities |\n|---|---:|\n");
    for (label, count) in &counts {
        let _ = writeln!(out, "| {label} | {count} |");
    }

    let mut backlog: BTreeMap<(&str, &str), Vec<&str>> = BTreeMap::new();
    for fx in &manifest.fixture {
        if fx.status == "xfail_splot" {
            let reason = fx.unsupported_reason.as_deref().unwrap_or("?");
            let row = fx.matrix_row.as_deref().unwrap_or("?");
            backlog.entry((reason, row)).or_default().push(&fx.id);
        }
    }
    out.push_str("\n## Feature-unlock backlog (`xfail_splot`)\n\n| unsupported_reason | matrix_row | fixtures | ids |\n|---|---|---:|---|\n");
    let mut rows: Vec<((&str, &str), Vec<&str>)> = backlog.into_iter().collect();
    rows.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.0.cmp(b.0.0)));
    for ((reason, row), mut ids) in rows {
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn manifest() -> Manifest {
        toml::from_str(
            r#"
vectors_dir = "tests/conformance/vectors/valid"
[[fixture]]
id = "a"
status = "must_pass"
features = ["intra.dc"]
ivf_sha256 = "0000000000000000000000000000000000000000000000000000000000000000"
avm_raw_sha256 = "1111111111111111111111111111111111111111111111111111111111111111"
[[fixture]]
id = "b"
status = "xfail_splot"
features = ["intra.cfl"]
ivf_sha256 = "0000000000000000000000000000000000000000000000000000000000000000"
avm_raw_sha256 = "2222222222222222222222222222222222222222222222222222222222222222"
unsupported_reason = "unsupported_cfl_intra"
matrix_row = "sequence-chroma-frontier"
"#,
        )
        .unwrap()
    }

    fn taxonomy() -> Taxonomy {
        toml::from_str(
            r#"
[[capability]]
id = "intra.dc"
name = "DC"
category = "intra"
fixtureable = true
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

    #[test]
    fn coverage_classifies_and_is_deterministic() {
        let (m, t) = (manifest(), taxonomy());
        let r = render_coverage(&m, &t);
        assert!(r.contains("covered (must_pass)"));
        assert!(r.contains("xfail only"));
        assert!(r.contains("not_fixtureable_with_avm_encoder"));
        assert!(r.contains("unsupported_cfl_intra"));
        assert_eq!(r, render_coverage(&m, &t));
    }
}

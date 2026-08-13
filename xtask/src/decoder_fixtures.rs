// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `cargo xtask decoder-fixtures {verify,coverage}` — the decoder-output oracle
//! harness (CONF-AVM-DECODE-ORACLE), CI-safe, no AVM. `verify` checks the
//! manifest/taxonomy shape, fixture hashes, feature ids, and that every committed
//! valid `.ivf` has an entry. `coverage` (re)generates the optional
//! `docs/decoder/DECODER-ORACLE-COVERAGE.md` report. `cargo xtask ci` verifies
//! the report only when it is committed. The bit-exact assertions live in
//! `crates/splot-cli/tests/decoder_oracle.rs`.

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
    schema_version: u32,
    vectors_dir: String,
    #[serde(default)]
    fixture: Vec<Fixture>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    id: String,
    #[serde(default)]
    features: Vec<String>,
    ivf_sha256: String,
    avm_raw_sha256: String,
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
    if manifest.schema_version != 2 {
        bail!(
            "{MANIFEST}: expected schema_version 2, got {}",
            manifest.schema_version
        );
    }
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
        "decoder-fixtures verify: ok ({} strict fixtures; {} capabilities)",
        manifest.fixture.len(),
        caps.len()
    );
    Ok(())
}

/// `cargo xtask decoder-fixtures coverage [--check]`.
pub(crate) fn run_coverage(root: &Path, check: bool) -> Result<()> {
    let manifest: Manifest = load_toml(&root.join(MANIFEST))?;
    if manifest.schema_version != 2 {
        bail!(
            "{MANIFEST}: expected schema_version 2, got {}",
            manifest.schema_version
        );
    }
    let taxonomy: Taxonomy = load_toml(&root.join(TAXONOMY))?;
    let rendered = render_coverage(&manifest, &taxonomy);
    let path = root.join(COVERAGE_DOC);
    if check {
        if !path.exists() {
            eprintln!("{COVERAGE_DOC}: not committed; generate on demand with `{REGEN}`");
            return Ok(());
        }
        let actual =
            std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        if actual.trim_end() != rendered.trim_end() {
            bail!("{COVERAGE_DOC} is out of date; regenerate with `{REGEN}`");
        }
        eprintln!("decoder-fixtures coverage: ok");
    } else {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create {}", parent.display()))?;
        }
        std::fs::write(&path, &rendered).with_context(|| format!("write {}", path.display()))?;
        eprintln!("decoder-fixtures coverage: wrote {COVERAGE_DOC}");
    }
    Ok(())
}

fn render_coverage(manifest: &Manifest, taxonomy: &Taxonomy) -> String {
    let mut by_feature: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for fx in &manifest.fixture {
        for feature in &fx.features {
            by_feature.entry(feature).or_default().push(&fx.id);
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
        "Generated from `{MANIFEST}` + `{TAXONOMY}` by `{REGEN}`. Do not edit by hand.\n"
    );
    let _ = writeln!(
        out,
        "Corpus: {} strict fixtures. Taxonomy: {} capabilities.\n",
        manifest.fixture.len(),
        taxonomy.capability.len()
    );

    let mut counts: BTreeMap<&str, u32> = BTreeMap::new();
    out.push_str("## Capability coverage\n\n| Category | Capability | Status | Spec | Fixtures |\n|---|---|---|---|---|\n");
    for cap in &taxonomy.capability {
        let fixtures = by_feature
            .get(cap.id.as_str())
            .map_or(&[][..], Vec::as_slice);
        let state = if !cap.fixtureable {
            "not_fixtureable_with_avm_encoder"
        } else if !fixtures.is_empty() {
            "covered"
        } else {
            "not covered"
        };
        *counts.entry(state).or_default() += 1;
        let mut fixtures = fixtures.to_vec();
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
            "| `{}` | {} | {state} | {spec} | {cell} |",
            cap.category, cap.name
        );
    }

    out.push_str("\n## Coverage counts\n\n| Status | Capabilities |\n|---|---:|\n");
    for (label, count) in &counts {
        let _ = writeln!(out, "| {label} | {count} |");
    }

    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn fixture_rejects_obsolete_fields() {
        let field = concat!("sta", "tus");
        let text = format!(
            "id = \"a\"\nfeatures = []\nivf_sha256 = \"0\"\navm_raw_sha256 = \"1\"\n{field} = \"old\"\n"
        );
        let error = toml::from_str::<Fixture>(&text).err().unwrap();
        assert!(error.to_string().contains("unknown field"));
    }
}

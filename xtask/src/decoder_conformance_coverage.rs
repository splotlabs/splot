// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use clap::ValueEnum;
use serde::Deserialize;

const COVERAGE_DOC_PATH: &str = "docs/DECODER-SPEC-COVERAGE.md";
const SPEC_INDEX_PATH: &str = "docs/spec/av2/1.0.0/index.md";
const SUPPORT_MATRIX_PATH: &str = "docs/DECODER-SUPPORT-MATRIX.toml";
const IMPLEMENTATION_MATRIX_PATH: &str = "docs/IMPLEMENTATION-MATRIX.toml";
const DIAGNOSTICS_DOC_PATH: &str = "docs/DECODER-DIAGNOSTICS.md";
const REGEN_COMMAND: &str = "cargo xtask decoder-conformance-coverage --format markdown --output docs/DECODER-SPEC-COVERAGE.md";

const NORMATIVE_STATUSES: &[&str] = &["normative", "informative", "mixed"];
#[rustfmt::skip]
const ROW_STATUSES: &[&str] = &["unsupported", "partial", "supported", "blocked", "out-of-scope-nonnormative"];

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum DecoderConformanceCoverageFormat {
    Markdown,
}

pub(crate) fn run_decoder_conformance_coverage(
    root: &Path,
    format: DecoderConformanceCoverageFormat,
    output: Option<PathBuf>,
) -> Result<()> {
    let checked = checked_rows(root)?;
    let rendered = match format {
        DecoderConformanceCoverageFormat::Markdown => render_markdown(&checked),
    };
    if let Some(path) = output {
        std::fs::write(&path, &rendered)
            .with_context(|| format!("failed to write {}", path.display()))?;
        eprintln!(
            "decoder-conformance-coverage: wrote {} row(s) to {}",
            checked.len(),
            path.display()
        );
    } else {
        print!("{rendered}");
        if !rendered.ends_with('\n') {
            println!();
        }
    }
    Ok(())
}

pub(crate) fn run_check_decoder_conformance_coverage(root: &Path) -> Result<()> {
    let checked = checked_rows(root)?;
    let expected = render_markdown(&checked);
    let path = root.join(COVERAGE_DOC_PATH);
    let actual = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {COVERAGE_DOC_PATH}"))?;
    if actual.trim_end() != expected.trim_end() {
        bail!("{COVERAGE_DOC_PATH} is out of date; regenerate with `{REGEN_COMMAND}`");
    }
    eprintln!(
        "check-decoder-conformance-coverage: ok ({} row(s))",
        checked.len()
    );
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct CoverageRow {
    id: &'static str,
    spec_sections: &'static [&'static str],
    spec_title: &'static str,
    normative_status: &'static str,
    implementation_owner: &'static str,
    decoder_support_rows: &'static [&'static str],
    feature_ids: &'static [&'static str],
    status: &'static str,
    tests: &'static [&'static str],
    fuzz_targets: &'static [&'static str],
    local_reference_evidence: &'static [&'static str],
    diagnostics: &'static [&'static str],
    notes: &'static str,
}

#[derive(Debug)]
struct CheckedCoverageRow<'a> {
    row: &'a CoverageRow,
}

const COVERAGE_ROWS: &[CoverageRow] = &[
    CoverageRow {
        id: "symbols-and-conventions",
        spec_sections: &["3", "4", "4.11"],
        spec_title: "Symbols, conventions, and descriptor primitives",
        normative_status: "mixed",
        implementation_owner: "splot-core parser primitives and future decoder consumers",
        decoder_support_rows: &[
            "decode-byte-stream-planner",
            "decode-runtime-hash-fuzz",
            "symbol-decoder",
        ],
        feature_ids: &[
            "DECODE-BYTE-STREAM-PLANNER",
            "CONF-DECODE-RUNTIME-HASH-FUZZ",
            "AV2-8.2-SYMBOL-DECODER",
        ],
        status: "partial",
        tests: &["cargo test -p splot-core bitio --locked"],
        fuzz_targets: &[
            "parse_obu",
            "decode_plan_bytes",
            "decode_runtime_hash_bytes",
        ],
        local_reference_evidence: &[],
        diagnostics: &["decode/malformed-source", "decode/unsupported-feature"],
        notes: "Section 3 is symbol notation context; Section 4 descriptor behavior is normative where syntax and decode processes consume it. Core descriptor parsing exists, but full runtime decoder consumption remains incomplete.",
    },
    CoverageRow {
        id: "obu-and-length-delimited-front-door",
        spec_sections: &["5.2", "5.3", "6.2", "6.3", "Annex B"],
        spec_title: "OBU syntax, reserved OBUs, semantics, and Annex B input",
        normative_status: "normative",
        implementation_owner: "splot-core stream parsing and splot-decode byte planner/runtime front door",
        decoder_support_rows: &[
            "decode-byte-stream-planner",
            "decode-runtime-hash-fuzz",
            "decode-runtime-y4m-fuzz",
            "cli-decode-entrypoint",
        ],
        feature_ids: &[
            "DECODE-BYTE-STREAM-PLANNER",
            "CONF-DECODE-RUNTIME-HASH-FUZZ",
            "CONF-DECODE-RUNTIME-Y4M-FUZZ",
            "CLI-DECODE",
        ],
        status: "partial",
        tests: &[
            "crates/splot-cli/tests/decode_cli.rs::decode_malformed_source_json_mode_emits_detail_fields",
            "crates/splot-cli/tests/decode_cli.rs::decode_unsupported_structure_json_mode_uses_planner_metadata",
        ],
        fuzz_targets: &[
            "parse_obu",
            "parse_bitstream",
            "decode_plan_bytes",
            "decode_runtime_hash_bytes",
            "decode_runtime_y4m_bytes",
        ],
        local_reference_evidence: &[],
        diagnostics: &[
            "decode/malformed-source",
            "decode/resource-limit",
            "decode/unsupported-feature",
        ],
        notes: "Input traversal and malformed-source diagnostics exist. Broad runtime decode remains intentionally unsupported outside the documented minimal hash/Y4M tier.",
    },
    CoverageRow {
        id: "sequence-hls-and-global-state",
        spec_sections: &[
            "5.4", "5.5", "5.6", "5.7", "5.8", "5.9", "5.10", "5.11", "5.12", "5.13", "5.14",
            "5.15", "5.16", "5.17", "6.4", "6.5", "6.6", "6.7", "6.8", "6.9", "6.10", "6.11",
            "6.12", "6.13", "6.14", "6.15", "6.16",
        ],
        spec_title: "Sequence, HLS, metadata, layer, operating-point, QM, and film-grain state",
        normative_status: "normative",
        implementation_owner: "splot-core parsers, splot-validate state, and future splot-decode runtime state",
        decoder_support_rows: &[
            "decode-stream-state",
            "minimal-decode-tier-contract",
            "decode-runtime-hash-fuzz",
        ],
        feature_ids: &[
            "DECODE-STREAM-STATE-PLANNER",
            "DOC-MINIMAL-DECODE-TIER-CONTRACT",
            "CONF-DECODE-RUNTIME-HASH-FUZZ",
        ],
        status: "partial",
        tests: &["cargo test --workspace --all-targets --locked"],
        fuzz_targets: &[
            "validate_bytes",
            "decode_plan_bytes",
            "decode_runtime_hash_bytes",
        ],
        local_reference_evidence: &[],
        diagnostics: &["decode/unsupported-feature"],
        notes: "Parser and validator coverage exists for many syntax structures, and the minimal runtime hash path consumes a supported 8-bit 4:2:0 sequence subset. Broad runtime decoder consumption, external-HLS handling, metadata effects, film-grain state, and operating-point behavior are incomplete.",
    },
    CoverageRow {
        id: "frame-header-state",
        spec_sections: &["5.18", "6.17"],
        spec_title: "Frame header syntax, semantics, and runtime frame state",
        normative_status: "normative",
        implementation_owner: "splot-core frame-header parsers and future splot-decode frame state",
        decoder_support_rows: &[
            "tile-payload-input-derivation",
            "minimal-decode-tier-contract",
            "decode-runtime-hash-fuzz",
        ],
        feature_ids: &[
            "DECODE-TILE-PAYLOAD-INPUT-DERIVATION",
            "DOC-MINIMAL-DECODE-TIER-CONTRACT",
            "CONF-DECODE-RUNTIME-HASH-FUZZ",
        ],
        status: "partial",
        tests: &["cargo test -p splot-core headers::frame --locked"],
        fuzz_targets: &[
            "validate_bytes",
            "decode_plan_bytes",
            "decode_runtime_hash_bytes",
        ],
        local_reference_evidence: &[],
        diagnostics: &["decode/unsupported-feature", "decode/resource-limit"],
        notes: "Frame-header parsing and tile input derivation are partial, with the minimal runtime hash path consuming its supported closed-loop-key subset. Full runtime frame state, inter paths, filters, global motion, segmentation, quantization, and film-grain config consumption are not yet runtime decoded.",
    },
    CoverageRow {
        id: "tile-group-and-payload-syntax",
        spec_sections: &["5.19", "5.20", "6.18", "6.19"],
        spec_title: "Tile group, tile payload, partition, block, mode, residual, and transform syntax",
        normative_status: "normative",
        implementation_owner: "splot-decode tile payload boundary and future decode_tile implementation",
        decoder_support_rows: &[
            "tile-payload-decode",
            "tile-payload-decode-fuzz",
            "tile-cdf-selection-boundary",
            "decode-context-tile-payload-handoff",
            "decode-runtime-hash-fuzz",
        ],
        feature_ids: &[
            "DECODE-TILE-PAYLOAD-BOUNDARY",
            "CONF-TILE-PAYLOAD-DECODE-FUZZ",
            "DECODE-TILE-CDF-SELECTION-BOUNDARY",
            "DECODE-CONTEXT-TILE-PAYLOAD-HANDOFF",
            "CONF-DECODE-RUNTIME-HASH-FUZZ",
        ],
        status: "partial",
        tests: &[
            "cargo test -p splot-decode tile_payload --locked",
            "cargo test -p splot-decode runtime_hash --locked",
        ],
        fuzz_targets: &[
            "decode_plan_bytes",
            "decode_runtime_hash_bytes",
            "tile_payload_decode_bytes",
        ],
        local_reference_evidence: &[],
        diagnostics: &["decode/unsupported-feature", "decode/resource-limit"],
        notes: "Tile payload, partition CDF boundary subsets, and targeted minimal tile-payload frontier fuzz evidence exist. Full tile group traversal, decode_tile, partition/block syntax, residual syntax, and multi-tile handling remain unsupported.",
    },
    CoverageRow {
        id: "decode-orchestration-and-random-access",
        spec_sections: &["7.1", "7.2", "7.3", "7.4"],
        spec_title: "General decode process, frame wrapup, ordering, and random access",
        normative_status: "normative",
        implementation_owner: "splot-decode planning and future runtime orchestration",
        decoder_support_rows: &[
            "cli-decode-entrypoint",
            "decode-stream-state",
            "decode-runtime-hash-fuzz",
            "decode-runtime-y4m-fuzz",
        ],
        feature_ids: &[
            "CLI-DECODE",
            "DECODE-STREAM-STATE-PLANNER",
            "CONF-DECODE-RUNTIME-HASH-FUZZ",
            "CONF-DECODE-RUNTIME-Y4M-FUZZ",
        ],
        status: "unsupported",
        tests: &[
            "crates/splot-cli/tests/decode_cli.rs::decode_hash_output_format_emits_unsupported_text_without_output_path",
            "crates/splot-cli/tests/decode_y4m_cli.rs::decode_y4m_source_error_wins_before_missing_output_parent",
        ],
        fuzz_targets: &[
            "decode_plan_bytes",
            "decode_runtime_hash_bytes",
            "decode_runtime_y4m_bytes",
        ],
        local_reference_evidence: &[],
        diagnostics: &["decode/unsupported-feature"],
        notes: "Broad public decode outside the minimal runtime tier still defers at the runtime tier boundary, so general decoding, wrapup, output units, and random-access runtime behavior are unsupported.",
    },
    CoverageRow {
        id: "cdf-layer-reference-and-motion-setup",
        spec_sections: &["7.5", "7.6", "7.7", "7.8", "7.9", "7.10", "7.11", "7.12"],
        spec_title: "CDF update, extended layers, reference lists, motion fields, contexts, and prediction setup",
        normative_status: "normative",
        implementation_owner: "future splot-decode CDF/reference/motion setup",
        decoder_support_rows: &[
            "symbol-decoder",
            "tile-cdf-selection-boundary",
            "reference-frame-store",
        ],
        feature_ids: &[
            "AV2-8.2-SYMBOL-DECODER",
            "DECODE-TILE-CDF-SELECTION-BOUNDARY",
            "RECON-REFERENCE-FRAME-STORE",
        ],
        status: "unsupported",
        tests: &["cargo test -p splot-recon reference --locked"],
        fuzz_targets: &[],
        local_reference_evidence: &[],
        diagnostics: &["decode/unsupported-feature"],
        notes: "The current implementation has a partition CDF boundary subset and a generic reference store container, but no full runtime CDF lifecycle, layer context management, reference list construction, reference refresh semantics, motion-field estimation, or motion-vector prediction.",
    },
    CoverageRow {
        id: "prediction-process",
        spec_sections: &["7.13"],
        spec_title: "Prediction process",
        normative_status: "normative",
        implementation_owner: "splot-recon intra subsets and future full prediction implementation",
        decoder_support_rows: &[
            "intra-dc-square-prediction",
            "intra-dc-rectangular-prediction",
            "intra-basic-paeth-prediction",
            "intra-dc-subsampled-prediction",
            "intra-ibp-dc-prediction",
            "intra-smooth-prediction",
            "intra-cardinal-directional-prediction",
            "recon-intra-prediction-fuzz",
        ],
        feature_ids: &[
            "RECON-INTRA-DC-SQUARE-PREDICTION",
            "RECON-INTRA-DC-RECTANGULAR-PREDICTION",
            "RECON-INTRA-BASIC-PAETH-PREDICTION",
            "RECON-INTRA-DC-SUBSAMPLED-PREDICTION",
            "RECON-INTRA-IBP-DC-PREDICTION",
            "RECON-INTRA-SMOOTH-PREDICTION",
            "RECON-INTRA-CARDINAL-DIRECTIONAL-PREDICTION",
            "CONF-RECON-INTRA-PREDICTION-FUZZ",
        ],
        status: "partial",
        tests: &[
            "cargo test -p splot-recon ibp --locked",
            "cargo test -p splot-recon intra --locked",
            "cargo test -p splot-recon workspace --locked",
        ],
        fuzz_targets: &["recon_intra_prediction_bytes"],
        local_reference_evidence: &[],
        diagnostics: &["decode/unsupported-feature"],
        notes: "Some scalar intra prediction and workspace primitives exist, including subsampled DC, IBP DC, H/V cardinal directional prediction, and no-panic fuzz coverage over bounded structured direct prediction and current-frame workspace cases. General directional angles, full edge preparation, data-driven, general directional-angle IBP, full CfL/MHCCP, palette, all inter prediction, and broad runtime integration remain unsupported.",
    },
    CoverageRow {
        id: "reconstruction-transform-and-filters",
        spec_sections: &["7.14", "7.15", "7.16", "7.17", "7.18", "7.19", "7.20"],
        spec_title: "Dequantization, inverse transform, residual add, and loop filters",
        normative_status: "normative",
        implementation_owner: "future splot-recon reconstruction and filter stages",
        decoder_support_rows: &["current-frame-workspace"],
        feature_ids: &["RECON-CURRENT-FRAME-WORKSPACE"],
        status: "unsupported",
        tests: &[],
        fuzz_targets: &[],
        local_reference_evidence: &[],
        diagnostics: &["decode/unsupported-feature"],
        notes: "Workspace allocation primitives exist, but dequantization, inverse transforms, residual addition, deblocking, CDEF, CCSO, loop restoration, and GDF are not implemented as runtime decode stages.",
    },
    CoverageRow {
        id: "output-film-grain-and-reference-update",
        spec_sections: &["7.21", "7.22", "7.23"],
        spec_title: "Output process, film grain, motion-field storage, and reference-frame update",
        normative_status: "normative",
        implementation_owner: "splot-recon output containers and future splot-decode frame lifecycle",
        decoder_support_rows: &[
            "decoded-frame-plane-runtime-types",
            "recon-frame-plane-types-fuzz",
            "deterministic-frame-hash",
            "decode-runtime-hash-fuzz",
            "decode-runtime-y4m-fuzz",
            "output-y4m",
            "recon-y4m-output-fuzz",
            "recon-frame-hash-fuzz",
            "reference-frame-store",
        ],
        feature_ids: &[
            "INFRA-RECON-FRAME-PLANE-TYPES",
            "CONF-RECON-FRAME-PLANE-TYPES-FUZZ",
            "RECON-FRAME-HASH-DIGEST",
            "CONF-DECODE-RUNTIME-HASH-FUZZ",
            "CONF-DECODE-RUNTIME-Y4M-FUZZ",
            "RECON-Y4M-OUTPUT-WRITER",
            "CONF-RECON-Y4M-OUTPUT-FUZZ",
            "CONF-RECON-FRAME-HASH-FUZZ",
            "RECON-REFERENCE-FRAME-STORE",
        ],
        status: "partial",
        tests: &["cargo test -p splot-recon --locked"],
        fuzz_targets: &[
            "decode_runtime_hash_bytes",
            "decode_runtime_y4m_bytes",
            "recon_frame_plane_types_bytes",
            "recon_y4m_output_bytes",
            "recon_frame_hash_bytes",
        ],
        local_reference_evidence: &[],
        diagnostics: &["decode/unsupported-feature"],
        notes: "Frame/hash/Y4M/reference primitives exist for caller-supplied frames, with decoded-frame/plane type validators, hash input, and Y4M serialization fuzzed from bounded structured frames, and the documented minimal runtime tier emits hash/Y4M output for one supported fixture shape. The runtime hash/Y4M byte APIs are fuzzed with bounded raw bytes, minimal fixture mutations, and in-memory writers. Broad runtime output ordering, post-film-grain output, show-existing/flush behavior, motion-field storage, and AV2 reference refresh semantics remain unsupported. Current raw AVM/dav2d MD5 metadata is background evidence, not runtime coverage proof.",
    },
    CoverageRow {
        id: "symbol-and-cdf-process",
        spec_sections: &["8.1", "8.2", "8.3", "9.3"],
        spec_title: "Symbol parsing and CDF selection/lifecycle",
        normative_status: "normative",
        implementation_owner: "splot-core symbol primitives and future splot-decode CDF lifecycle",
        decoder_support_rows: &[
            "symbol-decoder",
            "symbol-decoder-fuzz",
            "tile-cdf-selection-boundary",
            "tile-cdf-save-lifecycle-boundary",
            "decode-runtime-hash-fuzz",
        ],
        feature_ids: &[
            "AV2-8.2-SYMBOL-DECODER",
            "CONF-SYMBOL-DECODER-FUZZ",
            "DECODE-TILE-CDF-SELECTION-BOUNDARY",
            "DECODE-TILE-CDF-SAVE-LIFECYCLE-BOUNDARY",
            "CONF-DECODE-RUNTIME-HASH-FUZZ",
        ],
        status: "partial",
        tests: &[
            "cargo test -p splot-core symbol --locked",
            "cargo test -p splot-decode tile_payload::cdf --locked",
        ],
        fuzz_targets: &["symbol_decoder_bytes", "decode_runtime_hash_bytes"],
        local_reference_evidence: &[],
        diagnostics: &["decode/unsupported-feature"],
        notes: "Generic f(n)/symbol foundations, a public §8.2 symbol-decoder fuzz target, a partition/block CDF subset, and supported-subset Tile/Saved/Frame lifecycle behavior exist. Full Section 8.3 CDF selection/lifecycle and Section 9.3 default CDF banks are incomplete.",
    },
    CoverageRow {
        id: "decode-lookup-tables",
        spec_sections: &["9.2", "9.4", "9.5", "9.6", "9.7", "9.8"],
        spec_title: "Decode-relevant normative lookup tables",
        normative_status: "normative",
        implementation_owner: "splot-core generated tables and future decode consumers",
        decoder_support_rows: &["symbol-decoder", "minimal-decode-tier-contract"],
        feature_ids: &["AV2-8.2-SYMBOL-DECODER", "DOC-MINIMAL-DECODE-TIER-CONTRACT"],
        status: "partial",
        tests: &["cargo xtask gen-tables --check"],
        fuzz_targets: &[],
        local_reference_evidence: &[],
        diagnostics: &["decode/unsupported-feature"],
        notes: "Generated table plumbing exists for committed table sources, but many transform, quant-matrix, warp/filter, and restoration consumers are not wired into runtime decode.",
    },
    CoverageRow {
        id: "profiles-levels-and-decoder-conformance",
        spec_sections: &[
            "Annex A",
            "Annex A.2",
            "Annex A.3",
            "Annex A.4",
            "Annex A.5",
        ],
        spec_title: "Profiles, levels, tiers, and decoder conformance",
        normative_status: "normative",
        implementation_owner: "future splot-decode profile/level/tier conformance checks",
        decoder_support_rows: &["minimal-decode-tier-contract", "decode-limits-budget"],
        feature_ids: &[
            "DOC-MINIMAL-DECODE-TIER-CONTRACT",
            "DOC-DECODE-LIMITS-CONTRACT",
        ],
        status: "partial",
        tests: &["cargo xtask check-decoder-support"],
        fuzz_targets: &[],
        local_reference_evidence: &[],
        diagnostics: &["decode/resource-limit", "decode/unsupported-feature"],
        notes: "Minimal-tier documentation and local resource policy exist. Full Annex A profile, level, tier, and decoder conformance checks remain incomplete and must stay distinct from splot resource limits.",
    },
    CoverageRow {
        id: "decoder-model-constraints",
        spec_sections: &["Annex E"],
        spec_title: "Decoder model timing, buffer, and presentation constraints",
        normative_status: "normative",
        implementation_owner: "future splot-decode decoder model checks",
        decoder_support_rows: &["minimal-decode-tier-contract"],
        feature_ids: &["DOC-MINIMAL-DECODE-TIER-CONTRACT"],
        status: "unsupported",
        tests: &[],
        fuzz_targets: &[],
        local_reference_evidence: &[],
        diagnostics: &["decode/unsupported-feature"],
        notes: "Annex E model state, timing, buffer, presentation, deadline, and level-imposed runtime constraints are not implemented.",
    },
    CoverageRow {
        id: "informative-annexes",
        spec_sections: &["Annex C", "Annex D", "Annex F", "Annex G"],
        spec_title: "Informative annexes and examples",
        normative_status: "informative",
        implementation_owner: "documentation only unless a future normative dependency is identified",
        decoder_support_rows: &[],
        feature_ids: &[],
        status: "out-of-scope-nonnormative",
        tests: &[],
        fuzz_targets: &[],
        local_reference_evidence: &[],
        diagnostics: &[],
        notes: "These annexes are marked informative in the spec mirror and are not required for the full decoder conformance claim unless a future normative section explicitly depends on them.",
    },
];

#[derive(Debug, Deserialize)]
struct SupportMatrix {
    #[serde(default)]
    row: Vec<SupportRow>,
}

#[derive(Debug, Deserialize)]
struct SupportRow {
    id: Option<String>,
    feature_id: Option<String>,
    status: Option<String>,
    #[serde(default)]
    self_contained_tests: Vec<String>,
    #[serde(default)]
    tests: Vec<String>,
    #[serde(default)]
    fixtures: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ImplementationMatrix {
    #[serde(default)]
    feature: Vec<ImplementationFeature>,
}

#[derive(Debug, Deserialize)]
struct ImplementationFeature {
    id: Option<String>,
}

fn checked_rows(root: &Path) -> Result<Vec<CheckedCoverageRow<'static>>> {
    validate_rows(root, COVERAGE_ROWS)
}

fn validate_rows(
    root: &Path,
    rows: &'static [CoverageRow],
) -> Result<Vec<CheckedCoverageRow<'static>>> {
    let known_sections = load_spec_sections(root)?;
    let support_rows = load_support_rows(root)?;
    let feature_ids = load_feature_ids(root)?;
    let diagnostics = load_decoder_diagnostic_ids(root)?;
    let evidence_index = if rows
        .iter()
        .any(|row| !row.local_reference_evidence.is_empty())
    {
        Some(crate::reference_evidence::load_checked_reference_evidence_index(root)?)
    } else {
        None
    };

    let mut problems = Vec::new();
    let mut seen = BTreeSet::new();
    let mut checked = Vec::new();
    for row in rows {
        let label = format!("coverage row `{}`", row.id);
        if !seen.insert(row.id) {
            problems.push(format!("{label}: duplicate row id"));
        }
        if !ROW_STATUSES.contains(&row.status) {
            problems.push(format!(
                "{label}: status `{}` is invalid (allowed: {})",
                row.status,
                ROW_STATUSES.join(", ")
            ));
        }
        if !NORMATIVE_STATUSES.contains(&row.normative_status) {
            problems.push(format!(
                "{label}: normative_status `{}` is invalid (allowed: {})",
                row.normative_status,
                NORMATIVE_STATUSES.join(", ")
            ));
        }
        if matches!(row.normative_status, "mixed")
            && !row.notes.split_whitespace().any(|word| {
                word.trim_matches(|ch: char| !ch.is_ascii_alphanumeric())
                    .eq_ignore_ascii_case("normative")
            })
        {
            problems.push(format!(
                "{label}: mixed normative_status requires notes explaining the normative portion"
            ));
        }
        if row.status == "out-of-scope-nonnormative" && row.notes.trim().is_empty() {
            problems.push(format!(
                "{label}: out-of-scope-nonnormative rows require explanatory notes"
            ));
        }
        if row.spec_sections.is_empty() {
            problems.push(format!("{label}: spec_sections must not be empty"));
        }
        for section in row.spec_sections {
            if !known_sections.contains(*section) {
                problems.push(format!(
                    "{label}: spec section `{section}` is not present in {SPEC_INDEX_PATH}"
                ));
            }
        }
        for support_row_id in row.decoder_support_rows {
            let Some(support_row) = support_rows.get(*support_row_id) else {
                problems.push(format!(
                    "{label}: decoder_support_rows references unknown row `{support_row_id}`"
                ));
                continue;
            };
            if matches!(row.normative_status, "normative" | "mixed")
                && support_row.feature_id.trim().is_empty()
            {
                problems.push(format!(
                    "{label}: decoder support row `{support_row_id}` has an empty feature_id"
                ));
            }
        }
        for feature_id in row.feature_ids {
            if !feature_ids.contains(*feature_id) {
                problems.push(format!(
                    "{label}: feature_ids references unknown Feature ID `{feature_id}`"
                ));
            }
        }
        for diagnostic in row.diagnostics {
            if !diagnostics.contains(*diagnostic) {
                problems.push(format!(
                    "{label}: diagnostics references unknown decoder diagnostic `{diagnostic}`"
                ));
            }
        }
        for evidence in row.local_reference_evidence {
            let Some(evidence_id) =
                crate::reference_evidence::canonical_evidence_pointer_id(evidence)
            else {
                problems.push(format!(
                    "{label}: local_reference_evidence pointer `{evidence}` is missing `{}` prefix",
                    crate::reference_evidence::MANIFEST_POINTER_PREFIX
                ));
                continue;
            };
            if evidence_index
                .as_ref()
                .and_then(|index| index.rows_for(evidence_id))
                .is_none()
            {
                problems.push(format!(
                    "{label}: local_reference_evidence references unknown evidence id `{evidence_id}`"
                ));
            }
        }
        if row.status == "supported"
            && let Some(problem) = supported_row_problem(row, &support_rows)
        {
            problems.push(format!("{label}: {problem}"));
        }
        checked.push(CheckedCoverageRow { row });
    }

    if problems.is_empty() {
        Ok(checked)
    } else {
        bail!(
            "decoder conformance coverage problem(s):\n- {}",
            problems.join("\n- ")
        )
    }
}

#[derive(Debug)]
struct CheckedSupportRow {
    feature_id: String,
    status: String,
    tests: Vec<String>,
    fixtures: Vec<String>,
}

impl CheckedSupportRow {
    fn has_self_contained_proof(&self) -> bool {
        !self.tests.is_empty() || !self.fixtures.is_empty()
    }
}

fn load_support_rows(root: &Path) -> Result<BTreeMap<String, CheckedSupportRow>> {
    let text = std::fs::read_to_string(root.join(SUPPORT_MATRIX_PATH))
        .with_context(|| format!("failed to read {SUPPORT_MATRIX_PATH}"))?;
    let matrix: SupportMatrix =
        toml::from_str(&text).with_context(|| format!("failed to parse {SUPPORT_MATRIX_PATH}"))?;
    let mut rows = BTreeMap::new();
    for row in matrix.row {
        let Some(id) = row.id else {
            continue;
        };
        let tests = if row.self_contained_tests.is_empty() {
            row.tests
        } else {
            row.self_contained_tests
        };
        rows.insert(
            id,
            CheckedSupportRow {
                feature_id: row.feature_id.unwrap_or_default(),
                status: row.status.unwrap_or_default(),
                tests,
                fixtures: row.fixtures,
            },
        );
    }
    Ok(rows)
}

fn supported_row_problem(
    row: &CoverageRow,
    support_rows: &BTreeMap<String, CheckedSupportRow>,
) -> Option<String> {
    let non_supported: Vec<&str> = row
        .decoder_support_rows
        .iter()
        .filter(|id| {
            support_rows
                .get(**id)
                .is_none_or(|support_row| support_row.status != "supported")
        })
        .copied()
        .collect();
    if !non_supported.is_empty() {
        return Some(format!(
            "supported coverage requires every linked decoder support row to be `supported`; non-supported rows: {}",
            non_supported.join(", ")
        ));
    }

    if row.tests.is_empty()
        && !row.decoder_support_rows.iter().any(|id| {
            support_rows
                .get(*id)
                .is_some_and(CheckedSupportRow::has_self_contained_proof)
        })
    {
        return Some(
            "supported rows require self-contained tests or a supported support row with proof"
                .to_owned(),
        );
    }

    None
}

fn load_feature_ids(root: &Path) -> Result<BTreeSet<String>> {
    let text = std::fs::read_to_string(root.join(IMPLEMENTATION_MATRIX_PATH))
        .with_context(|| format!("failed to read {IMPLEMENTATION_MATRIX_PATH}"))?;
    let matrix: ImplementationMatrix = toml::from_str(&text)
        .with_context(|| format!("failed to parse {IMPLEMENTATION_MATRIX_PATH}"))?;
    Ok(matrix
        .feature
        .into_iter()
        .filter_map(|feature| feature.id)
        .collect())
}

fn load_spec_sections(root: &Path) -> Result<BTreeSet<String>> {
    let text = std::fs::read_to_string(root.join(SPEC_INDEX_PATH))
        .with_context(|| format!("failed to read {SPEC_INDEX_PATH}"))?;
    Ok(parse_spec_sections(&text))
}

fn parse_spec_sections(text: &str) -> BTreeSet<String> {
    text.lines()
        .filter_map(|line| {
            let (before, _) = line.split_once("` |")?;
            let section = before.rsplit_once('`')?.1;
            Some(section.strip_prefix("§ ").unwrap_or(section).to_owned())
        })
        .collect()
}

fn load_decoder_diagnostic_ids(root: &Path) -> Result<BTreeSet<String>> {
    let text = std::fs::read_to_string(root.join(DIAGNOSTICS_DOC_PATH))
        .with_context(|| format!("failed to read {DIAGNOSTICS_DOC_PATH}"))?;
    Ok(parse_decoder_diagnostic_ids(&text))
}

fn parse_decoder_diagnostic_ids(text: &str) -> BTreeSet<String> {
    text.split('`')
        .filter(|token| token.starts_with("decode/"))
        .map(str::to_owned)
        .collect()
}

fn render_markdown(rows: &[CheckedCoverageRow<'_>]) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# Decoder Spec Coverage");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "Generated from `xtask/src/decoder_conformance_coverage.rs` by `cargo xtask decoder-conformance-coverage --format markdown`. Do not edit by hand."
    );
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "> This document maps AV2 decoder conformance ownership. It is not a full decoder conformance certificate until every normative decode-relevant row is `supported` with self-contained runtime proof and conforming streams no longer emit `decode/unsupported-feature`."
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "{} row(s).", rows.len());
    let _ = writeln!(out);

    let _ = writeln!(out, "## Status Counts");
    let _ = writeln!(out);
    let _ = writeln!(out, "| Status | Rows |");
    let _ = writeln!(out, "|---|---:|");
    let counts = counts_by_status(rows);
    for status in ROW_STATUSES {
        let _ = writeln!(out, "| `{status}` | {} |", counts.get(status).unwrap_or(&0));
    }
    let _ = writeln!(out);

    let _ = writeln!(out, "## Rows");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "| ID | Sections | Title | Normative | Owner | Support Rows | Feature IDs | Status | Tests | Fuzz | Evidence | Diagnostics | Notes |"
    );
    let _ = writeln!(out, "|---|---|---|---|---|---|---|---|---|---|---|---|---|");
    for checked in rows {
        let row = checked.row;
        let _ = writeln!(
            out,
            "| `{}` | {} | {} | `{}` | {} | {} | {} | `{}` | {} | {} | {} | {} | {} |",
            md_escape(row.id),
            list_cell(row.spec_sections),
            md_escape(row.spec_title),
            row.normative_status,
            md_escape(row.implementation_owner),
            code_list_cell(row.decoder_support_rows),
            code_list_cell(row.feature_ids),
            row.status,
            list_cell(row.tests),
            list_cell(row.fuzz_targets),
            list_cell(row.local_reference_evidence),
            code_list_cell(row.diagnostics),
            md_escape(row.notes),
        );
    }
    out
}

fn counts_by_status(rows: &[CheckedCoverageRow<'_>]) -> BTreeMap<&'static str, usize> {
    let mut counts = BTreeMap::new();
    for checked in rows {
        *counts.entry(checked.row.status).or_insert(0) += 1;
    }
    counts
}

fn list_cell(values: &[&str]) -> String {
    if values.is_empty() {
        return "none".to_owned();
    }
    values
        .iter()
        .map(|value| md_escape(value))
        .collect::<Vec<_>>()
        .join("<br>")
}

fn code_list_cell(values: &[&str]) -> String {
    if values.is_empty() {
        return "none".to_owned();
    }
    values
        .iter()
        .map(|value| format!("`{}`", md_escape(value)))
        .collect::<Vec<_>>()
        .join("<br>")
}

fn md_escape(cell: &str) -> String {
    cell.replace('|', "\\|")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn spec_index_parser_accepts_sections_and_annexes() {
        let sections = parse_spec_sections(
            "| `§ 7.21` | Output process | [x](x) | 1 |\n\
             | `Annex E` | Decoder model | [x](x) | 2 |\n",
        );
        assert!(sections.contains("7.21"));
        assert!(sections.contains("Annex E"));
    }

    #[test]
    fn diagnostics_parser_collects_decode_ids() {
        let diagnostics = parse_decoder_diagnostic_ids(
            "`decode/malformed-source` and `decode/resource-limit` plus `not/decode`",
        );
        assert!(diagnostics.contains("decode/malformed-source"));
        assert!(diagnostics.contains("decode/resource-limit"));
        assert!(!diagnostics.contains("not/decode"));
    }

    #[test]
    fn renderer_has_warning_and_status_counts() {
        let rendered = render_markdown(&[CheckedCoverageRow {
            row: &COVERAGE_ROWS[0],
        }]);
        assert!(rendered.contains("not a full decoder conformance certificate"));
        assert!(rendered.contains("| `partial` | 1 |"));
        assert!(rendered.contains("symbols-and-conventions"));
    }

    #[test]
    fn unsupported_rows_remain_visible() {
        let rendered = render_markdown(&[CheckedCoverageRow {
            row: &COVERAGE_ROWS[5],
        }]);
        assert!(rendered.contains("| `unsupported` | 1 |"));
        assert!(rendered.contains("decode-orchestration-and-random-access"));
    }

    #[test]
    fn validation_rejects_invalid_status_and_mixed_nonnormative_notes() {
        #[rustfmt::skip]
        static BAD_ROWS: &[CoverageRow] = &[
            CoverageRow { status: "done", ..COVERAGE_ROWS[0] },
            CoverageRow { id: "bad-mixed-notes", normative_status: "mixed", notes: "covers the non-normative portion", ..COVERAGE_ROWS[0] },
        ];
        let root = temp_root("decoder-conformance-invalid-status").unwrap();
        write_minimal_repo_files(&root);
        let err = validate_rows(&root, BAD_ROWS).expect_err("invalid status should fail");
        assert!(err.to_string().contains("status `done` is invalid"));
        assert!(
            err.to_string()
                .contains("requires notes explaining the normative portion")
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn validation_rejects_missing_support_row() {
        static BAD_ROWS: &[CoverageRow] = &[CoverageRow {
            decoder_support_rows: &["missing-row"],
            ..COVERAGE_ROWS[0]
        }];
        let root = temp_root("decoder-conformance-missing-support").unwrap();
        write_minimal_repo_files(&root);
        let err = validate_rows(&root, BAD_ROWS).expect_err("missing support row should fail");
        assert!(
            err.to_string()
                .contains("decoder_support_rows references unknown row `missing-row`")
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn validation_rejects_missing_reference_evidence() {
        static BAD_ROWS: &[CoverageRow] = &[CoverageRow {
            local_reference_evidence: &["docs/LOCAL-REFERENCE-EVIDENCE.toml::missing-evidence"],
            ..COVERAGE_ROWS[0]
        }];
        let root = temp_root("decoder-conformance-missing-evidence").unwrap();
        write_minimal_repo_files(&root);
        let err = validate_rows(&root, BAD_ROWS).expect_err("missing evidence should fail");
        assert!(
            err.to_string()
                .contains("references unknown evidence id `missing-evidence`")
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn supported_coverage_requires_supported_support_rows() {
        static BAD_ROWS: &[CoverageRow] = &[CoverageRow {
            id: "bad-supported",
            spec_sections: &["7.1"],
            spec_title: "bad supported row",
            normative_status: "normative",
            implementation_owner: "test",
            decoder_support_rows: &["bad-support"],
            feature_ids: &["CLI-DECODE"],
            status: "supported",
            tests: &[],
            fuzz_targets: &[],
            local_reference_evidence: &[],
            diagnostics: &[],
            notes: "test",
        }];
        let root = temp_root("decoder-conformance-bad-supported").unwrap();
        write_minimal_repo_files(&root);
        std::fs::write(
            root.join("docs").join("DECODER-SUPPORT-MATRIX.toml"),
            "[[row]]\n\
             id = \"bad-support\"\n\
             feature_id = \"CLI-DECODE\"\n\
             status = \"unsupported-intentional\"\n\
             self_contained_tests = [\"cargo test\"]\n\
             fixtures = []\n",
        )
        .unwrap();
        let err = validate_rows(&root, BAD_ROWS)
            .expect_err("unsupported support row must not prove supported coverage");
        assert!(
            err.to_string()
                .contains("supported coverage requires every linked decoder support row")
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn check_detects_drift() -> Result<()> {
        let root = temp_root("decoder-conformance-drift")?;
        write_minimal_repo_files(&root);
        let expected = render_markdown(&validate_rows(&root, COVERAGE_ROWS)?);
        let docs = root.join("docs");
        std::fs::write(docs.join("DECODER-SPEC-COVERAGE.md"), expected)?;
        run_check_decoder_conformance_coverage(&root)?;
        std::fs::write(docs.join("DECODER-SPEC-COVERAGE.md"), "stale\n")?;
        let err = run_check_decoder_conformance_coverage(&root).expect_err("drift should fail");
        assert!(err.to_string().contains(REGEN_COMMAND));
        let _ = std::fs::remove_dir_all(&root);
        Ok(())
    }

    fn write_minimal_repo_files(root: &Path) {
        let docs = root.join("docs");
        std::fs::create_dir_all(docs.join("spec/av2/1.0.0")).unwrap();
        std::fs::write(
            docs.join("spec/av2/1.0.0/index.md"),
            spec_index_for_all_static_rows(),
        )
        .unwrap();
        std::fs::write(docs.join("DECODER-SUPPORT-MATRIX.toml"), support_matrix()).unwrap();
        std::fs::write(
            docs.join("IMPLEMENTATION-MATRIX.toml"),
            implementation_matrix(),
        )
        .unwrap();
        std::fs::write(docs.join("DECODER-DIAGNOSTICS.md"), diagnostics_doc()).unwrap();
        std::fs::write(
            docs.join("LOCAL-REFERENCE-EVIDENCE.toml"),
            local_reference_evidence(),
        )
        .unwrap();
    }

    fn spec_index_for_all_static_rows() -> String {
        let mut sections = BTreeSet::new();
        for row in COVERAGE_ROWS {
            sections.extend(row.spec_sections.iter().copied());
        }
        sections
            .into_iter()
            .map(|section| {
                if section.starts_with("Annex ") {
                    format!("| `{section}` | title | [x](x) | 1 |\n")
                } else {
                    format!("| `§ {section}` | title | [x](x) | 1 |\n")
                }
            })
            .collect()
    }

    fn support_matrix() -> String {
        let mut ids = BTreeMap::new();
        for row in COVERAGE_ROWS {
            for (id, feature_id) in row.decoder_support_rows.iter().zip(row.feature_ids.iter()) {
                ids.entry(*id).or_insert(*feature_id);
            }
        }
        ids.into_iter()
            .map(|(id, feature_id)| {
                format!(
                    "[[row]]\nid = \"{id}\"\nfeature_id = \"{feature_id}\"\nstatus = \"supported\"\nself_contained_tests = [\"cargo test\"]\nfixtures = []\n\n"
                )
            })
            .collect()
    }

    fn implementation_matrix() -> String {
        let mut ids = BTreeSet::new();
        for row in COVERAGE_ROWS {
            ids.extend(row.feature_ids.iter().copied());
        }
        ids.into_iter()
            .map(|id| format!("[[feature]]\nid = \"{id}\"\n\n"))
            .collect()
    }

    fn diagnostics_doc() -> String {
        "`decode/malformed-source` `decode/resource-limit` `decode/unsupported-feature`".to_owned()
    }

    fn local_reference_evidence() -> String {
        "manifest_version = 1\nlast_reviewed = \"2026-06-15\"\n".to_owned()
    }

    fn temp_root(name: &str) -> Result<PathBuf> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time is after Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("{name}-{}-{nanos}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root)?;
        Ok(root)
    }
}

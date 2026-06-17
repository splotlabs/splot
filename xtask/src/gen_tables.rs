// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `cargo xtask gen-tables`: generate the AV2 § 9 additional tables
//! (feature `AV2-9-ADDITIONAL-TABLES`) from the committed spec attachment
//! `all_tables.h`. Most modules are written into `crates/splot-core/src/tables/`;
//! the § 9.6/§ 9.7 transform-kernel modules instead live in the dependency-free
//! `crates/splot-tables/src/tables/` crate so `splot-recon` can use them without
//! depending on `splot-core` (see [`output_dir_for`]).
//!
//! The attachment (`docs/spec/av2/1.0.0/attachments/all_tables.h`) is a verbatim,
//! sha256-pinned copy of the spec's § 9 "additional tables" C header (see
//! `provenance.toml [attachments]` and the `check-spec-mirror` gate). This module
//! parses that header and emits one Rust module per § 9 subsection, with every
//! table value taken **directly** from the attachment — nothing is hand-transcribed.
//!
//! Coverage is explicit and loud:
//!
//! - Tables whose element values are all integer literals are **generated** as
//!   nested fixed-size `i32` arrays (the array shape is inferred from the brace
//!   nesting; named dimension expressions are recorded as a doc comment only).
//! - The two § 9.2 partition-size tables with `BLOCK_*` element values are also
//!   generated after resolving those spec-defined block-size symbols.
//! - Tables whose element values use other unresolved symbolic tokens (AV2 enum
//!   names like `TX_4X4`, or the `reserved` placeholder) cannot be emitted as
//!   integer arrays without an enum-value map, so they are listed in an
//!   **explicit skip-allowlist** ([`SKIP_ALLOWLIST`]) and enumerated in the run
//!   report.
//! - Any other unmodeled construct (a declaration the parser cannot classify, or a
//!   symbolic table that is *not* in the allowlist) **fails loudly** rather than
//!   being silently dropped.
//!
//! `--check` regenerates into memory and diffs against the committed files,
//! failing on any drift; it is wired into `cargo xtask ci`.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context as _, Result, bail};

mod block_symbols;

/// Relative path (from the workspace root) of the committed § 9 attachment.
const ATTACHMENT_REL: &str = "docs/spec/av2/1.0.0/attachments/all_tables.h";

/// Directory (from the workspace root) the § 9 mirror subsection files live in,
/// used to derive each table's owning module from the spec's own grouping.
const MIRROR_SECTION_DIR: &str = "docs/spec/av2/1.0.0/09-additional-tables";

/// Output directory (from the workspace root) for the in-`splot-core` generated
/// table modules.
const CORE_TABLES_DIR: &str = "crates/splot-core/src/tables";

/// Output directory for the shared transform-kernel table modules, in the
/// dependency-free `splot-tables` crate.
const SHARED_TABLES_DIR: &str = "crates/splot-tables/src/tables";

/// Returns the output directory for a § 9 module. The § 9.6 1D transform, § 9.7
/// secondary transform kernel, and § 9.4 quantizer-matrix tables live in the
/// shared `splot-tables` crate (so `splot-recon` can consume them without
/// depending on `splot-core`); every other module stays in `splot-core`.
fn output_dir_for(module: &str) -> &'static str {
    match module {
        "transform_1d" | "secondary_transform" | "quantizer" => SHARED_TABLES_DIR,
        _ => CORE_TABLES_DIR,
    }
}

/// Every distinct generated-table output directory, in a deterministic order.
const OUTPUT_DIRS: &[&str] = &[CORE_TABLES_DIR, SHARED_TABLES_DIR];

/// Counts emitted module files (i.e. every generated file except the per-directory
/// `mod.rs`).
fn module_file_count(files: &BTreeMap<String, String>) -> usize {
    files.keys().filter(|rel| !rel.ends_with("/mod.rs")).count()
}

/// The § 9 subsection modules, in spec order: `(module_file_stem, mirror_md, § N.M,
/// human title)`. The `mirror_md` file is parsed to learn which tables belong to
/// each module (the spec's grouping is the authority, not a hand-kept list).
const SECTIONS: &[Section] = &[
    Section {
        module: "conversion",
        mirror_md: "09-02-conversion-tables.md",
        spec: "9.2",
        title: "Conversion tables",
    },
    Section {
        module: "cdf",
        mirror_md: "09-03-default-cdf-tables.md",
        spec: "9.3",
        title: "Default CDF tables",
    },
    Section {
        module: "quantizer",
        mirror_md: "09-04-quantizer-matrix-tables.md",
        spec: "9.4",
        title: "Quantizer matrix tables",
    },
    Section {
        module: "warp_filter",
        mirror_md: "09-05-warp-filter-tables.md",
        spec: "9.5",
        title: "Warp filter tables",
    },
    Section {
        module: "transform_1d",
        mirror_md: "09-06-1d-transform-tables.md",
        spec: "9.6",
        title: "1D transform tables",
    },
    Section {
        module: "secondary_transform",
        mirror_md: "09-07-secondary-transform-tables.md",
        spec: "9.7",
        title: "Secondary transform tables",
    },
    Section {
        module: "loop_restoration",
        mirror_md: "09-08-loop-restoration-tables.md",
        spec: "9.8",
        title: "Loop restoration tables",
    },
];

struct Section {
    module: &'static str,
    mirror_md: &'static str,
    spec: &'static str,
    title: &'static str,
}

/// Tables in the attachment whose element values are not integer literals and so
/// cannot be emitted as typed integer arrays without an AV2 enum-value resolution
/// map (out of scope for this change). Each is listed with the reason it is
/// skipped; the run report enumerates them. A symbolic table NOT in this list is a
/// hard error (loud failure), preventing silent truncation.
const SKIP_ALLOWLIST: &[(&str, &str)] = &[
    (
        "Adjusted_Tx_Size",
        "TxSize enum element values (TX_4X4, ...)",
    ),
    ("Max_Tx_Size_Rect", "TxSize enum element values (TX_*)"),
    (
        "Mode_To_Txfm",
        "TxType enum element values (DCT_DCT, ADST_ADST, ...)",
    ),
    (
        "Size_To_Tx_Type_Group_Vert_And_Horz",
        "BlockSize enum element value (BLOCK_INVALID)",
    ),
    (
        "Size_To_Tx_Type_Group_Vert_Or_Horz",
        "BlockSize enum element value (BLOCK_INVALID)",
    ),
    ("Tx_Size_Sqr", "TxSize enum element values (TX_*)"),
    ("Tx_Size_Sqr_Up", "TxSize enum element values (TX_*)"),
    (
        "Tile_Area_Scaling_Factor",
        "`reserved` placeholder element tokens",
    ),
    (
        "Tile_Width_Scaling_Factor",
        "`reserved` placeholder element tokens",
    ),
];

/// A parsed table declaration from the attachment.
struct Decl {
    /// The table name exactly as written, e.g. `Default_Skip_Cdf`.
    name: String,
    /// The dimension expressions as written between the name and `=`, e.g.
    /// `[ COEFF_CDF_Q_CTXS ][ 3 ]`. Recorded verbatim for the generated doc
    /// comment; not used to size the Rust array (the brace nesting does that).
    dims: String,
    /// `true` if every element value is an integer literal (emittable as an `i32`
    /// array); `false` if any symbolic token appears (needs the skip-allowlist).
    numeric: bool,
    /// The raw brace body `{ ... }` with comments already stripped.
    body: String,
}

/// Entry point for `cargo xtask gen-tables [--check]`.
pub fn run_gen_tables(root: &Path, check: bool) -> Result<()> {
    let outputs = generate(root)?;

    if check {
        let mut drift = Vec::new();
        for (rel, content) in &outputs.files {
            let path = root.join(rel);
            match std::fs::read_to_string(&path) {
                Ok(on_disk) if &on_disk == content => {}
                Ok(_) => drift.push(format!("out of date: {rel}")),
                Err(_) => drift.push(format!("missing: {rel}")),
            }
        }
        // Also flag stray generated files that the generator would no longer
        // emit, across every output directory.
        for dir_rel in OUTPUT_DIRS {
            let dir = root.join(dir_rel);
            if dir.is_dir() {
                for entry in std::fs::read_dir(&dir)
                    .with_context(|| format!("failed to read {}", dir.display()))?
                {
                    let entry = entry?;
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                            bail!("non-UTF-8 filename under {dir_rel}: {}", path.display());
                        };
                        let rel = format!("{dir_rel}/{name}");
                        if !outputs.files.contains_key(&rel) {
                            drift.push(format!("unexpected (would be removed): {rel}"));
                        }
                    }
                }
            }
        }
        if !drift.is_empty() {
            for d in &drift {
                eprintln!("gen-tables --check: {d}");
            }
            bail!(
                "gen-tables --check: {} generated file(s) drifted; run `cargo xtask gen-tables`",
                drift.len()
            );
        }
        eprintln!(
            "gen-tables --check: ok ({} module(s), {} table(s) generated, {} skipped)",
            module_file_count(&outputs.files),
            outputs.generated,
            outputs.skipped.len()
        );
        return Ok(());
    }

    for dir_rel in OUTPUT_DIRS {
        std::fs::create_dir_all(root.join(dir_rel))
            .with_context(|| format!("failed to create {dir_rel}"))?;
        // Remove stale generated modules first (a renamed/no-longer-emitted table
        // group would otherwise survive every regeneration and keep --check failing
        // until deleted by hand — codex review, PR #66).
        for entry in std::fs::read_dir(root.join(dir_rel))
            .with_context(|| format!("failed to read {dir_rel}"))?
        {
            let entry = entry.with_context(|| format!("failed to read an entry in {dir_rel}"))?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                    bail!("non-UTF-8 filename under {dir_rel}: {}", path.display());
                };
                let rel = format!("{dir_rel}/{name}");
                if !outputs.files.contains_key(&rel) {
                    std::fs::remove_file(&path)
                        .with_context(|| format!("failed to remove stale {rel}"))?;
                    eprintln!("gen-tables: removed stale {rel}");
                }
            }
        }
    }
    for (rel, content) in &outputs.files {
        std::fs::write(root.join(rel), content)
            .with_context(|| format!("failed to write {rel}"))?;
    }
    eprintln!(
        "gen-tables: wrote {} module(s), {} table(s) generated.",
        module_file_count(&outputs.files),
        outputs.generated
    );
    if !outputs.skipped.is_empty() {
        eprintln!(
            "gen-tables: {} table(s) skipped (symbolic element values, see SKIP_ALLOWLIST):",
            outputs.skipped.len()
        );
        for (name, reason) in &outputs.skipped {
            eprintln!("    - {name}: {reason}");
        }
    }
    Ok(())
}

/// The result of a full generation pass: the file contents keyed by repo-relative
/// path, plus a report of what was generated vs skipped.
struct Outputs {
    /// repo-relative path -> file content (deterministically ordered).
    files: BTreeMap<String, String>,
    /// count of tables emitted as Rust consts.
    generated: usize,
    /// `(table_name, reason)` for each allowlisted skip, in spec/file order.
    skipped: Vec<(String, String)>,
}

/// A table body ready for rendering.
struct GeneratedTable<'a> {
    decl: &'a Decl,
    body: Cow<'a, str>,
}

/// Parse the attachment, assign each table to a § 9 module, and render every
/// generated module plus `mod.rs`. Pure over the committed inputs (no timestamps,
/// stable ordering), so two runs are byte-identical.
fn generate(root: &Path) -> Result<Outputs> {
    let attachment_path = root.join(ATTACHMENT_REL);
    let raw = std::fs::read_to_string(&attachment_path)
        .with_context(|| format!("failed to read {}", attachment_path.display()))?;
    let decls = parse_decls(&raw)?;

    let section_of = build_section_map(root, &decls)?;

    let skip_reason: BTreeMap<&str, &str> = SKIP_ALLOWLIST.iter().copied().collect();

    // Group declarations by module, preserving the attachment's declaration order
    // within each module for stable output.
    let mut by_module: BTreeMap<&'static str, Vec<GeneratedTable<'_>>> = BTreeMap::new();
    let mut skipped: Vec<(String, String)> = Vec::new();
    let mut generated = 0usize;

    for decl in &decls {
        let body = if decl.numeric {
            Cow::Borrowed(decl.body.as_str())
        } else if let Some(body) = block_symbols::resolve_body(&decl.name, &decl.body)? {
            Cow::Owned(body)
        } else {
            match skip_reason.get(decl.name.as_str()) {
                Some(reason) => {
                    skipped.push((decl.name.clone(), (*reason).to_string()));
                    continue;
                }
                None => bail!(
                    "gen-tables: table `{}` has symbolic element values but is not in \
                     SKIP_ALLOWLIST — model it or add it to the allowlist (never silently skip)",
                    decl.name
                ),
            }
        };
        let module = section_of.get(decl.name.as_str()).copied().ok_or_else(|| {
            anyhow::anyhow!(
                "gen-tables: table `{}` could not be assigned to a § 9 module",
                decl.name
            )
        })?;
        by_module
            .entry(module)
            .or_default()
            .push(GeneratedTable { decl, body });
        generated += 1;
    }

    let mut files: BTreeMap<String, String> = BTreeMap::new();
    let mut emitted_by_dir: BTreeMap<&'static str, Vec<&Section>> = BTreeMap::new();
    for section in SECTIONS {
        let Some(decls) = by_module.get(section.module) else {
            continue;
        };
        let content = render_module(section, decls)?;
        let dir = output_dir_for(section.module);
        files.insert(format!("{dir}/{}.rs", section.module), content);
        emitted_by_dir.entry(dir).or_default().push(section);
    }

    // One `mod.rs` per output directory, listing only that directory's modules.
    for (dir, sections) in &emitted_by_dir {
        files.insert(format!("{dir}/mod.rs"), render_mod_rs(sections));
    }

    Ok(Outputs {
        files,
        generated,
        skipped,
    })
}

/// Build a `table_name -> module` map. The owning module is read from the spec
/// mirror's own § 9 subsection files (the authoritative grouping). Two-line
/// declarations whose name is not matched by the per-line mirror scan fall back to
/// the rule "an unmatched `*_Cdf` table belongs to § 9.3 (the CDF module)"; any
/// table matched by neither is a hard error.
fn build_section_map(root: &Path, decls: &[Decl]) -> Result<BTreeMap<String, &'static str>> {
    let mut map: BTreeMap<String, &'static str> = BTreeMap::new();
    for section in SECTIONS {
        let md_path = root.join(MIRROR_SECTION_DIR).join(section.mirror_md);
        let md = std::fs::read_to_string(&md_path)
            .with_context(|| format!("failed to read {}", md_path.display()))?;
        for line in md.lines() {
            // A table declaration line in the mirror text: leading whitespace, a
            // capitalized identifier, then `[`.
            let trimmed = line.trim_start();
            if let Some(name) = table_name_before_bracket(trimmed) {
                map.entry(name.to_string()).or_insert(section.module);
            }
        }
    }

    for decl in decls {
        if !map.contains_key(&decl.name) {
            if decl.name.ends_with("_Cdf") {
                map.insert(decl.name.clone(), "cdf");
            } else {
                bail!(
                    "gen-tables: table `{}` is not grouped by any § 9 mirror section and is \
                     not a `*_Cdf` table; the section map cannot place it",
                    decl.name
                );
            }
        }
    }

    Ok(map)
}

/// If `s` begins with `Ident[` (a capitalized identifier directly followed by an
/// opening bracket, ignoring spaces), return the identifier.
fn table_name_before_bracket(s: &str) -> Option<&str> {
    let mut chars = s.char_indices();
    let (_, first) = chars.next()?;
    if !first.is_ascii_uppercase() {
        return None;
    }
    let mut end = first.len_utf8();
    for (i, c) in chars {
        if c.is_ascii_alphanumeric() || c == '_' {
            end = i + c.len_utf8();
        } else {
            // The identifier must be followed by `[` (allowing intervening spaces).
            let rest = s[end..].trim_start();
            if rest.starts_with('[') {
                return Some(&s[..end]);
            }
            return None;
        }
    }
    None
}

/// Parse every `Name[dims...] = { ... }` declaration from the attachment after
/// stripping `/* */` and `//` comments. The parser is deliberately strict: it
/// recognizes exactly this declaration shape and classifies each as numeric (all
/// integer literals) or symbolic.
fn parse_decls(raw: &str) -> Result<Vec<Decl>> {
    let text = strip_comments(raw);
    let bytes = text.as_bytes();
    let mut decls = Vec::new();
    let mut i = 0usize;
    let n = bytes.len();

    while i < n {
        // Find the start of a candidate identifier (a letter or underscore at a
        // position not preceded by an identifier char).
        let c = bytes[i];
        let prev_ident = i > 0 && is_ident_byte(bytes[i - 1]);
        if !prev_ident && (c.is_ascii_alphabetic() || c == b'_') {
            // Read the identifier.
            let name_start = i;
            while i < n && is_ident_byte(bytes[i]) {
                i += 1;
            }
            let name = &text[name_start..i];
            // Skip whitespace, then optional `[ ... ]` dimension groups, then `=`.
            let after_name = i;
            let mut j = i;
            skip_ws(bytes, &mut j, n);
            let dims_start = j;
            // Consume zero or more bracket groups.
            loop {
                skip_ws(bytes, &mut j, n);
                if j < n && bytes[j] == b'[' {
                    // advance to matching ']'
                    while j < n && bytes[j] != b']' {
                        j += 1;
                    }
                    if j < n {
                        j += 1; // consume ']'
                    }
                } else {
                    break;
                }
            }
            let dims_end = j;
            skip_ws(bytes, &mut j, n);
            if j < n && bytes[j] == b'=' {
                j += 1;
                skip_ws(bytes, &mut j, n);
                if j < n && bytes[j] == b'{' {
                    // Brace-match the body.
                    let body_start = j;
                    let mut depth = 0i32;
                    while j < n {
                        match bytes[j] {
                            b'{' => depth += 1,
                            b'}' => {
                                depth -= 1;
                                if depth == 0 {
                                    j += 1;
                                    break;
                                }
                            }
                            _ => {}
                        }
                        j += 1;
                    }
                    if depth != 0 {
                        bail!(
                            "gen-tables: unbalanced braces in declaration `{name}` (malformed attachment)"
                        );
                    }
                    let body = text[body_start..j].to_string();
                    let numeric = is_numeric_body(&body);
                    let dims = text[dims_start..dims_end].trim().to_string();
                    decls.push(Decl {
                        name: name.to_string(),
                        dims,
                        numeric,
                        body,
                    });
                    i = j;
                    continue;
                }
            }
            // Not a declaration; resume scanning right after the identifier.
            i = after_name;
            continue;
        }
        i += 1;
    }

    if decls.is_empty() {
        bail!(
            "gen-tables: parsed zero declarations from {ATTACHMENT_REL} (parser or attachment broken)"
        );
    }
    Ok(decls)
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn skip_ws(bytes: &[u8], i: &mut usize, n: usize) {
    while *i < n && bytes[*i].is_ascii_whitespace() {
        *i += 1;
    }
}

/// Replace `/* ... */` and `// ...` comments with spaces (preserving newlines so
/// line-based reasoning stays valid).
fn strip_comments(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let n = bytes.len();
    let mut out = String::with_capacity(raw.len());
    let mut i = 0usize;
    while i < n {
        if i + 1 < n && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < n && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                if bytes[i] == b'\n' {
                    out.push('\n');
                }
                i += 1;
            }
            i += 2; // consume "*/"
            out.push(' ');
        } else if i + 1 < n && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            i += 2;
            while i < n && bytes[i] != b'\n' {
                i += 1;
            }
            // leave the newline (if any) to the outer loop
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

/// A body is numeric iff it contains no identifier characters outside numbers —
/// i.e. every element is an integer literal (optionally signed). Anything else
/// (an enum token, `reserved`, ...) makes it symbolic.
///
/// A `-` is only allowed as a unary sign for the following number; the attachment
/// sometimes prints a space between the sign and the digits (`- 1`), so whitespace
/// between the sign and the digit is tolerated.
///
/// ASSUMPTION (claude review, PR #66): a binary subtraction like `1-2` would be
/// misread as the two values `1` and `-2` — the attachment contains no arithmetic
/// expressions (only literals), and the determinism test's exact table count would
/// flag a future spec version that introduced one.
fn is_numeric_body(body: &str) -> bool {
    let bytes = body.as_bytes();
    for (k, &b) in bytes.iter().enumerate() {
        if b.is_ascii_alphabetic() || b == b'_' {
            return false;
        }
        if b == b'-' {
            let mut j = k + 1;
            while bytes.get(j).is_some_and(|c| c.is_ascii_whitespace()) {
                j += 1;
            }
            if !bytes.get(j).is_some_and(u8::is_ascii_digit) {
                return false;
            }
        }
    }
    true
}

/// Render one § 9 module file: SPDX + provenance header, then one `pub static` per
/// table.
fn render_module(section: &Section, decls: &[GeneratedTable<'_>]) -> Result<String> {
    // Writing to a `String` is infallible, so `write!`/`writeln!` results are
    // discarded with `let _ = ...` (the workspace denies `unwrap`).
    let mut out = String::new();
    out.push_str(GENERATED_HEADER);
    let _ = writeln!(out, "//! AV2 § {} — {}.", section.spec, section.title);
    out.push_str("//!\n");
    let _ = writeln!(
        out,
        "//! Generated by `cargo xtask gen-tables` from the committed § 9 attachment"
    );
    let _ = writeln!(out, "//! ({ATTACHMENT_REL}). Do not edit by hand.");
    out.push('\n');
    // Generated nested arrays trip a few pedantic/style lints; allow them
    // module-wide since the data is machine-emitted and not meant to be read.
    out.push_str(
        "#![allow(clippy::all, clippy::pedantic)]\n#![allow(clippy::unreadable_literal)]\n\n",
    );

    for table in decls {
        let decl = table.decl;
        let rust_name = to_screaming_snake(&decl.name);
        let value = render_value(&table.body)?;
        let ty = array_type(&table.body)?;
        // Collapse the declaration dims onto one line so multi-line declarations
        // (the dims spill across lines in the attachment) stay inside the `///`.
        let dims = normalize_ws(&decl.dims);
        let _ = writeln!(
            out,
            "/// `{}{}` (AV2 § {}, generated from `{ATTACHMENT_REL}`).",
            decl.name, dims, section.spec
        );
        // The const value is one long array literal; `#[rustfmt::skip]` keeps it on
        // a single line so `cargo fmt --check` is stable and the generator's output
        // does not depend on the installed rustfmt version.
        let _ = writeln!(out, "#[rustfmt::skip]");
        // `pub static`, not `pub const`: a const is value-substituted at every
        // mention, so the large §9 arrays (e.g. the ~216 KiB QUANTIZER_MATRIX)
        // could be re-materialized at call sites; a static has one read-only
        // storage location (codex review, PR #66).
        let _ = writeln!(out, "pub static {rust_name}: {ty} = {value};");
        out.push('\n');
    }

    // End with exactly one trailing newline (rustfmt-clean), not the blank line
    // left after the last item.
    Ok(trim_trailing_blank_lines(&out))
}

/// Trim trailing blank lines so the file ends with exactly one `\n`.
fn trim_trailing_blank_lines(s: &str) -> String {
    let mut out = s.trim_end().to_string();
    out.push('\n');
    out
}

/// Render the `tables/mod.rs` that declares and re-exports every generated module.
fn render_mod_rs(sections: &[&Section]) -> String {
    let mut out = String::new();
    out.push_str(GENERATED_HEADER);
    out.push_str("//! AV2 § 9 additional tables, generated by `cargo xtask gen-tables`.\n");
    out.push_str("//!\n");
    out.push_str("//! One submodule per § 9 subsection; see each module's docs for the\n");
    out.push_str("//! generating attachment and spec citation.\n\n");
    // Emit `pub mod` declarations alphabetically by module name so the file is
    // rustfmt-clean (rustfmt reorders module declarations).
    let mut sorted: Vec<&&Section> = sections.iter().collect();
    sorted.sort_by_key(|s| s.module);
    for section in sorted {
        let _ = writeln!(
            out,
            "/// AV2 § {} — {}.\npub mod {};",
            section.spec, section.title, section.module
        );
    }
    trim_trailing_blank_lines(&out)
}

/// The SPDX + "generated, do not edit" banner that opens every generated file.
const GENERATED_HEADER: &str = "\
// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

// @generated by `cargo xtask gen-tables` — DO NOT EDIT.
// Source: AV2 v1.0.0 § 9 additional tables, docs/spec/av2/1.0.0/attachments/all_tables.h
// (sha256 c3837e1c3b333e9ed51885c642562b519e3c3ed2ab385557d296c30a29c04ca1).
// Regenerate with `cargo xtask gen-tables`; the drift check runs in `cargo xtask ci`.

";

/// Convert a spec table name (`Default_Skip_Cdf`) to a Rust `SCREAMING_SNAKE_CASE`
/// const name (`DEFAULT_SKIP_CDF`).
fn to_screaming_snake(name: &str) -> String {
    name.to_ascii_uppercase()
}

/// Collapse every run of ASCII whitespace to a single space and trim the ends, so
/// a value can be safely embedded in a single-line `///` doc comment.
fn normalize_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Render the array literal for a numeric body, normalizing whitespace to a single
/// canonical form so output is deterministic regardless of the attachment's
/// formatting. Nesting becomes nested `[ ... ]`.
fn render_value(body: &str) -> Result<String> {
    let toks = tokenize_body(body)?;
    let mut idx = 0usize;
    let s = render_node(&toks, &mut idx)?;
    if idx != toks.len() {
        bail!("gen-tables: trailing tokens after array body");
    }
    Ok(s)
}

/// Tokens of a numeric array body: braces, commas, and integer literals.
enum Tok {
    Open,
    Close,
    Int(i64),
}

fn tokenize_body(body: &str) -> Result<Vec<Tok>> {
    let bytes = body.as_bytes();
    let n = bytes.len();
    let mut toks = Vec::new();
    let mut i = 0usize;
    while i < n {
        let b = bytes[i];
        if b.is_ascii_whitespace() || b == b',' {
            i += 1;
        } else if b == b'{' {
            toks.push(Tok::Open);
            i += 1;
        } else if b == b'}' {
            toks.push(Tok::Close);
            i += 1;
        } else if b == b'-' || b.is_ascii_digit() {
            let mut neg = false;
            if b == b'-' {
                neg = true;
                i += 1;
                // The attachment may print a space between the sign and digits
                // (`- 1`); skip it so the literal parses.
                while i < n && bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
            }
            let digits_start = i;
            while i < n && bytes[i].is_ascii_digit() {
                i += 1;
            }
            let digits = &body[digits_start..i];
            let mag: i64 = digits
                .parse()
                .with_context(|| format!("gen-tables: bad integer literal `{digits}`"))?;
            toks.push(Tok::Int(if neg { -mag } else { mag }));
        } else {
            bail!("gen-tables: unexpected byte 0x{b:02x} in numeric body");
        }
    }
    Ok(toks)
}

/// Recursively render one node (a `{...}` group or a leaf integer) from the token
/// stream as a Rust array literal / integer.
fn render_node(toks: &[Tok], idx: &mut usize) -> Result<String> {
    match toks.get(*idx) {
        Some(Tok::Int(v)) => {
            *idx += 1;
            // The generated consts are typed i32: reject out-of-range values here with
            // a clear generator error instead of deferring to a rustc type error in the
            // generated file (claude review, PR #66).
            if i32::try_from(*v).is_err() {
                bail!("table value {v} does not fit the generated i32 type");
            }
            Ok(v.to_string())
        }
        Some(Tok::Open) => {
            *idx += 1;
            let mut parts = Vec::new();
            loop {
                match toks.get(*idx) {
                    Some(Tok::Close) => {
                        *idx += 1;
                        break;
                    }
                    None => bail!("gen-tables: unterminated array group"),
                    _ => parts.push(render_node(toks, idx)?),
                }
            }
            Ok(format!("[{}]", parts.join(", ")))
        }
        Some(Tok::Close) => bail!("gen-tables: unexpected '}}' in array body"),
        None => bail!("gen-tables: unexpected end of array body"),
    }
}

/// Compute the Rust fixed-size array type (`[[i32; N]; M]`) from the brace nesting
/// and group lengths of a numeric body. Requires every sibling group at a level to
/// have equal length (a rectangular array); a ragged array is a hard error.
fn array_type(body: &str) -> Result<String> {
    let toks = tokenize_body(body)?;
    let mut idx = 0usize;
    let dims = shape(&toks, &mut idx)?;
    if idx != toks.len() {
        bail!("gen-tables: trailing tokens while computing array shape");
    }
    let mut ty = String::from("i32");
    for &d in dims.iter().rev() {
        ty = format!("[{ty}; {d}]");
    }
    Ok(ty)
}

/// Returns the dimension list of a rectangular nested array, validating that
/// sibling groups have matching lengths and depth.
fn shape(toks: &[Tok], idx: &mut usize) -> Result<Vec<usize>> {
    match toks.get(*idx) {
        Some(Tok::Int(_)) => {
            *idx += 1;
            Ok(vec![])
        }
        Some(Tok::Open) => {
            *idx += 1;
            let mut count = 0usize;
            let mut child_shape: Option<Vec<usize>> = None;
            loop {
                match toks.get(*idx) {
                    Some(Tok::Close) => {
                        *idx += 1;
                        break;
                    }
                    None => bail!("gen-tables: unterminated group while computing shape"),
                    _ => {
                        let s = shape(toks, idx)?;
                        match &child_shape {
                            None => child_shape = Some(s),
                            Some(prev) if *prev != s => bail!(
                                "gen-tables: ragged array (sibling groups differ in shape: {prev:?} vs {s:?})"
                            ),
                            Some(_) => {}
                        }
                        count += 1;
                    }
                }
            }
            let mut dims = vec![count];
            if let Some(child) = child_shape {
                dims.extend(child);
            }
            Ok(dims)
        }
        Some(Tok::Close) => bail!("gen-tables: unexpected '}}' while computing shape"),
        None => bail!("gen-tables: unexpected end while computing shape"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_block_and_line_comments() {
        let src = "A[2] = { 1, /* mid */ 2 } // tail\nB[1] = { 3 }\n";
        let stripped = strip_comments(src);
        assert!(!stripped.contains("mid"));
        assert!(!stripped.contains("tail"));
        assert!(stripped.contains('3'));
    }

    #[test]
    fn parses_numeric_and_symbolic_tables() -> Result<()> {
        let src = "Foo[3] = { 1, 2, 3 }\nBar[2] = { TX_4X4, TX_8X8 }\n";
        let decls = parse_decls(src)?;
        assert_eq!(decls.len(), 2);
        assert_eq!(decls[0].name, "Foo");
        assert!(decls[0].numeric);
        assert_eq!(decls[1].name, "Bar");
        assert!(!decls[1].numeric);
        Ok(())
    }

    #[test]
    fn renders_nested_array_value_and_type() -> Result<()> {
        let body = "{ { 1, 2 }, { -3, 4 } }";
        assert_eq!(render_value(body)?, "[[1, 2], [-3, 4]]");
        assert_eq!(array_type(body)?, "[[i32; 2]; 2]");
        Ok(())
    }

    #[test]
    fn flat_array_type_is_one_dimensional() -> Result<()> {
        let body = "{ 5, 6, 7 }";
        assert_eq!(array_type(body)?, "[i32; 3]");
        assert_eq!(render_value(body)?, "[5, 6, 7]");
        Ok(())
    }

    #[test]
    fn ragged_array_is_rejected() {
        let body = "{ { 1, 2 }, { 3 } }";
        assert!(array_type(body).is_err());
    }

    #[test]
    fn negative_literals_are_numeric() {
        assert!(is_numeric_body("{ -1, 2, -3 }"));
        assert!(!is_numeric_body("{ reserved, 2 }"));
        assert!(!is_numeric_body("{ TX_4X4 }"));
    }

    #[test]
    fn table_name_before_bracket_matches_decl_lines() {
        assert_eq!(table_name_before_bracket("Foo_Bar[ 3 ] ="), Some("Foo_Bar"));
        assert_eq!(table_name_before_bracket("Foo [2][3]"), Some("Foo"));
        assert_eq!(table_name_before_bracket("lowercase[2]"), None);
        assert_eq!(table_name_before_bracket("Foo = 3"), None);
    }

    /// Workspace root for tests (the parent of this xtask crate).
    fn workspace_root() -> Result<std::path::PathBuf> {
        Ok(std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .context("xtask manifest has no parent directory")?
            .to_path_buf())
    }

    #[test]
    fn regeneration_is_byte_identical_and_deterministic() -> Result<()> {
        let root = workspace_root()?;
        // Two independent generation passes over the committed attachment must
        // produce byte-identical output (no timestamps, stable ordering).
        let a = generate(&root)?;
        let b = generate(&root)?;
        assert_eq!(
            a.files.keys().collect::<Vec<_>>(),
            b.files.keys().collect::<Vec<_>>()
        );
        for (rel, content) in &a.files {
            assert_eq!(
                b.files.get(rel),
                Some(content),
                "nondeterministic output for {rel}"
            );
        }
        // The committed files on disk must match the freshly generated output.
        for (rel, content) in &a.files {
            let on_disk = std::fs::read_to_string(root.join(rel))
                .with_context(|| format!("failed to read committed {rel}"))?;
            assert_eq!(
                &on_disk, content,
                "committed {rel} drifted from gen-tables output"
            );
        }
        // Sanity: 234 numeric tables plus the two resolved `BLOCK_*` tables.
        assert_eq!(a.generated, 236, "generated-table count changed");
        assert_eq!(a.skipped.len(), SKIP_ALLOWLIST.len());
        Ok(())
    }

    #[test]
    fn every_allowlisted_skip_is_actually_symbolic() -> Result<()> {
        // Guards the allowlist against rot: each listed table must still parse as
        // symbolic in the committed attachment (else it should be generated, not
        // skipped).
        let root = workspace_root()?;
        let raw = std::fs::read_to_string(root.join(ATTACHMENT_REL))?;
        let decls = parse_decls(&raw)?;
        for (name, _) in SKIP_ALLOWLIST {
            let decl = decls
                .iter()
                .find(|d| d.name == *name)
                .with_context(|| format!("allowlisted table `{name}` not found in attachment"))?;
            assert!(
                !decl.numeric,
                "allowlisted table `{name}` is numeric and should be generated, not skipped"
            );
        }
        Ok(())
    }
}

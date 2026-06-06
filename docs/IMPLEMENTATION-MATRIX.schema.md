# Implementation matrix schema

`docs/IMPLEMENTATION-MATRIX.toml` is the **canonical** record of what is
implemented in `splot` and how far. This document defines its schema, the allowed
values, the status model, and the proof rules enforced by
`cargo xtask check-feature-status`.

GitHub Issues/Projects, README checklists, and `STATUS.md` are **not** canonical —
they are an execution queue and a snapshot, respectively. When they disagree with
the matrix, the matrix wins.

See also: [FEATURE-TRACKING.md](./FEATURE-TRACKING.md) (workflow and conventions),
[FEATURE-STATUS.md](./FEATURE-STATUS.md) (generated render).

## 1. The matrix is canonical

Every non-trivial feature has exactly one `[[feature]]` row keyed by a stable
Feature ID. Code comments (`TODO(spec: <id>)`), diagnostics, tests, OpenSpec
changes, GitHub issues, and PRs all reference that same ID. `cargo xtask
check-feature-status` fails the build if the matrix and the tree drift apart.

## 2. File-level fields

```toml
matrix_version = 1            # integer; the only supported version is 1
last_reviewed = "YYYY-MM-DD"  # date the matrix was last reviewed (string)
```

## 3. Feature row fields

Every `[[feature]]` row **must** define all of these top-level keys:

| Field | Type | Meaning |
|---|---|---|
| `id` | string | Stable Feature ID (see §6 and `FEATURE-TRACKING.md`). |
| `name` | string | Human-readable feature name. |
| `category` | string | One of the allowed categories (§4). |
| `kind` | string | One of the allowed kinds (§4). |
| `spec_sections` | array of strings | AV2 section ids (e.g. `"5.2.2"`, `"Annex B"`). May be empty for non-normative work. |
| `sources` | array of strings | URLs backing the row. May be empty. |
| `crate` | string | Owning crate (`splot-core`, `splot-validate`, `splot-encode`, `splot-cli`, `xtask`, `fuzz`, or `docs`). |
| `module` | string | Repo-relative path to the primary module/file. |
| `openspec_change` | string | OpenSpec change id. May name an existing folder under `openspec/changes/`, a *planned* change id that does not have a folder yet, or `""`. The checker does not require the folder to exist. |
| `tracking_issue` | string | GitHub issue reference, or `""`. |
| `owner` | string | Logical owner (`core`, `validator`, `encoder`, `conformance`, `cli`, `automation`, `docs`). |
| `risk` | string | One of the allowed risk values (§4). |
| `notes` | string | Short status note. |
| `replaces` | array of strings | *(optional)* IDs this row supersedes (§7). |

Two sub-tables are also required on every row:

```toml
[feature.status]   # the ten maturity stages of §5
[feature.proof]    # tests / commands / fixtures / diagnostics arrays (§5.3)
```

`tracking_issue` and `openspec_change` may be empty strings, but the keys must
exist. `replaces` is the only optional key.

## 4. Allowed enumerations

```text
category : normative | encoder | conformance | cli | docs | automation | infrastructure
kind     : bitstream-syntax | bitstream-semantics | validator-check | writer |
           encoder-api | encoder-tool | cli | conformance | docs | automation | infrastructure
risk     : low | medium | high | unknown
status   : todo | partial | done | blocked | not-applicable | experimental | pending
```

Use `pending` mainly for external proof such as AVM vectors or conformance corpora
that are not yet available.

## 5. Status model

`[feature.status]` has ten stages. **Never** use percentages.

| Stage | Meaning |
|---|---|
| `mapped` | Spec/design area is identified and linked. |
| `types` | Strong Rust types or API shape exist. |
| `parse` | Parser reads the syntax, if applicable. |
| `validate` | Validator/checks/diagnostics exist, if applicable. |
| `write` | Bitstream writer can emit the syntax, if applicable. |
| `encode` | Encoder can produce/use the feature, if applicable. |
| `decode_check` | Decoder/inspector/validator can check it enough for conformance work. |
| `tests` | Unit/property/fuzz/snapshot/conformance tests exist. |
| `avm_diff` | Differential/conformance proof against AVM or public vectors exists. |
| `perf` | Benchmarked/optimized where relevant. |

The `table` and `markdown` renders show a curated nine-stage projection (they omit
`perf`, which is uniformly low-signal today). `cargo xtask feature-status --format
json` always emits all ten stages.

### 5.1 Status definitions

- `todo` — not started.
- `partial` — started but incomplete (more cases, more constraints, or hardening
  remain).
- `done` — complete **and proven** (see §5.3).
- `blocked` — cannot proceed until a dependency or decision lands.
- `not-applicable` — the stage does not apply (e.g. `encode` for a validator-only
  feature, `write` for a parser-only feature).
- `experimental` — implemented but deliberately unstable / behind no guarantees.
- `pending` — waiting on something external (vectors, AVM, a corpus).

### 5.2 Good vs. bad status

Good (per-stage, honest):

```text
OBU header: parse done, validate done, write todo, encode not-applicable
```

Bad (percentages, vague):

```text
OBU header: 70% complete
```

### 5.3 Proof requirements ("done means proof exists")

A **code** stage (`parse`, `validate`, `write`, `encode`, `decode_check`, `tests`,
`avm_diff`, `perf`) may be `done` only when `[feature.proof]` records proof. When
any of those stages is `done`, the row's `[feature.proof]` must contain at least
one entry across:

```toml
[feature.proof]
tests       = ["crates/.../module.rs::tests"]   # test module/path
commands    = ["cargo test -p splot-core obu"]  # reproducible command
fixtures    = ["tests/fixtures/conformant.av2"] # fixture / vector
diagnostics = ["obu-header/global-xlayer-required"]  # diagnostic rule id
```

`mapped` and `types` are structural and do not by themselves require proof entries
(the `module` path serves as evidence). When in doubt, mark a stage `partial` or
`todo` and explain why in `notes`.

`cargo xtask check-feature-status` enforces:

1. The matrix file exists and `matrix_version` is supported.
2. Every row has all required fields.
3. Feature IDs are unique.
4. Feature IDs match the ID regex (§6).
5. Status values are from the allowed set.
6. `category` / `kind` / `risk` / `crate` / `owner` values are allowed.
7. The `module` path exists whenever any implementation stage is `partial`,
   `done`, or `experimental`.
8. A code stage marked `done` has proof (§5.3) — unless it is `not-applicable`.
9. Every `TODO(spec: <id>)` in Rust source references a known Feature ID.
10. Every feature-ID-shaped token (`AV2-…`, `ENC-…`, `CONF-…`, `CLI-…`,
    `XTASK-…`, `DOC-…`) in source/docs is a known Feature ID, a known ID with a
    `.SUFFIX` (diagnostic sub-rule), or in the checker's documented allowlist.
11. Validator diagnostic rule ids use a documented prefix or a known Feature ID.
12. `docs/FEATURE-STATUS.md`, if present, is up to date with the matrix.

## 6. Feature ID convention

```text
^[A-Z0-9]+(-[A-Z0-9.]+)+$
```

- Normative AV2: `AV2-<SECTION>-<SLUG>` (e.g. `AV2-5.2.2-OBU-HEADER`).
- Annexes: `AV2-<ANNEX>-<SLUG>` (e.g. `AV2-B-ANNEXB-OBU-ENVELOPE`).
- Encoder: `ENC-<SLUG>` (e.g. `ENC-BITSTREAM-WRITER`).
- Conformance: `CONF-<SLUG>` (e.g. `CONF-FUZZ-NO-PANIC`).
- Tooling/docs/CLI: `XTASK-<SLUG>`, `DOC-<SLUG>`, `CLI-<SLUG>`.

A merged ID is **stable**: do not rename it casually.

## 7. Adding, splitting, and retiring rows

**Add a row.** Copy [templates/FEATURE_MATRIX_ROW.toml](./templates/FEATURE_MATRIX_ROW.toml)
into the matrix, set every field, default unknown stages to `todo` (or
`not-applicable`), and run `cargo xtask check-feature-status`.

**Split a large row.** Some rows (e.g. `AV2-5.18-FRAME-HEADER`) are too large to
implement at once. Keep the umbrella row and add narrower child rows whose ids
extend it (for example a `…-REFS` or `…-TILE-INFO` child of
`AV2-5.18-FRAME-HEADER`). Point each child's `module` at the real code, and note
the relationship in `notes`. Do not mark the umbrella `done` until its children
are.

**Retire / replace a row.** Never silently rename an ID. Add the new row and list
the old ID in `replaces`:

```toml
[[feature]]
id = "AV2-SECTION-SLUG"   # the replacement id
replaces = ["OLD-ID"]
# ...
```

The checker treats an ID listed in `replaces` as historical. Document the
rationale in the relevant OpenSpec change and in `STATUS.md`.

## 8. Example row

```toml
[[feature]]
id = "AV2-SECTION-SLUG"
name = "Human-readable feature name"
category = "normative"
kind = "bitstream-syntax"
spec_sections = ["5.x", "6.x"]
sources = ["https://av2.aomedia.org/v1.0.0/index.html"]
crate = "splot-core"
module = "crates/splot-core/src/example.rs"
openspec_change = "change-id"
tracking_issue = ""
owner = "core"
risk = "unknown"
notes = "Short status note."

[feature.status]
mapped = "todo"
types = "todo"
parse = "todo"
validate = "todo"
write = "todo"
encode = "todo"
decode_check = "todo"
tests = "todo"
avm_diff = "pending"
perf = "not-applicable"

[feature.proof]
tests = []
commands = []
fixtures = []
diagnostics = []
```

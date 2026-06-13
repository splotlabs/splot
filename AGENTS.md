# AGENTS.md

Canonical instructions for humans and coding agents working in this repository.
`CLAUDE.md` and `.github/copilot-instructions.md` point here; keep this file the
single source of truth.

## 1. Project overview

`splot` is a Rust toolkit for the **AV2** video codec. It is **validator-first**:
the first useful milestone is a safe AV2 bitstream validator and inspector. It is a
solo-developer, source-available project optimized for maintainability, clear
boundaries, and automation. Toolchain: Rust **1.96.0**, edition **2024**, resolver
**3**.

## 1a. Encoder reference gate

Before changing `crates/splot-encode`, encoder-facing `splot-core` syntax/parsing code, or any
encoder research documentation, read:

1. `docs/references/ENCODER-RESEARCH-NOTES.md`
2. `docs/references/THIRD-PARTY-NOTICES.md`
3. `docs/references/RAV1E-SOURCE-MAP.md` when using Rust API, RDO, tiling, fuzzing, profiling, or
   safe data-structure ideas from rav1e
4. `docs/references/SVT-AV1-RESEARCH-MAPPING.md` when using production pipeline, mode-decision,
   motion-estimation, rate-control, filter-search, threading, or SIMD ideas from SVT-AV1

rav1e and SVT-AV1 are engineering inspiration only. Do not copy AV1 syntax, source code, tables,
constants, entropy CDFs, comments, or prose. AV2 behavior must be derived from the AV2 specification
and AVM. If a feature touches syntax, reconstruction, reference state, or layer behavior, find or create
its row in `docs/IMPLEMENTATION-MATRIX.toml` before implementation (see the Feature tracking
section); `docs/SPEC-MAPPING.md` holds the spec sources and rules, not per-feature status.

## 2. Repository map and dependency direction

```text
crates/splot-core      AV2 bitstream model + parsers (no other splot-* dependency)
crates/splot-recon     future reconstruction primitives (no other splot-* dependency)
crates/splot-decode    decoder diagnostic API; future driver (approved future -> splot-core, splot-recon)
crates/splot-validate  parser-driven conformance diagnostics  -> splot-core
crates/splot-encode    future encoder API (stub)              -> splot-core
crates/splot-cli       thin `splot` binary -> splot-core, splot-decode, splot-validate, splot-encode
xtask                  standalone automation (no splot-* dependency)
fuzz                   cargo-fuzz target (outside the workspace)
```

**Hard rule (one-way dependencies):**

- `splot-core` depends on no other `splot-*` crate.
- `splot-recon` depends on no other `splot-*` crate.
- `splot-decode` depends only on `splot-core` and `splot-recon` once runtime
  decode source code needs internal dependencies.
- `splot-validate` depends only on `splot-core`.
- `splot-encode` depends only on `splot-core`.
- `splot-cli` depends only on `splot-core`, `splot-decode`,
  `splot-validate`, and `splot-encode`.
- Nothing depends on `splot-cli`.
- Nothing depends on `splot-encode` except `splot-cli`.
- `xtask` is standalone.

This is enforced by `cargo xtask check-dependency-direction`. Crate
responsibilities, the error model, and the unsafe/SIMD policy are expanded in
[docs/ARCHITECTURE.md](./docs/ARCHITECTURE.md); the review checklist is
[docs/CODE_REVIEW.md](./docs/CODE_REVIEW.md).

## 3. Before editing

- Run `git status --short`.
- Inspect the files you are about to change.
- Preserve existing user work; never discard uncommitted changes.

## 4. Commands

```bash
cargo xtask ci          # the acceptance gate: fmt + clippy + build + test + doctests
                        # + typos + machete + deny + repo checks (external tools run-if-present)
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo build --workspace --all-targets --locked
cargo test --workspace --all-targets --locked
cargo test --doc --workspace --locked      # doctests (not covered by --all-targets)
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked   # rustdoc gate (warnings denied)
typos                                       # spell-check (config: _typos.toml)
cargo machete --with-metadata               # unused-dependency check
cargo deny check bans licenses sources      # offline supply-chain policy
openspec validate --all --no-interactive    # OpenSpec specs + active changes (optional tool, run-if-present)
cargo xtask audit                           # networked supply-chain advisories (cargo-deny)
cargo xtask coverage                        # local HTML coverage report (cargo-llvm-cov)
cargo xtask fuzz [--time <secs>]            # local fuzz smoke over every target (nightly + cargo-fuzz), default 30s each
cargo xtask check-conventional-commits      # validates the current HEAD commit subject
cargo +nightly fuzz run parse_obu   # full local fuzz run of one target (nightly-only; `cargo install cargo-fuzz --locked`).
                                    # Targets: parse_obu, validate_bytes, parse_ivf, parse_bitstream (`cargo +nightly fuzz list`).
                                    # CI also runs a blocking per-target smoke (~45s each) over every target on every PR.
```

The external-tool checks (`typos`, `cargo-machete`, `cargo-deny`, `cargo-llvm-cov`,
`cargo-fuzz`) are **external binaries, not cargo dependencies**. CI installs them so
they always gate; locally `cargo xtask ci` runs each one if present and otherwise
prints an install hint and continues.

## 5. Coding conventions

- **Library-first, thin CLI:** `splot-cli` only parses args, sets up logging,
  reads/writes files, and calls library APIs. All logic lives in libraries.
- **Errors:** libraries use typed errors with `thiserror`; `anyhow` is allowed only
  in `splot-cli` and `xtask`.
- **No runtime panics in libraries:** no `unwrap`, `expect`, `panic!`, `todo!`, or
  `unimplemented!` reachable in library code. Stubs return
  `Error::Unimplemented { feature }` or a structured `Diagnostic`.
- **Strong types:** use newtypes/enums (`ObuType`, `TemporalLayerId`, `ByteOffset`,
  …) at public boundaries, not bare integers.
- **Public docs:** every public item has a doc comment; every crate has `//!` docs.
- **SPDX headers:** every `.rs` file starts with the two-line SPDX header (enforced
  by `cargo xtask check-license-headers`):

  ```rust
  // SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
  // SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
  ```

- **Source file size budget:** Rust source files should stay at or below **1000
  physical lines**. `cargo xtask check-source-lines` prints advisory warnings
  above that soft limit and fails above the **2500-line hard cap** unless the file
  has a documented temporary allowance in `xtask/src/source_lines.rs`. Split large
  files by responsibility before adding new code; do not grow an allowlisted file
  past its recorded cap without deliberately updating the allowance and rationale.
- **Unsafe:** forbidden in the workspace (`unsafe_code = "forbid"`). Future SIMD/FFI
  must live behind narrowly-scoped, documented, tested modules.

## 5.1 Commit messages

Use Conventional Commits for every commit subject and pull request title:

```text
<type>[optional scope][!]: <description>
```

Allowed types are `build`, `chore`, `ci`, `docs`, `feat`, `fix`, `perf`,
`refactor`, `revert`, `style`, and `test`. CI enforces this with
`cargo xtask check-conventional-title` and `cargo xtask check-conventional-commits`
(tracked as `XTASK-CONVENTIONAL-COMMITS`).
Use squash or rebase merges only; generated GitHub merge commits are not
Conventional Commits subjects.
One exemption: a git-generated sync merge on a feature branch (a
multi-parent commit whose subject starts with `Merge `) is skipped by
`check-conventional-commits` — never force-push a pushed branch to sync it
with `main`; merge `main` in instead. The squash merge to `main` drops the
merge commit, so it never reaches the default branch.

## 6. AV2 spec honesty

- **Never invent** AV2 syntax, constants, table contents, or semantics. If a detail
  is not modeled, add `// TODO(spec: <FEATURE-ID>): <topic>` referencing the matrix
  id (see the Feature tracking section); `cargo xtask check-feature-status` rejects
  a bare or unknown spec TODO.
- Cite the spec section in the doc comment or code comment for each syntax element.
- The AV2 OBU header is from **§ 5.2.2**, not AV1. There is no AV1 OBU type table.
- Treat **AVM** (<https://github.com/AOMediaCodec/avm>) as the differential-testing
  oracle.
- **The committed spec mirror is the single source of truth.** A faithful,
  versioned copy of the spec lives at
  [`docs/spec/av2/1.0.0/`](./docs/spec/av2/1.0.0/) (generated by
  `scripts/spec/regenerate-av2-spec.sh`). Ground every AV2 syntax/semantics claim
  in it — do not rely on memory or paraphrase. Find any section via
  [`docs/spec/av2/1.0.0/index.md`](./docs/spec/av2/1.0.0/index.md) and cite it as
  `§ N.M` plus the mirror path, e.g.
  `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-16`. The PDF remains normative;
  the mirror is a byte-faithful navigation/citation aid (verbatim spec text inside
  fences). Treat the mirror as read-only third-party material — never hand-edit it
  (the `cargo xtask check-spec-mirror` gate fails on drift); regenerate instead.
- Normative reference: AV2 v1.0.0, <https://av2.aomedia.org/v1.0.0/index.html>.
  See [docs/SPEC-MAPPING.md](./docs/SPEC-MAPPING.md).

## 7. Validator principle

Diagnostics are the product. Every finding is structured data: stable `rule_id`,
`severity`, optional `spec_section`, optional byte/bit offset, and a human-readable
`message`. The validator never "just logs".

## 8. Testing expectations

In priority order: parser unit tests (LEB128, OBU header, Annex B) → property/fuzz
tests (parsers never panic) → `inspect` snapshots → conformance vectors → AVM
differential testing. See [docs/TESTING.md](./docs/TESTING.md). Add positive,
negative, and EOF cases for every parser change.

## 8a. Audit protocols

Use repo-local audit skills instead of expanding this file with full audit
procedures:

- Documentation/guidance audits: `.codex/skills/splot-doc-audit/SKILL.md` or
  `.claude/skills/splot-doc-audit/SKILL.md` (`DOC-AUDIT-PROTOCOLS`).
- Heavy AV2 conformance audits: `.codex/skills/splot-av2-conformance-audit/SKILL.md`
  or `.claude/skills/splot-av2-conformance-audit/SKILL.md` (`XTASK-AUDIT-SCOPE`,
  `DOC-AUDIT-PROTOCOLS`).

Heavy audits must start from `cargo xtask audit-scope --format json` so changed
files, force-wide triggers, future workspace members, and audit ledger state are
selected deterministically. Do not rely on `.agents/skills/` as the only project
skill location; mirror or generate into the agent-specific project skill paths
above.

## Feature tracking

Every non-trivial change must use a stable Feature ID.

The canonical status file is:

```text
docs/IMPLEMENTATION-MATRIX.toml
```

Before implementing a feature:

1. Find or create a matrix row.
2. Find or create an OpenSpec change under `openspec/changes/` unless the work is trivial.
3. Use the Feature ID in code comments, diagnostics, tests, and PR text.
4. Add `TODO(spec: FEATURE-ID): ...` for any intentionally unmapped AV2 detail.

Before finishing:

```bash
cargo xtask feature-status
cargo xtask check-feature-status
cargo xtask ci
```

Do not mark a stage `done` unless tests/proof are recorded in the matrix. The
schema, status model, and ID convention live in
[docs/FEATURE-TRACKING.md](./docs/FEATURE-TRACKING.md) and
[docs/IMPLEMENTATION-MATRIX.schema.md](./docs/IMPLEMENTATION-MATRIX.schema.md).

## 9. Licensing

`splot` project code, documentation, tests, fixtures, and automation are
**PolyForm Noncommercial 1.0.0**. Commercial use requires a separate license.

Narrow exception (assistant integrations): OpenSpec-generated assistant
integration files under `.claude/commands/opsx/`, `.claude/skills/openspec-*`,
`.codex/skills/openspec-*`, `.github/prompts/opsx-*`, and
`.github/skills/openspec-*` are **MIT** as generated by OpenSpec. Keep this
material isolated to those paths, preserve generated license metadata when
present, and do not copy it into `splot` source or original docs.

Narrow exception (AV2 specification mirror): the committed spec mirror under
`docs/spec/av2/<version>/` is verbatim **AOMedia copyright** material (© Alliance
for Open Media), **not** PolyForm. It is a maintainer-approved, attributed,
quarantined copy recorded in
[`docs/references/THIRD-PARTY-NOTICES.md`](./docs/references/THIRD-PARTY-NOTICES.md).
Keep it isolated to that path, do not add the PolyForm SPDX header to its files,
and do not copy its text into `splot` source or original docs (cite it instead).

Do not introduce any other mixed-license material without explicit maintainer
approval and an update to `docs/references/THIRD-PARTY-NOTICES.md`.

## 10. When to ask the human

- Algorithmic encoder choices.
- Ambiguous spec interpretation.
- Adding a new third-party dependency.
- Any change to the crate dependency graph.
- Any legal/licensing change.

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
and AVM. If a feature touches syntax, reconstruction, reference state, or layer behavior, update
`docs/SPEC-MAPPING.md` before implementation.

## 2. Repository map and dependency direction

```text
crates/splot-core      AV2 bitstream model + parsers (no other splot-* dependency)
crates/splot-validate  parser-driven conformance diagnostics  -> splot-core
crates/splot-encode    future encoder API (stub)              -> splot-core
crates/splot-cli       thin `splot` binary -> splot-core, splot-validate, splot-encode
xtask                  standalone automation (no splot-* dependency)
fuzz                   cargo-fuzz target (outside the workspace)
```

**Hard rule (one-way dependencies):**

- `splot-core` depends on no other `splot-*` crate.
- Nothing depends on `splot-cli`.
- Nothing depends on `splot-encode` except `splot-cli`.
- `xtask` is standalone.

This is enforced by `cargo xtask check-dependency-direction`.

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
typos                                       # spell-check (config: _typos.toml)
cargo machete --with-metadata               # unused-dependency check
cargo deny check bans licenses sources      # offline supply-chain policy
cargo xtask audit                           # networked supply-chain advisories (cargo-deny)
cargo xtask coverage                        # local HTML coverage report (cargo-llvm-cov)
cargo xtask fuzz [--time <secs>]            # local fuzz smoke (nightly + cargo-fuzz), default 30s
cargo xtask check-conventional-commits      # validates the current HEAD commit subject
cargo +nightly fuzz run parse_obu   # full local fuzz run (nightly-only; `cargo install cargo-fuzz --locked`).
                                    # CI also runs a blocking 60s parse_obu smoke on every PR.
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

## 6. AV2 spec honesty

- **Never invent** AV2 syntax, constants, table contents, or semantics. If a detail
  is not modeled, add `// TODO(spec: <FEATURE-ID>): <topic>` referencing the matrix
  id (see the Feature tracking section); `cargo xtask check-feature-status` rejects
  a bare or unknown spec TODO.
- Cite the spec section in the doc comment or code comment for each syntax element.
- The AV2 OBU header is from **§ 5.2.2**, not AV1. There is no AV1 OBU type table.
- Treat **AVM** (<https://github.com/AOMediaCodec/avm>) as the differential-testing
  oracle.
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

Narrow exception: OpenSpec-generated assistant integration files under
`.claude/commands/opsx/`, `.claude/skills/openspec-*`,
`.codex/skills/openspec-*`, `.github/prompts/opsx-*`, and
`.github/skills/openspec-*` are **MIT** as generated by OpenSpec. Keep this
material isolated to those paths, preserve generated license metadata when
present, and do not copy it into `splot` source or original docs.

Do not introduce any other mixed-license material without explicit maintainer
approval and an update to `docs/references/THIRD-PARTY-NOTICES.md`.

## 10. When to ask the human

- Algorithmic encoder choices.
- Ambiguous spec interpretation.
- Adding a new third-party dependency.
- Any change to the crate dependency graph.
- Any legal/licensing change.

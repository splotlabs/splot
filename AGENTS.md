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
cargo xtask ci          # fmt + clippy + build + test + repo checks (the acceptance gate)
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo build --workspace --all-targets --locked
cargo test --workspace --all-targets --locked
cargo fuzz run parse_obu   # requires `cargo install cargo-fuzz --locked`; not part of CI
```

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

## 6. AV2 spec honesty

- **Never invent** AV2 syntax, constants, table contents, or semantics. If a detail
  is not modeled, add `// TODO(spec): <section/topic>`.
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

## 9. Licensing

The entire repository is **PolyForm Noncommercial 1.0.0**. All components are
noncommercial; commercial use requires a separate license. Do not mix licenses.

## 10. When to ask the human

- Algorithmic encoder choices.
- Ambiguous spec interpretation.
- Adding a new third-party dependency.
- Any change to the crate dependency graph.
- Any legal/licensing change.

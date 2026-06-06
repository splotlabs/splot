# STATUS

Scaffold of the `splot` AV2 toolkit. Validator-first: the Annex B envelope parser,
AV2 OBU header parser, and header-level conformance validator are implemented; the
decoder/encoder are reserved API shapes.

Toolchain: Rust 1.96.0, edition 2024, resolver 3. Generated 2026-06-06.

## Implemented

- **`splot-core`**
  - `error` — typed `Error` (`thiserror`) + `Result`.
  - `span` — `ByteOffset`, `BitOffset`, `ByteSpan` newtypes (serde-serializable).
  - `types` — `ObuType` (AV2 Table 6.1), `TemporalLayerId`/`EmbeddedLayerId`/
    `ExtendedLayerId`, `GLOBAL_XLAYER_ID`, and § 5.2.1 / § 6.2.2 helper predicates,
    all verified against the AV2 v1.0.0 PDF.
  - `bitio` — MSB-first `BitReader` for `f(n)`; `RangeDecoder`/`RangeEncoder` stubs
    returning `Error::Unimplemented`.
  - `leb128` — `read_leb128` (§ 4.11.6): ≤ 8 bytes, value ≤ `u32::MAX`, byte-7 MSB
    rule, non-minimal allowed.
  - `obu` — `read_obu_header` (§ 5.2.2) with the no-extension xlayer inference.
  - `annexb` — `parse_annex_b_obus` (Annex B § B.2): LEB128-prefixed OBUs, payload
    slicing, panic-free on malformed input.
  - `headers`, `tables` — documented placeholders (`TODO(spec: <FEATURE-ID>)`).
- **`splot-validate`**
  - `diagnostic` — `Severity`, `Diagnostic` (rule id / section / severity / byte /
    bit / message), `ValidationReport` with `is_conformant`, `errors`, `warnings`,
    `Display`, and serde `Serialize`.
  - `validator` — `Validator::validate_bytes` (never returns `Err`); parse failures
    become error diagnostics.
  - `checks` — `Check` trait + registry: seven checks (five § 6.2.2 header
    constraints, one informational reserved-type check, and a reserved-OBU
    all-zero-payload error per § 5.3 / § 6.2.3), all spec-cited.
- **`splot-cli`** (`splot`) — `validate` and `inspect` are functional;
  `encode`/`decode` print clear "not yet implemented" messages and exit non-zero.
  Global `-v/--verbose`/`--quiet`, `tracing` logging to stderr, `--json` output,
  documented exit codes (0/1/2), and a project-wide PolyForm-Noncommercial notice in
  `--help`.
- **`xtask`** — `ci`, `check-license-headers`, `check-dependency-direction`
  implemented; `gen-tables`, `fetch-vectors`, `conformance` are explanatory stubs.
- **`fuzz`** — `parse_obu` libFuzzer target over all three parsers (outside the
  workspace).

## Stubbed / not implemented

- Entropy (range) coder, decoder, encoder — all return `Error::Unimplemented`.
- Sequence/frame header syntax, spec tables (`TODO(spec: <FEATURE-ID>)` / codegen).
- OBU ordering and sequence-header-activated conformance checks.
- `insta` snapshot tests, conformance vectors, AVM differential testing.

## Feature tracking framework (added 2026-06-06)

A canonical, machine-readable AV2 implementation matrix plus `xtask` enforcement.
The matrix is the source of truth; OpenSpec records intent; GitHub is the execution
queue; tests are proof.

**Files added:**

- `docs/IMPLEMENTATION-MATRIX.toml` (canonical, 27 rows), `*.schema.md`,
  `docs/FEATURE-STATUS.md` (generated), `docs/FEATURE-TRACKING.md`,
  `docs/ENCODER-ROADMAP.md`, `docs/CONFORMANCE.md`,
  `docs/DECISIONS/0001-feature-tracking.md`, `docs/templates/FEATURE_MATRIX_ROW.toml`.
- `openspec/` (README, `specs/{bitstream,validator,encoder-api,encoder-tools,conformance}/spec.md`,
  `changes/` with a `.template/` and six initial change folders).
- `xtask/src/feature_status.rs` (matrix model, render, and the drift checker).
- `.github/ISSUE_TEMPLATE/{av2-feature,conformance,bug}.yml`,
  `.github/PULL_REQUEST_TEMPLATE.md`.

**xtask subcommands added:** `feature-status` (`--format table|json|markdown`,
`--category`, `--kind`, `--output`), `check-feature-status`, `spec-coverage`
(`--format text|markdown`). `check-feature-status` is wired into `cargo xtask ci`
and `.github/workflows/ci.yml`.

**Matrix rows seeded and adjusted after inspecting the code:** the 23 seeded rows
plus four added to reflect shipped reality — `AV2-5.2.1-OBU-TYPE`,
`AV2-9-ADDITIONAL-TABLES`, `CLI-VALIDATE`, `CLI-INSPECT`. Statuses were upgraded
from the seed where the code is real and proven: LEB128, OBU header, Annex B, and
the OBU type table are `parse`/`tests` `done`; the OBU-header §6.2.2 checks and the
reserved-OBU checks are `validate`/`tests` `done` with diagnostic + test proof; the
no-panic proptest makes `CONF-FUZZ-NO-PANIC` `tests` `done`. Writer/encoder/AVM
stages remain `todo`/`pending`. `cargo xtask spec-coverage` flags
`CONF-INSPECT-SNAPSHOTS` as the one row that has progressed but records no proof
(its snapshot tests do not exist yet).

**Bare `TODO(spec)` markers** in `headers.rs`, `tables.rs`, `config.rs`,
`context.rs`, and `checks/mod.rs` were migrated to the
`TODO(spec: <FEATURE-ID>): …` form so they reference matrix ids; the checker
rejects bare or unknown spec TODOs.

## Dependencies added

| Crate | Where | Purpose |
|-------|-------|---------|
| `serde` 1 (derive) | xtask (added) | typed matrix model |
| `serde_json` 1 | xtask (added) | `feature-status --format json` |
| `thiserror` 2 | core, (re-used) | typed library errors |
| `serde` 1 (derive) | core, validate, cli | serialize types/reports |
| `serde_json` 1 | cli | `--json` output |
| `clap` 4 (derive) | cli, xtask | argument parsing |
| `anyhow` 1 | cli, xtask | application errors |
| `tracing` 0.1 + `tracing-subscriber` 0.3 (env-filter) | cli | logging |
| `toml` 1 | xtask | parse manifests for the dependency-direction check |
| `proptest` 1 | core (dev) | parser "never panics" property test |
| `libfuzzer-sys` 0.4 | fuzz | fuzz harness |

## Deviations from the scaffolding prompt

1. **clippy lint priority.** `[workspace.lints.clippy] all` is written as
   `{ level = "warn", priority = -1 }` instead of `all = "warn"`. Rust 1.96 clippy's
   `lint_groups_priority` check fails the plain form under `-D warnings`.
2. **`CONTACT_EMAIL` / holder.** Commercial-licensing contact is `bartekplus@gmail.com`
   and the SPDX copyright holder is `Bartosz Tomczyk`. The PolyForm `Required Notice`
   example references Splot Labs.
3. **`xtask ci`** calls the license-header and dependency-direction checks as
   in-process functions rather than spawning `cargo xtask …` subprocesses (same
   effect, no recompile). CI (`.github/workflows/ci.yml`) still invokes them as
   separate `cargo xtask` steps, as specified.
4. **`toml` dependency** added to `xtask` (not in the prompt's suggested list) to
   parse member manifests robustly for the dependency-direction check.
5. **License SPDX** `PolyForm-Noncommercial-1.0.0` is accepted by Cargo; the
   `license-file` fallback was not needed.
6. Local toolchain is Homebrew Rust 1.96.0 (no `rustup`); `rust-toolchain.toml`
   still pins `1.96.0` for `rustup`/CI users.

Feature-tracking framework (2026-06-06):

7. **Four matrix rows beyond the prompt's seed.** Added `AV2-5.2.1-OBU-TYPE`,
   `AV2-9-ADDITIONAL-TABLES`, `CLI-VALIDATE`, and `CLI-INSPECT` so the matrix
   reflects shipped reality, and upgraded seeded statuses where the code is real
   and proven (the prompt explicitly asks to adjust statuses after inspection).
8. **Checker allowlists are documented in `xtask/src/feature_status.rs`.** Per the
   prompt's rule 11, feature-ID-shaped tokens must resolve to a matrix id, a
   `<known-id>.SUFFIX` diagnostic sub-rule, or an allowlisted placeholder
   (`AV2-SECTION-SLUG`). Validator diagnostic ids use a documented kebab/slash
   prefix allowlist (`obu-header/`, `obu-reserved/`, `bitstream/`).
9. **Rule 13 is strict.** `check-feature-status` regenerates the markdown render and
   fails if `docs/FEATURE-STATUS.md` is stale (no warning-only mode).
10. **`CLAUDE.md` is a symlink to `AGENTS.md`**, so only `AGENTS.md` was edited; the
    new "Feature tracking" section is unnumbered to keep the existing "§ 9
    Licensing" cross-reference stable.
11. **Issue forms use `description:`** (the valid GitHub issue-form key) rather than
    the prompt's `about:` (a legacy markdown-template key).
12. The TODO scanner runs over `.rs` files; the feature-ID token scanner runs over
    `.rs`/`.md`/`.toml`/`.yml`/`.yaml` (skipping `target`, `.git`, and the fuzz
    `corpus`).
13. **Diagnostic rule-id convention.** The prompt suggests feature-corresponding
    diagnostics use the Feature ID as the base rule id (with optional `.SUFFIX`).
    The existing validator instead uses a stable kebab/slash namespace
    (`obu-header/`, `obu-reserved/`, `bitstream/`), which predates this framework.
    To avoid churning stable, user-facing rule ids, `check-feature-status` accepts
    **both**: a documented kebab/slash prefix *or* a known Feature ID (optionally
    with a `.SUFFIX`). New feature-specific diagnostics may use either form; new
    kebab prefixes must be added to the allowlist in `xtask/src/feature_status.rs`
    and documented in `docs/FEATURE-TRACKING.md`.

## Acceptance command results (current head)

All run from the repo root:

```text
cargo fmt --all -- --check                                                      # ok (no diff)
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings   # ok, 0 warnings
cargo build --workspace --all-targets --locked                                  # ok
cargo test --workspace --all-targets --locked                                   # ok: 73 passed, 0 failed
cargo xtask feature-status                                                       # ok (renders 27-row table)
cargo xtask feature-status --format markdown --output docs/FEATURE-STATUS.md    # ok, worktree stays clean
cargo xtask check-feature-status                                                # ok (27 features)
cargo xtask spec-coverage                                                       # ok
cargo xtask ci                                                                  # ok: all checks passed
git diff --check                                                                # ok (no whitespace errors)
openspec validate --all --no-interactive                                        # ok: 11/11 (CLI present)
```

Test breakdown: `splot-core` 37, `splot-encode` 2, `splot-validate` 15,
`splot-cli` 7 (CLI integration tests over `tests/fixtures/`), `xtask` 12.

Also verified:

```text
cargo run -p splot-cli -- --help            # shows subcommands, aliases, PolyForm notice
cargo run -p splot-cli -- inspect --help    # shows inspect args
cargo run -p xtask -- --help                # shows xtask subcommands (incl. feature-status, check-feature-status, spec-coverage)
splot validate good.av2                     # conformant, exit 0
splot validate bad.av2                      # 1 error (§6.2.2), exit 1
splot inspect good.av2 --headers            # lists 2 OBUs with inferred xlayer
```

> OpenSpec CLI: `openspec validate --all --no-interactive` was run and passed
> (11/11). It is optional — CI runs it only when the CLI is installed.

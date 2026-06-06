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
  - `headers`, `tables` — documented placeholders (`TODO(spec)`).
- **`splot-validate`**
  - `diagnostic` — `Severity`, `Diagnostic` (rule id / section / severity / byte /
    bit / message), `ValidationReport` with `is_conformant`, `errors`, `warnings`,
    `Display`, and serde `Serialize`.
  - `validator` — `Validator::validate_bytes` (never returns `Err`); parse failures
    become error diagnostics.
  - `checks` — `Check` trait + registry: six checks (five § 6.2.2 header
    constraints + one informational reserved-type check), all spec-cited.
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
- Sequence/frame header syntax, spec tables (`TODO(spec)` / codegen).
- OBU ordering and sequence-header-activated conformance checks.
- `insta` snapshot tests, conformance vectors, AVM differential testing.

## Dependencies added

| Crate | Where | Purpose |
|-------|-------|---------|
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

## Acceptance command results (2026-06-06)

All run from the repo root:

```text
cargo fmt --all -- --check                                              # ok (no diff)
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings  # ok, 0 warnings
cargo build --workspace --all-targets --locked                          # ok
cargo test --workspace --all-targets --locked                           # ok: 50 passed, 0 failed
cargo xtask ci                                                          # ok: all checks passed
```

Test breakdown: `splot-core` 34, `splot-encode` 2, `splot-validate` 8,
`splot-cli` 6 (CLI integration tests over `tests/fixtures/`), `xtask` 0.

Also verified:

```text
cargo run -p splot-cli -- --help            # shows subcommands, aliases, PolyForm notice
cargo run -p splot-cli -- inspect --help    # shows inspect args
cargo run -p xtask -- --help                # shows xtask subcommands
splot validate good.av2                     # conformant, exit 0
splot validate bad.av2                      # 1 error (§6.2.2), exit 1
splot inspect good.av2 --headers            # lists 2 OBUs with inferred xlayer
```

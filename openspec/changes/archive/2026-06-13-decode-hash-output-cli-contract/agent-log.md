# Agent Log: decode-hash-output-cli-contract

## Orchestrator Plan

Objective: add a narrow `splot decode` hash-output CLI contract and parse
surface so future hash-first decode success does not require Y4M output.

Reason for selecting this slice: dependency-graph changes for `splot-recon` and
`splot-decode` still need explicit maintainer approval, but the mission can
advance by removing the documented CLI-shape blocker for hash-first output.

Feature ID: `CLI-DECODE-HASH-OUTPUT`.

## Planning Agents

### @architect / Euclid

- Agent ID: `019ec17b-58cb-7852-9899-e58771c75a17`
- Objective: assess whether `decode-hash-output-cli-contract` is a valid
  PR-sized slice while dependency graph changes are not approved.
- Output: confirmed the slice is valid if it stays a CLI contract and parse
  surface, preserves current `decode/unsupported-feature` behavior, performs no
  input reads or output writes, adds no crates or dependencies, and keeps
  AVM/dav2d out of repo and CI. Recommended Feature ID
  `CLI-DECODE-HASH-OUTPUT`, docs/matrix updates, and focused CLI tests.

### @spec-reader / Euler

- Agent ID: `019ec17b-5b88-7751-b36e-386f91f0e089`
- Objective: read committed AV2 v1.0.0 spec mirror sections relevant to decoded
  output and hash semantics.
- Output: confirmed hash-output wording must refer to decoded AV2 output frames
  from § 7.1 / § 7.21 and repository-owned `splot-dfh-sha256-v1`, not parser
  facts, OBU bytes, metadata payload bytes, AV2 `hash_type = 0` MD5, reserved
  AV2 hash types, or current runtime hash support.

### @api-designer / Singer

- Agent ID: `019ec17b-5e33-7d32-9572-0f476e06af93`
- Objective: recommend the CLI argument shape and compatibility rules.
- Output: recommended an explicit `--output-format <y4m|hash>` enum without a
  clap-level default, preserving `decode <input> -o <output>` as implicit Y4M,
  allowing `--output-format hash` without `-o`, keeping `decode <input>` a clap
  usage error, and leaving every valid parse unsupported at runtime.

### @reference-oracle / Lovelace

- Agent ID: `019ec17b-61b5-73b2-97fb-254d1d4aea3f`
- Objective: determine whether local AVM/dav2d evidence is needed for this
  CLI contract slice.
- Output: confirmed AVM/dav2d execution is not applicable because this change
  decodes no bytes and computes no hashes. Recommended relying on self-contained
  CLI/OpenSpec tests and recording no local reference evidence.

## Local Reference Boundary

No AVM or dav2d command was run for this change. No AVM/dav2d source, snippets,
binaries, submodules, dependencies, build probes, wrappers, CI jobs, required
scripts, `xtask` commands, or mandatory tests are added by this change.

Existing archived AVM/dav2d raw MD5 agreement remains scoped to deterministic
hash planning only and is not proof of `splot` hash output, CLI behavior, parse
surface behavior, or runtime decode support.

## Implementation Notes

- Added `DecodeOutputFormat` with `--output-format <y4m|hash>` to
  `crates/splot-cli/src/commands/decode.rs`.
- Preserved `splot decode <input> -o <output>` as the implicit Y4M-compatible
  form and added explicit `--output-format y4m`.
- Allowed `--output-format hash` without an output path so a future hash-first
  success path can avoid requiring Y4M output.
- Kept every valid parse on the current unsupported runtime path:
  `decode/unsupported-feature`, exit code `1`, no input reads, no output writes,
  no hash computation, and no external decoder invocation.
- Added CLI tests for hash output selection, explicit Y4M selection, usage
  errors when no output selection exists, and unchanged diagnostic rendering.
- Updated `docs/DECODER-ROADMAP.md`, `docs/DECODER-SUPPORT-MATRIX.toml`, and
  `docs/IMPLEMENTATION-MATRIX.toml`, then regenerated
  `docs/DECODER-SUPPORT-STATUS.md`, `docs/FEATURE-STATUS.md`, and
  `docs/SPEC-COVERAGE.md`.

## Verification

- `openspec validate decode-hash-output-cli-contract --strict` passed.
- `cargo test -p splot-cli --test decode_cli --locked` passed with 11 decode
  CLI tests.
- `cargo test -p splot-cli --test cli --locked` passed with 34 remaining CLI
  integration tests after the decode tests moved to `decode_cli.rs`.
- `openspec validate --all --no-interactive` passed.
- `cargo xtask check-decoder-support` passed with 17 decoder support rows.
- `cargo xtask check-feature-status` passed with 146 tracked features.
- `cargo xtask check-diagnostic-registry` passed.
- `git diff --check` passed.
- `cargo xtask ci` passed. The earlier `crates/splot-cli/tests/cli.rs` source
  line advisory was removed by splitting decode tests into
  `crates/splot-cli/tests/decode_cli.rs`; remaining advisories are pre-existing.

## Review

### @reviewer / Ampere

- Agent ID: `019ec18b-d19b-7ef3-a555-be9eee123e4f`
- Initial findings:
  - `design.md` contained an accidental patch marker and duplicate spec delta.
  - `CLI-DECODE-HASH-OUTPUT` incorrectly listed AV2 decoded-frame-hash metadata
    sections as coverage instead of contrast citations.
  - Hash mode did not directly test a missing input path.
- Resolution:
  - Removed the duplicate patch tail from `design.md`.
  - Narrowed `CLI-DECODE-HASH-OUTPUT` coverage to § 7.1 / § 7.21 and
    regenerated status docs.
  - Added `decode_hash_output_format_missing_input_does_not_touch_files`.
- Re-review: no remaining findings.

### @security-reviewer / Halley

- Agent ID: `019ec18b-d4b1-78d3-a09a-4e474abce419`
- Initial finding: hash mode did not prove the no-read/no-default-output
  boundary for missing input and no explicit output path.
- Resolution: strengthened
  `decode_hash_output_format_missing_input_does_not_touch_files` to run from an
  empty temporary working directory and assert both the missing input and the
  directory remain untouched. Added invalid `--output-format raw` usage-error
  coverage.
- Re-review: no remaining findings.

### @spec-conformance-reviewer / Nietzsche

- Agent ID: `019ec18b-d74a-7721-80a1-008031f0ab2f`
- Initial findings:
  - `CLI-DECODE-HASH-OUTPUT` coverage should not claim § 5.17.12 / § 6.16.13
    metadata decoded-frame-hash implementation coverage.
  - `design.md` contained the accidental patch marker and duplicated delta.
  - Invalid `--output-format` values lacked a usage-error test.
- Resolution:
  - Kept § 5.17.12 / § 6.16.13 as contrast wording only; coverage now lists
    only § 7.1 / § 7.21 for this CLI feature.
  - Cleaned `design.md`.
  - Added `decode_invalid_output_format_is_usage_error`.
- Re-review: no remaining findings.

### @encoder-impact-reviewer / Mill

- Agent ID: `019ec18b-da14-7091-b094-df7200fb6cb2`
- Findings: none. Signed off that the change stays CLI-only, helps future
  encoder roundtrip evidence by avoiding forced Y4M output, and preserves
  dependency boundaries.

## Post-Review Verification

- `cargo test -p splot-cli --test decode_cli --locked` passed with 11 tests.
- `openspec validate decode-hash-output-cli-contract --strict` passed.
- `openspec validate --all --no-interactive` passed.
- `cargo xtask check-decoder-support` passed with 17 decoder support rows.
- `cargo xtask check-feature-status` passed with 146 tracked features.
- `git diff --check` passed.
- `cargo xtask ci` passed.

## Archive

- `openspec archive decode-hash-output-cli-contract --yes` archived the change
  as `2026-06-13-decode-hash-output-cli-contract` and applied one requirement
  to `openspec/specs/decoder-support/spec.md`.
- Removed the archive tool's trailing blank line at EOF in the updated main
  decoder-support spec.
- Post-archive verification passed:
  - `cargo test -p splot-cli --test decode_cli --locked`
  - `openspec validate --all --no-interactive`
  - `cargo xtask check-decoder-support`
  - `cargo xtask check-feature-status`
  - `cargo xtask check-diagnostic-registry`
  - `git diff --check`
  - `cargo xtask ci`

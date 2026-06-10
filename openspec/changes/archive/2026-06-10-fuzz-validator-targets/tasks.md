# Tasks: Fuzz every untrusted-input surface

## 1. Pre-implementation bookkeeping

- [x] 1.1 docs/IMPLEMENTATION-MATRIX.toml: set `openspec_change =
  "fuzz-validator-targets"` on `CONF-FUZZ-NO-PANIC`; extend the notes with the
  target-to-surface mapping (leb128/OBU-header/Annex-B → `parse_obu`; IVF →
  `parse_ivf`; container auto-detect + dispatch → `parse_bitstream`; all OBU
  payload parsers + validator checks → `validate_bytes`).
- [x] 1.2 Register the change in `openspec/changes/README.md` (Active changes
  table).

## 2. Fuzz targets

- [x] 2.1 `fuzz/Cargo.toml`: add the `splot-validate` path dependency and
  `[[bin]]` entries for `validate_bytes`, `parse_ivf`, `parse_bitstream`
  (same shape as `parse_obu`: `test = false`, `doc = false`,
  `bench = false`).
- [x] 2.2 `fuzz/fuzz_targets/validate_bytes.rs`: derive `ValidatorOptions`
  deterministically from the first input byte (covering option-gated
  branches), validate the remaining bytes via
  `Validator::validate_bytes_with_options`. SPDX header + the standard
  nightly/run comment block.
- [x] 2.3 `fuzz/fuzz_targets/parse_ivf.rs`: `is_ivf`, `parse_ivf_header`,
  `parse_ivf_partial` over the raw input.
- [x] 2.4 `fuzz/fuzz_targets/parse_bitstream.rs`:
  `parse_bitstream_partial` over the raw input (container auto-detect +
  payload dispatch).

## 3. Smoke jobs enumerate targets

- [x] 3.1 `.github/workflows/ci.yml` fuzz-smoke job: enumerate targets via
  `cargo +nightly fuzz list` and run each for a per-target slice (keep the
  existing `-timeout`/`-rss_limit_mb` guards and the gnu-target rationale);
  seed every target's corpus from `tests/fixtures/*.av2`; update the job
  comment and the crash-artifact upload to cover all targets.
- [x] 3.2 `xtask/src/main.rs::run_fuzz`: enumerate targets with
  `cargo +nightly fuzz list` and run each for `--time` seconds (default 30),
  mirroring the CI guard flags; update the doc comment and the `Fuzz` task
  help text.

## 4. splot-validate property tests

- [x] 4.1 `crates/splot-validate/Cargo.toml`: add `proptest.workspace = true`
  as a dev-dependency (workspace dep already exists; no new third-party
  crate).
- [x] 4.2 Add a `validator_never_panics` proptest (integration test or
  `#[cfg(test)]` module following the `splot-core` `parsers_never_panic`
  pattern): arbitrary bytes + arbitrary option byte through
  `validate_bytes_with_options` always return a `ValidationReport`.

## 5. Docs and generated artifacts

- [x] 5.1 `AGENTS.md` § 4: generalize the fuzz command lines (smoke runs every
  target; `parse_obu` is no longer the only name). `docs/TESTING.md`: update
  the fuzz section to list the four targets and the surface mapping.
- [x] 5.2 Matrix stages advance with proof (fuzz targets + smoke commands +
  proptest named); regenerate `docs/FEATURE-STATUS.md` and
  `docs/SPEC-COVERAGE.md`; re-record the audit ledger
  (`cargo xtask audit-scope --all --write-ledger`).

## 6. Verification

- [x] 6.1 `cargo +nightly fuzz list` shows all four targets;
  `cargo xtask fuzz --time 10` runs each and reports no panics.
- [x] 6.2 `cargo test -p splot-validate` passes on the pinned toolchain with
  the new proptest.
- [x] 6.3 `cargo xtask feature-status` / `check-feature-status` pass.
- [x] 6.4 `cargo xtask ci` passes end to end with
  `RUSTUP_TOOLCHAIN=1.96.0-aarch64-apple-darwin`.

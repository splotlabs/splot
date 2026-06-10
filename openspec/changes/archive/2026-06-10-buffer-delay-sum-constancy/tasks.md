# Tasks: Buffer-delay sum-constancy validation

## 1. Pre-implementation bookkeeping

- [x] 1.1 docs/IMPLEMENTATION-MATRIX.toml: set `openspec_change =
  "buffer-delay-sum-constancy"` on `AV2-5.11.3-OPS-DECODER-MODEL-INFO`; note the
  § 6.4.13 tier on `AV2-6.4-SEQUENCE-HEADER-SEMANTICS` (its § 6.4.13 maintainer
  question is now resolved — record the AVM-non-enforcement finding and the
  2026-06-10 two-tier decision in the row notes).
- [x] 1.2 Register the change in `openspec/changes/README.md` (Active changes
  table).

## 2. Namespace co-evolution

- [x] 2.1 Add `decoder-model/` to `DIAGNOSTIC_PREFIXES` in
  `xtask/src/feature_status.rs` and document the namespace rules in
  `docs/FEATURE-TRACKING.md` § 12.

## 3. Diagnostics

- [x] 3.1 Implement `decoder-model/buffer-delay-sum-changed` (error, § 6.10.5 with
  § 6.4.13): per-`(obu_xlayer_id, ops_id, operating-point index)` last-explicit-sum
  state carrying CVS epoch + OPS-reset generation, reusing the existing
  `OperatingPointSetRecord` reset semantics and `CvsTracker` boundaries. Inline
  spec quotes; soundness argument (non-conforming under every candidate reading)
  as a code comment. Tests: firing case, cross-CVS negative, reset-spanning
  negative, absent-info negative, Annex E defaults never compared.
- [x] 3.2 Implement `decoder-model/buffer-delay-sum-changed-across-cvs` (warning,
  § 6.4.13 / § 6.10.5): frame-confirmed activated seq-header sums across CLK
  boundaries per xlayer, plus OPS sums across CVS/reset boundaries. Advisory
  message names the scope ambiguity. Tests: seq-header firing case, OPS
  cross-boundary firing case, absent-info negative, fallback-guess-activation
  negative.
- [x] 3.3 `ExternalHlsMode::Provided` suppression for both ids, with tests.

## 4. Registry, docs, and generated artifacts

- [x] 4.1 Add both rule ids to `docs/VALIDATOR-DIAGNOSTICS.md` registry tables,
  including the explicit note that no AVM differential oracle exists for the
  `decoder-model/` rules (AVM parses but never consumes these values).
- [x] 4.2 docs/VALIDATOR-ROADMAP.md: resolve the § 6.4.13 maintainer-question
  mention (the `sequence-multistream-semantics` Non-goals entry stays as history;
  the roadmap should reflect the landed two-tier outcome).
- [x] 4.3 Matrix stages advance with proof (tests named in `proof`); rows stay
  honest (`validate = partial` where residuals remain).
- [x] 4.4 Regenerate `docs/FEATURE-STATUS.md` and `docs/SPEC-COVERAGE.md`; check
  `README.md` claims; re-record the audit ledger
  (`cargo xtask audit-scope --all --write-ledger`).

## 5. Verification

- [x] 5.1 `cargo xtask feature-status`, `check-feature-status`, and
  `check-diagnostic-registry` pass with the new namespace.
- [x] 5.2 `cargo xtask ci` passes with
  `RUSTUP_TOOLCHAIN=1.96.0-aarch64-apple-darwin`.
- [x] 5.3 Fuzz smoke: `cargo xtask fuzz --time 30` shows no panics.

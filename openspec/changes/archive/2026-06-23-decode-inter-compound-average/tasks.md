# Tasks

## 1. OpenSpec, Spec Mapping, and Feature Tracking

- [x] 1.1 Validate the `decode-inter-compound-average` OpenSpec artifacts.
- [x] 1.2 Confirm `docs/SPEC-MAPPING.md` covers the AV2 sections used by the
      change, updating it if required.
- [x] 1.3 Add `DECODE-INTER-COMPOUND-AVERAGE` and the
      `inter-compound-average` decoder-support row to
      `docs/IMPLEMENTATION-MATRIX.toml`.

## 2. Fixture and Oracle Evidence

- [x] 2.1 Generate or select a committed three-frame fixture that reaches the
      old `reference_select` compound gate and uses the narrow
      `COMPOUND_AVERAGE` subset.
- [x] 2.2 Confirm `avmdec --rawvideo --i420` output equals dav2d raw output and
      record the raw digest in `docs/LOCAL-REFERENCE-EVIDENCE.toml`.
- [x] 2.3 Add the fixture to `tests/conformance/vectors/valid/` and
      `tests/conformance/manifest.toml`.

## 3. CDF and Compound Syntax Reads

- [x] 3.1 Add tile CDF rows/selectors for `comp_mode`, `is_joint`, and
      `compound_mode_non_joint`, sourced from generated AV2 defaults.
- [x] 3.2 Add private minimal-inter helpers for the fixture-proven compound parse
      order: `comp_mode`, implicit two-ref selection, `is_joint`,
      `compound_mode_non_joint`, DRL, compound type/CWP gates, and interp filter.
- [x] 3.3 Add positive and negative parser tests covering the admitted branch,
      rejected compound branches, EOF/short-buffer cases, and typed errors.

## 4. Compound Motion Compensation

- [x] 4.1 Add a `splot-recon` compound subpel helper that returns § 7.13.3.18
      signed intermediates without early clipping.
- [x] 4.2 Add an equal-weight `COMPOUND_AVERAGE` blend helper that derives the
      final shift from `InterPostRound` and clips only after blending.
- [x] 4.3 Unit-test intermediate precision, blend length checks, bit-depth
      clipping, and raw sample equality against hand-computed cases.

## 5. Minimal Runtime Wiring

- [x] 5.1 Pass `reference_select` and the needed reference-order facts into the
      minimal inter block decoder.
- [x] 5.2 Return compound block metadata (`RefFrame[0]`, `RefFrame[1]`, `Mv[0]`,
      `Mv[1]`) and route it to compound MC when present.
- [x] 5.3 Preserve existing single-reference fixture behavior and add structured
      `decode/unsupported-feature` gates for every compound branch outside the
      proven subset before any output is written.

## 6. Bit-Exact Regression and Documentation

- [x] 6.1 Add CLI/runtime tests proving the compound fixture raw output matches
      the recorded `avmdec`/dav2d digest and fails the old compound gate.
- [x] 6.2 Regenerate decoder support/status and any generated docs affected by
      the new row.
- [x] 6.3 Keep broad decoder/conformance claims partial or unsupported in docs.

## 7. Verify and Gate

- [x] 7.1 Run focused tests for `splot-recon`, `splot-decode` tile/inter runtime,
      and CLI decode.
- [x] 7.2 Run `cargo xtask feature-status`, `cargo xtask check-feature-status`,
      `cargo xtask check-fixtures`, and `cargo xtask conformance`.
- [x] 7.3 Run `cargo xtask ci` and `openspec validate --all --no-interactive`.

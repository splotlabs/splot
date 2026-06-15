# Tasks

## 1. Contract Documents

- [x] 1.1 Add `docs/DECODER-FULL-CONFORMANCE.md` with current non-claim,
  final conformance definition, output variants, diagnostics requirements,
  resource/output safety, local-reference evidence boundary, and final
  completion criteria.
- [x] 1.2 Add generated `docs/DECODER-SPEC-COVERAGE.md` that renders the
  decoder-relevant AV2 section-family coverage rows.
- [x] 1.3 Update `docs/DECODER-ROADMAP.md` or adjacent decoder docs only where
  needed to point to the full conformance contract without claiming runtime
  decode support.

## 2. Matrix And Coverage Data

- [x] 2.1 Add `DOC-DECODER-FULL-CONFORMANCE-CONTRACT` to
  `docs/IMPLEMENTATION-MATRIX.toml` with proof for the contract document.
- [x] 2.2 Add `XTASK-DECODER-CONFORMANCE-COVERAGE` to
  `docs/IMPLEMENTATION-MATRIX.toml` with proof for the generator/check gate.
- [x] 2.3 Add decoder support matrix rows for the full conformance contract and
  decoder conformance coverage gate.
- [x] 2.4 Define the initial decoder conformance coverage row set, including all
  section families named in the Step 0 gap audit and mission final definition of
  done.
- [x] 2.5 Ensure every normative coverage owner row has a non-empty Feature ID,
  including fixing or avoiding the current empty `intra-reconstruction`
  `feature_id` before using that row as a normative owner.
- [x] 2.6 Regenerate `docs/FEATURE-STATUS.md`,
  `docs/SPEC-COVERAGE.md`, and `docs/DECODER-SUPPORT-STATUS.md` as required by
  matrix updates.

## 3. Xtask Gate

- [x] 3.1 Add `cargo xtask decoder-conformance-coverage --format markdown --output docs/DECODER-SPEC-COVERAGE.md`.
- [x] 3.2 Add `cargo xtask check-decoder-conformance-coverage`.
- [x] 3.3 Validate coverage row status values, support-row cross-links,
  diagnostic cross-links, local-reference evidence ids, and supported-row proof.
- [x] 3.4 Wire `check-decoder-conformance-coverage` into `cargo xtask ci`.
- [x] 3.5 Add focused xtask tests for rendering, drift detection, invalid status,
  missing support-row/evidence references, and unsupported rows remaining
  visible.

## 4. Review And Safety

- [x] 4.1 Record subagent planning results in an agent log or PR notes for
  spec-mapping, decoder architecture, and security/reference-evidence review.
- [x] 4.2 Run correctness/security review for false conformance claims, especially
  parser-only evidence accidentally marked as runtime decode support.
- [x] 4.3 Confirm no AVM/dav2d integration, wrapper, required setup, local path
  probe, external command execution, CI job, cache, or dependency was added.

## 5. Verification

- [x] 5.1 `openspec validate decoder-full-conformance-contract --strict`
- [x] 5.2 `cargo xtask decoder-conformance-coverage --format markdown --output docs/DECODER-SPEC-COVERAGE.md`
- [x] 5.3 `cargo xtask check-decoder-conformance-coverage`
- [x] 5.4 `cargo xtask check-decoder-support`
- [x] 5.5 `cargo xtask check-feature-status`
- [x] 5.6 `cargo xtask check-diagnostic-registry`
- [x] 5.7 `cargo xtask ci`

## 6. Archive

- [x] 6.1 Archive the change with `openspec archive decoder-full-conformance-contract --yes`.
- [x] 6.2 Run `openspec validate --all --no-interactive` after archive.
- [x] 6.3 Run `cargo xtask ci` after archive.

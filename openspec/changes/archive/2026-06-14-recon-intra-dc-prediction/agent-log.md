# Agent Log: recon-intra-dc-prediction

Objective: add a narrow, scheduler-free square DC intra prediction primitive in
`splot-recon`, tracked by Feature ID `RECON-INTRA-DC-SQUARE-PREDICTION`, without
claiming full scalar intra reconstruction or runtime decode success.

## Orchestrator Plan

- Continue the decoder mission after PR #130 merged to `origin/main`.
- Start from a clean detached `origin/main` worktree at merge commit `aa61b78`.
- Keep unrelated active OpenSpec changes untouched.
- Create and validate one OpenSpec change before creating the feature branch.
- Implement one PR-sized `splot-recon` primitive, then run focused and full
  gates.
- Archive the OpenSpec change before opening a ready PR.
- Do not make the PR draft unless explicitly asked.
- Do not merge before CI is green and Codex has completed review on the latest
  head commit. An `eyes` reaction is only in-progress acknowledgement.

## Carry-Forward Review Context

The Codex review on PR #113:
`https://github.com/splotlabs/splot/pull/113#pullrequestreview-4492663492`
raised four inline comments about byte-planner precedence, IVF retry state,
decode fuzz seeds, and `DecodeContext` docs.

Those comments were addressed by PR #114
(`fix(decode): address byte planner review feedback`), merged at
`2026-06-14T10:11:06Z`. This change may mention that review only as already
resolved carry-forward context; it is not re-fixing those comments.

## Subagent Roster And Prompts

| Agent | Layer | Objective | Output / Status |
|---|---|---|---|
| @orchestrator | Orchestrator | Own sequencing, OpenSpec, implementation, final acceptance. | In progress. |
| Schrodinger the 4th (`019ec767-4465-7923-af2b-b519c27bbf1d`) | @spec-reader explorer | Inspect AV2 §7.13-§7.15 and recommend the smallest safe intra reconstruction slice. | Complete: recommended a scheduler-free `splot-recon` DC intra prediction primitive, with a warning not to substitute ordinary division for §7.13.2.10 / §7.13.3.22 math. |
| Gauss the 4th (`019ec767-477b-72b0-928c-ed09293b8bf7`) | @api-designer explorer | Inspect `splot-recon` / `splot-decode` APIs and recommend a smallest PR-sized addition. | Complete: recommended a recon workspace/layout API before prediction integration; also confirmed `splot-recon` must remain scheduler-free and decode orchestration belongs to `DecodeContext`. |
| Euclid the 4th (`019ec767-4a46-7502-a7d4-4a64f348bbe6`) | @architect/backlog explorer | Audit decoder backlog, active OpenSpec conflicts, and agent-log requirements. | Complete: identified `intra-reconstruction` as the only pure todo row and recommended `recon-intra-dc-prediction` as the next PR-sized change, avoiding unrelated active changes. |
| Aristotle the 4th (`019ec767-4d0d-7701-8137-b36f94133708`) | @reference-boundary explorer | Audit AVM/dav2d boundary and local-only evidence rules. | Complete: repo boundary is clean; no AVM/dav2d run, wrapper, dependency, script, CI, or mandatory test should be added for this change. |

## Architect Findings

- Current decoder is still plan-only and stops at unsupported `decode_tile()`.
- The decoder support matrix has one pure todo row: broad scalar intra
  reconstruction.
- `recon-intra-dc-prediction` is PR-sized if scoped to `splot-recon` only:
  no runtime decode, no tile traversal, no transform/dequant, no CLI behavior,
  no diagnostics, no dependency graph change, and no AVM/dav2d integration.
- Active OpenSpec changes `add-bitstream-writer`, `toy-intra-encoder-v0`, and
  `avm-differential-harness` are unrelated/parked for this mission and must not
  be advanced here.

## Spec Reader Evidence

- §7.13.2.10 defines DC intra prediction inputs (`haveLeft`, `haveAbove`,
  `log2W`, `log2H`) and the four edge-availability cases.
- §7.13.2.11 defines `approx_divide(num, den)` in terms of `resolve_divisor`.
- §7.13.3.22 defines `resolve_divisor` and the `Div_Lut` table.
- §7.14 and §7.15 cover dequantization, reconstruction, and inverse transforms;
  those remain out of scope.
- The implementation must not approximate both-edge rectangular DC with ordinary
  integer division unless the §7.13.3.22 table path is modeled.

## API Designer Notes

- `splot-recon` currently exposes immutable frame/plane types, hash input,
  digest, Y4M writer, and reference-slot storage.
- There is no mutable reconstruction workspace/layout API yet. That is a strong
  candidate for a later change before runtime integration.
- For this change, the API remains square-DC-specific but exposes both
  no-allocation scalar/strided write entry points and an owned convenience block,
  so future workspace integration can avoid temporary allocation without pulling
  scheduling into `splot-recon`.

## Boundary Decision

The first primitive is square-only: `w = h = 1 << log2_size`. This makes the
both-edge denominator `w + h` a power of two, so the §7.13.3.22
`resolve_divisor` path specializes to `Round2(sum, log2_size + 1)` without
adding or hand-transcribing `Div_Lut`. Rectangular DC prediction remains a named
future residual.

## Reference Oracle Evidence Or Not Used

No AVM or dav2d run is planned for this change. The primitive is verified by
self-contained Rust tests against the committed AV2 spec mirror. AVM and dav2d
remain local-only tools and must not be introduced into repo code, source,
scripts, build, tests, `xtask`, or CI.

## Implementation Notes

- Added `crates/splot-recon/src/intra.rs` with scheduler-free square DC intra
  prediction types: `IntraSquareBlockSize`, `IntraDcEdges`,
  `SquareIntraPredictionBlock`, row iteration,
  `predict_intra_dc_square_value`, `predict_intra_dc_square_into`, and the
  owned convenience wrapper `predict_intra_dc_square`.
- The implementation supports 4x4 through 64x64 square regions, matching the
  `Tx_Width_Log2` / `Tx_Height_Log2` range in
  `docs/spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md`.
- Edge availability follows §7.13.2.10:
  both edges, left only, above only, and no-edge midpoint. The both-edge case is
  intentionally limited to square regions so `w + h` is a power of two and the
  §7.13.3.22 `resolve_divisor` path specializes without introducing `Div_Lut`.
- Added typed `ReconError` variants for invalid square block size, edge length
  mismatch, edge sample range violations, storage conversion failure, output
  stride/length violations, and allocation failure.
- Extended the sealed `ReconSample` abstraction with fallible `try_from_u16` so
  validated prediction samples can be stored in `u8` or `u16` without a public
  truncating conversion.
- No decode-driver, CLI, threading, dependency graph, AVM/dav2d, or CI behavior
  was added.

## Tests And Fixtures

- Added self-contained `splot-recon` unit tests for valid square size metadata,
  no-edge midpoint, left-only, above-only, both-edge square averaging, scalar
  prediction value, caller-owned strided output, wrong edge length, unsupported
  storage/bit-depth combinations, out-of-range edge samples, output stride,
  output length, overflowing output-shape arithmetic, fallible storage
  conversion, and typed allocation failure.
- No fixtures were added. No AVM or dav2d local run was required.

## Documentation And Matrix Updates

- Added `RECON-INTRA-DC-SQUARE-PREDICTION` to
  `docs/IMPLEMENTATION-MATRIX.toml` with proof commands and spec sections
  `7.13.2.10` / `7.13.3.22`.
- Added `intra-dc-square-prediction` to
  `docs/DECODER-SUPPORT-MATRIX.toml` as supported, while changing the broad
  `intra-reconstruction` row from pure todo to partial.
- Updated `docs/DECODER-ROADMAP.md` to state that square DC prediction is
  supported, while rectangular DC, non-DC modes, inverse transforms, dequant,
  residual reconstruction, runtime frame output, and reference refresh remain
  future work.
- Regenerated:
  `docs/DECODER-SUPPORT-STATUS.md`,
  `docs/FEATURE-STATUS.md`, and
  `docs/SPEC-COVERAGE.md`.

## Review Findings

- @reviewer: no initial whole-change findings; later re-review found stale
  agent-log text/counts and independently flagged the strided output arithmetic
  overflow before final cleanup.
- @performance-reviewer: medium finding that allocation-only owned output would
  force a temporary block for future reconstruction/encoder paths.
- @encoder-impact-reviewer: medium findings that public infallible
  `ReconSample::from_u16` could truncate and that `IntraPredictionBlock` was
  named broader than its square-only geometry; later high finding that
  `validate_output_shape` used unchecked arithmetic for caller-controlled
  stride.
- @security-reviewer: no initial security findings and AVM/dav2d boundary
  signed off; final re-review signed off after the checked-arithmetic fix.
- @spec-conformance-reviewer: no spec-conformance findings; square DC math and
  scope were signed off.
- @performance-reviewer: final re-review signed off that the no-allocation
  scalar/strided APIs resolve the allocation finding and introduce no scheduler
  or hot-loop issue.
- @reviewer: final whole-change re-review signed off after checked arithmetic,
  regression coverage, and agent-log cleanup.

## Security Review

- Initial security pass found no unsafe code, runtime `unwrap`/`expect`, panic
  macros, unchecked public indexing, scheduler state, Rayon/crossbeam usage, or
  dependency-direction impact.
- Final security pass signed off on the strided output API after checked
  arithmetic was added.
- AVM/dav2d boundary confirmation: no AVM/dav2d source, snippets, binaries,
  submodules, dependencies, build probes, wrappers, CI jobs, required scripts,
  required `xtask` commands, or mandatory tests were added.

## Spec Conformance Review

- Signed off against the committed AV2 mirror: §7.13.2.10 four DC cases,
  §7.13.2.11 / §7.13.3.22 `approx_divide` / `resolve_divisor` implications, and
  §9 transform log2 ranges.
- Confirmed docs/matrices remain square-DC-only and do not overclaim runtime
  decode, rectangular DC, non-DC intra modes, transforms, residuals, hashes,
  Y4M, or reference refresh.

## Encoder Impact Review

- Confirmed the final API avoids the earlier truncating conversion and broad
  block naming. It keeps `splot-recon` scheduler-free and avoids
  `splot-decode` / `splot-encode` coupling.
- Final re-review signed off after checked arithmetic was added to the strided
  output path.

## Fixes Made

- Added `predict_intra_dc_square_value` for no-allocation scalar DC prediction.
- Added `predict_intra_dc_square_into` for caller-owned strided output.
- Renamed the owned output and row iterator to `SquareIntraPredictionBlock` and
  `SquareIntraPredictionRows`.
- Replaced public infallible `ReconSample::from_u16` with fallible
  `try_from_u16` and typed `SampleValueUnsupportedStorage`.
- Added typed output stride/length errors.
- Changed strided output required-length derivation to checked
  multiplication/addition with `ReconError::ArithmeticOverflow` on overflow.
- Added regression coverage for the overflowing stride case.

## Gates Run

- `cargo xtask ci`: passed on baseline `origin/main` before this OpenSpec was
  edited.
- `openspec validate recon-intra-dc-prediction --strict`: passed after
  implementation/docs update.
- `cargo test -p splot-recon --locked`: passed, 73 tests after review fixes.
- `cargo clippy -p splot-recon --all-targets --locked -- -D warnings`: passed.
- `cargo xtask feature-status`: passed and listed
  `RECON-INTRA-DC-SQUARE-PREDICTION`.
- `cargo xtask check-feature-status`: passed, 167 features.
- `cargo xtask check-decoder-support`: passed, 26 rows.
- `cargo xtask check-concurrency-policy`: passed.
- `cargo xtask check-dependency-direction`: passed.
- `cargo xtask ci`: passed on the post-review-fix diff.

## AVM/dav2d Boundary Audit

- No AVM/dav2d source, snippets, binaries, submodules, dependencies, build
  probes, wrappers, CI jobs, required scripts, or mandatory tests were added.
- No local absolute paths or environment-specific reference evidence were
  committed.

## OpenSpec Validation And Archive Evidence

- `openspec validate recon-intra-dc-prediction --strict`: passed before
  archive.
- `openspec archive recon-intra-dc-prediction --yes`: completed and folded one
  decoder-support delta into `openspec/specs/decoder-support/spec.md`.

## Latest Codex Review Evidence

Pending.

## Final Acceptance Decision

Implementation phase complete locally: all review-agent findings are fixed or
closed with sign-off, `cargo xtask ci` passed on the post-review-fix diff, and
the OpenSpec change is ready to archive.

# Agent Log: recon-current-frame-workspace

Feature ID: `RECON-CURRENT-FRAME-WORKSPACE`

## Objective

Add a scheduler-free `splot-recon` mutable current-frame workspace that future
decoder and encoder code can fill before freezing into the existing immutable
decoded-frame, hash, Y4M, and reference-store surfaces. This change must not
add a `splot-decode -> splot-recon` dependency, runtime decode output, external
decoder integration, or scheduler state in `splot-recon`.

## Carry-forward Review Context

- PR #113 Codex review
  `https://github.com/splotlabs/splot/pull/113#pullrequestreview-4492663492`
  was re-read for this change. It contained four actionable comments:
  `discussion_r3409278210` unsupported-structure precedence before byte limits,
  `discussion_r3409278211` retry-stable IVF first-frame header errors,
  `discussion_r3409278212` prefixed `decode_plan_bytes` valid fuzz seeds, and
  `discussion_r3409278215` raw-byte `DecodeContext` docs.
- Current `origin/main` already contains the follow-up fix in merged, non-draft
  PR #114, `fix(decode): address byte planner review feedback`
  (`07a7bd9da821f2dbab00cad26b8f6ff3779af929`, merged
  `2026-06-14T10:11:06Z`).
- Verification evidence on current main: repository search shows prefixed
  `decode_plan_bytes` seeds in `.github/workflows/ci.yml`, the retry-stability
  test in `crates/splot-core/src/ivf.rs`, and the archived
  `decode-byte-stream-review-fixes` OpenSpec change.
- Process correction: future PRs must wait for Codex review completion on the
  latest head commit before merging. A GitHub `eyes` reaction means review is in
  progress, not complete.

## Planning Subagents

| Subagent | Role | Result |
| --- | --- | --- |
| Gibbs the 4th (`019ec792-37ca-73f1-a8a9-2bdc2d09547c`) | Decode/recon architecture | Recommended a minimal intra-tile handoff next. Useful later, but it requires source-backed decode state threading before producing pixels. |
| Herschel the 4th (`019ec792-3acd-7c31-8139-dafbdcc29459`) | AV2 spec reading | Recommended current-frame workspace first, with anchors in AV2 §7.13.2.1, §7.14.3, §5.20.7.24, §5.20.2.3, §6.19.2.3, §5.20.7.26, §6.4.1, §6.17.4.1, §6.17.4.4, and §7.21.2. |
| Maxwell the 4th (`019ec792-3d86-78e2-a8b9-934f73f3b5cc`) | Local reference boundary | Confirmed no AVM/dav2d run is required unless this PR claims decoded pixels, runtime hashes, Y4M parity, output ordering, reference refresh, or end-to-end external agreement. |
| Hooke the 4th (`019ec792-4069-7731-9d98-08c7d1e234d2`) | Security/resource review | Recommended allocation dimensions, strides, and bytes remain checked and typed before allocation; hostile byte-stream limits remain a future `splot-decode` responsibility. |
| Ramanujan the 4th (`019ec792-42fc-79f3-a003-d37f4a370bef`) | Encoder impact | Recommended a mutable reconstruction workspace before rectangular DC/all-DC assembly, with no `splot-encode -> splot-recon` dependency and no scheduler in recon. |

## PR #101 Concurrency Boundary

- This change incorporates the PR #101 direction by keeping `splot-recon`
  scheduler-free.
- Do not import or call Rayon, crossbeam, ad-hoc threads, global pools,
  `splot_parallel`, bounded queues, or worker pools from `splot-recon`.
- Future parallel decode orchestration must remain above this workspace through
  `splot-decode` `DecodeContext` and `splot_parallel::WorkerPool`.

## Reference Boundary

- No AVM/dav2d source, snippets, binaries, dependencies, wrappers, scripts, CI
  jobs, or required tests are introduced by this change.
- Local reference metadata gathered by the reference subagent remains advisory
  only because this PR does not claim decoded-pixel parity or external-decoder
  agreement.

## Implementation Notes

- Added `crates/splot-recon/src/workspace.rs` with public
  `CurrentFrameWorkspace<T>`, `CurrentFramePlane<T>`,
  `WorkspaceRectRows<'_, T>`, and `CurrentFrameIntraEdges<T>`.
- Added workspace-specific `ReconError` variants for allocation failure,
  missing planes, rectangle bounds, source stride, and source buffer length.
- Workspace allocation derives Y/chroma storage from `DecodedFrameInfo` and
  `PixelFormat` subsampling, validates sample type/fill range before
  allocation, computes required samples/bytes with checked arithmetic, and uses
  `try_reserve_exact`.
- Bounded APIs cover plane metadata, read rows, fill, row-strided writes,
  contiguous square-block writes, in-storage edge extraction, square DC
  prediction writes through `predict_intra_dc_square_into`, and freeze through
  `Plane::from_vec`, `FramePlanes::new`, and `DecodedFrame::try_new`.
- Public docs and generated support matrices explicitly keep runtime
  `splot decode`, tile syntax traversal, runtime hash/Y4M output, reference
  refresh, external decoder invocation, and `splot-decode -> splot-recon`
  dependency out of scope.
- `splot-recon` remains scheduler-free; no Rayon, crossbeam, `splot_parallel`,
  thread, pool, or queue import was added.

## Tests and Checks

- `cargo test -p splot-recon --locked` passed: initially 85 tests, then 86
  tests after the `fill_rect` error-precedence regression test was added.
- `cargo clippy -p splot-recon --all-targets --locked -- -D warnings` passed.
- `cargo fmt --all -- --check` passed.
- `cargo xtask check-source-lines` passed. Existing unrelated files above the
  soft 1000-line advisory were reported; new workspace files are below the
  soft limit.
- `cargo xtask check-feature-status` passed.
- `cargo xtask check-decoder-support` passed.
- `cargo xtask check-dependency-direction` passed.
- `cargo xtask check-concurrency-policy` passed.
- `openspec validate recon-current-frame-workspace --strict` passed.

## Review Notes

- Cicero the 4th (`019ec7a4-d70c-7f82-85ff-f8728923fd12`) reviewed
  concurrency, dependency, AVM/dav2d, and process boundaries. Findings: none.
- Parfit the 4th (`019ec7a4-bea7-73d1-93ae-55929d048730`) reviewed API,
  resource handling, spec claims, and PR #113 carry-forward. Findings fixed:
  - P2 matrix/spec-coverage overclaimed broad §7.14.3 / §7.21.2 reconstruct
    and output-prep coverage. Fixed by narrowing workspace spec sections to
    geometry/current-frame intra context and regenerating status docs.
  - P3 OpenSpec design asked for public mutable complete backing samples. Fixed
    by documenting validated mutation only and no public mutable slice escape.
  - P3 `fill_rect` validated sample range before missing-plane lookup. Fixed by
    resolving the plane first and adding
    `workspace_fill_checks_plane_before_sample_range`.
- Parfit targeted re-review after fixes reported no remaining findings for the
  three targeted items.

## Post-Archive Mission Handoff

- After this OpenSpec change is archived, continue with the mission-level
  release path: run `cargo xtask ci`, create a ready PR, trigger Codex review,
  and do not merge until required checks are green and Codex review has
  completed on the latest head commit.

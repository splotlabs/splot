# Agent Log: decode-context-tile-payload-handoff

Feature ID: `DECODE-CONTEXT-TILE-PAYLOAD-HANDOFF`

## Orchestrator Plan

- Verify PR #113 review carry-forward before starting new code.
- Plan a narrow context-orchestration slice that wires the existing
  crate-private tile-payload boundary through `DecodeContext` and
  `splot_parallel::WorkerPool`.
- Keep the API crate-private, avoid public tile payload contracts, avoid
  `splot-recon` dependency changes, and preserve the AVM/dav2d local-only
  boundary.
- Validate OpenSpec before creating a feature branch.
- Implement sequentially, then run focused tests, repo checks, review agents,
  archive OpenSpec, full `cargo xtask ci`, ready PR, CI watch, and latest-head
  Codex review wait before any merge.

## PR #113 / PR #114 Review Carry-Forward

The user explicitly requested review of
`https://github.com/splotlabs/splot/pull/113#pullrequestreview-4492663492` and
`discussion_r3409248110` before continuing.

- PR #113 review `4492663492` had four Codex comments on commit `3066f4d85e`:
  unsupported-structure precedence, IVF cursor retry behavior,
  `decode_plan_bytes` fuzz seeds, and `DecodeContext` docs.
- `discussion_r3409248110` was Claude's comment about duplicated Annex B/IVF
  parser logic in `splot-decode`.
- Current `origin/main` (`3524d31`) contains PR #114
  `fix(decode): address byte planner review feedback`, merged at
  `2026-06-14T10:11:06Z`, final head `07a7bd9da821f2dbab00cad26b8f6ff3779af929`.
- Verified on current `main`:
  - `splot-decode` uses `splot_core::annexb::AnnexBObuCursor` and
    `splot_core::ivf::IvfFrameCursor`; local parser copies named in
    `discussion_r3409248110` are absent from `splot-decode`.
  - `unsupported_prefix_is_reported_before_later_obu_limit` passes.
  - `frame_cursor_retry_preserves_truncated_initial_frame_header_error` passes.
  - CI seeds `decode_plan_bytes` with prefixed fixture payloads.
  - `DecodeContext` docs now describe raw-byte planning.
- PR #114 had green required checks and final Codex comment:
  `Codex Review: Didn't find any major issues` for reviewed commit `07a7bd9da8`.

Conclusion: no extra PR is needed solely for PR #113 review carry-forward; this
change must avoid regressing those fixes and must mention the carry-forward in
the PR body.

## Planning Agents

| Agent | Role | Objective | Result |
|---|---|---|---|
| Nietzsche the 4th (`019ec7b7-91da-7de2-b61f-47a6aeed289a`) | `@architect` | Evaluate whether a `DecodeContext` tile-payload handoff is the right next PR-sized slice and identify crate-boundary risks. | Complete: this is the right next PR-sized slice after `RECON-CURRENT-FRAME-WORKSPACE` if scoped as `DecodeContext` handoff, not real tile decode. Recommended `DECODE-CONTEXT-TILE-PAYLOAD-HANDOFF`, wrapper over existing `plan_tile_payload_boundary(...)` inside `self.pool.install(...)`, serial planner for now, no direct Rayon/crossbeam/global/nested pool/ad-hoc threads, `splot-recon` untouched/scheduler-free, no `splot-decode -> splot-recon` edge, no public tile-payload API. Also confirmed PR #113/#114 carry-forward is clean on current `main`. |
| Averroes the 4th (`019ec7b7-b6eb-79e2-a21b-47d18ec69aec`) | `@spec-reader` / `@spec-conformance-reviewer` | Identify exact spec sections, feature IDs, and overclaiming risks for the handoff. | Complete: no additional AV2 citations are required for a pure context handoff. Use the existing tile-payload sections (`5.20.1`, `5.20.2.1`, `6.19.1`, `7.1`, `8.2.2`, `8.3`) plus the non-normative `INFRA-PARALLEL-RUNTIME-POLICY`. Do not cite `5.19`/`6.18` unless deriving tile-group header/range facts. Keep unsupported residuals explicit: no `decode_tile()`, block syntax, `exit_symbol()` after real syntax, CDF copyback/averaging mutation, `frame_end_update_cdf()`, `decode_frame_wrapup()`, reconstruction/hash/Y4M/reference refresh, AVM/dav2d invocation, or direct concurrency primitives. Confirmed PR #113 fixes are present via PR #114. |
| Avicenna the 4th (`019ec7b7-d2c9-7bf0-bdbc-9e2db3926118`) | `@api-designer` / `@encoder-impact-reviewer` | Recommend crate-private/public API shape and encoder impact. | Complete: make this a crate-private `DecodeContext::plan_tile_payload_boundary` wrapper over the existing crate-private function. Do not make `TilePayloadBoundaryError`, `DecodeTilePayloadPlan`, CDF boundary types, mutable work units, or tile bytes public. Do not add `DecodeError::TilePayload` yet. Replace the manual `ctx.pool().install(...)` test with the context method and add an error propagation test. This proves the PR #101 concurrency model while keeping future encoder/recon work deterministic without freezing a public API. |
| Boyle the 4th (`019ec7b7-e810-7e60-8cc4-3f649a28c04e`) | `@security-reviewer` / `@performance` | Evaluate untrusted-input, bounds, allocation, panic, and concurrency risks. | Complete: no blocker for a crate-private wrapper around the existing bounded planner. Existing tile boundary checks tile-count multiplication, per-tile payload limits, tile slices, grid ranges, and byte-offset/span arithmetic. Scope must not grow into public API or raw byte tile decode. Required checks: `cargo test -p splot-decode tile_payload --locked`, full `splot-decode` tests, clippy, `check-concurrency-policy`, `check-dependency-direction`, `check-decoder-support`, `check-feature-status`, and full `cargo xtask ci`. |
| Faraday the 4th (`019ec7b7-fef2-7201-a7c3-68a3c5b66204`) | `@reference-oracle` with AVM/dav2d perspectives | Decide whether local reference evidence is required and confirm the external-decoder boundary. | Complete: local AVM/dav2d evidence is not required because this handoff does not decode tile syntax, reconstruct pixels, compute hashes, write Y4M, refresh references, or output. Local reference checkouts were observed only as background metadata: AVM commit `f6f0b9c8914f38be39a953c0a9aa6a2e4050717c`; dav2d commit `f4f96cb06bb3cd3f31e29e1f190f1c0e373ab352`. Both are explicitly deferred for this PR. |

## Local Evidence And Commands

- `git status --short --branch`: clean detached `origin/main` before planning.
- `git fetch origin --prune`: succeeded.
- `openspec list`: unrelated stale/other changes remain; this change is scoped
  separately and does not touch them.
- `cargo test -p splot-decode unsupported_prefix_is_reported_before_later_obu_limit --locked`: passed.
- `cargo test -p splot-core frame_cursor_retry_preserves_truncated_initial_frame_header_error --locked`: passed.

## Boundary Commitments

- No AVM/dav2d source, snippets, binaries, submodules, dependencies, wrappers,
  scripts, CI jobs, required `xtask` commands, or mandatory tests will be added.
- No public tile-payload API will be added.
- No `splot-decode -> splot-recon` dependency will be added in this slice.
- No direct Rayon, crossbeam, global pool, ad-hoc thread, nested pool, or queue
  usage will be added outside `splot_parallel`.
- Clippy showed that the new crate-private context handoff is still unused in
  non-test builds because no runtime path derives `TilePayloadBoundaryInput`
  facts yet. The implementation therefore replaces the stale module
  `allow(dead_code)` with conditional, reasoned `dead_code` allowances for
  non-test builds on the module and context handoff, rather than making tile
  payload types public or inventing a runtime caller.

## Review Findings

All required review agents signed off with no blockers:

- `@implementer` / `@module-implementer` Arendt the 4th
  (`019ec7c2-95df-7450-bad7-a5d7dcf32c7e`): no findings. Confirmed the
  handoff stays crate-private, routes through `self.pool.install(...)`, does not
  expose tile-payload/CDF types publicly, and tests cover deterministic thread
  policy output plus limit-error propagation.
- `@documenter` / `@integration-implementer` Aquinas the 4th
  (`019ec7c2-af7f-7a12-9e27-0c27505afb2c`): no edits needed. Confirmed
  docs/matrix/OpenSpec wording is accurate, generated files are consistent, no
  local absolute paths or external-decoder requirements were introduced, and PR
  #113/#114 carry-forward is documented correctly.
- `@reviewer` Banach the 4th (`019ec7c3-237f-73e0-89fb-7f5866946f9b`): no
  findings. Confirmed no draft PR or merge behavior was relevant before PR
  creation.
- `@security-reviewer` Meitner the 4th
  (`019ec7c3-3ca9-7333-b7f2-552371b02389`): no blockers. Explicitly confirmed
  no AVM/dav2d source, snippets, binaries, submodules, dependencies, build
  probes, wrappers, scripts, CI jobs, required `xtask` commands, or mandatory
  tests were added.
- `@spec-conformance-reviewer` Halley the 4th
  (`019ec7c3-5750-7723-8089-fe39fb579a47`): no findings. Confirmed the change
  does not overclaim runtime decode, `decode_tile()`, CDF mutation,
  reconstruction/output, or AVM/dav2d evidence.
- `@encoder-impact-reviewer` Pauli the 4th
  (`019ec7c3-726a-7c92-9033-23d9a1667a12`): no blockers. Confirmed the
  handoff helps future encoder/reconstruction work without freezing public APIs
  or adding scheduler ownership to `splot-recon`.

## Verification

- `openspec validate decode-context-tile-payload-handoff --strict`: passed.
- `cargo test -p splot-decode tile_payload --locked`: passed.
- `cargo test -p splot-decode --locked`: passed.
- `cargo clippy -p splot-decode --all-targets --all-features --locked -- -D warnings`: passed.
- `cargo fmt --all -- --check`: passed.
- `cargo xtask check-feature-status`: passed.
- `cargo xtask check-decoder-support`: passed.
- `cargo xtask check-dependency-direction`: passed.
- `cargo xtask check-concurrency-policy`: passed.
- `cargo xtask ci`: passed.
- `openspec archive decode-context-tile-payload-handoff --yes`: passed and
  folded the delta into `openspec/specs/decoder-support/spec.md`.
- Post-archive `openspec validate --all --no-interactive`: passed.
- Post-archive `cargo xtask check-feature-status`: passed.
- Post-archive `cargo xtask check-decoder-support`: passed.
- Post-archive `git diff --check`: passed after removing one generated EOF
  blank line.
- Post-archive `cargo xtask ci`: passed.

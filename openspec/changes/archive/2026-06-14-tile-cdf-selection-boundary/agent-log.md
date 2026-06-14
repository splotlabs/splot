# Agent Log: tile-cdf-selection-boundary

Objective: add a narrow crate-private CDF-selection / CDF-bank boundary for the
minimal tile payload path, tracked by Feature ID
`DECODE-TILE-CDF-SELECTION-BOUNDARY`, without claiming runtime tile decode.

## Orchestrator Plan

- Continue the decoder mission after PR #127 merged.
- Fetch `origin` and verify this branch is clean and at `origin/main`
  (`25237f7`).
- Treat open PR #128 as unrelated user work; it does not block this goal because
  the one-PR-in-flight rule applies only to this goal's work.
- Create one OpenSpec change, implement sequentially, run local gates, then open
  one ready PR.
- Do not make the PR draft unless explicitly asked.
- Do not merge before CI is green and Codex has completed review on the latest
  head commit. An `eyes` reaction is only in-progress acknowledgement.

## Carry-Forward Review Context

The linked Codex review on PR #113:
`https://github.com/splotlabs/splot/pull/113#pullrequestreview-4492663492`
was reviewed at commit `3066f4d85e3c3fa502d869b967f47da966cefaef` and raised
four inline comments:

- `discussion_r3409278210`: preserve unsupported-structure precedence before
  later byte traversal limits.
- `discussion_r3409278211`: keep `IvfFrameCursor` retry behavior stable on
  fatal first-frame-header errors.
- `discussion_r3409278212`: preserve valid fixture bytes for
  `decode_plan_bytes` fuzz seeds.
- `discussion_r3409278215`: update `DecodeContext` docs for raw-byte planning.

Those comments were addressed in PR #114
(`fix(decode): address byte planner review feedback`), merged at
`2026-06-14T10:11:06Z` with merge commit
`5f7a9006a51f4067d40ff93e0865e45e4cd52838`. PR #114's final head
`07a7bd9da821f2dbab00cad26b8f6ff3779af929` received a Codex no-major-issues
review before merge. This change may mention the review only as already
resolved carry-forward context; it is not re-fixing those comments.

## Agents

| Agent | Layer | Objective | Output / Status |
|---|---|---|---|
| @orchestrator | Orchestrator | Own sequencing, OpenSpec, implementation, final acceptance. | In progress. |
| Anscombe the 3rd (`019ec71d-cf77-72f0-8310-ba32351e5c9e`) | Planning explorer | Inspect spec/code around §8.3 CDF selection and recommend next milestone. | Complete: recommended `tile-cdf-selection-boundary` / `DECODE-TILE-CDF-SELECTION-BOUNDARY`, focused on first CDF boundary before `decode_tile()`; warned against full recursive partition/reconstruction. |
| McClintock the 3rd (`019ec71d-e53d-7071-9e52-15fecf2781f8`) | API explorer | Inspect `splot-core` symbol/default tables and `splot-decode` tile payload boundary. | Complete: recommended crate-private `crates/splot-decode/src/tile_cdf.rs`, no public API, no `splot-core` mutable CDF bank, tiny subset copied from generated defaults. |
| Aquinas the 3rd (`019ec724-a391-78e1-a239-549e7b6e6ed4`) | @architect planning subagent | Review crate boundaries, concurrency, and CDF-bank ownership. | Complete: keep the boundary crate-private in a sibling `splot-decode` module; no public `DecodeContext` API, no public mutable CDF-bank API in `splot-core`, no new dependencies, no direct Rayon/crossbeam, no AVM/dav2d integration. |
| Arendt the 3rd (`019ec724-c2ec-7883-bc3a-787a98b61a70`) | @spec-reader planning sub-subagent | Extract AV2 spec anchors and requirements from committed mirror. | Complete: identified §5.20.1 tile payload `init_symbol`/`decode_tile`/`exit_symbol`, §8.2.2 Tile CDF copy boundary, §8.2.4 Saved CDF copy/average policy, §8.2.6 CDF update behavior, §8.3 CDF selection by mutable reference, §6.19 frame-end update, and partition/reference-motion-vector boundaries to keep separate. |
| Gibbs the 3rd (`019ec724-db62-7d33-a0d0-84c17e09c827`) | @api-designer planning sub-subagent | Recommend crate-private Rust API shape and tests. | Complete: keep implementation in `splot-decode`, preferably `tile_payload/cdf.rs`; use partition-entry `DoSplit` and `DoSquareSplit` selectors copied from generated defaults; expose private typed selector errors, save-policy calculation, and selected-row symbol handoff; avoid broad CDF banks and public APIs. |
| Copernicus the 3rd (`019ec724-fc07-7e91-a3ce-fe69eb58deba`) | @reference-oracle reference subagent | Confirm whether local AVM/dav2d evidence is needed and verify boundary statement. | Complete: no AVM/dav2d runs required; self-contained Rust tests and spec-cited docs are the right evidence. No local reference source, snippets, binaries, dependencies, scripts, CI jobs, or mandatory tests should be added for this boundary. |

## Local Reference Evidence

No AVM or dav2d run is planned for this boundary. The change copies default CDF
rows from generated repository tables and does not compare decoded output. AVM
and dav2d remain local-only reference tools and must not be introduced into
repo code, source, scripts, build, tests, `xtask`, or CI.

Reference-oracle boundary statement: no AVM/dav2d source, snippets, binaries,
submodules, dependencies, build probes, wrappers, CI jobs, required scripts, or
mandatory tests should be added for this change.

## Boundary Decisions

- Mutable tile CDF state belongs in `splot-decode`, crate-private.
- `splot-core` remains the owner of generated default tables and the generic
  §8.2 `SymbolDecoder`.
- The first subset is intentionally small and partition-entry scoped:
  `DoSplitCdf` and `DoSquareSplitCdf`.
- Copy/average is policy metadata only until a future row wires real
  `decode_tile()` completion and `exit_symbol()`.
- `splot-recon` remains scheduler-free; this change adds no reconstruction
  primitive and no runtime scheduler.
- Reference-motion-vector bank reset, above/left probability contexts, and
  partition traversal are separate future responsibilities; this CDF boundary
  only owns a small mutable CDF-row subset and row-selection handoff.

## Review Log

- @reviewer (`019ec736-dfd4-79a1-8b9e-97cd7e6cfe0c`, Sartre the 3rd): initial read-only review found one important issue: the planner used `TileCdfPolicyInput` tile dimensions independently from `TileGridFacts`, allowing stale CDF policy grid metadata on a one-tile plan. Fixed by deriving CDF policy dimensions from `TileGridFacts` before `tile_cdf_save_policy`, with plan-level tests for explicit avg-CDF and context-update tile-id policies.
- @security-reviewer (`019ec736-e4e2-7e00-9665-a22954c50958`, Feynman the 3rd): no blocking security issues. Confirmed no dependency changes, no production panic/unwrap/unsafe/Command/direct scheduler use, checked arithmetic/indexing, and bounded tile slicing. Informational local-reference metadata concern addressed by scrubbing local AVM/dav2d checkout status from this log.
- @spec-conformance-reviewer (`019ec736-eaad-7b53-aba6-aa44befe1fca`, Hilbert the 3rd): no blocking or important spec issues. Confirmed §8.2.4 copy/average policy, three-entry avg-CDF row math, generated §9.3 table dimensions, update-disable handoff, and docs that avoid overclaiming full `decode_tile()` / full CDF-bank support.
- @encoder-impact-reviewer (`019ec736-effd-7972-b860-39bebca46bd5`, Banach the 3rd): no blocking encoder-impact or PR #101 concurrency issues. Confirmed no public API change, no new public `DecodeContext` API, no new dependencies, no `splot-recon` scheduler state, no direct Rayon/crossbeam/thread use, and no AVM/dav2d repo integration.

## Final Acceptance

- `cargo test -p splot-decode tile_payload --locked`: passed after the CDF policy/grid fix.
- `cargo clippy -p splot-decode --all-targets --all-features --locked -- -D warnings`: passed after the CDF policy/grid fix.
- `openspec validate tile-cdf-selection-boundary --strict`: passed.
- `cargo xtask feature-status`: passed.
- `cargo xtask check-feature-status`: passed.
- `cargo xtask check-decoder-support`: passed.
- `cargo xtask check-concurrency-policy`: passed.
- `cargo xtask check-dependency-direction`: passed.
- `cargo xtask ci`: passed after review-agent fixes.

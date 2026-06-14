# Agent Log: tile-payload-decode-boundary

## Orchestrator Plan

- Objective: advance the decoder mission after PR #125 by planning and then
  implementing a PR-sized AV2 tile payload decode boundary.
- Baseline: started from `origin/main` at
  `9d63e97039db6d366b97ae8ab3356515ecb9bdc0`
  (`feat(core): add AV2 symbol decoder foundation (#125)`).
- Local `main` branch is checked out in another worktree, so this thread used a
  detached `origin/main` for planning and will create a feature branch only
  after `openspec validate tile-payload-decode-boundary --strict` passes.
- Baseline gate: `cargo xtask ci` passed on 2026-06-14.
- OpenSpec note: `openspec/project.md` is not present in this repo; project
  rules were read from `openspec/README.md` and `openspec/config.yaml`.

## Subagents

| Agent | Role | Objective | Status |
|---|---|---|---|
| @architect | Planning subagent | Plan scope, crate boundaries, matrix/docs/tests for `tile-payload-decode-boundary`. | complete |
| @spec-reader | Planning sub-subagent | Extract AV2 § 5.20, § 6.19, and § 8.3 requirements from the committed spec mirror. | complete |
| @api-designer | Planning sub-subagent | Propose internal API and diagnostics for the tile-payload boundary. | complete |
| @reference-oracle | Reference subagent | Inspect local-only AVM/dav2d availability and conceptual reference behavior without repo integration. | complete |
| @security-reviewer | Planning/security subagent | Threat-model untrusted tile payload bytes, CDF state, arithmetic, panic, and boundary risks. | complete |
| @encoder-impact-reviewer | Planning subagent | Check that the boundary helps future closed-loop encoder reconstruction without premature dead ends. | complete |

## Local Evidence

- Git status before planning: clean detached `origin/main`.
- `cargo xtask ci`: passed; warnings were existing source-line advisories and
  unmatched license allowance warnings from `cargo-deny`.
- PR #113 Codex review carry-forward checked on 2026-06-14:
  `pullrequestreview-4492663492` reviewed commit `3066f4d85e`. The four
  actionable comments are already present in current `origin/main` through
  `5f7a900 fix(decode): address byte planner review feedback`: unsupported
  prefix precedence before later OBU limits is covered by
  `unsupported_prefix_is_reported_before_later_obu_limit`, initial IVF truncated
  frame-header retry state is covered by
  `frame_cursor_retry_preserves_truncated_initial_frame_header_error`, CI seeds
  `decode_plan_bytes` with flag-prefixed fixture/conformance copies, and
  `DecodeContext` docs now describe byte-consuming raw Annex B/IVF planning.
- AVM local checkout outside the repository: HEAD
  `f6f0b9c8914f38be39a953c0a9aa6a2e4050717c`, clean.
- dav2d local checkout outside the repository: HEAD
  `f4f96cb06bb3cd3f31e29e1f190f1c0e373ab352`, with unrelated untracked
  subproject files.
- Local reference observation, non-executable: both AVM and dav2d split tile
  group metadata/payload slicing from per-tile entropy/block/reconstruction
  state, and update frame CDF state after tile decode. This supports a
  `splot-decode` boundary that proves tile byte ranges first and defers CDF
  selection/copyback to future work.

## Boundary Requirements

- No AVM/dav2d source, snippets, binaries, submodules, dependencies, build
  probes, wrappers, scripts, CI jobs, required `xtask` commands, runtime process
  execution, local absolute paths, or mandatory tests may be introduced.
- Runtime concurrency must follow PR #101: `splot-decode` owns orchestration
  through `DecodeContext` and `splot_parallel::WorkerPool`; `splot-recon`
  remains scheduler-free; no direct Rayon/crossbeam/global pools/ad-hoc threads.
- Full `decode_tile()`, § 8.3 CDF bank ownership, reconstruction, decoded-frame
  hashes, runtime Y4M output, and reference refresh semantics remain out of
  scope for this change unless explicitly supported by later OpenSpec updates.

## Findings And Fixes

- `@spec-reader`: § 5.20.1 derives tile payload spans, `tileSize`, tile row/col,
  MI bounds, `BruTileActive`, `CurrentQIndex = base_q_idx`, then calls
  `init_symbol(tileSize)`, `decode_tile()`, and `exit_symbol()` for non-bridge
  tiles; § 8.3 CDF selection and CDF copyback/averaging must stay unsupported
  until real tile syntax exists. Updated proposal/design/spec to defer
  `exit_symbol()` instead of validating it before symbols are consumed.
- `@reference-oracle`: local AVM/dav2d evidence supports separating payload
  slicing from per-tile entropy/block state and later CDF update. No repo
  integration, wrappers, copied source/prose/constants, scripts, CI, or path
  assumptions should be added.
- `@security-reviewer`: require checked arithmetic before slicing, limit checks
  before retaining tile metadata or symbol handoff, no `exit_symbol()` before
  `decode_tile()`, no CDF mutation, no panics, and explicit AVM/dav2d boundary.
  Added tasks for offset/limit/deferral tests.
- `@encoder-impact-reviewer`: future encoder usefulness requires deterministic
  tile work units with exact source provenance, tile number/order, payload
  offset/length, and selected frame/layer metadata. Added a task for work-unit
  metadata and thread-policy determinism when reachable through `DecodeContext`.
- `@architect`: recommended a minimal-tier `splot-decode` boundary: base-layer
  closed-loop-key, complete intra first tile group, one tile, one tile group,
  bounded payload size, and a borrowed plan type separate from metadata-only
  `DecodeStreamPlan`. Updated proposal/design/spec/tasks to narrow scope to
  that first PR-sized boundary.
- `@api-designer`: recommended crate-private `crates/splot-decode/src/tile_payload.rs`,
  reusing `TileGroupFraming` / `TileFraming` and `SymbolDecoder::with_base_and_config`,
  with no tile plan exports from `lib.rs`; reuse `decode/unsupported-feature`
  and `decode/resource-limit`. Kept bridge as unsupported for this PR despite
  the API note that bridge skips symbol init/exit, because the chosen minimal
  tier is non-bridge only.

## Implementation Evidence

- Added crate-private `splot-decode::tile_payload` boundary types and
  `plan_tile_payload_boundary`, plus split module tests under
  `crates/splot-decode/src/tile_payload/tests.rs` to keep Rust source files
  under the repository line budget.
- Focused test: `cargo test -p splot-decode tile_payload --locked` passed with
  13 tests after review fixes.
- Touched-crate test: `cargo test -p splot-decode --locked` passed with 54
  tests after review fixes.
- Focused clippy: `cargo clippy -p splot-decode --all-targets --all-features
  --locked -- -D warnings` passed after review fixes.

## Mandatory Review Findings

- `@security-reviewer`: found that hostile `TileGridFacts` could provide
  zero-length or inverted MI ranges. Fixed by rejecting non-increasing row/col
  MI boundaries as `InvalidTileGrid` before work-unit retention or symbol
  initialization; added `non_increasing_mi_grid_ranges_are_invalid`.
- `@reviewer`: found four P2 issues: `ByteSpan` end overflow was not checked,
  MI ranges could be non-monotonic, unsupported spec sections were hard-coded to
  `8.3`, and the boundary lacked explicit closed-loop-key / intra-frame facts.
  Fixed with checked span-end arithmetic, non-monotonic MI rejection,
  per-reason spec sections, and `TileFrameFacts` fields for `ObuType` plus
  intra-frame proof. Also found a P3 that task 3.5 overclaimed `DecodeContext`
  coverage; fixed by running the determinism test through
  `DecodeContext::pool().install(...)`.
- `@spec-conformance-reviewer`: found the same hard-coded `8.3` spec-section
  issue for pre-handoff unsupported reasons. Fixed with per-reason sections:
  `5.20.2.1`, `5.20.1`, `6.19.1`, and `7.1` as appropriate.
- `@encoder-impact-reviewer`: found work-unit provenance docs could be lost
  when dispatching individual units because source/layer lived only on the
  plan, and found the `DecodeContext` determinism proof overclaimed. Fixed by
  copying source and selected-layer metadata into every `DecodeTileWorkUnit`
  and by using `DecodeContext::pool().install(...)` in the thread-policy test.
- `@reviewer` follow-up: found residual docs drift after per-reason spec
  sections because OpenSpec and matrices did not include `5.20.2.1`. Fixed by
  updating the OpenSpec scenario, `docs/IMPLEMENTATION-MATRIX.toml`, and
  `docs/DECODER-SUPPORT-MATRIX.toml`, then regenerating
  `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`, and
  `docs/DECODER-SUPPORT-STATUS.md`.
- Follow-up verification: `@reviewer`, `@security-reviewer`,
  `@spec-conformance-reviewer`, and `@encoder-impact-reviewer` all reported
  their prior findings closed.

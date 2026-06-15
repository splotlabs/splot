# Agent Log: decode-tile-payload-input-derivation

Feature ID: `DECODE-TILE-PAYLOAD-INPUT-DERIVATION`

## Orchestrator Plan

- Continue the decoder mission after merged PR #134.
- Preserve the PR #101 concurrency model by keeping all runtime orchestration in
  `DecodeContext` and `splot_parallel::WorkerPool`.
- Preserve PR #113 / PR #114 carry-forward by reusing existing `splot-core`
  parser surfaces and not reintroducing duplicated Annex B/IVF parsing or
  unsupported/limit precedence regressions.
- Plan a narrow crate-private bridge that derives the existing tile-payload
  boundary input from source-backed parser facts and still stops before
  `decode_tile()`, reconstruction, hashes, Y4M, reference refresh, and external
  decoders.
- Validate OpenSpec before creating the implementation branch.

## Planning Agents

| Agent | Role | Objective | Result |
|---|---|---|---|
| `019ec7d1-564c-7360-a2a5-22717df9ac59` | `@architect` | Decide whether tile-payload input derivation is the right next PR-sized slice. | Complete: recommended change id `decode-tile-payload-input-derivation` and Feature ID `DECODE-TILE-PAYLOAD-INPUT-DERIVATION`. Confirmed this is the right next slice if crate-private and plan-only. Warned not to derive from `DecodeStreamPlan` alone because it lacks payload slices and stateful frame facts. |
| `019ec7d1-5924-7c73-b8d1-7efeba0fc505` | `@spec-reader` / `@spec-conformance-reviewer` | Identify exact AV2 sections, parser-derived facts, unsupported paths, and reference-evidence needs. | Complete: cite § 5.2.1, § 5.18.1, § 6.17.1, § 5.18.2, § 5.18.6.1, § 5.18.7.2, § 5.18.7.3, § 6.17.7.2, § 5.19, § 6.18, § 5.20.1, § 6.19.1, § 5.20.2.1, § 8.2.2, § 8.2.4, and § 8.3 as applicable. Flagged that `disable_cdf_update` is read by `FrameHeaderCore` but not exposed yet; the bridge must expose it or remain unsupported rather than hardcoding. |
| `019ec7d1-5bee-7202-a861-f425c5be8fa7` | `@api-designer` / `@encoder-impact-reviewer` | Recommend crate-private API shape and encoder impact. | Complete: keep the bridge crate-private near `tile_payload`, validate planned provenance against borrowed `ObuEnvelope`, split tile boundary lifetimes so returned plans borrow only payload bytes, and expose `FrameHeaderCore::disable_cdf_update`. Keep encoder/recon out of scope. |
| `019ec7d1-5eb7-7a82-beb2-44e96117f783` | `@security-reviewer` / `@performance` | Review hostile-input, bounds, lifetimes, slicing, allocation limits, concurrency, and external-decoder boundary. | Complete: deriving from `DecodeStreamPlan` alone is a blocker. Validate forged parsed inputs, use checked containment and arithmetic, slice only the § 5.20 payload region after complete § 5.19 parsing, enforce `max_tile_count` before retaining tile work, and add raw/IVF, truncation, overflow, limits, deterministic-thread, and fuzz coverage. |
| `019ec7d1-6167-71f2-a76b-409eda76eaaa` | `@reference-oracle` | Decide whether local AVM/dav2d evidence is needed. | Complete: no AVM/dav2d run or source reading is needed for this plan-only bridge. No local reference evidence should be added. |

## Boundary Commitments

- No public tile-payload API.
- No runtime CLI decode success path.
- No `decode_tile()`, block syntax traversal, CDF copyback/averaging mutation,
  reconstruction, hashes, Y4M output, output scheduling, reference refresh, or
  `decode_frame_wrapup()`.
- No `splot-decode -> splot-recon` dependency edge.
- No direct Rayon, crossbeam, global pool, nested pool, ad-hoc thread, or queue
  usage outside `splot_parallel`.
- No AVM/dav2d source, snippets, binaries, submodules, dependencies, build
  probes, wrappers, scripts, CI jobs, required `xtask` commands, or mandatory
  tests.

## Review Agents

| Agent | Role | Result |
|---|---|---|
| `019ec7ed-f0b3-77b2-abf1-736ee872ffd0` | API/concurrency review | No findings. Confirmed the new derivation API remains crate-private, the split lifetimes return plans borrowing only payload bytes, the PR #101 `DecodeContext`/`WorkerPool::install` model is used, and `splot-decode` has no `splot-recon` dependency or direct Rayon/crossbeam/thread/global-pool usage. |
| `019ec7ed-d9c1-7ef0-a7b8-be798d9babef` | Spec/conformance review | Found that accepting caller-supplied `TileGroupStructure` could trust forged public fields, and that the `disable_cdf_update` accepted-path test used only `new_for_test`. Fixed by removing `TileGroupStructure` from `FrameCandidateTileBoundaryInput`, deriving § 5.19 inside the bridge from the same envelope payload using the parser-derived post-frame-header bit position, and adding a fixture-backed `FrameCandidateTileFacts::from_frame_core` accepted-path test. |
| `019ec7ee-4528-73c3-b723-bb29df273463` | Hostile-input/security/performance review | Found that matching envelope metadata was not enough to prove the payload slice came from the planned input bytes, and that `MaxTileCount` should fire before the unsupported multi-tile tier. Fixed by requiring the original input byte slice in the bridge input, checking the envelope payload is the exact slice inside that buffer, and moving the grid tile-count limit before unsupported single-tile gating with a regression test. |
| GitHub Codex review `4493639796` | External PR review | Found two P3 issues after PR #135 opened: the proof ledger named a nonexistent `derived_boundary_rejects_malformed_tile_group_payload_region` test, and the new derived-boundary tests made `crates/splot-decode/src/tile_payload/tests.rs` exceed the 1000-line advisory limit. Fixed by renaming the ledger proof to the existing `derived_boundary_rejects_invalid_locally_parsed_tile_group_structure` test, moving derived-input tests and helpers into `crates/splot-decode/src/tile_payload/derived_tests.rs`, and regenerating decoder/feature status docs. |

## Verification

- `openspec validate decode-tile-payload-input-derivation --strict` passed.
- `cargo test -p splot-core frame_header_core --locked` passed.
- `cargo test -p splot-core reached_shared_tail_consumes_disable_cdf_update --locked` passed.
- `cargo check -p splot-core --locked` passed.
- `cargo check -p splot-decode --locked` passed.
- `cargo test -p splot-decode tile_payload --locked` passed after adding
  source-backed raw Annex B, IVF, malformed metadata, incomplete § 5.19,
  malformed § 5.20, limit, unsupported-path, CDF-update, and thread-policy
  derivation cases.
- After review fixes, `cargo test -p splot-decode tile_payload --locked` passed
  again with 37 cases, including forged envelope-source rejection, local § 5.19
  re-derivation errors, tile-count precedence, and fixture-backed
  `FrameCandidateTileFacts::from_frame_core` CDF propagation.
- `cargo clippy -p splot-core -p splot-decode --all-targets --all-features --locked -- -D warnings` passed.
- `cargo xtask check-feature-status` passed.
- `cargo xtask check-decoder-support` passed.
- `cargo xtask check-dependency-direction` passed.
- `cargo xtask check-concurrency-policy` passed, confirming the PR #101
  context-owned worker-pool model remains enforced.
- `openspec validate --all --no-interactive` passed.
- `cargo xtask check-source-lines` passed after tightening comments in
  `crates/splot-core/src/headers/frame/info.rs` to stay below the existing hard-cap
  allowance instead of increasing it.
- `cargo xtask ci` passed.
- After review fixes, `cargo xtask ci` passed again.
- After GitHub Codex review fixes, `cargo test -p splot-decode tile_payload --locked` passed with 37 cases.
- After GitHub Codex review fixes, `cargo xtask check-source-lines` passed; the moved test files are below the 1000-line advisory limit.
- After GitHub Codex review fixes, `cargo xtask check-feature-status`, `cargo xtask check-decoder-support`, `cargo xtask check-dependency-direction`, and `cargo xtask check-concurrency-policy` passed.
- After GitHub Codex review fixes, `cargo xtask ci` passed.

## Fuzz Coverage Note

- The byte-to-tile-boundary derivation bridge remains crate-private by design.
  The out-of-workspace `fuzz/` crate cannot call it without widening decoder API
  or adding a fuzz-only public surface, which this slice deliberately avoids.
  Coverage for the new bridge is therefore in `splot-decode` unit tests; existing
  `decode_plan_bytes` fuzz coverage continues to exercise the public byte
  planner and CI now keeps fixture bytes intact behind that target's flag byte.

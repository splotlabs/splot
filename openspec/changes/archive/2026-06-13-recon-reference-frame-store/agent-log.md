# Agent Log: recon-reference-frame-store

## Orchestrator

- Objective: add a dependency-free `splot-recon` reference-frame-store runtime
  API for future decoder and encoder closed-loop reuse.
- Feature ID: `RECON-REFERENCE-FRAME-STORE`.
- Branch: `codex/recon-reference-frame-store`.
- Constraints: no AVM/dav2d repo integration, no new dependencies, no new crate
  dependency edges, no byte-consuming decode, no AV2 reference refresh semantics.

## Plan

1. Scaffold and validate OpenSpec artifacts.
2. Use read-only planning agents for architecture, spec citations, and API
   shape.
3. Implement `ReferenceSlot` and `ReferenceFrameStore<F>` in `splot-recon`.
4. Update docs, matrix, generated status, and feature tracking.
5. Run focused checks, full CI, and review agents before PR.

## Agents

| Role | Agent ID | Objective | Status |
|---|---|---|---|
| `@architect` | `019ec281-b165-7281-ae1e-8f54d937b9b4` | Architecture and dependency-boundary plan | complete |
| `@spec-reader` | `019ec281-d2c3-77d0-b834-59073a233fab` | AV2 §7.23 and related citation review | complete |
| `@api-designer` | `019ec281-f40c-7f82-87bc-e54f987d4d9e` | Public API shape and tests | complete |

## Findings

- `@spec-reader`: the runtime store may honestly claim only storage/modeling
  support unless a real decoder supplies § 7.23 refresh facts. `NUM_REF_FRAMES`
  is 16; active `NumRefFrames`, `RefValid`, output scheduling, film grain,
  CDF/motion-vector/segment/global-motion/reference metadata, and long-term
  reference semantics remain future work. The store must not claim AV2 decoder
  conformance, reconstruction, frame hashes, Y4M, or AVM/dav2d proof.
- `@api-designer`: expose typed `ReferenceSlot`, a fixed-capacity reference
  store, immutable `get`/iteration, owned `put` replacement, `take`, `clear`,
  occupancy, and typed errors. Keep capacity caller-provided and keep
  `DecodeLimits` outside `splot-recon`.
- `@architect`: use a single dependency-free `reference.rs` module in
  `splot-recon`; cap capacity at `ReferenceSlot::MAX_SLOTS = 16`; validate
  every public slot operation before indexing; iterate occupied entries in
  ascending slot order; document that occupancy is not AV2 `RefValid`.

## Implementation Notes

- Added `crates/splot-recon/src/reference.rs` with:
	  - `ReferenceSlot::MAX_SLOTS == 16`;
	  - checked `ReferenceSlot::new`;
	  - fixed-capacity generic `ReferenceFrameStore<F>`;
	  - bounds-checked `put`, `get`, `take`, `clear`, `occupied`, `is_empty`,
	    `contains_slot`, and immutable `entries`;
	  - `ReferenceFrameEntry` / `ReferenceFrameEntries` slot-order iteration.
- Exported the API from `crates/splot-recon/src/lib.rs`.
- Added `ReconError` variants for invalid store capacity, invalid slot index,
  and store-capacity slot bounds.
- Added unit tests covering valid construction, zero/excess capacity, slot
  construction bounds, max-capacity edge-slot access, out-of-range access
  preserving seeded state, generic non-output payload storage, replacement,
  occupancy, clearing, and deterministic slot-order iteration.
- Updated decoder roadmap, decoder support matrix, feature matrix, generated
  decoder support status, feature status, and spec coverage.

## Verification

- `openspec validate recon-reference-frame-store --strict`: passed.
- `openspec validate --all --no-interactive`: passed, 15 items.
- `cargo test -p splot-recon --locked`: passed, 32 tests plus doctests.
- `cargo clippy -p splot-recon --all-targets --locked -- -D warnings`:
  passed.
- `cargo xtask check-dependency-direction`: passed.
- `cargo xtask check-feature-status`: passed, 152 features.
- `cargo xtask check-decoder-support`: passed, 22 rows.
- `cargo xtask ci`: passed.

## Review

- `@test-writer` (`019ec28c-8d05-7c61-bb53-36fd47292944`): requested
  max-capacity slot-15 coverage and seeded out-of-bounds mutation-preservation
  coverage. Fixed with focused unit tests. Final review: LGTM.
- `@reviewer` (`019ec28c-2dd5-7b42-9ee7-371b7b306305`): requested removal
  of stale decoder-matrix wording that said reference-frame-store behavior
  remained unimplemented, and requested explicit § 3 citation for
  `NUM_REF_FRAMES`. Fixed in code/docs/matrix/OpenSpec. Final review: LGTM.
- `@documenter` (`019ec28c-ad29-7840-86bf-6d383027304a`): found the same
  stale decoder-support matrix wording. Fixed and regenerated generated status
  docs. Final review: LGTM.
- `@security-reviewer` (`019ec28c-4599-7ad2-a200-2da5ec990c59`): no findings;
  final review confirmed fixed-capacity allocation, pre-index bounds checks,
  no unwraps outside tests, no new dependencies, and no AVM/dav2d integration.
- `@spec-conformance-reviewer` (`019ec28c-5dd9-7121-a5cb-391b06eba4d4`):
  requested separating AV2 § 3 `NUM_REF_FRAMES` citation from § 7.23 storage
  motivation. Fixed in public docs and OpenSpec. Final review: LGTM.
- `@encoder-impact-reviewer` (`019ec28c-7a83-7a80-96b4-7755252dd087`):
  identified that a `DecodedFrame<T>`-specific store would force future
  reference/reconstruction payloads to fabricate output-emission metadata.
  Fixed by making the store payload-generic as `ReferenceFrameStore<F>`.
  Final review: LGTM.

Final acceptance: all local review roles signed off, `cargo xtask ci` passed,
and no AVM/dav2d repository integration or dependency graph change was added.

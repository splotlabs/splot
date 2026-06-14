## Overview

The existing `tile_payload` module models the point where AV2 § 5.20.1 tile
payload framing hands one bounded tile byte slice to AV2 § 8.2 symbol
initialization and then stops before § 5.20.2.1 `decode_tile()`. The module is
crate-private because the caller still has to supply already-derived frame,
tile-grid, source, and CDF-policy facts; exposing that as public API would freeze
an unstable internal contract before the runtime decode driver exists.

This change keeps those types private and adds only a crate-private
`DecodeContext` handoff. The value is architectural rather than algorithmic:
future tile syntax traversal has a single context-owned entry point, and review
can verify that it follows the repository concurrency policy before the code
becomes hot or public.

Because no production path can yet derive `TilePayloadBoundaryInput` from parsed
frame state, the crate-private handoff remains future-facing in non-test builds.
The code therefore uses conditional, reasoned `dead_code` allowances for
non-test builds instead of the stale module comment that previously said the
boundary was waiting to be wired into `DecodeContext`.

## Decisions

### 1. Keep the handoff crate-private

The method belongs on `DecodeContext` but should be `pub(crate)` until the decode
driver can derive its input facts from parsed headers and return public
diagnostics. Returning `TilePayloadBoundaryError` publicly would expose
work-in-progress tile syntax internals, while translating it into `DecodeError`
too early would lose useful internal test coverage. Tests in the same crate can
still prove the context routing and deterministic outputs.

### 2. Reuse the existing tile-payload boundary unchanged

The handoff calls `plan_tile_payload_boundary(input)` inside
`self.pool.install(...)`. This does not change tile-range validation,
limit-ordering, unsupported reason selection, symbol initialization, CDF subset
attachment, or frame-end residual metadata. Any behavior change in those areas
would belong to the tile-payload or tile-CDF feature rows, not this handoff row.

### 3. Encode PR #101 concurrency model directly in the integration point

The only scheduler in this change is the `WorkerPool` already owned by
`DecodeContext`. The code must not import Rayon, crossbeam, `std::thread`,
global pools, bounded queues, or a second worker pool. `splot-recon` remains
untouched and scheduler-free; future reconstruction callers should be invoked by
`splot-decode` from inside this same context-owned orchestration layer.

### 4. AVM/dav2d evidence is explicitly deferred

This change does not decode tile syntax, reconstruct pixels, compute decoded
hashes, write Y4M, or compare output. Local AVM/dav2d evidence is useful for
future decoded-output milestones but would not prove this context handoff. The
portable proof for this PR is self-contained Rust tests plus the dependency and
concurrency gates.

### 5. PR #113 / PR #114 review carry-forward stays documented

The earlier byte-planner review comments from PR #113 were fixed by PR #114 and
are already present on current `main`: unsupported/error precedence,
`IvfFrameCursor` retry behavior, `decode_plan_bytes` fuzz seeds, and
`DecodeContext` docs. This change must not reintroduce raw-byte planner behavior
or stale context docs; the agent log records the verification.

## Risks

- **Public API churn:** avoided by keeping the method and tile payload types
  crate-private.
- **False runtime support signal:** mitigated by docs/matrix wording that the
  CLI still has no runtime decode success path and the boundary still stops at
  unsupported `decode_tile()`.
- **Concurrency bypass:** mitigated by code review and `cargo xtask
  check-concurrency-policy`; tests call the boundary through `DecodeContext`
  across `auto`, `1`, and a fixed positive worker count.
- **Overclaiming AV2 semantics:** mitigated by citing only the existing tile
  payload and symbol-boundary sections plus the non-normative runtime
  concurrency policy. This row does not claim new § 5.20.2 block syntax or §
  7.13 reconstruction behavior.

## Acceptance

- `DecodeContext` owns a crate-private tile-payload boundary method that runs
  inside its single `WorkerPool`.
- Tile-payload worker-pool determinism tests call the new context method and
  preserve existing plan metadata.
- Docs, decoder support matrix, implementation matrix, and OpenSpec specs record
  the handoff without claiming public API or runtime decode success.
- No AVM/dav2d repo integration, no `splot-recon` dependency edge, no new
  dependencies, and no direct concurrency primitives are added.

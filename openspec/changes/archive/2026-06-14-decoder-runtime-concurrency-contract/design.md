## Context

`INFRA-PARALLEL-RUNTIME-POLICY` is already implemented and tracked in
`docs/IMPLEMENTATION-MATRIX.toml`. It added `splot-parallel`, the
`ThreadCount` model, one local `WorkerPool` per encode/decode context, bounded
queues, `splot encode|decode --threads`, and the
`cargo xtask check-concurrency-policy` gate. The decoder roadmap and support
matrix predate that merge in their design language and do not yet state how
future decoder/reconstruction work must consume the model.

Current source state:

- `splot-decode::DecodeContext` owns one `WorkerPool` built from
  `DecodeRuntimeConfig`.
- `splot-cli decode` accepts `--threads auto|N` and still emits the intentional
  unsupported diagnostic without reading input.
- `splot-recon` exposes decoded frame, plane, hash-input, and reference-store
  infrastructure, but has no concurrency dependency and no reconstruction
  algorithms.
- `docs/CONCURRENCY.md` is the canonical concurrency policy.

This change is a design/docs/spec alignment change. It does not add AV2 decode
semantics, parser behavior, reconstruction algorithms, new diagnostics, or
reference-tool integration.

## Goals / Non-Goals

**Goals:**

- Make the merged `INFRA-PARALLEL-RUNTIME-POLICY` mandatory for future decoder
  and reconstruction implementation work.
- Record the crate ownership boundary: `splot-decode` orchestrates work through
  its context-owned pool; `splot-recon` remains pool-agnostic and reusable by a
  future encoder.
- Require deterministic frame hashes, Y4M output, diagnostics, stats, reference
  updates, and other observable artifacts across `--threads 1`, `auto`, and
  fixed thread counts before future runtime decode rows are marked supported.
- Keep bounded queues limited to coarse decode pipeline boundaries.
- Regenerate decoder support status from the matrix and validate OpenSpec.

**Non-Goals:**

- No byte-consuming decode implementation.
- No AV2 reconstruction, transform, prediction, loop filtering, film grain, or
  reference refresh implementation.
- No new concurrency primitive, direct Rayon/crossbeam use outside
  `splot-parallel`, or process-global worker pool.
- No new dependency.
- No AVM/dav2d source, wrapper, runner, script, build probe, test dependency, or
  CI requirement.

## Decisions

1. Keep `docs/CONCURRENCY.md` and `openspec/specs/runtime/spec.md` as the
   primitive-policy source of truth.

   Rationale: PR #101 already reviewed and implemented the primitive set. This
   change should not duplicate or relax it. Decoder docs should point at the
   existing policy and add decoder-specific consequences.

   Alternative considered: add a decoder-only concurrency abstraction. Rejected
   because it would create a second policy surface and weaken the single
   `splot-parallel` enforcement model.

2. Put the orchestration boundary in `splot-decode`, not `splot-recon`.

   Rationale: decode has the bitstream order, limits, diagnostics, output order,
   and CLI runtime config needed to schedule work and commit results
   deterministically. `splot-recon` should stay reusable by future encoder
   closed-loop code and should not own pools, spawn workers, or decide pipeline
   structure.

   Alternative considered: let reconstruction helpers own internal pools for
   transforms or prediction. Rejected because it risks nested pools, hidden
   thread-count behavior, and non-deterministic merge order.

3. Require future parallel decode work to collect local results and commit in a
   stable order.

   Rationale: Rayon can execute tasks in any schedule. Observable outputs must
   follow AV2 bitstream/presentation order and repository-owned emission index
   order, not worker completion order.

   Alternative considered: stream completed tasks directly to output queues.
   Rejected for decoder success artifacts because it would make hashes, Y4M,
   diagnostics, and stats schedule-sensitive unless every queue consumer added
   reordering logic.

4. Keep queues coarse and bounded.

   Rationale: bounded queues are useful for future stage handoffs, but pixel,
   block, row, or transform inner loops should use deterministic data-parallel
   iterators or local buffers. This follows the runtime policy and avoids
   per-item channel overhead in hot decode/recon paths.

   Alternative considered: per-tile or per-row channels as a generic scheduling
   primitive. Rejected because it conflicts with the approved policy and would
   complicate deterministic ordering.

## Risks / Trade-offs

- [Risk] The docs may overclaim current decode capability because a worker pool
  exists. -> Mitigation: support-matrix notes explicitly say no byte-consuming
  decode, reconstruction algorithm, hash digest, Y4M output, or AV2 reference
  refresh behavior is implemented by this change.
- [Risk] Future code may accidentally use direct Rayon APIs outside
  `WorkerPool::install`. -> Mitigation: point decoder docs to
  `cargo xtask check-concurrency-policy` and require it in the new support row.
- [Risk] Deterministic ordering could require buffering completed work. ->
  Mitigation: the contract allows parallel local computation, but requires stable
  commit order for observable artifacts; bounded queues remain available only at
  coarse boundaries.
- [Risk] Keeping `splot-recon` pool-agnostic may require decode-side wrappers
  for parallel loops. -> Mitigation: wrappers live in `splot-decode`, where
  limits, diagnostics, and thread policy already belong.

## Migration Plan

1. Add a `decoder-support` OpenSpec delta for the runtime concurrency contract.
2. Update `docs/DECODER-ROADMAP.md` with the decoder/recon concurrency section.
3. Add a decoder support matrix row tied to `INFRA-PARALLEL-RUNTIME-POLICY` and
   fix stale crate-scaffolding notes.
4. Regenerate `docs/DECODER-SUPPORT-STATUS.md`.
5. Validate with OpenSpec and the decoder/concurrency drift gates.

Rollback is documentation-only: revert the OpenSpec artifacts and docs updates.
No runtime state, data migration, or API behavior changes are introduced.

## Open Questions

- None for this change. Actual task partitioning for tile, transform, loop
  filter, and reference update stages remains future implementation design work
  under this contract.

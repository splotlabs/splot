## Context

Earlier decoder mission slices established the roadmap, diagnostics, decode
limits, output contracts, minimal tier contract, hash CLI contract, and local
reference evidence manifest while explicitly avoiding crate/dependency graph
changes. The maintainer has now approved the crate split, so the next smallest
slice is structural: add the workspace crates and gates that future runtime
decode work can build on.

This change must remain a scaffold. It creates boundaries, not behavior:
`splot decode` continues to emit the existing structured unsupported diagnostic.

## Goals / Non-Goals

**Goals:**

- Add `splot-recon` as the future home for pixel buffers, deterministic hashes,
  reconstruction primitives, and reference-frame storage.
- Add `splot-decode` as the future decoder driver crate.
- Record and enforce approved dependency direction:
  `splot-recon` has no `splot-*` dependencies, and future `splot-decode`
  dependencies may point to `splot-core` and `splot-recon`.
- Keep new crates minimal, documented, linted, and included in workspace checks.
- Update docs/matrices so readers can see the scaffold exists without mistaking
  it for runtime decode support.

**Non-Goals:**

- No public decoded-frame, plane, hash, reconstruction, reference-frame, entropy,
  inverse transform, prediction, loop filter, tile, or output API.
- No `splot-cli` wiring to `splot-decode`.
- No input reads, output writes, deterministic hash computation, Y4M output, or
  AV2 decode support.
- No AVM/dav2d/ffmpeg invocation, source read, wrapper, script, CI job, fixture,
  dependency, or local path.
- No encoder-facing source or encoder research documentation changes.

## Decisions

1. Add empty library crates with crate-level docs only.

   Placeholder public types can look stable before the runtime design exists.
   Empty crates with clear docs make the approved package boundary visible while
   avoiding misleading API commitments.

2. Defer actual Cargo dependencies in `splot-decode` until source uses them.

   The dependency-direction rule allows future `splot-decode -> splot-core` and
   `splot-decode -> splot-recon` edges, but the scaffold crate keeps
   `[dependencies]` empty. This avoids fake marker code and keeps
   unused-dependency tooling meaningful until byte-consuming decode code has real
   imports.

3. Do not wire `splot-cli` to the new crate in this slice.

   CLI behavior is already covered by the unsupported diagnostic contract. Wiring
   the CLI would create an unused or no-op dependency without changing runtime
   behavior.

4. Keep coverage gating focused on `splot-validate`.

   The coverage job deliberately gates only `crates/splot-validate`. New empty
   crates must be added to the existing ignore regex so the scaffold does not
   accidentally join the validator line-coverage threshold.

5. Treat this as infrastructure, not AV2 syntax coverage.

   The scaffold has no AV2 syntax or semantics. It needs no AV2 section citation
   and must not mark any codec decode stage as done.

## Risks / Trade-offs

- Overclaiming support -> Docs and matrices must say scaffold/boundary only and
  preserve the existing unsupported `splot decode` behavior.
- Placeholder API churn -> Empty crates avoid public marker types that would
  later need compatibility handling.
- Tooling drift -> Dependency-direction and coverage regex changes keep CI
  aligned with the expanded workspace.
- Delayed compile-time dependency proof -> The allowed graph is enforced by
  `xtask`; actual dependencies will be added when implementation source uses
  them.

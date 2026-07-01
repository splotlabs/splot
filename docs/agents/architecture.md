# Agent Architecture Notes

Use this file for repository boundaries that agents need while deciding where a
change belongs. `docs/ARCHITECTURE.md`, `docs/CONCURRENCY.md`, and
`docs/ZERO_COPY.md` hold the broader design rationale.

## Crate Responsibilities

- `splot-core`: AV2 bitstream model and parsers. It has no other `splot-*`
  dependency.
- `splot-parallel`: approved Rayon worker-pool and bounded crossbeam queue
  primitives. It has no other `splot-*` dependency.
- `splot-tables`: generated AV2 § 9 tables shared across crates. It has no
  `splot-*` dependency and no external crate dependency.
- `splot-recon`: reconstruction primitives over `splot-core` and `splot-tables`.
- `splot-decode`: decoder diagnostics, stream planning, and minimal hash/Y4M
  runtime over `splot-core`, `splot-parallel`, and `splot-recon`.
- `splot-validate`: parser-driven conformance diagnostics over `splot-core`.
- `splot-encode`: future encoder API, borrowed input views, and current
  encoder-side plumbing over `splot-core`, `splot-parallel`, `splot-recon`, and
  `splot-tables`.
- `splot-cli`: thin binary over the library crates.
- `xtask`: standalone automation.
- `fuzz`: cargo-fuzz target outside the workspace.

## Dependency Direction

The canonical dependency graph is in `AGENTS.md` section 2 and is enforced by:

```bash
cargo xtask check-dependency-direction
```

Do not change the crate dependency graph without maintainer approval.

## Concurrency

Only `splot-parallel` may depend on `rayon`, `rayon-core`, or
`crossbeam-channel`. Other crates use `splot-parallel` APIs.

Rules agents must preserve:

- Use local owned `WorkerPool` instances, not the Rayon global pool.
- Use bounded queues only.
- Keep `splot-core` runtime-free and `splot-validate` single-threaded.
- Preserve deterministic output across thread counts.

Enforcement:

```bash
cargo xtask check-concurrency-policy
```

See [../CONCURRENCY.md](../CONCURRENCY.md) for the complete policy.

## Zero-Copy Media Ownership

Media buffers are view-first. Prefer borrowed `PlaneRef`, `PlaneMut`,
`FrameRef`, and `FrameMut` views; share immutable frames explicitly through
`SharedFrame`.

Rules agents must preserve:

- Do not add `Clone` to media-storage types.
- Do not introduce implicit frame/sample duplication.
- Mark intentional materialization boundaries with a specific
  `splot-copy-ok: <reason>` marker.
- Keep `zerocopy` limited to private fixed-layout byte/wire structs in approved
  crates.

Enforcement:

```bash
cargo xtask check-zero-copy-policy
```

See [../ZERO_COPY.md](../ZERO_COPY.md) for the complete policy.

# Architecture

`splot` is split so AV2 syntax, validation, reconstruction, runtime orchestration,
and CLI presentation stay independently reviewable.

## Dependency Direction

```text
splot-cli ───────┬──> splot-validate ───> splot-core
                 ├──> splot-decode   ───> splot-core, splot-parallel, splot-recon
                 ├──> splot-parallel
                 └──> splot-core
splot-encode ────────> splot-core, splot-parallel, splot-recon
```

Rules:

- `splot-core`, `splot-parallel`, and `splot-tables` depend on no other
  `splot-*` crate.
- `splot-tables` has no external dependencies.
- `splot-recon` depends only on `splot-core` and `splot-tables`.
- `splot-decode` depends only on `splot-core`, `splot-parallel`, and
  `splot-recon`.
- `splot-validate` depends only on `splot-core`.
- `splot-encode` depends only on `splot-core`, `splot-parallel`, and `splot-recon`.
- `splot-cli` has normal edges to core, parallel, decode, and validate.
- Nothing depends on `splot-cli`; its tests use an encoder dev edge, while the
  out-of-workspace fuzz crate has a normal encoder dependency.
- `xtask` is standalone.

Gate: `cargo xtask check-dependency-direction`.

## Crate Roles

- `splot-core`: AV2 syntax model, bit readers/writers, Annex B, IVF, OBU header,
  payload parsers, and typed parse errors.
- `splot-validate`: parser output to structured conformance diagnostics.
- `splot-parallel`: the only approved concurrency runtime surface.
- `splot-tables`: generated AV2 § 9 table crate, dependency-free.
- `splot-recon`: frame/plane storage, reference storage, hashes, output
  serializers, and reconstruction primitives.
- `splot-decode`: byte planning, pipeline orchestration, decode diagnostics, and
  narrow supported-tier hash/raw/Y4M output.
- `splot-encode`: limited packet emitters; no general input pipeline or CLI command.
- `splot-cli`: argument parsing, logging, file IO, and presentation.
- `xtask`: repository automation and gates.

## Concurrency

Only `splot-parallel` may depend on Rayon or `crossbeam-channel`. Work runs
through an owned local `WorkerPool`; the Rayon global pool and `build_global`
are banned. Queues are bounded. `splot-core` and `splot-validate` stay
runtime-free.

Gate: `cargo xtask check-concurrency-policy`.

## Zero-Copy

Media storage is view-first. Algorithms borrow `PlaneRef`, `PlaneMut`,
`FrameRef`, or `FrameMut`; immutable frames are shared explicitly through
`SharedFrame`; frame/plane/workspace storage does not use implicit `Clone`.
Intentional materialization must carry a `splot-copy-ok:` marker.

`zerocopy` is allowed only for private fixed-layout wire views, such as IVF
headers. It is not the media-buffer ownership model and must not parse AV2
bit-level, entropy, or variable-length syntax.

Gate: `cargo xtask check-zero-copy-policy`.

## Unsafe and Errors

`unsafe_code = "forbid"` applies across the workspace. Libraries return typed
errors and do not panic on malformed input. `anyhow` is limited to `splot-cli`
and `xtask`.

# Architecture

`splot` is split so AV2 syntax, validation, reconstruction, runtime orchestration,
and CLI presentation stay independently reviewable.

## Dependency Direction

```text
splot-cli ───────┬──> splot-validate ───> splot-core
                 ├──> splot-decode   ───> splot-core, splot-parallel, splot-recon
                 ├──> splot-encode   ───> splot-core, splot-parallel, splot-recon, splot-decode
                 ├──> splot-parallel
                 └──> splot-core
```

Rules:

- `splot-core`, `splot-parallel`, and `splot-tables` depend on no other
  `splot-*` crate.
- `splot-tables` has no external dependencies.
- `splot-recon` depends only on `splot-core` and `splot-tables`.
- `splot-decode` depends only on `splot-core`, `splot-parallel`, and
  `splot-recon`.
- `splot-validate` depends only on `splot-core`.
- `splot-encode` depends only on `splot-core`, `splot-parallel`, `splot-recon`,
  and `splot-decode` (its `encoder_syntax` facade).
- `splot-cli` has normal edges to core, parallel, decode, validate, and encode.
- Nothing depends on `splot-cli`; the out-of-workspace fuzz crate has a normal
  encoder dependency.
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
- `splot-encode`: encoder API, Y4M input, IVF output; backs `splot encode`.
- `splot-cli`: argument parsing, logging, file IO, and presentation.
- `xtask`: repository automation and gates.

## Encoder

`splot-encode` is a staged pipeline; each stage is a module with its own tests:
source input (`SourceFrame`, `u16` samples at every bit depth, or `y4m`) ->
GOP and reference structure -> rate control (`RateController`) -> frame
coding (partition choice, mode decision, transform and quantization, tile
writer) -> temporal unit and IVF output.

- The tile writer takes CDF rows, contexts, neighbour state and partition rules
  from `splot_decode::encoder_syntax`. The encoder never copies decoder syntax.
- Reconstruction calls `splot-recon` kernels. Tests compare every coded frame
  with `splot-decode` output sample for sample, and an env-gated test
  (`SPLOT_AVM_DECODER`) does the same with AVM.
- Decisions are RD searches: a trial codes a leaf through the tile writer's own
  syntax walk into a rate sink and rolls back with a facade snapshot, so the
  rate is exact and no second syntax walk exists.
- An inter leaf predicts and resolves its motion vector through the decoder's
  own MV stack, resolution and motion compensation; publication fails when the
  decoder would resolve another vector.
- Speed presets are rows of one data table; code reads fields, never preset
  numbers. Hot kernels implement one trait twice (scalar and `std::simd`), and
  property tests compare the two bit for bit.
- Output depends only on input, configuration and thread count.

## Concurrency

Only `splot-parallel` may depend on Rayon or spawn threads. The decoder runs
its frames on the `RowScheduler` (dedicated threads claiming frame rows); the
encoder uses the owned `WorkerPool`. The Rayon global pool is banned.

Gate: `cargo xtask check-concurrency-policy`.

## Zero-Copy

Media storage is view-first. Algorithms borrow `PlaneRef`, `PlaneMut`,
`FrameRef`, or `FrameMut`; immutable frames are shared explicitly through
`SharedFrame`; frame/plane/workspace storage does not use implicit `Clone`.
Intentional materialization must carry a `splot-copy-ok:` marker.
Plane storage is `PlaneBuf`: the first sample is 64-byte aligned and the row
stride (`plane_stride`) is a multiple of 64 bytes, so hosts can wrap decoded
planes without a copy.

`zerocopy` is allowed only for private fixed-layout wire views, such as IVF
headers. It is not the media-buffer ownership model and must not parse AV2
bit-level, entropy, or variable-length syntax.

Gate: `cargo xtask check-zero-copy-policy`.

## Unsafe and Errors

`unsafe_code = "forbid"` applies across the workspace, except reviewed blocks
in `splot-decode`, the `splot-recon` plane allocator and the `splot-capi` C
ABI. Libraries return typed
errors and do not panic on malformed input. `anyhow` is limited to `splot-cli`
and `xtask`.

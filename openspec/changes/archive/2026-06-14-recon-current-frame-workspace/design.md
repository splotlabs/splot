## Context

`splot-recon` currently owns immutable decoded frame/plane models, canonical
hash input serialization and SHA-256 digest computation, Y4M writing for
caller-supplied frames, a generic reference-slot store, and the first square DC
intra prediction primitive. Those APIs are enough to validate finished frames,
but future decoder and encoder paths still need a safe mutable target for
incremental reconstruction work.

The current `splot-decode` runtime context is plan-only and owns the approved
concurrency boundary through `DecodeContext` and `splot_parallel::WorkerPool`.
This change deliberately stays in `splot-recon`: reconstruction storage remains
scheduler-free and pool-agnostic, while future decode orchestration can call
workspace methods from inside `DecodeContext::pool().install(...)`.

## Goals / Non-Goals

**Goals:**

- Add a source-backed mutable current-frame workspace in `splot-recon`.
- Allocate Y/U/V plane storage from `DecodedFrameInfo` using checked arithmetic
  and fallible allocation.
- Provide bounded read/write APIs for rectangular sample regions and square
  prediction blocks.
- Provide edge extraction helpers for future intra prediction callers without
  deciding AV2 block availability from bitstream syntax.
- Provide a square DC prediction write path using the existing
  `predict_intra_dc_square_into` primitive.
- Freeze the workspace into an immutable `DecodedFrame<T>` and prove
  interoperability with hash, Y4M, and reference-store APIs.

**Non-Goals:**

- No `splot-decode -> splot-recon` dependency edge in this PR.
- No `DecodeContext`, CLI, `splot decode`, or `splot-encode` behavior change.
- No runtime decoded-frame hash output, runtime Y4M output, output scheduling,
  reference refresh, film-grain synthesis, loop filtering, tile syntax
  traversal, dequantization, inverse transforms, or residual generation.
- No direct Rayon, crossbeam, global pools, ad-hoc threads, worker pools, or
  bounded queues in `splot-recon`.
- No AVM/dav2d source, dependency, wrapper, script, CI job, required local
  reference command, or mandatory test.

## Decisions

### Add A `CurrentFrameWorkspace<T>` In `splot-recon`

Add a public workspace type that owns mutable `Vec<T>` plane buffers plus the
existing `DecodedFrameInfo` metadata. Construction derives plane storage and
visible rectangles from the same geometry rules as `DecodedFrame<T>`:

- Y storage is the coded luma size.
- Y visible rect is the luma visible crop from `DecodedFrameInfo`.
- Non-monochrome chroma storage dimensions come from the coded luma size shifted
  by `PixelFormat` subsampling.
- Non-monochrome chroma visible rectangles come from the luma visible crop
  shifted by `PixelFormat` subsampling.
- Monochrome workspaces allocate only Y.

The workspace is mutable by construction but freezes through existing
`Plane::from_vec`, `FramePlanes::new`, and `DecodedFrame::try_new`. That keeps
immutable output validation centralized in the existing model.

Alternative considered: make `Plane<T>` mutable directly. That would weaken the
immutable decoded-output contract and complicate hash/Y4M/reference guarantees.
A separate workspace keeps mutable reconstruction state and immutable output
state distinct.

### Use Fallible Allocation And Typed `ReconError`

Workspace construction must compute required sample counts and byte counts with
checked arithmetic before allocation. It should use `try_reserve_exact` and then
initialize samples to a caller-provided fill value.

Add targeted `ReconError` variants for workspace allocation failure,
workspace plane absence, rectangle bounds, sample write length mismatch, and
block shape mismatch rather than panicking or reusing unrelated errors.

Alternative considered: allocate through `vec![fill; len]` directly. That is
compact but does not let the API report allocation failure as a typed
reconstruction error.

### Keep Read/Write Surfaces Rectangular And Plane-Scoped

Expose explicit plane-scoped methods:

- immutable complete backing samples for inspection and tests;
- bounded mutation through validated fill, rectangle, and square-block writers,
  without exposing a public mutable slice that would bypass bit-depth checks;
- bounded visible/coded rectangle reads and writes;
- row-wise rectangle copy to avoid per-pixel heap allocation;
- square block write helpers that validate stride and shape before copying.

Coordinates are local to each plane, not luma-global, except for helper APIs
whose names explicitly say they derive chroma coordinates from luma geometry.

Alternative considered: expose a high-level block-grid API immediately. That
would require decisions about transform-block addressing, chroma plane residual
geometry, and `BlockDecoded` semantics that belong with later tile syntax.

### Support Edge Extraction Without Deciding Availability

Provide helpers that can read left and above edge samples adjacent to a
plane-local rectangle when the requested edge lies inside already allocated
workspace storage. If an edge is out of bounds, return `None` for that edge or
a typed bounds error depending on the API shape chosen during implementation.
The workspace must not decide AV2 `count_top_right_avail`,
`count_bottom_left_avail`, `BlockDecoded`, superblock boundary, tile boundary,
or palette/CfL availability; those are future tile/block-syntax responsibilities.

### Keep Square DC Prediction As A Convenience Wrapper

Add a workspace method that calls the existing square DC primitive and writes
the predicted block into the selected plane. This proves the workspace is useful
for reconstruction work while keeping the underlying prediction math in the
already supported `RECON-INTRA-DC-SQUARE-PREDICTION` primitive.

Rectangular DC prediction remains out of scope because the both-edge case needs
the full `resolve_divisor` / `Div_Lut` path from AV2 §7.13.3.22.

### Preserve The PR #101 Concurrency Boundary

The workspace type must not import `splot_parallel`, Rayon, crossbeam, or any
threading primitive. Future decoder or encoder callers may partition work above
the workspace and call into it from the context-owned worker pool, but
`splot-recon` stays deterministic and scheduler-free.

## Risks / Trade-offs

- **Workspace API grows into a decoder too early** -> Keep the API plane/sample
  oriented and explicitly exclude tile symbol parsing, transform syntax,
  residuals, output scheduling, and reference refresh.
- **Mutable output breaks hash/Y4M/reference guarantees** -> Freeze into
  `DecodedFrame<T>` before using immutable output APIs; do not expose mutable
  access after freezing.
- **Coordinate semantics become ambiguous** -> Name plane-local APIs clearly and
  keep luma-to-chroma derivation inside construction/finalization.
- **Allocation from hostile dimensions later bypasses decode limits** -> This
  PR lives in `splot-recon` and reports typed `ReconError`; future
  byte-consuming decode code must still charge `DecodeLimits` before calling
  workspace allocation.
- **Spec overclaim** -> Track the feature as workspace infrastructure, not as
  full intra reconstruction, runtime decode output, or AV2 decoder conformance.

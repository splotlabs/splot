# Design: zero-copy-media-policy

## Context

`splot-recon` already holds the first media-buffer types (`Plane<T>`,
`FramePlanes<T>`, `DecodedFrame<T>`, `CurrentFrameWorkspace<T>`,
`ReferenceFrameStore<F>`). Several derive `Clone`, which clones the backing
`Vec<T>` sample storage. That was early scaffold convenience, not the intended
ownership model: future decode/reconstruction/encoder code must default to
borrowing views and never accidentally duplicate frames or planes.

This change locks in the ownership model with three layers: a written policy
(`docs/ZERO_COPY.md`), view-first APIs in `splot-recon`, and a deterministic
`xtask` gate. `zerocopy` is a separate, narrow tool for fixed-layout wire structs
and is explicitly not the ownership model.

## Data model / API

### View types (`splot-recon`)

Borrow existing storage; construction validates geometry (stride, visible rect,
length) and never allocates or copies samples.

```rust
pub struct PlaneRef<'a, T> { samples: &'a [T], stride_samples: usize, visible_rect: PlaneRect }
pub struct PlaneMut<'a, T> { samples: &'a mut [T], stride_samples: usize, visible_rect: PlaneRect }
pub struct FrameRef<'a, T> { info: DecodedFrameInfo, y: PlaneRef<'a, T>, u: Option<PlaneRef<'a, T>>, v: Option<PlaneRef<'a, T>> }
pub struct FrameMut<'a, T> { /* mirrors FrameRef with PlaneMut planes */ }
```

`PlaneMut`/`FrameMut` require exclusive `&mut` storage and expose
stride/visible-rect-preserving row iterators. Owned `Plane`/`DecodedFrame`/
`CurrentFrameWorkspace` gain `.as_ref()`/`.as_mut()`-style accessors that hand out
these views without copying.

### Shared frame handle

```rust
pub struct SharedFrame<T> { inner: Arc<DecodedFrame<T>> }
```

`new` wraps an owned frame; `share()` is the only way to get a second handle
(`Arc::clone`, visible in review); `get()` borrows the frame. `SharedFrame` does
**not** derive `Clone` (so sharing is always the explicit `.share()`), exposes no
mutable access to storage, and never uses `make_mut`. This is the first `Arc` in
`splot-recon`.

### Clone removal

Remove `Clone` from `Plane`, `FramePlanes`, `DecodedFrame`,
`ReferenceFrameStore`, `CurrentFrameWorkspace`, `CurrentFramePlane`. Keep
`Debug`/`Eq`/`PartialEq` (comparison, not duplication). Keep `Clone`/`Copy` on
small metadata (`DecodedFrameInfo`, `PlaneSize`, `PlaneRect`, `PlaneId`,
`BitDepth`, `PixelFormat`, `ReferenceSlot`, `OutputIndex`) and on borrow iterators
(`VisibleRows`, `WorkspaceRectRows`, `ReferenceFrameEntry/Entries`).
`ReferenceFrameStore` does not require `F: Clone`. Reference stores move/share
handles; they never clone payloads.

`CurrentFrameIntraEdges` retains `Clone`: it owns only bounded per-block edge
scratch (≤ block dimension), not frame storage, and is analogous to the borrowed
`IntraDcEdges` inputs — documented as a deliberate retained `Clone`.

### Copy markers

Every intentional copy in `splot-recon` carries a `splot-copy-ok:` marker naming
the boundary: Y4M/hash sample→byte serialization, workspace sample writes, bounded
intra edge-scratch materialization, and test I/O sinks.

## Gate design (`xtask check-zero-copy-policy`)

Mirrors `xtask/src/concurrency_policy.rs`: a pure `evaluate_zero_copy_policy(crates,
sources) -> Vec<String>` plus an IO `check_zero_copy_policy(root)` wrapper, with
synthetic-fixture unit tests and a real-repo-passes test. Deterministic and
line-based.

**Dependency checks (all manifests):** `zerocopy` may be a direct dependency only
of approved crates (`splot-core`; `splot-recon` only with a documented raw-sample
view); it must flow through the workspace dep. Banned alternative byte/transmute
crates (`bytes`, `bytemuck`, `safe-transmute`, `rkyv`, `memmap2`, `smallvec`,
`arrayvec`) are rejected.

**Source checks:**

- Media-name `Clone` derives/impls, suspicious `.clone()` on media-named bindings
  (matched on the identifier immediately before `.clone()`), `Arc/Rc::make_mut`,
  `unsafe`/`transmute`/`from_raw_parts(_mut)`, and unmarked `read_from_bytes` are
  scanned across `splot-recon`/`splot-decode`/`splot-encode`/`splot-core` src.
- Bulk sample-copy patterns (`.to_vec()`, `Vec::from(&`, `extend_from_slice`,
  `copy_from_slice`, `clone_from_slice`) are scanned in `splot-recon/src` only —
  the only crate that owns decoded sample buffers today. `splot-decode` handles
  compressed bytes + diagnostics and delegates the frame model to `splot-recon`;
  `splot-encode` is a stub. Scanning their compressed-byte/scalar code for these
  patterns would over-flag the ordinary scalar parser code the policy says to
  leave alone. The scan widens to those modules when they grow real sample
  buffers (a one-line scope addition), tracked in `docs/ZERO_COPY.md`.
- `include!` is scanned in `splot-recon`/`splot-decode`/`splot-encode` (not
  `splot-core`, which legitimately `include!`s test files under `src/write/`).

A flagged copy passes only with a nearby specific `splot-copy-ok: <reason>` marker
(same line or within the preceding two lines). Vague markers (`splot-copy-ok`
alone, `temporary`, `fix`, `needed`, `convenience`, `TODO`) are rejected.
Comment-only lines are skipped for the banned-token scan so prose naming a banned
construct is not flagged (matching the concurrency gate).

## Spec mapping

None. This is non-normative codec-runtime infrastructure; it adds requirements to
the `runtime` capability and no AV2 conformance coverage.
TODO(spec: INFRA-ZERO-COPY-MEDIA-POLICY): future decoded-sample-bearing modules in
`splot-decode`/`splot-encode` extend the bulk-copy scan scope.

## Diagnostics

None (no validator diagnostics). The gate emits `xtask` violation strings with the
file:line, the matched pattern, and the marker hint.

## Tests

- recon: view construction without allocation; `*Mut` exclusivity + row access;
  `FrameRef`/`FrameMut` plane-presence/geometry validation; owned-type borrowed
  views without copy; `SharedFrame::share()` pointer identity; reference-store
  move/share without `T: Clone`.
- xtask: accept/reject fixtures for every rule (media `Clone`, suspicious
  `.clone()`, unmarked vs. marked sample copy, vague marker rejected, `make_mut`,
  `unsafe`/`transmute`/`from_raw_parts`, `read_from_bytes`, `include!`, banned dep,
  wrong-crate `zerocopy`) plus a real-repo-passes test.
- IVF wire parsing + invalid magic/version/length/fourcc + preserved error
  behavior — only if `zerocopy` is actually wired.

## Alternatives considered

- **`zerocopy` as the frame ownership model.** Rejected: `zerocopy` is for
  fixed-layout byte/wire views; it does not prevent `frame.clone()`, `.to_vec()`,
  or accidental reference copies. The ownership model is `splot`'s own view/share
  types plus the gate.
- **Scanning every crate for bulk-copy patterns.** Rejected: `splot-decode`/
  `splot-encode`/`splot-core` copy compressed bytes and scalar metadata, not
  decoded samples; broad scanning would over-flag ordinary code and churn files
  shared with the concurrent decoder/writer streams. Scoped to `splot-recon`.
- **`include!` scan over all crates.** Rejected: `splot-core` legitimately
  `include!`s test modules under `src/write/`; scoped to the media crates.

## Risks

- Spec ambiguity: none (no AV2 behavior).
- Performance: views add no copies; `SharedFrame` shares via `Arc::clone`. No hot
  path exists yet.
- Compatibility: dropping `Clone` from media types is a pre-alpha public API
  change; verified no in-tree caller clones these types except one test, which is
  rewritten.
- Maintenance: the gate is line-based defense-in-depth; the dependency-direction
  and concurrency gates plus code review remain the backstop. Scope decisions are
  documented so future contributors widen the scan deliberately.

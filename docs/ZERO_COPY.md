# Zero-copy media-buffer policy

Feature ID: `INFRA-ZERO-COPY-MEDIA-POLICY`.

This document is the canonical media-buffer **ownership and copy policy** for
`splot`. It governs how future decoder, reconstruction, and encoder code owns and
moves frames, planes, reference-frame storage, lookahead retention, and
pixel/sample storage. It is enforced by `cargo xtask check-zero-copy-policy` (run
inside `cargo xtask ci` and in CI) together with `splot`'s own view/share APIs in
`splot-recon`. It cross-references [CONCURRENCY.md](./CONCURRENCY.md) and
[ARCHITECTURE.md](./ARCHITECTURE.md).

This is non-normative codec-runtime infrastructure. It adds **no** AV2 conformance
coverage and marks no decoder/encoder algorithmic stage implemented.

## 1. Why this exists (and what it is not)

Media buffers are large. A frame is megabytes of samples; reference stores hold
several at once; lookahead retains more. The cost of accidentally duplicating one
is invisible in a diff: `frame.clone()`, `samples.to_vec()`, a stray
`#[derive(Clone)]` on a frame type, `copy_from_slice` into a fresh `Vec`, or
`Arc::make_mut` on shared frame storage all read as ordinary Rust and all copy a
whole frame. Once real hot paths exist, those copies compound.

This policy makes the ownership model **explicit and enforced before** that code
is written, so the default is borrowing and every genuine duplication is a
deliberate, named, reviewable boundary.

What this policy **is not**:

- It is **not** `zerocopy`. The [`zerocopy`](#9-zerocopy--allowed-surface-and-the-ivf-pattern)
  crate is a narrow tool for fixed-layout byte/wire view structs (e.g. the IVF
  header). It does not prevent `frame.clone()`, `.to_vec()`, `copy_from_slice`, or
  accidental reference copies, and it is **not** the frame-buffer ownership model.
- It does **not** override thread-safety or determinism. Parallel code still uses
  `splot-parallel` with disjoint mutable views (see [§ 10](#10-determinism-and-parallel-ownership)).

## 2. The four terms (and the copy-ok marker)

- **Borrow** — read a media buffer through a shared view (`&[T]`, `PlaneRef`,
  `FrameRef`) without taking ownership or copying. The default for reads.
- **Mutable view** — write a media buffer through an exclusive view (`&mut [T]`,
  `PlaneMut`, `FrameMut`) that borrows existing storage. The default for writes.
  Parallel writers split one buffer into **disjoint** mutable regions.
- **Share** — hand a second owner an immutable frame **without copying pixels**,
  via an explicit `SharedFrame::share()` (an `Arc::clone`). Visible in review;
  never an implicit `Clone`.
- **Materialize** — deliberately allocate and copy at a named boundary because no
  borrowed view can satisfy the lifetime or layout (see [§ 6](#6-allowed-copies-materialization-boundaries)).
  Spelled `materialize_copy_for_<reason>`, never a generic `clone`.
- **Copy-ok marker** — a `splot-copy-ok: <reason>` comment that records why a
  specific copy is an intentional materialization boundary (see [§ 7](#7-the-splot-copy-ok-marker-grammar)).

## 3. The default ownership model

Future decode/reconstruction/encoder code MUST follow this model. None of it is
implemented yet; this is the contract those features are written against.

- **Compressed bitstreams are borrowed as `&[u8]`.** Parsers and planners read the
  input slice; they do not own or copy the compressed payload.
- **Decoded/current frames are reconstructed in owned workspaces.** A
  `CurrentFrameWorkspace` owns its plane storage and is filled in place; it freezes
  into an immutable `DecodedFrame` by **moving** its buffers, never copying them.
- **Algorithms operate on borrowed plane/frame views.** Prediction, transform,
  filtering, and analysis take `PlaneRef`/`PlaneMut`/`FrameRef`/`FrameMut`, not
  owned planes/frames.
- **Parallel work splits mutable storage into disjoint regions.** Per-tile /
  per-frame writers take disjoint `&mut` sub-regions; results merge back in a
  stable (index/presentation) order (see [CONCURRENCY.md § 6](./CONCURRENCY.md)).
- **Reference updates move or share handles, never clone pixels.** A reference
  store moves an owned frame in, or stores a `SharedFrame`; it never requires
  `T: Clone` and never duplicates payloads.
- **Encoder input accepts borrowed views when lifetimes allow.** The encoder reads
  caller-owned input planes through views; it copies only when it must retain the
  input past the caller's lifetime.
- **Lookahead retention has exactly one explicit materialization point.** If the
  caller cannot provide a buffer that outlives the lookahead window, the lookahead
  performs a single, marked materialization; it does not copy on every access.
- **Output serialization may copy** (Y4M rows, frame-hash input, packet bytes):
  this is an external byte stream the writer owns, a marked copy boundary.
- **Pixel-format / bit-depth conversion, padding, edge extension, and film-grain
  synthesis are explicit materialization boundaries.** They produce new owned
  storage by definition and are marked as such.

## 4. Banned patterns

These never appear on media-frame / plane / reference / sample storage; the gate
rejects them:

- **Large-media `Clone`** — a `#[derive(Clone)]` or `impl Clone` on a frame, plane,
  frame-planes set, reference store, workspace, or sample/pixel buffer type.
- **Hidden sample copies** — `samples.to_vec()`, `Vec::from(&samples)`,
  `extend_from_slice`, `copy_from_slice`, or `clone_from_slice` on a sample buffer
  without a [marker](#7-the-splot-copy-ok-marker-grammar).
- **Clone-on-write on frame storage** — `Arc::make_mut` / `Rc::make_mut` (or any
  copy-on-write) on shared frame storage.
- **`read_from_bytes` dressed up as a borrow** — a `zerocopy`
  `*::read_from_bytes(…)` that silently copies where a `ref_from_*` borrow was
  intended, without a marker stating it is a tiny intentional wire-header copy.
- **Raw wire structs in public APIs** — returning or exposing a `zerocopy` wire
  struct across a public boundary instead of converting to a validated domain type.
- **`unsafe` transmute/cast views** — `unsafe`, `transmute`, or
  `from_raw_parts(_mut)` to reinterpret bytes as samples (also forbidden workspace
  wide by `unsafe_code = "forbid"`).

## 5. Default model in one line per crate

- `splot-core` — compressed bytes borrowed (`&[u8]`); no decoded-sample storage.
- `splot-recon` — owns decoded/current-frame storage; hands out views; shares via
  `SharedFrame`; never clones sample buffers.
- `splot-decode` — borrows compressed bytes, plans, and (future) delegates the
  frame model to `splot-recon`; owns no decoded samples today.
- `splot-encode` — (future) borrows input views; one marked lookahead
  materialization point if retention is required.

## 6. Allowed copies (materialization boundaries)

A copy is allowed only at a real boundary, and only with a specific marker:

| Boundary | Why a copy is unavoidable | Example marker reason |
|---|---|---|
| Output serialization | external byte stream the writer owns | `serialize decoded output; Y4M writer owns output bytes` |
| Frame-hash input | deterministic byte serialization for hashing | `serialize decoded samples into hash-input buffer` |
| Workspace sample write | caller-provided samples written into owned plane storage | `write samples into owned current-frame workspace plane` |
| Intra edge scratch | bounded per-block neighbor materialization (≤ block dimension) | `materialize bounded above-edge scratch for intra prediction` |
| Lookahead retention | caller buffer does not outlive the lookahead window | `materialize external encoder input; lookahead retains frame after caller returns` |
| Format/depth/padding/grain | produces new owned storage by definition | `materialize converted-bit-depth output plane` |
| Test fixtures / sinks | test setup builds or accumulates owned bytes | `test fixture construction only` |

## 7. The `splot-copy-ok:` marker grammar

A marker must sit on the **same line** as the copy or **within the preceding two
lines**, and must name the specific boundary that requires materialization:

```rust
// splot-copy-ok: serialize decoded output; Y4M writer owns output bytes
writer.write_all(row.as_bytes())?;

// splot-copy-ok: materialize external encoder input; lookahead retains frame after caller returns
let retained = InputFrame::copy_from_view(input)?;
```

The gate **rejects vague markers** — `splot-copy-ok` alone (no reason), or a reason
that is only `temporary`, `fix`, `needed`, `convenience`, or `TODO`. It also
rejects an unmarked `frame.clone()` / `make_mut` / `.to_vec()` on media. The point
is that a reviewer reading the marker can tell, without context, exactly which
boundary forced the copy.

Because the window is two lines, one marker can cover more than one copy within
it; keep each marker directly above the single copy it explains so the reason
always names the right boundary.

## 8. What the gate scans

`cargo xtask check-zero-copy-policy` is deterministic, line-based defense-in-depth.
It is **not** so broad that ordinary scalar code needs markers. Like the
concurrency gate, it skips comment-only lines so prose naming a banned construct is
not flagged. Scope is deliberate:

**Dependency checks (all manifests).** `zerocopy` may be a direct dependency only
of approved crates (`splot-core` by default; `splot-recon` only with a documented
raw-sample view; never `splot-decode`/`splot-encode`/`splot-validate`/`splot-cli`/
`splot-parallel`) and must be inherited via the workspace dependency
(`zerocopy.workspace = true`) — a local version/feature pin in an approved crate is
flagged. Banned alternative byte/transmute crates (`bytes`, `bytemuck`,
`safe-transmute`, `rkyv`, `memmap2`, `smallvec`, `arrayvec`) are rejected.

**Source checks across `splot-recon` / `splot-decode` / `splot-encode` /
`splot-core` src:**

- `Clone` derives/impls on large-media type names (`Plane`, `Frame`,
  `FramePlanes`, `DecodedFrame`, `CurrentFrame*`, `ReferenceFrame*`, `SharedFrame`,
  `FrameStore`, `LookaheadFrame`, `FrameBuffer`, `SampleBuffer`, `PixelBuffer`,
  `Workspace`, `Reconstruction`), but **not** small-metadata names (`PlaneSize`,
  `PlaneRect`, `PlaneId`, `DecodedFrameInfo`, `BitDepth`, `PixelFormat`,
  `ReferenceSlot`, `OutputIndex`). The derive scan is block-aware, so a
  rustfmt-wrapped multi-line `#[derive(Clone, …)]` is still caught.
- `.clone()` on a suspiciously-named binding (`frame`, `ref_frame`, `reference`,
  `plane`, `samples`, `pixels`, `buffer`, `workspace`, `lookahead`, `current`,
  `decoded`, `recon`), matched on the identifier immediately before `.clone()`.
- `Arc::make_mut` / `Rc::make_mut`, always.
- `unsafe` / `transmute` / `from_raw_parts(_mut)`, always.
- `*::read_from_bytes(` unless marked a tiny intentional wire-header copy.
- In `splot-core`/`splot-recon`, a **fully public** type that derives `zerocopy`
  layout traits (`FromBytes`/`TryFromBytes`/`IntoBytes`/`KnownLayout`/`Immutable`/
  `Unaligned`) — wire-view structs must stay private (never a public API).

Copy needles tolerate whitespace before `(` (so `samples.to_vec ()` cannot evade
the gate by formatting), and a `splot-copy-ok:` marker is honored only inside a
line comment (a string literal containing the token is not a marker).

**Source checks across `splot-recon` / `splot-decode` / `splot-encode` src:**

- `include!` bypasses (which could hide a copy in an unscanned file). `splot-core`
  is excluded here because it legitimately `include!`s test modules under
  `src/write/`.

**Sample-copy checks in `splot-recon/src` only:**

- `.to_vec()` / `Vec::from(&` / `extend_from_slice` / `copy_from_slice` /
  `clone_from_slice` without a nearby marker.

  `splot-recon` is the only crate that owns decoded sample buffers today.
  `splot-decode` handles compressed bytes and diagnostics and delegates the frame
  model to `splot-recon`; `splot-encode` is a stub. Scanning their compressed-byte
  and scalar code for these bulk-copy patterns would over-flag the ordinary scalar
  parser/test code this policy explicitly leaves alone, and would churn files
  shared with the concurrent decoder/writer work.
  TODO(spec: INFRA-ZERO-COPY-MEDIA-POLICY): widen the sample-copy scan to
  `splot-decode`/`splot-encode` modules when they grow owned decoded-sample
  buffers.

Tests are scanned too; a fixture or sink copy passes with a specific marker such
as `splot-copy-ok: test fixture construction only`.

A diagnostic is actionable — file:line, the matched pattern, and the marker hint:

```text
zero-copy policy violation: crates/splot-recon/src/frame.rs:123
  suspicious media copy: `.to_vec()`
  add a specific `splot-copy-ok: <reason>` marker only if this is an intentional materialization boundary
```

## 9. `zerocopy` — allowed surface and the IVF pattern

`zerocopy` is allowed only for **private fixed-layout byte/wire view structs**, via
the workspace dependency shape:

```toml
zerocopy = { version = "0.8", default-features = false, features = ["derive"] }
```

**Allowed:** private `#[repr(C)]` / `#[repr(C, packed)]` wire structs;
byteorder-aware wrappers (`zerocopy::byteorder::little_endian::{U16, U32, U64}`);
the layout derives (`FromBytes`, `TryFromBytes`, `IntoBytes`, `KnownLayout`,
`Immutable`, `Unaligned`); borrowed views (`ref_from_bytes` / `ref_from_prefix` /
`Ref::from_*`); and immediate conversion from a wire struct into a **validated
domain type**.

**Banned / discouraged:** any public-API use; AV2 bit-level / LEB128 / entropy /
variable-length parsing; the owned frame/plane/reference model; `read_from_bytes`
unless marked a tiny intentional copy; and hand-written `unsafe` trait impls (always
derive — `unsafe` is forbidden workspace-wide).

Preferred IVF pattern — parse by borrowing, then validate into a strong type:

```rust,ignore
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout, Unaligned};
use zerocopy::byteorder::little_endian::{U16, U32, U64};

#[repr(C)]
#[derive(FromBytes, IntoBytes, KnownLayout, Immutable, Unaligned)]
struct IvfFileHeaderWire {
    magic: [u8; 4], version: U16, header_len: U16, fourcc: [u8; 4],
    width: U16, height: U16, timebase_den: U32, timebase_num: U32,
    frame_count: U32, unused: U32,
}

let (wire, payload) = IvfFileHeaderWire::ref_from_prefix(bytes)
    .map_err(|_| Error::truncated_ivf_header(offset))?;
let header = IvfHeader::try_from_wire(wire, offset)?; // validated domain type
// Never return `*Wire` from a public function.
```

> Status of the IVF use site: `zerocopy` **is in use** — wired through the private
> `IvfFileHeaderWire` in `crates/splot-core/src/ivf.rs`, which is borrowed via
> `ref_from_prefix` and validated into the public `IvfHeader` while preserving all
> existing IVF error behavior. The governing rule above still holds: `zerocopy` is
> added only for a real fixed-layout use site, never unused, and only in the
> approved crates. See the implementation matrix row
> `INFRA-ZERO-COPY-MEDIA-POLICY` and
> [`docs/references/THIRD-PARTY-NOTICES.md`](./references/THIRD-PARTY-NOTICES.md) §12.

## 10. Determinism and parallel ownership

Zero-copy and concurrency are complementary, not in tension. Parallel media work
still follows [CONCURRENCY.md](./CONCURRENCY.md):

- Parallelism runs through `splot-parallel`'s `WorkerPool` (CONCURRENCY.md § 2.1,
  § 5.1) — never a direct `rayon` dependency or the global pool.
- Per-frame / per-tile work writes into **disjoint** mutable regions or local
  buffers and merges back in a stable index/presentation order (CONCURRENCY.md
  § 6). Disjoint `&mut` sub-views are exactly the mutable-view default in [§ 2](#2-the-four-terms-and-the-copy-ok-marker).
- Output (packets, decoded-frame hashes, diagnostics) is committed in
  presentation/bitstream order, not completion order (CONCURRENCY.md § 6).

Zero-copy does not introduce shared mutable state: views are either shared and
immutable or exclusive and mutable, which is what makes disjoint parallel writes
sound.

## 11. Enforcement

`cargo xtask check-zero-copy-policy` runs in `cargo xtask ci` and in CI, alongside
[`check-dependency-direction`](./ARCHITECTURE.md) and
[`check-concurrency-policy`](./CONCURRENCY.md). The gate is line-based
defense-in-depth: it does not resolve multi-hop re-exports or type inference, so
the dependency-direction gate, the `unsafe_code = "forbid"` lint, and code review
remain the backstop. The review checklist lives in
[CODE_REVIEW.md](./CODE_REVIEW.md).

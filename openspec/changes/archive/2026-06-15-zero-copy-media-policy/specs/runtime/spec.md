# runtime delta: zero-copy-media-policy

Adds the media-buffer ownership and copy policy to the `runtime` capability,
sibling to the concurrency-runtime policy. Non-normative codec-runtime
infrastructure: it adds no AV2 conformance coverage. Tracked by
`INFRA-ZERO-COPY-MEDIA-POLICY`.

## ADDED Requirements

### Requirement: view-first media-buffer ownership

Algorithms that read or write decoded media SHALL operate on borrowed plane and
frame views, not owned copies. `splot-recon` SHALL provide `PlaneRef`/`PlaneMut`
and `FrameRef`/`FrameMut` view types that borrow existing storage, validate
stride/visible-rect/length on construction, and never allocate or copy samples.
Owned media-storage types SHALL expose these views without copying their backing
storage. Tracked by `INFRA-ZERO-COPY-MEDIA-POLICY`.

#### Scenario: borrowing a plane does not allocate

- **WHEN** a `PlaneRef` or `PlaneMut` is constructed over existing sample storage
- **THEN** construction validates geometry and borrows the storage without
  allocating or copying any samples

#### Scenario: invalid view geometry is rejected

- **WHEN** a view is constructed with a stride, visible rect, or length that does
  not fit the backing storage
- **THEN** a typed `ReconError` (never a panic) is returned

### Requirement: no clone-on-write of media storage

Frame, plane, reference-frame, workspace, and pixel/sample storage SHALL NOT
derive or implement `Clone`, and SHALL NOT use `Arc::make_mut` or `Rc::make_mut`.
Any genuine duplication SHALL be an explicit, documented, tested
`materialize_copy_for_<reason>` rather than a generic clone. Small metadata types
(sizes, rectangles, identifiers, bit depth, pixel format) MAY remain `Copy`/`Clone`.
Tracked by `INFRA-ZERO-COPY-MEDIA-POLICY`.

#### Scenario: a frame type gains a Clone derive

- **WHEN** a media-storage type (frame, plane, reference store, workspace, sample
  buffer) is given a `Clone` derive/impl or a `make_mut` call
- **THEN** `cargo xtask check-zero-copy-policy` fails with the offending location

### Requirement: explicit frame sharing via a share handle

Sharing an immutable decoded frame without copying its pixels SHALL go through an
explicit `SharedFrame` handle whose only sharing operation is a visible `.share()`
(an `Arc::clone`). `SharedFrame` SHALL NOT derive `Clone`, expose mutable access
to its storage, or use `make_mut`. Reference-frame stores SHALL move or share
handles and SHALL NOT require `T: Clone`. Tracked by
`INFRA-ZERO-COPY-MEDIA-POLICY`.

#### Scenario: sharing yields two handles to one storage

- **WHEN** `SharedFrame::share()` is called
- **THEN** the result is a second handle to the same underlying frame storage,
  with no pixel copy

### Requirement: intentional copies carry a specific marker

Every intentional media copy or materialization boundary SHALL carry a nearby
specific `splot-copy-ok: <reason>` marker naming the boundary (same line or within
the preceding two lines). Vague markers SHALL be rejected. Tracked by
`INFRA-ZERO-COPY-MEDIA-POLICY`.

#### Scenario: an unmarked sample copy is rejected

- **WHEN** a sample-buffer copy (`.to_vec()`, `copy_from_slice`,
  `extend_from_slice`, `clone_from_slice`, `Vec::from(&…)`) appears in a scanned
  media module without a nearby specific `splot-copy-ok:` marker
- **THEN** `cargo xtask check-zero-copy-policy` fails with the offending location

#### Scenario: a vague marker is rejected

- **WHEN** a copy is marked only with a vague reason (`splot-copy-ok` alone,
  `temporary`, `fix`, `needed`, `convenience`, `TODO`)
- **THEN** the gate fails and requires a specific boundary reason

### Requirement: zerocopy restricted to fixed-layout wire views

`zerocopy` SHALL be used only for private fixed-layout byte/wire view structs
(e.g. the IVF container header) in approved crates, via the workspace dependency
shape, and SHALL NOT appear in public APIs or parse AV2 bit-level syntax, LEB128,
entropy-coded data, or variable-length/state-dependent syntax. Banned alternative
byte/transmute crates SHALL be rejected. Tracked by
`INFRA-ZERO-COPY-MEDIA-POLICY`.

#### Scenario: zerocopy in a disallowed crate

- **WHEN** a crate other than the approved set declares a direct `zerocopy`
  dependency, or a banned alternative byte/transmute crate is added
- **THEN** `cargo xtask check-zero-copy-policy` fails with the offending manifest

### Requirement: enforcement by check-zero-copy-policy

The zero-copy media-buffer policy SHALL be enforced by `cargo xtask
check-zero-copy-policy`, which runs in `cargo xtask ci` and in CI, alongside the
dependency-direction and concurrency gates. Tracked by
`INFRA-ZERO-COPY-MEDIA-POLICY`.

#### Scenario: policy gate runs in ci

- **WHEN** `cargo xtask ci` runs
- **THEN** `cargo xtask check-zero-copy-policy` SHALL run and SHALL fail the build
  on any policy violation

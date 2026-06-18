## Context

`splot-encode` now has validated borrowed 8-bit YUV420 input frames, a
deterministic lifecycle, private syntax planning records, a private minimal
header plan, and typed runtime speed presets. It still has no arithmetic stage
between prediction and future transform/quantization work.

The decoder-visible residual-addition process lives in `splot-recon` as
`RECON-RESIDUAL-ADDITION` and follows AV2 § 7.14.3
(`docs/spec/av2/1.0.0/07-decoding-process.md#s-7-14-3`): decoded
reconstruction adds signed residual samples to prediction and clamps. The
encoder-side residual foundation is the non-normative inverse signal
preparation step: for a selected block and prediction, compute
`source_sample - prediction_sample` as signed row-major values for later forward
transform and quantization.

No packet-producing path exists yet. This change therefore introduces only a
private arithmetic primitive and proof. It must not emit syntax, mutate writer
state, or claim Baseline Encoder Profile v1 support.

## Goals / Non-Goals

**Goals:**

- Add a stable `ENC-RESIDUAL-FOUNDATION` matrix row.
- Add a private `crates/splot-encode/src/residual.rs` module.
- Compute checked signed residual blocks for the current 8-bit input surface.
- Use visible-plane-relative block rectangles over `splot_recon::PlaneRef<u8>`.
- Accept row-strided prediction samples, validate their shape before producing
  residuals, and return row-major signed residual samples.
- Use explicit signed storage and checked/fallible allocation.
- Prove zero, min/max, checkerboard/gradient, odd-edge, stride, and mismatch
  behavior with focused tests.
- Preserve the existing no-output encoder lifecycle.

**Non-Goals:**

- No 10-bit input extension, Y4M adapter, forward transform, quantization,
  coefficient tokenization, CDF selection, range encoding, tile-body emission,
  packet output, CLI success path, rate control, speed-policy consumption, or
  public Baseline Encoder Profile v1 claim.
- No dependency graph changes and no changes to `splot-core` or `splot-recon`.
- No external reference-code use or external decoder invocation.

## Decisions

### Private block-oriented residual type

The change will add a crate-private `ResidualBlock` with plane id, block
rectangle, and row-major `Vec<i16>` samples. Its constructor will take:

- `PlaneId`;
- a borrowed `PlaneRef<'_, u8>`;
- a `PlaneRect` whose coordinates are relative to the plane's visible area;
- prediction samples plus prediction row stride.

`i16` is sufficient for the current 8-bit surface (`-255..=255`) and the later
10-bit baseline (`-1023..=1023`), while subtraction uses `i16`/`i32`-safe
intermediates and never wraps. The type stays private so a future transform PR
can adjust the handoff before public API stabilization.

Alternative considered: compute full-frame residual planes. That would
materialize more data than needed and blur the block-level transform boundary.

### Use `PlaneRef` visible rows instead of raw indexing

Input rows will be read through the existing `PlaneRef::visible_rows()` iterator,
then sliced by the checked visible-relative block rectangle. This reuses the
current zero-copy plane validation and avoids duplicating backing-buffer stride
and visible-origin arithmetic.

Alternative considered: expose a new `PlaneRef::rect_rows` helper in
`splot-recon`. That may become useful later, but this PR does not need a shared
API change.

### Fallible residual materialization

Residual samples are a necessary materialization boundary before transform
work. The implementation will validate block geometry, prediction stride, and
prediction buffer span before allocating. It will use fallible reservation and
return a typed encoder error if allocation fails.

The source will include a narrow `splot-copy-ok:` marker where signed residual
samples are materialized, because this is an intentional transform input rather
than hidden media-frame duplication.

### No context or packet behavior

The residual module will not be called from `Context::receive_packet` yet. The
existing tests proving no fake packets remain valid, and the matrix/roadmap will
state that residual calculation alone does not produce legal AV2 output.

## Flight manifest

- Change ID: `encoder-residual-foundation`
- Feature IDs: `ENC-RESIDUAL-FOUNDATION`
- Base commit: `d42a7fbe345e8dbb7f7e2d332a3ec703dfd89161`
- Depends on merged changes: `encoder-program-contract`,
  `encoder-recon-dependency`, `encoder-frame-input-views`,
  `encoder-context-state-machine`, `encoder-syntax-ir`,
  `encoder-minimal-header-plan`, `encoder-speed-presets`
- Exact files/directories owned by this PR:
  - `crates/splot-encode/src/residual.rs`
  - `crates/splot-encode/src/lib.rs`
  - `crates/splot-encode/src/error.rs`
  - `docs/IMPLEMENTATION-MATRIX.toml`
  - `docs/FEATURE-STATUS.md`
  - `docs/SPEC-COVERAGE.md`
  - `docs/ENCODER-ROADMAP.md`
  - `docs/ENCODER-GAP-AUDIT.md`
  - `openspec/changes/encoder-residual-foundation/**`
  - `openspec/specs/encoder-tools/spec.md`
- Exact files/directories forbidden to this PR:
  - `Cargo.toml`
  - `Cargo.lock`
  - `crates/splot-core/**`
  - `crates/splot-recon/**`
  - `crates/splot-decode/**`
  - `crates/splot-validate/**`
  - `crates/splot-cli/**`
  - `fuzz/**`
  - `docs/spec/av2/**`
- Public APIs/types owned: none
- Matrix rows owned: `ENC-RESIDUAL-FOUNDATION`
- Generated files owned: `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`
- Open sibling PRs audited: none (`gh pr list --state open` returned `[]`)
- Changed-file intersection with each sibling PR: none
- Semantic overlap with each sibling PR: none
- Can build/test/merge directly onto main without another open PR: yes

## Risks / Trade-offs

- The first residual type is private and 8-bit-focused. Mitigation: keep the
  matrix status scoped to the current input surface and extend deliberately in a
  later 10-bit input PR.
- Residual materialization copies derived signed samples. Mitigation: document
  the transform-input boundary with a `splot-copy-ok:` marker and avoid copying
  source or prediction media storage.
- Future transform stages may want a different signed storage type. Mitigation:
  keep the type crate-private and avoid public API commitments.

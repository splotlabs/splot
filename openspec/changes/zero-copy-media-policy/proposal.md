# Change: zero-copy-media-policy

## Feature IDs

- `INFRA-ZERO-COPY-MEDIA-POLICY`

## Why

Decoder, reconstruction, and encoder work will move large media buffers — frames,
planes, reference-frame storage, lookahead retention, and pixel/sample storage.
Without an explicit, enforced ownership model, ordinary-looking code
(`frame.clone()`, `.to_vec()`, `copy_from_slice`, an accidental `#[derive(Clone)]`
on a frame type, or `Arc::make_mut`) can silently duplicate whole frames. The cost
is invisible in review and compounds once real hot paths exist.

This change makes the media-buffer **ownership model** explicit, documented,
tested, and CI-enforced **before** that code is written, so the default is
view-first borrowing and every genuine duplication is a deliberate, named,
reviewable materialization. This is infrastructure / codec-runtime API policy; it
adds no AV2 conformance coverage and implements no decoder/encoder algorithmic
stage.

`zerocopy` is in scope only as a narrow tool for fixed-layout byte/wire view
structs (e.g. the IVF container header). It is **not** the frame-buffer ownership
model and never appears in public APIs or AV2 bit-level/entropy/variable-length
parsing.

## Scope

- Spec sections: none (infrastructure; the `runtime` capability, sibling to the
  concurrency-runtime policy).
- Crates/modules: `splot-recon` (view types `PlaneRef`/`PlaneMut`/`FrameRef`/
  `FrameMut`, an explicit `SharedFrame` share handle, removal of `Clone` from
  media-storage types, and `splot-copy-ok:` markers on intentional copies);
  `xtask` (new `check-zero-copy-policy` gate wired into `cargo xtask ci`);
  optionally `splot-core` (a private `zerocopy` IVF wire struct) only if it
  preserves all current IVF error behavior.
- CLI/docs/tests: `docs/ZERO_COPY.md` (canonical policy), `docs/ARCHITECTURE.md`
  (zero-copy ownership subsection + `zerocopy` dependency-direction note),
  `docs/CODE_REVIEW.md` (zero-copy checklist), the implementation matrix, and the
  `runtime` capability spec. Gate accept/reject unit tests plus recon view/handle
  tests.

## Non-goals

- No decoder, reconstruction, encoder, or residual algorithm work; no
  algorithmic stage marked implemented.
- No change to AV2 conformance behavior, validator diagnostics, rule IDs, spec
  sections, byte/bit offsets, message text, ordering, or the CLI contract.
- No `unsafe`, `transmute`, raw-pointer cast, or `from_raw_parts` to claim
  zero-copy.
- No new third-party dependency other than the single authorized `zerocopy`, and
  `zerocopy` is never added unused.
- No `zerocopy` use for AV2 bit-level syntax, LEB128, entropy-coded data, or
  variable-length/state-dependent syntax, and no wire struct in any public API.

## Acceptance criteria

- [ ] Implementation matrix row `INFRA-ZERO-COPY-MEDIA-POLICY` exists and is
      updated to reality with proof.
- [ ] `docs/ZERO_COPY.md` defines borrow / mutable view / share / materialize /
      copy-ok and the required default ownership model, banned patterns, and the
      `splot-copy-ok:` marker grammar.
- [ ] `splot-recon` is view-first: `PlaneRef`/`PlaneMut`/`FrameRef`/`FrameMut`
      construct without allocating or copying; owned media-storage types do not
      derive/impl `Clone`; `SharedFrame` shares via `.share()` only.
- [ ] No `Clone`/`Arc::make_mut`/`Rc::make_mut` on frame/sample storage anywhere.
- [ ] `cargo xtask check-zero-copy-policy` exists, is deterministic, is
      unit-tested for accept/reject cases, and runs inside `cargo xtask ci`.
- [ ] Every remaining intentional copy carries a nearby specific `splot-copy-ok:`
      marker with a real boundary reason.
- [ ] `zerocopy` is added only via the workspace dep shape and only where a real
      private fixed-layout use site exists; otherwise it is documented as an
      approved future dependency and not added.
- [ ] Positive tests exist (view construction without allocation, `*Mut`
      exclusivity + row access, `SharedFrame::share()` pointer identity,
      reference-store move/share without `T: Clone`).
- [ ] Negative/reject tests exist for every gate rule.
- [ ] `cargo xtask check-feature-status` passes.

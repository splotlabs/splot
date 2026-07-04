## Context

The local decoder mission minimal runtime now consumes active frame-level Wiener NS LR unit
syntax, retains §7.20.1 source/tile bounds, resolves §7.20.4 classified-luma
source-read coordinates, and can derive `FilterClass` when tests supply source
sample values plus `LrTxSkip` values. That callback proof is not yet a storage
proof: real decode will need immutable current/CDEF frame views and retained
transform-skip state, and those lookups can fail with typed storage errors.

`splot-recon::pc_wiener_classify` currently treats `LrTxSkip` reads as
infallible even though a decoder-owned grid has dimensions and backing storage.
The source-sample side already returns `ReconResult<T>`, so this change aligns
the `LrTxSkip` side with real storage while keeping the existing dependency
direction (`splot-decode -> splot-recon`) unchanged.

## Goals / Non-Goals

**Goals:**

- Add `DECODE-LR-CLASSIFIED-WIENER-STORAGE` as the matrix-owned decoder
  frontier after callback-backed value derivation.
- Allow `pc_wiener_classify` to propagate fallible `LrTxSkip` lookup errors.
- Add decoder-side storage helpers that read §7.20.2 luma source samples from
  `DecodedFrame` frame views and read boolean `LrTxSkip` values from a bounded
  grid.
- Update the live local decoder mission diagnostic to name the remaining retention/filtering
  boundary without claiming output or byte equality.
- Prove the helper with synthetic 10-bit current/CDEF frame storage and focused
  `LrTxSkip` grid error tests.

**Non-Goals:**

- No live local decoder mission frame reconstruction handoff.
- No LR filtering, chroma Wiener NS filtering, subclass lookup, output storage,
  reference refresh, AVM/dav2d differential success, new dependency, or crate
  dependency graph change.

## Decisions

- Keep storage ownership in `splot-decode`. `splot-recon` remains the reusable
  math and frame-read layer; decoder code adapts retained LR block facts and
  future transform-skip state into recon inputs.
- Make only the `pc_wiener_classify` `tx_skip` callback fallible. This is the
  smallest API correction needed for real storage and mirrors the existing
  source-sample callback error path.
- Use a decoder-local `WienerNsLrTxSkipGrid` rather than widening public APIs.
  The current mission needs a bounded storage proof, not a final transform block
  state model.
- Keep the live diagnostic fail-closed. The runtime still reaches LR before it
  retains decoded 10-bit current/CDEF frame storage and `LrTxSkip`; the new
  helper is a proven handoff for when those buffers are wired.

## Risks / Trade-offs

- [Risk] `pc_wiener_classify` has a small public API change inside the
  source-available workspace. -> Mitigation: update all in-repo call sites and
  tests in the same brick.
- [Risk] The new storage helper can look final even though live local decoder mission still
  fails. -> Mitigation: matrix, decoder support, and diagnostic text explicitly
  record the remaining live runtime boundary.
- [Risk] `runtime_minimal/wienerns_lr.rs` continues to grow. -> Mitigation: keep
  additions private and focused; defer module splitting to a cleanup brick.

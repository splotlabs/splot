## Context

The ac0ej3 minimal runtime now consumes active frame-level Wiener NS LR unit
syntax, retains §7.20.1 source/tile bounds, and resolves the §7.20.4
classified-luma source-read plus `LrTxSkip` lookup coordinates. The existing
frontier intentionally fails before reading values because the live path reaches
LR before 10-bit current/CDEF frame storage and before any decoder-owned
`LrTxSkip` grid is available.

`splot-recon` already exposes `pc_wiener_classify`, which implements the
scheduler-free §7.20.4 classification math over caller-provided source samples
and `LrTxSkip` values. This change connects the decoder frontier to that
primitive through typed callbacks and leaves storage ownership unresolved.

## Goals / Non-Goals

**Goals:**

- Add `DECODE-AC0EJ3-LR-CLASSIFIED-WIENER-VALUES` as the matrix-owned decoder
  frontier after coordinate derivation.
- Add decoder helper state that derives `FilterClass[y >> 2][x >> 2]` values for
  active luma classified Wiener blocks when callers supply source sample values
  and `LrTxSkip` values.
- Keep live ac0ej3 fail-closed and structured, with a diagnostic that identifies
  missing 10-bit current/CDEF frame storage and missing `LrTxSkip` storage.
- Prove the helper with focused synthetic-value tests and local ac0ej3 diagnostic
  evidence.

**Non-Goals:**

- No real ac0ej3 frame-buffer reads in the live runtime.
- No LR filtering, chroma Wiener NS filtering, subclass lookup, output storage,
  reference refresh, AVM/dav2d differential success, dependency graph changes, or
  broad AV2 decode claims.

## Decisions

- Keep classification value derivation private to `splot-decode` for now. The
  public, reusable math remains in `splot-recon`; decoder state only adapts
  retained LR block facts into recon parameters and captures a fail-closed proof
  summary.
- Use caller callbacks for source samples and `LrTxSkip` values instead of
  synthetic live values. This prevents the ac0ej3 runtime from fabricating
  current/CDEF frame data before the 10-bit storage brick exists.
- Move the live diagnostic to a new matrix row once the value-capable path is
  proven. The message will say the decoder can compose classified-Wiener
  classification over supplied values, but ac0ej3 lacks the runtime storage
  needed to supply real values.
- Add focused tests in `runtime_minimal/inter/lr_source_read_tests.rs` rather
  than growing the already-large `inter/tests.rs`.

## Risks / Trade-offs

- `wienerns_lr.rs` is already above the soft source-line budget -> keep the new
  code compact and consider a later split if the LR runtime grows again.
- The live fixture still fails at byte offset 74 -> acceptable because this brick
  is a frontier advance, not a successful decode.
- Callback-based proof can drift from future storage ownership -> mitigate by
  recording the missing storage in the matrix row and diagnostic, and by keeping
  the helper shape close to `pc_wiener_classify` inputs.

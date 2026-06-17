## Context

`RECON-INVERSE-TRANSFORM-1D` added the § 7.15.2.1 kernel transform. This change
completes the § 7.15.2 1D transform group with the two matrix-free transforms the
§ 7.15.4 2D process also invokes: § 7.15.2.2 Walsh-Hadamard (lossless) and
§ 7.15.2.3 identity (`IDT`). Both are pure and need no kernel tables.

## Goals / Non-Goals

Goals:

- Implement § 7.15.2.2 and § 7.15.2.3 exactly, reusing the existing module's
  `Round2` and clamp-bound helpers.
- Keep both total and panic-free.

Non-Goals:

- The § 7.15.4.1 `get_identity_scale` derivation, the § 7.15.3 secondary
  transform, the § 7.15.4 2D orchestration, dequantization, residual addition,
  runtime decode, or reference refresh.

## Decisions

- **Walsh-Hadamard returns a fixed `[i32; 4]`.** § 7.15.2.2 is always a 4-element
  in-place transform, so a fixed-size array input and output is the most precise
  signature (no length error is possible). The spec applies no `Clip3`; the
  butterfly result is returned directly, computed with `i64` intermediates so it
  is total (lossless residuals are bounded well within `i32`).
- **Identity transform shares the clamp bound with § 7.15.2.1.** § 7.15.2.3 uses
  the same `colTx`-dependent `Clip3` bound as § 7.15.2.1, so the inline bound
  computation from `inverse_transform_1d` is factored into a
  `transform_clip_bounds` helper used by both. `scale` is supplied by the caller
  (the § 7.15.4.1 `get_identity_scale` derivation stays with the future 2D
  orchestration row). The identity transform accepts any length and returns a
  typed `ReconError` only on a source/output length mismatch.
- **Totality.** Both use `i64` intermediates; `Round2` already guards large
  shifts (from the § 7.15.2.1 row), so neither panics for any caller input.

## Risks / Trade-offs

- The Walsh-Hadamard signature is `[i32; 4]` rather than a slice, diverging from
  the slice-based identity/kernel signatures, but it matches the spec's
  fixed 4-element transform and removes a class of caller error.

## Migration Plan

Additive; extends an existing module. No API changes to existing functions
(`transform_clip_bounds` is an internal refactor), and the runtime is
unaffected.

## Open Questions

None.

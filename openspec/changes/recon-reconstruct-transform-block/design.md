## Context

`reconstruct_add_residual` (`RECON-RESIDUAL-ADDITION`) is deliberately scoped to
the § 7.14.3 add step alone, "independent of how the residual is produced." The
§ 7.14.4 dequantization and § 7.15.4 inverse transform that produce the residual
are separate primitives. A decoder, and the encoder closed loop today, run all
three in a fixed order; this change captures that order once.

## Goals / Non-Goals

**Goals:**

- Provide one tested entry point for the dequant → inverse-transform → add chain.
- Centralize the buffer-size contract that spans the three steps so a caller
  cannot size the dequant, residual, and output buffers inconsistently.
- Keep it a total, panic-free, allocation-free `pub` composition with no new
  error variant and no runtime rewiring.

**Non-Goals:**

- Producing `Quant[]` (coefficient entropy decode), the § 7.15.3 secondary
  transform, the § 7.15.4 DPCM-direction selection, prediction, or any wiring
  into the runtime decode path.

## Decisions

- **New module `reconstruct_block.rs`, not `reconstruct.rs`.** `reconstruct.rs`
  documents itself as the § 7.14.3 add step only and explicitly out-of-scopes the
  transform and dequantization; the full-chain composition lives in its own
  module so neither module's scope statement is contradicted.
- **`pub` free function, not a decode-side helper.** The composition belongs in
  `splot-recon` (it composes recon primitives) and is reusable by both the
  encoder closed loop and the future decoder wiring. As `pub` API it is exercised
  by tests without a dead-code hack; a crate-private decode-side helper with only
  a test caller would be dead in the library build until a residual-carrying
  block exists.
- **Caller-owned scratch buffers.** Mirrors the other `splot-recon` caller-buffer
  primitives (`inverse_transform_2d_outer` takes `dequant` / `residual` slices),
  keeps the composition allocation-free, and lets the caller reuse buffers across
  blocks.
- **Resolve the transform with `InverseTransform2dOuter::resolve`.** Dogfoods the
  resolve helper so a test (and a future caller) cannot desync the shifts, types,
  and dimensions; the encoder closed loop hand-builds the struct and could adopt
  the same helper later.
- **No new error variant.** Each step already returns a precise typed
  `ReconError`; the composition propagates the first failure, and a focused test
  pins that an inconsistent dequant-scratch length is rejected before `out` is
  written.

## Risks / Trade-offs

- The composition is "only three calls," so it risks reading as trivial. It earns
  its place by centralizing the cross-step buffer-size contract (a real footgun:
  the dequant buffer is adjusted-size, the residual/out buffers original-size)
  and by giving the chain one tested, documented entry point that both crates can
  share.
- It is loaded ahead of its runtime caller, matching the established pattern of
  building the residual-math primitives before wiring; the matrix and roadmap
  keep the coefficient decode, `compute_tx_type`, and runtime wiring partial or
  unimplemented.

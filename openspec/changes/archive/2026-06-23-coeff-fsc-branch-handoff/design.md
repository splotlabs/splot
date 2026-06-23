## Context

`DECODE-COEFF-FSC-CONTEXT-COMMIT` leaves the FSC/IDTX path with all local
nonzero coefficient stages available, but only when tests manually assemble the
nonzero EOB start, checked FSC scan window, level pass, and context commit
configuration. The ordinary non-FSC path already has a branch-level wrapper that
starts from the decoded `all_zero` choice and delegates to its loaded nonzero
pipeline.

The FSC path needs the same loaded-but-unwired branch boundary so later
`coeffs()` integration can route `useFsc` into one helper instead of recreating
the EOB-start and scan-walk composition at every call site.

## Goals / Non-Goals

**Goals:**

- Add a crate-private FSC branch handoff for the nonzero `useFsc` arm of AV2
  §5.20.7.27.
- Derive the checked FSC `bob..segEob` scan walk from a nonzero EOB start,
  caller-resolved `segEob`, and caller-resolved scan order before symbol
  consumption beyond EOB syntax.
- Compose the existing FSC level pass with the existing FSC quant/context
  commit wrapper.
- Reject all-zero and non-luma routing atomically for this FSC-specific boundary.
- Keep the helper loaded-but-unwired and private to `splot-decode`.

**Non-Goals:**

- Runtime `coeffs()` wiring or `useFsc` derivation.
- Deriving `segEob`, scan order, transform type, transform size, or frame
  feature facts from runtime syntax.
- Dequantization, inverse transform, residual add, reconstruction/output,
  reference refresh, inter prediction, filters, public APIs, AVM/dav2d
  integration, or broad `decode_tile()` support.

## Decisions

- Model the wrapper as FSC-only and nonzero-only. AV2 §5.20.7.27 enters the FSC
  loops only after `all_zero == 0` and `useFsc` is true, so accepting an
  all-zero arm here would hide a caller-routing bug rather than model real FSC
  behavior.
- Reuse `read_coeff_block_eob_branch` for the nonzero EOB start. This preserves
  the existing allocation-before-symbol-read ordering and the existing typed EOB
  errors.
- Derive `FscCoeffScanWalk` inside the wrapper from caller-resolved `segEob` and
  scan order, then run `apply_nonzero_coeff_fsc_level_pass` followed by
  `apply_nonzero_coeff_fsc_quant_pass_with_context_commit`. This keeps the
  wrapper focused on branch composition instead of duplicating level, sign,
  quant, or context logic.
- Preflight `context.plane == 0` before calling the EOB branch helper. The lower
  context-commit wrapper already enforces this, but checking at the branch
  boundary prevents a misrouted chroma call from consuming EOB syntax first.

## Risks / Trade-offs

- The wrapper still depends on caller-resolved `segEob`, scan, transform
  geometry, and CDF context facts. That is intentional for this brick; those
  runtime derivations remain separate tracked work.
- A later runtime caller must still choose between ordinary and FSC branches
  from the full `useFsc` expression. This helper only makes the FSC target
  cohesive once that decision is available.

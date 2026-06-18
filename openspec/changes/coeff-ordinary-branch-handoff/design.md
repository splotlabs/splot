## Context

The coefficient loop has staged pieces for AV2 § 5.20.7.27 and § 5.20.7.28:
all-zero state application, nonzero EOB start, checked scan walk, derived
base/level first pass, derived sign-source handoff, interleaved sign and
`read_quant`, signed `Quant[]` writes, and state-backed context commit. The
latest state-context handoff still leaves callers responsible for stitching the
decoded `all_zero` branch to the ordinary nonzero pass.

The next runtime integration seam should keep the same boundary discipline:
caller-resolved syntax facts remain explicit, while the branch dispatcher owns
the control-flow handoff from all-zero versus nonzero to the corresponding
coefficient state mutation.

## Goals / Non-Goals

**Goals:**
- Add a crate-private ordinary coefficient branch composer tracked by
  `DECODE-COEFF-ORDINARY-BRANCH-HANDOFF`.
- Reuse the existing all-zero branch behavior for current runtime trace calls.
- Route nonzero branches through `read_coeff_block_eob_branch` and
  `apply_nonzero_coeff_ordinary_pass_with_state_context`.
- Preserve typed errors and state-preservation behavior at the branch boundary.
- Update implementation matrix, decoder support, conformance coverage, roadmap,
  generated docs, and OpenSpec artifacts.

**Non-Goals:**
- No broad runtime `coeffs()` caller from transform-block syntax.
- No derivation of `scan = get_scan(txSz, txClass)`, transform class, plane type,
  TCQ, lossless, or transform geometry from parsed syntax.
- No FSC or IDTX coefficient path.
- No dequantization, inverse transform, residual add, reconstruction, output,
  reference refresh, AVM/dav2d invocation, public API, or dependency changes.

## Decisions

1. Add the branch composer in `coeff_loop/ordinary_pass.rs`.

   Rationale: the composer is logically adjacent to the ordinary non-FSC pass
   and can reuse `CoeffOrdinaryStateContextPassInput` directly. Keeping it in
   `ordinary_pass.rs` avoids spreading the state-context pass wiring into the
   EOB branch module.

   Alternative considered: extend `read_coeff_block_eob_branch` directly. That
   would make the EOB-start helper know about the full ordinary pass and its
   caller-resolved scan/transform facts, widening a module that is currently
   limited to all-zero versus EOB-start behavior.

2. Model inputs as an enum with an all-zero arm and a nonzero arm.

   Rationale: this mirrors the spec `if (all_zero) ... else ...` branch and
   allows the all-zero runtime trace to migrate without constructing dummy
   nonzero facts. The nonzero arm carries the existing nonzero start input plus
   the state-context ordinary pass facts.

   Alternative considered: require callers to pass a pre-built
   `CoeffBlockEobBranch`. That would skip EOB-start composition and would not
   prove the full branch path from caller-selected `all_zero`.

3. Preserve the current all-zero minimal trace behavior.

   Rationale: the committed fixture remains all-zero. This change should be a
   no-output-change integration seam, with nonzero behavior proven by focused
   unit tests until real transform-block syntax can supply nonzero facts.

## Risks / Trade-offs

- [Risk] The composer can be mistaken for full runtime `coeffs()` support.
  -> Mitigation: matrix, support row, docs, and comments state that scan,
  transform, TCQ, lossless, and broader block facts remain caller-resolved and
  reconstruction remains unsupported.
- [Risk] Failures after nonzero EOB reading may have already consumed symbols or
  updated CDF rows.
  -> Mitigation: tests assert coefficient context state preservation where the
  existing lower layers promise it; whole-trace rollback remains owned by the
  minimal block-symbol trace wrapper.
- [Risk] `ordinary_pass.rs` is near the source-line budget.
  -> Mitigation: keep the new API compact and move tests to a separate test
  module if needed.

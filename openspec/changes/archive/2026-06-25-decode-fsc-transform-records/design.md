## Context

The local decoder mission Wiener NS LR runtime path now parses the local stream far enough to
populate active-MRL metadata and then reaches
`unsupported_wienerns_lr_live_transform_record_fsc_mode` while deriving live
`LrTxSkip` transform records. AV2 §5.20.5.3 stores block-level `FscModes`,
§8.3.2 derives `fsc_mode` CDF context from `NPos`, §5.20.8.2 maps active
`fsc_mode` luma transform type to `IDTX`, and §5.20.7.27 derives `useFsc` from
`enable_fsc`, `PlaneTxType == IDTX`, luma plane, and `fsc_mode || is_inter`.

The coefficient loop already has crate-private FSC/IDTX branch helpers and a
frame-facts `useFsc` wrapper. The missing piece is the local decoder mission LR record path
threading observed `fsc_mode` into the existing nonzero residual handoff and
recording only metadata needed for `LrTxSkip`, while decoded samples and
reconstruction remain fail-closed.

## Goals / Non-Goals

**Goals:**

- Consume the observed local decoder mission luma `fsc_mode` syntax in the Wiener NS LR
  transform-record path.
- Carry caller-resolved `fsc_mode` into nonzero luma residual parsing so the
  existing frame-facts `useFsc` wrapper can select the FSC branch.
- Retain `skip_flag`/`eob` facts needed for live `LrTxSkip` storage and advance
  the local decoder mission unsupported diagnostic frontier.
- Keep unobserved FSC shapes, chroma FSC, decoded samples, loop restoration,
  output, and reference refresh fail-closed.

**Non-Goals:**

- Broad FSC/IDTX support outside the local decoder mission LR record walk.
- Inverse transform, dequantized sample reconstruction, `CurrFrame` or
  `CdefFrame` population, loop-restoration filtering/output, or reference
  refresh.
- New fixtures, external AVM/dav2d invocation from repository code, public APIs,
  CLI options, dependencies, or encoder changes.

## Decisions

1. Reuse the existing coefficient `useFsc` frame-facts wrapper.

   Rationale: `apply_coeff_use_fsc_branch_from_frame_facts` already models the
   §5.20.7.27 ordering boundary and selects between ordinary and FSC branches
   from caller-resolved frame/block facts. Wiring the runtime path to it avoids
   duplicating FSC scan, level, sign, quant, and context-commit logic.

   Alternative considered: add a local FSC-only parser in
   `wienerns_lr/tx_records.rs`. That would duplicate coefficient-loop behavior
   and make future context fixes harder to apply uniformly.

2. Treat `fsc_mode` as a record-walk fact, not reconstruction support.

   Rationale: the current local decoder mission path only needs syntax consumption and
   `LrTxSkip` metadata to advance the loop-restoration frontier. A successful
   FSC coefficient parse still must fail closed before decoded sample
   population.

   Alternative considered: return coefficient `Quant[]` into reconstruction.
   That crosses into inverse transform, residual add, filtering, output, and
   oracle equality, which is too broad for this brick.

3. Keep the supported subset local and explicit.

   Rationale: active FSC changes mode-info context, transform type, coefficient
   CDF selection, and coefficient scan order. The safe slice is the observed
   luma transform-record subcase in the local decoder mission stream; other plane,
   geometry, and tool combinations should remain structured unsupported
   diagnostics.

   Alternative considered: broaden all `fsc_mode` CDF selectors at once. That
   would widen more syntax surface than the local probe proves and risks
   confident-wrong residual records.

## Risks / Trade-offs

- [Risk] `fsc_mode` context selection can desynchronize if it reuses the wrong
  neighbor ordering.
  Mitigation: derive it from the same AV2 `NPos` semantics used for MRL-style
  same-superblock-row context state and add focused tests around neighbor
  context behavior when implemented.

- [Risk] Advancing the unsupported frontier may expose a larger downstream
  filter/reconstruction gap immediately.
  Mitigation: keep the CLI ignored local decoder mission test asserting the precise current
  diagnostic after this brick and do not claim successful decode.

- [Risk] FSC branch result may be parsed but then accidentally treated as
  reconstructed output.
  Mitigation: only retain `skip_flag`/`eob`-style LR tx-skip facts and maintain
  fail-closed diagnostics before `CurrFrame`/`CdefFrame` sample population.

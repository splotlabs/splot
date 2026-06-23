## Context

`DECODE-COEFF-SCAN-WALK` validates the ordinary non-FSC nonzero coefficient scan
window by returning entries in reverse order (`eob - 1` down to `0`). The AV2
§5.20.7.27 `useFsc` branch is structurally different: the decoded nonzero EOB
identifies how many coefficients are coded at the end of the segment, then
`bob = segEob - eob`, `eob = segEob`, and the level pass visits the scan window
forward from `bob` to `segEob - 1`.

The IDTX CDF row families are now loaded by `DECODE-COEFF-IDTX-CDF-ROWS`, but a
future symbol reader should not derive or trust raw scan indices ad hoc. This
change adds the missing checked scan-window object only.

## Goals / Non-Goals

**Goals:**

- Add `FscCoeffScanWalk` with `bob`, `seg_eob`, and forward
  `CoeffScanEntry` records.
- Validate decoded EOB is positive, `eob <= segEob`, `segEob <= scan.len()`,
  and every visited `scan[c]` fits the initialized coefficient block.
- Preserve the no-CDF/no-symbol/no-state-mutation scan-walk boundary.
- Keep matrix, decoder support, decoder conformance coverage, roadmap, and
  generated docs honest.

**Non-Goals:**

- No runtime `coeffs()` integration, `useFsc` derivation, `coeff_base_bob`,
  `coeff_base_idtx`, `coeff_br_idtx`, or `idtx_sign` symbol reads.
- No `Level[]`, `QuantSign[]`, or `Quant[]` writes, `read_quant` composition,
  tile context commit, dequantization, reconstruction, reference refresh, or
  decoded output change.
- No public API, CLI, dependency, licensing, encoder, or broad CDF lifecycle
  expansion.
- No AVM/dav2d invocation from repository code, tests, scripts, or CI.

## Decisions

1. **Use a separate walk type.**
   `FscCoeffScanWalk` keeps the forward `bob..segEob` order explicit and avoids
   overloading `NonZeroCoeffScanWalk`, whose semantics are reverse ordinary
   non-FSC traversal.

2. **Take `segEob` as caller-resolved input.**
   The staged wrapper does not yet derive raw `Tx_Width[txSz]` /
   `Tx_Height[txSz]` for the FSC path. It validates the supplied value against
   the scan table and local coefficient block before returning entries.

3. **Reuse `CoeffScanEntry`.**
   Both ordinary and FSC paths need the same checked `c`, `pos`, `row`, and
   `col` facts. Reusing the entry type keeps future symbol readers compatible
   with existing coefficient state helpers.

## Risks / Trade-offs

- **Overclaiming support** -> Tracking must say this is scan-window validation
  only; runtime FSC/IDTX symbols and coefficient writes remain unsupported.
- **Caller-resolved `segEob`** -> A later runtime wrapper must derive it from
  real `txSz` dimensions. This helper only validates the staged caller fact.
- **Shared errors** -> The helper reuses ordinary scan length/position errors
  and adds only the FSC-specific `eob > segEob` error.

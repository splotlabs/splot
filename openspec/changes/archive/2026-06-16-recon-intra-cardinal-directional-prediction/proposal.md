## Why

The minimal runtime reconstruction path still materializes neutral chroma for a
traced `H_PRED` mode because `splot-recon` does not expose the cardinal
horizontal/vertical directional intra primitives. Adding only the pAngle 90/180
subset removes that explicit fallback while keeping broad AV2 intra prediction
honest and partial.

## What Changes

- Add a source-backed `splot-recon` primitive for AV2 cardinal directional intra
  prediction:
  - `V_PRED` (pAngle 90) copies prepared above samples across each row.
  - `H_PRED` (pAngle 180) copies prepared left samples across each column.
- Add current-frame workspace helpers for in-storage H/V prediction when the
  required prepared edge is available.
- Update the minimal runtime reconstruction handoff to use explicit traced
  chroma `H_PRED` fallback materialization instead of the undocumented neutral
  fill, updating the deterministic minimal hash/Y4M expectations to the
  spec-correct chroma samples.
- Extend the existing intra prediction fuzz target and support/matrix docs for
  the new primitive.
- Non-goals: no general directional angles, IDIF, MRL, wide-angle mapping, IBP,
  filter intra, CfL/CCTX/MHCCP, palette, residuals, transforms, loop filters,
  reference refresh, broad `decode_tile()`, AVM/dav2d integration, or new
  dependencies.

## Capabilities

### New Capabilities

- `recon-intra-cardinal-directional-prediction`: H/V-only cardinal directional
  intra prediction primitives and workspace handoff for the existing minimal
  runtime trace.

### Modified Capabilities

- `decoder-support`: Record Feature ID
  `RECON-INTRA-CARDINAL-DIRECTIONAL-PREDICTION`, its tests/fuzz evidence, and
  the unchanged partial status of broad intra reconstruction.
- `minimal-intra-reconstruction-frontier`: Replace the documented neutral
  chroma fallback with explicit traced chroma `H_PRED` handling for the minimal
  fixture and record the corrected output contract.

## Impact

- Affected code: `splot-recon` intra/workspace modules, the minimal
  `splot-decode` reconstruction handoff, the existing recon intra fuzz target,
  decoder support/matrix docs, and OpenSpec artifacts.
- Validator impact: none.
- User-facing diagnostics: none added or changed; out-of-tier streams continue
  to fail through existing `decode/unsupported-feature` and resource-limit
  paths.
- Dependencies and licensing: no new dependency, no AVM/dav2d invocation, and no
  copied third-party code or tables.

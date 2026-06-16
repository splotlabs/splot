## Why

The one-sided and middle directional-angle primitives now exist, but current-frame workspace callers still cannot use in-storage directional-angle neighbor samples to write predicted blocks. This change adds the narrow workspace handoff needed for source-backed decoder progress while preserving honest scope around AV2 edge preparation and runtime decode.

## What Changes

- Add Feature ID `RECON-WORKSPACE-DIRECTIONAL-ANGLE-PREDICTION` to the implementation matrix and decoder support metadata.
- Add `splot-recon` chroma/no-IDIF workspace helpers that gather fully available in-storage prepared edges for one-sided pAngles `45`, `67`, and `203`, then call the existing directional-angle primitive.
- Add `splot-recon` chroma/no-IDIF workspace helpers that gather fully available in-storage logical `AboveRow[-1..w)` and `LeftCol[-1..h)` edges for middle pAngles `113`, `135`, and `157`, then call the existing middle directional-angle primitive.
- Return typed reconstruction errors for luma-IDIF attempts, missing in-storage prepared edges, invalid geometry, invalid sample type/range, and allocation failure without panics or partial target mutation. Callers that start from raw AV2 pAngles continue to use the existing typed `try_from_p_angle()` constructors before calling the workspace helpers.
- Extend focused workspace tests and the existing self-contained `recon_intra_prediction_bytes` fuzz target to cover the new public workspace paths.
- Regenerate feature/status, decoder-support, and conformance coverage documents.

Non-goals: this does not implement AV2 §7.13.2.1 fallback edge synthesis, `predict_intra()` runtime dispatch, luma IDIF, MRL, angle deltas, wide-angle mapping, directional IBP, CfL/CCTX/MHCCP, palette, transform/residual handling, loop filtering, runtime `splot decode`, reference refresh, AVM/dav2d integration, external decoder wrappers, new dependencies, or new crate dependency edges. Validator diagnostics are unchanged because this is a `splot-recon` workspace API change, not a bitstream validation rule.

## Capabilities

### New Capabilities

- `recon-workspace-directional-angle-prediction`: chroma/no-IDIF workspace handoff for source-backed one-sided and middle directional-angle prediction over fully available in-storage prepared edges, with luma rejected until IDIF is implemented.

### Modified Capabilities

- `decoder-support`: record the new source-backed workspace directional-angle support row and keep broad decoder rows partial or unsupported.
- `conformance`: record the extended bounded fuzz evidence for the workspace directional-angle public APIs without broad AV2 runtime conformance claims.

## Impact

- Affected code: `crates/splot-recon/src/workspace.rs`, `crates/splot-recon/src/error.rs`, `crates/splot-recon/src/workspace_tests.rs`, `fuzz/fuzz_targets/recon_intra_prediction_bytes.rs`, and status-generation inputs.
- Affected APIs: additive typed public `CurrentFrameWorkspace` helpers for one-sided and middle directional-angle prediction.
- Dependencies and systems: no new third-party dependencies, no dependency-direction change, no `splot-decode` runtime behavior change, and no validator diagnostic change.

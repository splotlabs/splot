## 1. Workspace API

- [x] 1.1 Add `RECON-WORKSPACE-DIRECTIONAL-ANGLE-PREDICTION` to workspace feature docs and matrix tracking inputs.
- [x] 1.2 Add a private workspace directional-angle module with typed `CurrentFrameWorkspace` one-sided and middle helper methods.
- [x] 1.3 Add workspace-specific missing-edge error reporting with plane, pAngle, edge, and rectangle context.

## 2. Edge Gathering

- [x] 2.1 Gather one-sided `AboveRow[0..w+h)` and `LeftCol[0..w+h)` scratch edges from fully available in-storage ranges.
- [x] 2.2 Gather middle `AboveRow[-1..w)` and `LeftCol[-1..h)` scratch edges with the logical `-1` top-left sample at slice index zero.
- [x] 2.3 Ensure all edge-availability and allocation checks complete before target mutation.

## 3. Tests And Fuzzing

- [x] 3.1 Add focused workspace unit tests for one-sided success, middle success, missing prepared edges, missing planes/out-of-bounds targets, 10-bit handoff, and no mutation on invalid prepared inputs.
- [x] 3.2 Extend `recon_intra_prediction_bytes` to exercise the new workspace directional-angle helpers in interior and random workspace cases.

## 4. Metadata And Verification

- [x] 4.1 Update implementation matrix, decoder support matrix, roadmap/status, conformance coverage, and generated status documents with proof commands.
- [x] 4.2 Run focused recon/fuzz/OpenSpec/status checks and `cargo xtask ci`.

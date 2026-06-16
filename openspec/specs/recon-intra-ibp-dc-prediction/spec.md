# recon-intra-ibp-dc-prediction Specification

## Purpose
TBD - created by archiving change recon-intra-ibp-dc-prediction. Update Purpose after archive.
## Requirements
### Requirement: IBP DC Prediction Primitive
`splot-recon` SHALL provide a scheduler-free scalar primitive for AV2 v1.0.0
§7.13.2.12 IBP DC process, tracked by Feature ID
`RECON-INTRA-IBP-DC-PREDICTION`. The primitive SHALL accept a decoded bit depth,
rectangular block size, prepared optional `LeftCol[0..h]` and `AboveRow[0..w]`
edge samples, and caller-owned strided prediction storage containing the prior
DC prediction. It SHALL validate sample type, edge lengths, edge sample ranges,
output stride, and output length before reporting success. When an above edge is
available, it SHALL blend the top `h >> 2` rows with `AboveRow[c]` using
`Ibp_Weights[log2H - 2][r]`, skipping the left `w >> 2` columns only when
`w < h` and a left edge is also available. When a left edge is available, it
SHALL blend the left `w >> 2` columns with `LeftCol[r]` using
`Ibp_Weights[log2W - 2][c]`, skipping the top `h >> 2` rows only when
`w >= h` and an above edge is also available. Each blended sample SHALL use the
AV2 `Round2(edge * (IBP_WEIGHT_MAX - s) + pred * s, IBP_WEIGHT_SHIFT)` formula.

#### Scenario: Above-edge blend modifies top rows
- **WHEN** a caller applies IBP DC prediction to an 8-bit or 10-bit block with a
  prepared above edge and no left edge
- **THEN** only rows `0..(h >> 2)` are modified
- **AND** each modified sample uses the AV2 §7.13.2.12 above-edge blend formula

#### Scenario: Left-edge blend modifies left columns
- **WHEN** a caller applies IBP DC prediction to an 8-bit or 10-bit block with a
  prepared left edge and no above edge
- **THEN** only columns `0..(w >> 2)` are modified
- **AND** each modified sample uses the AV2 §7.13.2.12 left-edge blend formula

#### Scenario: Both-edge overlap follows rectangular skip rules
- **WHEN** both prepared edges are available for a rectangular block
- **THEN** the top-row pass and left-column pass do not double-modify the
  overlap region
- **AND** the skipped region follows the AV2 `w < h` and `w >= h` conditions

#### Scenario: Invalid primitive inputs return typed errors
- **WHEN** the edge length, edge sample range, output stride, output length, or
  sample type is invalid for the requested block and bit depth
- **THEN** the primitive returns a typed `splot-recon` error without panicking,
  silently clamping invalid input, or emitting `decode/*` diagnostics

### Requirement: IBP DC Workspace Handoff
`splot-recon` current-frame workspace helpers SHALL expose a bounded in-storage
IBP DC helper that first writes §7.13.2.10 DC prediction for the target block and
then applies the §7.13.2.12 IBP DC modifier using in-storage left and/or above
neighbors when those neighbors exist. The helper SHALL validate the target
rectangle, preserve existing plane bounds and sample-range checks, and SHALL NOT
decide AV2 `enable_ibp`, `useDip`, `mode`, `UV_CFL_PRED`, tile-boundary,
transform, residual, runtime output, or reference-refresh semantics.

#### Scenario: Workspace applies IBP DC from in-storage edges
- **WHEN** a workspace block has valid samples immediately left of it and above
  it and the caller requests IBP DC prediction
- **THEN** the workspace writes the DC prediction and applies the same
  §7.13.2.12 edge blending as the direct primitive

#### Scenario: Workspace top-left block remains DC midpoint
- **WHEN** a workspace block has neither left nor above in-storage neighbors
- **THEN** the helper writes the normal DC prediction for that no-edge block
- **AND** it does not invent left-only or above-only fallback samples for future
  §7.13.2.1 availability rules

#### Scenario: Workspace rejects invalid geometry
- **WHEN** the requested workspace rectangle extends outside a plane or the
  plane does not exist for the current pixel format
- **THEN** the helper returns a typed `splot-recon` error without writing outside
  the plane

### Requirement: IBP DC Fuzz Coverage
The existing `recon_intra_prediction_bytes` fuzz target SHALL exercise the new
direct primitive and workspace helper with bounded structured inputs. The target
SHALL assert only public invariants and typed errors and SHALL NOT invoke
runtime `splot decode`, AVM, dav2d, filesystem I/O, subprocesses, or network
access.

#### Scenario: IBP DC prediction fuzz target is self-contained
- **WHEN** CI fuzz-smoke enumerates `recon_intra_prediction_bytes`
- **THEN** the target covers direct and workspace IBP DC cases without adding
  external decoder requirements or broad AV2 conformance claims


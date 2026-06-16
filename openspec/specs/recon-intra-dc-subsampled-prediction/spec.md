# recon-intra-dc-subsampled-prediction Specification

## Purpose
Define the source-backed `splot-recon` primitive and workspace handoff for AV2
v1.0.0 §7.13.2.11 DC intra prediction subsampled process.

## Requirements
### Requirement: DC Subsampled Intra Prediction Primitive
`splot-recon` SHALL provide a scheduler-free scalar primitive for AV2 v1.0.0
§7.13.2.11 DC intra prediction subsampled process, tracked by Feature ID
`RECON-INTRA-DC-SUBSAMPLED-PREDICTION`. The primitive SHALL accept a decoded bit
depth, rectangular block size, prepared `LeftCol[0..h]` and `AboveRow[0..w]`
edge samples, and caller-owned strided output. It SHALL validate sample type,
edge lengths, sample ranges, output stride, and output length before reporting
success. For dimensions greater than 32 it SHALL average every second sample in
that edge direction; otherwise it SHALL average every sample. If no edge is
available it SHALL fill the block with `1 << (BitDepth - 1)`. If at least one
sample is averaged it SHALL compute `Clip1(approx_divide(sum, count))` using the
AV2 approximate divisor path.

#### Scenario: No-edge subsampled DC uses midpoint
- **WHEN** a caller predicts an 8-bit or 10-bit block with neither left nor above
  prepared edge available
- **THEN** every output sample is `1 << (BitDepth - 1)`
- **AND** no external decoder, runtime `splot decode`, filesystem I/O, or
  scheduler state is used

#### Scenario: Large edges are subsampled before averaging
- **WHEN** a caller predicts a block with height greater than 32 and a left edge
  available
- **THEN** the average uses only `LeftCol[k]` where `k` advances by 2
- **AND** the full provided left edge is still validated against the active bit
  depth

#### Scenario: Both-edge prediction uses AV2 approximate division
- **WHEN** a caller predicts a rectangular block with at least one sampled left
  value and at least one sampled above value
- **THEN** the output value is `Clip1(approx_divide(sum, count))` from AV2
  §7.13.2.11 and §7.13.3.22 rather than normal integer division

#### Scenario: Invalid primitive inputs return typed errors
- **WHEN** the edge length, edge sample range, output stride, output length, or
  sample type is invalid for the requested block and bit depth
- **THEN** the primitive returns a typed `splot-recon` error without panicking,
  silently clamping invalid input, or emitting `decode/*` diagnostics

### Requirement: DC Subsampled Workspace Handoff
`splot-recon` current-frame workspace helpers SHALL expose a bounded in-storage
subsampled DC prediction helper that uses the §7.13.2.11 sampled-sum process
when left and/or above samples are available inside workspace storage. The
helper SHALL validate the target rectangle, preserve existing plane bounds and
sample-range checks, and SHALL NOT decide AV2 `largeChroma`, `UV_CFL_PRED`,
MRL, tile-boundary, transform, residual, runtime output, or reference-refresh
semantics.

#### Scenario: Workspace predicts from in-storage edges
- **WHEN** a workspace block has valid samples immediately left of it and above
  it and the caller requests subsampled DC prediction
- **THEN** the workspace writes the predicted rectangle using the same stepped
  averaging and approximate-division behavior as the direct primitive

#### Scenario: Workspace no-edge case uses midpoint
- **WHEN** a workspace block has neither left nor above in-storage neighbors
- **THEN** the helper writes the AV2 midpoint value for the active bit depth
- **AND** it does not invent left-only or above-only fallback samples for future
  §7.13.2.1 availability rules

#### Scenario: Workspace rejects invalid geometry
- **WHEN** the requested workspace rectangle extends outside a plane or the
  plane does not exist for the current pixel format
- **THEN** the helper returns a typed `splot-recon` error without writing outside
  the plane

### Requirement: DC Subsampled Fuzz Coverage
The existing `recon_intra_prediction_bytes` fuzz target SHALL exercise the new
direct primitive and workspace helper with bounded structured inputs. The target
SHALL assert only public invariants and typed errors and SHALL NOT invoke
runtime `splot decode`, AVM, dav2d, filesystem I/O, subprocesses, or network
access.

#### Scenario: Subsampled DC prediction fuzz target is self-contained
- **WHEN** CI fuzz-smoke enumerates `recon_intra_prediction_bytes`
- **THEN** the target covers direct and workspace subsampled DC cases without
  adding external decoder requirements or broad AV2 conformance claims

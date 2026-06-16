## ADDED Requirements

### Requirement: Cardinal Directional Intra Prediction Primitive
`splot-recon` SHALL provide a scheduler-free scalar primitive for the cardinal
AV2 directional intra cases defined by AV2 v1.0.0 §7.13.2.8:
`V_PRED` (pAngle 90) SHALL copy prepared `AboveRow[0..w]` into every output row,
and `H_PRED` (pAngle 180) SHALL copy prepared `LeftCol[0..h]` into every output
column. The primitive SHALL validate bit depth, sample type, edge lengths,
output stride, output length, and computed sample ranges before reporting
success.

#### Scenario: Vertical cardinal prediction copies the above edge
- **WHEN** a caller predicts an 8-bit or 10-bit block with the vertical cardinal
  mode and a prepared above edge of width `w`
- **THEN** every output row is byte-for-sample equal to `AboveRow[0..w]`

#### Scenario: Horizontal cardinal prediction copies the left edge
- **WHEN** a caller predicts an 8-bit or 10-bit block with the horizontal
  cardinal mode and a prepared left edge of height `h`
- **THEN** every output column uses the corresponding `LeftCol[i]` value

#### Scenario: Invalid primitive inputs return typed errors
- **WHEN** the edge length, output stride, output length, sample type, or sample
  range is invalid for the requested block and bit depth
- **THEN** the primitive returns a typed `splot-recon` error without panicking

### Requirement: Cardinal Directional Workspace Handoff
`splot-recon` current-frame workspace helpers SHALL expose bounded in-storage
H/V prediction helpers that call the cardinal primitive when the required
prepared above or left edge is available. The helpers SHALL fail with typed
errors when the requested rectangle lacks the required in-storage edge, extends
outside the plane, or violates the existing workspace sample/geometry checks.

#### Scenario: Workspace predicts from an interior above edge
- **WHEN** a workspace block has valid samples immediately above it and the
  caller requests vertical cardinal prediction
- **THEN** the workspace writes the predicted block by copying that above edge
  into each row

#### Scenario: Workspace predicts from an interior left edge
- **WHEN** a workspace block has valid samples immediately to its left and the
  caller requests horizontal cardinal prediction
- **THEN** the workspace writes the predicted block by copying that left edge
  into each column

#### Scenario: Workspace rejects missing cardinal edges
- **WHEN** the requested workspace block is on the top edge for vertical
  prediction or on the left edge for horizontal prediction
- **THEN** the helper returns a typed missing-edge error without writing outside
  the plane

### Requirement: Cardinal Directional Fuzz Coverage
The existing `recon_intra_prediction_bytes` fuzz target SHALL exercise the new
cardinal primitive and workspace helpers with bounded structured inputs. The
target SHALL assert only public invariants and typed errors and SHALL NOT invoke
runtime `splot decode`, AVM, dav2d, filesystem I/O, subprocesses, or network
access.

#### Scenario: Cardinal prediction fuzz target is self-contained
- **WHEN** CI fuzz-smoke enumerates `recon_intra_prediction_bytes`
- **THEN** the target covers H/V cardinal direct and workspace cases without
  adding external decoder requirements or broad AV2 conformance claims

## ADDED Requirements

### Requirement: local decoder mission general-intra reconstruction bridge

The decoder SHALL extend `DECODE-SELECTABLE-TRANSFORM-RECORDS` with a
reconstruction bridge that reconstructs the verified NON-IntrABC general-intra DC
subset of the local decoder mission frame into a current-frame workspace. As the selectable
transform-record walk decodes each general-intra block's §5.20.7.27 coefficients,
the runtime SHALL reconstruct the supported subset (DC_PRED luma and DC chroma
over rectangular, multi-transform-per-block geometries) into a
`CurrentFrameWorkspace<u16>` in decode order, reusing the existing
`runtime_minimal_recon` §7.13.2 / §7.14.4 / §7.15.4 / §7.14.3 reconstruction
primitives, so that later blocks predict from already-reconstructed neighbours.
The public `splot decode` path SHALL remain fail-closed before any decoded frame
is emitted: the bridge is exercised only by a test-only sink driver, and the
public decode still rejects at the first active IntrABC block.

The bridge SHALL NOT write samples it cannot prove: a non-DC mode, an IntrABC
block, or a transform geometry the rectangular DC primitive does not handle leaves
that region unreconstructed (the workspace keeps its fill value there).

#### Scenario: The reconstruction bridge populates a workspace region

- **WHEN** the local decoder mission selectable transform-record walk runs with a
  reconstruction sink attached
- **THEN** the sink reconstructs the verified DC region into the workspace in
  decode order using the existing reconstruction primitives
- **AND** every reconstructed sample is within the active bit-depth range

#### Scenario: Unsupported regions stay unreconstructed, never wrong

- **WHEN** the walk reaches a block whose mode is not DC_PRED, a block that uses
  IntrABC, or a transform geometry the primitives do not cleanly reconstruct
- **THEN** the runtime leaves that region unreconstructed (it does not write
  prediction/residual samples claimed correct for that region)

#### Scenario: Public decode stays fail-closed

- **WHEN** `splot decode` is run on the local decoder mission fixture
- **THEN** the runtime still rejects with a structured
  `decode/unsupported-feature` diagnostic at the first active IntrABC block
- **AND** it does not emit a partial or garbage decoded frame as success

#### Scenario: The frame-origin DC luma block reconstructs bit-exact against the oracle

- **WHEN** the bridge reconstructs the frame-origin §5.20.5.3 `DC_PRED` 16x16 luma
  leaf (the first superblock now parses bit-exact vs AVM after the per-block
  §5.20.10.2 CCSO read)
- **THEN** every sample equals the committed AVM pre-filter reconstruction oracle
  value `68`, and the block's sample sum and FNV-1a-64 checksum match the committed
  oracle assertion — the first bit-exact local decoder mission reconstruction milestone
- **AND** the remaining first-superblock samples (SMOOTH/directional luma, the
  SMOOTH chroma leaf, and one `DC_PRED` luma leaf with a small AC residual) are
  OUTSIDE the verified DC subset and stay unreconstructed, to be reconstructed by
  follow-on prediction/residual rows

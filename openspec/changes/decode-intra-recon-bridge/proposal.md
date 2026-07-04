## Why

The local decoder mission probe parses every general-intra block of frame 0 — modes,
selectable transform records, coefficients, IntrABC vectors — through the
`DECODE-SELECTABLE-TRANSFORM-RECORDS` walk, then fails closed without ever
calling the reconstruction primitives that `general_intra.rs` already uses to
reconstruct synthetic intra fixtures. The decoded coefficient blocks
(`LumaCoeffBlock { all_zero, eob, quant }`) are produced during the walk and
discarded after their `eob`/`skip_flag` are recorded. This is the foundational
missing bridge for Milestone 1 (a bit-exact local decoder mission key frame): the parse path and
the recon primitives are both present but not connected.

## What Changes

- Add a reconstruction sink that owns a `CurrentFrameWorkspace<u16>` sized to the
  local decoder mission frame and reconstructs each NON-IntrABC general-intra luma transform
  block and 4:2:0 chroma group into the workspace in walk (decode) order, reusing
  the existing `reconstruct_general_intra_block_rect_into` /
  `reconstruct_general_intra_chroma_block_into` primitives.
- Reconstruction is gated to the verified subset (DC_PRED luma + DC chroma,
  rectangular transforms, multi-transform-per-block); any other mode, an IntrABC
  block, or a transform geometry the primitives do not cleanly handle leaves that
  region UNRECONSTRUCTED rather than emitting wrong samples.
- The public `splot decode local decoder mission` path STAYS FAIL-CLOSED: the selectable walk
  still rejects at the first active IntrABC block, so no partial/garbage frame is
  emitted as success. The bridge is exercised only by a test-only sink driver.
- Update the implementation matrix, decoder support matrix/status, and OpenSpec
  specs/tasks with the bridge wiring evidence.

## Milestone (achieved) and remaining scope

The bridge wiring and primitive reuse are complete, and with PR #497's per-block
§5.20.10.2 CCSO read the first-superblock parse is now AVM-faithful (the former
2x TX_16X32 vs 4x TX_16X16 `txb_skip` desync is resolved). Fed the now-correct
parse, the bridge reconstructs the verified DC subset to the spec-correct samples:
the frame-origin §5.20.5.3 `DC_PRED` 16x16 luma leaf reconstructs BIT-EXACT (all
`68`) against the AVM pre-filter reconstruction oracle — the first bit-exact local decoder mission
reconstruction milestone, proven by
`frontier_frame_origin_dc_block_reconstructs_bit_exact_against_prefilter_oracle`. The
bridge reconstructs every general-intra `DC_PRED` luma transform it reaches before
the IntrABC fail-closed rejection (4096 frame-wide luma samples; one 16x16 block
off only by a small AC residual the DC-flat primitive drops).

The remaining first-superblock samples are OUTSIDE the verified DC subset and stay
UNRECONSTRUCTED (not blocked-on-parse): the first-SB chroma leaf is `SMOOTH` (not
`DC`), most first-SB luma is SMOOTH/directional, and one `DC_PRED` luma leaf carries
a small non-flat AC residual. Extending bit-exact to the full superblock needs
SMOOTH/directional/CfL prediction plus full residual inverse-transform
reconstruction (follow-on rows), then chroma and IntrABC reconstruction. The sink
never writes a sample it has not proven bit-exact, and the public decode stays
fail-closed at the §7.13.3.18 IntrABC wall.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `selectable-transform-records`: add a reconstruction bridge that
  reconstructs the verified NON-IntrABC general-intra DC region of local decoder mission frame 0
  into a current-frame workspace, reusing the existing recon primitives, while
  keeping the public decode fail-closed. The frame-origin `DC_PRED` luma leaf is
  bit-exact vs the AVM pre-filter oracle (first bit-exact local decoder mission reconstruction
  milestone); non-DC and chroma samples are deferred to follow-on rows.
- `decoder-support`: update support-row requirements and proof expectations for
  the local decoder mission selectable-transform frontier after the reconstruction bridge.

## Impact

- Affected code stays within `splot-decode` runtime internals: a new
  `crates/splot-decode/src/runtime_minimal/wienerns_lr/recon.rs` module plus the
  residual-chunk decode sites in
  `crates/splot-decode/src/runtime_minimal/wienerns_lr/tx_records.rs`.
- Affected tracking files are `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, generated decoder status docs, and this
  OpenSpec change.
- No public API, CLI option, dependency graph, encoder, or external reference
  tool invocation changes are intended. The 6 MB oracle YUV is NOT committed; the
  test pins a small hash of the verified first-superblock region.
- Non-goals: IntrABC reconstruction, non-DC intra modes, deblock/CDEF/LR over the
  local decoder mission frame, decoded-frame output for the public path, reference refresh,
  AVM/dav2d byte equality for the whole frame, and successful local decoder mission decode.

## Why

The local decoder mission decoder mission now consumes the observed IntrABC mode-info and
block-vector syntax, but the local stream still stops at
`unsupported_wienerns_lr_selectable_transform_records_intrabc_prediction` before
current-frame block-copy prediction. The next useful slice should move that
frontier through a real bounded IntrABC prediction handoff rather than another
metadata-only parse step.

## What Changes

- Add a source-backed current-frame IntrABC copy primitive for checked
  in-workspace block copies over already reconstructed samples.
- Wire the local decoder mission Wiener NS LR selectable-transform path to derive admitted
  IntrABC prediction rectangles from §5.20.7.13/§5.20.7.20 block vectors and
  hand them off to the current-frame copy boundary only for the known-empty
  IntrABC MV-stack subset, without fabricating decoded samples when the live
  path still lacks a populated `CurrFrame`.
- Update structured unsupported diagnostics so the local decoder mission probe advances
  past the IntrABC prediction stop to the next unimplemented decoded-sample,
  residual, or loop-restoration frontier.
- Keep broad IntrABC, chroma prediction, residual reconstruction, loop
  filtering, output/reference refresh, and AVM/dav2d equality claims out of
  scope until separately proved.

## Capabilities

### New Capabilities
- `recon-intrabc-current-frame-copy`: Checked current-frame workspace block-copy
  primitive for bounded IntrABC prediction over already reconstructed samples.

### Modified Capabilities
- `selectable-transform-records`: Advance the local decoder mission IntrABC
  selectable-transform path from block-vector metadata handoff to bounded
  current-frame prediction handoff.
- `decoder-support`: Track the new `RECON-INTRABC-CURRENT-FRAME-COPY` support
  row and the updated local decoder mission selectable-transform frontier evidence.

## Impact

- Feature IDs: `RECON-INTRABC-CURRENT-FRAME-COPY` and
  `DECODE-SELECTABLE-TRANSFORM-RECORDS`.
- Affected crates: `splot-recon` for the workspace copy primitive and
  `splot-decode` for the local decoder mission IntrABC runtime handoff.
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, generated decoder support/status docs,
  and the local decoder mission CLI probe expectation.
- No new third-party dependencies, no crate dependency graph changes, and no
  encoder-facing changes.

## Context

The merged ac0ej3 selectable-transform path consumes the observed IntrABC
`use_intrabc`, `read_intrabc_info()`, DRL, optional precision, and NEWMV
`read_mv()` syntax, then stops at
`unsupported_wienerns_lr_selectable_transform_records_intrabc_prediction`.
The retained block vector is therefore syntactically useful but still not
output-affecting.

AV2 §7.13.3.18 defines intra block copy as block inter prediction with
`refIdx == -1`, where the reference frame is `CurrFrame`. The existing
`splot-recon::CurrentFrameWorkspace` can read and write checked rectangles but
does not yet expose a same-workspace copy primitive, and the live ac0ej3 LR path
still has unpopulated `CurrFrame`/`CdefFrame` sample shells. The next slice must
therefore separate the reusable copy primitive from the live-stream storage
frontier.

## Goals / Non-Goals

**Goals:**

- Add `RECON-INTRABC-CURRENT-FRAME-COPY` as a source-backed, checked
  current-frame workspace copy primitive for one plane and one rectangle.
- Derive luma IntrABC target rectangles, source sample envelopes, and
  subpel prediction phase from the retained ac0ej3 block geometry and
  §5.20.7.13 block vector.
- Advance the live ac0ej3 probe past the generic IntrABC prediction stop to a
  more precise missing-populated-`CurrFrame` diagnostic.
- Preserve all existing IntrABC syntax ordering, CDF updates, transform-record
  behavior, and fail-closed diagnostics for unsupported branches.

**Non-Goals:**

- Populate decoded `CurrFrame` or `CdefFrame` samples for the full ac0ej3 tile.
- Decode chroma IntrABC prediction, residual addition, loop restoration,
  output/reference refresh, or the full ac0ej3 stream.
- Implement broad IntrABC availability / `BlockDecoded` validation beyond the
  geometry checks needed for the observed luma/shared local frontier.
- Add dependencies or change crate dependency direction.

## Decisions

1. Add the actual copy primitive in `splot-recon`.

   `CurrentFrameWorkspace` already owns plane storage and validates
   rectangular reads/writes. The new helper should copy from a checked source
   `PlaneRect` to a checked target `PlaneRect` in the same plane, using a
   bounded scratch buffer before mutation so overlapping source/target
   rectangles cannot corrupt reads. This keeps sample ownership and validation
   in `splot-recon` rather than duplicating storage math in `splot-decode`.

   Alternative considered: copy directly in `splot-decode` through row views.
   That duplicates workspace internals and makes overlap/fail-atomic behavior
   harder to centralize.

2. Keep the live ac0ej3 runtime honest about unpopulated samples.

   The decoder should parse IntrABC mode-info as today, derive the luma copy
   geometry, and only dispatch to the workspace copy primitive when a populated
   current-frame workspace is available. The current live ac0ej3 path does not
   have one, so it should fail with a structured diagnostic that names the
   missing decoded `CurrFrame` sample frontier rather than pretending current
   samples exist.

   Alternative considered: fill the workspace with neutral samples and apply
   the copy. That would fabricate prediction input and produce misleading
   evidence, so it is rejected.

3. Admit checked luma prediction geometry, not sample output.

   IntrABC block vectors are retained in eighth-pel units. The observed local
   path derives the target rectangle, the source sample envelope needed by
   integer or fractional luma prediction, and the §7.13.3.17 scaling phase
   when the source envelope stays inside current-frame storage. Chroma
   subsampling, morph prediction, decoded sample population, and broad
   reference-area clipping stay fail-closed until separately proved.

## Risks / Trade-offs

- [Risk] The new diagnostic may not advance the byte offset even though the
  semantic frontier moves past prediction geometry.
  -> Mitigation: record the exact local probe reason and explain in matrix
  notes that progress is semantic when the same tile byte offset is still the
  first active IntrABC block.

- [Risk] A workspace copy API can be misused on unavailable source samples.
  -> Mitigation: keep `splot-recon` responsible only for storage bounds and
  sample copying; `splot-decode` remains responsible for AV2 availability and
  must not call it in the live ac0ej3 path until decoded samples are populated.

- [Risk] The scope could drift into full IntrABC reconstruction.
  -> Mitigation: tests and matrix wording must keep chroma, residuals, loop
  restoration, output/reference refresh, and full ac0ej3 decode unclaimed.

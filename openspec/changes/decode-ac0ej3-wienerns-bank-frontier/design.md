## Context

`parse_lr_params()` currently preserves the pre-Wiener `lr_params()` facts and
returns `StoppedBeforeWienerNsFilter` as soon as any intra plane sets
`frame_filters_on`. The live ac0ej3 key frame reaches that stop with luma
`RESTORE_WIENER_NONSEP`, `frame_filters_on[0] == 1`, `NumFilterClasses == 2`,
base q index 149, and chroma Wiener NS tools selected without frame-level
filters. AV2 5.18.7.11 then calls `read_wienerns_filter(0, 0, 0, 1)`.

The called AV2 5.20.10.6 body is fixed-coded on the frame-level
`readFrameFilters == 1` path: merge flags and length/symmetry flags use `f(1)`,
and coefficient deltas use `decode_signed_subexp_with_ref()`. The project
already implements the subexp primitive for global motion, so this brick can
reuse it instead of adding a duplicate decoder.

## Goals / Non-Goals

**Goals:**

- Parse and model the ac0ej3-proven intra luma frame-filter-bank syntax for
  `read_wienerns_filter(0, 0, 0, 1)`.
- Keep the parser honest: a consumed frame-filter bank is represented as
  explicit core model data, not as a silent discard.
- Advance the live ac0ej3 diagnostic past `unsupported_wienerns_filter` to the
  next fail-closed runtime loop-filter/tool boundary before tile symbols,
  decoded-frame allocation, or output.
- Keep `restoration.rs` from growing further by putting the Wiener NS parser and
  tables in a dedicated restoration submodule.

**Non-Goals:**

- No loop-restoration reconstruction/filter application.
- No entropy-coded LR unit parser (`readFrameFilters == 0`, `S()`/`L()`)
  support.
- No inter temporal-copy/reference-frame Wiener NS state.
- No entropy-coded LR unit bank update for tile syntax, 10-bit output, or
  successful ac0ej3 decode. The fixed-coded frame-filter parser may translate
  luma PC-Wiener matches through the existing generated table when the sequence
  does not disable that group.
- No writer support for `frame_filters_on == true`.

## Decisions

- Add a dedicated `headers/frame/restoration/wienerns.rs` submodule.
  Rationale: `restoration.rs` is already above the 1000-line soft budget. A
  submodule keeps the new AV2 5.20.10.6 model and tap tables isolated without a
  broader refactor.
- Reuse `decode_signed_subexp_with_ref()` from `global_motion`.
  Rationale: AV2 5.20.10.6 references the same subexp primitive, and the
  existing helper already has hand-vector and panic-freedom tests.
- Model only the intra frame-level path in this brick.
  Rationale: on the ac0ej3 key frame, `NumTotalRefs == 0`, so
  `search_frame_filters()` has no reference filters. The luma bank has
  `numClasses == NumFilterClasses` and the PC-Wiener group is available only
  through sequence flags; those derivations can be modeled without reference
  state.
- Keep runtime output fail-closed after header completion.
  Rationale: parsing the bank syntax does not prove loop-restoration
  reconstruction. The minimal runtime must still reject before tile mode-info
  symbol decode or output when LR/CDEF/deblocking tools are present.

## Risks / Trade-offs

- Risk: consuming the Wiener NS syntax could be mistaken for reconstruction
  support. Mitigation: keep a separate support row and runtime diagnostic that
  clearly rejects loop filters before output.
- Risk: a too-broad parser model could silently admit unproven LR shapes.
  Mitigation: gate the new parser to the intra `readFrameFilters == 1` path and
  keep unsupported branches as explicit coverage stops or non-goals.
- Risk: coefficient table transcription errors. Mitigation: cite the spec
  mirror, add hand-vector unit tests, and keep the tables isolated in the
  submodule for review.

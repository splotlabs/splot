# Change: film-grain-layer-dependency-constraints

## Feature IDs

- `AV2-5.18.10-FILM-GRAIN-STRUCTURES`
- `AV2-5.14-FILM-GRAIN`

## Why

A frame's `film_grain_config()` with `apply_grain == 1` references a film-grain
model slot `fgm_id`. § 6.17.10.1 (docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-10-1,
lines 6028-6032) makes three additional bitstream-conformance requirements beyond the
already-checked `FilmGrainPresent[ fgm_id ] == 1` availability:

- `TLayerDependencyMap[obu_mlayer_id][obu_tlayer_id][FgmTLayerId[fgm_id]] == 1`,
- `MLayerDependencyMap[obu_mlayer_id][FgmMLayerId[fgm_id]] == 1`,
- `FgmChromaIdc[fgm_id] == chroma_format_idc`.

These are residual (b) on `AV2-5.18.10-FILM-GRAIN-STRUCTURES` (validate=partial). All
three are decidable from already-modeled state: the § 5.4.1 dependency maps and
`chroma_format_idc` live on the active sequence header
(`SequenceHeaderGeneral::{mlayer,tlayer}_dependency_map`, `chroma_format_idc`), and the
referenced model's stored layer identity (`FgmMLayerId` / `FgmTLayerId` / `FgmChromaIdc`)
is already recorded per slot by the § 5.14 film-grain OBU observer (`FgmSlotRecord`). The
parallel § 7.3.8.7 multi-frame-header layer-dependency check
(`frame-header/mfh-{m,t}layer-dependency-missing`) is the exact template.

## Scope

- Spec sections: § 6.17.10.1 (frame film-grain config layer-dependency conformance);
  derived maps from § 5.4.1 / § 6.4.1; `chroma_format_idc` equality from § 6.8 LCR
  agreement (`lcr_chroma_format_idc` shall equal `chroma_format_idc`, so it is a single
  sequence-level value).
- Crates/modules: `crates/splot-validate/src/context/film_grain.rs`
  (`frame_film_grain_reference_checks`, `FgmSlotRecord`),
  `crates/splot-validate/src/context/frame_header_checks.rs` (call site, threads
  `active_sequence`).
- CLI/docs/tests: new diagnostics registered in `docs/VALIDATOR-DIAGNOSTICS.md`; matrix
  notes/status updated; `layer_dependency_core.rs` tests.

## Non-goals

- The § 7.3.8.1 random-access-point-visibility direction for film-grain references (the
  `available[]`-is-monotonic under-report) stays a named residual on
  `AV2-7.3.8-HLS-AVAILABILITY` — closed by the paired `rap-replay-film-grain-qm-references`
  change, not here.
- No change to the § 5.14 / § 5.18.10 parsers — the model identity is already recorded.
- The analogous QM § 6.17.6.2 layer-dependency constraints remain deferred (QM does not
  yet record the defining OBU's layer identity — `frame_qm_reference_checks` TODO).

## Acceptance criteria

- [ ] Implementation matrix row `AV2-5.18.10-FILM-GRAIN-STRUCTURES` updated (residual (b)
      closed; residual (a) re-stated as the only remaining validate residual).
- [ ] Public API shape is documented (no new public API; internal `FgmSlotRecord` typed).
- [ ] Validator behavior implemented only where in scope (three § 6.17.10.1 constraints).
- [ ] Diagnostics have stable rule IDs, spec sections, offsets, and messages.
- [ ] Positive tests exist (satisfied dependencies / matching chroma stay silent).
- [ ] Negative tests exist (each of the three constraints fires; unavailable model is not
      layer-checked; external/Provided suppression holds).
- [ ] `cargo xtask check-feature-status` passes.
- [ ] `cargo xtask ci` passes.

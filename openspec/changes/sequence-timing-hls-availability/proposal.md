# Change: sequence-timing-hls-availability

## Feature IDs

- `AV2-6.4-SEQUENCE-HEADER-SEMANTICS`
- `AV2-7.3.8-HLS-AVAILABILITY`

## Why

The `sequence-hls-validator-coverage` change completed the §5.4 sequence-header
parser and the first HLS state, but deliberately deferred two stateful items that
depend on syntax not yet modeled:

1. **Cross-embedded-layer timing consistency** (§6.4.12). `timing_info()` is parsed
   (`AV2-5.4.12-TIMING-INFO`) but is not reached from `sequence_header_obu()`; it is
   referenced by the content-interpretation OBU via `ci_timing_info_present_flag`.
   Until `AV2-5.15-CONTENT-INTERPRETATION` is parsed, no timing values are available
   to compare across embedded layers.
2. **A full HLS availability store** (§7.3.8). The current context tracks active
   sequence headers by extended layer and a payload fingerprint for the
   repeated-identical check, but it does not yet model availability of MSDO / MFH /
   LCR / atlas / OPS objects, caller-provided external HLS, or MFH/frame-header
   sequence-header references.

## Scope

- Spec sections: §6.4.12 (timing semantics), §7.3.8 (HLS availability), and the
  parser dependency §5.15 (content-interpretation OBU) for timing.
- Crates/modules: `splot-validate` (`context`, `checks`), `splot-core`
  (`headers`/content-interpretation parsing) as the timing dependency lands.
- CLI/docs/tests: validator diagnostics, fixtures, and matrix/STATUS updates.

## Non-goals

- No frame-header or tile-group parser, entropy/range coder, decoder, or encoder.
- No fabricated activation semantics: where availability or timing depends on
  syntax not yet parsed, keep the check bounded/partial rather than guessing.
- No external-HLS assumptions unless the caller supplies the objects explicitly.

## Acceptance criteria

- [ ] Implementation matrix rows `AV2-6.4-SEQUENCE-HEADER-SEMANTICS` and
      `AV2-7.3.8-HLS-AVAILABILITY` are updated with proof.
- [ ] Cross-embedded-layer timing-consistency diagnostics are implemented once
      `timing_info()` is reachable, or remain explicitly bounded.
- [ ] A full HLS availability store (in-band + optional external) is modeled.
- [ ] Diagnostics have stable rule IDs, spec sections, offsets, and messages.
- [ ] Positive and negative/EOF tests exist.
- [ ] `cargo xtask check-feature-status` passes.
- [ ] `STATUS.md` is updated.

## Context

`parse_frame_header_core` already distinguishes coverage stops from truncation.
For loop restoration, it preserves parsed `lr_params()` prefix facts and returns
`FrameHeaderParseStatus::StoppedBeforeWienerNsFilter` when AV2 5.18.7.11 reaches
`read_wienerns_filter(plane, 0, 0, 1)`. The minimal runtime currently collapses
that status into the generic `incomplete_frame_header` unsupported feature.

## Goals / Non-Goals

**Goals:**

- Preserve the existing fail-closed behavior before tile mode-info decode,
  sample allocation, reference retention, hash, raw, or Y4M output.
- Emit a precise unsupported-feature diagnostic for the Wiener NS frontier using
  `DECODE-WIENERNS-FRONTIER`.
- Keep the local decoder mission regression pinned to byte offset 74 and to the precise
  parser status reached on current main.

**Non-Goals:**

- No implementation of `read_wienerns_filter()`, `search_frame_filters()`,
  `predict_group()`, Wiener NS CDF reads, loop-restoration reconstruction, or
  successful local decoder mission decode.
- No parser status, crate dependency, CLI parsing, or output serialization API
  changes.

## Decisions

- Match `FrameHeaderParseStatus::StoppedBeforeWienerNsFilter` inside
  `ensure_intra_header_complete`.
  Rationale: this keeps the decision at the existing runtime boundary that
  already decides whether a parsed key frame may proceed. Alternative:
  introduce a new parser error, rejected because the current parser status is a
  deliberate coverage stop, not malformed input.
- Use spec section `5.18.7.11` on the emitted diagnostic.
  Rationale: the runtime reaches the `lr_params()` call site that invokes
  `read_wienerns_filter(plane, 0, 0, 1)`; the notes cite `5.20.10.6` as the
  unimplemented subroutine body without making the diagnostic point at syntax
  the runtime has not entered.
- Keep all existing non-Wiener incomplete-header statuses on the old generic
  fallback.
  Rationale: only the Wiener NS stop is proven by the local decoder mission stream in
  this brick.

## Risks / Trade-offs

- Risk: the new diagnostic could be mistaken for Wiener NS support.
  Mitigation: matrix/support/OpenSpec notes explicitly mark it as fail-closed
  frontier tracking only and list Wiener NS parsing/reconstruction as non-goals.
- Risk: old rows still mention the former live gate.
  Mitigation: update adjacent local decoder mission and minimal-tier notes so generated status
  documents point to the current `unsupported_wienerns_filter` frontier.

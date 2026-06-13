# validator delta: inter-ref-frame-scale-ratio

Advances `AV2-6.17.2-FRAME-HEADER-INFO-SEMANTICS` by enforcing the § 6.17.2 inter-frame
reference-scaling-ratio constraint from already-modeled frame-header and §7.23 reference state.

## ADDED Requirements

### Requirement: inter frame reference scaling stays within the §6.17.2 bounds

The validator SHALL, for an inter frame whose resolved FrameWidth/FrameHeight is known
(`core.frame_size`), verify for each explicit-reference-map `ref_frame_idx[i]` whose §7.23
slot the modeled buffer PROVES valid (`SlotState::Valid`, so `RefFrameWidth`/`RefFrameHeight`
are known) the four § 6.17.2 conditions
(docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-2, mirror :4638-4644):
`2*FrameWidth >= RefFrameWidth`, `2*FrameHeight >= RefFrameHeight`,
`FrameWidth <= 16*RefFrameWidth`, `FrameHeight <= 16*RefFrameHeight`. A violation of any
condition produces the error diagnostic `frame-header/ref-frame-scale-ratio` (one per frame,
naming the first violating slot and axis), with the multiplications saturated so that integer
overflow never manufactures a violation. A reference slot the buffer cannot prove valid
(`Unknown` or `ProvenInvalid`) has no proven dimensions and is silent (a `ProvenInvalid` slot
is the `frame-header/ref-frame-idx-invalid-slot` domain); the implicit reference map
(`get_ref_frames()`, unmodeled) records no `ref_frame_idx` and is silent.

#### Scenario: a reference upscaled beyond 2x fires

- **WHEN** an inter frame's resolved FrameWidth (or FrameHeight) is less than half a
  proven-valid referenced slot's RefFrameWidth (or RefFrameHeight)
- **THEN** an error diagnostic `frame-header/ref-frame-scale-ratio` (§ 6.17.2) is produced

#### Scenario: a reference downscaled beyond 16x fires

- **WHEN** an inter frame's resolved FrameWidth (or FrameHeight) exceeds 16 times a
  proven-valid referenced slot's RefFrameWidth (or RefFrameHeight)
- **THEN** an error diagnostic `frame-header/ref-frame-scale-ratio` (§ 6.17.2) is produced

#### Scenario: scaling within bounds stays silent

- **WHEN** every proven-valid referenced slot satisfies all four inequalities (including the
  2x-upscale boundary `2*FrameWidth == RefFrameWidth`)
- **THEN** no `frame-header/ref-frame-scale-ratio` diagnostic is produced

#### Scenario: an unproven slot is not judged

- **WHEN** a referenced slot is `Unknown` or `ProvenInvalid` (no proven dimensions)
- **THEN** no `frame-header/ref-frame-scale-ratio` diagnostic is produced for that slot

## MODIFIED Requirements

(none)

## REMOVED Requirements

(none)

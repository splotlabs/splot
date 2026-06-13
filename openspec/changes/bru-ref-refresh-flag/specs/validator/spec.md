# validator delta: bru-ref-refresh-flag

Lands the §6.17.2 `use_bru` refresh-mask-bit conformance clause on
`AV2-6.17.2-FRAME-HEADER-INFO-SEMANTICS`.

## ADDED Requirements

### Requirement: a use_bru frame refreshes the slot it backward-updates

The validator SHALL, for an inter frame with `use_bru == 1`, verify the §6.17.2 requirement
(docs/spec/av2/1.0.0/06-syntax-structures-semantics.md :4596) that
`refresh_frame_flags & (1 << ref_frame_idx[bru_ref])` is non-zero, firing
`frame-header/bru-ref-refresh-flag-unset` when it is zero. The check uses only parsed header
state (`refresh_frame_flags`, `bru_ref`, `ref_frame_idx[bru_ref]`); `bru_ref` is bounds-checked
against the recorded `ref_frame_idx` and the shift is guarded against an out-of-range slot
index, so it never panics.

#### Scenario: a BRU frame that does not refresh its bru_ref slot

- **WHEN** `use_bru == 1` and `refresh_frame_flags & (1 << ref_frame_idx[bru_ref]) == 0`
- **THEN** an error diagnostic `frame-header/bru-ref-refresh-flag-unset` (§6.17.2) is produced

#### Scenario: a BRU frame that refreshes its bru_ref slot stays silent

- **WHEN** the `refresh_frame_flags` bit for `ref_frame_idx[bru_ref]` is set
- **THEN** no `frame-header/bru-ref-refresh-flag-unset` diagnostic is produced

## MODIFIED Requirements

(none)

## REMOVED Requirements

(none)

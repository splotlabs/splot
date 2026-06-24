# ac0ej3 SDP CflAllowedInSdp Frontier Specification

## Purpose
Define the fail-closed ac0ej3 Wiener NS loop-restoration frontier that retains
SDP `CflAllowedInSdp` state for chroma mode-info synchronization and wires the
next luma/shared intra prelude transform-record boundary without claiming
decoded output.

## Requirements

### Requirement: ac0ej3 SDP CflAllowedInSdp Frontier

The decoder SHALL track `DECODE-AC0EJ3-SDP-CFL-ALLOWED-FRONTIER` as a partial
runtime prerequisite for the ac0ej3 Wiener NS LR path. During intra SDP
partition traversal, the runtime SHALL retain the AV2 §5.20.3.1
`CflAllowedInSdp` state derived from the top luma and chroma partition decisions
and SHALL use it when decoding AV2 §5.20.5.6 chroma mode-info for SDP
`CHROMA_PART` leaves. When `CflAllowedInSdp == 0`, the runtime SHALL NOT read
the `is_cfl` or MHCCP syntax that §5.20.5.6 disables for that leaf.

#### Scenario: SDP chroma leaf skips disabled CfL syntax

- **WHEN** an intra SDP `CHROMA_PART` leaf is reached with `CflAllowedInSdp == 0`
- **THEN** chroma mode-info decoding treats `cflAllowed` and MHCCP allowance as
  false for that leaf
- **AND** it reads the next coded symbol as `uv_mode`, not as `is_cfl`
- **AND** it remains fail-closed before decoded frame samples or output

#### Scenario: Local ac0ej3 advances past the uv-mode desync gate

- **WHEN** the local ac0ej3 mission stream reaches the current SDP chroma
  transform-record frontier
- **THEN** the runtime no longer stops at
  `unsupported_wienerns_lr_live_transform_record_uv_mode`
- **AND** it stops at the next structured unsupported frontier before output

#### Scenario: No broad chroma reconstruction claim

- **WHEN** `CflAllowedInSdp` state is retained and applied
- **THEN** the decoder SHALL NOT claim CfL prediction, MHCCP prediction, decoded
  chroma samples, decoded `CurrFrame` or `CdefFrame` samples, `FilterClass`
  retention, loop-restoration filtering/output, reference refresh,
  AVM/dav2d byte equality, or successful ac0ej3 decode

### Requirement: ac0ej3 Intra Prelude Transform Frontier

The decoder SHALL track `DECODE-AC0EJ3-INTRA-PRELUDE-TX-FRONTIER` as a partial
runtime prerequisite for the ac0ej3 Wiener NS LR selectable transform-record
path. Before reading luma mode, chroma mode, or transform-record syntax from a
luma/shared intra leaf, the runtime SHALL consume the supported AV2 §5.20.5.3
prelude symbols in spec order: zero `use_intrabc` when coded by the frame/block
facts, CDEF strength-index syntax when §5.20.10.1 requires a CDEF unit value,
and delta-Q syntax when §5.20.5.11 requires a superblock delta. The runtime
SHALL reject unsupported mode/coeff/filter tools before tile decode unless this
path intentionally consumes their syntax, and SHALL reject chroma-offset leaves
before deriving chroma residual coordinates from the luma leaf.

#### Scenario: Luma leaf prelude stays synchronized before tx partition

- **WHEN** an ac0ej3 luma/shared intra leaf has coded `use_intrabc`, CDEF, and
  delta-Q prelude syntax before luma mode
- **THEN** the LR transform-record handoff reads those symbols before
  `read_intra_y_mode`
- **AND** it reads AV2 §5.20.6 transform partition syntax from the same symbol
  position as the reference decoder
- **AND** it remains fail-closed before decoded frame samples or output

#### Scenario: Unsupported tools are rejected before tile symbols

- **WHEN** a frame enables a mode, coefficient, or filtering tool that can insert
  unmodelled tile syntax before or inside the LR transform-record handoff
- **THEN** the runtime rejects it before reading tile mode/coeff symbols
- **AND** the diagnostic identifies the unsupported frontier instead of reporting
  a populated LR transform-record frontier from a desynchronized tile

#### Scenario: Chroma-offset leaves do not fabricate chroma coordinates

- **WHEN** partition traversal reaches a chroma-offset leaf whose chroma residual
  belongs to an ancestor chroma block
- **THEN** the LR transform-record handoff rejects the leaf before deriving U/V
  transform size or start coordinates from the luma leaf

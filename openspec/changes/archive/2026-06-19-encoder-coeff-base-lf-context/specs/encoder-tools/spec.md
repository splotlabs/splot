## ADDED Requirements

### Requirement: Encoder coeff_base low-frequency luma context

The encoder SHALL provide a private `coeff_base` low-frequency luma context
derivation tracked by `ENC-COEFF-BASE-LF-CONTEXT`. Given a low-frequency luma
coefficient's scan position, the adjusted transform geometry, the transform class,
the scan index `c`, and the per-block `Level[]` magnitudes, it SHALL return the
AV2 §8.3.2 `coeff_base` low-frequency context, mirroring the decoder's
`CoeffBaseContext` low-frequency luma branch: the `SIG_REF_DIFF_OFFSET`
neighbour-sum with the low-frequency `magLimit` (5 for the near-DC samples, else
3), `ctx = (mag + 1) >> 1`, and the §8.3.2 low-frequency luma context mapping. It
SHALL import the shared `splot-core` §9 `SIG_REF_DIFF_OFFSET` table, SHALL be total
and panic-free (saturating geometry; out-of-range or short-slice neighbours
contribute 0), and SHALL be loaded but unread (no token emission, no CDF, no
packet). It SHALL NOT derive chroma, parity-hidden DC, or high-frequency contexts.

#### Scenario: DC context for a single low-level AC neighbour

- **WHEN** the context is derived for the DC position (scan index 0, pos 0) of a
  4x4 luma block whose only nonzero neighbour is an AC coefficient of level 1 at
  pos 1
- **THEN** the returned context SHALL be 1 (neighbour magnitude 1 → `ctx = 1` →
  low-frequency `c == 0` band `ctx.min(8)`).

#### Scenario: Neighbour magnitudes are clamped and banded

- **WHEN** the context is derived with representative `Level[]` inputs spanning the
  three 2D low-frequency bands (`c == 0`, `row + col < 2`, otherwise)
- **THEN** the returned context SHALL match the §8.3.2 low-frequency luma mapping
  with the near-DC `magLimit` of 5 applied to the summed neighbours.

#### Scenario: Out-of-range neighbours do not panic

- **WHEN** the context is derived for a position whose neighbour offsets fall
  outside the transform bounds or the `Level[]` slice
- **THEN** those neighbours SHALL contribute 0 and the derivation SHALL return a
  context without panicking.

#### Scenario: The primitive emits nothing

- **WHEN** the `coeff_base_lf` luma context primitive is available in `splot-encode`
- **THEN** it SHALL remain loaded but unread, producing no token, CDF row, or coded
  packet
- **AND** no documentation or matrix row SHALL claim multi-coefficient emission or
  Baseline Encoder Profile v1 output from it.

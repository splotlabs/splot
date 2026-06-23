# coeff-fsc-quant-pass Specification

## Purpose

Capture the completed OpenSpec requirements synchronized for `coeff-fsc-quant-pass`.

## Requirements
### Requirement: FSC quant pass
The decoder SHALL provide a crate-private loaded-but-unwired FSC/IDTX coefficient
sign/quant pass tracked by `DECODE-COEFF-FSC-QUANT-PASS`. The pass SHALL consume
a completed `DECODE-COEFF-FSC-LEVEL-PASS` result, walk the checked `0..segEob`
entries in forward order, read `idtx_sign` when local `Level[row][col]` is
nonzero, write local `QuantSign[]` before later sign contexts can observe it,
call AV2 §5.20.7.28 `read_quant` with `isHidden = 0`, `maxLevel =
NUM_BASE_LEVELS + COEFF_BASE_RANGE + 1`, and `allowTcq = 0`, and write signed
`Quant[pos]` values in the same per-entry order.

#### Scenario: FSC quant state is produced
- **WHEN** the FSC quant pass receives a valid level pass and enough symbol and
  literal bits for every reached sign and `read_quant` extended path
- **THEN** it returns per-entry quant reads, signed `Quant[pos]` writes, final
  `culLevel`, final `dcCategory`, and the updated local block state without
  committing tile context lines or invoking reconstruction

#### Scenario: FSC sign and quant reads are interleaved
- **WHEN** a checked FSC block reaches an extended `read_quant` before a later
  nonzero coefficient's `idtx_sign`
- **THEN** the pass consumes the extended `read_quant` bits before reading that
  later `idtx_sign`

#### Scenario: Static FSC quant validation is fail-atomic
- **WHEN** caller-resolved scan or block geometry facts are invalid before any
  second-loop sign or `read_quant` syntax would be consumed
- **THEN** the pass returns a typed error without consuming additional symbol bits
  and without mutating local `Quant[]` state

#### Scenario: FSC quant remains unwired
- **WHEN** runtime `coeffs()` or broader `decode_tile()` execution is considered
- **THEN** this change SHALL NOT claim runtime support, decoded output changes,
  dequantization, reconstruction, or tile context commits for FSC blocks

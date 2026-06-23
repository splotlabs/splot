## Why

The ordinary non-FSC quant-pass composer can parse `read_quant` syntax and write
signed `Quant[]`, and a separate helper can derive AV2 § 5.20.7.27 `maxLevel`
records. The composer still accepts those per-coefficient `maxLevel` records as
caller facts. A small handoff helper removes that caller fact while staying
loaded-but-unwired.

## What Changes

- Add Feature ID `DECODE-COEFF-QUANT-PASS-MAXLEVEL-HANDOFF`.
- Add a crate-private helper that derives `maxLevel` inputs from checked scan
  entries, caller-resolved plane and transform class, and the quant-pass hidden
  parity flag.
- Delegate to the existing quant-pass composer after converting derived records
  into quant-pass inputs.
- Keep sign-source, base-symbol, hidden-parity, `sumAbs1`, TCQ, lossless,
  runtime `coeffs()`, dequantization, reconstruction, and output bytes
  unchanged.

## Capabilities

### New Capabilities

- `coeff-quant-pass-maxlevel-handoff`: crate-private ordinary non-FSC quant-pass
  wrapper that derives `maxLevel` before running the quant pass.

### Modified Capabilities

- `decoder-support`: records the handoff row and keeps broad coefficient-loop
  runtime support partial.

## Impact

- Affected code: `crates/splot-decode/src/tile_payload/coeff_loop/quant_pass.rs`.
- Affected docs/status: `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, generated feature/support/spec coverage
  docs, and `docs/DECODER-ROADMAP.md`.
- Public APIs, crate dependencies, and runtime decode output are unchanged.

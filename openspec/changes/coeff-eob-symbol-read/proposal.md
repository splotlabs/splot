## Why

`DECODE-COEFF-EOB-VALUE-STATE` added checked nonzero-EOB arithmetic, but the
coefficient loop still does not consume the existing `eob_pt_*` and `eob_extra`
CDF rows. The next aligned step is a narrow symbol-read helper that turns those
syntax elements and literal refinement bits into the checked EOB value.

Feature ID: `DECODE-COEFF-EOB-SYMBOL-READ`.

## What Changes

- Add crate-private `splot-decode` coefficient-loop helper(s) that read the
  active `eob_pt_*` CDF row, any size-specific `eob_pt_*_extra` literal bits,
  `eob_extra`, and any `eob_extra_bit` literals required by AV2 § 5.20.7.27.
- Feed the decoded pieces into the existing `nonzero_coeff_eob` helper.
- Add focused tests proving selected CDF rows are consumed/mutated according to
  the symbol decoder update policy and invalid selectors fail before state
  mutation.
- Track the new partial decoder-support and implementation-matrix rows, with
  generated docs refreshed.
- Do not wire this into the minimal flat-intra trace, walk coefficient scan
  order, read coefficient base/br/sign symbols, write nonzero `Quant[]`, or
  change decoded output in this change.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `decoder-support`: add the partial `coeff-eob-symbol-read` row for the
  checked § 5.20.7.27 nonzero-EOB symbol-read helper.

## Impact

- Affected code: `crates/splot-decode/src/tile_payload/coeff_loop.rs` and
  focused tests for the tile CDF symbol-read boundary.
- Affected tracking/docs: `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, generated decoder support/status/coverage
  docs, and decoder-conformance coverage grouping.
- APIs/dependencies: no public API changes and no new dependencies.
- Diagnostics: no new user-facing diagnostics; this is crate-private decoding
  infrastructure and remains behind the existing unsupported runtime frontier.

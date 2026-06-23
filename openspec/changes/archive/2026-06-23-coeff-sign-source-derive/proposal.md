## Why

The derived-base ordinary coefficient composer still receives caller-supplied
sign sources even though the sign-source branch is a direct AV2 §5.20.7.27
decision from post-first-pass `Level[]`, hidden-parity state, plane, transform
class, and DC-context buffers. Deriving those inputs is the next narrow
decoder-conformance brick before runtime `coeffs()` can stop fabricating sign
facts.

Feature ID: `DECODE-COEFF-SIGN-SOURCE-DERIVE`.

## What Changes

- Add a crate-private ordinary non-FSC helper that derives `CoeffSignReadInput`
  records from checked scan entries, local `Level[]`, hidden-parity summary
  facts, plane, transform class, and above/left DC context slices.
- Select `dc_sign`, `dc_sign_horz_vert`, raw `sign_bit`, or no sign source
  according to AV2 §5.20.7.27 while using the existing §8.3.2 `dc_sign_ctx`
  helper for the luma DC row.
- Keep existing sign-reading and ordinary-pass composition APIs staged; this
  change derives inputs but does not wire them into runtime `coeffs()`.
- Update implementation/support matrices, decoder conformance coverage,
  roadmap notes, generated status docs, and OpenSpec artifacts.

## Capabilities

### New Capabilities

- `coeff-sign-source-derive`: ordinary non-FSC coefficient sign-source
  derivation from post-level coefficient state, hidden parity, transform class,
  plane, and DC context buffers.

### Modified Capabilities

- `decoder-support`: record the new partial decoder boundary and clarify that
  runtime `coeffs()` integration remains unsupported.

## Impact

Affected code is limited to crate-private `splot-decode` coefficient-loop sign
helpers, tests, and tracking documents. There are no public API, dependency,
licensing, encoder, CLI, fixture-output, or crate graph changes. The minimal
runtime decode path remains unchanged because real nonzero coefficient blocks
still do not call the ordinary coefficient-pass composer.

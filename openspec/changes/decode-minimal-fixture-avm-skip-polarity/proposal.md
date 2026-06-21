## Why

The committed `syn-flat-intra-64x64-minimal.ivf` conformance fixture was not
AVM-conformant. Its tile payload was hand-retimed so the minimal decoder's frozen
block-symbol trace would accept it, but the luma and V `txb_skip` (`all_zero`)
symbols were coded with **inverted** polarity versus AV2 § 5.20.7.27 / AVM
(`av2/decoder/decodetxb.c`), where the decoded symbol *is* `all_zero` and a
*skipped* (no-coefficient) transform block carries `all_zero == 1`. `avmdec`
rejects the old fixture as a corrupt frame, so the "minimal tier" hash/raw/Y4M
contract was circular — validated only against splot's own (wrong) decoder, never
against an oracle. The frozen block-symbol trace asserted `all_zero == 0` for the
skip branch, baking the inversion into decoder code.

An AVM/dav2d cross-check confirmed the bug and the fix:

- `avmdec` and `dav2d` both reject the old hand-retimed fixture.
- A real `avmenc` 64x64 intra frame of a flat input quantizes its luma block to an
  all-zero residual (a `all_zero == 1` skipped transform) but always codes a real
  chroma residual, so a conformant stream is *not* an all-planes skip and does not
  match the frozen tier's hand-traced "all planes all-zero" sequence.
- splot's **general** intra path already decodes such a conformant luma-skip stream
  byte-for-byte identically to `avmdec` and `dav2d`, so the general path's
  `all_zero == 1` skip branch is correct; only the frozen tier carried the
  inverted polarity.

## What Changes

- Correct the frozen block-symbol trace (`block_symbol.rs::consume_trace`) so the
  luma and V `txb_skip` reads assert the AVM `all_zero == 1` skip polarity, with
  comments citing § 5.20.7.27 / AVM `decodetxb.c`.
- Replace the committed `syn-flat-intra-64x64-minimal.ivf` with an
  `avmenc`-generated, `avmdec`/`dav2d`-conformant luma-skip stream (base_q_idx 210;
  broad tools plus intra DIP and tx-partition disabled) whose single 64x64 luma
  transform block is `all_zero == 1` (skipped) over a real coded chroma residual.
  It routes off the frozen `base_q_idx == 255` path into the general intra path
  (`DECODE-GENERAL-INTRA-FRAME-RECON`) and decodes bit-for-bit identically to
  `avmdec` and `dav2d`.
- Commit the oracle raw output as `syn-flat-intra-64x64-minimal.raw` and record the
  `avmdec`/`dav2d` byte-for-byte agreement in `docs/LOCAL-REFERENCE-EVIDENCE.toml`.
- Update the minimal-tier hash/raw/Y4M expectations to the general-path output
  (luma flat 128 skip; chroma a real coded residual; `splot-dfh-sha256-v1`
  `92c4477c…`). Add a `general_intra_tests` test proving the `all_zero == 1` luma
  skip branch decodes bit-exact through the real decoder.
- Repoint the frozen-frontier tests: the frozen trace is now exercised by a
  legacy-rejection regression test (proving the retired inverted-polarity payload
  is rejected by the corrected assertion) plus the existing synthetic block-symbol
  unit tests. The frozen `base_q_idx == 255` path remains in code but has no
  committed conformant fixture (no AVM-producible stream is an all-planes skip).

Non-goals:

- No change to the general intra decode algorithm (it was already correct); no new
  decode tools, partition shapes, in-loop filters, inter prediction, or tiles.
- No removal of the frozen minimal-tier code (it stays per the standing decision);
  only its inverted assertion is corrected and its conformance fixture retired.
- No AVM/dav2d source, dependency, wrapper, script, or required CI job; the oracle
  cross-check is recorded as portable evidence metadata only.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `decoder-support`: records that the committed minimal-tier fixture is now an
  AVM/dav2d-conformant luma-skip stream decoded by the general intra path, and that
  the frozen block-symbol trace asserts the AVM `all_zero == 1` skip polarity.

## Impact

- `crates/splot-decode/src/tile_payload/block_symbol.rs`
- `crates/splot-decode/src/tile_payload/runtime_frontier.rs`
- `crates/splot-decode/src/runtime_hash.rs`
- `crates/splot-decode/src/runtime_raw.rs`
- `crates/splot-decode/src/runtime_y4m.rs`
- `crates/splot-decode/src/runtime_minimal/general_intra_tests.rs`
- `crates/splot-cli/tests/decode_cli.rs`, `decode_raw_cli.rs`, `decode_y4m_cli.rs`
- `tests/conformance/vectors/valid/syn-flat-intra-64x64-minimal.ivf` (+ `.raw`)
- `tests/conformance/manifest.toml`
- `docs/LOCAL-REFERENCE-EVIDENCE.toml`
- `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`
- generated status/coverage docs and the audit ledger
- `openspec/specs/decoder-support/spec.md`

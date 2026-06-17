## Why

The last two hardcoded context literals in the minimal flat-intra block-symbol
trace are the `all_zero` contexts for the luma `txb_skip` and the V-plane
`v_txb_skip` symbols. An empirical probe confirmed both literals (luma 0, V 3)
are *forced* by the conformant fixture — flipping either fails the
no-output-change snapshot — so they are the correct values, not luck. This change
implements the § 8.3.2 `all_zero` context *formula* and uses it in the trace with
the level-context contribution derived for the first transform block, so the
literals become spec-grounded computations rather than bare constants.

## What Changes

- Add `txb_skip_ctx_luma` and `v_txb_skip_ctx` to
  `crates/splot-decode/src/tile_payload/cdf/block_context.rs`, implementing the
  § 8.3.2 `all_zero` context for plane 0 (luma) and plane 2 (V) over
  caller-supplied level context and transform-block geometry:
  - luma: `fsc_active -> TXB_SKIP_CONTEXTS - 1`; `tx_fills_block -> 0`; else
    `(Min(top,4) + Min(left,4) + 3) >> 1`.
  - V: `(above != 0) + (left != 0)`, `+3` if the chroma block exceeds the
    transform, `+6` if `EobU != 0`.
- Use them in `block_symbol.rs::consume_trace`: the level context is *derived* as
  0 for the first transform block (no prior decoded blocks; out-of-frame
  neighbours), and the U plane was decoded all-zero so `EobU == 0`. The
  transform-block geometry (`tx_fills_block`, `chroma_block_larger_than_tx`)
  remains caller-asserted to the fixture-forced values with a `TODO(spec)` until
  the § 5.20 transform-block syntax supplies it.
- The existing no-output-change snapshot proves the computed contexts (luma 0,
  V 3) match the previous literals.
- Update feature tracking and OpenSpec artifacts.

Non-goals:

- No § 5.20 transform-block syntax, no `AboveLevelContext` / `LeftLevelContext`
  (or DC-context) buffer infrastructure, no coefficient decode, no `EobU`
  derivation, no `fsc_mode` / `txSz` / residual-geometry derivation, and no
  U-plane `txb_skip` branch.
- No partition decisions, full § 8.3 CDF selection, `decode_tile()`,
  reconstruction, hashes, Y4M output, reference refresh, or runtime support
  changes.
- No AVM/dav2d source, dependency, wrapper, script, CI job, or required test.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `decoder-support`: records that the tile CDF selection boundary now computes
  the `all_zero` (`txb_skip` / `v_txb_skip`) block-symbol contexts via the § 8.3.2
  formula (with the first-block level context derived and the transform geometry
  deferred), while the boundary remains partial.

## Impact

- `crates/splot-decode/src/tile_payload/cdf/block_context.rs`
- `crates/splot-decode/src/tile_payload/block_symbol.rs`
- `docs/IMPLEMENTATION-MATRIX.toml`
- generated status/coverage docs
- `openspec/specs/decoder-support/spec.md`

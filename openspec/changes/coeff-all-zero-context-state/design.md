## Context

The previous state-buffer brick introduced `TileCoeffContextState`, which owns
the three-plane `AboveLevelContext`, `LeftLevelContext`, `AboveDcContext`, and
`LeftDcContext` lines. The earlier `tile-txb-skip-context-derivation` brick
introduced pure §8.3.2 formula helpers for luma `txb_skip` and V `v_txb_skip`,
but the minimal trace still supplies the level/DC reductions as literals.

## Decisions

- Keep `coeff_state.rs` storage-focused. A new `coeff_loop.rs` module composes
  state slices with CDF context formula helpers.
- Support only the luma and V branches already present in the minimal trace.
  The U-plane branch and broad `coeffs()` loop remain separate work.
- Treat transform geometry facts (`tx_fills_block`,
  `chroma_block_larger_than_tx`, `fsc_active`, `EobU`) as caller-resolved
  inputs. This keeps the brick honest until transform-block syntax derives them.
- Bound reductions by the owned slice tails (`get(start..).unwrap_or(&[])` and
  `take(count)`) so malformed caller counts cannot spin.

## Verification

- Unit tests cover zero state, nonzero above/left reductions, V level/DC
  combination, out-of-range starts, and pathological counts.
- The existing minimal block-symbol frontier snapshot remains unchanged, proving
  no output change for the current fixture.

## Non-Goals

- No coefficient symbols beyond the existing `txb_skip` / `v_txb_skip` trace
  reads.
- No `Quant[]`, scan order, EOB, sign, `read_quant`, dequantization,
  reconstruction, or output change.

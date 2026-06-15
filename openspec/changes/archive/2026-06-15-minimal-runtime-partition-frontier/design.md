## Context

The minimal runtime already validates one committed 64x64 intra IVF fixture and
then emits deterministic flat hash/Y4M output. Its tile trace currently reads the
root `do_split` partition symbol directly from generated default CDF rows, even
though `DECODE-TILE-PARTITION-TRAVERSAL-BOUNDARY` now reaches the same root
`decode_block()` frontier with spec-cited §5.20.3.1 traversal and §8.3 partition
CDF selection.

The runtime still cannot claim full `decode_tile()` or `decode_block()` support.
After the root partition frontier, the existing trace only checks the five
already-modeled flat intra/block symbols and then calls `exit_symbol()`. This
change integrates the first partition decision through the traversal frontier
without changing output semantics or broadening the public success tier.

## Goals / Non-Goals

**Goals:**

- Route the minimal runtime's first partition decision through
  `tile-partition-traversal-boundary`.
- Carry a live §8.2 symbol decoder cursor from the traversal bridge so the
  existing manual flat-block trace continues from the same arithmetic state.
- Derive traversal frame facts and context state from the parsed sequence,
  frame core, and tile work unit where available.
- Keep hash/Y4M output bytes unchanged for the committed fixture.
- Update matrix/status docs as runtime integration evidence for
  `DECODE-MINIMAL-TIER-RUNTIME-SUCCESS`.

**Non-Goals:**

- No public API expansion.
- No new `splot-core` symbol checkpoint resume API.
- No broad `decode_tile()` loop, `decode_block()` syntax parser, `MiSizes`
  mutation, block reconstruction, CDF save/average behavior, reference refresh,
  scheduler changes, or AVM/dav2d integration.
- No changes to unrelated active OpenSpec changes.

## Decisions

1. **Add a crate-private live-cursor traversal bridge.**

   Keep the existing `plan_tile_partition_traversal_frontier()` plan-only API
   and implement it through a new crate-private helper that can return both the
   plan and the live `SymbolDecoder`. Runtime code uses the live cursor to
   continue the remaining five traced symbols. This avoids a wider public
   checkpoint-resume API in `splot-core`.

2. **Keep traversal internals private behind a runtime adapter.**

   `runtime_minimal.rs` should not construct private block-size constants or
   test-only context fixtures. Add a small crate-private tile-payload adapter
   that accepts a mutable `DecodeTileWorkUnit`, parsed sequence/header facts, and
   decode limits, then returns the minimal root frontier plus live cursor.

3. **Use spec-derived initial context values.**

   The runtime adapter initializes `LeftMiSizes` and `AboveMiSizes` with
   `BLOCK_256X256` as specified by `clear_left_context()` and
   `clear_above_context()` semantics. `MiSizes` is not read for the root tile
   call because top/left availability is false, but the adapter still supplies a
   bounded 2-plane grid so any accidental read remains checked.

4. **Assert the precise frontier before continuing.**

   The minimal trace requires exactly one root partition step, `PARTITION_NONE`,
   no pending children, symbol count `1`, and a `decode_block()` frontier at the
   tile origin. A mismatch returns the existing typed unsupported decode error
   instead of silently falling back to manual partition reads.

## Risks / Trade-offs

- Returning a live symbol cursor adds a lifetime-bearing helper to the private
  traversal module. Keeping it crate-private and tested avoids a public API
  commitment.
- The runtime bridge derives a minimal context for only the first root frontier.
  Broader traversal after `decode_block()` must wait for block-state mutation and
  is explicitly outside this change.
- `partition_traversal.rs` is near the source-line soft budget, so the helper
  should stay small and avoid moving runtime fact derivation into that file.

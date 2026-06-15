## Context

The current tile-payload path stops after §5.20.1 framing, §8.2 symbol
initialization, tile-local partition CDF subset creation, and one-decision
partition helpers. The missing link is a crate-private traversal surface for
AV2 §5.20.3.1 `decode_partition()` that can consume partition syntax in
recursive spec order until the first `decode_block()` frontier.

The traversal is still narrower than full `decode_tile()`: §5.20.2.1 setup,
loop-restoration/read_lr details, block syntax, prediction, residuals,
reconstruction, output, CDF copyback/averaging, and reference refresh remain
separate backlog items. The key boundary is `decode_block()`: §5.20.4.1
consumes syntax and mutates `MiSizes`, `LeftMiSizes`, and `AboveMiSizes` before
later sibling partition decisions can be read. This change therefore stops at
the first block frontier and returns enough continuation metadata for a future
block decoder to resume traversal honestly after those mutations.

## Goals / Non-Goals

**Goals:**

- Add a crate-private `tile_payload::partition_traversal` boundary for
  `DECODE-TILE-PARTITION-TRAVERSAL-BOUNDARY`.
- Advance through §5.20.3.1 child partitions in AV2 order until the first
  `decode_block()` frontier using existing helpers for
  partition size lookup, allowed-set derivation, CDF context derivation,
  partition symbol reads, and final partition decisions.
- Record deterministic traversal nodes, pending continuation children, and a
  `decode_block()` frontier with coordinates, block sizes, parent sizes, chroma
  flags, selected partition, and syntax trace.
- Use caller-provided bounded block-size context state for §8.3.2 selectors
  without mutating it in this boundary.
- Update the support/implementation matrices without promoting broad
  `tile-payload-decode` or runtime output rows.

**Non-Goals:**

- No public API or CLI success expansion.
- No full §5.20.2.1 `decode_tile()` loop integration beyond calling the
  traversal boundary from tests or crate-private work-unit helpers.
- No §5.20.4 `decode_block()` syntax parsing, `MiSizes` or neighbor-state mutation,
  mode info, transform, prediction, residual, reconstruction, filtering, output,
  reference refresh, or CDF save/average mutation.
- No multi-tile, multi-tile-group, bridge, BRU, inter, or SDP behavior beyond
  typed unsupported/residual boundaries already needed to keep the traversal
  honest.
- No AVM/dav2d integration, dependency changes, or scheduler changes.

## Decisions

1. **Use a new crate-private traversal module.**

   Add `crates/splot-decode/src/tile_payload/partition_traversal.rs` and wire it
   from `tile_payload.rs`. Keeping traversal separate from
   `partition.rs`/`partition_allowed.rs` preserves the existing single-purpose
   helpers and avoids growing one file past the source-line budget.

2. **Stop before block-state mutation.**

   The traversal input receives bounded 2-plane `MiSizes`, `LeftMiSizes`, and
   `AboveMiSizes` context state for the current frontier. It records the first
   `decode_block()` boundary but does not write the §5.20.4.1 block-size state
   entries. A later block-syntax PR must perform those mutations before asking
   traversal to continue with sibling children.

   Alternative considered: keep traversing after a leaf by writing only the
   block-size state subset. Rejected because `decode_block()` consumes syntax
   and also owns other state that future contexts may depend on; pretending the
   leaf completed would overclaim.

3. **Keep traversal inputs explicit.**

   `PartitionTraversalInput` should receive root tile geometry, frame/tile facts
   (`MiRows`, `MiCols`, `SbSize`, `NumPlanes`, subsampling, feature flags,
   `FrameIsIntra`, `MaxPbAspectRatio`, BRU-active state), immutable partition
   context state, and a mutable tile work unit. It clones symbol/CDF state
   transactionally and commits CDF rows back to the work unit only if the
   frontier is planned successfully.

4. **Stop honestly at unsupported SDP/BRU/inter surfaces.**

   Initial implementation supports the minimal intra/shared-partition traversal
   that the current runtime tier can reach. SDP luma/chroma partition split,
   extended SDP region signaling, BRU-active region decisions, bridge paths, and
   inter/mixed-region behavior should return typed traversal unsupported errors
   until a later OpenSpec change models the required state.

5. **Trace child call order instead of decoding blocks.**

   The result records the decision chain to the first `decode_block()` frontier
   and the pending siblings that remain after that frontier. Tests can assert
   ordering for `NONE`, `HORZ`, `VERT`, `SPLIT`, `HORZ_3`, `VERT_3`,
   `HORZ_4A/B`, and `VERT_4A/B` as frontier prefixes without depending on
   reconstruction.

## Risks / Trade-offs

- A future continuation API could accidentally skip required `decode_block()`
  mutations → return explicit pending children and keep this change from
  advancing past the first block frontier.
- Recursive traversal can loop or overflow on malformed table/coordinate facts →
  use checked coordinate arithmetic, bounded recursion depth/stack accounting,
  and typed errors for invalid `Partition_Subsize` or out-of-frame child calls.
- Supporting every partition form in one PR can grow too large → include child
  frontier geometry for §5.20.3.1, but keep SDP/BRU/inter/block syntax as
  explicit residuals with tests for unsupported gates.
- The runtime minimal hash path may still use its existing six-symbol trace →
  do not route public `splot decode` through the traversal until the trace and
  fixture are deliberately updated in a later runtime PR.

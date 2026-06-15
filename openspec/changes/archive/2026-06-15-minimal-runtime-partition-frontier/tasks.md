## 1. OpenSpec Scope

- [x] 1.1 Revise proposal/design/spec deltas for the existing
  `DECODE-MINIMAL-TIER-RUNTIME-SUCCESS` and
  `DECODE-TILE-PARTITION-TRAVERSAL-BOUNDARY` rows.
- [x] 1.2 Validate the change strictly before implementation.

## 2. Runtime Bridge

- [x] 2.1 Add a crate-private traversal helper that returns the existing
  frontier plan plus the live `SymbolDecoder` cursor.
- [x] 2.2 Add a minimal runtime adapter that derives traversal facts/context
  from parsed sequence, frame core, tile work unit, and decode limits.
- [x] 2.3 Replace the minimal trace's manual root partition-symbol read with
  frontier assertions, then continue the existing five-symbol trace on the live
  cursor.

## 3. Tests

- [x] 3.1 Add traversal tests proving the live cursor matches the frontier
  checkpoint and can continue after the frontier.
- [x] 3.2 Add minimal runtime regression tests for unchanged hash/Y4M output and
  partition-symbol mutation failure through the new frontier path.
- [x] 3.3 Add a focused test proving runtime context initialization uses
  `BLOCK_256X256` initial left/above values.

## 4. Documentation And Matrix

- [x] 4.1 Update `docs/IMPLEMENTATION-MATRIX.toml` and
  `docs/DECODER-SUPPORT-MATRIX.toml` with runtime bridge evidence.
- [x] 4.2 Regenerate derived decoder support/status docs if the xtask output
  reports drift.

## 5. Verification And Review

- [x] 5.1 Run targeted decode tests for the bridge and minimal runtime.
- [x] 5.2 Run `cargo xtask check-decoder-support`,
  `cargo xtask check-feature-status`, `openspec validate --all --no-interactive`,
  and `cargo xtask ci`.
- [x] 5.3 Use subagent review outputs to fix or explicitly disposition findings.

## Context

The repository already has the generic AV2 § 8.2 symbol decoder in
`splot-core`, plus a crate-private `splot-decode` tile CDF subset for the
minimal runtime path. That subset contains partition-entry rows and the minimal
block-symbol rows used by the traced one-tile fixture. The current code computes
copy/average policy and has test-only saved-CDF helpers, but runtime completion
does not yet model the AV2 lifecycle boundary:

- § 5.20.1: `tile()` runs `init_symbol(tileSize)`, `decode_tile()`,
  `exit_symbol()`, and `frame_end_update_cdf()` on the final tile group.
- § 6.19.1 / § 7.5: frame-end update copies Saved CDF rows into frame CDF rows
  and scales each row count with `(3 * count) >> 2`.
- § 8.2.2: tile-local CDF arrays are copied from frame CDF arrays at symbol
  decoder initialization.
- § 8.2.4: after successful `exit_symbol()`, final Tile CDF rows are copied or
  averaged into Saved CDF rows according to `copyCdf` / `avgCdf`.
- § 8.2.6 and § 8.3.1-§ 8.3.2: `S()` reads mutate the selected CDF row by
  reference when CDF update is enabled.

## Goals / Non-Goals

**Goals:**

- Provide a crate-private, typed lifecycle boundary for the currently supported
  CDF subset.
- Make Tile-to-Saved CDF application explicit and gated by successful
  `exit_symbol()`.
- Add frame-end subset promotion from saved rows to frame rows with § 7.5 count
  scaling.
- Keep saved/frame state transactional across runtime frontier failures.
- Preserve existing minimal hash and Y4M output bytes.

**Non-Goals:**

- Full `symbol-decoder-complete`.
- Full § 8.3 selector coverage or all § 9.3 CDF banks.
- Reference-frame CDF persistence, `load_cdfs`, `save_cdfs`, or `blend_cdfs`.
- Multi-tile scheduling semantics beyond deterministic local policy tests.
- Recursive `decode_tile()` / broad `decode_block()` traversal.
- Reconstruction, reference refresh, film grain, AVM/dav2d integration, new
  dependencies, or public APIs.

## Decisions

1. Keep the lifecycle boundary in `splot-decode`.

   Rationale: `splot-core` owns generic symbol arithmetic and generated tables,
   while CDF lifecycle depends on tile/frame decode state. Keeping the boundary
   crate-private preserves the existing dependency direction and avoids a public
   API commitment.

2. Model only the supported subset.

   Rationale: The current runtime can only reach partition rows and a small
   minimal block-symbol subset. Adding all § 9.3 CDF banks would be large,
   mostly unused, and would require separate memory/resource-limit design.
   This change should make the existing subset lifecycle-correct before adding
   more rows.

3. Gate Tile-to-Saved application on successful tile completion.

   Rationale: AV2 § 8.2.4 applies copy/average after `exit_symbol()`. Runtime
   trace mismatch, symbol/CDF errors, or padding failures must not mutate saved
   or frame state. The implementation should use local clones or explicit
   completion methods so failure paths naturally roll back.

4. Add frame-end count scaling as a method on the subset frame CDF type.

   Rationale: § 7.5 frame-end update is independent of the current minimal
   output bytes but is required CDF lifecycle behavior. A focused method can be
   unit-tested against row counts without broad decode scheduling.

## Risks / Trade-offs

- [Risk] The feature is mistaken for full CDF lifecycle support.
  Mitigation: name and matrix row must say "supported subset" and keep
  `symbol-decoder` / `tile-cdf-selection-boundary` partial.

- [Risk] Copy/average tests pass but runtime frontier still mutates state before
  failure.
  Mitigation: add tests that inject trace mismatch or `exit_symbol()` failure
  and assert saved/frame state is unchanged.

- [Risk] Row-count scaling drifts across row bundles.
  Mitigation: implement count scaling through shared row-walk helpers on the
  subset rows, covering both partition and block rows in tests.

- [Risk] The current single-tile runtime cannot prove multi-tile policy.
  Mitigation: keep multi-tile scheduling out of scope and cover policy behavior
  with deterministic local unit tests.

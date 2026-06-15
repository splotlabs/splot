## Context

`splot-decode` already has the crate-private pieces immediately below AV2 §5.20.3.2 `read_partition()`:

- `tile_payload/cdf/context.rs` derives bounded §8.3.2 selectors for partition-entry CDF rows.
- `tile_payload/cdf/partition_read.rs` routes one selected row through §8.3.1 `S()` parsing via `SymbolDecoder::read_symbol(cdf)`.
- `tile_payload.rs` remains a boundary: it does not recursively run `decode_partition()` or `decode_tile()`.

The next honest increment is a single partition-decision boundary that follows the §5.20.3.2 branch order over caller-provided facts. It must not derive the allowed partition set itself because `Partition_Subsize` and `H_Partition_Midsize` are symbolic §9.2 tables that are intentionally skipped by generated table support today.

## Goals / Non-Goals

**Goals:**
- Add a crate-private `tile_payload/partition.rs` module tracked by `DECODE-TILE-PARTITION-DECISION-BOUNDARY`.
- Model the ten §5.20.3.2 partition outcomes as a typed enum.
- Accept caller-provided allowed partitions, implied partition facts, BRU-active state, rect-type implication, and already-bounded CDF context inputs.
- Call the existing partition-entry `S()` helper only along the reached §5.20.3.2 branches.
- Read `uneven_4way_partition_type L(1)` only when the §5.20.3.2 branch order requires it.
- Return a trace describing which branch-local syntax elements were consumed so future traversal tests can assert transactionality.
- Update `docs/SPEC-MAPPING.md` with the tile partition syntax/citation surface before implementing the bitstream-affecting boundary.

**Non-Goals:**
- No `partition_implied`, `init_allowed_partitions`, `is_partition_allowed`, chroma-offset derivation, symbolic `Partition_Subsize`, or symbolic `H_Partition_Midsize` implementation.
- No recursive `read_partition()`, `decode_partition()`, `decode_tile()`, block syntax, `MiSizes` mutation, `exit_symbol()`, CDF copyback/average mutation, frame-end CDF update, reconstruction, hash/Y4M output, reference refresh, public API, CLI behavior, dependency graph, fixture, AVM/dav2d, or CI integration changes.

## Decisions

1. **Place the boundary in `crates/splot-decode/src/tile_payload/partition.rs`.**
   - Rationale: final partition decisions belong to tile payload syntax, not CDF row storage. Keeping it outside `tile_payload/cdf/` preserves the existing CDF module boundary.
   - Alternative considered: extend `partition_read.rs`. Rejected because that file is intentionally a narrow CDF-row-to-symbol handoff.

2. **Use caller-provided allowed/implied facts instead of deriving them.**
   - Rationale: the full derivation requires broader block geometry, tree/chroma state, and symbolic tables that are out of scope for this slice.
   - Alternative considered: hand-transcribe `Partition_Subsize`/`H_Partition_Midsize`. Rejected because symbolic table support needs a separate table/enum design and would exceed the smallest safe slice.

3. **Return both the final `PartitionType` and a consumed-syntax trace.**
   - Rationale: branch-local trace fields make tests prove that early returns do not advance the symbol stream or mutate CDF rows, and that each conditional symbol is consumed exactly when reached.
   - Alternative considered: return only the partition enum. Rejected because it would force tests to infer branch behavior only from bit counters and CDF mutations.

4. **Keep errors crate-private and typed.**
   - Rationale: this boundary is not yet public runtime decode behavior. Typed internal errors can later map into `decode/conformance-error`, `decode/malformed-source`, or `decode/internal-invariant` when full traversal exists.
   - Alternative considered: emit public diagnostics now. Rejected because there is no complete source offset/tile traversal context for public diagnostics in this slice.

## Risks / Trade-offs

- **Risk:** The helper might appear to claim full `read_partition()` support. → **Mitigation:** Name and matrix row explicitly say decision boundary from caller-provided facts; docs list recursive traversal and allowed-partition derivation as non-goals.
- **Risk:** Branches could consume symbols before validating caller facts. → **Mitigation:** Pre-validate allowed/implied facts and forced rect direction where possible; tests assert no stream advancement or CDF mutation on invalid facts and early returns.
- **Risk:** `L(1)` handling might accidentally use entropy decoding. → **Mitigation:** Use `SymbolDecoder::read_literal(1)` or the existing primitive that implements `L(1)` and test consumed-bit behavior separately from `S()` reads.
- **Risk:** `tile_payload.rs` is near the source-line soft budget. → **Mitigation:** Add only `mod partition;` there and keep implementation/tests in the new module.

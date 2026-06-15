## Context

PR #167 moved the minimal runtime from a hardcoded root partition symbol read to the crate-private tile partition traversal frontier. After that frontier, `runtime_minimal.rs` still reads the traced flat block symbols directly: `y_mode_set`, `y_mode_index`, luma/U all-zero transform, `uv_mode`, and V all-zero transform, followed by `exit_symbol()`. The scoped spec anchors are AV2 §5.20.4.1, §5.20.5.1/§5.20.5.3/§5.20.5.5/§5.20.5.6, §5.20.6.1-§5.20.6.2, §5.20.7.23-§5.20.7.24/§5.20.7.27, §8.2.4/§8.2.6, §8.3.1-§8.3.2, and generated §9.3 default CDF rows.

Those reads are AV2 tile/block syntax, not runtime output policy. Keeping them in `runtime_minimal.rs` makes the supported minimal tier work, but leaves the tile-payload boundary with no explicit block-symbol trace contract after the partition frontier.

## Goals / Non-Goals

**Goals:**

- Add a crate-private minimal block-symbol trace frontier owned by `splot-decode::tile_payload`.
- Consume the same traced all-flat intra 64x64 block symbols currently consumed by `runtime_minimal.rs`.
- Preserve the exact minimal hash/Y4M behavior, diagnostics, output hashes, and fixture bytes.
- Add regression tests for success, symbol mismatch, parse failure, and `exit_symbol()` validation through the new frontier.
- Update matrix/docs to record `DECODE-MINIMAL-BLOCK-SYNTAX-FRONTIER`.

**Non-Goals:**

- No broad `decode_block()` or `decode_tile()` implementation.
- No full `mode_info()`, transform syntax, coefficient parsing, reconstruction, loop filtering, reference refresh, film grain, or metadata-hash verification.
- No broad §8.3 CDF bank ownership beyond the traced generated default rows.
- No §6.17.2 `init_coeff_cdfs()` ownership beyond preserving the current fixed trace verifier.
- No AVM/dav2d invocation, wrapper, fixture regeneration, CI integration, or new dependency.
- No public API or CLI behavior change.

## Decisions

1. Put the frontier in `tile_payload`, not `runtime_minimal`.

   The symbols belong to §5.20.4.1 `decode_block()` syntax and §8.3 CDF selection, but this change only verifies the already-supported trace. The runtime should ask tile payload code to validate that the supported tile reaches and consumes the traced flat block symbols, then build the already-supported flat frame.

2. Keep the frontier minimal and typed.

   The new API should accept the live post-partition `SymbolDecoder`, the source tile work unit for offsets, and a small typed expected-trace contract. It should return a summary/checkpoint or typed error. Non-limit syntax mismatches should continue to map to `decode/unsupported-feature` in the minimal runtime; resource-limit behavior is unchanged.

3. Reuse generated CDF defaults.

   The frontier should keep using generated §9.3 default CDF arrays already used by the runtime. This avoids inventing CDF state while making ownership of the traced rows explicit in tile payload code.

4. Preserve output identity.

   The minimal fixture digest and Y4M bytes must not change. Any retiming, fixture rewrite, or local reference evidence update is outside this change.

## Risks / Trade-offs

- [Risk] The frontier could look like broader `decode_block()` support than it is. → Mitigation: name it as a trace frontier, document exclusions in code/docs/matrix, and keep tests tied to the single flat trace.
- [Risk] Moving symbol reads can accidentally alter CDF update behavior or `exit_symbol()` positioning. → Mitigation: assert the same symbol count, trailing bit position, padding end position, and digest/Y4M outputs.
- [Risk] Adding another crate-private boundary increases local API surface. → Mitigation: keep it internal to `tile_payload` and use it immediately from the minimal runtime.

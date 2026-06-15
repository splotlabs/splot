## Context

`splot-decode` already has a crate-private tile CDF subset for the partition-entry CDF arrays named by AV2 § 8.3.2 and context derivation for `do_split`, `do_square_split`, `rect_type`, `do_ext_partition`, and `do_uneven_4way_partition`. Tests currently prove that a selected row can be handed to `SymbolDecoder::read_symbol(cdf)`, but production code still exposes only generic closure-scoped row access.

This change adds the next boundary layer for the `S()` reads in AV2 § 5.20.3.2, while staying below full `read_partition()` behavior. It must preserve the existing crate dependency direction and remain private to `splot-decode`.

## Goals / Non-Goals

**Goals:**

- Provide a named crate-private API for one partition-entry `S()` symbol read over an existing `TileCdfSelector`.
- Preserve separate error sources for CDF selector/bounds errors and symbol-decoder errors.
- Keep `SymbolDecoder` state owned by the caller so sequential reads consume the same tile payload stream.
- Add tests covering every supported partition-entry selector family and failure mode.
- Update matrix/OpenSpec/docs to record the new boundary without overclaiming full tile syntax traversal.

**Non-Goals:**

- No implementation of `partition_implied`, `init_allowed_partitions`, `Rect_Part_Table`, final `Partition` values, or recursive `read_partition()`.
- No `uneven_4way_partition_type L(1)` read.
- No `decode_tile()`, `exit_symbol()`, CDF copyback/averaging completion, reconstruction, output, reference refresh, or public API.
- No external decoder invocation or new dependency.

## Decisions

1. Add a focused `crates/splot-decode/src/tile_payload/cdf/partition_read.rs` module.

   `tile_payload.rs` and `cdf/context.rs` are both close to the source-line budget, and `context.rs` should remain pure context derivation. A new module keeps the symbol-read boundary isolated and testable.

2. Implement the API on `TileCdfSubset`.

   The API shape is:

   ```rust
   pub(crate) fn read_partition_entry_symbol(
       &mut self,
       selector: TileCdfSelector,
       symbol: &mut SymbolDecoder<'_>,
   ) -> Result<Symbol, PartitionEntrySymbolReadError>
   ```

   This uses the existing `with_row_mut` selector validation and passes the row into the caller-owned `SymbolDecoder`. It does not reinitialize symbol state and does not infer CDF update mode.

3. Return raw `Symbol` values.

   AV2 § 5.20.3.2 gives the control-flow meaning of each read, but final partition decisions depend on allowed partition sets, implied partitions, rectangular type rules, and extra literal reads. Returning `Symbol` avoids inventing or partially encoding those semantics.

4. Preserve nested errors.

   The new error enum has one variant for `TileCdfError` and one for `splot_core::Error`. Selector failures must happen before symbol decoding advances, and symbol/CDF validation failures must leave the selected row unmodified by relying on `SymbolDecoder::read_symbol` validation.

## Risks / Trade-offs

- [Risk] The new helper could be misread as complete partition support. -> Mitigation: names, docs, matrix notes, and OpenSpec requirements explicitly say it is a single-symbol boundary and not `read_partition()`.
- [Risk] Callers could pass a `SymbolDecoder` configured with a different update mode than the tile work unit metadata. -> Mitigation: the helper accepts the caller-owned decoder and does not claim to enforce work-unit policy; current callers/tests configure the decoder from boundary metadata.
- [Risk] More code in near-limit files. -> Mitigation: use a new module and keep `cdf.rs` changes to `mod` plus re-export.

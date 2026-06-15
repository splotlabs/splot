## 1. Planning

- [x] 1.1 Create proposal, design, and spec deltas for `tile-partition-symbol-read-boundary`.
- [x] 1.2 Validate the OpenSpec change strictly before implementation.

## 2. Core Implementation

- [x] 2.1 Add a focused `tile_payload::cdf::partition_read` module with `PartitionEntrySymbolReadError`.
- [x] 2.2 Add a crate-private `TileCdfSubset::read_partition_entry_symbol` helper that uses existing selector validation and caller-owned `SymbolDecoder` state.
- [x] 2.3 Replace existing ad hoc test-only handoff with the production helper where appropriate.

## 3. Tests

- [x] 3.1 Cover successful reads for all five supported partition-entry selector families.
- [x] 3.2 Cover enabled and disabled CDF update modes, including non-selected row preservation.
- [x] 3.3 Cover selector failures before symbol consumption and symbol/CDF validation failure propagation without mutation.

## 4. Documentation And Status

- [x] 4.1 Add `DECODE-TILE-PARTITION-SYMBOL-READ-BOUNDARY` to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] 4.2 Add `tile-partition-symbol-read-boundary` to `docs/DECODER-SUPPORT-MATRIX.toml` and update related row notes.
- [x] 4.3 Update `docs/DECODER-ROADMAP.md` and generated decoder/feature status docs.

## 5. Verification

- [x] 5.1 Run targeted OpenSpec, unit, clippy, matrix/status, and decoder-support checks.
- [x] 5.2 Run `cargo xtask ci`.
- [x] 5.3 Run independent subagent reviews and address findings.
- [x] 5.4 Archive the OpenSpec change and rerun required gates.

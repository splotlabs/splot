## 1. Tracking

- [x] 1.1 Add `DECODE-TX-SIZE-SYMBOLIC-TABLES` to the implementation matrix.
- [x] 1.2 Add decoder support and conformance coverage rows for `tx-size-symbolic-tables`.

## 2. Implementation

- [x] 2.1 Extend `cargo xtask gen-tables` to resolve `TX_*` symbols for `Adjusted_Tx_Size`, `Tx_Size_Sqr`, and `Tx_Size_Sqr_Up`.
- [x] 2.2 Remove those three tables from the skip allowlist and regenerate generated conversion tables.
- [x] 2.3 Add focused tests for TxSize symbol resolution and generated table values.

## 3. Documentation And Verification

- [x] 3.1 Update the decoder roadmap and regenerate feature/status coverage docs.
- [x] 3.2 Validate OpenSpec, feature tracking, decoder support, decoder conformance coverage, generated-table drift, and the Rust acceptance gate.

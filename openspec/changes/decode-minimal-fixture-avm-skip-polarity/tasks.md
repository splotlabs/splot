# Tasks

- [x] Confirm the AVM ground truth (`decodetxb.c`: `all_zero == 1` is the skip
      branch) and that `avmdec` rejects the old hand-retimed fixture.
- [x] Generate an `avmenc` luma-skip 64x64 intra fixture (base_q_idx 210; broad
      tools, intra DIP, and tx-partition disabled) and verify `avmdec` == `dav2d`
      raw output byte-for-byte.
- [x] Verify splot's general intra path decodes the new fixture byte-for-byte
      identically to both oracles (raw md5 `f618317b…`, sha256 `92c4477c…`).
- [x] Replace `tests/conformance/vectors/valid/syn-flat-intra-64x64-minimal.ivf`
      and commit the oracle raw output as the sibling `.raw` reference.
- [x] Flip the frozen `block_symbol.rs::consume_trace` luma and V `txb_skip`
      assertions to the AVM `all_zero == 1` skip polarity, with citing comments.
- [x] Update the minimal-tier hash/raw/Y4M expectations to the general-path output
      across `runtime_hash.rs`, `runtime_raw.rs`, `runtime_y4m.rs`, and the
      `decode_cli` / `decode_raw_cli` / `decode_y4m_cli` integration tests.
- [x] Rework the frozen-frontier tests (legacy-rejection regression + MI-state
      limit tests against the embedded retired payload) and add the
      `general_intra_tests` luma-skip decode test.
- [x] Update `tests/conformance/manifest.toml`, `docs/LOCAL-REFERENCE-EVIDENCE.toml`
      (new oracle-agreement entry), and the affected `docs/IMPLEMENTATION-MATRIX.toml`
      and `docs/DECODER-SUPPORT-MATRIX.toml` rows.
- [x] Re-record the audit ledger and pass the full acceptance gate
      (`cargo xtask ci`, `check-reference-evidence`, `check-decoder-support`,
      `check-feature-status`).

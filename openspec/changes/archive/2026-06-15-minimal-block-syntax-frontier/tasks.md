## 1. Implementation

- [x] 1.1 Add `DECODE-MINIMAL-BLOCK-SYNTAX-FRONTIER` to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] 1.2 Add a crate-private tile-payload minimal block-symbol trace frontier that consumes the traced flat intra symbols after the partition frontier.
- [x] 1.3 Route `runtime_minimal.rs` through the new frontier without changing minimal hash or Y4M output identity.
- [x] 1.4 Preserve existing `decode/unsupported-feature`, `decode/resource-limit`, `decode/malformed-source`, and `decode/output-error` behavior.

## 2. Tests

- [x] 2.1 Add tile-payload unit tests for the successful traced block-symbol frontier and `exit_symbol()` summary.
  - Recorded tests:
    `crates/splot-decode/src/tile_payload/cdf/block_read.rs::tests::reads_supported_block_symbol_rows`.
    `crates/splot-decode/src/tile_payload/runtime_frontier.rs::tests::block_symbol_frontier_accepts_minimal_fixture_trace`.
- [x] 2.2 Add negative tests for traced symbol mismatch and block-symbol parse/exit failure.
  - Recorded tests:
    `crates/splot-decode/src/tile_payload/cdf/block_read.rs::tests::invalid_block_symbol_selector_fails_before_symbol_read`,
    `crates/splot-decode/src/tile_payload/block_symbol.rs::tests::traced_symbol_mismatch_fails_closed_and_rolls_back_cdfs`,
    `crates/splot-decode/src/tile_payload/block_symbol.rs::tests::invalid_cdf_row_reports_parse_failure_and_preserves_rows`,
    and
    `crates/splot-decode/src/tile_payload/runtime_frontier.rs::tests::block_symbol_frontier_rejects_exit_symbol_padding_failure`
    with post-partition CDF rollback coverage.
- [x] 2.3 Add CDF update-mode coverage proving selected traced rows mutate only when enabled.
  - Recorded test:
    `crates/splot-decode/src/tile_payload/cdf/block_read.rs::tests::update_mode_controls_only_selected_block_symbol_rows`.
- [x] 2.4 Keep runtime hash and Y4M tests green, including deterministic output across thread policies.
  - Keep and record these exact runtime tests with the feature proof:
    `crates/splot-decode/src/runtime_hash.rs::tests::minimal_fixture_decodes_to_hash_report`,
    `crates/splot-decode/src/runtime_hash.rs::tests::decoded_hash_is_deterministic_across_thread_policies`,
    `crates/splot-decode/src/runtime_y4m.rs::tests::minimal_fixture_decodes_to_exact_y4m_bytes`,
    and
    `crates/splot-decode/src/runtime_y4m.rs::tests::y4m_output_is_deterministic_across_thread_policies`.

## 3. Documentation

- [x] 3.1 Update `docs/DECODER-SUPPORT-MATRIX.toml` with the new boundary row and keep broad tile/CDF/reconstruction rows partial.
- [x] 3.2 Regenerate `docs/DECODER-SUPPORT-STATUS.md` and `docs/SPEC-COVERAGE.md` if the repo generators report drift.
- [x] 3.3 Update decoder roadmap notes if the supported frontier list changes.

## 4. Review And Gates

- [x] 4.1 Run subagent reviews for spec exactness, safety, and implementation shape.
- [x] 4.2 Run targeted tests: `cargo test -p splot-decode tile_payload --locked`, `cargo test -p splot-decode runtime_hash --locked`, and Y4M runtime tests.
- [x] 4.3 Run `cargo xtask feature-status`, `cargo xtask check-feature-status`, `openspec validate --all --no-interactive`, and `cargo xtask ci`.
- [x] 4.4 Archive the OpenSpec change before PR merge and rerun the required gates.

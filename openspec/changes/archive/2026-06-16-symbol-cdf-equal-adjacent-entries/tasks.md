## 1. Feature Tracking And OpenSpec

- [x] 1.1 Reuse Feature ID `AV2-8.2-SYMBOL-DECODER`; confirm no matrix status or proof-path change is required (new tests live under the listed `crates/splot-core/src/symbol.rs::tests` module).
- [x] 1.2 Author the `decoder-support` MODIFIED delta relaxing the CDF-validation scenario wording.
- [x] 1.3 Run `openspec validate symbol-cdf-equal-adjacent-entries --strict`.

## 2. Reachability

- [x] 2.1 Add `adaptation_can_equalize_adjacent_cumulative_entries`, adapting a real § 9.3 default row through `update_cdf` until adjacent cumulative entries are equal.
- [x] 2.2 Assert the equalized entries remain inside the valid `[1, 32767]` coding range.

## 3. Relaxation And Tests

- [x] 3.1 Relax `validate_cdf` to reject only a strict decrease (`value < cdf[index - 1]`); keep length, probability-range, adaptation-rate-index, and use-count checks.
- [x] 3.2 Rename `SymbolCdfErrorKind::NonIncreasingCumulative` to `DecreasingCumulative` and update its doc comment and `Display` message.
- [x] 3.3 Update the existing rejection test to a strictly-decreasing row (`[100, 99, 0, 0]`).
- [x] 3.4 Add `read_symbol_accepts_and_decodes_equal_adjacent_cumulative_entries` (deterministic decode over an explicit equal-adjacent row, all symbols including the equal-pair bucket).
- [x] 3.5 Add `adapted_row_with_equal_adjacent_entries_is_accepted_and_decodes` (the real adapted row is accepted and decodes a valid symbol).

## 4. Gates

- [x] 4.1 `cargo test -p splot-core symbol --locked`.
- [x] 4.2 `openspec validate --all --no-interactive` and `cargo xtask ci`.

## 5. Archive

- [x] 5.1 Archive the change with `openspec archive symbol-cdf-equal-adjacent-entries --yes` to apply the scenario wording to the main spec, then re-run `openspec validate --all` and `cargo xtask ci`.

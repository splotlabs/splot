## Why

`SymbolDecoder::validate_cdf` (Feature ID `AV2-8.2-SYMBOL-DECODER`) requires
**strictly** increasing cumulative entries (`value <= cdf[index - 1]` errors).
AV2 § 8.2.6 (`docs/spec/av2/1.0.0/08-parsing-process.md#s-8-2`) imposes no such
precondition, and its adaptation step can drive two adjacent cumulative entries
equal — a smaller entry gains more per increment, so adjacent entries converge.
From the shipped § 9.3 row `Default_Cctx_Type_Cdf`, decoding symbol 0 ~140 times
already yields `cdf[4] == cdf[5]`, and `read_symbol` still decodes correctly
because `Prob_Inc` separates the affected thresholds.

This is harmless today (minimal-tier rows are read at most once), but becomes a
conformance bug once `tile-payload-decode` broadens and a persistent CDF bank row
is read many times: an adapted equal-adjacent row would be wrongly rejected.

## What Changes

- Relax `validate_cdf` so equal adjacent cumulative entries are accepted; only a
  strict decrease (`value < cdf[index - 1]`) is rejected. The length,
  probability-range `[1, 32767]`, adaptation-rate-index `0..125`, and use-count
  `0..=32` checks are unchanged.
- Rename the typed CDF error variant `SymbolCdfErrorKind::NonIncreasingCumulative`
  to `DecreasingCumulative` and update its doc comment and `Display` message to
  match the relaxed semantics (it now fires only on a strict decrease).
- Add a reachability test that adapts a real § 9.3 default row until adjacent
  cumulative entries are equal, a decode-correctness test over an explicit
  equal-adjacent row, and a regression test that the adapted equal-adjacent row
  is accepted and decodes.
- Reconcile the `decoder-support` "CDF rows are validated before symbol decoding"
  scenario wording from "monotonic" to "non-decreasing (adjacent equal entries
  allowed)".

## Capabilities

### New Capabilities

### Modified Capabilities

- `decoder-support`: Refine the AV2 symbol decoder foundation's CDF-validation
  scenario so equal adjacent cumulative entries are accepted, matching AV2 § 8.2.6
  adaptation. No change to the `symbol-decoder` row's partial status.

## Impact

- Affected code: `crates/splot-core/src/symbol.rs` (`validate_cdf`, tests) and
  `crates/splot-core/src/error.rs` (`SymbolCdfErrorKind` variant rename + message).
- Affected docs/status: `openspec/specs/decoder-support/spec.md` scenario wording
  (applied at archive). The `docs/IMPLEMENTATION-MATRIX.toml` `AV2-8.2-SYMBOL-DECODER`
  row keeps its status and existing proof paths (new tests live under the already
  listed `crates/splot-core/src/symbol.rs::tests` module).
- APIs: public API change in `splot-core` — `splot_core::error::SymbolCdfErrorKind`
  variant `NonIncreasingCumulative` is renamed to `DecreasingCumulative`. No other
  crate references the old name.
- Diagnostics: no validator `rule_id` change; malformed CDF input remains a typed
  `splot_core::Error` value.
- Dependencies: no new third-party dependency, no AVM/dav2d integration, no
  scheduler change.
- Runtime behavior: no `splot decode` behavior change; the symbol decoder
  primitive simply accepts a strictly larger (spec-correct) set of valid CDF rows.

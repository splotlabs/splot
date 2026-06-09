# Design: diagnostic-registry-enforcement

## Extraction model

Validator diagnostics are constructed via `Diagnostic::{new,error,warning,info}(rule_id, …)`
plus a few `&'static str` helper fns (`tile_params_error`, `ordering_error`,
`syntax_error_diagnostic`). Every rule ID is a plain string literal — there are no
`format!`-built IDs. So the canonical set of emitted IDs is extracted **syntactically**:
every `"<ns>/<id>"` string literal in `crates/splot-validate/src`, where `ns` is
`[a-z][a-z0-9-]*`, `id` is `[a-z0-9-]+`, with exactly one `/` and no empty segments.

Two source regions must be excluded or the extractor over-collects:

- **`#[cfg(test)]` modules** — tests contain `has_error(report, "ns/id")` assertions (real
  IDs, also emitted in non-test code), `.starts_with("ns/prefix-")` literals (not full IDs),
  and fake examples like `Diagnostic::error("obu-header/x", …)`. Every test module in the
  crate is a single top-level `mod tests` running to EOF, so the extractor cuts from the
  first top-level `#[cfg(test)] … mod` line to EOF. The cut triggers only on `#[cfg(test)]`
  followed by `mod` (never `fn`); the invariant is asserted by a guard test.
- **Comments** — `//`, `///`, and `/* */` (e.g. a doc comment mentioning
  `"obu-header/global-xlayer-required"`). A small state-machine scanner
  (`string_literals_skipping_comments`) strips comments while honoring string escapes.

## The 13 registry-only IDs (Option A)

12 `<ns>/syntax` literals and `trailing-bits/empty-syntax-obu-payload` are `Check::id()`
*registry* identifiers (see `crates/splot-validate/src/checks/mod.rs`), not diagnostics —
those checks emit through `syntax_error_diagnostic()` with a different rule ID. They are
indistinguishable from emitted IDs without a semantic model of which `Check` calls `emit()`.

**Decision (Option A): the registry documents every rule-ID *literal* in non-test, non-comment
validator source, and the 13 registry-only IDs are documented in a clearly-labeled
sub-table.** This keeps the extractor a cheap, robust syntactic pass and tells the truth,
rather than carrying a fragile hardcoded exclusion list that a future 14th `<ns>/syntax`
literal would silently bypass (Option B, rejected).

## Doc contract

`docs/VALIDATOR-DIAGNOSTICS.md` carries a CI-enforced region between
`<!-- diagnostics-registry:begin -->` and `<!-- diagnostics-registry:end -->`. The check
parses backtick-wrapped `ns/id` tokens inside that region and requires the documented set to
equal the extracted set exactly (failing on emitted-but-undocumented and
documented-but-unemitted). Planning / not-yet-emitted material lives outside the markers and
is advisory.

## Scope limits

The check enforces the rule-ID *set* only. Severity and spec-section columns are documented
for humans but not machine-verified in v1 (the helper indirection makes per-ID severity hard
to extract syntactically). Machine-checking severity/section is a noted future enhancement.
The pre-existing prefix-level `scan_diagnostics` guard in `XTASK-FEATURE-STATUS` is retained
as a complementary coarser check.

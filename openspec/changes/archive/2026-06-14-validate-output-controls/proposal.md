# Change: validate-output-controls

## Feature IDs

- `CLI-VALIDATE-OUTPUT-CONTROLS`

## Why

On a large or noisy bitstream `splot validate` prints every diagnostic, which is
hard to scan and awkward to consume in CI. Two opt-in, presentation-only controls
fix that without changing what the validator computes or how conformance is
decided: cap the listed diagnostics, or show just the summary.

## Scope

- Spec sections: none (CLI presentation, not AV2 syntax).
- Crates/modules: `crates/splot-validate/src/render.rs` (new, library-first
  `RenderOptions` / `RenderedReport` + `render_text` / `rendered` on
  `ValidationReport`); `crates/splot-cli/src/commands/validate.rs` (additive flags,
  wired through to the render API).
- CLI/docs/tests: `--max-diagnostics N`, `--summary-only`; README quick-start;
  render unit tests + `validate_snapshots.rs` (text + JSON) + `cli.rs` behavioral
  tests; the `validate --help` golden updates intentionally.

## Non-goals

- Does not change which diagnostics are produced, the summary counts, or the exit
  code (still `Validator::is_acceptable` over the full report).
- Does not reuse or alter the global `--quiet` flag (which controls logging only).
- The truncation notice is render metadata, never a `Diagnostic`/rule-id, so it
  does not touch the CI-enforced diagnostic registry.
- No new dependency (`RenderedReport` is `Serialize`; the CLI serializes it).

## Acceptance criteria

- [ ] Matrix row `CLI-VALIDATE-OUTPUT-CONTROLS` exists.
- [ ] `--max-diagnostics N` caps text and JSON identically, with a truncation
      notice / `truncation` object computed from the full report.
- [ ] `--summary-only` prints only the summary (text) / a `summary` object with an
      empty `diagnostics` array (JSON), and takes precedence over the cap.
- [ ] Exit codes are unchanged by either flag; default output is byte-compatible
      with the previous text and JSON.
- [ ] Snapshot (text + JSON) + behavioral tests + the updated `--help` golden ship.
- [ ] `cargo xtask check-feature-status` and `cargo xtask ci` pass.

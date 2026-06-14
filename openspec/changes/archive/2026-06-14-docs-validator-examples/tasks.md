# Tasks

## Matrix and docs

- [ ] Add the `DOC-VALIDATOR-EXAMPLES` row to `docs/IMPLEMENTATION-MATRIX.toml`.
- [ ] Regenerate `docs/FEATURE-STATUS.md` with `cargo xtask feature-status --format markdown --output docs/FEATURE-STATUS.md`.

## Implementation (docs)

- [ ] Add a `splot explain` worked example to the README (describe text), with the
      unknown-id "did you mean" hint and a note on `--json` / `--list`.
- [ ] Add a worked output-controls example (`--max-diagnostics`, `--summary-only`)
      showing the truncation notice and the unchanged counts / exit code.
- [ ] Add a Status-table row for the now-shipping `splot explain` command.
- [ ] Capture every example's output from the built binary (no hand-edited output).

## Checks

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- [ ] `cargo test --workspace --all-targets --locked`
- [ ] `cargo xtask check-feature-status`
- [ ] `openspec validate docs-validator-examples --strict`
- [ ] `cargo xtask ci` green; conflict-zone denylist untouched.

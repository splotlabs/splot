## Summary

## Feature IDs

-

## Checklist

- [ ] I ran `git status --short` before editing and preserved user work.
- [ ] Feature IDs are present in code/docs/tests where relevant.
- [ ] `docs/IMPLEMENTATION-MATRIX.toml` is updated.
- [ ] `docs/FEATURE-STATUS.md` is regenerated if matrix status changed.
- [ ] OpenSpec change exists for non-trivial design/behavior changes.
- [ ] No AV1 OBU assumptions were introduced.
- [ ] No fabricated AV2 syntax/semantics were introduced.
- [ ] Diagnostics have stable rule IDs, spec sections, offsets, and messages where applicable.
- [ ] Tests/proof were added and recorded in the matrix.
- [ ] `cargo fmt --all -- --check` passes.
- [ ] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` passes.
- [ ] `cargo test --workspace --all-targets --locked` passes.
- [ ] `cargo xtask check-feature-status` passes.
- [ ] `cargo xtask ci` passes.

## Deviations or follow-ups

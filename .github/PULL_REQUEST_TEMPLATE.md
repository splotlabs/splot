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
- [ ] PR title and commit subjects use Conventional Commits.
- [ ] `cargo fmt --all -- --check` passes.
- [ ] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` passes.
- [ ] `cargo test --workspace --all-targets --locked` passes.
- [ ] `cargo xtask check-feature-status` passes.
- [ ] `cargo xtask ci` passes.

## Encoder research gate, if applicable

- Feature / module:
- AV2 spec sections read:
- AVM files/tests/streams used as oracle:
- Decoder-visible behavior? yes/no
- Matrix row added/updated in `docs/IMPLEMENTATION-MATRIX.toml`? yes/no
- Reference docs read:
  - [ ] `docs/references/ENCODER-RESEARCH-NOTES.md`
  - [ ] `docs/references/THIRD-PARTY-NOTICES.md`
  - [ ] `docs/references/RAV1E-SOURCE-MAP.md`
  - [ ] `docs/references/SVT-AV1-RESEARCH-MAPPING.md`
- rav1e/SVT concepts used for inspiration:
- Third-party material copied: none
- AV1 syntax/tables/constants excluded: yes
- Tests/traces added:
- How this design aims to be better than the reference:

## Deviations or follow-ups

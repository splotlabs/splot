# Tasks

## Matrix and docs

- [ ] Add or update `docs/IMPLEMENTATION-MATRIX.toml`.
- [ ] Regenerate `docs/FEATURE-STATUS.md` with `cargo xtask feature-status --format markdown --output docs/FEATURE-STATUS.md`.
- [ ] Update `docs/SPEC-MAPPING.md` if a new AV2 section is modeled.
- [ ] Update `STATUS.md`.

## Implementation

- [ ] Add or update strong Rust types.
- [ ] Add or update parser/writer/validator/encoder code only as scoped.
- [ ] Add stable diagnostic IDs when applicable.
- [ ] Avoid AV1 assumptions and fabricated syntax.

## Tests and proof

- [ ] Add positive tests.
- [ ] Add malformed/EOF/error tests.
- [ ] Add fuzz/property/snapshot/conformance tests where applicable.
- [ ] Add proof commands to the matrix row.

## Checks

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- [ ] `cargo test --workspace --all-targets --locked`
- [ ] `cargo xtask check-feature-status`
- [ ] `cargo xtask ci`

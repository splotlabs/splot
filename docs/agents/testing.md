# Agent Testing Notes

Use this file when deciding what evidence a change needs. The complete testing
strategy is [../TESTING.md](../TESTING.md).

## Priority Order

1. Parser unit tests: LEB128, OBU header, Annex B, IVF, and module-specific
   syntax.
2. Property and fuzz tests proving parsers and validators do not panic.
3. `inspect` snapshots.
4. Conformance vectors.
5. Differential testing against AVM.

Parser changes need positive, negative, and EOF cases.

## Proof in the Matrix

When a feature stage becomes `done`, record proof in
`docs/IMPLEMENTATION-MATRIX.toml`:

- test module or path
- reproducible command
- fixture or vector
- diagnostic id when relevant

Enforcement:

```bash
cargo xtask check-feature-status
```

## Common Test Commands

```bash
cargo test --workspace --all-targets --locked
cargo test --doc --workspace --locked
cargo xtask conformance
cargo xtask fuzz [--time <secs>]
cargo xtask ci
```

Fuzzing requires nightly and `cargo-fuzz`. See [commands.md](./commands.md) for
the command list.

## Fixtures and Conformance

Fixture metadata and hashes are checked by:

```bash
cargo xtask check-fixtures
```

Decoder support and decoder conformance status are checked by:

```bash
cargo xtask check-decoder-support
cargo xtask check-decoder-conformance-coverage
cargo xtask check-reference-evidence
```

See [../FIXTURES.md](../FIXTURES.md), [../CONFORMANCE.md](../CONFORMANCE.md), and
[../DECODER-FULL-CONFORMANCE.md](../DECODER-FULL-CONFORMANCE.md).

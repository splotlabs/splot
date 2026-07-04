# Testing

## Layers

1. Parser unit tests: positive, negative, and EOF cases for syntax parsers.
2. Property tests and fuzz targets: malformed input returns errors/reports and
   never panics.
3. CLI integration tests: exit codes, JSON/text rendering, `inspect`, `validate`,
   `explain`, and narrow `decode` output behavior.
4. Conformance corpus: committed validator vectors under `tests/conformance/`,
   with no AVM dependency in CI.
5. Decoder-output oracle: `tests/conformance/decoder-oracle.toml` stores AVM raw
   output hashes, but CI runs only `splot` against the committed hashes.
6. Local differential testing: AVM is the oracle, but live AVM runs are local and
   opt-in.

## Commands

```bash
cargo test --workspace --all-targets --locked
cargo test --doc --workspace --locked
cargo xtask conformance
cargo xtask fuzz --time 30
cargo xtask ci
```

`cargo xtask fuzz` requires nightly and `cargo-fuzz`; it is run-if-present
locally. CI runs registered fuzz-target smoke jobs separately.

## Proof

When a feature stage becomes `done`, record proof in
`docs/IMPLEMENTATION-MATRIX.toml`:

- test modules or integration tests;
- reproducible commands;
- fixtures or vector paths;
- diagnostic rule ids, when relevant.

`cargo xtask check-feature-status` rejects code stages marked `done` without
proof.

## Parser Rule

Parser changes need at least one runnable check that would fail if the parser
accepts malformed input, rejects valid input, or panics/truncates incorrectly.
Tiny mechanical one-line changes can rely on existing coverage only when the
existing test would fail on the changed behavior.

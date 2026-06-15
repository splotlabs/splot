## Why

The minimal decoder hash tier now proves one byte-consuming runtime success path, but the compatibility form `splot decode <input> -o <output>` still emits `decode/unsupported-feature` and leaves Y4M output unwired. This change makes the next Phase 2 step measurable by publishing Y4M bytes for the same narrow, already-validated minimal tier while preserving fail-closed diagnostics outside that tier.

## What Changes

- Add Feature ID `DECODE-Y4M-RUNTIME-OUTPUT` for the first byte-consuming runtime Y4M file-output path.
- Extend `splot-decode` with a bounded minimal-tier Y4M artifact path that reuses the existing `minimal-intra-8bit420-hash-v1` validation/reconstruction surface and the `splot-recon` Y4M writer.
- Update `splot-cli decode` so implicit `-o <path>` and explicit `--output-format y4m -o <path>` write Y4M output for the committed minimal IVF fixture with exit code 0.
- Publish Y4M output atomically from the CLI: write a same-directory temporary file, flush/sync as appropriate, and rename only after complete success.
- Preserve current no-touch behavior for hash mode and for malformed, resource-limit, and out-of-tier Y4M failures.
- Update decoder support/status/coverage docs and tests for the new runtime output row.
- Non-goals: broad AV2 reconstruction, raw output, film grain, multi-frame output scheduling, reference refresh, AVM/dav2d integration, new dependencies, and any change to validator behavior.

## Capabilities

### New Capabilities
- None.

### Modified Capabilities
- `decoder-support`: Adds a minimal-tier runtime Y4M output requirement and status row, distinct from the existing source-backed `RECON-Y4M-OUTPUT-WRITER` library primitive.

## Impact

- `crates/splot-decode`: expose a crate-owned minimal Y4M byte artifact API without adding filesystem I/O.
- `crates/splot-cli`: publish Y4M files atomically and map decode/output failures to existing diagnostic or operational exit paths.
- `crates/splot-recon`: no new writer capability is expected; the existing Y4M writer is used through `splot-decode`.
- `crates/splot-cli/tests/decode_cli.rs` and `crates/splot-decode` tests: add success, atomicity/no-touch failure, resource-limit, and deterministic thread-policy coverage.
- `docs/DECODER-SUPPORT-MATRIX.toml`, generated status/coverage docs, and OpenSpec `decoder-support` deltas: record the new runtime support and remaining exclusions.

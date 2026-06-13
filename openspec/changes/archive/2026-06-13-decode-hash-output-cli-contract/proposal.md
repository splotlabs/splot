## Why

The decoder roadmap requires deterministic frame hashes before Y4M output can
be treated as a success artifact, but the current `splot decode` CLI only has a
Y4M-shaped `-o/--output` sink. A small CLI contract is needed before runtime
decode work can expose hash-first success without forcing Y4M output.

## What Changes

- Add Feature ID `CLI-DECODE-HASH-OUTPUT` for the `splot decode` hash-output
  CLI contract.
- Add a CLI parse surface for selecting the future decode output artifact:
  `--output-format y4m` or `--output-format hash`.
- Preserve current compatibility: `splot decode <input> -o <output>` remains
  valid and still means Y4M-shaped output.
- Permit `splot decode <input> --output-format hash` as the future hash-output
  mode without requiring a Y4M output path.
- Keep current runtime behavior unchanged for every valid parse: all decode
  invocations still emit `decode/unsupported-feature`, exit `1`, do not read
  input bytes, do not touch output paths, and do not invoke external decoders.
- Update decoder roadmap, support matrix, implementation matrix, generated
  status docs, tests, and OpenSpec.

## Capabilities

### New Capabilities

### Modified Capabilities

- `decoder-support`: add the `splot decode` hash-output CLI contract while
  preserving the current intentional unsupported runtime behavior.

## Impact

- Affected code: `crates/splot-cli/src/commands/decode.rs` and CLI tests.
- Affected docs/status: decoder roadmap, decoder support matrix/status,
  implementation matrix/status, spec coverage, and OpenSpec.
- No Cargo manifest, dependency graph, crate scaffolding, runtime decode,
  hash computation, Y4M output, diagnostic registry, fixture, AVM/dav2d,
  script, `xtask`, or CI change.

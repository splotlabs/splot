## Why

The full decoder mission needs the first runtime success path before deeper
tile, CDF, and reconstruction work can be measured. Today `splot decode`
successfully plans minimal closed-loop-key input but always returns
`decode/unsupported-feature`, so hash-output CLI behavior is still a contract
rather than a supported runtime artifact.

## What Changes

- Add a minimal-tier runtime decode success path for the existing closed-loop
  key-frame planning tier that can produce `splot decode --output-format hash
  --json` success output.
- Keep the success path tightly scoped to the currently supported minimal tier;
  unsupported AV2 structures continue to fail with structured diagnostics until
  their feature rows are implemented.
- Ensure hash-only mode never creates, truncates, rewrites, or deletes an output
  path, including when `-o` is supplied.
- Make the hash report deterministic across `--threads 1`, `--threads auto`,
  and selected fixed `--threads N` policies.
- Preserve deterministic structured diagnostics for malformed, unsupported, and
  resource-limited inputs.
- Record portable local AVM/dav2d evidence metadata for the tiny minimal-tier
  fixture without adding external decoder source, wrappers, commands, CI, or
  runtime invocation.
- Update `docs/DECODER-SUPPORT-MATRIX.toml`, generated status docs, decoder
  roadmap/full-conformance docs as needed, and Feature ID proof without
  overclaiming broader AV2 runtime decode support.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `decoder-support`: adds the first supported minimal-tier runtime hash success
  requirement for `splot decode`, while preserving unsupported diagnostics for
  all out-of-tier normative features.

## Impact

- Feature ID: `DECODE-MINIMAL-TIER-RUNTIME-SUCCESS`.
- Affected crates: `splot-decode`, `splot-cli`, and possibly `splot-recon` only
  through existing decoded-frame/hash APIs.
- Affected tests: CLI runtime decode tests, decode library tests, fixture
  manifest/reference-evidence checks, and existing decode fuzz smoke coverage.
- Affected docs/status: decoder support matrix/status, feature/spec coverage,
  decoder roadmap/full-conformance status, and local reference evidence
  metadata.
- Dependency impact: no new third-party dependencies and no AVM/dav2d
  integration, subprocess invocation, CI job, wrapper, or required local setup.
- Non-goals: full tile payload decoding, full CDF lifecycle, inter/intra
  reconstruction beyond the minimal supported fixture, Y4M/raw output,
  film-grain synthesis, decoded-frame-hash metadata verification, reference
  refresh completeness, and broad AV2 conformance claims.

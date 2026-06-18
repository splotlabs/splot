## Why

The encoder now has private residual and forward-transform foundations, but it
still has no deterministic quantization policy to turn transform coefficients
into quantized coefficients and decoder-visible dequantized values. The next
minimal closed-loop step is a small, private fixed-quantizer path that proves
quantization and reconstruction math before coefficient tokenization or packet
output exists.

## What Changes

- Add `ENC-QUANTIZATION-V0` as a private `splot-encode` encoder-tool feature.
- Add a deterministic fixed-quantizer policy for the current 4x4 DCT_DCT
  DC-only coefficient subset.
- Validate shape, coefficient count, quantizer inputs, and checked arithmetic
  before returning quantized or dequantized data.
- Prove quantized coefficients dequantize through `splot-recon` and feed the
  existing inverse transform path deterministically.
- Keep quantization private and non-emitting: no token writer, range coding,
  tile body, packet output, CLI success path, rate control, or Baseline Encoder
  Profile v1 claim.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `encoder-tools`: add the encoder fixed-quantizer v0 requirement and scenarios
  for quantization, dequant/inverse proof, input rejection, and no packet output.

## Impact

- Affected code: `crates/splot-encode/src/quantization.rs`,
  `crates/splot-encode/src/error.rs`, `crates/splot-encode/src/lib.rs`, and a
  no-packet regression in `crates/splot-encode/src/context.rs`.
- Affected docs/status: `docs/IMPLEMENTATION-MATRIX.toml`, generated status and
  coverage docs, `docs/ENCODER-ROADMAP.md`, `docs/ENCODER-GAP-AUDIT.md`, and
  `openspec/specs/encoder-tools/spec.md`.
- No public API, dependency graph, CLI behavior, validator behavior, concurrency
  policy, or zero-copy policy changes.

## Context

The previous decoder slice added empty `splot-recon` and `splot-decode`
workspace crates. The CLI still owns the `decode/unsupported-feature`
descriptor even though the diagnostic is decoder-domain behavior. Moving that
descriptor into `splot-decode` is the smallest library-backed decoder step that
does not decode bytes.

## Goals / Non-Goals

**Goals:**

- Make `splot-decode` own the stable unsupported diagnostic descriptor.
- Keep `splot-cli` responsible for clap arguments, output-path validation, JSON
  serialization, text rendering, and exit code mapping.
- Preserve the current `decode/unsupported-feature` field values exactly:
  rule id, severity spelling, §7.1 section, matrix row, Feature ID, message, and
  remediation.
- Add the `splot-cli -> splot-decode` dependency edge.

**Non-Goals:**

- No byte-consuming decode API, decode options, frame/plane model, resource
  limits enforcement, reconstruction, hash computation, Y4M output, or reference
  state.
- No `splot-decode -> splot-core` or `splot-decode -> splot-recon` dependency.
- No AVM/dav2d evidence, source reads, command runs, wrappers, scripts, CI jobs,
  fixtures, or local paths.

## Decisions

1. Keep `splot-decode` dependency-free.

   The diagnostic descriptor can use `&'static str` fields and a small severity
   enum. JSON serialization stays in `splot-cli`, which already depends on
   `serde` and `serde_json`. This avoids adding an external dependency to the
   decoder crate before runtime decode exists.

2. Expose a narrow descriptor API.

   `splot-decode` will expose a documented `DecodeDiagnostic`,
   `DecodeSeverity`, `UNSUPPORTED_FEATURE_DIAGNOSTIC`, and
   `unsupported_feature_diagnostic()`. It will not expose a runner, options
   type, path type, output format, or error enum.

3. Preserve CLI behavior through existing integration tests.

   The CLI should copy the descriptor into a private serializable view for JSON
   output so the JSON field names and severity string stay unchanged. Text output
   should use the same descriptor fields in the same order.

4. Treat §7.1 as diagnostic context, not conformance proof.

   The diagnostic remains tied to the general decoding process because the entry
   point is unsupported. This change does not implement §7 decoding semantics.

## Risks / Trade-offs

- Public API stability -> expose only the stable diagnostic descriptor needed by
  the CLI.
- Diagnostic registry drift -> the literal moves to `splot-decode`, which is
  already a decoder registry scan root.
- Dependency-direction drift -> update `xtask`, docs, and OpenSpec so the
  `splot-cli -> splot-decode` edge is intentional.
- Runtime overclaim -> matrix/docs must keep `cli-decode-entrypoint` as
  `unsupported-intentional`.

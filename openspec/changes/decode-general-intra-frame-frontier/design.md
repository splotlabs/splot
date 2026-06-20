## Context

`decode_minimal_frame_from_plan_with_ivf_preflight` parses the sequence and
frame headers, derives one tile work unit, and then validates the frame against
a frozen gate (`validate_frame_core`) that requires `base_q_idx == 255` and the
exact committed minimal fixture before running a fixed six-symbol trace and a
hardcoded DC-prediction reconstruction. AVM `avmenc` can emit a tiny
minimal-tool intra `OBU_CLOSED_LOOP_KEY` stream that `avmdec` and `dav2d` decode
identically; splot already parses such a stream up to `validate_frame_core` and
the real AV2 § 5.20.3.1 partition traversal already handles its single 64x64
block. The gap is that the frozen gate rejects any non-fixture frame and the
trace/recon path is fixture-specific.

## Goals / Non-Goals

**Goals:**
- Route a minimal-tool intra key frame off the frozen hash tier into a general
  intra decode path tracked by `DECODE-GENERAL-INTRA-FRAME-FRONTIER`.
- Run the real partition traversal on a real AVM stream and confirm the
  single-block root frontier.
- Keep the frozen `base_q_idx == 255` minimal hash contract byte-identical.
- Commit the bit-exact oracle fixture and record the avmdec/dav2d agreement.

**Non-Goals:**
- No arbitrary intra mode decode, coefficient symbol reads, `Quant` writes,
  dequantization, inverse transform, residual add, reconstruction, or output.
- No split partitions, multiple tiles, non-64x64 frames, or non-8-bit/4:2:0.
- No in-repo AVM/dav2d dependency; AVM is the local generator and oracle only.

## Decisions

1. Discriminate the frozen tier by `base_q_idx == 255`.

   Rationale: the committed frozen fixture uses `base_q_idx == 255`; routing only
   non-255 minimal-tool intra frames to the general path keeps the frozen hash
   contract on its existing trace path with zero behavior change, and any other
   `base_q_idx == 255` frame still falls through to the frozen gate's precise
   diagnostics exactly as before.

   Alternative considered: fingerprint the exact fixture bytes. Rejected as
   brittle and unnecessary; the quantizer split is deterministic and documented.

2. Keep `validate_frame_core` unchanged; add a separate `is_general_minimal_intra`
   predicate.

   Rationale: the frozen gate's per-condition diagnostics stay intact for
   genuinely unsupported frames, while the general predicate is a single boolean
   that mirrors the same tool constraints minus the `base_q_idx` pin.

3. Reuse `derive_tile_plan` and `plan_minimal_runtime_partition_frontier`.

   Rationale: tile derivation and partition traversal are already general (built
   from parsed `core` facts); the general path adds only the acceptance predicate
   and the unsupported-at-block-decode boundary.

## Risks / Trade-offs

- [Risk] The general path can be mistaken for full intra decode support.
  -> Mitigation: it returns a structured `decode/unsupported-feature` diagnostic
  with a dedicated reason and remediation; matrix, support row, and docs state
  that block-symbol, coefficient, and reconstruction decode are unimplemented.
- [Risk] A future general frame could split the superblock.
  -> Mitigation: the partition frontier rejects non-single-block shapes with a
  dedicated `general_intra_partition_frontier` reason rather than panicking.
- [Risk] Committing an AVM-encoded fixture.
  -> Mitigation: it is bytes generated from project-owned synthetic input under
  the established `avm-generated-from-project-owned-synthetic-input` provenance;
  AVM is not vendored, depended on, or run by CI.

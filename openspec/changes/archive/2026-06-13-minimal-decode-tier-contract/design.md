## Context

`splot decode` currently reports the supported structured
`decode/unsupported-feature` diagnostic for every input. The decoder roadmap has
foundation contracts for limits, decoded frame/plane layout, and deterministic
frame hashes, but it does not yet make the first intended success tier precise
enough for future runtime implementation and rejection diagnostics.

The mission asks for encoder-grade decoder support, not a production AV2
decoder. Crate and dependency-graph changes still need explicit maintainer
approval, so this change intentionally stays in docs, matrices, and OpenSpec.

## Goals / Non-Goals

**Goals:**

- Define a conservative first decode tier contract with stable identifiers:
  `contract_id = "splot.decode.minimal_tier"`,
  `contract_version = 1`, and
  `tier_id = "minimal-intra-8bit420-hash-v1"`.
- Ground the tier in committed AV2 v1.0.0 spec sections.
- State that this is an implementation-supported subset, not an Annex A
  level-conformant decoder claim.
- Record future rejection behavior for unsupported streams with existing planned
  decoder diagnostics.
- Leave all runtime rows and emitted diagnostic registries unchanged.

**Non-Goals:**

- No new `splot-recon` or `splot-decode` crate.
- No `Cargo.toml`, `Cargo.lock`, public Rust API, CLI behavior, parser,
  decoder, reconstruction, hash, Y4M, fixture, fuzz, `xtask`, or CI change.
- No AVM/dav2d source, snippets, binaries, wrappers, scripts, build probes,
  required tests, or CI integration.
- No claim that `splot` currently decodes pixels, computes frame hashes, writes
  Y4M, selects operating points, or rejects tier violations at runtime.

## Decisions

1. Treat the first tier as a `splot` subset, not an AV2 conformance level.

   Annex A.5 level-conformant decoder obligations are broader than the intended
   encoder-MVP subset. The roadmap will describe the tier as a repository-owned
   supported subset so it cannot be confused with an Annex A decoder profile or
   level.

2. Require Annex B length-delimited input, optionally through IVF/DKIF framing.

   The core stream parser already handles raw Annex B and IVF-wrapped streams.
   The tier contract should not claim bare OBU streams, Y4M input, arbitrary
   containers, or external decoder wrappers. IVF support remains a container
   envelope for Annex B payloads, not a separate AV2 syntax claim.

3. Make the sequence and layer boundary deliberately narrow.

   The first tier requires one selected stream/layer: `obu_xlayer_id == 0` for
   non-global OBUs, no temporal or embedded enhancement layers, `max_tlayer_id`
   and `max_mlayer_id` equal to zero, and no external HLS, MSDO, LCR, Atlas, or
   OPS selection path. This keeps the future planner transactional and avoids
   silent sub-bitstream extraction.

4. Require the first Main 4:2:0 interoperability profile, 8-bit 4:2:0,
   and closed-loop key-frame-only streams.

   AV2 signals profile identity with `seq_profile_idc`; the first tier accepts
   only `seq_profile_idc == 0` (`Main_420_10_IP0`) and further narrows that
   profile to `bit_depth_idc == 1` for 8-bit and `chroma_format_idc == 0` for
   4:2:0. The tier accepts only `obu_type == OBU_CLOSED_LOOP_KEY` closed-loop
   key-frame output, not open-loop key frames, show-existing frames,
   inter/switch/TIP/bridge/RAS paths, film grain, multi-frame headers, or
   external-HLS-dependent behavior.

5. Prefer a positive allowlist over a denylist.

   Future source should admit streams only when parsed facts prove every tier
   precondition. A missing implementation row or unsupported AV2 tool should
   route to `decode/unsupported-feature`; limit overflow or configured limit
   excess should route to `decode/resource-limit`.

6. Make deterministic hashes the first success artifact.

   `splot-dfh-sha256-v1` over cropped visible output samples is the first stable
   proof artifact. The current CLI shape still requires `-o`, so future
   implementation may need an explicit hash-output mode before CLI decode can
   become supported. Y4M remains a later row and must not become the only proof
   source.

7. Keep local reference evidence absent for this contract.

   Existing archived AVM/dav2d evidence is scoped to deterministic hash planning
   and does not prove tier selection or `splot` runtime behavior. This change
   records no local reference evidence.

## Risks / Trade-offs

- Overclaiming runtime support -> keep matrix status `partial`, retain current
  `CLI-DECODE` unsupported behavior, and leave runtime rows as `todo`.
- Tier too broad for the first implementation -> use closed-loop key frames,
  8-bit 4:2:0, no crop, no film grain, one tile, and a positive allowlist.
- CLI/output mismatch -> document hash-first output as a future requirement and
  do not change CLI behavior in this docs-only change.
- Spec ambiguity -> cite committed spec sections and keep unsupported areas
  outside the tier until implementation evidence exists.
- External reference boundary drift -> keep AVM/dav2d evidence absent and avoid
  committed local paths, wrappers, scripts, dependencies, or required tests.

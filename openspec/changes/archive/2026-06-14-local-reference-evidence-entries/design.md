## Context

`docs/LOCAL-REFERENCE-EVIDENCE.toml` is already the canonical portable evidence
manifest and is checked by `cargo xtask check-reference-evidence`. The manifest
currently contains only a schema comment and no entries, while the decoder
roadmap and support matrix cite archived free-form AVM/dav2d raw MD5 agreement
for two committed fixtures:

- `tests/conformance/vectors/valid/syn-key-intra-64x64.ivf`
- `tests/conformance/vectors/valid/syn-intra-64x64-10bit.ivf`

This change converts that already-recorded evidence into checked manifest
metadata. It does not generate new vectors and does not rerun AVM/dav2d.

## Goals / Non-Goals

**Goals:**

- Add portable evidence entries with repo-relative fixture paths, fixture
  SHA-256/length, reference tool revisions, sanitized command summaries, raw
  MD5 output digests, and equality assertions.
- Keep the entries tied to existing Feature IDs and decoder support rows without
  claiming current `splot decode` hash/runtime support.
- Update docs and matrix text so generated status reflects that the manifest now
  has non-empty evidence metadata.
- Prove the entries with existing offline gates only.

**Non-Goals:**

- No AVM/dav2d execution, source import, wrapper, script, build probe, CI job,
  dependency, or mandatory local setup.
- No decoded-frame SHA-256 digest implementation.
- No new conformance vectors, no fixture regeneration, no runtime decode, no
  reconstruction, and no Y4M output.
- No crate or dependency graph change.

## Decisions

1. Use the existing `XTASK-LOCAL-REFERENCE-EVIDENCE-MANIFEST` Feature ID for the
   manifest mechanics, while each evidence entry points at
   `RECON-HASH-INPUT-SERIALIZATION` and decoder-support row
   `deterministic-frame-hash`.

   Rationale: the checker and schema are already supported; the new entries are
   evidence for future hash comparison policy, not new checker behavior.

2. Record the existing AVM/dav2d raw MD5 agreement as reference-output metadata,
   not as `splot-dfh-sha256-v1` output.

   Rationale: current `splot-recon` source-backs hash input byte serialization
   only. Treating reference raw MD5 as the repository-owned decoded-frame hash
   would overclaim runtime digest computation.

3. Keep command summaries descriptive and path-free.

   Rationale: the checker intentionally rejects executable paths, shell
   composition, and local paths. Manifest entries should survive across
   machines and CI environments.

4. Do not change PR #101 concurrency surfaces.

   Rationale: this change has no runtime work. It must not add worker pools,
   queues, Rayon/crossbeam usage, or any decode/recon scheduling behavior.

## Risks / Trade-offs

- [Risk] Archived evidence is free-form, so command details are less precise
  than a freshly recorded manifest-first run. → Mitigate by limiting entries to
  the exact tool revisions and raw MD5 digests already recorded, and by using
  sanitized command summaries rather than reconstructing local paths.
- [Risk] Readers may interpret raw MD5 agreement as proof of `splot` hash
  implementation. → Mitigate with `output_scope`, matrix notes, and roadmap text
  that explicitly say this is local reference raw output metadata only.
- [Risk] Fixture bytes drift after entries are added. → Mitigated by the
  existing checker verifying committed byte length and SHA-256.

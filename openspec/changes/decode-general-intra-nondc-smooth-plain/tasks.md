## 1. Tracking

- [x] 1.1 Add `DECODE-GENERAL-INTRA-NONDC-LUMA-SMOOTH-PLAIN` to the implementation matrix.
- [x] 1.2 Add the `general-intra-nondc-luma-smooth-plain` decoder support row.
- [x] 1.3 Add the `syn-smooth-intra-64x64-q124.ivf` and negative `syn-smoothnondc-intra-64x64-q132.ivf` fixtures, conformance manifest entries, and reciprocal LOCAL-REFERENCE-EVIDENCE entries.

## 2. Implementation

- [x] 2.1 Add `SupportedNonDcLumaMode::Smooth` and map `SMOOTH_PRED` (canonical §9.2 mode 9) to it in `IntraYMode::supported_nondc`.
- [x] 2.2 Map `SupportedNonDcLumaMode::Smooth` to `splot-recon` `IntraSmoothMode::Smooth` in both the no-neighbour and neighbour smooth reconstruction entry points.
- [x] 2.3 Admit plain SMOOTH only at the top-left (no-neighbour) 64x64 superblock with DC chroma; reject neighbour-having and sub-64x64 plain SMOOTH before reconstruction.

## 3. Documentation And Verification

- [x] 3.1 Regenerate feature/status/support docs.
- [x] 3.2 Validate OpenSpec, feature tracking, decoder support, reference evidence, and the Rust acceptance gate.

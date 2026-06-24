# Encoder Reference Gate

Use this file before changing `crates/splot-encode`, encoder-facing
`splot-core` syntax/parsing code, or encoder research documentation.

## Required Reading

Read:

1. [../references/ENCODER-RESEARCH-NOTES.md](../references/ENCODER-RESEARCH-NOTES.md)
2. [../references/THIRD-PARTY-NOTICES.md](../references/THIRD-PARTY-NOTICES.md)
3. [../references/RAV1E-SOURCE-MAP.md](../references/RAV1E-SOURCE-MAP.md) when
   using Rust API, RDO, tiling, fuzzing, profiling, or safe data-structure ideas
   from rav1e.
4. [../references/SVT-AV1-RESEARCH-MAPPING.md](../references/SVT-AV1-RESEARCH-MAPPING.md)
   when using production pipeline, mode-decision, motion-estimation,
   rate-control, filter-search, threading, or SIMD ideas from SVT-AV1.

## Non-Negotiable Boundary

rav1e and SVT-AV1 are engineering inspiration only. Do not copy AV1 syntax,
source code, tables, constants, entropy CDFs, comments, or prose.

AV2 behavior must be derived from:

1. The AV2 specification mirror.
2. AVM as the oracle.
3. Original `splot` code and documentation.

## Matrix Requirement

If a feature touches syntax, reconstruction, reference state, or layer behavior,
find or create its row in:

```text
docs/IMPLEMENTATION-MATRIX.toml
```

`docs/SPEC-MAPPING.md` holds spec sources and citation rules, not per-feature
status.

## When to Ask

Ask the maintainer before making algorithmic encoder choices or resolving
ambiguous decoder-visible behavior.

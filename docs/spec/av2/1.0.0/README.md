# AV2 v1.0.0 specification mirror

> **Copyright 2026, Alliance for Open Media.**
> This directory is a faithful, verbatim mirror of the **AV2 Bitstream &
> Decoding Process Specification, version 1.0.0**, produced by
> converting the official AOM PDF with poppler's `pdftotext -layout`.
>
> This material is **NOT** covered by the repository's PolyForm Noncommercial
> license. It is reproduced here, with attribution, as third-party reference
> material under the copyright of the Alliance for Open Media. See
> [../../../references/THIRD-PARTY-NOTICES.md](../../../references/THIRD-PARTY-NOTICES.md).

## Status of this copy

- The **PDF is the normative reference.** This Markdown is a navigation and
  citation aid: every byte of spec text is preserved verbatim inside
  ```` ```text ```` fences; only `§` navigation headings and anchors are added.
- Tables, math, and pseudocode are preserved as monospaced text (alignment from
  `-layout`), not reconstructed into Markdown tables.

## How to use it

- Start from [index.md](./index.md): it maps every `§` section to its file,
  anchor, and PDF page.
- Cite the section number (e.g. `§ 5.16`) plus the mirror path, e.g.
  `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-16`.
- `§ 9` (additional tables) is large and lives under
  [`09-additional-tables/`](./09-additional-tables/).

## Provenance

| Field | Value |
| --- | --- |
| Spec | AV2 Bitstream & Decoding Process Specification |
| Version | 1.0.0 |
| PDF URL | https://av2.aomedia.org/v1.0.0/20260528_38f28e7_AV2_Spec_v1.0.0.pdf |
| PDF sha256 | `e9916f091e4e83446aad6b4601641c5b292e569c144c4163b26a4497573b533f` |
| HTML source | https://av2.aomedia.org/v1.0.0/index.html |
| Converter | `pdftotext -layout` (pdftotext version 26.04.0) |

Machine-readable provenance is in [provenance.toml](./provenance.toml).

## Regenerating

This mirror is generated, not hand-written. Do not edit these files directly —
the `cargo xtask check-spec-mirror` gate fails on any drift. To regenerate (e.g.
for a new spec version):

```sh
scripts/spec/regenerate-av2-spec.sh \
  --version 1.0.0 \
  --url https://av2.aomedia.org/v1.0.0/20260528_38f28e7_AV2_Spec_v1.0.0.pdf \
  --sha256 e9916f091e4e83446aad6b4601641c5b292e569c144c4163b26a4497573b533f
```

The `pdftotext version 26.04.0` line records the poppler build used; `-layout`
output can vary across poppler versions, so reproduce with a matching toolchain
or run the script's `--verify` mode.

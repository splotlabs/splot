# Tasks

## 1. Mode-context ordering (§ 5.20.7.6)
- [x] 1.1 Move `find_mode_ctx` after single-reference selection.
- [x] 1.2 Split the compound reader into reference-pair + mode stages;
      re-check the neighbour-context defer against the pair context.

## 2. Smooth-mask interintra (§ 7.13.3.29 / § 7.13.3.30)
- [x] 2.1 `Ii_Weights_1d` + per-plane mask weight + blend kernel in
      `splot-recon` (table byte-compared against the spec mirror).
- [x] 2.2 WARPMV wiring: predict DC(+IBP)/V/H per plane before MC,
      blend after; thread the mode through `InterBlock`.
- [x] 2.3 Fail-closed defers: wedge mask, II_SMOOTH, SIMPLE-path tail
      (negative fixture pin for the SIMPLE-path defer).

## 3. Per-transform-unit intra prediction (§ 5.20.7.24)
- [x] 3.1 Per-unit re-scoped plans through the single-rect arms;
      square-plan arms mapped to kernel-identical rect arms.
- [x] 3.2 Mark `BlockDecoded` per transform unit; re-derive the
      above-MRL read offset per unit.
- [x] 3.3 IBP-DC on the intra-in-inter DC path (`enable_ibp` threaded).
- [x] 3.4 Convert the txsplit fixture to a positive byte-exact pin with
      dual-reference evidence; verify the retained b02 oracle streams.

# Tasks

## 1. Warp reference list
- [x] 1.1 `WarpParamStack` construction under `DeriveWrl` (§ 7.12.2.3/.4
      corners, § 7.12.2.9 spatial inserts, § 7.12.2.20 bank/gm/default
      tail, § 7.12.2.11 no-dedup four-slot insert).
- [x] 1.2 § 5.20.2.2 warp parameter bank (row-scoped contents, per-SB
      hits + list-0 seeding, § 5.20.7 unconditional per-block update)
      via shared seed-walk/bank-ring helpers with the MV bank.
- [x] 1.3 Consumers: § 7.12.2.2 WARPMV predicted MV, stack-based
      WARPMV/DELTAWARP parameter bases; identity-base bypasses and the
      `ref_warp_idx != 0` defer retired.
- [x] 1.4 Unit tests: stack ordering/cap/no-dedup, bank MRU/evict/
      row-scope/seed, hand-computed corner model, § 7.12.2.2 projection.

## 2. MV stack inputs
- [x] 2.1 § 7.13.3.20 `SubMvs` per-cell projections for warp blocks;
      § 7.12.2.12 `get_mv` consumers switched.
- [x] 2.2 § 7.12.2.20 mixture candidates for >32x32 blocks.
- [x] 2.3 § 7.12.2.19 strict-max-weight nearest reorder under the
      sequence `DrlReorder` mode, suppressed by `useTemporalFirst`;
      unit test pins both arms.
- [x] 2.4 § 7.14.4 TCQ `dqDenom` extra shift gated on `TX_CLASS_2D`.

## 3. Verification
- [x] 3.1 AVM-exact warp params/MVs at every pinned frame-2
      discriminator; mixture pick verified at the drl=3 block.
- [x] 3.2 Frame-0 sentinel, 182-stream sweep, frontier pin unchanged.
- [x] 3.3 Frame-2 residual attribution loop (intra-in-inter tx-type
      chain, WARP_CAUSAL fit) until the parse-value dirt is owned by
      named follow-ups (all 135 frame-2 MVs byte-exact; remaining
      recon dirt owned by the § 7.13.2.9 IBP angular-blend follow-up).

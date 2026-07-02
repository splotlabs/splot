# Tasks

## 1. Warp motion family
- [x] 1.1 Neighbour grid groundwork (motion mode, warp params, geometry,
      MV-stack offsets, extend deltas, WarpSampleFound[1]).
- [x] 1.2 EXTENDWARP syntax arm + § 7.13.3.24 estimation (AVM-verified
      params on the mission stream's first EXTENDWARP block).
- [x] 1.3 LOCALWARP § 7.12.3 samples + § 7.13.3.23 least squares.
- [x] 1.4 § 7.13.3.20 extended block warp for skipPred geometry.
- [x] 1.5 Warp blocks record non-NEWMV neighbour modes (§ 7.11.3);
      DRL divergence pinned against AVM accounting.

## 2. BAWP (§ 7.13.3.25)
- [x] 2.1 Retain the full parsed syntax; derive implicit/explicit alpha
      and the template beta; apply Clip1((orig*alpha+beta)>>8) post-MC.

## 3. Per-unit intra completion
- [x] 3.1 Directional/smooth square-plan mappings; middle-unit
      zeroed-corner counts.

## 4. Output ordering (§ 7.21)
- [x] 4.1 Display-order scheduler (hold/flush/successive/refresh/EOS).

## 5. MV stack
- [x] 5.1 § 7.12.2.21 reference MV bank + § 5.20.2.2 reset/re-seed.

## 6. Header tail
- [x] 6.1 § 5.18.2 tip_frame_mode read; TIP frames fail closed.

## ADDED Requirements

### Requirement: General intra § 7.18 chroma CDEF decode-support coverage
The decoder support model SHALL record, on the `general-intra-cdef` row
(`DECODE-GENERAL-INTRA-CDEF`), that nonzero chroma (uv) CDEF strengths are admitted
and oracle-pinned: the § 7.18.1 `Cdef_Uv_Dir` direction selection, the 4:2:0
subsampled chroma tap addressing, and the `CdefDamping - 1` chroma damping. The row
SHALL carry the reciprocal LOCAL-REFERENCE-EVIDENCE pointer for the
`syn-2sb-cdefuv-intra-128x64-q170.ivf` fixture, SHALL stay partial, and SHALL keep
multi-strength frames, 10-bit CDEF, non-4:2:0 chroma, multiple tiles, the other
in-loop filters, and inter frames out of scope.

#### Scenario: Matrix records general intra chroma CDEF support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support matrix
- **THEN** row `general-intra-cdef` records the nonzero-uv chroma CDEF admission
  and cites the `syn-2sb-cdefuv-intra-128x64-q170.ivf` LOCAL-REFERENCE-EVIDENCE
  entry
- **AND** it remains marked partial rather than supported for full runtime decode
- **AND** it does not claim multi-strength CDEF, 10-bit CDEF, non-4:2:0 chroma, the
  other in-loop filters, or inter frames

#### Scenario: Reference evidence pins the chroma CDEF oracle agreement
- **WHEN** `cargo xtask check-reference-evidence` validates
  `docs/LOCAL-REFERENCE-EVIDENCE.toml`
- **THEN** the `lref-avm-dav2d-syn-2sb-cdefuv-intra-128x64-q170` entry records the
  avmdec and dav2d raw-output MD5 digests with a digest-equality assertion for the
  nonzero-uv fixture

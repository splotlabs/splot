## 1. Runtime Gate

- [x] 1.1 Admit leading IVF payloads with at least `[TD, SEQ, CLK]` through
      sequence parsing and validation.
- [x] 1.2 Reject any additional OBU after the leading key OBU for otherwise
      supported sequences before caller-visible output.
- [x] 1.3 Preserve the typed leading OBU checks for temporal delimiter,
      sequence header, and closed-loop-key positions.

## 2. Tests And Tracking

- [x] 2.1 Add a self-contained 10-bit leading-extra-payload regression proving the
      runtime reaches `unsupported_bit_depth`.
- [x] 2.2 Add a self-contained 8-bit leading-extra-payload regression proving the
      runtime still fails closed at `unexpected_leading_obu_after_key`.
- [x] 2.3 Update the local ac0ej3 CLI diagnostic regression to the new gate.
- [x] 2.4 Update feature/support tracking and run the required checks.

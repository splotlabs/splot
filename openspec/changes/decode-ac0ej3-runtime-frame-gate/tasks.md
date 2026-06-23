## 1. Runtime Gate

- [x] 1.1 Remove the total-frame-count cap from `ensure_multiframe_plan_shape`.
- [x] 1.2 Preserve leading `[TD, SEQ, CLK]` validation, fatal container-error
      rejection, and per-following-candidate temporal-delimiter checks while
      allowing only terminal trailing partial IVF header warnings.
- [x] 1.3 Ensure unsupported long streams still fail before caller-visible output
      at the first precise runtime gate.

## 2. Tests And Tracking

- [x] 2.1 Add a four-frame multiref regression proving the old total-count gate is
      bypassed and the existing `inter_too_many_valid_references` gate fires.
- [x] 2.2 Add a local ac0ej3 diagnostic regression for the current first runtime
      gate after this change.
- [x] 2.3 Update decoder support / implementation tracking and regenerate derived
      decoder support status.
- [x] 2.4 Run focused tests plus the required repository gates.

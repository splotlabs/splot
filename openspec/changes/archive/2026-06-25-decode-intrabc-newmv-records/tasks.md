# Tasks

- [x] 1.1 Extend the tile CDF selector/storage for §5.20.7.20 MV rows needed by
      IntrABC `MV_INTRABC_CONTEXT` and P=3/P=5 precision while preserving the
      existing P=6 inter path.
- [x] 1.2 Generalize the bounded SHELL-coded `read_mv()` helper to accept
      `MvPrecision` and `MvCtx`, keeping the existing inter wrapper behavior.
- [x] 1.3 Implement bounded IntrABC `assign_mv(0)` record handoff that retains
      NEARMV/NEWMV block vectors and rejects before prediction.
- [x] 1.4 Update focused unit tests, the local decoder mission probe expectation, feature
      tracking, decoder-support tracking, and generated docs.
- [x] 1.5 Run focused tests plus feature-status, decoder-support, conformance,
      fixtures, and `cargo xtask ci`.

# ADR-0006: Minimal version selection instead of a solver

**Status:** accepted

**Context.** Dependency solvers (npm, Cargo, pub, pip) pick the "newest compatible" version.
The result depends on when they run; reproducibility comes from a lockfile, and losing or
regenerating it changes behavior. Solver errors ("could not find a consistent set") are hard
to explain and fix — especially for a model. On top of that, a Vard app links one version of
each package (a shared namespace), so version selection must be unambiguous.

**Decision.** Minimal version selection (as in Go modules). A requirement is a minimum on a
compatibility line (`"1.4"` = ≥ 1.4.0, < 2.0.0; `0.x` lines are separate). Walk the
requirement graph from the app; for each package take the largest of the minimums;
requirements on different lines are `P006`, listing who wants what. Versions on a line are
published only in increasing order, so "the smallest existing version ≥ the minimum" is
stable. An upgrade is an explicit edit of the requirements in `vard.toml` (`vard update`),
visible in the diff.

**Consequences.** + the same result today, tomorrow and in CI, with or without a lockfile;
no backtracking; every error is explainable in one line; the lockfile only pins hashes and
capabilities. − new fixes do not arrive by themselves — `vard update` brings them; package
authors should raise their minimum requirements when they need a dependency's fix.

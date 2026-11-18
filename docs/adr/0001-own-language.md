# ADR-0001: Our own Vard language instead of a DSL in Kotlin/Swift/TS

**Status:** accepted

**Context.** Applications are written by an LLM. It needs a short reference, unambiguous
rules, instant checking with fixes, and clear errors. Existing languages offer a huge
surface (generics, inheritance, async models, 1–5 minute toolchain builds) and compiler
errors designed for people.

**Decision.** A small statically typed language with syntax close to Swift/Kotlin/TS (strong
priors in models), one way to write each thing, and a compiler that knows models' typical
"habits" from other frameworks and turns them into machine fixes.

**Consequences.** + the full reference ≈ 3k tokens; a check ≈ 20 ms; errors fix themselves.
− we maintain our own compiler and tooling; platform interop only through `extern fn` (see
the roadmap).

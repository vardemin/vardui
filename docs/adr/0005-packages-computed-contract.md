# ADR-0005: Packages with a contract computed by the compiler

**Status:** accepted

**Context.** Dependencies in VardUI projects are added mostly by an LLM agent. In classic
package managers (npm, Cargo, pub, SwiftPM) a package is described by its author's claims:
version, description, changelog. For an agent that means: (1) a guessed name may belong to
someone else's package (slopsquatting), (2) semver and descriptions cannot be verified more
cheaply than by reading the code, (3) what the package does with the device is written down
nowhere, (4) an upgrade requires reading a changelog and editing by hand. Separately: the
language deliberately has no imports (ADR-0001), and packages bring in foreign names.

**Decision.**
1. A package = code + **a contract computed by the compiler** (public API and capabilities) +
   knowledge (AGENT.md ≤ 6 KB, the generated API, skills) + proof (headless tests at publish)
   + migrations (`[[migrate]]` → `E0234` with a `fix:`) + simulation (`[sim]`).
2. Semver is enforced: the registry compares the API with the previous version (`P015`).
3. Capabilities (`secure permissions push native network prefs links share deeplinks`) are
   computed from the code; the manifest must match (`P013`); a person approves sensitive ones
   in `[policy]` (`P008`); `vard.lock` records them.
4. Installation by exact names only (`P001` with "did you mean"); the registry cuts off
   look-alike names (`P010`).
5. Namespaces without imports: own → direct dependencies → std; a conflict of two packages —
   `pkg.Name`; transitive dependencies are invisible; `_` is private; native names are
   global. The linker renames conflicting declarations before type checking — the runtime
   knows nothing about packages.

**Consequences.** + an agent knows a package's API and risks before installing, upgrades
mechanically, cannot install a guessed name or grant a package unapproved rights; adding a
package does not change the meaning of other code. − packages need an AGENT.md and an
example with tests; one version of a package per app (a consequence of the shared
namespace); capabilities limit "what", not "how" (an approved `network` can still send data).

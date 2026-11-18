# Package system overview

## The problem

In today's ecosystems (npm, Cargo, Go modules, SwiftPM, pub.dev) a package is an archive of
code plus what its author *claims* about it: a version, a description, dependencies.
Everything else — what the package can do, what breaks on upgrade, whether to trust it — a
person finds out by reading code, changelogs and issues.

In VardUI, dependencies are mostly added by an LLM agent. For an agent the usual model has
three systemic defects:

1. **Hallucinated names.** Models confidently "remember" packages that do not exist, and
   attackers register those names ("slopsquatting"). Installing by a guessed name is a
   classic attack path.
2. **Claims instead of facts.** Semver is an honor system; changelogs are written for
   people; what a package does with the device is written down nowhere. An agent cannot
   check any of it more cheaply than by reading all of the code.
3. **Expensive knowledge.** To start using a package an agent reads a README of thousands of
   tokens, and after an upgrade it guesses what was renamed.

## The solution: package = code + contract + knowledge + proof

| Part | What it is | Who produces it |
|---|---|---|
| **Code** | `.vard` sources, optionally a native module and resources | the author |
| **Contract** | the public API and **capabilities** | **the compiler** — computed from code, not declared |
| **Knowledge** | `AGENT.md` ≤ 6 KB (≈ 1500 tokens) + the generated API + skills | the author + the compiler |
| **Proof** | headless tests of `example/` and the package, run by the registry at publish | the registry |
| **Migrations** | renames that become `fix:`es for consumers | the author |
| **Simulation** | `[sim]` answers for the package's externs in headless mode | the author |

This gives properties classic package managers do not have:

- **Semver is enforced.** The compiler compares the new version's API with the previous
  one: removing or changing something is major, adding is minor, anything else is a patch.
  The registry will not accept 1.0.1 with a broken API (`P015`) and suggests the minimal
  acceptable version.
- **Capabilities are facts, not promises.** The compiler sees the code call `secureGet`,
  declare an `extern fn`, subscribe to `pushReceived` or load `https://`. The manifest must
  match what was computed (`P013`); the consumer must approve sensitive capabilities in
  `[policy]` (`P008`); `vard.lock` records them, so a new capability in a dependency is a
  line in a code-review diff.
- **Upgrades are mechanical.** `vard update` prints the API diff, and `vard check --fix`
  applies the package's migrations to the app's code (`E0234`). The agent does not read a
  changelog.
- **Installation never guesses.** `vard add` installs exact names only; a typo gets "did you
  mean" from the real index (`P001`). The registry refuses a name one edit away from an
  existing name of another owner (`P010`).
- **Knowledge is cheap and bounded.** `vard doc` shows the project's packages one line
  each; `vard doc NAME` shows `AGENT.md` and the generated API. The document budget is
  checked at publish (`P011`). The package block in the project's `AGENTS.md` is updated on
  install, so an agent sees the dependencies without a single tool call.
- **A package is verifiable without a device.** Tests run headless at publish; `[sim]` gives
  consumers a working `vard run` right after `vard add`, even for packages with native code.

## Namespaces without imports

The language deliberately has no imports ([ADR-0001](../adr/0001-own-language.md)): one form
of writing, no line where a model can get a path wrong. Packages keep that:

1. an unqualified name is looked up in its own package (the app is a package too), then in
   **direct** dependencies, then in std;
2. your own declarations always win over packages;
3. if two direct dependencies export a name — `E0231`, write `pkg.Name`;
4. transitive dependencies are invisible (`E0232`), `_` names are private (`E0230`);
5. `extern fn/event` names are global — the platform calls them (`E0233`).

The result: adding or upgrading one package **cannot change the meaning** of anyone else's
code. The compiler renames conflicting declarations to `pkg.Name` before type checking, so
the runtime and inspection know nothing about packages. Details:
[Using packages](using.md#namespaces); implementation: `crates/vard-lang/src/link.rs`.

## Version resolution: minimal version selection

A requirement is a minimum on a compatibility line (`"1.4"` = ≥ 1.4.0, < 2.0.0). For each
package the **largest of the minimums** anyone requires is selected
([ADR-0006](../adr/0006-minimal-version-selection.md)). No SAT solver, no "latest": the result
depends only on the manifests — today, tomorrow and in CI. The lockfile decides nothing; it
pins hashes and records capabilities. One version of a package per app (a shared
namespace); incompatible lines are `P006`.

## Hosting: static, content-addressed, signed

A registry is a set of static files ([ADR-0007](../adr/0007-static-signed-registry.md)):
`config.json`, `catalog.json`, `index/NAME.json`, `blobs/SHA256.vpk`, and a signed log
`log.txt` + `log.head`. Any CDN, bucket, GitHub Pages or directory can serve it;
`vard registry serve` adds a publish API. A client pins the registry key on first contact,
verifies the signature and the log's hash chain, and then the hash of every archive. Trust
does not depend on the host: a mirror can delay an update but cannot substitute what was
published. Details: [Registries and hosting](registry.md); threats: [Security model](security.md).

## Delivery to devices

`vard bundle` collects everything a device executes: the app's and packages' sources
(`@name/...`), `vard.lock`, resources, and `bundle.json` with a hash and a **native
fingerprint** — the list of externs and native modules. The Android shell runs
`vard bundle` during the build and includes the packages' native modules automatically. The
fingerprint is the compatibility key for over-the-air updates: an interpreted bundle can be
delivered without rebuilding the APK as long as the native part did not change. Details:
[Delivery to devices](delivery.md).

## What is implemented and what is on the roadmap

| Implemented (v0.3) | Roadmap |
|---|---|
| vard.toml, vard.lock, MVS, paths and registries as sources | git sources for dependencies |
| namespace linker, `pkg.Name`, privacy, migrations | migrations of field types and enum variant renames |
| API and capabilities computed by the compiler; enforced semver | behavioral compatibility checks with the previous version's tests |
| publish gate: compile, capabilities, tests, semver, AGENT.md budget, name | quarantine for new packages, build attestations (provenance) |
| static registry, signed log, key pinning, prefix check | Merkle inclusion proofs instead of the full log; key rotation |
| `vard registry serve` with token-based publishing, yank | web UI, organizations and roles |
| the `builtin` registry (tailwind, firebase) | an official public registry |
| `vard bundle` with the native fingerprint; automatic native modules on Android | an OTA update client in the Android/iOS hosts; iOS modules in packages |

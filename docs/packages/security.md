# Package security model

The starting premise: dependencies are added by an agent that can make mistakes and can be
deceived by text (documentation, an issue, a web page). So the system is built so that
(1) the agent cannot install the wrong thing, (2) a package cannot get more than a person
approved, and (3) nobody between the author and the device can substitute the code.

## Threats and defenses

| Threat | Defense | Code |
|---|---|---|
| The agent "remembered" a non-existent name; an attacker registered it (slopsquatting) | `vard add` installs exact names only; a miss gets "did you mean" from the real catalog, never an automatic substitution; the MCP tool description requires searching, not guessing | `P001` |
| Typosquatting (`tailwnd`, `charts` ↔ `chart`) | the registry refuses a name one edit away from an existing name of another owner, and reserved words | `P010` |
| A package silently gains access to secrets, permissions, the network, native code | the compiler computes capabilities; the manifest must match; sensitive ones need `[policy] allow`; the MCP tool cannot approve | `P013`, `P008` |
| An update gives a package a new capability | `vard.lock` stores `caps=`; the change is a `!` line in the report and in the diff; without approval installation is refused | `P008` |
| A private registry's dependency replaced by a public one (dependency confusion) | a dependency comes from exactly one registry; where it resolved is recorded in the author's index entry | — |
| An archive substituted on the host/mirror/CDN | content addressing: the archive is checked against the `sha256` of the index and the lockfile | `P004` |
| The index substituted | the `(name, version, sha256)` entry must be in the signed log | `P004` |
| The whole registry substituted / MITM | the public registry's key is compiled into `vard`; other registries' keys are pinned on first contact; a changed key is an error | `P005` |
| History rewritten (an old version quietly replaced) | the log is a hash chain; the client checks that the head it saw is a prefix; versions are immutable | `P005`, `P016` |
| A package breaks other code by "capturing" names | your own names win; transitive dependencies are invisible; a conflict of two packages needs explicit qualification | `E0231`, `E0232` |
| Two packages claim one native function | native names are global, a conflict is a compile error | `E0233` |
| A false "backward compatible" | semver is computed from the API diff | `P015` |
| A malicious package with deliberately broken tests etc. | the gate runs on the server: compile, headless tests, size limit | `P012`, `P014`, `P019` |
| Prompt injection through a package's docs | AGENT.md is limited to 6 KB; the skill and the project's AGENTS.md tell the agent never to approve capabilities by itself; approval happens only through `vard.toml`, which a person reviews | — |
| A leaked publish token | only `sha256(token)` is stored; a token is bound to its owner; yank to withdraw versions | `P018` |

## What the system does not promise

- **No malicious intent in a package's code.** Capabilities limit *what* a package can do,
  not *how*: a package with approved `network` can send data. The approval is a person's
  decision; `vard info` shows why each capability is needed.
- **Native code.** The `native` capability means running the package's Kotlin/Swift code
  outside Vard's limits. Approve it as carefully as a Gradle dependency.
- **Availability.** A mirror can withhold updates (a freeze attack). That is detectable only
  by comparing with another source; log witnesses are on the roadmap.

## Recommendations

- Commit `vard.lock` and watch `caps=` and `sha256=` in diffs.
- In CI: `vard install` (without `--allow`) fails if someone added a dependency with an
  unapproved capability; run `vard registry verify URL` periodically.
- Keep a registry's `private/` out of public hosting; keep the key in CI secrets.
- For agents: the `vard-packages` skill already contains the rule "on `P008`, stop and ask
  the user".

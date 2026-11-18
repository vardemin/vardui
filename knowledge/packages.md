# Vard packages (`vard doc packages`)

A package is code + a contract the compiler computes (public API, capabilities) + agent
docs (AGENT.md, ≤ 6 KB) + proof (headless tests run at publish). Dependencies live in
`vard.toml`; there are still no imports.

## Using packages
```
vard search "bar chart"            # find by words: names, keywords, summaries, exported names
vard info charts                   # versions, caps, full API, tests — before adding
vard add charts                    # exact names only; writes vard.toml + vard.lock
vard add charts@1.4                # minimum version 1.4.0 (line 1.x)
vard add ui --path ../ui           # a local package (monorepo)
vard doc charts                    # its AGENT.md + generated API: read this before using it
vard update [charts] [--major]     # newer versions + API diff; then `vard check --fix`
vard remove charts
```
- Never invent a package name. Unknown name = `P001` with "did you mean"; search instead.
- After `vard add`, public names of the package are visible in app code without imports:
  `BarChart(values: [1, 2])`, `ChartStyle`, `formatAxis(x)`.
- Names collide? Your own declarations always win. Two packages exporting one name =
  `E0231`: qualify it, `charts.Badge(...)`, `ui.Theme`, `geo.distance(a, b)`.
- Only direct dependencies are visible (`E0232`); `_names` are package-private (`E0230`).
- `P008` = the package needs a sensitive capability (secure, permissions, push, native,
  network). Do not work around it: tell the user what it needs and why (`vard info NAME`);
  a person approves by adding it to `[policy] allow` (or `vard add NAME --allow native`).
- `E0234` after an update = a renamed name/param; `vard check --fix` applies the package's
  migration.
- Package externs answer headless from the package's `[sim]`; override with `mock`/`resolve`.
- Diagnostics in package files are shown as `@name/file.vard:LINE`; never edit them (they
  are read-only inputs); fix your code or change the version.

## vard.toml (app)
```toml
[app]
name = "shop"

[dependencies]
charts = "1.4"                                  # >= 1.4.0, < 2.0.0 (0.x: "0.3" = 0.3.*)
auth = { version = "2", registry = "acme" }
ui = { path = "../ui" }

[policy]
allow = ["native", "push"]                      # approved sensitive capabilities

[registries]
acme = "https://vard.acme.dev"                  # or a directory
default = "acme"                                # default: vard (the public registry)
```
Registries: the project's [registries], ~/.vard/config.toml, then the public registry `vard`
(default; https://vardemin.github.io/vard-registry, publish = a pull request to
github.com/vardemin/vard-registry), then `builtin` (bundled, offline). Offline:
`--registry builtin` or VARD_OFFLINE=1.

vard.lock is generated: one line per package with `sha256=`, `api=`, `caps=`, `deps=`. Commit
it. Any vard command reinstalls when vard.toml changed, so editing vard.toml by hand is fine.

## Resolution
Minimal version selection: each package gets the largest of the minimum versions anyone
requires — never "latest". Same inputs, same result, no solver. One version per package per
app; requirements on different major lines are `P006`.

## Writing a package
```
mylib/
  vard.toml          [package] name version summary keywords vard capabilities
  *.vard             sources (everything except example/ native/ skills/ tests/)
  AGENT.md           for agents: purpose, main views with one example each, headless notes
  example/           an app using the package: vard.toml (mylib = { path = ".." }), app.vard, tests/*.vtest
  skills/NAME/SKILL.md   optional, copied into consumers' .claude/skills/mylib-NAME/
  native/android/    optional Gradle library; [native.android] module = "...", gradle = "native/android"
```
```toml
[package]
name = "charts"                 # lowercase identifier; also the qualifier in code
version = "1.4.0"
summary = "Bar and line charts"
keywords = ["chart", "graph"]
vard = ">=0.1"
capabilities = []               # must equal what `vard api --caps` computes

[sim]                           # headless answers for your externs (JSON in a string)
currentUser = 'null'

[[migrate]]                     # renames → E0234 + fix in consumers
since = "2.0.0"
from = "BarChart.max"           # a view param; or a top-level name: from = "Chart"
to = "BarChart.scale"
```
Prefix private helpers with `_`. Keep exported names distinctive (they share the app's
namespace). `vard api` shows the public API; `vard publish --dry-run --registry R` runs the
gate: compiles with deps, capabilities, tests (example/ included), semver vs the previous
version (removing/changing = major, adding = minor), AGENT.md budget, name policy.

## Registries and delivery
- `vard registry init DIR` — static registry: `public/` (config.json, catalog.json,
  index/NAME.json, blobs/SHA.vpk, signed log) + `private/` (key, tokens). Serve `public/`
  from anything; `vard registry serve DIR` adds `POST /api/v1/publish`.
- `vard publish --registry DIR|URL --token T`; `vard yank NAME@VER --registry DIR`.
- Clients pin the registry key, verify the signed hash-chained log and every blob hash:
  mirrors and CDNs are untrusted (`P004`, `P005`).
- `vard bundle --out DIR|FILE.vpk` — everything a device runs + bundle.json with the native
  fingerprint (externs + native modules): an over-the-air bundle is compatible with an
  installed app only if the fingerprints match.

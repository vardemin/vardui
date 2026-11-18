# Writing a package

## Layout

```text
charts/
  vard.toml              manifest: [package], dependencies, [sim], [[migrate]], [native.android]
  bars.vard              sources: every .vard outside the directories below
  AGENT.md               for agents (required, ≤ 6 KB)
  README.md              for people (optional)
  example/               the proof app: compiled and tested at publish
    vard.toml            [dependencies] charts = { path = ".." }
    app.vard
    tests/*.vtest
  skills/NAME/SKILL.md   optional skills; consumers get them as .claude/skills/charts-NAME/
  native/android/        an optional Gradle library with the native implementation
  assets/, fonts/        resources
```

A package's sources are all `.vard` files except those in `example/ examples/ native/
skills/ tests/` and hidden directories. The archive gets every file except hidden ones,
`target/`, `build/`, `*.vpk` and `vard.lock`.

Examples in the repository:
[`packages/tailwind`](https://github.com/vardemin/vardui/tree/master/packages/tailwind)
(a design system) and
[`packages/firebase`](https://github.com/vardemin/vardui/tree/master/packages/firebase)
(a native SDK).

## The manifest

```toml
[package]
name = "charts"                 # [a-z][a-z0-9_]*, up to 32 chars; also the qualifier in code
version = "1.4.0"
summary = "Bar and line charts drawn with Vard elements"   # one line — shown in search
keywords = ["chart", "graph", "bar"]
license = "MIT"
repository = "https://example.com/charts"
vard = ">=0.1"                  # core versions the package works with
capabilities = []               # must equal the computed set (`vard api --caps`)

[dependencies]
tailwind = "1.0"                # registry versions only: path dependencies are not publishable

[sim]                           # headless answers for your extern fns (JSON in a string)
chartsServerTime = '1700000000'

[[migrate]]                     # renames → E0234 + fix for consumers
since = "2.0.0"
from = "BarChart.max"           # a view parameter: View.param
to = "BarChart.scale"

[[migrate]]
since = "2.0.0"
from = "Chart"                  # a top-level name
to = "BarChart"

[native.android]
module = "dev.example.charts.ChartsModule"   # the VardModule class
gradle = "native/android"                    # an Android library inside the package
```

## Public API

Public is everything declared at the top level that does not start with `_`: views,
structs, enums, functions, `extern fn/event`, constants, theme tokens. See what consumers
see:

```bash
vard api charts/            # API + capabilities with evidence
```

```text
view BarChart(values: [Int], max: Int = 100)
view Badge(label: String)
capabilities: none
```

Recommendations:

- Helpers start with `_`: `view _Bar(...)`, `fn _scale(...)`. They can change without a
  major version.
- Exported names should be distinctive: they live in the app's namespace. A conflict with
  another package will not silently break the build (`E0231`), but it forces consumers to
  qualify.
- Parameters with defaults — adding one stays a minor change.
- A package's theme tokens (`theme { }`) become the app's tokens: that is how design systems
  are shipped (this is how `tailwind` works).

## AGENT.md

What an agent reads to use the package (`vard doc charts` = AGENT.md + the generated API +
capabilities). The budget is 6 KB ≈ 1500 tokens (`P011`). Write for a model:

1. one sentence — what it is and when to use it;
2. the main views/functions — one compiling example each;
3. non-obvious rules (what goes into a `bind`, which tokens to use);
4. how to test headless: what `[sim]` answers, what needs `resolve`/`emit`.

Do not repeat the API list — it is generated and always accurate.

## The example as proof

`example/` is an ordinary app that depends on the package by path. The publish gate
compiles it and runs every `.vtest` (a test runs against the nearest `vard.toml`). It is both
"how to use it" documentation and a check that the package works with the current core.

```bash
vard test charts/           # the package's and example/'s tests
```

## Capabilities

The compiler computes capabilities from the code: calls to `secureGet/secureSet` →
`secure`, `requestPermission` → `permissions`, `getPushToken`, `showNotification`,
`on pushReceived` → `push`, `pref` and `prefGet` → `prefs`, `openUrl` → `links`, `shareText` →
`share`, `on deepLink` → `deeplinks`, `https://` strings → `network`, declaring
`extern fn/event` → `native`. The manifest's `capabilities` must match (`P013`, with a ready
replacement line). Extra capabilities are a warning: consumers would have to approve
something unneeded.

## Native code

A package can ship the implementation of its `extern fn/event`:

1. declare the API in Vard in the package's sources;
2. generate a base class: `vard bindings charts/ --kotlin --package dev.example.charts --class ChartsModuleBase`;
3. put an Android library into `native/android/` (`build.gradle.kts`, an `AndroidManifest.xml`
   with `<meta-data android:name="dev.vard.module.charts" android:value="dev.example.charts.ChartsModule"/>`,
   the Kotlin implementation);
4. add `[native.android]` to the manifest;
5. set `[sim]` so consumers can run the app headless.

The Android shell includes the module automatically: `settings.gradle.kts` asks
`vard android-modules <app>` and includes each library as `:vard-<name>`. Example:
[`packages/firebase/native/android`](https://github.com/vardemin/vardui/tree/master/packages/firebase/native/android).

## Versions and migrations

The gate compares the API with the previous published version:

| Change | Minimal step |
|---|---|
| a name, parameter, field or enum variant removed | major |
| a type, `bind`, return value or the first (positional) parameter changed | major |
| a parameter became required; a required parameter or a field without a default added | major |
| a name, enum variant, or a parameter/field with a default added | minor |
| nothing in the API | patch |

For `0.x` a major change is a minor bump (`0.3 → 0.4`), as in Cargo.

```text
error P015: API change since 1.0.0 is major: ! ~ view BarChart: `max` removed — the version must be at least 2.0.0
  fix: version = "2.0.0"
```

Accompany every rename with `[[migrate]]` — then `vard check --fix` rewrites the consumer's
code by itself.

## Publishing

```bash
vard publish charts --registry acme --dry-run        # the gate only
vard publish charts --registry acme --token $TOKEN
```

```text
gate: charts 1.0.0
  ok    AGENT.md: 78 B of 6.0 KB
  ok    compiles (1 file(s), 1 dependency)
  ok    capabilities: none
  ok    tests: 1 passed (headless)
  ok    api: 2 views (first version)
  ok    archive: 6 files, 822 B, sha256 100e077ddd80943d
published charts 1.0.0 to acme (log #1)
```

The gate checks: the name (valid, not reserved, not one edit from someone else's), a
one-line summary, no path dependencies, AGENT.md and its budget, archive size (≤ 8 MB),
compilation with dependencies, capabilities, `[sim]`, migrations, tests, version
immutability and semver. The registry server runs the same checks when accepting over HTTP.

To withdraw a version (a serious bug, a leaked secret): `vard yank charts@1.0.1 --registry DIR`.
Existing lockfiles keep working; new resolutions skip it.

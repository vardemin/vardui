---
name: vard-packages
description: Find, add, update or write Vard packages (vard.toml, vard.lock, registries). Use when the app needs a capability that may exist as a package (charts, auth, payments, maps, a design system), when a vard command reports P0xx / E023x codes, or when creating or publishing a package.
---

# Packages in Vard

Reference: `vard doc packages`. Codes: `vard explain P008`.

## Need a feature
1. `vard search WORDS` (e.g. `vard search date picker`). Never guess a name.
2. `vard info NAME` — check caps and API. Prefer packages with tests ✓ and no sensitive caps.
3. `vard add NAME`, then `vard doc NAME` and build with its views/functions.
4. `vard check` → `vard run --do tree` → add a `.vtest`.

## When something fails
- `P001`: name does not exist — use the did-you-mean list or search.
- `P008`: needs a sensitive capability. Stop and ask the user, quoting what the package
  needs (`vard info NAME` explains each capability). Do not add `[policy] allow` yourself
  unless the user said to.
- `E0231`: two packages export the name — write `pkg.Name`.
- `E0232`: name from a package you do not depend on directly — `vard add` it.
- `E0234` after `vard update`: run `vard check --fix`.
- `P024`: the package is brand new (< 72h). Tell the user who published it (`vard info NAME`:
  owner, provenance) and let them decide; only they run `vard add NAME --allow-new`.
- `P006`: two dependencies need different major versions of one package — update the
  older dependent (`vard update NAME --major`) or ask the user.

## Writing a package
`vard doc packages` → "Writing a package". Checklist: lowercase name, one-line summary,
AGENT.md ≤ 6 KB written for agents (purpose, one example per main view, headless notes),
example/ app with tests, `_` prefix for private helpers, `capabilities` equal to
`vard api --caps`, `[sim]` for externs, `[[migrate]]` for every rename. Validate with
`vard publish --dry-run --registry R`; publishing itself is the user's call.

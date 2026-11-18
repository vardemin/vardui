# Diagnostics

Every diagnostic carries a stable code, the location and the line of code, and — when the
fix is known — a `fix:` that `vard check --fix` applies. A short explanation:
`vard explain CODE`; the full list with explanations: `vard doc errors`.

```text
app.vard:2:14 error E0202: unknown prop `fontSize` on `Text`
  2 | Text("Hi", fontSize: 18)
  fix: `fontSize` -> `size` (auto: vard check --fix)
```

## Classes

| Prefix | Where | Examples |
|---|---|---|
| `E01xx` | syntax | `E0106` stray `;`, `E0110` a foreign keyword, `E0111` an import, `E0119` a modifier chain |
| `E02xx` | names, elements, props, packages | `E0201` unknown element, `E0202` unknown prop, `E0203` unknown name, `E0206` missing required argument |
| `E03xx` | types | `E0301` type mismatch, `E0304` unknown enum variant, `E0314` optional not unwrapped, `E0317` cannot cross the native boundary |
| `E04xx` | mutability | `E0401` assignment to something immutable, `E0402` in-place method on an immutable list |
| `W04xx` | warnings | `W0401` loop without `by`, `W0403` `Input` without `value` |
| `Rxxxx` | runtime (printed as `! R0003 … (file:line)`) | `R0003` state written during render, `R0010` no native implementation |
| `Lxxx` | UI lints | `L001` overflow, `L002` small tap target, `L005` contrast |
| `Pxxx` | packages and registries (location: a line of `vard.toml`) | `P001` no such package, `P008` policy, `P015` semver |

## Packages in the compiler (E023x)

| Code | Meaning | What to do |
|---|---|---|
| `E0230` | a `_` name is private to its package | use the public API (`vard doc PACKAGE`) |
| `E0231` | two direct dependencies export the name | qualify it: `charts.Badge(...)` |
| `E0232` | the name comes from a package that is not a direct dependency | `vard add NAME` |
| `E0233` | two sides declare the same native name | remove the duplicate or rename it on both sides |
| `E0234` | a name/parameter was renamed in a newer package version | `vard check --fix` |
| `E0235` | `pkg.Name`: no such package or no such public name | check the name; a typo comes with a `fix:` |

## Package manager (P0xx)

| Code | Meaning |
|---|---|
| `P001` | no package with this name; "did you mean" lists real names |
| `P002` | the registry is unreachable or is not a Vard registry |
| `P003` | no version satisfies the requirement |
| `P004` | integrity failure: the archive hash differs, or the version is not in the signed log |
| `P005` | the registry key changed or the log history was rewritten |
| `P006` | incompatible requirements for one package (different major lines) |
| `P007` | the package needs a different vard core version |
| `P008` | a sensitive capability is not approved in `[policy]` |
| `P009` | the version was yanked |
| `P010` | an invalid, reserved or too-similar-to-someone-else's name |
| `P011` | no AGENT.md, or it exceeds the 6 KB budget |
| `P012` | the package does not compile |
| `P013` | declared capabilities differ from the computed ones |
| `P014` | the package's tests failed |
| `P015` | the version number does not match the API change |
| `P016` | the version is already published or is not newer than the latest on its line |
| `P017` | `[sim]` refers to an undeclared extern |
| `P018` | no token, or not the owner's token |
| `P019` | the archive is larger than 8 MB |
| `P020` | `vard.toml` is not valid TOML (the subset Vard uses) |
| `P021` | a wrong or unknown field in `vard.toml` |
| `P022` | the project's packages are missing or out of date |
| `P023` | a minor/patch release fails the previous version's tests (behavior changed) |
| `P024` | the package is new (first published < 72 h ago): a person confirms with `--allow-new` |
| `P025` | a git dependency could not be fetched, or its tag/rev/branch does not exist |

## Rules for new diagnostics (for framework developers)

A new code is stable, has an entry in `explain.rs` (compiler), `vard_runtime::EXPLAIN`
(runtime, lints) or `vard_pkg::EXPLAIN` (packages), and a `fix` whenever the correct edit is
known. When the fix is obvious, checking continues as if it were already applied — one
iteration instead of several, no avalanche of follow-up errors.

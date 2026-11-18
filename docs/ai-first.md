# Working with AI agents

VardUI was designed around one question: *what does each step of the "write → check → run →
understand what went wrong" loop cost an agent* — in tokens, in time, and in the probability
of a mistake. Below is what the answer is made of and how to use it.

## Principles

| Principle | How it is implemented |
|---|---|
| Write it right the first time | a small regular language, syntax close to Swift/Kotlin/TS, one form for each thing |
| Fix without thinking | every diagnostic is `file:line`, the line of code, `fix:`; `vard check --fix` applies fixes to a fixed point; "habit" tables (`VStack → Column`, `onPress → onTap`, `"a" + n → "a{n}"`) |
| See without vision | a semantic tree ≈ 100 tokens; actions print diffs; `why` explains sizes; lints describe the recipe |
| Knowledge from one source | the reference is compiled into the binary and generated from the element registry — it cannot drift from the compiler |
| Determinism | headless runtime, bundled font, platform simulator, MVS for versions: same inputs, same result |
| A person decides outward-facing and risky steps | approving package capabilities and publishing are not available through MCP |

## What an agent gets in a project

`vard new` creates:

- `AGENTS.md` (and `CLAUDE.md` → `@AGENTS.md`) — the work loop, debugging rules,
  conventions, and a `<!-- vard:packages -->` block that the package manager keeps current;
- `.claude/skills/` — `vard-app` (screen patterns), `vard-debug`, `vard-native`,
  `vard-design`, `vard-packages`; packages' skills are added on install as `<package>-<skill>`;
- `.mcp.json` — the `vard mcp` server.

## Token budget

| Operation | Cost |
|---|---|
| the full language reference (`vard doc`) | ≈ 3k tokens, once per session |
| the whole screen (`tree`) | ≈ 110 tokens (a screenshot: 450–1600) |
| the result of an action | ≈ 30 tokens: only the lines that changed |
| `vard check` | ~20 ms; whatever is fixable is fixed without the model |
| a package: one line in `vard doc` / `vard doc NAME` | ≈ 30 / ≤ 1500 tokens |
| a package update | an API diff + `--fix` instead of reading a changelog |

## Recommended agent loop

1. Once per session — `vard doc` (or `vard_doc`).
2. Edit `.vard` → `vard check --fix` → fix the rest.
3. `vard run --do "…"` — the screen and actions; `why` for sizes, `state`/`log` for values.
4. A test for every feature and bug: `tests/*.vtest`, `vard test`.
5. Need "library" functionality — `vard search` → `vard info` → `vard add` → `vard doc NAME`.
   Never guess names; on `P008`, ask the user.
6. A screenshot — only to judge how it looks.

## Why packages are built this way

The package system continues the same principles ([overview](packages/)):

- **the compiler computes the contract** — an agent does not need to trust a description or
  read code to know a package's API and capabilities;
- **installation never guesses** — protection against hallucinated names;
- **knowledge has a budget** — AGENT.md ≤ 6 KB, the API is generated;
- **updates are mechanical** — package migrations become `fix:`es;
- **a package runs headless** — `[sim]` and tests at publish time;
- **risk stays with a person** — sensitive capabilities are approved in `vard.toml`.

## Working on the framework itself

Rules for agents changing VardUI are in
[CLAUDE.md](https://github.com/vardemin/vardui/blob/master/CLAUDE.md): where the single
sources of truth are (element registry, std, explain tables), what must be updated together
(knowledge, skills, examples), and how to verify (`cargo test --workspace`, `vard test` on
the examples and packages).

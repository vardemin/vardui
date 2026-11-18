# ADR-0004: A knowledge base for agents, in English, compiled into the tool

**Status:** accepted (amended: all documentation is in English)

**Context.** Documentation a model reads must be short, precise and must not drift from the
compiler's behavior.

**Decision.** `knowledge/llms.txt` is the complete reference; it is compiled into the binary
(`vard doc`, `vard_doc` in MCP), so the reference version always matches the compiler
version. Element descriptions (`vard doc Text`) are generated from `registry.rs`. Texts for
agents are in English (fewer tokens, stronger priors). The documentation for people —
README and the `docs/` site (https://vardemin.github.io/vardui/) — is in English too, so one
language serves the whole community; it was originally written in Russian.
`vard new` puts `AGENTS.md`, `CLAUDE.md`, skills and `.mcp.json` into every project.

**Consequences.** + one source of truth; an agent gets everything it needs with one command.
− when the language changes, `llms.txt` and the skills must be updated; the snippets from
the `vard-app` skill live in `examples/patterns` and are checked by `vard test`.

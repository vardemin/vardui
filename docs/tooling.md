# Tooling: CLI and MCP

`vard` is the only tool a person or an agent needs. Output discipline: every command prints
the minimum needed to decide the next step. Success is one line; a failure is `file:line`,
the line of code and a fix. Exit code: 0 on success, 1 otherwise. `PATH` is a `.vard` file or
a project directory (default: the current directory).

## Development

| Command | What it does |
|---|---|
| `vard check [PATH] [--fix] [--json]` | types; `--fix` applies machine fixes to a fixed point (never touches package files); `--json` — diagnostics as an array |
| `vard run [PATH] [--do "…"] [--script FILE] [--view V] [--size WxH] [--dark] [--no-lint]` | headless run and an action script |
| `vard test [PATH]` | every `*.vtest` |
| `vard doc [TOPIC]` | reference: no topic — the full one (+ the project's packages); topics `elements common actions std errors design native packages`, an element/enum/view/package name |
| `vard explain CODE` | explains an E/W/R/L/P code |
| `vard new NAME` | a project with `vard.toml`, a test, AGENTS.md, skills and `.mcp.json` |
| `vard bindings [PATH] --kotlin [--package P] [--class C] [--out FILE]` | typed Kotlin for the project's `extern fn/event` (excluding packages' externs — they ship their own implementation) |

## Packages

| Command | What it does |
|---|---|
| `vard search WORDS` | search by name, keywords, summary and exported names in all of the project's registries |
| `vard info NAME [PATH]` | versions, capabilities with explanations, dependencies, publication time and provenance, the full API, tests at publish time |
| `vard add NAME[@REQ] [PATH] [--path DIR \| --git URL [--tag T\|--rev R\|--branch B] [--dir D]] [--registry R] [--allow CAPS] [--allow-new]` | add a dependency (exact names only), install, update `vard.lock`; `--allow` records the approval in `[policy]`; `--allow-new` confirms a quarantined package |
| `vard remove NAME [PATH]` | remove a dependency |
| `vard install [PATH] [--allow CAPS] [--allow-new]` | resolve `vard.toml`, download and verify, rewrite `vard.lock` |
| `vard update [NAME] [PATH] [--major]` | raise requirements to the newest versions (git: re-resolve the branch/tag); prints the API diff |
| `vard api [PATH\|NAME] [--caps]` | public API and capabilities (with evidence: which call needs which capability) |
| `vard pack [PATH] [--out FILE]` | a deterministic `.vpk` archive |
| `vard publish [PATH] --registry DIR\|URL\|NAME [--token T] [--dry-run]` | the gate and publishing |
| `vard yank NAME@VER --registry DIR [--undo]` | withdraw a version |
| `vard bundle [PATH] --out DIR\|FILE.vpk` | everything a device needs + `bundle.json` |
| `vard registry init DIR [--name N]` | create a registry (`public/` + `private/`) |
| `vard registry token DIR OWNER` | a publish token (shown once) |
| `vard registry serve DIR [--addr HOST:PORT]` | HTTP hosting of a registry (web page at `/`) with a publish API |
| `vard registry rotate-key DIR` | hand the registry over to a new signing key (signed by the old one) |
| `vard registry verify DIR\|URL` | verify signatures, the log chain and every archive |

Environment variables: `VARD_HOME` (cache, keys, tokens; default `~/.vard`),
`VARD_REGISTRY` (the default registry for every project), `VARD_OFFLINE=1` (never contact
the public registry; `builtin` becomes the default), `VARD_TOKEN` (publish token).
`vard publish --owner LOGIN` sets the owner when trusted CI writes a directory registry
(the public registry's workflow passes the pull request author).

## The MCP server

`vard mcp` speaks JSON-RPC 2.0 over stdio and works on the files of the project it is
pointed at, so it runs on the developer's machine — there is nothing to host. `vard new` puts
`.mcp.json` into the project:

```json
{ "mcpServers": { "vard": { "command": "vard", "args": ["mcp"] } } }
```

Other ways to get it:

- **MCP Registry**: `io.github.vardemin/vardui`
  ([registry.modelcontextprotocol.io](https://registry.modelcontextprotocol.io)), published
  by the release workflow; clients that read the registry can install it directly.
- **`vard.mcpb`** on every [release](https://github.com/vardemin/vardui/releases) — a one-file
  bundle (macOS universal, Linux x86_64, Windows x86_64) for clients that install MCP bundles,
  such as Claude Desktop.
- Claude Code without a project file: `claude mcp add vard -- vard mcp`.

| Tool | Equivalent |
|---|---|
| `vard_check` | `vard check` (`fix: true`) |
| `vard_run` | `vard run --do` |
| `vard_screenshot` | a PNG after actions (expensive in tokens — only to judge design) |
| `vard_test` | `vard test` |
| `vard_doc` | `vard doc` (including package docs) |
| `vard_explain` | `vard explain` |
| `vard_search`, `vard_info` | search and a package card |
| `vard_add` | `vard add` **without** `--allow`: approving a sensitive capability is a person's decision |
| `vard_update` | `vard update` with the API diff |
| `vard_api` | `vard api` |
| `vard_publish_check` | the publish gate without publishing |
| `vard_bindings` | `vard bindings --kotlin` |

The tools are stateless: `vard_run` mounts the app from scratch on every call (50–100 ms), so
nothing can go stale between calls. Publishing through MCP is deliberately absent — it is an
outward-facing action a person takes.

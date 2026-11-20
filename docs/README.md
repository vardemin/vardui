# VardUI documentation

VardUI is a cross-platform UI framework whose primary "developer" is an LLM agent: the
**Vard** language, a fine-grained reactive runtime, a headless platform simulator, tooling
with machine-applicable fixes, and a package system designed for a world where a model adds
dependencies and a person approves them.

These pages are for people. The references for agents are compiled into the `vard` binary
and printed by `vard doc` (sources in
[`knowledge/`](https://github.com/vardemin/vardui/tree/master/knowledge)).

## Start here

| Page | What it covers |
|---|---|
| [Getting started](getting-started.md) | install, first project, the "edit → check → run → test" loop |
| [Working with AI agents](ai-first.md) | how the AI-first approach works: AGENTS.md, skills, MCP, token budget |

## Guide

| Page | What it covers |
|---|---|
| [The Vard language](language.md) | types, declarations, views, the UI tree, expressions, mutability, functions |
| [UI: elements, layout, style](ui.md) | built-in elements, flex/grid, common props, tokens, themes, dark mode |
| [Reactivity and execution model](reactivity.md) | signals/computed/effects, keyed lists, rules for pure rendering |
| [Platform and native modules](platform.md) | lifecycle, back, insets, permissions, storage, push, `extern fn`/`extern event` |
| [Testing and debugging](testing.md) | headless runs, the action language, `.vtest`, `why`, lints, the platform simulator |
| [Tooling: CLI and MCP](tooling.md) | every `vard` command, the MCP server, output formats |
| [Diagnostics](diagnostics.md) | E/W/R/L/P codes, how to read and fix them |

## Packages, registries and delivery

| Page | What it covers |
|---|---|
| [Package system overview](packages/) | the idea and key decisions: a contract computed by the compiler |
| [Using packages](packages/using.md) | `vard search/add/update`, vard.toml, vard.lock, namespaces, the capability policy |
| [Writing a package](packages/authoring.md) | layout, manifest, AGENT.md, the example as proof, migrations, native code, publishing |
| [Registries and hosting](packages/registry.md) | protocol, file formats, `vard registry serve`, CDNs, registries in git, private registries |
| [Delivery to devices](packages/delivery.md) | `vard bundle`, the native fingerprint, Android, over-the-air updates |
| [Security model](packages/security.md) | threats (including LLM-specific ones) and defenses |

## Platforms and architecture

| Page | What it covers |
|---|---|
| [Architecture](architecture.md) | layers, decisions, roadmap |
| [Rendering and animation](rendering.md) | the draw list, the GPU renderer, `animate:`/`enter:` and the frame clock |
| [Android shell](android.md) | build, Gradle properties, native modules |
| [Apple shell](apple.md) | iOS and macOS: Swift package, Metal view, modules |
| [ADRs](adr/) | architecture decision records 0001–0007 |

## Developing the framework

See [CLAUDE.md](https://github.com/vardemin/vardui/blob/master/CLAUDE.md) (rules for agents
working on the framework) and "Developing the framework itself" in the
[README](https://github.com/vardemin/vardui#developing-the-framework-itself).

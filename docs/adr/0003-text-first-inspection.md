# ADR-0003: Text inspection as the primary debugging interface

**Status:** accepted

**Context.** A screenshot costs a model ~450–1600 tokens and requires "vision" to understand
what is where. And exact sizes and the causes of a layout cannot be extracted from a picture.

**Decision.** A deterministic headless runtime and an action language (`tree`, `tap`,
`type`, `why`, `state`, `lint`, `expect`). Mutating actions print a diff of the semantic
tree. Tests (`.vtest`) and MCP use the same language. A bundled font makes layout identical
on every machine.

**Consequences.** + a screen ≈ 100 tokens, an action ≈ 30; reproducible bugs and tests
without an emulator. − the headless render is not pixel-identical to the platform; the final
visual check happens on a device (plan: the same protocol over a WebSocket to the device).

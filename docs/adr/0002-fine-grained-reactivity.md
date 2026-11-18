# ADR-0002: Fine-grained reactivity without a virtual DOM

**Status:** accepted

**Context.** Performance is a key criterion. The "re-render a component and diff the trees"
model (React, Compose recomposition, SwiftUI body) spends work proportional to the size of
the subtree and requires memoization from the author — something models systematically get
wrong.

**Decision.** Signals/computed/effects (like Solid/Leptos): a view body runs once, every
dynamic prop is a separate effect, structural `if`/`for` are effects over fragments, keyed
lists have a value signal per row, and equality cut-offs apply at every level.

**Consequences.** + the cost of an update is proportional to the change, not to the UI size;
the author needs no `memo`/`key` tricks (except `by` in lists, which the compiler checks).
− state cannot be changed while the UI is being computed (R0003) — a deliberate restriction
caught by the runtime.

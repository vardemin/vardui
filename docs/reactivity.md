# Reactivity and execution model

The decision and its alternatives: [ADR-0002](adr/0002-fine-grained-reactivity.md). This
page explains how it works and what rules follow for code.

## The graph

| In code | In the runtime |
|---|---|
| `state x`, `pref x` | Signal — a cell with a list of subscribers |
| `let y = ...`, view parameters | Computed — a lazy cached value; dependencies are collected on read |
| every dynamic prop | Effect — writes one field of one node |
| `if` / `for` in the tree | a structural Effect over a fragment of the tree |
| a view | Scope — owns everything created inside it |

A view body runs **once**, at mount. After that, changing a `state` wakes exactly the
effects that read it: `count += 1` rewrites the text of one `Text` and rebuilds nothing.
No virtual DOM, no component re-rendering, no manual memoization.

## Equality cut-offs

At every level: a signal does not notify for the same value; a prop does not touch the node
for the same value; unchanged props do not mark layout dirty. Static props (literals, enum
values, lambdas) create no effects at all.

## Lists

`for t in todos by t.id` reconciles rows by key. Each row has its own value signal, so
changing one item of a 3000-item list re-runs the effects of one row (measured in the
repository: 7 actions over 3000 rows — 35 effect runs). Without `by`, rows are matched by
position: a reorder turns into changes in every row.

## Order and consistency

Effects run in creation order (parent before child), so there are no intermediate
inconsistent states ("glitches"). Disposing a scope (a view left the screen, an `if` branch
switched) frees all of its effects, nodes, signals and event handlers.

## Rules for code

1. **Rendering is pure.** Writing state while computing a prop, a `let` or an `if`/`for`
   condition is `R0003`. Writes go into handlers (`onTap`, `onChange`, `onSubmit`,
   `on event`) and functions called from them.
2. **A `let` does not read itself** (`R0004`), directly or through other `let`s.
3. **Values are copied.** `let t = todos[0]; t.title = "x"` does not change the list; change
   the place: `todos[0].title = "x"`.
4. **Keys are unique.** `R0002` — two equal `by` keys.

Runtime errors do not crash the app: the problem becomes an `R` diagnostic with
`file:line`, the value becomes `nil`.

## Measuring

`vard run --do 'tap "Add"; stats'` prints effect runs, layouts and node count — enough to see
wasted work without a profiler.

# The Vard language

Vard is a declarative, statically typed language for user interfaces. Its syntax is close
to Swift/Kotlin/TypeScript (where models have strong priors), but there is **one way to
write each thing**: no modifier chains, no decorator annotations, no alternative syntaxes.
The full reference for agents is `vard doc` (≈ 3k tokens); this page says the same in more
detail, with the reasons.

## Files and projects

- Sources are `*.vard`. All files of the project directory (recursively, except hidden
  directories, `target`, `build`, `node_modules`) form **one namespace**: there are no imports.
- The app's root view is `view App`. Without it, `vard run` mounts the last view declared in
  the project's code.
- Dependencies are declared in `vard.toml`; their public names are visible without imports
  (rules: [Using packages](packages/using.md#namespaces)).
- Statements end at the line break; `;` is error `E0106` with an auto-fix. Line breaks are
  allowed inside `(`/`[` and before continuation lines (starting with `.x`, `&&`, `||`, `?`, `:`).

## Top-level declarations

```
struct Todo { id: Int, title: String, done: Bool = false }   // fields `name: Type [= default]`
enum Filter { all, active, done }                            // values: .all .active .done
const maxItems = 50
fn total(xs: [Int]) -> Int { return xs.sum() }               // no `-> T` = returns nothing
extern fn share(text: String) -> Bool                        // a platform call
extern fn load(id: Int, done: fn(Item?, String?))            // an async call: callback
extern event orderUpdated(order: Order)                      // a platform event
theme { brand: #7C3AED, primary: #7C3AED, font: "Inter" }   // color tokens and the default font
theme dark { brand: #C4B5FD }                                // dark-mode values
view Name(params) { members }
```

Declaration order does not matter: types and functions can refer to each other in any
order. Keywords from other languages (`func`, `function`, `component`, `class`,
`interface`...) are recognized and fixed to the Vard equivalent (`E0110`, with `fix:`).

## Types

| Type | Example | Notes |
|---|---|---|
| `Int` | `42` | 64-bit integer; `Int / Int` truncates |
| `Float` | `3.5` | `Int` converts to `Float` implicitly; back with `toInt()` |
| `String` | `"Count: {n}"` | interpolation `{expr}`; escapes `\n \t \" \\ \{ \}` |
| `Bool` | `true` | |
| `Color` | `#RGB #RRGGBB #RRGGBBAA` | or a token such as `.primary` |
| `[T]` | `[1, 2]` | list |
| `[String: T]` | `["a": 1]`, `[:]` | map; keys are always `String` (`E0125`) |
| `T?` | `nil` | optional |
| `fn(A, B) -> R` | `x => x * 2` | function |
| `struct`/`enum` | `Todo(id: 1, title: "A")`, `.active` | user-defined |

Nothing but `Int → Float` converts implicitly. Strings are built by interpolation:
`"a" + n` is an error fixed to `"a{n}"`. `+` joins only `String + String`.

When a type cannot be inferred (an empty list, `nil`), annotate it:
`state items: [Todo] = []`, `state picked: Todo? = nil` (`E0308`).

## Views

```
view Card(title: String, onTap: fn() = {}, bind value: Bool, content: slot) {
  state count = 0                      // reactive mutable state (the initializer runs once)
  state items: [Todo] = []
  pref darkMode = false                // state kept by the platform; app-wide by name
  let doubled = count * 2              // derived value, recomputes itself, read-only
  fn inc() { count += 1 }              // can read and write state
  on appear { load() }                 // event handler while the view is on screen
  on back if count > 0 { count -= 1 }  // `if` = active only while the condition holds
  Column(gap: 8) {                     // then exactly one UI tree
    Text(title)
    content                            // a bare lowercase name inserts the slot parameter
  }
}
```

Order inside a view: members first (`state`, `pref`, `let`, `fn`, `on`), then elements.
A view body is declarative: there are no statements in the tree (`E0113`); logic lives in
`fn` members and handlers.

### Parameters

| Form | Meaning |
|---|---|
| `name: Type [= default]` | read-only; reactive — updates when the caller's value changes |
| `bind name: Type` | two-way: the caller passes a mutable place (`Checkbox(value: todo.done)`) |
| `name: slot` | a child block: `Card { Text("Hi") }`; at most one slot per view |

The first argument (the first non-slot parameter) may be positional; the rest are always
`name: value` (`E0207`). Every view use also accepts "outer" props applied to its root:
`width height minWidth minHeight maxWidth maxHeight aspectRatio grow alignSelf id opacity
offsetX offsetY scale`.

### State: state, pref, let

- `state` — a signal. Changing it re-runs exactly the props that read it.
- `pref` — the same as state, but the value is kept in platform preferences
  (`SharedPreferences`, `UserDefaults`, headless — the simulator's memory) under the key
  `vard.pref.<name>` and restored at launch. Any data type (structs, lists, maps). All
  `pref`s with one name are one app-wide value, and their types must match (`E0223`).
- `let` — a computed value (lazy, cached, dependencies tracked automatically).

## The UI tree

```
Element(args) { children }      // braces only for elements that take children
if cond { ... } else if c2 { ... } else { ... }
for t in todos by t.id { ... }      // `by` = a stable unique key
for t, i in todos by t.id { ... }   // i = index (Int)
for i in range(0, 5) { ... }
```

`by` is effectively required for structs with an id (`W0401` without it): the key keeps a
row's identity across sorting and filtering. Two equal keys are `R0002`.

Elements are not values: they cannot be stored in a variable or returned from a function
(`E0212`); compose with views and slot parameters.

## Expressions

```
+ - * / %   == != < <= > >=   && || !   cond ? a : b   a ?? fallback   x! (unwrap)
a ?? b ?? c        // `??` is right-associative: optional fallbacks chain
list[i]   map["k"] (T?)   s.field   .name (an enum variant or a color token)
x => x.done   (a, b) => a + b   x => { stmts }   { stmts }   // lambdas
```

`.name` is resolved from the expected type: an argument, a variable's annotation, or the
other side of `==` (`x == .all`). Lambda parameter types come from where the lambda is
passed — no annotations needed.

Not in Vard: `self`/`this`, `?.` (`E0121`), `++`/`--` (`E0115`), `===`, semicolons,
imports, classes, modifier chains (`E0119`), `@State` (`E0124`), `{ x -> }` (`E0126`).
Each of these habits is recognized and comes with a fix.

## Statements (function and handler bodies)

```
let x = 1          var y = 2          x = v   x += v   x -= v
map["k"] = v       map["k"] = nil     // removes the key
if c { } else { }  for x in list { }  return v   print(a, b)
```

Built-in functions: `range(a, b)` (a to b-1), `min`, `max`, `print` (to the log:
`vard run --do log`), and for everyday apps:

| Function | Result |
|---|---|
| `pow(x, y)` `sqrt(x)` `log(x)` `exp(x)` | `Float` (finance math: annuities, compound interest) |
| `clamp(x, lo, hi)` | `Int` when all three are `Int`, else `Float` |
| `now()` | `Int` — milliseconds of local wall-clock time (std `extern fn`, answered by the host) |
| `date(ms)` | `DateParts { year month day hour minute second weekday }` (weekday 1 = Monday … 7 = Sunday) |
| `dateOf(year, month, day, hour: 0, minute: 0)` | `Int` timestamp; out-of-range months and days roll over |
| `addDays(ms, n)` `addMonths(ms, n)` | `Int`; `addMonths` clamps the day (Jan 31 + 1 month = Feb 28) |
| `startOfDay(ms)` `daysBetween(from, to)` | midnight of the day; whole calendar days between two timestamps |
| `formatDate(ms, pattern)` | `String`: `yyyy yy MMMM MMM MM M dd d EEEE EEE HH H mm ss`, `'literal'`; names follow `locale` (en, ru) |

Timestamps already include the device's time zone, so calendar math is plain arithmetic and
is the same on every platform and in headless tests (where `now()` is 2026-01-15 09:30 until
the `clock` action moves it — see [Testing](testing.md)).

## What is mutable

`state`, `pref`, `bind` parameters, local `var`s, fields and elements of those
(`todos[i].done = true`, `profile.name = "x"`, `counts["a"] = 1`) and the loop variable of a
`for` that runs **directly** over state (`for t in todos by t.id { Checkbox(value: t.done) }`).

Parameters, `let` and items of a computed list (`for t in todos.filter(...)`) are
read-only (`E0401`); such state is changed by key: `todos.removeWhere(x => x.id == t.id)`.

Values are copied, never shared (copy-on-write inside the runtime): a function that
receives a struct received a copy. The model never has to think about aliasing.

## Methods and properties

| Type | Properties | Methods |
|---|---|---|
| `String` | `count isEmpty` | `trim() upper() lower() contains(s) startsWith(s) endsWith(s) split(sep) toInt() toFloat() replace(a, b) padStart(n, fill)` |
| `Int` | | `toFloat() toString() abs() format()` |
| `Float` | | `toInt() round() floor() ceil() abs() fixed(digits) format(digits)` |
| `[T]` | `count isEmpty first last` | `contains indexOf map filter find any all sum join sortedBy reversed take(n) distinct() max() min()`; in place: `append insert removeAt remove removeWhere clear` |
| `[String: T]` | `count isEmpty keys values` | `contains(key)`; in place: `remove(key) clear()` |

Properties are read without `()`, methods are called with `()` (`E0215` with a fix).

`x.format(2)` is `fixed` plus digit grouping for the user's `locale`: `1,234.50` (en),
`1 234,50` (ru, fr, …; the separator is a no-break space), `1.234,50` (de, es, …).
`list.max()` / `min()` return `T?` (nil for an empty list): `values.max() ?? 1`.

## Themes and tokens

Colors in views are set through tokens only — then dark mode just works:
`.background .surface .text .muted .primary .onPrimary .border .danger .success .warning .clear`.
Custom tokens are declared in `theme { }` (light values) and `theme dark { }`.
`font: "Inter"` in a theme sets the default family (font files go into the project's `fonts/`).
The app's own `theme { }` wins over a package's: override a single token such as `success`
without forking the package.
Details: [UI](ui.md#tokens-and-themes).

## Localized text

```
strings { hello: "Hello, {name}", items: "{n} items" }      // the default table: the source language
strings ru { hello: "Привет, {name}", items: "{n} шт." }    // a translation
view Greeting(name: String, n: Int) {
  Column {
    Text(t("hello", ["name": name]))
    Text(t("items", ["n": "{n}"]))
  }
}
```

- `t(key, [placeholder: value])` returns the text for the current `locale` (a reactive
  global, e.g. `"ru"`, `"pt-BR"`): the exact language table, else its base language
  (`pt-BR` → `pt`), else the default table. Changing the locale re-renders exactly the texts
  that use `t()`.
- The compiler checks literal keys against the default table (`E0240`, with a fix for typos),
  placeholders against the default text (`E0241`), and reports missing translations
  (`W0405`). Placeholders are names (`{n}`), not expressions (`E0242`) — compute values in code.
- The host sets the locale from the system (Android: the configuration's first locale, also on
  change); in tests: the `locale ru` action.

## The platform boundary

`extern fn` is a call into the platform, `extern event` is an event from the platform with a
handler `on name(params) [if cond] { }`. Only data crosses the boundary (`Int Float String Bool
Color`, structs, enums, lists, maps, optionals) and callbacks `fn(T)` for results that arrive
later (`E0317`). Details: [Platform](platform.md).

## Qualified names

A name from a package can be qualified with the package name: `charts.BarChart(...)`,
`state s: ui.Style`, `geo.distance(a, b)`, and a std one with `std.Button`. This is needed
only when two packages conflict (`E0231`); otherwise names are written as they are. Names
starting with `_` are private to their package.

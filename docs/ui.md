# UI: elements, layout, style

The unit is dp. Every element with its prop types: `vard doc Column`, `vard doc Text`; common
props: `vard doc common`. All of it is generated from one registry
(`crates/vard-lang/src/registry.rs`), so the docs cannot drift from the compiler.

## Built-in elements

| Element | Props | Meaning |
|---|---|---|
| `Column` / `Row` | `gap align justify` (+ `wrap` on Row) | flex containers; `Row(wrap: true)` flows children onto new lines |
| `Grid` | `columns gap align` | equal columns, filled row by row |
| `Stack` | `align justify` | children overlap; z-order = source order; a `grow` child fills the height |
| `Scroll` | `gap align` | a vertically scrolling column; needs a bounded height (`grow: 1`) |
| `Text(text)` | `style size weight color textAlign maxLines strike underline italic font letterSpacing lineHeight` | text |
| `Input` | `value` (bind String) `placeholder onSubmit onChange secure keyboard size weight font color placeholderColor lines` | text field; on devices a native editor over the frame. `lines: 5` makes a multi-line text area 5 lines tall: text wraps, Enter inserts a newline (Shift+Enter submits on desktop and web) |
| `Image(src)` | `fit tint` | a project file (`"assets/logo.png"`) or an https URL |
| `Spacer` | | flexible empty space (`grow: 1` by default) |

Std views (written in Vard itself, source: `vard doc std`):
`Button(label, onTap, kind: .filled|.tonal|.outline|.text, enabled)`,
`Checkbox(bind value, label)`, `Switch(bind value, label)`, `Card { }`, `Divider()`.
A design system is plain views: copy std's pattern (tokens + a `kind` parameter).

## Layout

The model is CSS flex semantics in a single pass: constraints go down, sizes go up.

- `align` — the cross axis: `.start .center .end .stretch` (default `.stretch`).
- `justify` — the main axis: `.start` (default) `.center .end .between .around .evenly`.
- `grow` — a share of the free space on the main axis; `alignSelf` overrides `align` for a child.
- Sizes: `width height minWidth minHeight maxWidth maxHeight aspectRatio`.
- Spacing: `padding paddingX paddingY paddingTop paddingBottom paddingLeft paddingRight`;
  `safeArea: .top|.bottom|.all|.vertical|.horizontal` adds the system insets to padding.

Every node remembers **why** it got its size on each axis. `vard run --do 'why @2'` prints
the chain of rules up to the root — the main tool for "why doesn't it fit":

```text
> why @2
Text @2 358x32 at 16,16  examples/todo/app.vard:27
    width: stretched by parent Column (align: .stretch)
    height: content (1 line(s) x 32dp)
in App @1 390x844 at 0,0  examples/todo/app.vard:26
    width: stretched by parent Column (align: .stretch)
    height: grow: 1 share of parent's free space
```

## Common props

Every element accepts: sizes and `grow`, padding, `background`, `gradient` (`[Color]`) and
`gradientAngle` (CSS degrees, 180 = down), `radius` and per-corner radii
(`radiusTopLeft`...), `border` and per-side borders, `borderColor`, `shadow`
(`.sm .md .lg .xl .xxl`), `shadowColor`, `opacity`, `offsetX/offsetY`, `scale`, `clip`,
`alignSelf`, `onTap`, `id`, and semantics: `role` (`.button .checkbox .switch .heading
.link .image`), `label`, `checked`, `enabled`.

Motion: `animate: .spring|.bouncy|.easeOut|.easeInOut|.easeIn|.linear` interpolates changes
of opacity, offsets, scale, colors, radius, border and width/height over `duration:` ms;
`enter: .fade|.slideUp|.slideDown|.slideLeft|.slideRight|.scale|.pop` plays when the element
appears. Headless, the clock moves with `wait MS` — see [Rendering and animation](rendering.md).

`id: "email"` gives a stable address `#email` for tests and inspection. A non-unique `id` on
screen is lint `L006`.

## Typography

`style`: `.display` 34, `.title` 24, `.heading` 18, `.body` 16 (default), `.label` 15 bold,
`.caption` 13. `weight`: `.thin` … `.black` (100–900). Project fonts go into `fonts/`
(`Inter-SemiBold.ttf` → family `Inter`, weight 600); select with `font: "Inter"` on an
element or in the theme. Headless layout uses the bundled DejaVu font, so it is identical on
every machine; on devices the platform shaper measures text, and line breaks match what the
layout computed.

## Tokens and themes

```
theme { brand: #7C3AED, primary: #7C3AED, onPrimary: #FFFFFF }
theme dark { brand: #C4B5FD, primary: #A78BFA, onPrimary: #1E1B4B }
```

- Semantic tokens: `.background .surface .text .muted .primary .onPrimary .border
  .danger .success .warning .clear`. Pairings: `.text`/`.muted` on `.background`/`.surface`,
  `.onPrimary` on `.primary`. Contrast below WCAG AA is lint `L005`.
- Custom tokens are declared in a theme and used as `.brand`.
- Raw hex colors in views break dark mode — use tokens.
- Dark theme: the system's, or `setColorScheme(.dark|.light|.system)`; read it with the
  global `isDark`; in tests — `theme dark` or `@dark`.

"Make it like Tailwind / Material / iOS": `vard doc design` has class → prop mapping tables;
the `tailwind` package (`vard add tailwind`) ships the full palette, semantic tokens with
dark values and the components `TwButton TwCard Badge Avatar Field`.

## UI lints

After a script, `vard run` prints lints — real bugs, not style:

| Code | What |
|---|---|
| `L001` | children overflow their container (needs a `Scroll`, or `grow: 1` on the flexible child) |
| `L002` | an interactive element smaller than 44×44 dp |
| `L004` | a container with children has zero width or height |
| `L005` | text contrast below WCAG AA |
| `L006` | a repeated `id` on screen |

The fix recipe: `vard explain L001`.

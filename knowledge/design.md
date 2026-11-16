# Vard design reference (`vard doc design`)

Any visual design is built from the same pieces: **tokens** (`theme { }`), **props** on
built-in elements, and **views** with variant enums (your design system). There are no
class names; translate each CSS/framework class to a prop.

## "Make it look like <framework>" — the recipe
1. Tokens: `vard add tailwind` (full Tailwind palette + semantic tokens) or write a
   `theme { }` / `theme dark { }` with the framework's colors mapped onto Vard's semantic
   tokens (`.background .surface .text .muted .primary .onPrimary .border .danger ...`).
   Use semantic tokens in views (dark mode works); palette tokens only for accents.
2. Components: one `view` per component with an enum `variant`/`size` parameter (see
   `TwButton` in tailwind.vard, `Button` in `vard doc std`).
3. Screens: compose components; map remaining utility classes with the table below.
4. Check: `vard run --do "shot out.png 2"` to look, `lint` for contrast/tap targets,
   `theme dark` + `shot` for dark mode.
5. Fonts: put `Inter-Regular.ttf`, `Inter-SemiBold.ttf`, ... into `fonts/`, then
   `theme { font: "Inter" }` (or `font:` per Text). Weight/italic pick the closest file.

## Tailwind -> Vard (1 Tailwind unit = 4dp)
| Tailwind | Vard |
|---|---|
| `p-4` `px-3` `py-2` `pt-6` | `padding: 16` `paddingX: 12` `paddingY: 8` `paddingTop: 24` |
| `m-*` (margin) | parent `gap`, a `Spacer`, or padding on a wrapper Column |
| `w-24` `h-10` `min-h-11` `max-w-md` `size-10` | `width: 96` `height: 40` `minHeight: 44` `maxWidth: 448` `width: 40, height: 40` |
| `w-full` | default in a Column (`align: .stretch`); in a Row: `grow: 1` |
| `flex flex-col` / `flex-row` | `Column` / `Row` |
| `gap-4` | `gap: 16` |
| `items-center` `justify-between` | `align: .center` `justify: .between` |
| `flex-1` `grow` | `grow: 1` |
| `flex-wrap` | `Row(wrap: true, gap: 8)` |
| `grid grid-cols-3 gap-4` | `Grid(columns: 3, gap: 16)` |
| `aspect-video` `aspect-square` | `aspectRatio: 16/9` `aspectRatio: 1` |
| `overflow-hidden` | `clip: true` (rounded by `radius`) |
| `overflow-y-auto` | `Scroll(grow: 1)` |
| `absolute top-2 right-2` | inside a `Stack`: `alignSelf: .end` + `offsetX: -8, offsetY: 8` |
| `-mt-8`, `translate-x-2` | `offsetY: -32`, `offsetX: 8` (visual only, like CSS transforms) |
| `scale-95` | `scale: 0.95` |
| `rounded` `-md` `-lg` `-xl` `-2xl` `-3xl` `-full` | `radius: 4` `6` `8` `12` `16` `24` `999` |
| `rounded-t-xl` | `radiusTopLeft: 12, radiusTopRight: 12` |
| `border` `border-2` `border-slate-200` | `border: 1` `border: 2` `borderColor: .slate200` |
| `border-b` | `borderBottom: 1` |
| `ring-1 ring-slate-200` | `border: 1, borderColor: .slate200` |
| `shadow-sm` `shadow` `shadow-md` `shadow-lg` `shadow-xl` `shadow-2xl` | `shadow: .sm` `.sm` `.md` `.lg` `.xl` `.xxl` (+ `shadowColor: #6366F166` for colored) |
| `bg-white` `bg-indigo-600` | `background: .card` (semantic) / `background: .indigo600` |
| `bg-gradient-to-r from-indigo-500 to-purple-500` | `gradient: [.indigo500, .purple500], gradientAngle: 90` (`-to-b` 180, `-to-br` 135, `-to-t` 0) |
| `opacity-50` | `opacity: 0.5` |
| `text-xs` `sm` `base` `lg` `xl` `2xl` `3xl` `4xl` | `size: 12, lineHeight: 16` · `14/20` · `16/24` · `18/28` · `20/28` · `24/32` · `30/36` · `36/40` |
| `font-light` `normal` `medium` `semibold` `bold` `extrabold` | `weight: .light` `.regular` `.medium` `.semibold` `.bold` `.extraBold` |
| `tracking-tight` `tracking-wide` `tracking-widest` | `letterSpacing: -0.4` `0.4` `1.6` (em × font size) |
| `leading-6` | `lineHeight: 24` |
| `text-center` `text-right` | `textAlign: .center` `.end` |
| `italic` `underline` `line-through` `uppercase` | `italic: true` `underline: true` `strike: true` `"{s.upper()}"` |
| `truncate` `line-clamp-2` | `maxLines: 1` `maxLines: 2` |
| `text-slate-500` | `color: .muted` (semantic) or `color: .slate500` |
| `hidden` / conditional | `if cond { ... }` |
| `sm:` `md:` breakpoints | `screenWidth >= 640 ? ... : ...` (reactive) |
| `dark:` | `theme dark { }` values for the tokens you use |
| `hover:` | n/a on touch; pressed/selected state comes from your state |

Touch targets: keep the tappable element at least 44×44 even when the visual is smaller —
wrap a 32dp pill in `Row(height: 44, align: .center, onTap: ...)` (see Interests chips in
examples/showcase).

## Material 3 (baseline) tokens
```
theme { primary: #6750A4, onPrimary: #FFFFFF, background: #FEF7FF, surface: #F3EDF7,
        text: #1D1B20, muted: #49454F, border: #79747E, danger: #B3261E }
theme dark { primary: #D0BCFF, onPrimary: #381E72, background: #141218, surface: #211F26,
        text: #E6E0E9, muted: #CAC4D0, border: #938F99, danger: #F2B8B5 }
```
Shapes 4/8/12/16/28 (`radius`), buttons `height: 40, radius: 20`, cards `radius: 12`,
FAB `56×56, radius: 16`. Type: display 57/64, headline 28/36, title 22/28 (`.medium`),
body 16/24, label 14/20 `.medium`. Elevation 1..3 -> `shadow: .sm .md .lg`.

## iOS (HIG) tokens
```
theme { primary: #007AFF, background: #F2F2F7, surface: #FFFFFF, text: #000000,
        muted: #3C3C4399, border: #3C3C434A, danger: #FF3B30, success: #34C759 }
theme dark { primary: #0A84FF, background: #000000, surface: #1C1C1E, text: #FFFFFF,
        muted: #EBEBF599, border: #54545899, danger: #FF453A, success: #30D158 }
```
Large title 34 `.bold`, headline 17 `.semibold`, body 17, footnote 13, caption 12. Grouped
lists: `Column(radius: 10, background: .surface)` rows `height: 44, paddingX: 16,
borderBottom: 0.5`. Buttons: text style in `.primary`, filled `radius: 12, height: 50`.

## Bootstrap 5
`primary #0D6EFD, success #198754, danger #DC3545, warning #FFC107, border #DEE2E6,
muted #6C757D`; radius 6 (`.rounded`), 8 (`-3`); button `paddingX: 12, height: 38` (keep
44 touch target); spacing scale 4 8 16 24 48.

## Checklist for a polished screen
- One accent color; text only in `.text` / `.muted` / `.onPrimary`.
- Consistent radius family (e.g. 8 controls, 12 cards, 999 pills) and spacing (4/8/12/16/24).
- Shadows only on raised surfaces (cards, sheets), `.sm`/`.md` mostly.
- `lint` clean: contrast, 44dp targets, no overflow. Check `theme dark`.
- Edge-to-edge: headers paint under the status bar with `safeArea: .top`; scrolling
  content takes `safeArea: .bottom` (also lifts above the keyboard).

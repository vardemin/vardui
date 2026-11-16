---
name: vard-design
description: Style a Vard app or build its design system — implement a mockup, a brand, or "make it look like Tailwind / Material 3 / iOS / Bootstrap / shadcn"; tokens, dark mode, fonts, shadows, gradients, grids, polish.
---

# Designing with Vard

Reference with the Tailwind -> prop table and M3/iOS/Bootstrap tokens: `vard doc design`.

## Recipe
1. Tokens first. Tailwind: `vard add tailwind` (palette `.slate50`...`.rose950` + semantic
   tokens + dark values + TwButton/TwCard/Badge/Avatar/Field). Other systems: a `theme { }`
   and `theme dark { }` mapping their colors onto `.background .surface .text .muted
   .primary .onPrimary .border .danger .success .warning` (+ your own names).
2. Fonts: files in `fonts/` (`Inter-Regular.ttf`, `Inter-SemiBold.ttf`, ...), then
   `theme { font: "Inter" }`. Headless measuring and device drawing use the same files.
3. Components as views with enum variants; screens use only those views + layout elements.
4. Look at it: `vard run --size 390x844 --do "shot a.png 2"`; then `theme dark; shot b.png`.
   Fix `lint` output (contrast, 44dp targets, overflow).

## Vocabulary (props on any element)
`background` `gradient: [c1, c2] gradientAngle: 135` `radius` `radiusTopLeft...`
`border` `borderBottom` `borderColor` `shadow: .sm|.md|.lg|.xl|.xxl` `shadowColor`
`opacity` `offsetX/offsetY` `scale` `clip` `aspectRatio` `maxHeight` `safeArea`.
Text: `size` `weight: .thin...black` `lineHeight` `letterSpacing` `italic` `underline`
`strike` `maxLines` `font`. Layout: `Row(wrap: true)`, `Grid(columns: 2, gap: 12)`,
`Stack` for overlays, `Spacer`, `grow`.

## Rules that make it look designed
- Semantic tokens in views (`.text`, `.muted`, `.surface`), palette tokens only as accents;
  raw hex only inside `theme`. Every color you use needs a dark value or a semantic token.
- Spacing scale 4/8/12/16/24/32; radius family (8 controls, 12 cards, 999 pills).
- Hierarchy through weight and color (`.semibold` titles, `.muted` secondary), not size alone.
- Shadows `.sm/.md` on cards over `.surface` backgrounds; borders `.border` at 1dp.
- Small visuals keep a 44dp touch target: outer `Row(height: 44, align: .center, onTap:)`
  around the 32dp pill.
- Edge-to-edge hero: `Column(safeArea: .top, gradient: [...])` paints under the status bar.
- Responsive: `Grid(columns: screenWidth >= 600 ? 4 : 2)`.

Worked example: examples/showcase (Tailwind dashboard, tested in tests/showcase.vtest).

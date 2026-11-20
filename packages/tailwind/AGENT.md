# tailwind — Tailwind CSS look for Vard

Adds (1) the full Tailwind palette as color tokens `.slate50` … `.rose950` (same in light and
dark), (2) Vard's semantic tokens (`.background .surface .text .muted .primary .onPrimary
.border .danger .success .warning`) remapped to a Tailwind look (slate neutrals, indigo
accent) with dark values, plus `.card .ring .accent .onAccent`, and (3) components.

Use semantic tokens in views (dark mode keeps working); palette tokens for accents only.
Class → prop mapping (p-4 = padding: 16, rounded-xl = radius: 12, shadow-sm = shadow: .sm,
text-sm = size: 14, font-semibold = weight: .semibold): `vard doc design`.

```
TwCard {
  Text("Plan", size: 14, weight: .semibold)
  Badge("Active", tone: .green)
  Field("Email", value: email, placeholder: "you@example.com")   // keyboard: .number .email ...
  TextArea("Notes", value: body, placeholder: "Write…", lines: 5)  // multi-line Input
  TwButton("Save", onTap: save)                       // variant: .primary .secondary .soft .outline .ghost .danger
  TwButton("Cancel", variant: .ghost, size: .sm)      // size: .sm .md .lg (touch target stays 44dp)
}
Avatar("LA", size: 40, color: .indigo600)
Segmented(options: ["Expense", "Income"], selected: kind)   // bind String; 44dp segments
Chip("Food", selected: cat == "Food", onTap: { cat = "Food" }) // filter pill, 44dp target
Progress(value: spent / budget, color: .rose500)             // 0..1 bar (clamped), height: 8
IconButton("+", label: "Add note", onTap: add, variant: .primary)  // glyph icon, round
```

Headless: nothing to mock (no platform calls). Components carry `role`/`label`, so tests
address them by text: `tap "Save"`, `expect "Active"`.

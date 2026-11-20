# Showcase: Pocket

[`examples/pocket`](https://github.com/vardemin/vardui/tree/master/examples/pocket) is an
everyday all-in-one app built with Vard and the `tailwind` package: **notes**, a **money
tracker**, a **journal** and **financial calculators** behind one tab bar. It is about 900
lines of Vard in eight files, runs natively on Android and iOS (drawn by the GPU renderer,
with native text fields), on desktop and on the web, and every flow is covered by `.vtest`
files that run headless in about a second.

| Tab | What it shows |
|---|---|
| Home | a gradient hero under the status bar, quick actions overlapping it (`offsetY`), budget `Progress`, journal streak, pinned notes, recent transactions |
| Notes | search, pinned and other notes in a responsive `Grid`, color tags, a full-screen editor with a multi-line `Input(lines: 12)`; an untouched new note is discarded |
| Money | month switcher (`addMonths`), balance card, budget, spending by category, a 7-day `BarChart`, transactions grouped by day (`startOfDay` + `distinct()`), a bottom sheet to add income or an expense |
| Journal | one entry a day with a mood, a streak (`daysBetween`), a two-week mood strip, expandable entries, a `TextArea` editor |
| Calc | loan payment (annuity, `pow`), compound savings, time to a goal (`log`), the 50/30/20 budget |

Settings (from the gear on Home) pick the currency (`$ € £ ₽`), the monthly budget and the
appearance (`setColorScheme`). Money is formatted with `x.format(2)`, so a Russian locale
shows `1 234,50 ₽`; dates use `formatDate` with localized month and weekday names.

## Files

| File | Contents |
|---|---|
| `model.vard` | structs (`Note`, `Tx`, `Entry`, `Category`), sample data, pure helpers (`money`, `streak`, `inMonth`, `greeting`) |
| `ui.vard` | the app's design system on top of `tailwind`: `ScreenHeader`, `SheetHeader`, `TabBar`, `BarChart`, `StatTile`, `CategoryIcon`, `EmptyState` |
| `app.vard` | the root: `pref`s for all data, tabs, pages, the bottom sheet over a `Stack`, `on back` |
| `home.vard` `notes.vard` `money.vard` `journal.vard` `calc.vard` | one file per area |

All data lives in `pref`s (`pref notes: [Note] = seedNotes(now())`), so it is kept across
launches in SharedPreferences / UserDefaults and across `restart` in tests.

## Run it

```bash
vard run examples/pocket --do "tap #tab-money; shot money.png 2"   # headless
vard test examples/pocket                                          # 5 test files
cargo run -p vard-desktop -- examples/pocket                       # a window, live reload
```

```bash
cd platforms/android && ./gradlew installDebug -Pvard.app=examples/pocket
platforms/apple/run-ios-sim.sh examples/pocket "iPhone 15 Pro"
platforms/web/build.sh examples/pocket
```

## Tests

`tests/*.vtest` drive the real app: write a multi-line note and find it after a cold start,
add an expense from the sheet and watch the balance and budget change, switch the currency
and the locale, write a journal entry and move the clock a day forward (`clock +1d`,
`restart`) to check the streak, and check every calculator result
(`expect #result "$1,580.17"`).

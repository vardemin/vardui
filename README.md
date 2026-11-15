# VardUI

Кроссплатформенный UI-фреймворк, у которого основной «разработчик» — LLM-агент.
Язык `Vard` + реактивный рантайм + тулинг, где каждый шаг цикла
«написать → проверить → запустить → понять, что не так» стоит минимум токенов и
не требует устройства.

```
view App {
  state count = 0
  Column(padding: 24, gap: 16, align: .center, justify: .center, grow: 1) {
    Text("Tapped {count} times", style: .title)
    Button("Tap me", onTap: { count += 1 })
  }
}
```

```
$ vard run --do 'tap "Tap me"'
> tap "Tap me"
~ Text "Tapped 1 times" @2
```

## Что уже работает (v0.1, этот репозиторий)

| Компонент | Что делает |
|---|---|
| `crates/vard-lang` | лексер, парсер с восстановлением после ошибок, bidirectional type checker, диагностики с машинно-применимыми исправлениями, реестр элементов (единый источник правды для компилятора, рантайма и документации), std-виды на самом Vard |
| `crates/vard-runtime` | fine-grained реактивность (signals / computed / effects, без virtual DOM), keyed-списки, flex-layout с объяснением размеров (`why`), семантическое дерево, диффы, UI-линты (overflow, tap target, WCAG-контраст, дубли id), headless-растеризация в PNG, темы light/dark, `extern fn` с моками |
| `crates/vard-cli` | `vard check [--fix] [--json]`, `run --do`, `test` (`.vtest`), `doc`, `explain`, `new`, `mcp` (MCP-сервер на stdio) |
| `knowledge/` | `llms.txt` (полный справочник языка, ~2k токенов), `AGENTS.md`, скиллы `vard-app` и `vard-debug` — `vard new` кладёт их в каждый проект вместе с `.mcp.json` |
| `examples/` | todo-приложение с тестом |

Нативные оболочки Android/iOS, GPU-рендер, анимации, навигация и async — в
[дорожной карте](docs/architecture.md#дорожная-карта). Ядро (язык, реактивность,
layout, инспекция) спроектировано так, чтобы переехать на устройство без изменений.

## Быстрый старт

```bash
cargo build --release
```

```bash
./target/release/vard new myapp
```

```bash
cd myapp && ../target/release/vard run && ../target/release/vard test
```

Для Claude Code / любых MCP-клиентов: `vard new` создаёт `.mcp.json` с сервером
`vard mcp` (инструменты `vard_check`, `vard_run`, `vard_screenshot`, `vard_test`,
`vard_doc`, `vard_explain`).

## Почему это удобно для LLM — в цифрах (examples/todo)

| Операция | Стоимость |
|---|---|
| весь справочник языка (`vard doc`) | ~7.8 KB ≈ 2k токенов, один раз за сессию |
| экран целиком (`tree`) | ~400 байт ≈ 110 токенов (скриншот того же экрана ≈ 450–1600) |
| результат действия (`tap`) | ~100 байт: только изменившиеся строки дерева |
| `vard check` | ~18 мс; ошибки с готовым `fix:` применяются `--fix` без участия модели |
| headless-запуск + сценарий | ~50–100 мс, детерминированно, без эмулятора |

Подробно о решениях — [docs/architecture.md](docs/architecture.md) и [docs/adr](docs/adr).

## Разработка самого фреймворка

```bash
cargo test --workspace
```

```bash
./target/release/vard test examples/todo
```

Rust 1.98 (`rust-toolchain.toml`). Зависимости рантайма: `slotmap`, `fontdue`,
`tiny-skia`, `dejavu` (встроенный шрифт для детерминированного layout); CLI: `serde_json`.

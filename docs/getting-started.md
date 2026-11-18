# Быстрый старт

## Установка

Нужен Rust 1.98 (версия зафиксирована в `rust-toolchain.toml`, rustup подтянет её сам).

```bash
cargo build --release
```

Бинарник — `target/release/vard`. Добавьте каталог в `PATH` или вызывайте по полному пути.
Всё, что нужно агенту и человеку — компилятор, headless-рантайм, тест-раннер, справочник,
менеджер пакетов, сервер реестра и MCP-сервер, — в этом одном файле.

## Первый проект

```bash
vard new hello
```

```text
hello/
  app.vard              корневой вид App
  vard.toml             манифест: имя приложения, зависимости, политика
  tests/smoke.vtest     сценарий-тест
  AGENTS.md, CLAUDE.md  инструкции для агентов (блок со списком пакетов обновляется сам)
  .mcp.json             MCP-сервер `vard mcp` для Claude Code и других клиентов
  .claude/skills/       скиллы vard-app, vard-debug, vard-native, vard-design, vard-packages
```

`app.vard`:

```
view App {
  state count = 0

  Column(padding: 24, gap: 16, justify: .center, align: .center, grow: 1) {
    Text("Hello, Vard", style: .title)
    Text("Tapped {count} times", color: .muted)
    Button("Tap me", onTap: { count += 1 })
  }
}
```

## Цикл разработки

Один и тот же для человека и для агента; каждый шаг — миллисекунды и детерминирован.

```bash
cd hello
vard check --fix          # типы + машинные исправления
vard run                  # экран текстом (семантическое дерево)
vard run --do 'tap "Tap me"; expect "Tapped 1 times"'
vard test                 # все tests/*.vtest
```

`vard run` печатает экран как дерево с идентификаторами:

```text
> tree
screen 390x844 light
App @1
  Text "Hello, Vard" @2
  Text "Tapped 0 times" @3
  Button "Tap me" @4
```

Мутирующее действие печатает только изменения:

```text
> tap "Tap me"
~ Text "Tapped 1 times" @3
```

Ошибка компиляции — это место, строка кода и исправление:

```text
app.vard:4:5 error E0201: unknown element `VStack`
  4 | VStack(gap: 8) {
  fix: `VStack` -> `Column` (auto: vard check --fix)
```

## Добавить пакет

```bash
vard search tailwind
vard add tailwind
vard doc tailwind          # AGENT.md пакета + сгенерированный компилятором API
```

После `vard add` публичные имена пакета видны без импортов: `TwButton("Save", onTap: save)`.
Подробнее — [Использование пакетов](packages/using.md).

## Запуск на Android

```bash
cd platforms/android
./gradlew installDebug -Pvard.app=examples/todo
```

Нужны Android SDK/NDK и `cargo-ndk` — см. [platforms/android/README.md](../platforms/android/README.md).
Сборка сама упакует приложение и его пакеты (`vard bundle`) и подключит нативные модули
пакетов.

## Дальше

- [Язык Vard](language.md) — полное описание языка.
- [Тестирование и отладка](testing.md) — язык действий и симулятор платформы.
- [Работа с ИИ-агентами](ai-first.md) — как устроить работу агента над проектом.
- Примеры: [`examples/`](../examples) — `todo`, `patterns`, `native`, `showcase`, `firebase`.

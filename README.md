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

## Что уже работает (v0.2, этот репозиторий)

| Компонент | Что делает |
|---|---|
| `crates/vard-lang` | лексер, парсер с восстановлением после ошибок, bidirectional type checker, диагностики с машинно-применимыми исправлениями, реестр элементов (единый источник правды для компилятора, рантайма и документации), std на самом Vard: виды и платформенный API (`extern fn` / `extern event`) |
| `crates/vard-runtime` | fine-grained реактивность (signals / computed / effects, без virtual DOM), keyed-списки, flex/grid/wrap-layout с объяснением размеров (`why`), семантическое дерево, диффы, UI-линты, headless-растеризация в PNG (тени, градиенты, скругления по углам, шрифты проекта), JSON-мост к нативному коду, симулятор платформы (prefs, secure storage, разрешения, пуши, back, insets) |
| `crates/vard-cli` | `vard check [--fix] [--json]`, `run --do`, `test` (`.vtest`), `doc`, `explain`, `new`, `add` (tailwind, firebase), `bindings --kotlin`, `mcp` (MCP-сервер на stdio) |
| `crates/vard-ffi` | `cdylib` для нативных оболочек: C ABI (`vard_*`) и JNI — mount, касания, ввод, тема, insets, predictive back, события, колбэки нативных модулей, платформенное измерение текста, кадр (формат v2) |
| `platforms/android` | библиотека `:vard` (рантайм + `VardActivity` + модули ядра), приложение `:app`, модуль `:vard-firebase` ([как собрать](platforms/android/README.md)) |
| `knowledge/` | `llms.txt` (полный справочник языка), `design.md`, `native.md`, кит `tailwind.vard`, модуль `firebase.vard`, `AGENTS.md`, скиллы `vard-app`, `vard-debug`, `vard-native`, `vard-design` — `vard new` кладёт их в каждый проект вместе с `.mcp.json` |
| `examples/` | `todo`, `patterns`, `native` (разрешения, пуши, prefs, keychain, edge-to-edge, back), `showcase` (дизайн в стиле Tailwind), `firebase` — все с `.vtest` |

### Платформенные возможности

| Возможность | В языке | На Android |
|---|---|---|
| Пуши и их обработка | `on pushReceived(m)`, `on pushOpened(m)`, `getPushToken`, `showNotification` | FCM (`vard-firebase`), уведомления в фоне, открытие по тапу (в т. ч. холодный старт) |
| Разрешения | `permissionStatus`, `requestPermission(.camera, s => ...)`, `.blocked` → `openAppSettings()` | runtime permissions, учёт «больше не спрашивать» |
| Преференсы | `pref theme = "light"` — state, сохраняемый между запусками; `prefGet/prefSet` | SharedPreferences |
| Keychain | `secureGet/secureSet/secureRemove` | AES-256-GCM, ключ в Android Keystore |
| Edge-to-edge | рисуем под системными барами; `safeArea: .top/.bottom/...`, `safeArea.top`, `keyboardHeight` | insets баров, выреза и клавиатуры; фокусный `Input` прокручивается над клавиатурой |
| Predictive back | `on back if stack.count > 1 { ... }`, `backProgress` для анимации | `OnBackAnimationCallback` (API 34+), системная анимация «назад на домашний», если приложение не перехватывает |
| Любой дизайн | тени, градиенты, радиусы по углам, `Grid`, `Row(wrap:)`, веса 100–900, шрифты проекта, letter-spacing, offset/scale, clip | тот же кадр рисуется Canvas'ом; текст меряется платформенным шейпером |
| Нативные SDK | `extern fn` / `extern event`, данные — структуры/enum/списки/map | `vard bindings --kotlin` → типизированный базовый класс; модули-библиотеки регистрируются сами (`dev.vard.module.*`) |

Оболочка iOS, GPU-рендер, анимации и компиляция в байткод — в
[дорожной карте](docs/architecture.md#дорожная-карта).

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
`vard_doc`, `vard_explain`, `vard_add`, `vard_bindings`).

«Сделай в стиле Tailwind»: `vard add tailwind` добавляет палитру, семантические токены с
тёмной темой и компоненты; таблица соответствия классов и пропсов — `vard doc design`.
Firebase: `vard add firebase`, на Android — `-Pvard.firebase=true` и `google-services.json`.

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
for e in todo patterns native showcase firebase; do ./target/release/vard test examples/$e; done
```

Rust 1.98 (`rust-toolchain.toml`). Зависимости рантайма: `slotmap`, `fontdue`,
`tiny-skia`, `dejavu` (встроенный шрифт для детерминированного layout), `serde_json`
(нативный мост); FFI: `jni`.

# Использование пакетов

## Найти и добавить

```bash
vard search bar chart        # по имени, ключевым словам, описанию и экспортам
vard info charts             # перед добавлением: версии, возможности, API, тесты
vard add charts              # точное имя; пишет vard.toml и vard.lock
vard doc charts              # AGENT.md + сгенерированный компилятором API
```

```text
$ vard search chart
charts 1.0.0 [lan] — Bar charts drawn with Vard elements
    caps: none · exports: Badge BarChart · tests 1 ✓
details: vard info NAME · install: vard add NAME

$ vard add charts
added charts 1.0.0 (lan)
  caps: none
  api: 2 views — `vard doc charts`
vard.lock: + charts 1.0.0
vard.lock: + tailwind 1.0.0
next: vard check
```

Варианты:

```bash
vard add charts@1.4                 # минимальная версия 1.4.0 на линии 1.x
vard add auth --registry acme       # из конкретного реестра
vard add ui --path ../ui            # локальный пакет (монорепозиторий)
vard add firebase --allow native    # с одобрением чувствительной возможности
vard remove charts
```

Опечатка в имени не превращается в установку чего-то похожего:

```text
$ vard add chart
error P001: no package `chart` in registry `acme`; did you mean: charts?
  fix: search instead of guessing: vard search chart
```

## vard.toml приложения

```toml
[app]
name = "shop"

[dependencies]
charts = "1.4"                                 # >= 1.4.0, < 2.0.0
legacy = "0.3"                                 # 0.x: линия 0.3.* (как в Cargo)
exact = "=2.1.3"                               # ровно эта версия
auth = { version = "2", registry = "acme" }    # из реестра acme
ui = { path = "../ui" }                        # локальный пакет

[policy]
allow = ["native", "push"]                     # одобренные чувствительные возможности

[registries]
acme = "https://vard.acme.dev"                 # http(s) или каталог
default = "acme"                               # откуда берутся зависимости без registry=
```

Манифест можно править руками: любая команда `vard` (check, run, test, doc) заметит, что
`vard.lock` устарел, и переустановит зависимости, сообщив об этом одной строкой.

Реестры ищутся в порядке: `[registries]` проекта → `~/.vard/config.toml` → `builtin`
(пакеты, встроенные в инструмент). Реестр по умолчанию — `builtin`, если не задан
`default` (или переменная `VARD_REGISTRY`). Каждая зависимость берётся **ровно из одного**
реестра — своего `registry = ...` или реестра по умолчанию, — никогда «из того, где нашлось»:
это исключает атаку подмены зависимости (dependency confusion).

## vard.lock

```text
# vard.lock — written by `vard install`. Commit it; review caps= changes like code.
lock 1
root deps=charts
pkg charts 1.0.0 src=reg:acme sha256=100e077d… api=350f1ca96bc8 caps= deps=tailwind
pkg tailwind 1.0.0 src=builtin sha256=6d7a5aa6… api=386f0117f3f1 caps= deps=
```

Одна строка на пакет, отсортировано — дифф читается как ревью:

- новый `caps=` — пакет получил новую возможность;
- новый `sha256=` при той же версии — подмена содержимого (менеджер выделяет `!`);
- `api=` — короткий хеш публичного API: одинаковый хеш = одинаковый API.

Lock-файл коммитят. Версии он не решает — это делает MVS по `vard.toml`.

## Пространства имён

После `vard add` публичные имена пакета видны в коде приложения без импортов:

```
view App {
  state data: [Int] = [10, 40, 25]
  BarChart(values: data, max: 50)
}
```

| Ситуация | Что происходит |
|---|---|
| имя есть в вашем коде и в пакете | побеждает ваше; пакет внутри себя использует своё |
| имя экспортируют две прямые зависимости | `E0231` — пишите `charts.Badge(...)` или `tailwind.Badge(...)` |
| имя из транзитивной зависимости | `E0232` — добавьте её явно (`vard add NAME`) |
| имя на `_` из чужого пакета | `E0230` — приватное |
| `pkg.Name`, которого нет | `E0235`, при опечатке — с исправлением |
| две зависимости объявляют одно нативное имя | `E0233` |

Квалифицировать можно виды (`ui.Card { }`), типы (`state s: ui.Style`), функции и
конструкторы (`geo.distance(a, b)`, `ui.Style(size: 2)`); `std.Button` — стандартный вид.

## Возможности и политика

| Возможность | Что значит | Чувствительная |
|---|---|---|
| `prefs` | читает/пишет настройки (`pref`, `prefGet`) | |
| `links` | открывает URL в других приложениях | |
| `share` | системное меню «Поделиться» | |
| `deeplinks` | получает диплинки | |
| `secure` | Keychain / Keystore | ✓ |
| `permissions` | запрашивает разрешения | ✓ |
| `push` | токен, уведомления, события пушей | ✓ |
| `network` | грузит удалённые ресурсы (https:// в коде) | ✓ |
| `native` | объявляет `extern fn/event`: исполняет нативный код пакета | ✓ |

Чувствительные возможности требуют одобрения в `[policy] allow`. Без него установка
отклоняется:

```text
error P008: not allowed by [policy]: firebase 1.0.0 needs native (declares extern fn/event: runs native code shipped with the package)
  fix: a person approves this: add to vard.toml `[policy] allow = ["native"]` or rerun with --allow native
```

`--allow` записывает одобрение в `[policy]` — оно попадает в дифф и проходит ревью. MCP-
инструмент `vard_add` флага `--allow` не принимает: агент сообщает пользователю, что нужно
пакету, а решение принимает человек.

## Обновление

```bash
vard update                 # все зависимости — до новейших совместимых версий
vard update charts --major  # через границу major
vard check --fix            # миграции пакетов применяются к вашему коду
vard test
```

```text
$ vard update charts --major
~ charts 1.0.0 -> 2.0.0
  API change: major
  ! ~ view BarChart: `max` removed, `scale` added
vard.lock: ~ charts 1.0.0 -> 2.0.0
next: vard check --fix (applies the packages' migrations), then vard test

$ vard check
app/app.vard:3:32 error E0234: `max` of `BarChart` was renamed to `scale` in charts 2.0.0
  3 | BarChart(values: [10, 50], max: 80)
  fix: `max` -> `scale` (auto: vard check --fix)
```

`vard update` меняет требования в `vard.toml` (это видно в диффе), затем переустанавливает.

## Пакеты в headless-режиме

Внешние функции пакета отвечают значениями из его `[sim]` (например, `authCurrentUser` →
`nil`), остальные — висят в `pending`, пока тест не скажет `resolve`/`send`. `mock` в тесте
переопределяет `[sim]`. Поэтому `vard run` работает сразу после `vard add` — даже для пакета с
нативным SDK.

## Диагностики в коде пакетов

Файлы пакетов показываются как `@name/file.vard:LINE`. Предупреждения пакетов не выводятся
(это забота автора), ошибки — выводятся. `vard check --fix` никогда не пишет в файлы
пакетов: они — входные данные из кэша (`~/.vard/cache/pkgs/`).

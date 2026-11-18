# Инструменты: CLI и MCP

`vard` — единственный инструмент, который нужен и человеку, и агенту. Дисциплина вывода:
каждая команда печатает минимум, достаточный для решения о следующем шаге. Успех — одна
строка; ошибка — `файл:строка`, строка кода и исправление. Код возврата: 0 — успех, 1 — нет.
`PATH` — файл `.vard` или каталог проекта (по умолчанию текущий).

## Разработка

| Команда | Что делает |
|---|---|
| `vard check [PATH] [--fix] [--json]` | типы; `--fix` применяет машинные исправления до неподвижной точки (файлы пакетов не трогает); `--json` — диагностики массивом |
| `vard run [PATH] [--do "…"] [--script FILE] [--view V] [--size WxH] [--dark] [--no-lint]` | headless-запуск и сценарий действий |
| `vard test [PATH]` | все `*.vtest` |
| `vard doc [TOPIC]` | справочник: без темы — полный (+ пакеты проекта); темы `elements common actions std errors design native packages`, имя элемента/enum/вида/пакета |
| `vard explain CODE` | объяснение кода E/W/R/L/P |
| `vard new NAME` | проект с `vard.toml`, тестом, AGENTS.md, скиллами и `.mcp.json` |
| `vard bindings [PATH] --kotlin [--package P] [--class C] [--out FILE]` | типизированный Kotlin для `extern fn/event` проекта (без внешних функций пакетов — у них своя реализация) |

## Пакеты

| Команда | Что делает |
|---|---|
| `vard search WORDS` | поиск по имени, ключевым словам, описанию и экспортируемым именам во всех реестрах проекта |
| `vard info NAME` | версии, возможности с пояснениями, зависимости, полный API, тесты при публикации |
| `vard add NAME[@REQ] [PATH] [--path DIR] [--registry R] [--allow CAPS]` | добавить зависимость (только точные имена), установить, обновить `vard.lock`; `--allow` фиксирует одобрение в `[policy]` |
| `vard remove NAME [PATH]` | удалить зависимость |
| `vard install [PATH] [--allow CAPS]` | разрешить `vard.toml`, скачать и проверить, переписать `vard.lock` |
| `vard update [NAME] [PATH] [--major]` | поднять требования до новейших версий; печатает API-дифф |
| `vard api [PATH\|NAME] [--caps]` | публичный API и возможности (с доказательством: какой вызов требует какой возможности) |
| `vard pack [PATH] [--out FILE]` | детерминированный архив `.vpk` |
| `vard publish [PATH] --registry DIR\|URL\|NAME [--token T] [--dry-run]` | гейт и публикация |
| `vard yank NAME@VER --registry DIR [--undo]` | отозвать версию |
| `vard bundle [PATH] --out DIR\|FILE.vpk` | всё, что нужно устройству, + `bundle.json` |
| `vard registry init DIR [--name N]` | создать реестр (`public/` + `private/`) |
| `vard registry token DIR OWNER` | токен публикации (показывается один раз) |
| `vard registry serve DIR [--addr HOST:PORT]` | HTTP-хостинг реестра с API публикации |
| `vard registry verify DIR\|URL` | проверить подписи, цепочку журнала и все архивы |

Переменные окружения: `VARD_HOME` (кэш, ключи, токены; по умолчанию `~/.vard`),
`VARD_REGISTRY` (реестр по умолчанию для всех проектов), `VARD_TOKEN` (токен публикации).

## MCP-сервер

`vard mcp` — JSON-RPC 2.0 по stdio. `vard new` кладёт в проект `.mcp.json`:

```json
{ "mcpServers": { "vard": { "command": "vard", "args": ["mcp"] } } }
```

| Инструмент | Соответствует |
|---|---|
| `vard_check` | `vard check` (`fix: true`) |
| `vard_run` | `vard run --do` |
| `vard_screenshot` | PNG после действий (дорого в токенах — только для оценки дизайна) |
| `vard_test` | `vard test` |
| `vard_doc` | `vard doc` (включая документацию пакетов) |
| `vard_explain` | `vard explain` |
| `vard_search`, `vard_info` | поиск и карточка пакета |
| `vard_add` | `vard add` **без** `--allow`: одобрение чувствительной возможности — решение человека |
| `vard_update` | `vard update` с API-диффом |
| `vard_api` | `vard api` |
| `vard_publish_check` | гейт публикации без публикации |
| `vard_bindings` | `vard bindings --kotlin` |

Инструменты без состояния: `vard_run` каждый раз монтирует приложение заново (50–100 мс),
поэтому между вызовами нечему «протухать». Публикация через MCP намеренно отсутствует — это
внешнее действие, которое совершает человек.

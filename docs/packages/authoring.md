# Создание пакета

## Структура

```text
charts/
  vard.toml              манифест: [package], зависимости, [sim], [[migrate]], [native.android]
  bars.vard              исходники: все .vard, кроме каталогов ниже
  AGENT.md               для агентов (обязательно, ≤ 6 КБ)
  README.md              для людей (необязательно)
  example/               приложение-доказательство: компилируется и тестируется при публикации
    vard.toml            [dependencies] charts = { path = ".." }
    app.vard
    tests/*.vtest
  skills/NAME/SKILL.md   необязательные скиллы; у потребителя — .claude/skills/charts-NAME/
  native/android/        необязательная Gradle-библиотека с нативной реализацией
  assets/, fonts/        ресурсы
```

Исходниками пакета считаются все `.vard`, кроме каталогов `example/ examples/ native/ skills/
tests/` и скрытых. В архив попадают все файлы, кроме скрытых, `target/`, `build/`, `*.vpk` и
`vard.lock`.

Пример из репозитория: [`packages/tailwind`](../../packages/tailwind) (дизайн-система) и
[`packages/firebase`](../../packages/firebase) (нативный SDK).

## Манифест

```toml
[package]
name = "charts"                 # [a-z][a-z0-9_]*, до 32 символов; это же — квалификатор в коде
version = "1.4.0"
summary = "Bar and line charts drawn with Vard elements"   # одна строка — видна в поиске
keywords = ["chart", "graph", "bar"]
license = "MIT"
repository = "https://example.com/charts"
vard = ">=0.1"                  # версии ядра, с которыми работает пакет
capabilities = []               # обязано совпадать с вычисленным (`vard api --caps`)

[dependencies]
tailwind = "1.0"                # только версии из реестров: path-зависимости не публикуются

[sim]                           # headless-ответы для своих extern fn (JSON в строке)
chartsServerTime = '1700000000'

[[migrate]]                     # переименования → E0234 + fix у потребителей
since = "2.0.0"
from = "BarChart.max"           # параметр вида: View.param
to = "BarChart.scale"

[[migrate]]
since = "2.0.0"
from = "Chart"                  # имя верхнего уровня
to = "BarChart"

[native.android]
module = "dev.example.charts.ChartsModule"   # класс VardModule
gradle = "native/android"                    # Android-библиотека внутри пакета
```

## Публичный API

Публично всё, что объявлено на верхнем уровне и не начинается с `_`: виды, структуры, enum,
функции, `extern fn/event`, константы, токены темы. Посмотреть, что видят потребители:

```bash
vard api charts/            # API + возможности с доказательством
```

```text
view BarChart(values: [Int], max: Int = 100)
view Badge(label: String)
capabilities: none
```

Рекомендации:

- Помощники — с `_`: `view _Bar(...)`, `fn _scale(...)`. Их можно менять без major-версии.
- Экспортируемые имена — отличимые: они живут в пространстве имён приложения. Конфликт
  с другим пакетом не сломает сборку молча (`E0231`), но заставит потребителя квалифицировать.
- Параметры с умолчаниями — добавление такого параметра остаётся minor-изменением.
- Токены темы пакета (`theme { }`) становятся токенами приложения: это и есть способ
  поставлять дизайн-системы (так устроен `tailwind`).

## AGENT.md

То, что агент прочитает, чтобы пользоваться пакетом (`vard doc charts` = AGENT.md +
сгенерированный API + возможности). Бюджет — 6 КБ ≈ 1500 токенов (`P011`). Пишите для модели:

1. одна фраза — что это и когда брать;
2. главные виды/функции — по одному компилируемому примеру;
3. неочевидные правила (что передавать в `bind`, какие токены использовать);
4. как тестировать headless: что отвечает `[sim]`, что нужно `resolve`/`emit`.

Список API не дублируйте — он генерируется и всегда точен.

## Пример-доказательство

`example/` — обычное приложение, зависящее от пакета по пути. Гейт публикации компилирует
его и прогоняет все `.vtest` (тест исполняется против ближайшего `vard.toml`). Это
одновременно документация «как пользоваться» и проверка, что пакет работает с текущим ядром.

```bash
vard test charts/           # тесты пакета и example/
```

## Возможности

Компилятор вычисляет возможности из кода: вызовы `secureGet/secureSet` → `secure`,
`requestPermission` → `permissions`, `getPushToken`, `showNotification`, `on pushReceived` →
`push`, `pref` и `prefGet` → `prefs`, `openUrl` → `links`, `shareText` → `share`,
`on deepLink` → `deeplinks`, строки `https://` → `network`, объявление `extern fn/event` →
`native`. `capabilities` в манифесте обязаны совпадать (`P013` с готовой строкой-исправлением).
Лишние возможности — предупреждение: потребителям пришлось бы одобрять ненужное.

## Нативный код

Пакет может поставлять реализацию своих `extern fn/event`:

1. объявите API на Vard в исходниках пакета;
2. сгенерируйте базовый класс: `vard bindings charts/ --kotlin --package dev.example.charts --class ChartsModuleBase`;
3. положите Android-библиотеку в `native/android/` (`build.gradle.kts`, `AndroidManifest.xml` с
   `<meta-data android:name="dev.vard.module.charts" android:value="dev.example.charts.ChartsModule"/>`,
   Kotlin-реализация);
4. укажите `[native.android]` в манифесте;
5. задайте `[sim]`, чтобы потребители могли запускать приложение headless.

Android-оболочка подключает модуль автоматически: `settings.gradle.kts` спрашивает
`vard android-modules <app>` и включает каждую библиотеку как `:vard-<name>`. Пример —
[`packages/firebase/native/android`](../../packages/firebase/native/android).

## Версии и миграции

Гейт сравнивает API с предыдущей опубликованной версией:

| Изменение | Минимальный шаг |
|---|---|
| удалено имя, параметр, поле, вариант enum | major |
| изменён тип, `bind`, возвращаемое значение, первый (позиционный) параметр | major |
| параметр стал обязательным; добавлен обязательный параметр или поле без умолчания | major |
| добавлено имя, вариант enum, параметр или поле с умолчанием | minor |
| ничего в API | patch |

Для `0.x` major-изменение — это рост minor (`0.3 → 0.4`), как в Cargo.

```text
error P015: API change since 1.0.0 is major: ! ~ view BarChart: `max` removed — the version must be at least 2.0.0
  fix: version = "2.0.0"
```

Каждое переименование сопровождайте `[[migrate]]` — тогда `vard check --fix` у потребителя
перепишет код сам.

## Публикация

```bash
vard publish charts --registry acme --dry-run        # только гейт
vard publish charts --registry acme --token $TOKEN
```

```text
gate: charts 1.0.0
  ok    AGENT.md: 78 B of 6.0 KB
  ok    compiles (1 file(s), 1 dependency)
  ok    capabilities: none
  ok    tests: 1 passed (headless)
  ok    api: 2 views (first version)
  ok    archive: 6 files, 822 B, sha256 100e077ddd80943d
published charts 1.0.0 to acme (log #1)
```

Гейт проверяет: имя (допустимое, не зарезервированное, не в одной правке от чужого),
однострочное описание, отсутствие path-зависимостей, AGENT.md и бюджет, размер архива
(≤ 8 МБ), компиляцию с зависимостями, возможности, `[sim]`, миграции, тесты, неизменяемость
версий и semver. Те же проверки выполняет сервер реестра при приёме по HTTP.

Отозвать версию (серьёзная ошибка, утёкший секрет): `vard yank charts@1.0.1 --registry DIR`.
Существующие lock-файлы продолжат работать, новые разрешения её пропустят.

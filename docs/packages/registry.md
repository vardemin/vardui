# Реестры и хостинг

Реестр Vard — это **статические файлы**. Чтение — обычные `GET`, без серверной логики;
запись (публикация) происходит в одном месте и всегда проходит один и тот же гейт. Поэтому
реестр можно держать где угодно: в каталоге, в бакете S3/R2/GCS за CDN, на GitHub Pages, на
nginx — и раздавать через недоверенные зеркала.

## Создать реестр

```bash
vard registry init ./reg --name acme
vard registry token ./reg alice            # токен публикации, показывается один раз
vard publish ./charts --registry ./reg --token vard_…
```

```text
reg/
  public/                 ← это и есть реестр: раздавайте как статику
    config.json           {"vard_registry": 1, "name": "acme", "key": "<ed25519 public key>"}
    catalog.json          строка на пакет: name latest summary keywords caps exports tests
    index/charts.json     JSON-строки: по одной на версию (+ строки статуса yank)
    blobs/<sha256>.vpk    неизменяемые архивы
    log.txt               журнал публикаций (хеш-цепочка)
    log.head              размер, голова цепочки, подпись ed25519
  private/                ← никогда не публикуйте
    registry.key          ключ подписи
    tokens                sha256(token) owner
```

## Протокол чтения

| Запрос | Ответ | Кэширование |
|---|---|---|
| `GET config.json` | имя, версия протокола, публичный ключ | короткое |
| `GET catalog.json` | каталог для поиска (клиентский поиск — серверу ничего считать не надо) | короткое |
| `GET index/NAME.json` | все версии пакета; 404 — пакета нет | короткое |
| `GET blobs/SHA256.vpk` | архив | вечное (`immutable`): адрес = хеш содержимого |
| `GET log.txt`, `GET log.head` | подписанный журнал | короткое |

Строка индекса:

```json
{"name":"charts","version":"1.0.0","sha256":"100e…","size":822,"vard":">=0.1.0",
 "deps":[{"name":"tailwind","req":"1.0","registry":"builtin"}],
 "caps":[],"android":null,"summary":"Bar charts drawn with Vard elements","keywords":["chart"],
 "api":[{"kind":"view","name":"BarChart","params":[{"name":"values","ty":"[Int]","default":null,"bind":false}]}],
 "doc_bytes":78,"tests":{"passed":1,"failed":0},"tested_with":"0.1.0","owner":"alice","yanked":false}
```

`api` — полный публичный API, вычисленный компилятором при публикации: по нему работают
`vard info`, поиск по экспортам и API-дифф в `vard update` без скачивания архивов. `registry`
у зависимости — где она разрешилась у автора (`builtin`, URL или каталог), поэтому потребитель
получит её из того же места.

Отзыв версии — дописанная строка `{"name":"charts","version":"1.0.1","yanked":true}`: индекс
только дописывается.

## Журнал и подпись

```text
log.txt    1 charts 1.0.0 <sha256 архива> <chain_1>
           2 charts 2.0.0 <sha256 архива> <chain_2>
           chain_n = sha256(chain_{n-1} " " n " " name " " version " " sha)
log.head   2 <chain_2> <ed25519(«vard-log/1 2 <chain_2>»)>
```

Клиент при первом обращении к реестру:

1. закрепляет его ключ (`~/.vard/known_registries`, как `known_hosts` у SSH); смена ключа —
   `P005`;
2. проверяет подпись головы и пересчитывает цепочку;
3. проверяет, что голова, увиденная в прошлый раз, — префикс нового журнала (история не
   переписана) — `P005`;
4. устанавливает только архив, чьи `(name, version, sha256)` есть в журнале, и чей хеш
   совпадает — `P004`.

`vard registry verify DIR|URL` проверяет реестр целиком: подписи, цепочку и каждый архив.

## Хостинг

### Каталог или сетевой диск

`[registries] acme = "/mnt/share/vard-registry"` — клиент читает файлы напрямую. Подходит для
команды и CI.

### `vard registry serve`

```bash
vard registry serve ./reg --addr 0.0.0.0:8470
```

- `GET /…` — статика из `public/`; архивы — с `Cache-Control: immutable`.
- `POST /api/v1/publish` — тело: `.vpk`, заголовок `Authorization: Bearer TOKEN`. Сервер
  распаковывает пакет во временный каталог, прогоняет **тот же гейт**, что и `vard publish`
  (компиляция, тесты, semver, возможности…), и дописывает журнал. Ответ — отчёт гейта.
- `POST /api/v1/yank` — тело `NAME VERSION [undo]`, только владелец.

Первый опубликовавший имя становится его владельцем; чужие токены получают `P018`. Сервер
говорит по HTTP — поставьте его за любой TLS-терминатор. Целостность от TLS не зависит.

### CDN или бакет

Публикуйте локально (`vard publish --registry ./reg`) или в CI, затем синхронизируйте
`public/`:

```bash
aws s3 sync reg/public s3://vard-registry --cache-control "no-cache"
aws s3 cp reg/public/blobs s3://vard-registry/blobs --recursive --cache-control "public, max-age=31536000, immutable"
```

Клиенты: `[registries] acme = "https://registry.acme.dev"`.

### Реестр в git (GitOps)

Храните `public/` в git-репозитории. Публикация = pull request, который CI проверяет командой
`vard publish --dry-run` и после слияния применяет `vard publish --registry` с ключом из
секретов CI; GitHub Pages раздаёт результат. История реестра получает второй независимый
журнал — git.

## Клиентская конфигурация

| Где | Что |
|---|---|
| `vard.toml` `[registries]` | реестры проекта и `default` |
| `~/.vard/config.toml` `[registries]` | реестры пользователя (`default = "acme"`) |
| `VARD_REGISTRY` | реестр по умолчанию для всех проектов (CI) |
| `~/.vard/credentials.toml` `[tokens] acme = "vard_…"` или `VARD_TOKEN` | токены публикации |
| `~/.vard/known_registries` | закреплённые ключи |
| `~/.vard/cache/` | архивы (`blobs/`), распакованные пакеты (`pkgs/`), увиденные головы журналов |

Встроенный реестр `builtin` — пакеты из `packages/` репозитория фреймворка, вшитые в
бинарник. Работает офлайн и не требует подписи: его содержимое — часть исполняемого файла.

## Приватные реестры и смешение

Зависимость берётся ровно из одного реестра: из своего `registry = "..."` или из реестра по
умолчанию. Имя из приватного реестра не может «перехватить» публичный реестр и наоборот.
Для зеркал: зеркалу не нужно доверять — оно раздаёт те же подписанные файлы.

## Масштабирование

Текущий формат рассчитан на тысячи пакетов: клиент читает только индексы нужных пакетов,
каталог — один файл. Для больших реестров (в дорожной карте): Merkle-дерево вместо полного
журнала (доказательства включения в O(log n), как sumdb в Go), шардированный каталог, ротация
ключа с подписью нового ключа старым.

# Тестирование и отладка

Обоснование — [ADR-0003](adr/0003-text-first-inspection.md): экран текстом стоит ~100 токенов
против 450–1600 у скриншота и, в отличие от картинки, содержит точные размеры и причины.

## Headless-запуск

```bash
vard run [PATH] --do "ACTIONS" [--view NAME] [--size 390x844] [--dark]
vard run --script steps.txt
```

Каждый запуск монтирует приложение заново и проигрывает действия — детерминированно, за
50–100 мс, без эмулятора. «Сессия» — это список действий: чтобы продолжить, повторите его
целиком с новым шагом в конце.

## Язык действий

Одно действие на строку или через `;`. Полный список — `vard doc actions`.

| Действие | Что делает |
|---|---|
| `tree` | семантическое дерево с `@id` (по умолчанию) |
| `layout` | все узлы с `[x,y WxH]`, стилем и `file:line` |
| `tap SEL` · `type SEL "text"` · `submit SEL` · `focus SEL` · `scroll SEL DY` | взаимодействие |
| `why SEL` | почему у элемента такой размер и позиция — по цепочке родителей |
| `find SEL` · `state` · `log` · `stats` · `lint` | инспекция |
| `expect SEL` · `expect "text"` · `expect no "text"` | утверждения |
| `expect SEL checked\|unchecked\|enabled\|disabled\|offscreen\|visible` | состояние элемента |
| `theme light\|dark` · `size W H` · `insets T B [L R]` · `keyboard H` | окружение |
| `shot FILE.png [scale]` | скриншот (только для оценки визуального дизайна) |

Селекторы: `@12` (id из `tree`), `#email` (prop `id:`), `"Sign in"` (точная метка, затем слово
целиком, затем подстрока). Мутирующие действия печатают дифф дерева: `+` добавлено,
`-` удалено, `~` изменено.

## Симулятор платформы

Headless-платформа `Sim` воспроизводит всё из [platform.md](platform.md):

```text
permission notifications answer denied      # что ответит диалог (по умолчанию granted)
permission camera granted                   # текущий статус
push received "Title" "Body" {"screen":"inbox"}
push opened "Title" "Body"
emit deepLink "vard://item/7"               # любое extern event, аргументы — JSON
back            back 0.5            back cancel
insets 47 34    keyboard 300        focus #email
restart                                     # холодный старт; prefs/secure/разрешения сохраняются
storage                                     # дамп prefs, secure, разрешений, уведомлений
```

Свои `extern fn`:

```text
mock NAME VALUE         # функция возвращает VALUE, её колбэки получают VALUE
pending                 # какие вызовы ждут ответа
resolve NAME VALUE      # завершить самый старый ожидающий вызов (состояния загрузки!)
send NAME VALUE         # доставить значение слушателю и оставить его ожидающим
```

Колбэк с двумя параметрами получает JSON-массив: `resolve firestoreListNotes [[{"title":"A"}], null]`.
Пакеты задают ответы по умолчанию в `[sim]` своего манифеста (например, `authCurrentUser`
отвечает `nil`); `mock` в тесте их переопределяет.

## Файлы .vtest

```text
@view LoginScreen       # необязательные заголовки
@size 390x844
@dark
@app ../other-project
type #email "a@b.c"
tap "Sign in"
expect "Welcome"
```

`vard test [PATH]` находит все `*.vtest`. Тест исполняется против ближайшего каталога с
`vard.toml` (так тесты в `example/` пакета проверяют пример), иначе — против указанного проекта.
При падении печатается только упавший шаг (и экран в этот момент):

```text
PASS tests/smoke.vtest (52ms)
FAIL tests/x.vtest
  > expect "Welcome"
  FAIL: "Welcome" not found
  screen 390x844 light
  App @1
    Text "Hello, Vard" @2
    ...
1 passed, 1 failed
```

## Порядок отладки

1. `vard check` → ошибки с исправлениями (`--fix` применяет их до неподвижной точки).
2. `vard run --do tree` → экран текстом.
3. Действия печатают только изменения.
4. `why`/`layout` дают числа и правило, создавшее размер.
5. `lint:`-строки и `! Rxxxx` — реальные ошибки, исправлять первыми.
6. Скриншот — только чтобы оценить внешний вид.

# Интерфейс: элементы, раскладка, стиль

Единица измерения — dp. Описание каждого элемента с типами props: `vard doc Column`,
`vard doc Text`, общие props — `vard doc common`; всё генерируется из единого реестра
(`crates/vard-lang/src/registry.rs`), поэтому документация не расходится с компилятором.

## Встроенные элементы

| Элемент | Props | Смысл |
|---|---|---|
| `Column` / `Row` | `gap align justify` (+ `wrap` у Row) | flex-контейнеры; `Row(wrap: true)` переносит детей на новые строки |
| `Grid` | `columns gap align` | равные колонки, заполнение по строкам |
| `Stack` | `align justify` | дети накладываются; z-порядок = порядок в коде; ребёнок с `grow` заполняет высоту |
| `Scroll` | `gap align` | вертикально прокручиваемая колонка; нужна ограниченная высота (`grow: 1`) |
| `Text(text)` | `style size weight color textAlign maxLines strike underline italic font letterSpacing lineHeight` | текст |
| `Input` | `value` (bind String) `placeholder onSubmit onChange secure keyboard size weight font color placeholderColor` | поле ввода; на устройстве — нативный редактор поверх кадра |
| `Image(src)` | `fit tint` | файл проекта (`"assets/logo.png"`) или https-URL |
| `Spacer` | | гибкое пустое место (`grow: 1` по умолчанию) |

Стандартные виды (написаны на самом Vard, исходник — `vard doc std`):
`Button(label, onTap, kind: .filled|.tonal|.outline|.text, enabled)`,
`Checkbox(bind value, label)`, `Switch(bind value, label)`, `Card { }`, `Divider()`.
Дизайн-система — это обычные виды: скопируйте паттерн std (токены + параметр `kind`).

## Раскладка

Модель — семантика CSS flex за один проход: ограничения вниз, размеры вверх.

- `align` — поперечная ось: `.start .center .end .stretch` (по умолчанию `.stretch`).
- `justify` — главная ось: `.start` (по умолчанию) `.center .end .between .around .evenly`.
- `grow` — доля свободного места по главной оси; `alignSelf` — переопределить `align` для ребёнка.
- Размеры: `width height minWidth minHeight maxWidth maxHeight aspectRatio`.
- Отступы: `padding paddingX paddingY paddingTop paddingBottom paddingLeft paddingRight`;
  `safeArea: .top|.bottom|.all|.vertical|.horizontal` добавляет системные отступы к padding.

Каждый узел помнит, **почему** получил свой размер по каждой оси. `vard run --do 'why #card'`
печатает цепочку правил до корня — это главный инструмент для «почему не помещается».

```text
> why @2
Text @2 358x32 at 16,16  examples/todo/app.vard:27
    width: stretched by parent Column (align: .stretch)
    height: content (1 line(s) x 32dp)
in App @1 390x844 at 0,0  examples/todo/app.vard:26
    width: stretched by parent Column (align: .stretch)
    height: grow: 1 share of parent's free space
```

## Общие props

Все элементы принимают: размеры и `grow`, отступы, `background`, `gradient` (`[Color]`) и
`gradientAngle` (градусы CSS, 180 = вниз), `radius` и радиусы по углам
(`radiusTopLeft`...), `border` и границы по сторонам, `borderColor`, `shadow`
(`.sm .md .lg .xl .xxl`), `shadowColor`, `opacity`, `offsetX/offsetY`, `scale`, `clip`,
`alignSelf`, `onTap`, `id`, а также семантику: `role` (`.button .checkbox .switch .heading
.link .image`), `label`, `checked`, `enabled`.

`id: "email"` даёт стабильный адрес `#email` для тестов и инспекции. Неуникальный `id` на
экране — линт `L006`.

## Типографика

`style`: `.display` 34, `.title` 24, `.heading` 18, `.body` 16 (по умолчанию), `.label` 15 bold,
`.caption` 13. `weight`: `.thin` … `.black` (100–900). Шрифты проекта кладутся в `fonts/`
(`Inter-SemiBold.ttf` → семейство `Inter`, вес 600); выбор — `font: "Inter"` у элемента или в теме.
Headless-раскладка использует встроенный шрифт DejaVu, поэтому одинакова на любой машине; на
устройстве текст меряет платформенный шейпер, и переносы совпадают с тем, что посчитал layout.

## Токены и темы

```
theme { brand: #7C3AED, primary: #7C3AED, onPrimary: #FFFFFF }
theme dark { brand: #C4B5FD, primary: #A78BFA, onPrimary: #1E1B4B }
```

- Семантические токены: `.background .surface .text .muted .primary .onPrimary .border
  .danger .success .warning .clear`. Сочетания: `.text`/`.muted` на `.background`/`.surface`,
  `.onPrimary` на `.primary`. Нарушение контраста WCAG AA — линт `L005`.
- Свои токены объявляются в теме и используются как `.brand`.
- Сырые hex-цвета в видах ломают тёмный режим — используйте токены.
- Тёмная тема: системная, либо `setColorScheme(.dark|.light|.system)`; читать — глобал `isDark`;
  в тестах — `theme dark` или `@dark`.

«Сделай как Tailwind / Material / iOS»: `vard doc design` — таблицы соответствия классов и
props; готовый пакет `tailwind` (`vard add tailwind`) — полная палитра, семантические токены с
тёмными значениями и компоненты `TwButton TwCard Badge Avatar Field`.

## Линты интерфейса

`vard run` после сценария печатает линты — это реальные ошибки, а не стиль:

| Код | Что |
|---|---|
| `L001` | дети выходят за контейнер (нужен `Scroll` или `grow: 1` у гибкого ребёнка) |
| `L002` | интерактивный элемент меньше 44×44 dp |
| `L004` | контейнер с детьми имеет нулевую ширину/высоту |
| `L005` | контраст текста ниже WCAG AA |
| `L006` | повторяющийся `id` на экране |

Рецепт исправления — `vard explain L001`.

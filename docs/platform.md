# Платформа и нативные модули

Всё платформенное проходит через две декларации — они же граница между Vard и нативным кодом:

- `extern fn name(params) -> T` — приложение вызывает платформу;
- `extern event name(params)` — платформа сообщает приложению; обработчик в виде:
  `on name(params) [if cond] { ... }` активен, пока вид на экране.

Стандартная платформа объявлена на самом Vard (`vard doc std`); каждую функцию реализуют
headless-симулятор и Android-хост (iOS — в дорожной карте). Справочник для агентов —
`vard doc native`.

## Жизненный цикл и система

```
on appear { load() }                  // этот экземпляр вида только что показан
on appResume { refresh() }    on appPause { save() }
on deepLink(url) { route(url) }       // vard://...
```

Глобалы (реактивные, только чтение): `safeArea` (`top bottom left right`), `keyboardHeight`,
`backProgress`, `isDark`, `screenWidth`, `screenHeight`, `platform`.
Функции: `setColorScheme(.dark|.light|.system)`, `openUrl(url)`, `shareText(text)`,
`haptic(.success)`, `openAppSettings()`.

## Назад и predictive back

```
on back if stack.count > 1 { stack.removeAt(stack.count - 1) }
Column(offsetX: backProgress * 64, opacity: 1 - backProgress * 0.3) { ... }
```

Жест забирает самый внутренний активный обработчик. Если активных нет, назад обрабатывает
система (на Android 14+ — системная анимация «назад на домашний экран»). Флаг «перехватывает ли
приложение back» передаётся хосту в каждом кадре.

## Edge-to-edge

Корень всегда рисуется под системными панелями. Если код нигде не упоминает `safeArea`,
рантайм сам добавляет отступы корню. Иначе раскладкой управляете вы: `safeArea: .top` у
элемента добавляет отступы к его padding, `safeArea.bottom` читает значение (с клавиатурой).
Поле ввода в фокусе прокручивается над клавиатурой автоматически.

## Хранилища

```
pref onboarded = false            // настройки, общие для приложения по имени
let t = secureGet("token")        // String? — Keychain / Android Keystore (AES-256-GCM)
secureSet("token", value)         // -> Bool;  secureRemove("token")
prefGet(key) / prefSet(key, v) / prefRemove(key)   // сырые строковые настройки
```

Секреты — только в `secure*`, никогда в `pref`.

## Разрешения

```
state cam: PermissionStatus = .notDetermined
on appear { cam = permissionStatus(.camera) }
Button("Allow camera", onTap: { requestPermission(.camera, s => { cam = s }) })
if cam == .blocked { Button("Open settings", onTap: { openAppSettings() }) }
```

`Permission`: `.notifications .camera .microphone .location .photos .contacts`.
`PermissionStatus`: `.notDetermined .granted .denied .blocked` (`blocked` — система больше не
покажет диалог). На Android разрешение должно быть в манифесте; отсутствующее отвечает `denied`
и пишет в logcat, что добавить.

## Уведомления и пуши

```
on pushReceived(m) { inbox.insert(0, m) }            // пришло, пока приложение открыто
on pushOpened(m) { route(m.data["screen"] ?? "") }   // тап по уведомлению (в т. ч. холодный старт)
on pushTokenChanged(t) { sendTokenToServer(t) }
getPushToken(t => { token = t ?? "" })
showNotification("Title", "Body", ["screen": "inbox"])
```

Провайдер пушей на Android — FCM из пакета `firebase`.

## Свои нативные функции

1. Объявить API на Vard; данные — структуры и enum:
   ```
   struct Place { name: String, lat: Float, lng: Float }
   extern fn searchPlaces(query: String, done: fn([Place], String?))
   extern event locationChanged(lat: Float, lng: Float)
   ```
2. Построить интерфейс и протестировать headless (`mock`, `resolve`, `emit` — [testing.md](testing.md)).
3. Сгенерировать типизированный Kotlin: `vard bindings --kotlin --class PlacesModule --out ...`
   и реализовать абстрактные методы (колбэки можно вызывать с любого потока; `send()` вместо
   `resolve()` — для слушателей).
4. Зарегистрировать: `MainActivity.modules()`, мета-данные `dev.vard.module.<name>` в манифесте
   библиотеки — или, лучше, **оформить пакетом** с `[native.android]`: тогда любое приложение
   получит его одной командой `vard add`, а Android-оболочка подключит модуль сама
   ([packages/authoring.md](packages/authoring.md#нативный-код)).

### Как ходят данные

Формат — JSON. Кодирование нетипизированное, декодирование — по типу, *объявленному в Vard*
(поля структур по имени, enum по имени варианта, умолчания для отсутствующих полей). Поэтому
одна нативная функция обслуживает любые типы приложения: `firestoreGet*` из пакета `firebase`
возвращает документ, а Vard превращает его в вашу структуру.

Функции в аргументах становятся `{"$callback": id}`; хост отвечает `callback(id, json, keep)`.
Колбэк вида, который ушёл с экрана, тихо отбрасывается. Нереализованная на устройстве функция —
`R0010`.

Имена `extern fn`/`extern event` глобальны для приложения: платформа вызывает их по имени.
Два пакета не могут объявить одно нативное имя (`E0233`).

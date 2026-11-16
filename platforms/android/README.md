# VardUI на Android

То же Rust-ядро (`vard-runtime`), что и в `vard run`, собранное как `libvard_ffi.so`, плюс
Kotlin-хост, который рисует результат и подключает платформу.

```
 platforms/android
 ├── vard/            библиотека: libvard_ffi.so + VardView, VardHost, VardActivity, модули ядра
 ├── app/             приложение-оболочка: бандлит Vard-проект (vard.app) и наследует VardActivity
 └── vard-firebase/   модуль-библиотека: Firebase Auth / Firestore / Analytics / FCM (-Pvard.firebase=true)
```

```
 assets/app/**.vard, fonts/, assets/ ──► VardNative.create ──► Rust: compile → mount → layout
                                                     │  frame(): записи отрисовки (формат v2)
 VardView.onDraw  ◄──────────── ByteArray ───────────┘  rect, фон, градиент, тень, радиусы, текст, ...
 касание ─► VardNative.tap(x, y)          hit-test и всплытие к onTap — в ядре
 EditText ─► inputText / submit / focus   настоящий EditText поверх каждого Input, IME работает
 extern fn ─► VardHost.call(name, json)  ─► VardModule (Kotlin) ─► callback(id, json) / emit(event)
 insets / back-жест / onResume / интенты ─► setInsets / backProgress / back / emit(...)
```

## Что делает `VardActivity`

| Возможность | Как устроено |
|---|---|
| Edge-to-edge | окно рисуется под статус-баром, навигацией и вырезом на всех API 26+; insets баров, выреза и клавиатуры уходят в ядро (`safeArea`), иконки баров следуют теме приложения; фокусный `Input` прокручивается над клавиатурой |
| Predictive back | `android:enableOnBackInvokedCallback="true"`; пока в приложении есть включённый `on back`, регистрируется `OnBackAnimationCallback` (API 34+) и прогресс жеста идёт в `backProgress`; иначе система показывает свою анимацию «назад на домашний экран». API 33 — `OnBackInvokedCallback`, ниже — `onBackPressed` |
| Разрешения | `PermissionsModule`: `requestPermissions` + `shouldShowRequestPermissionRationale` → `granted`/`denied`/`blocked`; отсутствующее в манифесте разрешение — `denied` и подсказка в logcat |
| Преференсы | `PrefsModule` на SharedPreferences; ими же сохраняются `pref`-переменные |
| Keychain | `SecureModule`: AES-256-GCM, ключ не покидает Android Keystore |
| Уведомления и пуши | `VardPush`: в foreground — событие `pushReceived`, в фоне — системное уведомление; тап открывает приложение и шлёт `pushOpened` (и при холодном старте). Токен и доставку даёт провайдер (`vard-firebase`) |
| Жизненный цикл, диплинки | `appResume`/`appPause`, `deepLink(url)` для `vard://...` |
| Модули | ядро + все библиотеки с `<meta-data android:name="dev.vard.module.<имя>" android:value="<класс>"/>` + `modules()` приложения |
| Шрифты и картинки | `fonts/*.ttf` проекта регистрируются в ядре и рисуются тем же файлом; `Image("assets/x.png")` из assets, `https://` — загрузка в фоне |

## Что нужно

| Компонент | Версия | Где взять |
|---|---|---|
| Rust | из `rust-toolchain.toml` | `rustup target add aarch64-linux-android x86_64-linux-android` |
| cargo-ndk | 4.x | `cargo install cargo-ndk --locked` |
| Android SDK | platform 35, build-tools 35 | Android Studio → SDK Manager |
| NDK | 28.2.13676358 (`ndkVersion` в `vard/build.gradle.kts`) | SDK Manager → SDK Tools → NDK (Side by side) |
| JDK | 17–21 | например, JBR из Android Studio |

Gradle-обёртка (Gradle 9.3.1, AGP 9.1.0) лежит в репозитории. Путь к SDK — в
`platforms/android/local.properties` (не коммитится): `sdk.dir=C\:\\Users\\<you>\\AppData\\Local\\Android\\Sdk`.

## Сборка и запуск

Из `platforms/android`:

```bash
./gradlew installDebug -Pvard.app=examples/native
```

```bash
adb shell am start -n dev.vard.host/dev.vard.app.MainActivity
```

| Свойство | По умолчанию | Смысл |
|---|---|---|
| `vard.app` | `examples/todo` | какой Vard-проект упаковать (путь от корня репозитория) |
| `vard.abis` | `arm64-v8a,x86_64` | ABI для `cargo ndk -t`; для эмулятора хватит `-Pvard.abis=x86_64` |
| `vard.skipCargo` | `false` | не вызывать cargo, взять готовые `.so` из `vard/src/main/jniLibs/<abi>/` |
| `vard.firebase` | `false` | подключить `:vard-firebase` (нужен `app/google-services.json`) |
| `vard.applicationId` | `dev.vard.host` | applicationId приложения |

Ошибки компиляции Vard показываются на экране; ошибки рантайма (`R…`) и подсказки модулей
пишутся в logcat:

```bash
adb logcat -s Vard VardFirebase
```

## Проверка платформенных функций на эмуляторе

Пуш через тот же путь, что и FCM (только debug-сборки, `DebugPushReceiver`):

```bash
adb shell am broadcast -a dev.vard.DEBUG_PUSH -p dev.vard.host --es title "Hi" --es body "Text" --es data '{"route":"inbox"}'
```

Predictive back на API 34 включается опцией разработчика (на 35+ — по умолчанию):

```bash
adb shell settings put global enable_back_animation 1
```

Диплинк:

```bash
adb shell am start -a android.intent.action.VIEW -d "vard://item/7" dev.vard.host
```

## Свой нативный модуль (любой SDK)

1. Объявить API в `.vard`: `extern fn searchPlaces(query: String, done: fn([Place], String?))`,
   `extern event locationChanged(lat: Float, lng: Float)`.
2. Сгенерировать типизированный базовый класс:
   `vard bindings examples/myapp --kotlin --package com.example --class PlacesModule --out app/src/main/kotlin/PlacesModule.kt`.
3. Реализовать абстрактные методы (колбэки можно вызывать с любого потока) и вернуть модуль из
   `MainActivity.modules()` — или вынести в библиотеку с `<meta-data android:name="dev.vard.module.places" .../>`
   (так устроен `vard-firebase`).

## Firebase

```bash
vard add firebase examples/myapp
```

Положить `google-services.json` в `platforms/android/app/` и собрать с `-Pvard.firebase=true`.
Без файла модуль стартует, но все вызовы отвечают `FirebaseError(code: "not-configured")`.
Firestore работает с любыми структурами приложения: объявите `firestoreGet<X>`/`firestoreList<X>`/...
со своим типом — нативная часть одна на все.

## Ограничения

- Отрисовка через `Canvas` целым кадром; GPU-рендер и диффы дерева — дорожная карта (`docs/architecture.md`).
- Тени через `Paint.setShadowLayer` (аппаратно — API 28+).
- Доступность (TalkBack) ещё не подключена к семантическому дереву; прокрутка без инерции.
- iOS-оболочки пока нет: C ABI (`vard_set_host`, `vard_callback`, `vard_emit`, `vard_back*`, `vard_set_insets`) для неё готов.

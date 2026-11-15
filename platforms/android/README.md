# VardUI на Android

Первая нативная оболочка: то же Rust-ядро (`vard-runtime`), что и в `vard run`, собранное как
`libvard_ffi.so`, плюс минимальное Kotlin-приложение, которое рисует результат.

```
 assets/app/*.vard ──► VardNative.create ──► Rust: compile → mount → layout
                                               │  frame(): плоский список записей в порядке отрисовки
 VardView.onDraw  ◄──────── ByteArray ─────────┘  (rect, фон, радиус, рамка, строки текста, цвета, opacity)
 касание ─► VardNative.tap(x, y)      — hit-test и всплытие к onTap делает ядро
 EditText ─► VardNative.inputText/submit — настоящий EditText поверх каждого Input, IME работает
```

- Ядро считает layout и перенос строк (эталонный шрифт DejaVu, как в `vard run`), поэтому
  прямоугольники на устройстве совпадают с `vard run --do layout`. Android рисует текст своим
  `Paint` (Roboto) теми же размерами в dp; выравнивание `center`/`end` и зачёркивание
  пересчитываются по ширине платформенного шрифта.
- Формат кадра описан в `crates/vard-ffi/src/frame.rs`; C ABI (`vard_*`) там же, в `lib.rs`, —
  его же будет использовать iOS-оболочка.
- Тема следует системной (светлая/тёмная), поворот экрана и клавиатура не пересоздают Activity —
  состояние приложения живёт в ядре.

## Что нужно

| Компонент | Версия | Где взять |
|---|---|---|
| Rust | из `rust-toolchain.toml` | `rustup target add aarch64-linux-android x86_64-linux-android` |
| cargo-ndk | 4.x | `cargo install cargo-ndk --locked` |
| Android SDK | platform 35, build-tools 35 | Android Studio → SDK Manager |
| NDK | 28.2.13676358 (`ndkVersion` в `app/build.gradle.kts`) | SDK Manager → SDK Tools → NDK (Side by side) |
| JDK | 17–21 | например, JBR из Android Studio |

Gradle-обёртка (Gradle 9.3.1, AGP 9.1.0) лежит в репозитории, ставить Gradle не нужно.

Путь к SDK указывается в `platforms/android/local.properties` (файл не коммитится):

```properties
sdk.dir=C\:\\Users\\<you>\\AppData\\Local\\Android\\Sdk
```

Android Studio создаёт его сама при открытии проекта. Если SDK не в стандартном месте, посмотрите
путь в Android Studio → Settings → Languages & Frameworks → Android SDK.

## Сборка и запуск

Из `platforms/android`:

```bash
./gradlew installDebug
```

Gradle сам вызывает `cargo ndk -t arm64-v8a -t x86_64 -P 26 build --release -p vard-ffi`
(задача `cargoNdk`, переменные `ANDROID_HOME`/`ANDROID_NDK_HOME` выставляются из SDK проекта) и
копирует `.vard`-файлы проекта в assets (`copyVardSources`). Затем:

```bash
adb shell am start -n dev.vard.host/.MainActivity
```

Параметры (`gradle.properties` или `-P...`):

| Свойство | По умолчанию | Смысл |
|---|---|---|
| `vard.app` | `examples/todo` | какой Vard-проект упаковать (путь от корня репозитория) |
| `vard.abis` | `arm64-v8a,x86_64` | ABI для `cargo ndk -t`; для эмулятора хватит `-Pvard.abis=x86_64` |
| `vard.skipCargo` | `false` | не вызывать cargo, взять готовые `.so` из `app/src/main/jniLibs/<abi>/` |

Собрать библиотеку вручную (например, для `vard.skipCargo=true`):

```bash
cargo ndk -t arm64-v8a -t x86_64 -P 26 -o platforms/android/app/src/main/jniLibs build --release -p vard-ffi
```

Ошибки компиляции Vard показываются на экране; ошибки рантайма (`R…`) пишутся в logcat с тегом
`Vard`:

```bash
adb logcat -s Vard
```

## Ограничения v0.1

- Отрисовка через `Canvas` на каждый кадр целиком; GPU-рендер и диффы дерева — следующие шаги
  дорожной карты (`docs/architecture.md`).
- Текст переносится по метрикам эталонного шрифта, а рисуется платформенным: строки не
  переполняются (Roboto уже DejaVu), но перенос может отличаться от «родного» Android.
- `Image` рисуется плейсхолдером (как в headless-растре), `extern fn` пока не связаны с Kotlin.
- Доступность (TalkBack) ещё не подключена к семантическому дереву.
- Прокрутка — простое перетаскивание без инерции.

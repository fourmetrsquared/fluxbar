# FluxBar: пошаговое руководство

Практическое руководство по настройке панели, созданию виджетов и написанию
плагинов. Оно дополняет справочники:

- [CONFIGURATION.md](CONFIGURATION.md) — полный список полей конфигурации;
- [PLUGIN_GUIDE.md](PLUGIN_GUIDE.md) — протокол плагинов, SDK, скрипты,
  совместимость со SketchyBar и плагин рабочих столов;
- [examples/config/full.toml](../examples/config/full.toml) — большой пример;
- [examples/plugins/hello](../examples/plugins/hello) и
  [hello-python](../examples/plugins/hello-python) — рабочие примеры;
- [examples/plugins/workspaces](../examples/plugins/workspaces) — плитки
  рабочих столов для yabai и AeroSpace.

Команды ниже предполагают, что `fluxbar` установлен в `PATH` (`./build.sh
install`) или запускается через `./build.sh run`. Если бинарник ещё не
установлен, замените `fluxbar` на `./build.sh run` в командах запуска.

## Содержание

1. [Первый запуск](#1-первый-запуск)
2. [Как устроена панель](#2-как-устроена-панель)
3. [Виджеты](#3-виджеты)
4. [Плагины: основные понятия](#4-плагины-основные-понятия)
5. [Пишем Rust-плагин с нуля](#5-пишем-rust-плагин-с-нуля)
6. [Пишем плагин на Python](#6-пишем-плагин-на-python)
7. [Редактирование плагинов](#7-редактирование-плагинов)
8. [События, таймеры и IPC](#8-события-таймеры-и-ipc)
9. [Что применяется при reload](#9-что-применяется-при-reload)
10. [Отладка](#10-отладка)
11. [Быстрый справочник](#11-быстрый-справочник)

## 1. Первый запуск

### 1.1. Сборка и установка

```bash
git clone https://github.com/fourmetrsquared/fluxbar.git fluxbar
cd fluxbar

./build.sh install          # release-сборка в ~/.local/bin/fluxbar
fluxbar config init         # необязательно: config создастся при первом start
fluxbar start                # daemon + нативная панель
```

`./build.sh install` можно заменить на `cargo build --release`; тогда бинарник
лежит в `target/release/fluxbar`. `fluxbar start` работает в foreground и
держит daemon/панель до `Ctrl+C` или `fluxbar stop` из другого терминала.
Проверить окружение и пути:

```bash
./build.sh doctor
fluxbar paths
```

Типичные пути на macOS:

| Что | Где |
| --- | --- |
| Конфигурация | `~/.config/fluxbar/fluxbar.toml` |
| Исходники установленных плагинов | `~/.config/fluxbar/plugins/` |
| Регистрации плагинов | `~/.config/fluxbar/plugins.d/*.toml` |
| Unix-сокет IPC | `~/Library/Caches/dev.fluxbar.fluxbar/fluxbar.sock` |
| PID-файл | `~/Library/Caches/dev.fluxbar.fluxbar/fluxbar.pid` (рядом с сокетом) |

Путь конфигурации учитывает `XDG_CONFIG_HOME`, `FLUXBAR_CONFIG_DIR` и флаг
`--config` (или переменную `FLUXBAR_CONFIG`). Путь сокета можно переопределить
через `--socket` или `FLUXBAR_SOCKET`. При запуске без явного `--config` FluxBar
сам создаёт `fluxbar.toml`, `plugins/` и `plugins.d/`, если их нет. Существующий
конфиг не перезаписывается. Рядом с сокетом daemon держит заблокированный
PID-файл `0600`; второй экземпляр с тем же сокетом не запустится, а оставшийся
после падения PID-файл будет переиспользован. `SIGINT`, `SIGTERM` и
`fluxbar stop` завершают таймеры, плагины и UI штатно.

### 1.1.1. Homebrew и автозапуск через `brew services`

Если нужен автозапуск при входе в систему, установите FluxBar через Homebrew.
Формула находится в `Formula/fluxbar.rb` и содержит описание пользовательского
LaunchAgent. До публикации первого тега используйте HEAD-установку из этого
репозитория:

```bash
brew tap fourmetrsquared/dev https://github.com/fourmetrsquared/fluxbar.git
brew install --HEAD fourmetrsquared/dev/fluxbar
brew services start fluxbar
```

После публикации стабильного tap команды короче:

```bash
brew tap fourmetrsquared/tap
brew install fluxbar
brew services start fluxbar
```

`brew services start` сразу запускает панель и регистрирует её автозапуск при
следующем входе. Управление сервисом:

```bash
brew services list
brew services restart fluxbar   # после обновления бинарника
brew services stop fluxbar      # остановить и убрать из автозапуска
```

В Homebrew 7+ для сторонних tap может потребоваться однократная команда
`brew trust fourmetrsquared/dev` (или `brew trust fourmetrsquared/tap` для
стабильного tap).

Запускайте `brew services` от своего пользователя, без `sudo`: это
пользовательский LaunchAgent, и панель должна работать в графической сессии
(`Aqua`), иначе у неё не будет доступа к WindowServer. Конфигурация остаётся в
`~/.config/fluxbar/fluxbar.toml` и создаётся автоматически при первом запуске.
Логи сервиса пишутся в `$(brew --prefix)/var/log/fluxbar.log` и
`$(brew --prefix)/var/log/fluxbar.err.log`. Для остановки управляемого сервиса
используйте `brew services stop fluxbar`; команда `fluxbar stop` завершает
процесс, но зарегистрированный сервис снова запустится при следующем входе.
Чтобы вернуть его раньше, выполните `brew services restart fluxbar`.

### 1.1.2. Готовые релизы и проверка артефактов

Тег `v*` запускает release-workflow: он проверяет исходники, собирает
универсальный бинарник для Apple Silicon и Intel, публикует `SHA256SUMS`,
release notes и стабильную Homebrew-формулу. Подписанная сборка и обновление
tap включаются, когда в репозитории настроены соответствующие секреты.

Те же артефакты можно собрать локально:

```bash
./build.sh dist
cd target/dist/assets
shasum -a 256 -c SHA256SUMS
```

Архив содержит `fluxbar`, `LICENSE`, `README.md`, `CHANGELOG.md`,
`SECURITY.md` и `THIRD_PARTY_NOTICES.md`. Для удаления установки из исходников
используйте `./build.sh uninstall --yes`: бинарник удаляется, конфигурация и
кэш сохраняются.

### 1.2. Проверка и перезагрузка

```bash
fluxbar config path
fluxbar config validate
fluxbar config show
fluxbar config reload
```

`config validate` проверяет TOML, includes, таймеры и основные ограничения.
`fluxbar config reload` сначала полностью валидирует новый файл, затем
атомарно применяет items, плагины, таймеры, монитор и видимость панели.
Подробная таблица — в разделе
[«Что применяется при reload»](#9-что-применяется-при-reload). Если поведение
не изменилось, перезапустите daemon:

```bash
fluxbar stop
fluxbar start
```

Логи удобнее смотреть в foreground-режиме:

```bash
fluxbar start -v
# ещё подробнее:
fluxbar start -vv
```

## 2. Как устроена панель

FluxBar рисует нативную панель AppKit. Конфигурация состоит из четырёх
основных частей:

```toml
[bar]          # геометрия, цвета, шрифты всей панели
[defaults]     # стили по умолчанию для всех виджетов
[items.<name>] # виджеты и их размещение
[plugins.<name>] # фоновые процессы-плагины
```

Панель делится на три группы: `left`, `center`, `right`. Внутри группы
виджеты идут слева направо (для `center` группа центрируется целиком).
Абсолютных координат и глобального z-index сейчас нет.

### 2.1. Минимальная панель

```toml
[bar]
height = 32
position = "top"
background = "0xE61E1E2E"
corner_radius = 8
item_spacing = 8
padding = { top = 0, left = 10, bottom = 0, right = 10 }
font = "SF Pro"
font_size = 13
foreground = "#FFFFFF"

[items.app]
position = "left"
icon = "◆"
label = "FluxBar"

[items.clock]
position = "right"
kind = "time"
icon = "◷"
label = "--:--"
```

После редактирования:

```bash
fluxbar config validate
fluxbar config reload
```

### 2.2. Геометрия и внешний вид

```toml
[bar]
height = 36                  # 0..256 pt
position = "top"             # "top" или "bottom"
background = "0xCC1E1E2E"    # 0xAARRGGBB: alpha идёт первой
corner_radius = 10
opacity = 1.0                # 0..1, влияет только на alpha фона панели
blur_radius = 18             # 0 = выключено; >0 включает нативный blur
item_spacing = 8
padding = { top = 0, left = 12, bottom = 0, right = 12 }
top_margin = 6               # отступ от верхнего/нижнего края
horizontal_margin = 12       # боковые отступы: плавающая капсула по центру
full_width = true
font = "SF Pro"
font_size = 13
foreground = "0xFFFFFFFF"
icon_font = "JetBrainsMono Nerd Font"
```

Цвета можно задавать как `#RRGGBB`, `#RRGGBBAA`, `0xAARRGGBB`, `rgb(...)`,
`rgba(...)` или именем (`black`, `white`, `transparent`, `red`, `green`,
`blue`, `yellow`, `orange`, `purple`, `pink`, `gray`).

`padding` — внутренние отступы панели. `item_spacing` — расстояние между
соседними виджетами одной группы. `top_margin` задаёт зазор между видимой
границей экрана и панелью сверху или снизу. `horizontal_margin` оставляет
одинаковые поля слева и справа; при значении больше нуля панель становится
плавающей капсулой по центру. `full_width = true` занимает всю доступную
ширину (экран минус боковые поля), при `false` ширина дополнительно
ограничивается 1200 pt. В обоих случаях панель центрируется.

`opacity` умножается на alpha фона `background`; цвета item-ов и текста не
меняются. `blur_radius` включает нативное размытие за панелью. На macOS 26
используется системный `NSGlassEffectView`: он даёт настоящую Liquid Glass
деформацию и преломление по границам, а также подстраивает материал под фон.
Внутренняя тёмная заливка рисуется с отступом, поэтому стеклянная кромка
остаётся видимой. На более старых версиях macOS используется
`NSVisualEffectView` с семантическими уровнями (`1–8` — мягкий, `9–24` —
средний, больше `24` — сильный) и нарисованные вручную блик, кромка и
затемнение снизу.

### 2.3. Стили по умолчанию

`[defaults]` позволяет не повторять шрифты, отступы и цвета в каждом виджете:

```toml
[defaults]
label_font = "SF Pro"
label_font_size = 13
icon_font = "JetBrainsMono Nerd Font"
icon_font_size = 15
label_padding = { top = 0, left = 4, bottom = 0, right = 4 }
icon_padding = { top = 0, left = 4, bottom = 0, right = 4 }
padding = { top = 0, left = 4, bottom = 0, right = 4 }
label_color = "0xFFCDD6F4"
icon_color = "0xFF89B4FA"
```

Значения из `[defaults]` подмешиваются при создании виджета. Поэтому изменения
`[defaults]` применяются к новым виджетам сразу, а к уже существующим — после
перезапуска daemon (или создания виджета заново).

### 2.4. Модульный конфиг

Большую конфигурацию удобно разбивать на файлы:

```toml
# ~/.config/fluxbar/fluxbar.toml
include = ["theme.toml", "work.toml"]
include_dir = "plugins.d"
```

- `include` — файлы относительно основного конфига; мержатся по порядку.
- `plugins.d/*.toml` — автоматически подключаемая директория; сюда
  `fluxbar plugin install` пишет регистрации плагинов.
- Мерж глубокий: частичный файл переопределяет только указанные ключи.
- `plugins.d` мержится после `include`, поэтому регистрации плагинов могут
  переопределять значения из основного файла.

Не кладите исходники плагинов в `plugins.d`: это директория **регистраций**
(`[plugins.<name>]`), а исходники лежат в `plugins/<name>/`.

### 2.5. Анимации

```toml
[animations]
duration = 480
easing = "spring"
disabled = false
```

Анимации интерполируют границы виджетов, цвета текста/фона/рамки, progress и
радиус панели. Доступны `linear`, `ease_in_*`, `ease_out_*`,
`ease_in_out_*` для quad/cubic/quart/quint/sine/circ/expo/back/elastic/bounce,
`ease_in_out_expo` (по умолчанию) и `spring` — пружинная кривая с небольшим
overshoot, похожая на анимации UIKit. Чтобы отключить анимации полностью:

```toml
[animations]
disabled = true
```

### 2.6. Скрытие и показ панели

```toml
[bar]
hidden = true
```

Во время работы панелью можно управлять событиями:

```bash
fluxbar trigger bar_toggle
fluxbar trigger bar_show
fluxbar trigger bar_hide
```

`bar.hidden` применяется при старте и при `fluxbar config reload` — daemon
показывает или скрывает панель без перезапуска. Вручную панелью можно
управлять событиями `bar_show`/`bar_hide`.

## 3. Виджеты

Виджет — это запись `[items.<name>]`. Имена item-ов — ключи в этой таблице.
Для создания через CLI/IPC (`fluxbar add`, `item.add`) допустимы латинские
буквы, цифры, `-`, `_`, `.`, до 64 символов. В конфиге используйте такие же
имена, чтобы команды и плагины могли к ним обращаться.

### 3.1. Типы виджетов

| `kind` | Что показывает | Кто обновляет |
| --- | --- | --- |
| `text` | иконка + текст | CLI, IPC, плагин |
| `icon` | иконка (текстовый глиф) | CLI, IPC, плагин |
| `progress` | текст; тонкая полоса при заполненном `progress` | плагин/IPC через `progress` |
| `graph` | линейный график | плагин/IPC через `graph` |
| `cpu` | загрузка CPU | системный монитор |
| `memory` | занятая/общая память | системный монитор |
| `battery` | заряд батареи | системный монитор |
| `volume` | громкость | системный монитор |
| `media` | исполнитель и трек | системный монитор (нужен `system.media = true`) |
| `time` | текущее время `HH:MM` | внутренние часы |
| `custom` | текст; при `properties.format` — дата/время | внутренние часы |
| `brightness` | зарезервировано | пока нет poller-а |

`kind` влияет на отрисовку не только текста: `graph` рисует график, а
`progress` (и любой item с заполненным `progress`) — полосу прогресса.
Встроенные `cpu`, `memory`, `battery`, `volume` дополнительно обновляют
`progress` и накапливают точки для графика.

### 3.2. Размещение

```toml
[items.left-one]
position = "left"

[items.center-one]
position = "center"

[items.right-one]
position = "right"
```

Правила компоновки:

- `left` начинается от `bar.padding.left`;
- `right` заканчивается у `bar.padding.right`;
- группа `center` центрируется по внутренней области панели (с учётом
  `bar.padding`);
- внутри группы порядок задаётся `drawing_order` (по возрастанию), затем
  именем виджета по алфавиту;
- между виджетами добавляется `bar.item_spacing`;
- высота виджета — внутренняя высота панели (с учётом `bar.padding`);
- `drawing_order` сейчас сортирует только внутри одной группы; глобального
  z-index нет;
- панель не раздвигает группы при нехватке места: широкий `center` может
  пересечься с `left`/`right`. Уменьшайте ширины или количество виджетов.

Пример порядка:

```toml
[items.10-app]
position = "left"
drawing_order = 10
label = "App"

[items.20-cpu]
position = "left"
drawing_order = 20
kind = "cpu"

[items.30-memory]
position = "left"
drawing_order = 30
kind = "memory"
```

Если `drawing_order` одинаковый, порядок будет алфавитным по имени.

### 3.3. Шаг за шагом: три группы

1. Откройте `~/.config/fluxbar/fluxbar.toml`.
2. Добавьте виджеты:

```toml
[items.app]
position = "left"
icon = "◆"
label = "FluxBar"

[items.media]
position = "center"
kind = "media"
icon = "♪"
label = ""
width = 220
max_width = 360

[items.clock]
position = "right"
kind = "time"
icon = "◷"
label = "--:--"
```

3. Проверьте и перезагрузите:

```bash
fluxbar config validate
fluxbar config reload
```

4. Если хотите изменить размещение без правки файла:

```bash
fluxbar set media position=left
```

Но помните: `fluxbar set` меняет только runtime-состояние. После перезапуска
виджет вернётся в позицию из конфига.

### 3.4. Шаг за шагом: стилизация виджета

```toml
[items.weather]
position = "right"
kind = "text"
icon = "☀"
label = "18°"
icon_color = "0xFFFFD166"
label_color = "0xFFFFFFFF"
label_font = "SF Pro"
label_font_size = 14
icon_font = "JetBrainsMono Nerd Font"
icon_font_size = 15
label_padding = { top = 0, left = 6, bottom = 0, right = 4 }
icon_padding = { top = 0, left = 4, bottom = 0, right = 2 }
padding = { top = 0, left = 4, bottom = 0, right = 4 }
background = "0x33FFFFFF"
background_corner_radius = 6
background_shape = "rounded"       # rectangle | rounded | circle
background_blur_radius = 12        # отдельный blur под этим item
opacity = 0.9                      # текст, иконка, фон и рамка
background_padding = { top = 0, left = 6, bottom = 0, right = 6 }
border_color = "0x44FFFFFF"
border_width = 1
align = "center"
width = 96
```

Что важно знать:

- `width = 0` (по умолчанию) означает автоматическую ширину по содержимому;
- `min_width`/`max_width` ограничивают коробку виджета;
- `align` выравнивает содержимое внутри коробки: `left`, `center`, `right`;
- `background_padding` вставляет фон внутрь коробки;
- `background_height` задаёт высоту фона вручную;
- `background_shape = "rectangle"` убирает скругление, `"rounded"` использует
  `background_corner_radius`, `"circle"` рисует эллипс (для круга сделайте
  item квадратным);
- `background_blur_radius` включает отдельное нативное стекло под фоном
  item-а; на macOS 26 это `NSGlassEffectView` с настоящей деформацией кромки,
  на старых системах — `NSVisualEffectView`; `0` выключает его;
- `opacity` применяется ко всему item-у: текст, иконка, фон, рамка и progress;
- `progress_color`/`progress_track_color` задают цвета заливки и трека,
  `progress_height` — высоту полосы, `progress_position` — `bottom`, `top` или
  `background`; для `top`/`bottom` содержимое автоматически сдвигается и не
  перекрывается с progress;
- рамка рисуется по границам фона с учётом `background_padding`; на macOS 26
  у размытых item-ов внешнюю кромку рисует системное Liquid Glass, поэтому
  `border_color`/`border_width` для них не используются;
- на macOS ширина текста измеряется настоящими метриками CoreText с учётом
  `label_font`/`icon_font` и размера, поэтому коробка item-а и отрисованный
  текст совпадают;
- если содержимое не помещается в `width`/`max_width`, сначала сжимается
  label, затем icon; длинный label обрезается многоточием в конце.

### 3.5. Шаг за шагом: встроенные системные виджеты

```toml
[items.cpu]
position = "left"
kind = "cpu"
icon = "▦"
label = "--%"

[items.memory]
position = "left"
kind = "memory"
icon = "▤"
label = "--"

[items.volume]
position = "right"
kind = "volume"
icon = "◍"
label = "--%"

[items.battery]
position = "right"
kind = "battery"
icon = "▮"
label = "--%"

[items.clock]
position = "right"
kind = "time"
icon = "◷"
label = "--:--"
```

Интервалы опроса задаются в `[system]`:

```toml
[system]
poll_interval = 2
battery_interval = 30
volume_interval = 5
emit_system_events = true
media = false
```

- CPU, память: `poll_interval`;
- батарея: `battery_interval` (минимум 5 секунд);
- громкость: `volume_interval`;
- медиа: `media = true` (использует AppleScript, опрашивается вместе с
  громкостью и может потребовать разрешение Automation);
- `space_command` — необязательная команда, печатающая id текущего space
  (например, `aerospace list-workspaces --focused`).
- `emit_system_events = false` сейчас отключает только `cpu_change` и
  `memory_change`; остальные события монитора продолжают приходить.

### 3.6. Шаг за шагом: progress и graph из плагина

Добавьте два виджета:

```toml
[items.demo]
position = "center"
kind = "text"
label = "demo"
width = 160

[items.demo-graph]
position = "center"
kind = "graph"
width = 90
```

Плагин обновляет их по имени:

```rust
let mut out = PluginOutput::new();
out.set_progress("demo", "Hello #42", 42.0)
    .set_graph("demo-graph", vec![0.1, 0.4, 0.2, 0.8]);
```

- `progress` — число от 0 до 100; значения обрезаются;
- `graph` — массив чисел; он **заменяет** текущие данные. Если нужна история,
  храните буфер в плагине и отправляйте весь массив (см. Rust-demo ниже);
  график сам масштабируется по min/max;
- `graph` рисуется только у item с `kind = "graph"`; его цвет берётся из
  `label_color` (отдельного цвета графика нет);
- progress рисуется у любого item, у которого заполнено поле `progress`.
  Цвет, высоту и положение можно задать в конфигурации:

  ```toml
  [items.cpu]
  progress_color = "0xFF34C759"
  progress_track_color = "0x3334C759"
  progress_height = 3
  progress_position = "bottom" # "top" или "background"
  ```

  `top`/`bottom` резервируют отдельную полосу, поэтому progress не
  перекрывает иконку, текст и график. `background` рисует заливку позади
  содержимого. Цвет по умолчанию наследуется от `label_color`;
- чтобы обновление сработало, item уже должен существовать. JSON-RPC-плагин
  обновляет только существующие item-ы; скриптовый плагин может создать их
  через `sketchybar --add item`.

### 3.7. Шаг за шагом: действия мыши

```toml
[items.terminal]
position = "left"
icon = "›_"
label = "Terminal"
click_action = { type = "shell", command = "open -a Terminal" }

[items.volume]
position = "right"
kind = "volume"
icon = "◍"
label = "--%"
click_action = { type = "shell", command = "open -a 'System Settings'" }
scroll_action = { type = "trigger", event = "volume_scroll", payload = { delta = 1 } }
```

Типы действий:

- `type = "shell"` — запускает `command` через `/bin/sh -c`;
- `type = "trigger"` — отправляет событие `event` с `payload`;
- `type = "plugin"` пока не реализован.

Каждое взаимодействие дополнительно отправляет событие:

| Событие | Payload |
| --- | --- |
| `mouse.clicked` | `{ "item": "...", "button": 0 или 1 }` |
| `mouse.scrolled` | `{ "item": "...", "delta": ... }` |
| `mouse.entered` | `{ "item": "..." }` |
| `mouse.left` | `{ "item": "..." }` |

Поэтому плагин может реагировать на клик, даже если `click_action` не задан.
Правый клик сейчас вызывает тот же `click_action`, что и левый; кнопка
различается только в payload события. `scroll_action.payload` — статический
payload действия; фактический `delta` колеса приходит только в событии
`mouse.scrolled`.

### 3.8. Item-скрипты

Item может сам запускать shell-команду по событию или по таймеру:

```toml
[items.weather]
position = "right"
label = "--"
script = "~/.config/fluxbar/scripts/weather.sh"
subscribe = ["system_woke", "weather_refresh"]
update_interval = 300
click_script = "open -a Weather"
```

`script` по умолчанию выполняется через `/bin/sh`; путь с расширением
`.bash` запускается через `/bin/bash`, а `.lua` — через `lua`. При старте
daemon каждый item-скрипт выполняется один раз для инициализации. `subscribe`
задаёт события, а `update_interval` — период `routine`-запусков. Скрипт должен
напечатать JSON с
обновлением (`{"weather":{"label":"19°"}}`) или вызвать
`sketchybar --set weather label=...`. `click_script` выполняется только при
клике по этому item-у. Обычный текст stdout игнорируется, таймаут — 60 секунд.

### 3.9. Runtime-команды

```bash
fluxbar add weather --kind text --position right --label "18°" --icon "☀"
fluxbar set weather label="19°" icon="☀"
fluxbar set weather progress=40
fluxbar set weather graph=1,2,3,4
fluxbar set weather visible=false
fluxbar set weather position=left
fluxbar remove weather
fluxbar list
```

`fluxbar add` поддерживает только `--kind`, `--position`, `--label`, `--icon`.
Цвета, фон, ширина, шрифты и действия мыши задаются в TOML.

`fluxbar set` принимает:

- `label=...`
- `icon=...`
- `progress=0..100`
- `graph=1,2,3`
- `status=...`
- `visible=true|false`
- `position=left|center|right`
- `kind=...`

Стили (цвета, фон, `background.shape`, `opacity`, шрифты) меняются через
совместимую команду, например:

```bash
fluxbar sketchybar --set weather background.color=0x663A6EA5 opacity=0.8
```

Runtime-изменения живут в памяти. Чтобы они сохранились после перезапуска,
перенесите их в `fluxbar.toml`. `fluxbar add` не записывает item в конфиг.
`graph` отображается только у item с `kind = "graph"`, поэтому в примере выше
для графика нужно сначала выполнить `fluxbar set weather kind=graph`.

### 3.10. Готовый пример панели: Liquid Glass

```toml
[bar]
height = 44
position = "top"
background = "0x2E0A0A0F"     # тёмное стекло, alpha идёт первой
corner_radius = 18
blur_radius = 22              # включает нативный blur + блик и рамку
item_spacing = 8
padding = { top = 0, left = 12, bottom = 0, right = 12 }
top_margin = 6                # зазор между меню-баром и капсулой
horizontal_margin = 12        # поля слева и справа
full_width = true
font = "SF Pro"
font_size = 14
foreground = "0xFFFFFFFF"

# Все item-ы по умолчанию — стеклянные капсулы. background_height = 30 и
# background_padding.bottom = 7 центрируют капсулу в панели высотой 44.
[defaults]
label_font_size = 13
icon_font_size = 14
label_color = "0xE6FFFFFF"
icon_color = "0xE6FFFFFF"
label_padding = { top = 0, left = 5, bottom = 0, right = 5 }
icon_padding = { top = 0, left = 4, bottom = 0, right = 4 }
padding = { top = 0, left = 9, bottom = 0, right = 9 }
background = "0x1FFFFFFF"
background_corner_radius = 13
background_blur_radius = 14
background_height = 30
background_padding = { top = 0, left = 0, bottom = 7, right = 0 }
border_color = "0x26FFFFFF"
border_width = 1

[items.app]
position = "left"
icon = "◆"
label = "FluxBar"
label_color = "0xFFFFFFFF"
click_action = { type = "shell", command = "open -a Terminal" }

[items.cpu]
position = "left"
kind = "cpu"
icon = "▦"
label = "--%"

[items.memory]
position = "left"
kind = "memory"
icon = "▤"
label = "--"

[items.media]
position = "center"
kind = "media"
icon = "♪"
label = ""
width = 220
max_width = 360

[items.volume]
position = "right"
kind = "volume"
icon = "◍"
label = "--%"

[items.battery]
position = "right"
kind = "battery"
icon = "▮"
label = "--%"

[items.clock]
position = "right"
kind = "time"
icon = "◷"
label = "--:--"
click_action = { type = "shell", command = "open -a Calendar" }

[events.timers.second]
interval = 1
event = "tick"
enabled = true

[animations]
duration = 480
easing = "spring"             # пружина в духе UIKit

[system]
poll_interval = 2
battery_interval = 30
volume_interval = 5
media = false
```

В этом примере медиа выключено: чтобы `media`-виджет показывал трек,
поставьте `system.media = true` и разрешите Automation. Таймер `tick` в примере
не обязателен; он оставлен как образец для плагинов, подписанных на `tick`.

## 4. Плагины: основные понятия

FluxBar поддерживает два вида плагинов.

**JSON-RPC-плагин** — отдельный долгоживущий процесс. FluxBar запускает его
один раз и обменивается сообщениями JSON-RPC 2.0 по одной строке:

- stdin — запросы от FluxBar;
- stdout — ответы плагина (только JSON, по одной строке);
- stderr — диагностика, попадает в лог daemon.

Такой плагин обновляет существующие item-ы (`label`, `icon`, `progress`,
`graph`, `status`, `visible`, `position`, `kind`), отправляет события в шину и
пишет лог. Через протокол он не создаёт item-ы: их нужно объявить в конфиге
или создать через `fluxbar add`.

**Плагин-скрипт** (`sh`, `bash`, `lua`) запускается на каждое событие и тик.
Он может печатать JSON-обновление или вызывать совместимый CLI `sketchybar`.
Через `sketchybar --add item` скрипт может создавать и удалять item-ы
самостоятельно — так работает плагин рабочих столов.

### 4.1. Регистрация плагина

Обычно регистрацию создаёт установщик:

```bash
fluxbar plugin install github:user/repo
fluxbar plugin install ./my-plugin
fluxbar plugin install ./my-plugin --no-build
fluxbar plugin install ./my-plugin --link
```

> **Безопасность:** плагин, его `build`-команда, item-скрипты и действия мыши
> выполняются с правами вашего пользователя. Устанавливайте только доверенные
> плагины. Если в репозитории есть `build`, сначала просмотрите его или
> используйте `--no-build`. Подробности — в [SECURITY.md](../SECURITY.md).

Установщик создаёт `~/.config/fluxbar/plugins.d/<name>.toml`:

```toml
[plugins.my-plugin]
type = "external"
command = "/Users/you/.config/fluxbar/plugins/my-plugin/target/release/my-plugin"
subscribe = ["tick"]
tick_interval = 2
auto_restart = true
```

Поля `[plugins.<name>]`:

| Поле | Значение |
| --- | --- |
| `type` | `external` (stdio); `uds`, `wasm`, `builtin` зарезервированы |
| `runtime` | `jsonrpc`, `sh`, `bash` или `lua`; иначе определяется по расширению |
| `command` | исполняемый файл |
| `args` | дополнительные аргументы |
| `path` | альтернатива `command` |
| `env` | переменные окружения |
| `working_dir` | рабочая директория процесса |
| `subscribe` | список событий; `["*"]` — все, кроме внутренних `item_*` |
| `tick_interval` | период `tick` в секундах |
| `auto_restart` | перезапускать после падения |
| `max_restarts` | лимит перезапусков (`0` — без лимита) |
| `restart_delay` | базовая задержка перезапуска |
| `config` | произвольный JSON/TOML, передаётся в `init` |

Дополнительные настройки (`config`, `env`, `working_dir`) лучше держать в
основном `fluxbar.toml`:

```toml
[plugins.my-plugin.config]
greeting = "Hi"
```

Установщик перезаписывает `plugins.d/<name>.toml` при `plugin update`, поэтому
ручные правки этого файла могут потеряться. `subscribe`, `tick_interval` и
`build`/`entry` удобнее менять в `fluxbar-plugin.toml` и переустанавливать
плагин.

### 4.2. Манифест плагина

В корне плагина можно положить `fluxbar-plugin.toml`:

```toml
name = "my-plugin"
description = "What it does"
build = "cargo build --release"       # необязательно
entry = "target/release/my-plugin"    # относительно корня плагина
runtime = "jsonrpc"                   # jsonrpc | sh | bash | lua
subscribe = ["tick", "my_event"]
tick_interval = 2
```

Без манифеста установщик пытается определить тип проекта:

| Проект | Build | Entry |
| --- | --- | --- |
| `Cargo.toml` | `cargo build --release` | `target/release/<package>` |
| `package.json` | `npm install` | `index.js` |
| `main.py` | нет | `main.py` |
| одиночный `.sh` / `.bash` / `.lua` | нет | сам файл |

Если `entry` заканчивается на `.py`, установщик регистрирует `python3` +
путь к скрипту. Если на `.js` — `node` + путь. Можно также указать строку
`entry = "python3 plugin.py"`. Одиночный shell/Lua-файл устанавливается через
`fluxbar plugin install ./tick.sh --no-build`; runtime определяется
автоматически.

### 4.3. Жизненный цикл

1. FluxBar запускает процесс.
2. Отправляет `init` с `name`, `config`, `subscribe`, `version`. Ошибка `init`
   считается падением запуска и запускает обычную restart-политику; плагин не
   остаётся в состоянии running.
3. Отправляет `event` для подписанных событий и `tick` каждые
   `tick_interval` секунд. `plugin.notify` доставляет событие только указанному
   плагину, минуя подписки.
4. При остановке отправляет `shutdown` и ждёт завершения (с таймаутом).
   `max_restarts` считает подряд идущие падения и сбрасывается после 60 секунд
   стабильной работы.

Плагин должен отвечать на каждый запрос строкой JSON с тем же `id`. Ответ на
`event` может содержать:

```json
{
  "jsonrpc": "2.0",
  "id": 7,
  "result": {
    "updates": {
      "my-item": { "label": "42%", "progress": 42.0 }
    },
    "events": [
      { "name": "my_event_done", "payload": { "ok": true } }
    ],
    "log": "optional diagnostic"
  }
}
```

Host также выставляет процессу переменные `FLUXBAR_PLUGIN_NAME` и
`FLUXBAR_PROTOCOL=1`, а строки stderr пробрасывает в лог daemon.

Таймаут ответа на `event` — 10 секунд; он считается падением плагина и
запускает restart-политику. stdout нельзя использовать для отладочных
сообщений: только JSON-ответы. Для отладки пишите в stderr.

### 4.4. Плагины-скрипты: sh, bash и Lua

Скриптовый плагин не держит процесс и не говорит на JSON-RPC. FluxBar запускает
его для `plugin_started`, для каждого события из `subscribe` и для каждого
`tick_interval`. Регистрация:

```toml
[plugins.clock_script]
type = "external"
runtime = "sh"
command = "~/.config/fluxbar/plugins/clock-script/tick.sh"
subscribe = ["space_change"]
tick_interval = 1
```

Если `runtime` не указан, он определяется по `.sh`, `.bash`, `.lua` или по
интерпретатору в `command`/`args`. Для Lua нужен `lua` в `PATH`, для sh
используется `/bin/sh`, для bash — `/bin/bash`.

Скрипт получает переменные `NAME`, `SENDER`, `INFO`, `CONFIG_DIR`,
`FLUXBAR_SOCKET`, `FLUXBAR_BIN`, `SKETCHYBAR` и `PATH` с каталогом shim-ов
`<config-dir>/bin`. Есть два способа обновить панель.

Первый — напечатать JSON:

```sh
#!/bin/sh
printf '{"clock_script":{"label":"%s"}}\n' "$(date +%H:%M)"
```

Второй — вызвать `sketchybar`, как это делают плагины SketchyBar:

```bash
#!/bin/bash
set -euo pipefail
sketchybar --set clock_script label="$(date +%H:%M)" icon="◷"
```

Пример на Lua:

```lua
#!/usr/bin/env lua
io.write(string.format('{"clock_script":{"label":"%s"}}\n', os.date("%H:%M")))
```

Обычный текст в stdout игнорируется. Если скрипт печатает JSON, это должен быть
объект с обновлениями item-ов или полный ответ `PluginResult` с полями
`updates`, `events`, `log`. Wildcard `subscribe = ["*"]` не получает
внутренние события `item_*`, чтобы скрипт, меняющий item через `sketchybar
--set`, не зациклился.

### 4.5. Плагин рабочих столов (yabai и AeroSpace)

Готовый плагин лежит в
[examples/plugins/workspaces](../examples/plugins/workspaces). Он показывает
по одному item-у на каждый существующий рабочий стол и сам добавляет/удаляет
плитки при изменении их количества. На плитке — номер стола и до трёх
символов имени (`1 web`); активный стол рисуется чуть ярче остальных. Клик
переключает рабочий стол.

macOS не даёт публичного API для обнаружения и переключения Mission Control,
поэтому плагин опрашивает оконный менеджер: yabai (нативные Spaces) или
AeroSpace (его workspace-ы). `backend = "auto"` сначала пробует yabai, затем
AeroSpace.

#### yabai

```bash
brew install koekeishiya/formulae/yabai
brew services start yabai
cargo build --release --manifest-path examples/plugins/workspaces/Cargo.toml
fluxbar plugin install examples/plugins/workspaces --no-build
```

У нативных Spaces нет имён. Задайте label в yabai или сопоставьте номер стола
с именем в `names`:

```bash
yabai -m space 1 --label web
```

Для переключения по клику нужно scripting addition yabai.

#### AeroSpace

```bash
brew install --cask nikitabobko/tap/aerospace
cargo build --release --manifest-path examples/plugins/workspaces/Cargo.toml
fluxbar plugin install examples/plugins/workspaces --no-build
```

Номер — позиция workspace в `aerospace list-workspaces`, имя — имя workspace
(его можно переопределить в `names`). Новые окна переносятся по явным правилам
через `aerospace move-node-to-workspace`.

Добавьте регистрацию и настройки в `fluxbar.toml` или `plugins.d/workspaces.toml`:

```toml
[plugins.workspaces]
type = "external"
command = "/Users/you/.config/fluxbar/plugins/workspaces/target/release/fluxbar-plugin-workspaces"
subscribe = []
tick_interval = 1
auto_restart = true

[plugins.workspaces.config]
# auto: сначала yabai, если его демон отвечает, затем AeroSpace.
backend = "auto"          # auto | yabai | aerospace
all_monitors = true
icon = ""                 # необязательно; номер уже входит в label
background = "0x1FFFFFFF"
selected_background = "0x52FFFFFF"
visible_background = "0x2EFFFFFF"
foreground = "0xC6FFFFFF"
selected_foreground = "0xFFFFFFFF"
opacity = 1.0
blur_radius = 14
corner_radius = 13
shape = "rounded"
width = 64
tile_height = 30
padding_left = 4
padding_right = 4
move_existing = false

# Короткие имена: ключ — имя workspace (AeroSpace) либо label/номер (yabai).
[plugins.workspaces.config.names]
"1" = "web"
"2" = "term"
"3" = "chat"

[plugins.workspaces.config.icons]
"1" = "W"
"2" = "T"

[plugins.workspaces.config.colors]
"2" = "0x663A6EA5"

# Полные переопределения для одной плитки; остальные поля берутся из глобальных.
[plugins.workspaces.config.tiles."2"]
icon = "T"
color = "0x663A6EA5"
opacity = 0.65
blur_radius = 0
shape = "rectangle"
corner_radius = 0
width = 56
tile_height = 24

# Только для AeroSpace: bundle id или имя приложения -> workspace.
[plugins.workspaces.config.rules]
"com.apple.Safari" = "2"
"com.apple.Terminal" = "3"
```

`names`, `icons`, `colors` и `tiles` используют имя workspace (AeroSpace) или
label/номер стола (yabai) как ключ. Имя обрезается до трёх символов; числовое
имя не дублируется. `color`/`foreground` из `tiles` перекрывают и
выбранное/видимое состояние. В `rules` допустимы простые `*`-шаблоны; правила
сейчас работают только с AeroSpace. Новые окна опрашиваются примерно раз в
секунду; `move_existing = true` применяет правила и к уже открытым окнам при
старте.

Сгенерированные имена item-ов хранятся в `<config-dir>/workspaces.items` и
удаляются при следующем запуске, если стол исчез. Не объявляйте item-ы
`workspace.*` вручную.

## 5. Пишем Rust-плагин с нуля

### Шаг 1. Создайте проект

```bash
mkdir -p ~/Projects/fluxbar-plugins/demo/src
cd ~/Projects/fluxbar-plugins/demo
cargo init --bin --name demo
```

### Шаг 2. Подключите SDK

`Cargo.toml`:

```toml
[package]
name = "demo"
version = "0.1.0"
edition = "2024"

[dependencies]
fluxbar-sdk = { path = "/absolute/path/to/fluxbar/fluxbar-sdk" }
tokio = { version = "1", features = ["rt", "macros"] }
serde_json = "1"

# Плагин — самостоятельный workspace, как пример hello.
[workspace]
```

Замените путь на свой checkout FluxBar. Если SDK будет опубликован, здесь
можно использовать git- или crates.io-зависимость.

### Шаг 3. Напишите код

`src/main.rs`:

```rust
use fluxbar_sdk::async_trait;
use fluxbar_sdk::{run, Plugin, PluginContext, PluginEvent, PluginOutput};

struct Demo {
    greeting: String,
    ticks: u64,
    graph: Vec<f64>,
}

#[async_trait]
impl Plugin for Demo {
    async fn on_init(&mut self, ctx: &PluginContext) -> fluxbar_sdk::Result<()> {
        if let Some(greeting) = ctx.config_str("greeting") {
            self.greeting = greeting.to_string();
        }
        Ok(())
    }

    async fn on_event(
        &mut self,
        event: &PluginEvent,
        _ctx: &PluginContext,
    ) -> fluxbar_sdk::Result<PluginOutput> {
        let mut out = PluginOutput::new();
        if event.kind() == "refresh" {
            out.set_label(
                "demo",
                format!("{} ({})", self.greeting, event.source()),
            );
        }
        Ok(out)
    }

    async fn on_tick(&mut self, _ctx: &PluginContext) -> fluxbar_sdk::Result<PluginOutput> {
        self.ticks += 1;
        let progress = (self.ticks as f64 * 10.0) % 100.0;
        self.graph.push(progress / 100.0);
        if self.graph.len() > 60 {
            self.graph.remove(0);
        }

        let mut out = PluginOutput::new();
        out.set_progress(
            "demo",
            format!("{} #{}", self.greeting, self.ticks),
            progress,
        )
        .set_graph("demo-graph", self.graph.clone());
        Ok(out)
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> fluxbar_sdk::Result<()> {
    run(Demo {
        greeting: "Hello".into(),
        ticks: 0,
        graph: Vec::new(),
    })
    .await
}
```

Что здесь важно:

- `on_event` — обязательный метод; `on_init`, `on_tick`, `on_shutdown` —
  необязательные;
- `on_tick` вызывается для события `tick`; SDK сам перенаправляет его туда;
- `PluginContext::config_str`/`config_f64` читают `[plugins.<name>.config]`;
- `PluginOutput::set_label`, `set_progress`, `set_graph`, `emit`, `log`
  собирают ответ;
- `run` делает весь JSON-RPC-фрейминг.

### Шаг 4. Добавьте манифест

`fluxbar-plugin.toml`:

```toml
name = "demo"
description = "Demo plugin with text, progress and graph"
build = "cargo build --release"
entry = "target/release/demo"
subscribe = ["refresh"]
tick_interval = 2
```

`subscribe = ["refresh"]` означает, что плагин получает `refresh`, а `tick`
приходит отдельно благодаря `tick_interval`. Если нужны все события,
используйте `["*"]`.

### Шаг 5. Соберите и установите

```bash
cargo build --release
fluxbar plugin install ~/Projects/fluxbar-plugins/demo --no-build
```

Без `--no-build` установщик сам выполнит `cargo build --release`. Локальный
плагин по умолчанию **копируется** в
`~/.config/fluxbar/plugins/demo/`, а бинарник entry — в
`plugins/demo/target/release/demo`. Оригинальный проект остаётся на месте.

Для разработки без копирования:

```bash
fluxbar plugin install ~/Projects/fluxbar-plugins/demo --link
```

### Шаг 6. Добавьте item-ы

```toml
[items.demo]
position = "center"
kind = "text"
label = "demo"
width = 160

[items.demo-graph]
position = "center"
kind = "graph"
width = 90
```

Если daemon уже запущен, установщик вызывает `config.reload`, чтобы запустить
сам плагин. Item-ы, добавленные в конфиг **после** установки, подхватит
следующий `fluxbar config reload`. Можно создать их и в runtime:

```bash
fluxbar add demo --kind text --position center --label "demo"
fluxbar add demo-graph --kind graph --position center
```

### Шаг 7. Проверьте

```bash
fluxbar status
fluxbar list
fluxbar list --plugins
fluxbar trigger refresh --payload '{"reason":"manual"}'
```

В `fluxbar list --plugins` у плагина должно быть `STATE = running` и
`RUNNING = true`. Явные состояния: `starting`, `running`, `backoff`,
`stopped`, `failed`. Если плагин падает, смотрите stderr в логе daemon и
`last_error` в ответе JSON-RPC-метода `plugin.list`.

## 6. Пишем плагин на Python

Python-плагину SDK не нужен: достаточно читать stdin и писать JSON в stdout.

### Шаг 1. Создайте файлы

```bash
mkdir -p ~/Projects/fluxbar-plugins/py-demo
cd ~/Projects/fluxbar-plugins/py-demo
```

`plugin.py`:

```python
#!/usr/bin/env python3
import json
import sys
import time

ITEM = "py-demo"
started = time.time()
greeting = "Python"


def send(payload):
    sys.stdout.write(json.dumps(payload) + "\n")
    sys.stdout.flush()


def main():
    global greeting
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue

        try:
            request = json.loads(line)
        except json.JSONDecodeError:
            continue

        method = request.get("method")
        request_id = request.get("id")

        if method == "init":
            params = request.get("params", {})
            config = params.get("config") or {}
            greeting = config.get("greeting", greeting)
            result = None
        elif method == "event":
            kind = request.get("params", {}).get("kind", "event")
            if kind == "tick":
                seconds = int(time.time() - started)
                result = {
                    "updates": {
                        ITEM: {
                            "label": f"{greeting} {seconds}s",
                            "progress": min(100.0, seconds * 5.0),
                        }
                    }
                }
            else:
                result = {
                    "updates": {
                        ITEM: {"label": f"{greeting}: {kind}"}
                    }
                }
        elif method == "shutdown":
            send({"jsonrpc": "2.0", "id": request_id, "result": None})
            break
        else:
            send({
                "jsonrpc": "2.0",
                "id": request_id,
                "error": {
                    "code": -32601,
                    "message": f"unknown method {method}",
                },
            })
            continue

        send({"jsonrpc": "2.0", "id": request_id, "result": result})


if __name__ == "__main__":
    main()
```

`fluxbar-plugin.toml`:

```toml
name = "py-demo"
description = "Python demo plugin"
build = ""
entry = "plugin.py"
subscribe = ["refresh"]
tick_interval = 2
```

### Шаг 2. Установите и добавьте item

```bash
fluxbar plugin install ~/Projects/fluxbar-plugins/py-demo --no-build
```

```toml
[items.py-demo]
position = "right"
kind = "text"
label = "Python"
width = 140
```

```bash
fluxbar config reload
fluxbar list --plugins
```

### Правила для любого языка

- Один JSON-объект на строку, без многострочного вывода.
- `id` в ответе должен совпадать с `id` запроса.
- stdout — только протокол; отладочные `print` пишите в `sys.stderr`.
- После записи в stdout вызывайте `flush()`.
- Обрабатывайте `init`, `event`, `shutdown`; неизвестные методы возвращайте
  ошибкой `-32601`.
- Не завершайте процесс после одного события: плагин долгоживущий.

## 7. Редактирование плагинов

### 7.1. Два режима установки

| Режим | Где лежат исходники | Когда использовать |
| --- | --- | --- |
| Обычный (`install`) | копия в `~/.config/fluxbar/plugins/<name>` | стабильная установка |
| `--link` | ваш исходный каталог | активная разработка |

При обычной установке в `~/.config/fluxbar/plugins/<name>/.fluxbar-install.toml`
записывается путь к исходнику. `fluxbar plugin update <name>` читает его,
пересобирает исходник и синхронизирует копию.

### 7.2. Цикл разработки с `--link`

```bash
fluxbar plugin install ~/Projects/fluxbar-plugins/demo --link
# ... правите src/main.rs ...
cargo build --release
fluxbar plugin restart demo
```

`plugin restart` перезапускает процесс с тем же spec. Если вы изменили только
код и оставили `command`/`entry` прежними, этого достаточно.

### 7.3. Цикл разработки с копией

```bash
# правите исходник
cargo build --release
fluxbar plugin update demo
fluxbar plugin restart demo
```

`plugin update`:

- для git-плагина делает `git pull --ff-only`;
- пересобирает `build` из манифеста;
- для локальной копии заново копирует исходник и entry;
- перезаписывает регистрацию в `plugins.d/demo.toml`.

Он **не** перезапускает уже работающий плагин. После обновления выполните
`plugin restart`.

### 7.4. Изменение настроек плагина

Регистрация и spec плагина (`subscribe`, `tick_interval`, `config`, `env`,
`working_dir`, `command`) читаются при старте и сверяются при
`fluxbar config reload`. Если spec не изменился, процесс продолжает работать;
изменённый плагин перезапускается с новым spec, новый — запускается, удалённый
— штатно останавливается.

Поэтому:

- изменения кода → `cargo build` + `plugin update` (если копия) +
  `plugin restart`;
- изменения `[plugins.<name>]`/`plugins.d/*.toml` → `fluxbar config reload`
  (валидированный reload перезапускает изменённый spec, запускает новые и
  останавливает удалённые плагины);
- `plugin restart` не перечитывает конфиг и берёт уже загруженный spec.

### 7.5. Удаление и обновление

```bash
fluxbar plugin update demo
fluxbar plugin restart demo
fluxbar plugin remove demo
```

- `remove` удаляет регистрацию и установленную копию;
- при `--link` исходный каталог остаётся;
- `fluxbar config reload` останавливает удалённый плагин без перезапуска
  daemon.

## 8. События, таймеры и IPC

### 8.1. Встроенные события

| Событие | Payload |
| --- | --- |
| `cpu_change` | `{ "percent": 12.5 }` |
| `memory_change` | `{ "percent": 29.0, "used": ..., "total": ... }` |
| `power_state` | `{ "percent": 87, "charging": false }` |
| `volume_change` | `{ "volume": 80 }` |
| `media_change` | `{ "player": "...", "artist": "...", "title": "...", "playing": true }` |
| `app_focus` | `{ "app": "..." }` |
| `space_change` | `{ "space": "..." }` |
| `mouse.clicked` | `{ "item": "...", "button": 0 или 1 }` |
| `mouse.scrolled` | `{ "item": "...", "delta": ... }` |
| `mouse.entered` | `{ "item": "..." }` |
| `mouse.left` | `{ "item": "..." }` |

`brightness_change` объявлен, но poller-а пока нет.

### 8.2. Таймеры

```toml
[events.timers.second]
interval = 1
event = "tick"
enabled = true
payload = { source = "timer" }
```

Плагин подписывается:

```toml
[plugins.demo]
# ...
subscribe = ["tick"]
```

Таймер и `tick_interval` — разные механизмы:

- `[events.timers.*]` отправляет **ваше** событие (`tick`, `slow_tick`, ...);
- `plugin.tick_interval` заставляет host отправлять событие `tick` конкретному
  плагину (оно приходит даже при пустом `subscribe`).

### 8.3. Ручной trigger

```bash
fluxbar trigger build_finished --payload '{"job": 3, "ok": true}'
```

Плагин с `subscribe = ["build_finished"]` получит событие с этим payload.
Имена событий не могут быть пустыми и не должны содержать пробелы.

### 8.4. Плагин отправляет событие

```rust
out.emit("my_event_done", serde_json::json!({ "ok": true }));
```

Событие попадает в общую шину, и на него могут подписаться другие плагины.

### 8.5. JSON-RPC IPC

Кроме CLI, daemon доступен по Unix-сокету. Формат — JSON-RPC 2.0, одна строка
на сообщение. Точный путь сокета печатает `fluxbar paths`; пример через Python:

```python
import json, socket

path = "/Users/you/Library/Caches/dev.fluxbar.fluxbar/fluxbar.sock"
request = {
    "jsonrpc": "2.0",
    "id": 1,
    "method": "item.set",
    "params": {
        "name": "weather",
        "update": {"label": "19°", "visible": True},
    },
}

with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as sock:
    sock.connect(path)
    sock.sendall((json.dumps(request) + "\n").encode())
    print(sock.makefile().readline())
```

Основные методы: `item.add`, `item.set`, `item.remove`, `item.get`,
`item.list`, `event.trigger`, `config.reload`, `plugin.list`,
`plugin.restart`, `bar.show`, `bar.hide`, `system.status`.

Сокет создаётся с правами `0600`, а стандартная директория сокета — с правами
`0700`. Любой процесс, работающий от вашего пользователя, всё равно может
управлять панелью; не используйте `--socket` в общедоступной директории вроде
`/tmp`. Одно сообщение ограничено 1 MiB, число одновременных подключений и
время простоя также ограничены. Плагины и скрипты — это код с правами вашего
пользователя; см. [SECURITY.md](../SECURITY.md).

## 9. Что применяется при reload

`fluxbar config reload` (и `fluxbar trigger config_reload`, и JSON-RPC
`config.reload`) сначала полностью валидирует новый файл, затем применяет его
одной операцией. Если валидация не прошла, live-состояние не меняется.

| Изменение | `config_reload` |
| --- | --- |
| Фон, высота, padding, spacing, corner radius, position панели | применяется |
| `top_margin` / `horizontal_margin` / `bar.hidden` | применяется |
| Новые item-ы | добавляются как config-owned |
| `kind`/`position`/`visible` существующего config-item | применяется |
| Непустой статический `label`/`icon` | применяется, заменяет runtime-текст |
| Стили и `[defaults]` | применяется; runtime `label`/`icon`/`background` сохраняются только если поле не задано в item-секции |
| Item удалён из конфига | удаляется, если он config-owned |
| Item, созданный через `item.add`/CLI | сохраняется как runtime-owned |
| Новые плагины | запускаются |
| Изменённые плагины | перезапускаются с новым spec |
| Удалённые плагины | штатно останавливаются |
| Плагины без изменений | продолжают работать |
| Таймеры | пересоздаются из нового конфига |
| `[system]` | монитор перезапускается только при изменении настроек |
| Ошибка TOML/валидации | reload отклоняется, старая конфигурация остаётся активной |

После правки конфига минимальный цикл:

```bash
fluxbar config validate
fluxbar config reload
fluxbar status
```

Если что-то не совпало с ожиданием — `fluxbar stop && fluxbar start`.

## 10. Отладка

### 10.1. Headless-режим

```bash
fluxbar start --headless -vv
```

В другом терминале:

```bash
fluxbar status
fluxbar list
fluxbar list --plugins
fluxbar trigger refresh --payload '{"x":1}'
fluxbar stop
```

`--headless` не создаёт окно, но поднимает daemon, IPC, плагины и системный
монитор. Удобно для CI и проверки протокола.

### 10.2. Проверка плагина вручную

Можно отправить плагину несколько строк и посмотреть ответы:

```bash
printf '%s\n%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"init","params":{"name":"demo","config":{"greeting":"Hi"},"subscribe":["*"],"version":"0.1.0"}}' \
  '{"jsonrpc":"2.0","id":2,"method":"event","params":{"kind":"tick","payload":{},"timestamp":0,"source":"test"}}' \
  | ~/.config/fluxbar/plugins/demo/target/release/demo
```

Если в stdout появилось что-то кроме JSON-строк, протокол сломан.

### 10.3. Частые проблемы

| Симптом | Что проверить |
| --- | --- |
| Плагин не запускается | `command`/`entry` существует, бит исполнения, shebang, `python3`/`node` в PATH |
| Плагин запускается и сразу падает | stderr в логе, `fluxbar list --plugins`, `last_error` через RPC `plugin.list`; не пишет ли он отладку в stdout |
| Item не обновляется | имя item-а совпадает, item существует, плагин подписан на событие, `tick_interval` задан |
| Ответ плагина игнорируется | неверный `id`, неверная схема JSON, таймаут 10 с |
| Конфиг не применяется | `fluxbar config validate`, затем `fluxbar config reload`; проверьте лог daemon |
| Текст обрезается/наезжает | проверьте `width`/`max_width`, `padding` и `font_size`; длинный label обрезается с ellipsis |
| Иконка — квадрат | в шрифте нет глифа; проверьте `icon_font` (Nerd Font) |
| Медиа не появляется | `system.media = true`, разрешение Automation, запущен Music/Spotify |
| Нет панели | `fluxbar status`; не `hidden` ли панель; `bar_show`; логи `-v` |
| Несколько daemon-ов конфликтуют | проверьте сокет и процессы; не запускайте второй start |

### 10.4. Известные ограничения текущей версии

- Панель только macOS; core и headless-режим переносимы.
- Нет абсолютного `x`, глобального z-index и меню-барных алиасов.
- `display` (multi-monitor) не реализован.
- `blur_radius` использует семантические материалы macOS, а не точный радиус.
  На других платформах размытие не рисуется.
- `brightness_change` не опрашивается.
- `plugin`-тип mouse action и UDS/Wasm транспорты зарезервированы.
- Progress-bar рисуется тонкой полосой внизу; цвет, трек, высота и позиция
  (`bottom`/`top`/`background`) настраиваются, `top`/`bottom` резервируют
  отдельную полосу и не перекрывают содержимое.
- Фиксированный `width`/`max_width` может скрыть часть содержимого: label
  обрезается с ellipsis, icon сжимается только после label.
- Нет автоматического разрешения пересечений left/center/right; при
  `full_width = false` ширина панели ограничена 1200 pt.
- Скриптовые плагины требуют `/bin/sh`, `/bin/bash` или `lua` в `PATH`.
- Плагин рабочих столов опрашивает yabai (нативные Spaces) или AeroSpace;
  изменения подхватываются с задержкой до одного тика, для yabai нужен
  scripting addition.
- Совместимость со SketchyBar неполная: alias, bracket, popup, shadow и image
  свойства принимаются, но не реализованы.
- Wasm-песочницы нет: плагины выполняются с правами пользователя, устанавливайте
  только доверенный код.

## 11. Быстрый справочник

### Файлы

```text
~/.config/fluxbar/
├── fluxbar.toml
├── plugins/
│   └── <name>/
│       ├── fluxbar-plugin.toml      # если есть в исходнике
│       ├── <исходники и entry>
│       └── .fluxbar-install.toml   # только для скопированных локальных
└── plugins.d/
    └── <name>.toml                 # регистрация [plugins.<name>]
```

### Команды

```bash
fluxbar start [-v|-vv]
fluxbar start --headless
fluxbar stop
fluxbar status
fluxbar paths

fluxbar add <name> [--kind text] [--position left|center|right] [--label ...] [--icon ...]
fluxbar set <name> label=... icon=... progress=42 graph=1,2,3 visible=true position=right
fluxbar remove <name>
fluxbar list
fluxbar list --plugins

fluxbar trigger <event> [--payload '{"key":1}']
fluxbar config path
fluxbar config init [--force]
fluxbar config validate
fluxbar config show
fluxbar plugin install <source> [--name <name>] [--no-build] [--link]
fluxbar plugin update <name>
fluxbar plugin restart <name>
fluxbar plugin remove <name>
```

### Мини-чеклист нового плагина

- [ ] Плагин читает stdin построчно и пишет один JSON-ответ на строку.
- [ ] Реализованы `init`, `event`, `shutdown`; ответы содержат тот же `id`.
- [ ] В stdout нет отладочного вывода.
- [ ] Есть `fluxbar-plugin.toml` с `entry`, `subscribe`, `tick_interval`.
- [ ] Item-ы с нужными именами объявлены в конфиге.
- [ ] `fluxbar plugin install ...` выполнен; `fluxbar list --plugins` показывает
  `RUNNING = true` для плагина.
- [ ] `fluxbar trigger ...` и `plugin restart` проверены.
- [ ] Для релиза: `fluxbar config validate`, `./build.sh check`, документация
  обновлена.

### Полезные ссылки

- [Полный справочник конфигурации](CONFIGURATION.md)
- [Протокол и SDK плагинов](PLUGIN_GUIDE.md)
- [Пример конфигурации](../examples/config/full.toml)
- [Rust-плагин hello](../examples/plugins/hello)
- [Python-плагин hello-python](../examples/plugins/hello-python)
- [Статус реализации](STATUS.md)
- [Политика безопасности](../SECURITY.md)
- [Правила участия](../CONTRIBUTING.md)
- [Backlog](../TODO.md)

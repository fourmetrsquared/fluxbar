# FluxBar: пошаговое руководство

Практическое руководство по настройке панели, созданию виджетов и написанию
плагинов. Оно дополняет справочники:

- [CONFIGURATION.md](CONFIGURATION.md) — полный список полей конфигурации;
- [PLUGIN_GUIDE.md](PLUGIN_GUIDE.md) — протокол плагинов и SDK;
- [examples/config/full.toml](../examples/config/full.toml) — большой пример;
- [examples/plugins/hello](../examples/plugins/hello) и
  [hello-python](../examples/plugins/hello-python) — рабочие примеры.

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
git clone <repo> fluxbar
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

Путь конфигурации учитывает `XDG_CONFIG_HOME`, `FLUXBAR_CONFIG_DIR` и флаг
`--config` (или переменную `FLUXBAR_CONFIG`). Путь сокета можно переопределить
через `--socket` или `FLUXBAR_SOCKET`. При запуске без явного `--config` FluxBar
сам создаёт `fluxbar.toml`, `plugins/` и `plugins.d/`, если их нет. Существующий
конфиг не перезаписывается.

### 1.2. Проверка и перезагрузка

```bash
fluxbar config path
fluxbar config validate
fluxbar config show
fluxbar trigger config_reload
```

`config validate` проверяет TOML, includes, таймеры и основные ограничения.
`config_reload` полезен, но применяет не всё: подробная таблица — в разделе
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
fluxbar trigger config_reload
```

### 2.2. Геометрия и внешний вид

```toml
[bar]
height = 36                  # 0..256 pt
position = "top"             # "top" или "bottom"
background = "0xCC1E1E2E"    # 0xAARRGGBB: alpha идёт первой
corner_radius = 10
item_spacing = 8
padding = { top = 0, left = 12, bottom = 0, right = 12 }
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
соседними виджетами одной группы. `full_width = true` растягивает панель на
экран; при `false` ширина ограничивается 1200 pt.

> `top_margin` и `blur_radius` сейчас разбираются конфигом, но нативное
> размещение/размытие их не применяет. Не используйте их как рабочие настройки
> до исправления в backlog.

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
duration = 220
easing = "ease_out_expo"
disabled = false
```

Анимации интерполируют границы виджетов, цвета текста/фона/рамки, progress и
радиус панели. Доступны `linear`, `ease_in_*`, `ease_out_*`,
`ease_in_out_*` для quad/cubic/quart/quint/sine/circ/expo/back/elastic/bounce,
а также `ease_in_out_expo` (по умолчанию). Чтобы отключить анимации полностью:

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

`bar.hidden` применяется при старте. Изменение `hidden` в конфиге через
`config_reload` не переключает уже запущенную панель — используйте
`bar_show`/`bar_hide` или перезапуск.

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
fluxbar trigger config_reload
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
- рамка рисуется по границам коробки виджета;
- `max_width` пока не делает ellipsis для длинного текста — он может
  обрезаться или не помещаться в коробку; подбирайте ширину вручную;
- ширина текста в core оценивается эвристикой, а AppKit измеряет её отдельно
  и тоже не всегда точно. Для плотных панелей задавайте фиксированные `width`,
  чтобы избежать неожиданных пересечений.

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
- progress рисуется у любого item, у которого заполнено поле `progress`
  (сейчас это тонкая белая полоса внизу виджета; она может перекрывать
  содержимое);
- чтобы обновление сработало, item уже должен существовать. Плагин не создаёт
  новые item-ы, он только обновляет существующие.

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

### 3.8. Runtime-команды

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

Runtime-изменения живут в памяти. Чтобы они сохранились после перезапуска,
перенесите их в `fluxbar.toml`. `fluxbar add` не записывает item в конфиг.
`graph` отображается только у item с `kind = "graph"`, поэтому в примере выше
для графика нужно сначала выполнить `fluxbar set weather kind=graph`.

### 3.9. Готовый пример панели

```toml
[bar]
height = 34
position = "top"
background = "0xE61E1E2E"
corner_radius = 10
item_spacing = 8
padding = { top = 0, left = 12, bottom = 0, right = 12 }
font = "SF Pro"
font_size = 13
foreground = "0xFFCDD6F4"

[defaults]
label_padding = { top = 0, left = 4, bottom = 0, right = 4 }
icon_padding = { top = 0, left = 4, bottom = 0, right = 4 }
padding = { top = 0, left = 4, bottom = 0, right = 4 }

[items.app]
position = "left"
icon = "◆"
label = "FluxBar"
background = "0x33FFFFFF"
background_corner_radius = 6
background_padding = { top = 0, left = 6, bottom = 0, right = 6 }
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
duration = 220
easing = "ease_out_expo"

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

Плагин — это отдельный долгоживущий процесс. FluxBar запускает его один раз и
обменивается с ним JSON-RPC 2.0 по одной строке на сообщение:

- stdin — запросы от FluxBar;
- stdout — ответы плагина (только JSON, по одной строке);
- stderr — диагностика, попадает в лог daemon.

Плагин умеет:

- обновлять существующие item-ы (`label`, `icon`, `progress`, `graph`,
  `status`, `visible`, `position`, `kind`);
- отправлять события в общую шину;
- писать лог.

Плагин **не умеет** создавать новые item-ы через протокол. Item нужно заранее
объявить в конфиге или создать через `fluxbar add`.

### 4.1. Регистрация плагина

Обычно регистрацию создаёт установщик:

```bash
fluxbar plugin install github:user/repo
fluxbar plugin install ./my-plugin
fluxbar plugin install ./my-plugin --no-build
fluxbar plugin install ./my-plugin --link
```

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
| `command` | исполняемый файл |
| `args` | дополнительные аргументы |
| `path` | альтернатива `command` |
| `env` | переменные окружения |
| `working_dir` | рабочая директория процесса |
| `subscribe` | список событий; `["*"]` — все |
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
subscribe = ["tick", "my_event"]
tick_interval = 2
```

Без манифеста установщик пытается определить тип проекта:

| Проект | Build | Entry |
| --- | --- | --- |
| `Cargo.toml` | `cargo build --release` | `target/release/<package>` |
| `package.json` | `npm install` | `index.js` |
| `main.py` | нет | `main.py` |

Если `entry` заканчивается на `.py`, установщик регистрирует `python3` +
путь к скрипту. Если на `.js` — `node` + путь. Можно также указать строку
`entry = "python3 plugin.py"`.

### 4.3. Жизненный цикл

1. FluxBar запускает процесс.
2. Отправляет `init` с `name`, `config`, `subscribe`, `version`.
3. Отправляет `event` для подписанных событий и `tick` каждые
   `tick_interval` секунд.
4. При остановке отправляет `shutdown` и ждёт завершения (с таймаутом).

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
следующий `fluxbar trigger config_reload`. Можно создать их и в runtime:

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

В `fluxbar list --plugins` плагин должен быть в состоянии `RUNNING = true`.
Если он падает, смотрите stderr в логе daemon и `last_error` в ответе
JSON-RPC-метода `plugin.list` (CLI-таблица показывает только имя, running,
restarts и pid).

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
fluxbar trigger config_reload
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
`working_dir`, `command`) читаются при старте. `config_reload` запускает
**только новые** плагины; уже работающий плагин сохраняет старый spec.

Поэтому:

- изменения кода → `cargo build` + `plugin update` (если копия) +
  `plugin restart`;
- изменения `[plugins.<name>]`/`plugins.d/*.toml` → перезапуск daemon:
  `fluxbar stop && fluxbar start`;
- изменения `[plugins.<name>.config]` тоже требуют перезапуска daemon:
  `plugin restart` берёт сохранённый старый spec и не перечитывает конфиг.

### 7.5. Удаление и обновление

```bash
fluxbar plugin update demo
fluxbar plugin restart demo
fluxbar plugin remove demo
```

- `remove` удаляет регистрацию и установленную копию;
- при `--link` исходный каталог остаётся;
- если плагин запущен, после удаления регистрации перезапустите daemon, чтобы
  процесс точно завершился.

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

## 9. Что применяется при reload

`fluxbar trigger config_reload` — не полный перезапуск. Текущее поведение:

| Изменение | `config_reload` | Надёжный способ |
| --- | --- | --- |
| Фон, высота, padding, spacing, corner radius, position панели | применяется | reload |
| Новые item-ы | добавляются | reload |
| Стили существующих item-ов (цвета, ширина, padding) | частично применяются | reload, при сомнении restart |
| `kind`/`position` существующего item | не применяется | restart |
| `visible` существующего item | не применяется к runtime-состоянию | `fluxbar set <name> visible=...` или restart |
| Статический `label`/`icon` существующего item | может не обновиться | restart |
| Удаление item из конфига | item остаётся | `fluxbar remove <name>` |
| `[defaults]` | только новые item-ы | restart |
| Новые плагины | запускаются | reload |
| Изменённые/удалённые плагины | не перезапускаются/не останавливаются | restart daemon |
| Таймеры и `[system]` | не пересоздаются | restart daemon |
| `bar.hidden` | не применяется | `bar_show`/`bar_hide` или restart |
| `top_margin` | не применяется (баг нативной раскладки) | пока не реализовано |
| `blur_radius` | не применяется | пока не реализовано |

После правки конфига минимальный цикл:

```bash
fluxbar config validate
fluxbar trigger config_reload
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
| Конфиг не применяется | `config validate`; reload не всё применяет — см. раздел 9 |
| Текст обрезается/наезжает | фиксированный `width`, меньше `font_size`, больше `padding`; ellipsis пока нет |
| Иконка — квадрат | в шрифте нет глифа; проверьте `icon_font` (Nerd Font) |
| Медиа не появляется | `system.media = true`, разрешение Automation, запущен Music/Spotify |
| Нет панели | `fluxbar status`; не `hidden` ли панель; `bar_show`; логи `-v` |
| `top_margin` не работает | известное ограничение текущей версии |
| Несколько daemon-ов конфликтуют | проверьте сокет и процессы; не запускайте второй start |

### 10.4. Известные ограничения текущей версии

- Панель только macOS; core и headless-режим переносимы.
- Нет абсолютного `x`, глобального z-index и меню-барных алиасов.
- `top_margin` и `blur_radius` разбираются конфигом, но не применяются
  нативной панелью; `display` (multi-monitor), `script` и `update_interval`
  пока не реализованы полностью.
- `brightness_change` не опрашивается.
- `plugin`-тип mouse action и UDS/Wasm транспорты зарезервированы.
- Progress-bar рисуется тонкой белой полосой внизу, может перекрывать
  содержимое; цвет и высота не настраиваются.
- `max_width` не делает ellipsis; длинный текст может обрезаться или выйти за
  границы.
- Нет автоматического разрешения пересечений left/center/right; при
  `full_width = false` панель прижата влево.
- `config_reload` не является полным reconcile (см. раздел 9).
- Плагины не создают item-ы; их нужно объявлять в конфиге или через CLI.
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
- [Backlog](../TODO.md)

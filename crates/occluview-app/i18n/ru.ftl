## OccluView Russian catalog — DRAFT (machine draft, native review required).
## Status: DRAFT. Requires native dental/CAD terminology review + visual UI review before APPROVED.
## Contract: exact key/attribute/variable parity with en.ftl.

app-title = OccluView
app-window-title = OccluView 3D Viewer
align-panel-title = Сопоставление сканов
meshedit-window-title = Редактирование сетки

settings-language-label = Язык
settings-language-auto = Язык системы
settings-language-auto-current = Язык системы — { $language }
settings-language-catalog-fallback = { $tag } недоступен; используется английский.
settings-language-save-error = Не удалось сохранить язык. Повторная попытка…

about-title = Об OccluView
about-tagline = Исправление сеток · Редактирование для dental CAD
about-version = Версия { $version }

update-available-title = Доступно обновление
update-available-body = Версия { $version } готова к установке.
update-current-version = Установлена версия { $version }.
update-download = Загрузить обновление
update-open-release = Открыть страницу релиза
update-later = Позже
update-skip = Пропустить эту версию
update-skip-tooltip = Больше не предлагать эту версию; следующий релиз будет предложен
update-downloading = Загрузка OccluView { $version }
update-ready-title = OccluView { $version } готов к установке
update-ready-hint-windows = Установщик проверен. OccluView закроется, пока Windows применяет обновление.
update-ready-hint-other = Пакет проверен. Откроется системный установщик — подтвердите обновление там.
update-install-close = Установить и закрыть
update-failed-title = Обновление не удалось
update-dismiss = Скрыть

error-open-title = Не удаётся открыть файл
error-add-title = Не удаётся добавить файл
error-open-body = Не удалось открыть { $path }.

## Help surface — DRAFT. Gesture names stay invariant by contract.

help-title = Управление с клавиатуры и мыши
help-subtitle = Справка соответствует элементам управления, доступным в OccluView.
help-close = Закрыть

help-section-navigation = Навигация
help-section-tools = Инструменты
help-section-mesh-editing = Редактирование сетки
help-section-sculpt = Скульптинг
help-section-align-measure = Сопоставление и измерения
help-section-cut-view = Сечение
help-section-layers-preview = Слои и предпросмотр в проводнике

help-hintline-navigation = ПКМ вращение · СКМ панорама · колесо масштаб · СКМ фокус
help-hintline-mesh-editing = ЛКМ выбор · Shift+клик снять · рамка · Ctrl+Z отмена
help-hintline-sculpt = ЛКМ скульптинг · Shift меняет режим · Shift+колесо размер · Ctrl+колесо сила
help-hintline-align = ЛКМ точка · Ctrl/Command+перетаскивание поворот · Shift+перетаскивание стереть · ПКМ отмена
help-hintline-cut = ЛКМ установить/сдвинуть · Ctrl+колесо в сечении меняет размер · F переворот · Esc закрыть
help-hintline-measure = ЛКМ измерить · ПКМ очистить · колесо масштаб · Esc закрыть

help-hint-navigation-orbit-the-camera = Вращение камеры
help-hint-navigation-pan-the-camera = Панорама камеры
help-hint-navigation-pan-the-camera-2 = Панорама камеры
help-hint-navigation-zoom-toward-the-pointer = Масштаб к указателю
help-hint-navigation-recenter-on-the-surface = Центрировать на поверхности
help-hint-navigation-recenter-on-the-surface-when-enabled = Центрировать на поверхности, если включено
help-hint-navigation-open-the-layer-or-scene-menu-when-stationary = Открыть меню слоя или сцены неподвижным кликом
help-hint-tools-open-a-file = Открыть файл
help-hint-tools-open-cut-view = Открыть сечение
help-hint-tools-arm-the-ruler = Включить линейку
help-hint-tools-arm-thickness = Включить толщину
help-hint-tools-open-align = Открыть сопоставление
help-hint-tools-open-mesh-editing = Открыть редактирование сетки
help-hint-mesh-editing-select-a-face = Выбрать грань
help-hint-mesh-editing-unmark-a-face-or-screen-selection = Снять пометку с грани или экранного выбора
help-hint-mesh-editing-select-faces-in-a-screen-rectangle = Выбрать грани в экранном прямоугольнике
help-hint-mesh-editing-draw-a-freehand-selection-outline = Нарисовать произвольный контур выбора
help-hint-mesh-editing-close-and-apply-a-lasso-outline = Замкнуть и применить контур лассо
help-hint-mesh-editing-cancel-the-active-lasso-outline = Отменить активное лассо
help-hint-mesh-editing-select-all-visible-faces = Выбрать все видимые грани
help-hint-mesh-editing-delete-selected-faces = Удалить выбранные грани
help-hint-mesh-editing-undo-the-last-mesh-edit = Отменить последнее изменение сетки
help-hint-mesh-editing-redo-the-last-mesh-edit = Вернуть последнее изменение сетки
help-hint-sculpt-choose-add-remove = Выбрать добавление/удаление
help-hint-sculpt-choose-smooth = Выбрать сглаживание
help-hint-sculpt-sculpt-under-the-brush = Скульптинг под кистью
help-hint-sculpt-remove-or-strengthen-the-active-brush-mode = Удалить или усилить активный режим кисти
help-hint-sculpt-change-brush-size = Изменить размер кисти
help-hint-sculpt-change-brush-intensity = Изменить силу кисти
help-hint-align-measure-place-an-alignment-point-or-measurement-point = Поставить точку сопоставления или измерения
help-hint-align-measure-rotate-a-scan-in-align-s-manual-mode = Повернуть скан в ручном режиме сопоставления
help-hint-align-measure-erase-an-align-exclusion-region = Стереть исключённую область сопоставления
help-hint-align-measure-change-align-exclusion-brush-size = Изменить размер кисти исключения
help-hint-align-measure-undo-the-last-alignment-point-when-stationary = Отменить последнюю точку неподвижным кликом
help-hint-align-measure-clear-measurements-when-stationary = Очистить измерения неподвижным кликом
help-hint-align-measure-close-the-active-measurement-tool = Закрыть активный инструмент измерения
help-hint-cut-view-plant-or-move-the-cut-disc = Установить или сдвинуть секущий диск
help-hint-cut-view-change-disc-size = Изменить размер диска
help-hint-cut-view-zoom-the-section-view = Масштаб вида сечения
help-hint-cut-view-flip-the-kept-half-while-planted = Отразить оставляемую половину
help-hint-cut-view-unplant-the-disc-or-close-cut-view = Снять диск или закрыть сечение
help-hint-layers-preview-hide-the-layer-under-the-pointer = Скрыть слой под указателем
help-hint-layers-preview-restore-the-last-hidden-layer = Вернуть последний скрытый слой
help-hint-layers-preview-toggle-layer-translucency = Переключить полупрозрачность слоя
help-hint-layers-preview-orbit-the-preview-model = Вращение модели предпросмотра
help-hint-layers-preview-zoom-the-preview-model = Масштаб модели предпросмотра
help-hint-layers-preview-frame-the-preview-model = Вписать модель предпросмотра
help-hint-layers-preview-toggle-preview-wireframe = Переключить каркас предпросмотра

## Toolbar, empty state — DRAFT.

toolbar-open-label = Открыть
toolbar-open-hint = Открыть 3D-файлы ({ $shortcut })
toolbar-recent-hint = Недавние файлы
toolbar-add-label = Добавить
toolbar-add-hint = Добавить файлы в текущую сцену
toolbar-cut-label = Сечение
toolbar-cut-hint = Рассечь модель плоскостью ({ $shortcut })
toolbar-cut-unavailable = Для сечения нужен видимый слой
toolbar-ruler-label = Линейка
toolbar-ruler-hint = Измерить расстояние: две точки на модели ({ $shortcut })
toolbar-thickness-label = Толщина
toolbar-thickness-hint = Измерить толщину стенки: точка на оболочке ({ $shortcut })
toolbar-measure-blocked = Сначала завершите или отмените сессию редактирования
toolbar-measure-needs-layer = Для измерения нужен видимый слой сетки
toolbar-align-label = Сопоставление
toolbar-align-hint = Совместить два скана: по точке на каждом ({ $shortcut })
toolbar-edit-label = Правка
toolbar-edit-open = Редактирование сетки открыто
toolbar-edit-hint = Редактирование сетки: выбор и скульптинг ({ $shortcut })
toolbar-settings-label = Настройки
toolbar-settings-hint = Открыть настройки

empty-open-file = Открыть 3D-файл
empty-formats-hint = STL · PLY · OBJ · GLB · HPS — или перетащите файлы сюда

## Loading and export — DRAFT.

load-queued = { $count ->
    [one] В очереди { $count } слой
    [few] В очереди { $count } слоя
    [many] В очереди { $count } слоёв
   *[other] В очереди { $count } слоёв
}
load-opening = { $count ->
    [one] Открытие { $count } файла…
    [few] Открытие { $count } файлов…
    [many] Открытие { $count } файлов…
   *[other] Открытие { $count } файлов…
}
load-adding = { $count ->
    [one] Добавление { $count } файла…
    [few] Добавление { $count } файлов…
    [many] Добавление { $count } файлов…
   *[other] Добавление { $count } файлов…
}
load-open-failed-start = Не удалось открыть: загрузчик не запустился
load-add-failed-start = Не удалось добавить: загрузчик не запустился
load-open-failed-stopped = Не удалось открыть: загрузчик остановлен
load-loader-failed-summary = Не удалось запустить фоновый загрузчик сцены.
load-action-failed-open = Не удалось открыть: { $detail }
load-action-failed-add = Не удалось добавить: { $detail }

export-nothing-visible = Нет видимого для сохранения
export-unsupported-format = Неподдерживаемый формат
export-scene-saved = Сцена сохранена: { $path }
export-scene-saved-unmerged = Сцена сохранена (текстуры не объединены): { $path }
export-scene-failed-title = Не удалось сохранить сцену
export-scene-failed-summary = Не удалось сохранить сцену: { $detail }
export-layers-saved = { $written ->
    [one] Сохранён { $written } слой в { $dir }
    [few] Сохранено { $written } слоя в { $dir }
    [many] Сохранено { $written } слоёв в { $dir }
   *[other] Сохранено { $written } слоёв в { $dir }
}
export-layers-saved-failed = { $written ->
    [one] Сохранён { $written } слой в { $dir }
    [few] Сохранено { $written } слоя в { $dir }
    [many] Сохранено { $written } слоёв в { $dir }
   *[other] Сохранено { $written } слоёв в { $dir }
}; { $failed ->
    [one] { $failed } слой не записан
    [few] { $failed } слоя не записаны
    [many] { $failed } слоёв не записано
   *[other] { $failed } слоёв не записано
}
export-layers-saved-renamed = { $written ->
    [one] Сохранён { $written } слой в { $dir }
    [few] Сохранено { $written } слоя в { $dir }
    [many] Сохранено { $written } слоёв в { $dir }
   *[other] Сохранено { $written } слоёв в { $dir }
}; { $renamed ->
    [one] { $renamed } файл переименован, чтобы сохранить существующее
    [few] { $renamed } файла переименовано, чтобы сохранить существующее
    [many] { $renamed } файлов переименовано, чтобы сохранить существующее
   *[other] { $renamed } файлов переименовано, чтобы сохранить существующее
}
export-layers-saved-failed-renamed = { $written ->
    [one] Сохранён { $written } слой в { $dir }
    [few] Сохранено { $written } слоя в { $dir }
    [many] Сохранено { $written } слоёв в { $dir }
   *[other] Сохранено { $written } слоёв в { $dir }
}; { $failed ->
    [one] { $failed } слой не записан
    [few] { $failed } слоя не записаны
    [many] { $failed } слоёв не записано
   *[other] { $failed } слоёв не записано
}; { $renamed ->
    [one] { $renamed } файл переименован, чтобы сохранить существующее
    [few] { $renamed } файла переименовано, чтобы сохранить существующее
    [many] { $renamed } файлов переименовано, чтобы сохранить существующее
   *[other] { $renamed } файлов переименовано, чтобы сохранить существующее
}
mesh-exported-aligned = { $name } экспортирован в выровненной позиции как { $format }: { $path }
mesh-exported-aligned-warnings = { $name } экспортирован в выровненной позиции как { $format } (предупреждения: { $warnings }): { $path }
mesh-exported-unmoved = { $name } экспортирован (скан не сдвигался) как { $format }: { $path }
mesh-exported-unmoved-warnings = { $name } экспортирован (скан не сдвигался) как { $format } (предупреждения: { $warnings }): { $path }
mesh-warning-vertex-colors = цвета вершин не записаны
mesh-warning-uvs = UV не записаны
mesh-warning-texture-image = изображение текстуры не записано
mesh-export-warnings = Предупреждения экспорта: { $warnings }
mesh-export-failed-title = Не удалось экспортировать слой
mesh-export-failed-summary = Не удалось экспортировать слой: { $detail }

## Repair card and toasts — DRAFT.

repair-title = Исправление сетки
repair-clean-headline = Нечего исправлять — сетка чистая
repair-copy-details = Копировать детали
repair-copy-tooltip = Скопировать полный отчёт по проходам в буфер обмена
repair-line-welded = { $count ->
    [one] Сварена { $grouped } дублирующаяся вершина
    [few] Сварены { $grouped } дублирующиеся вершины
    [many] Сварено { $grouped } дублирующихся вершин
   *[other] Сварено { $grouped } дублирующихся вершин
}
repair-line-slivers = { $count ->
    [one] Удалена { $grouped } тонкая грань
    [few] Удалены { $grouped } тонкие грани
    [many] Удалено { $grouped } тонких граней
   *[other] Удалено { $grouped } тонких граней
}
repair-line-duplicate-faces = { $count ->
    [one] Удалена { $grouped } дублирующаяся грань
    [few] Удалены { $grouped } дублирующиеся грани
    [many] Удалено { $grouped } дублирующихся граней
   *[other] Удалено { $grouped } дублирующихся граней
}
repair-line-nonmanifold = { $count ->
    [one] Исправлено { $grouped } неманифолдное ребро
    [few] Исправлены { $grouped } неманифолдных ребра
    [many] Исправлено { $grouped } неманифолдных рёбер
   *[other] Исправлено { $grouped } неманифолдных рёбер
}
repair-line-bowtie = { $count ->
    [one] Разделена { $grouped } вершина-бабочка
    [few] Разделены { $grouped } вершины-бабочки
    [many] Разделено { $grouped } вершин-бабочек
   *[other] Разделено { $grouped } вершин-бабочек
}
repair-line-reoriented = { $count ->
    [one] Переориентирован { $grouped } треугольник
    [few] Переориентированы { $grouped } треугольника
    [many] Переориентировано { $grouped } треугольников
   *[other] Переориентировано { $grouped } треугольников
}
repair-line-flipped = { $count ->
    [one] Развёрнута { $grouped } вывернутая часть
    [few] Развёрнуты { $grouped } вывернутые части
    [many] Развёрнуто { $grouped } вывернутых частей
   *[other] Развёрнуто { $grouped } вывернутых частей
}
repair-line-debris = { $count ->
    [one] Удалена { $grouped } мусорная часть
    [few] Удалены { $grouped } мусорные части
    [many] Удалено { $grouped } мусорных частей
   *[other] Удалено { $grouped } мусорных частей
}
repair-line-pinholes = { $count ->
    [one] Закрыто { $grouped } точечное отверстие
    [few] Закрыты { $grouped } точечных отверстия
    [many] Закрыто { $grouped } точечных отверстий
   *[other] Закрыто { $grouped } точечных отверстий
}
repair-line-unused = { $count ->
    [one] Удалена { $grouped } неиспользуемая вершина
    [few] Удалены { $grouped } неиспользуемые вершины
    [many] Удалено { $grouped } неиспользуемых вершин
   *[other] Удалено { $grouped } неиспользуемых вершин
}
repair-open-rims = { $count ->
    [one] Осталась { $grouped } открытая кромка (граница скана)
    [few] Остались { $grouped } открытые кромки (граница скана)
    [many] Осталось { $grouped } открытых кромок (граница скана)
   *[other] Осталось { $grouped } открытых кромок (граница скана)
}
repair-skipped-rims = { $count ->
    [one] { $grouped } кромка не заполнена (непростой контур)
    [few] { $grouped } кромки не заполнены (непростой контур)
    [many] { $grouped } кромок не заполнено (непростой контур)
   *[other] { $grouped } кромок не заполнено (непростой контур)
}
repair-toast-welded = { $count ->
    [one] Сварена { $count } вершина
    [few] Сварены { $count } вершины
    [many] Сварено { $count } вершин
   *[other] Сварено { $count } вершин
}
repair-toast-slivers = { $count ->
    [one] Удалена { $count } тонкая грань
    [few] Удалены { $count } тонкие грани
    [many] Удалено { $count } тонких граней
   *[other] Удалено { $count } тонких граней
}
repair-toast-duplicate-faces = { $count ->
    [one] { $count } дублирующаяся грань
    [few] { $count } дублирующиеся грани
    [many] { $count } дублирующихся граней
   *[other] { $count } дублирующихся граней
}
repair-toast-nonmanifold = { $count ->
    [one] Исправлено { $count } неманифолдное ребро
    [few] Исправлены { $count } неманифолдных ребра
    [many] Исправлено { $count } неманифолдных рёбер
   *[other] Исправлено { $count } неманифолдных рёбер
}
repair-toast-bowtie = { $count ->
    [one] Разделена { $count } бабочка
    [few] Разделены { $count } бабочки
    [many] Разделено { $count } бабочек
   *[other] Разделено { $count } бабочек
}
repair-toast-reoriented = { $count ->
    [one] Переориентирован { $count } треугольник
    [few] Переориентированы { $count } треугольника
    [many] Переориентировано { $count } треугольников
   *[other] Переориентировано { $count } треугольников
}
repair-toast-flipped = { $count ->
    [one] Развёрнута { $count } вывернутая часть
    [few] Развёрнуты { $count } вывернутые части
    [many] Развёрнуто { $count } вывернутых частей
   *[other] Развёрнуто { $count } вывернутых частей
}
repair-toast-debris = { $count ->
    [one] Удалена { $count } мусорная часть
    [few] Удалены { $count } мусорные части
    [many] Удалено { $count } мусорных частей
   *[other] Удалено { $count } мусорных частей
}
repair-toast-pinholes = { $count ->
    [one] Закрыто { $count } точечное отверстие
    [few] Закрыты { $count } точечных отверстия
    [many] Закрыто { $count } точечных отверстий
   *[other] Закрыто { $count } точечных отверстий
}
repair-toast-unused = { $count ->
    [one] Удалена { $count } неиспользуемая вершина
    [few] Удалены { $count } неиспользуемые вершины
    [many] Удалено { $count } неиспользуемых вершин
   *[other] Удалено { $count } неиспользуемых вершин
}
repair-toast-skipped = { $count ->
    [one] Пропущена { $count } кромка (непростой контур)
    [few] Пропущены { $count } кромки (непростой контур)
    [many] Пропущено { $count } кромок (непростой контур)
   *[other] Пропущено { $count } кромок (непростой контур)
}
repair-toast-done = Отремонтировано { $layer }: { $parts }
repair-toast-clean-rims = Скан уже чист: { $layer }, { $count ->
    [one] { $count } открытая кромка
    [few] { $count } открытые кромки
    [many] { $count } открытых кромок
   *[other] { $count } открытых кромок
}
repair-toast-clean = Скан уже чист: { $layer }
repair-edit-busy = Слой уже редактируется
repair-edit-failed-title = Не удалось изменить слой
repair-edit-failed-summary = Не удалось изменить слой: { $detail }
edit-locked-status = { $status } (без отмены: снимок слишком велик)

## Layers overlay, layer menu, scene menu — DRAFT.

layers-title = Слои
layers-count = { $count ->
    [one] { $count } слой
    [few] { $count } слоя
    [many] { $count } слоёв
   *[other] { $count } слоёв
}
layers-row-hide = Скрыть слой
layers-row-show = Показать слой
layers-row-opacity = Прозрачность слоя
layers-row-remove = Удалить слой

layer-menu-next-tint = Следующий оттенок
layer-menu-hide-colors = Скрыть цвета скана
layer-menu-show-colors = Показать цвета скана
layer-menu-disable-texture = Отключить текстуру
layer-menu-show-texture = Показать текстуру
layer-menu-mesh-editing = Редактирование сетки
layer-menu-split-bridge = Разделить мост…
layer-menu-repair = Исправление сетки
layer-menu-flip-normals = Отразить нормали
layer-menu-export = Экспорт слоя…
layer-menu-hide-wireframe = Скрыть каркас
layer-menu-show-wireframe = Каркас поверх
layer-menu-remove = Удалить слой

scene-menu-title = Сцена
scene-menu-save = Сохранить сцену как…
scene-menu-save-each = Сохранить каждый слой…
scene-menu-reset = Сбросить позиции
scene-menu-fit = Вписать вид

## Mesh editor palette — DRAFT.

meshedit-tab-edit = Редактирование сетки
meshedit-tab-sculpt = Скульптинг
meshedit-cancel-session = Отменить сессию (изменения будут отменены)
meshedit-header-edit = Редактирование сетки
meshedit-section-selection = Выбор
meshedit-section-edit-selection = Правка выбора
meshedit-section-close-holes = Закрытие отверстий
meshedit-section-sculpt = Скульптинг
meshedit-cell-lasso = Лассо
meshedit-cell-lasso-hint = Произвольный контур: клик ставит точки, двойной клик замыкает · Shift снимает пометку
meshedit-cell-object = Объект
meshedit-cell-object-hint = Клик по целому объекту многосоставного STL · Shift снимает пометку
meshedit-cell-surface = Поверхность
meshedit-cell-surface-hint = Помечать только видимую лицевую поверхность
meshedit-cell-through = Насквозь
meshedit-cell-through-hint = Помечать сквозь сетку, включая скрытые стороны
meshedit-cell-all = Все
meshedit-cell-all-hint = Пометить все грани (Ctrl+A)
meshedit-cell-none = Ничего
meshedit-cell-none-hint = Снять пометку
meshedit-cell-invert = Инверсия
meshedit-cell-invert-hint = Поменять помеченные и непомеченные грани местами
meshedit-cell-delete = Удалить
meshedit-cell-delete-hint = Удалить помеченные грани
meshedit-cell-crop = Обрезать
meshedit-cell-crop-hint = Оставить только помеченную область, остальное удалить
meshedit-cell-cut = Вырезать
meshedit-cell-cut-hint = Переместить помеченные грани в новую сетку — исходная остаётся
meshedit-cell-separate = Разделить
meshedit-cell-separate-hint = Разбить помеченную область на сетки по связным частям
meshedit-cell-close-holes = Закрыть отверстия
meshedit-cell-close-holes-hint = Закрывать отверстия, только если выбраны окружающие грани. Границы скана остаются открытыми.
meshedit-sculpt-addremove = Добавить / Удалить  [1]
meshedit-sculpt-addremove-hint = Наращивать материал перетаскиванием по скану; Shift убирает. Shift+колесо меняет размер, Ctrl+колесо — силу. Клавиша: 1.
meshedit-sculpt-smooth = Сгладить  [2]
meshedit-sculpt-smooth-hint = Расслаблять поверхность перетаскиванием; Shift форсирует максимум сглаживания. Shift+колесо меняет размер, Ctrl+колесо — силу. Клавиша: 2.
meshedit-slider-size = размер
meshedit-slider-size-hint = Размер кисти (Shift + колесо мыши)
meshedit-slider-force = сила
meshedit-slider-force-hint = Сила кисти (Ctrl + колесо мыши)
meshedit-limit-label = лимит
meshedit-limit-checkbox-hint = Ограничить исправление кромками не больше этого периметра
meshedit-limit-drag-hint = Выкл закрывает все безопасные отверстия в выбранной области; граница скана остаётся открытой
meshedit-status-unsaved = Несохранённые правки
meshedit-status-unsaved-hint = Неприменённые правки: «Готово» применяет, «Отмена» откатывает
meshedit-status-hint-sculpt = Перетаскивайте по поверхности для скульптинга · ПКМ вращает
meshedit-status-hint-object = Кликните объект, чтобы выбрать целиком · Shift снимает пометку
meshedit-status-hint-lasso = Клик обводит · двойной клик замыкает · Shift снимает пометку
meshedit-status-hint-default = Перетащите рамку для пометки · Shift снять · Del удалить
meshedit-session-undo = Назад
meshedit-session-undo-hint = Отменить последнюю правку сетки (Ctrl+Z)
meshedit-session-redo = Вперёд
meshedit-session-redo-hint = Вернуть отменённую правку сетки (Ctrl+Y)
meshedit-session-cancel = Отмена
meshedit-session-cancel-hint = Отбросить все правки сессии
meshedit-session-done = Готово
meshedit-session-done-hint = Применить правки и закрыть редактор

## Align Scans window — DRAFT.

align-title = Сопоставление сканов
align-tab-auto = Автоматически
align-tab-manual = Вручную
align-constraint-free = Движение и поворот во всех направлениях
align-constraint-free-hint = Перетаскивайте скан в любом направлении
align-constraint-z = Движение по оси z
align-constraint-z-hint = Перетаскивайте только вдоль вертикальной оси
align-constraint-xy = Движение в плоскости xy
align-constraint-xy-hint = Перетаскивайте только в горизонтальной плоскости
align-manual-drag-hint = Двигается захваченный скан · Ctrl+перетаскивание поворачивает
align-undo = Назад
align-undo-hint = На шаг назад
align-redo = Вперёд
align-redo-hint = На шаг вперёд
align-prompt-moving = Кликните точку на сетке, которая должна двигаться
align-prompt-other = Кликните ту же позицию на другой сетке
align-prompt-alternate = Кликайте поочерёдно точки в одинаковых позициях на двух сетках
align-prompt-placed = { $count ->
    [one] Поставлена { $count } стрелка
    [few] Поставлены { $count } стрелки
    [many] Поставлено { $count } стрелок
   *[other] Поставлено { $count } стрелок
}
align-back = Назад
align-back-hint = Отменить стрелку — клик ПКМ в виде делает то же самое
align-clear = Очистить
align-clear-hint = Убрать все стрелки и выбрать два скана заново — сканы остаются на месте
align-fit-perform = Выполнить сопоставление
align-fit-perform-hint = Двигать сетку на стрелки — нужно не меньше двух стрелок
align-fit-refine = Точное совмещение
align-fit-refine-hint = Совместить неизменённые участки модели после препарирования с исходной. Проверьте результат перед подтверждением
align-matching-parts = совпадающие части
align-matching-parts-hint = Максимальная доля соответствий для уточнения. Если неизменённых участков мало, Best Fit уменьшит её автоматически
align-max-influence = макс. влияние
align-max-influence-hint = Влияет только поверхность ближе этой дистанции. Большое значение может ухудшить результат
align-orientation-title = Ориентация поверхностей должна совпадать
align-orientation-match = Ориентация поверхностей должна совпадать
align-orientation-inverted = Ориентация поверхностей должна совпадать инверсно
align-orientation-ignored = Ориентация поверхностей игнорируется
align-orientation-either-hint = Принимает любую направленность. Расчёт часто занимает заметно больше времени
align-orientation-facing-hint = Как две поверхности обращены друг к другу
align-exclude = Совмещение: исключить выбранные части
align-exclude-hint = Закрасить поверхность, которую совмещение должно игнорировать
align-commit-cancel = Отмена
align-commit-cancel-hint-moved = Вернуть все сканы на место и закрыть — Ctrl+Z вернёт сопоставление
align-commit-cancel-hint-clean = Закрыть ничего не меняя
align-commit-done = Готово
align-commit-done-hint = Сохранить сопоставление и закрыть — экспортируйте скан для записи на диск

## Deviation map — DRAFT.

align-map-heatmap = Теплокарта
align-map-heatmap-hint = Окрасить один скан по расстоянию до другого
align-map-requires-refine = Сначала выполните точное совмещение
align-map-max = макс
align-map-not-measured = не измерено
align-map-not-measured-hint = На другом скане нет поверхности в досягаемости этих вершин. Зуб или мост только на одном скане — обычная причина, и это не ошибка: измерять там нечего.

## Align roles, brush, mask commands, align status lines — DRAFT.

align-pair-decided = { $moving } → { $fixed }
align-pair-guessed = { $moving } → { $fixed } (предположение)
align-pair-hint-decided = { $moving } двигается, { $fixed } остаётся
align-pair-hint-guessed = Пока кликов нет, инструмент предположил по порядку открытия файлов. Первый клик решает: { $moving } двигается, { $fixed } остаётся
align-pair-swap = Поменять
align-pair-swap-hint = Совместить наоборот — стрелки двигаются вместе

align-brush-title = Кисть
align-brush-close-hint = Закрыть кисть — пометки сохранятся
align-brush-mesh-selection = Выбор сетки
align-brush-moving = Подвижная
align-brush-fixed = Неподвижная
align-brush-size = размер кисти
align-brush-inverse = Инверсия кисти
align-brush-inverse-hint = Простое перетаскивание стирает вместо пометки. Shift инвертирует снова
align-brush-auto-radius = авторадиус
align-brush-auto-radius-hint = Радиус области сетки у каждого конца стрелки
align-brush-size-status = Кисть { $size } мм
align-status-no-summary = Нет сопоставимой поверхности

align-mask-fit-everywhere = Совмещать везде
align-mask-fit-everywhere-hint = Снять все пометки
align-mask-fit-everywhere-report = Пометки сняты — совмещение по всему скану
align-mask-fit-nowhere = Нигде не совмещать
align-mask-fit-nowhere-hint = Пометить всю сетку — совмещение не даст эффекта
align-mask-fit-nowhere-report = Вся сетка помечена — совмещение не даст эффекта
align-mask-invert = Инвертировать пометки
align-mask-invert-hint = Пометить непомеченные области и наоборот
align-mask-invert-report = Пометки инвертированы
align-mask-automatic = Автопометка
align-mask-automatic-hint = Совмещать только по малой области у концов стрелок
align-mask-automatic-report = Совмещение только у концов стрелок

align-status-half-dropped = Незавершённая стрелка убрана
align-status-turned = Пара развёрнута
align-status-cleared = Пара очищена
align-status-click-moving = Кликните точку на скане, который должен двигаться
align-status-click-alternate = Кликайте поочерёдно точки в одинаковых позициях на двух сетках
align-status-two-scans = Два скана в виде — кликните по точке на каждом для пары
align-status-no-surface = У облака точек нет поверхности для пары
align-status-now-other = Теперь кликните соответствующее место на другом скане
align-status-moved = Точка сдвинута
align-status-wrong-scan = Этот скан не из этой пары — нажмите «Очистить» и начните заново
align-status-one-scan = Один из сканов
align-status-place-first = Сначала поставьте по точке на каждом скане
align-status-scaled = Этот скан несёт масштабированное размещение, его нельзя сопоставить
align-status-pose-refused = Подгонка завершена, но скан, для которого она была, уже недоступен
align-status-worker-unavailable = Обработчик совмещения остановился — перезапустите инструмент совмещения
align-status-measure-dropped = Измерение сброшено — цветами владеет кисть пометок
align-status-measure-unavailable = Измерение не применено — скан изменился; снова выполните точное совмещение
align-status-map-elsewhere = Карта расстояний на вкладке «Автоматически» — она вернётся туда
align-status-aligned-points = Совмещено по точкам

## Align result status lines — DRAFT.

align-status-aligned = Совмещено по точкам — выполните точное совмещение для посадки поверхностей.
align-status-refined = Точное совмещение готово
align-status-measured = Теплокарта обновлена
align-status-remeasure = { $reason } — запустите точное совмещение для повторного измерения
align-status-settings-changed = Настройки сопоставления изменены
align-status-visibility-changed = Видимость выбранного скана изменена
align-brush-not-in-alignment = Этот скан не входит в текущее совмещение
align-drag-moving = Перемещение { $name } вручную
align-drag-unrecorded = Перемещено вручную, но шаг не попал в историю — Ctrl+Z не отменит его
align-drag-moved = Перемещение вручную: { $name }, сдвиг { $moved } мм (Ctrl+Z отменяет)
align-status-moved-hand = Перемещено вручную
align-pair-placed = Пара { $n } поставлена
align-roles-swapped = Двигается: { $moving }. Стоит: { $fixed }.
align-status-scan-changed = Скан изменился
align-status-hidden = Слой скрыт: { $name } — покажите его, чтобы совмещать по нему
align-arrow-removed = { $n ->
    [one] Стрелка убрана — осталась { $n } пара
    [few] Стрелка убрана — осталось { $n } пары
    [many] Стрелка убрана — осталось { $n } пар
   *[other] Стрелка убрана — осталось { $n } пар
}
align-status-markings-changed = Пометки изменены
align-status-place-arrow-first = Поставьте хотя бы одну стрелку перед автопометкой
align-status-arrows-cleared = Стрелки убраны — дальше вручную

## Unsaved-work guards and error dialog buttons — DRAFT.

guard-close-title = Несохранённые правки сетки
guard-close-headline-one = 1 изменённый слой не записан на диск.
guard-close-headline-many = Изменённые слои не записаны на диск.
guard-close-note = Затронуто изменённых слоёв: { $count }.
guard-close-detail = Сохранение экспортирует каждый изменённый слой (PLY, STL или OBJ), затем закрывает.
guard-close-destructive = Закрыть без сохранения
guard-replace-title = Идёт редактирование
guard-replace-headline-session = На слое { $layer } активна сессия редактирования.
guard-replace-headline-one = 1 изменённый слой с несохранёнными изменениями.
guard-replace-headline-many = Слоёв с несохранёнными изменениями: { $count }.
guard-replace-detail = Открытие сцены закрывает сессию и отбрасывает несохранённые правки.
guard-replace-destructive = Отбросить и открыть
guard-save = Сохранить…
guard-cancel = Отмена

error-retry-graphics = Попробовать снова
error-close = Закрыть
error-copy-details = Копировать детали

about-website = Сайт
about-source = Исходники
about-licenses = Сторонние лицензии
about-license-kind = Apache License 2.0

## Mesh-edit operations, undo/redo, sculpt, measure, cut ruler, scene menu — DRAFT.

edit-select-faces-first = Сначала выберите грани сетки
edit-no-changes = Без изменений: { $layer }
edit-apply-failed-title = Не удалось изменить выбор
edit-apply-failed-summary = Не удалось изменить выбор: { $detail }
edit-no-changes-hidden = Без изменений: уточните выбор; скрытые слои не тронуты
edit-selected-faces = { $faces ->
    [one] Выбрана { $faces } грань
    [few] Выбраны { $faces } грани
    [many] Выбрано { $faces } граней
   *[other] Выбрано { $faces } граней
}
edit-selected-faces-across = { $faces ->
    [one] Выбрана { $faces } грань в { $layers } слоях
    [few] Выбраны { $faces } грани в { $layers } слоях
    [many] Выбрано { $faces } граней в { $layers } слоях
   *[other] Выбрано { $faces } граней в { $layers } слоях
}

holes-nothing = Нечего закрывать: { $layer }
holes-partial = { $segments }, не закрыто: { $layer }
holes-closed = { $filled ->
    [one] Закрыто { $filled } отверстие
    [few] Закрыты { $filled } отверстия
    [many] Закрыто { $filled } отверстий
   *[other] Закрыто { $filled } отверстий
}
holes-closed-detail = { $closed }: { $layer }
holes-closed-segments = { $closed } ({ $segments }): { $layer }
holes-seg-healed = { $n ->
    [one] Залечена { $n } щербина
    [few] Залечены { $n } щербины
    [many] Залечено { $n } щербин
   *[other] Залечено { $n } щербин
}
holes-seg-border = граница скана оставлена открытой
holes-seg-oversize-limit = { $n ->
    [one] { $n } отверстие больше лимита { $limit } мм
    [few] { $n } отверстия больше лимита { $limit } мм
    [many] { $n } отверстий больше лимита { $limit } мм
   *[other] { $n } отверстий больше лимита { $limit } мм
}
holes-seg-oversize = { $n ->
    [one] { $n } отверстие слишком велико
    [few] { $n } отверстия слишком велики
    [many] { $n } отверстий слишком велики
   *[other] { $n } отверстий слишком велики
}
holes-seg-damaged = { $n ->
    [one] Пропущена { $n } повреждённая кромка
    [few] Пропущены { $n } повреждённые кромки
    [many] Пропущено { $n } повреждённых кромок
   *[other] Пропущено { $n } повреждённых кромок
}
batchedit-close-holes = Закрыты безопасные внутренние отверстия
batchedit-delete = Выбор удалён
batchedit-crop = Обрезано по выбору
batchedit-cut = Выбор вырезан в новый слой
batchedit-separate = Выбор разделён
batchedit-invert = Нормали отражены
batch-close-holes = Закрыты безопасные внутренние отверстия
batch-delete = Выбор удалён
batch-crop = Обрезано по выбору
batch-cut = Выбор вырезан
batch-separate = Выбор разделён
batch-edited = Выбор изменён
batchedit-edited = Слой изменён
edit-applied-status = { $action }: { $layer }
batchedit-status = { $label } на { $n ->
    [one] { $n } видимом слое
    [few] { $n } видимых слоях
    [many] { $n } видимых слоях
   *[other] { $n } видимых слоях
}
batchedit-no-changes = Без изменений: уточните выбор; скрытые слои не тронуты

select-covers-all = Выбор уже покрывает всю сетку: { $layer }
select-covers-remove = Выбор покрывает всю сетку — вместо этого удалите слой: { $layer }
select-splits = Выбор распадается на { $parts } частей — уточните выбор: { $layer }
select-faces-cannot = Нельзя выбрать грани: { $layer }

undo-nothing = Нечего отменять
redo-nothing = Нечего возвращать
undo-undid = Правка сетки отменена: { $layer }
undo-unavailable = Отмена недоступна — сцена изменилась после этого шага: { $layer }
redo-redid = Правка сетки возвращена: { $layer }
redo-unavailable = Возврат недоступен — сцена изменилась после этого шага: { $layer }

sculpt-armed-addremove = Добавить/Удалить: перетаскивайте для наращивания, Shift убирает
sculpt-armed-smooth = Сгладить: перетаскивайте для расслабления, Shift форсирует
sculpt-off = Скульптинг выкл
sculpt-applied-undo = Скульптинг применён (Ctrl+Z отменяет)
sculpt-applied-locked = Скульптинг применён (без отмены: снимок слишком велик)
sculpt-failed-title = Скульптинг не выполнен
sculpt-failed = Скульптинг недоступен для этого слоя: { $detail }
sculpt-worker-stopped = Воркер скульптинга остановлен: { $detail }
sculpt-preparing = Подготовка скульптинга…
sculpt-nonuniform-scale = Скульптинг требует равномерного масштаба сетки
sculpt-failure-worker-panicked = Воркер скульптинга аварийно завершён: { $detail }
sculpt-failure-spawn = Не удалось запустить воркер скульптинга: { $detail }
sculpt-failure-kernel-pool = Не удалось создать пул ядер скульптинга: { $detail }
sculpt-failure-missing-undo-baseline = У штриха скульптинга нет базы для отмены
sculpt-failure-shadow-poisoned = Блокировка тени скульптинга отравлена
sculpt-failure-shadow-shape = Тень скульптинга больше не соответствует рабочей сетке
sculpt-failure-invalid-vertex-index = Воркер скульптинга вернул недопустимый индекс вершины
sculpt-failure-worker-state-poisoned = Состояние воркера скульптинга повреждено — перезапустите Sculpt
sculpt-failure-vertex-count-changed = Результат скульптинга изменил число вершин
sculpt-failure-topology-rebuild = Не удалось восстановить топологию скульптинга: { $detail }
sculpt-worker-unavailable = Воркер скульптинга недоступен
sculpt-finishing = Завершение штриха скульптинга…
sculpt-finishing-history = Завершение скульптинга перед изменением истории…
sculpt-lasso-armed = Лассо включено: клик или перетаскивание обводит; Enter, двойной клик или клик в начало замыкает
sculpt-lasso-off = Лассо выключено
sculpt-object-on = Выбор объекта: кликните объект для выбора целиком
sculpt-object-off = Выбор объекта выкл
sculpt-selection-cleared = Выбор снят
sculpt-through-on = Выбор сквозь сетку
sculpt-through-off = Выбор поверхности

## Session close-outs and layer shortcuts. — DRAFT.
session-applied = Сессия редактирования сетки применена
session-reverted = Сессия редактирования сетки отменена
edit-session-busy = Сначала завершите или отмените сессию редактирования
layers-none-hidden = Нет скрытых слоёв для возврата
layer-opaque-again = Снова непрозрачный: { $label }
layer-translucent = Полупрозрачный: { $label } (Shift+СКМ возвращает)
layer-restored = Снова видимый: { $label }
layer-hidden = Скрыт: { $label } (Shift+Ctrl+СКМ возвращает)
layer-unnamed = слой { $n }
layer-removed = Слой удалён: { $label }
layer-face-selection = Выбор граней: { $label }

measure-distance = Расстояние: { $len }
measure-thickness = Толщина стенки: { $len }
measure-open-wall = Открытая поверхность: нет встречной стенки вдоль внутренней нормали
measure-cannot-probe = Здесь нельзя измерить: вырожденная геометрия
measure-cleared = Измерения очищены

cut-lines = Линии
cut-mesh = Сетка
cut-dist = Расст
cut-dist-hint = Расстояние: клик по двум точкам
cut-thick = Толщ
cut-thick-hint = Толщина стенки: клик по точке контура
cut-close-section = Закрыть сечение
cut-snap = Привязка
cut-snap-hint = Магнит: клики прилипают к контуру сечения
cut-empty = Нет пересечения
cut-footer-distance = Перетаскивание = панорама · клик 2 т. = расстояние · ПКМ очистить · колесо = масштаб
cut-footer-thickness = Перетаскивание = панорама · клик контур = толщина · ПКМ очистить · колесо = масштаб

## Worker-built align failures — DRAFT.

align-fail-no-surface-fixed = У неподвижного скана нет пригодной поверхности
align-fail-no-surface-moving = У подвижного скана нет пригодной поверхности
align-fail-recolor = Измерение сброшено до окраски
align-fail-unobservable = Поверхность недостаточно наблюдаема для надёжной теплокарты
align-reject-toofew = Поставьте больше пар стрелок или приблизьте сканы
align-reject-unpaired = Завершите обе стороны каждой пары стрелок
align-reject-degenerate-plain = Разнесите точки сопоставления по поверхности
align-reject-unit = Сканы используют разные единицы измерения
align-reject-apart = Проверьте пары стрелок и приблизьте сканы
align-reject-runaway = Приблизьте сканы и повторите точное совмещение
align-reject-no-improvement = Улучшение не подтверждено — приблизьте сканы и повторите
align-reject-ambiguous = Найдено несколько одинаково вероятных поверхностей — отметьте нужную область или приблизьте сканы
align-reject-nonfinite = Выбранная точка или поверхность недействительны
align-status-stepped = Прошлись по истории
align-status-moving-hand = Двигаем вручную

## Bridge split panel and align session close-outs — DRAFT.

bridge-panel-title = Разделение моста
bridge-mode-place = Установите диск
bridge-mode-calculating = Расчёт…
bridge-mode-ready = Готово
bridge-mode-failed = Попытка разделения не удалась
bridge-kerf = Зазор
bridge-disc-size = Размер диска
bridge-cancel = Отмена
bridge-apply = Разделить мост
bridge-err-miss = Диск мимо моста. Сдвиньте его в коннектор.
bridge-err-tangent = Диск лишь касается поверхности. Проведите его сквозь коннектор.
bridge-err-small = Диаметр диска { $have } мм; здесь нужно минимум { $need } мм.
bridge-err-limit = Резу нужен диск { $need } мм, выше безопасного лимита { $max } мм.
bridge-err-no-result = Разделение попытались выполнить с сохранением исходной поверхности, но пригодного результата нет. Исходный меш сохранён.
bridge-err-invalid-cut = Разделение попытались выполнить, но рез не прошёл проверку. Исходный меш сохранён.
bridge-err-invalid-side = Разделение попытались выполнить, но { $side } не прошла проверку. Исходный меш сохранён.
bridge-err-gap = Разделение попытались выполнить, но запрошенный зазор не сохранён. Исходный меш сохранён.
bridge-err-empty = У выбранного слоя нет треугольной сетки для разделения.
bridge-err-invalid = Настройки диска неверны. Сбросьте инструмент и попробуйте снова.
bridge-err-unusable = Разделение не дало пригодного результата. Исходный меш сохранён.

align-session-canceled = Сопоставление отменено — все сканы вернулись на место (Ctrl+Z вернёт обратно)
align-session-closed = Сопоставление закрыто
align-session-closed-running = Сопоставление закрыто — подгонка ещё шла и была сброшена, сканы как вы их видели
align-session-kept = Сопоставление сохранено — сохраните скан для записи на диск

recent-clear = Очистить недавние

scene-already-origin = Все слои уже в исходной позиции
scene-positions-reset = Позиции слоёв сброшены (Ctrl+Z отменяет)

## Settings panel, bridge split, render error, tint — DRAFT.

settings-header = Настройки
settings-section-files = Файлы и экспорт
settings-export-format = Запасной формат экспорта
settings-export-format-hint = Используется, когда исходный формат нельзя экспортировать
settings-remember-export = Запоминать папку экспорта
settings-remember-export-hint = Использовать ту же папку после перезапуска OccluView
settings-section-scene = Вид и навигация
settings-frame-on-open = Вписывать сцену при открытии
settings-frame-on-open-hint = Возвращать камеру к домашнему виду, когда новый файл заменяет сцену
settings-double-click = Двойной клик перецентрирует вид
settings-double-click-hint = Двойной клик возвращает камеру к выбранной точке
settings-orbit = Скорость вращения
settings-orbit-hint = Как быстро вид вращается при перетаскивании правой кнопкой
settings-zoom = Скорость масштаба
settings-zoom-hint = Насколько каждое деление колеса приближает
settings-background = Фон
settings-bg-gray = Серый
settings-bg-white = Белый
settings-bg-dark = Тёмный
settings-ghost = Призрак отрезанной стороны
settings-ghost-hint = В сечении показывать удалённую сторону полупрозрачным призраком
settings-measurements = Измерения
settings-section-appearance = Внешний вид
settings-theme = Тема
settings-theme-light = Светлая
settings-theme-dark = Тёмная
settings-scale = Масштаб интерфейса
settings-scale-hint = Масштабирует все элементы; 1.0 оставляет системный
settings-section-mesh = Редактирование сетки
settings-remember-brush = Запоминать кисть скульптинга
settings-remember-brush-hint = Хранить ползунки размера и силы между сессиями
settings-section-updates = Обновления
settings-check-auto = Проверять при запуске
settings-check-now = Проверить
settings-check-disabled-hint = Проверка обновлений отключена окружением
settings-check-busy-hint = Проверка обновления уже идёт
settings-update-disabled = Отключено окружением
settings-update-checking = Проверка…
settings-update-current = Актуально
settings-update-skipped = Версия пропущена
settings-update-failed = Не удалось проверить
settings-save-error = Не удалось сохранить настройки. Повторная попытка…
settings-save-error-hint = Файл настроек сейчас недоступен
settings-shortcuts = Горячие клавиши
settings-about = Об OccluView

bridge-busy = Сначала завершите или отмените разделение моста
bridge-active = Разделение моста уже активно
bridge-target-gone = Цель разделения моста больше недоступна
bridge-needs-mesh = Для разделения моста нужен видимый треугольный меш
bridge-place-disc = Разделение моста: установите диск-разделитель
bridge-canceled-scene = Разделение моста отменено: сцена закрыта
bridge-canceled-camera = Разделение моста отменено: камера недоступна
bridge-canceled-changed = Разделение моста отменено: исходный меш изменился
bridge-canceled = Разделение моста отменено
bridge-calculating = Разделение моста: расчёт…
bridge-unavailable = Разделение моста временно недоступно
bridge-preview-stale = Предпросмотр разделения устарел
bridge-not-applied = Разделение моста не применено
bridge-complete = Разделение моста завершено
bridge-complete-surface = Разделение моста завершено (поверхностный результат; естественные границы сохранены)
bridge-complete-locked = Разделение моста завершено (без отмены: снимок слишком велик)

render-failed-title = Не удалось отрисовать сцену
render-failed-summary = Файл открыт, но вьюпорт отрисовать не удалось.
render-failed-status = Отрисовка не удалась

tint-choose = Выбрать оттенок

## Status tail: brush, lasso, loading, GPU, align jobs. — DRAFT.
brush-no-mesh = Кликните по точке на каждой сетке, затем красьте на любой
lasso-dropped = Контур лассо сброшен
lasso-needs-points = Лассо нужно минимум 3 точки
loading-scene = Загрузка сцены…
gpu-failed-status = Драйвер видеокарты сообщил о проблеме
gpu-retry-status = Повтор графики — если проблема остаётся, сохраните работу и перезапустите OccluView
gpu-failed-title = Проблема графики
gpu-failed-summary = Драйвер видеокарты сообщил о проблеме при отрисовке. Вид может быть неполным. Сохраните работу и перезапустите OccluView, если повторится.
align-job-align = Сопоставление…
align-job-refine = Уточнение…
align-job-measure = Измерение…
align-markings-dropped = Пометки сброшены — поверхность скана изменилась после закраски

## Окклюзионные контакты: правый клик по скану — и видно, где он смыкается со
## встречным сканом. Одно чтение — артикуляционная бумага (только отпечатки,
## окрашенные по глубине), другое — карта сближения (насколько близко, везде).
## Один ползунок задаёт глубину, которую шкала считает полной нагрузкой, и он
## перекрашивает уже измеренное поле, а не измеряет заново.
layer-menu-contacts = Показать контакты
layer-menu-hide-contacts = Скрыть контакты

contact-title = Окклюзионные контакты
contact-close-hint = Закрыть чтение и снять отметки с обоих сканов
contact-against = { $subject } относительно { $antagonist }
contact-unknown-layer = скан, который больше не открыт

contact-mode-marks = Контакты
contact-mode-marks-hint = Где поверхности смыкаются, с цветом по силе — остальное остаётся чистым, как после артикуляционной бумаги
contact-mode-approach = Сближение
contact-mode-approach-hint = Насколько близко встречный скан везде, включая нагрузку

contact-load-label = нагрузка при
contact-load-suffix = мм
contact-load-hint = Глубина, при которой шкала читается как полная нагрузка. Сдвиг перекрашивает уже измеренную карту — без повторного измерения.
contact-flatten = Один цвет на контакт
contact-flatten-hint = Свести каждый отпечаток контакта к его самой глубокой точке. Выключено — сохраняется распределение силы внутри отпечатка.

contact-legend-deepest = { $mm } мм в смыкание

contact-stats-area = Площадь контакта
contact-stats-contacts = Контакты
contact-stats-deepest = Самая глубокая

contact-readout-gap = зазор
contact-readout-load = нагрузка

contact-status-measuring = Измерение…
contact-status-measuring-hint = Поверхности читаются одна относительно другой
contact-status-remeasuring = Повторное измерение…
contact-status-remeasuring-hint = Скан переместился, расстояния изменились. Карта читается заново.
contact-status-needs-second = Для чтения контактов нужен второй видимый скан
contact-status-no-surface = У одного из сканов нет поверхности для измерения
contact-status-worker-failed = Измерение не завершилось

contact-opened = Чтение контактов на { $label }
contact-closed = Чтение контактов закрыто
help-section-contacts = Окклюзионные контакты
help-hintline-contacts = Правый клик по слою · Показать контакты · ползунок «нагрузка при» перекрашивает карту · Esc закрывает
help-hint-contacts-read-its-occlusal-contacts-against-the-scan-it-bites = Прочитать окклюзионные контакты относительно встречного скана
help-hint-contacts-read-the-contact-depth-under-the-cursor = Прочитать глубину контакта под курсором, на любой из челюстей
help-hint-contacts-move-the-depth-the-ramp-calls-fully-loaded = Сдвинуть глубину, которую шкала считает полной нагрузкой
help-hint-contacts-switch-between-marks-only-and-the-whole-approach = Переключить между только отпечатками и всей зоной сближения
help-hint-contacts-close-the-reading-and-take-the-marks-off-both-scans = Закрыть чтение и снять отметки с обоих сканов
contact-retry = Прочитать заново
contact-status-subject-unusable = Скан, о котором идёт чтение, сейчас нельзя измерить
contact-status-subject-unusable-hint = Покажите его снова или оставьте полигональной сеткой — чтение возобновится
contact-status-antagonist-unusable = Встречный скан, относительно которого идёт измерение, сейчас нельзя измерить
contact-status-antagonist-unusable-hint = Покажите его снова или оставьте полигональной сеткой — чтение возобновится
contact-status-no-overlap = Сканы слишком далеко друг от друга
contact-status-no-overlap-hint = Ни одна из поверхностей не попала в зону чтения. Проверьте, что сканы стоят в окклюзии.
contact-status-failed-hint = Прочитать заново; если снова не удаётся, пару, возможно, нужно сначала отремонтировать.
contact-status-needs-second-hint = Откройте встречный скан или покажите его снова и начните чтение
contact-legend-gap = зазор до { $mm } мм
contact-stats-balance = Площадь по сторонам
contact-stats-balance-hint = Площадь контакта по обе стороны от собственной средней линии скана. Деление идёт по координатам скана, поэтому при повороте или зеркале числа могут поменяться местами.
layer-menu-contacts-unavailable = Для чтения контактов нужны два видимых полигональных скана — сначала покажите или откройте встречный

contact-details = Подробности
contact-details-hint = Числа и правило «один цвет на контакт»
contact-details-close = Скрыть подробности
settings-shortcuts-hint = Справка по клавиатуре и мыши (F1)

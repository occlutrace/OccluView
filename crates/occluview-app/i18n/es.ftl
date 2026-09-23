## OccluView Spanish catalog — DRAFT (machine draft, native review required).
## Status: DRAFT. Requires native dental/CAD terminology review + visual UI review before APPROVED.
## Contract: exact key/attribute/variable parity with en.ftl.

app-window-title = OccluView 3D Viewer
align-panel-title = Alinear escaneos
meshedit-window-title = Edición de mallas

settings-language-label = Idioma
settings-language-auto = Idioma del sistema
settings-language-auto-current = Idioma del sistema — { $language }
settings-language-catalog-fallback = { $tag } no está disponible; se usa inglés.
settings-language-save-error = No se pudo guardar el idioma. Reintentando…

about-title = Acerca de OccluView
about-tagline = Reparación de mallas · Edición de mallas para CAD dental
about-version = Versión { $version }

update-available-title = Actualización disponible
update-available-body = La versión { $version } está lista para instalar.
update-current-version = Tienes la { $version }.
update-download = Descargar actualización
update-open-release = Abrir la página de la versión
update-later = Más tarde
update-skip = Omitir esta versión
update-skip-tooltip = No volver a ofrecer esta versión; se ofrecerá la siguiente
update-downloading = Descargando OccluView { $version }
update-ready-title = OccluView { $version } está listo para instalar
update-ready-hint-windows = El instalador se verificó. OccluView se cerrará mientras Windows aplica la actualización.
update-ready-hint-other = El paquete se verificó. Se abrirá el instalador del sistema — confirma allí.
update-install-close = Instalar y cerrar
update-failed-title = Falló la actualización
update-dismiss = Descartar

error-open-title = No se puede abrir el archivo
error-add-title = No se puede añadir el archivo

## Help surface — DRAFT. Gesture names stay invariant by contract.

help-title = Controles de teclado y ratón
help-subtitle = La referencia corresponde a los controles disponibles en OccluView.
help-close = Cerrar

help-section-navigation = Navegación
help-section-tools = Herramientas
help-section-mesh-editing = Edición de mallas
help-section-sculpt = Esculpido
help-section-align-measure = Alineación y medición
help-section-cut-view = Vista de corte
help-section-layers-preview = Capas y vista previa del Explorador

help-hintline-navigation = Arrastrar con BRM orbita · MMB desplaza · rueda zoom · clic MMB enfoca
help-hintline-mesh-editing = Clic IZM selecciona · Shift+clic desmarca · rectángulo · Ctrl+Z deshace
help-hintline-sculpt = IZM esculpe · Shift cambia modo · Shift+rueda tamaño · Ctrl+rueda fuerza
help-hintline-align = IZM coloca · Ctrl/Command+arrastrar rota · Shift+arrastrar borra · BRM deshace
help-hintline-cut = IZM planta o mueve · Ctrl+rueda en Sección redimensiona · F invierte · Esc cierra
help-hintline-measure = IZM mide · BRM limpia · rueda zoom · Esc cierra

help-hint-navigation-orbit-the-camera = Orbitar la cámara
help-hint-navigation-pan-the-camera = Desplazar la cámara
help-hint-navigation-pan-the-camera-2 = Desplazar la cámara
help-hint-navigation-zoom-toward-the-pointer = Zoom hacia el puntero
help-hint-navigation-recenter-on-the-surface = Recentrar en la superficie
help-hint-navigation-recenter-on-the-surface-when-enabled = Recentrar en la superficie si está activado
help-hint-navigation-open-the-layer-or-scene-menu-when-stationary = Abrir el menú de capa o escena con clic quieto
help-hint-tools-open-a-file = Abrir un archivo
help-hint-tools-open-cut-view = Abrir vista de corte
help-hint-tools-arm-the-ruler = Activar la regla
help-hint-tools-arm-thickness = Activar grosor
help-hint-tools-open-align = Abrir alineación
help-hint-tools-open-mesh-editing = Abrir edición de mallas
help-hint-mesh-editing-select-a-face = Seleccionar una cara
help-hint-mesh-editing-unmark-a-face-or-screen-selection = Desmarcar una cara o la selección
help-hint-mesh-editing-select-faces-in-a-screen-rectangle = Seleccionar caras en un rectángulo
help-hint-mesh-editing-draw-a-freehand-selection-outline = Dibujar un contorno libre
help-hint-mesh-editing-close-and-apply-a-lasso-outline = Cerrar y aplicar el lazo
help-hint-mesh-editing-cancel-the-active-lasso-outline = Cancelar el lazo activo
help-hint-mesh-editing-select-all-visible-faces = Seleccionar todas las caras visibles
help-hint-mesh-editing-delete-selected-faces = Eliminar las caras seleccionadas
help-hint-mesh-editing-undo-the-last-mesh-edit = Deshacer la última edición
help-hint-mesh-editing-redo-the-last-mesh-edit = Rehacer la última edición
help-hint-sculpt-choose-add-remove = Elegir añadir/quitar
help-hint-sculpt-choose-smooth = Elegir suavizar
help-hint-sculpt-sculpt-under-the-brush = Esculpir bajo el pincel
help-hint-sculpt-remove-or-strengthen-the-active-brush-mode = Quitar o reforzar el modo activo
help-hint-sculpt-change-brush-size = Cambiar el tamaño del pincel
help-hint-sculpt-change-brush-intensity = Cambiar la fuerza del pincel
help-hint-align-measure-place-an-alignment-point-or-measurement-point = Colocar un punto de alineación o medición
help-hint-align-measure-rotate-a-scan-in-align-s-manual-mode = Rotar un escaneo en modo manual
help-hint-align-measure-erase-an-align-exclusion-region = Borrar una región de exclusión
help-hint-align-measure-change-align-exclusion-brush-size = Cambiar el tamaño del pincel de exclusión
help-hint-align-measure-undo-the-last-alignment-point-when-stationary = Deshacer el último punto con clic quieto
help-hint-align-measure-clear-measurements-when-stationary = Limpiar mediciones con clic quieto
help-hint-align-measure-close-the-active-measurement-tool = Cerrar la herramienta de medición
help-hint-cut-view-plant-or-move-the-cut-disc = Plantar o mover el disco de corte
help-hint-cut-view-change-disc-size = Cambiar el tamaño del disco
help-hint-cut-view-zoom-the-section-view = Zoom de la vista de sección
help-hint-cut-view-flip-the-kept-half-while-planted = Invertir la mitad conservada
help-hint-cut-view-unplant-the-disc-or-close-cut-view = Quitar el disco o cerrar la vista
help-hint-layers-preview-hide-the-layer-under-the-pointer = Ocultar la capa bajo el puntero
help-hint-layers-preview-restore-the-last-hidden-layer = Restaurar la última capa oculta
help-hint-layers-preview-toggle-layer-translucency = Alternar la translucidez
help-hint-layers-preview-orbit-the-preview-model = Orbitar el modelo
help-hint-layers-preview-zoom-the-preview-model = Zoom del modelo
help-hint-layers-preview-frame-the-preview-model = Encuadrar el modelo
help-hint-layers-preview-toggle-preview-wireframe = Alternar malla de alambre

## Toolbar, empty state — DRAFT.

toolbar-open-label = Abrir
toolbar-open-hint = Abrir archivos 3D ({ $shortcut })
toolbar-recent-hint = Archivos recientes
toolbar-add-label = Añadir
toolbar-add-hint = Añadir archivos a la escena actual
toolbar-cut-label = Vista de corte
toolbar-cut-hint = Cortar el modelo por un plano ({ $shortcut })
toolbar-cut-unavailable = La vista de corte necesita una capa visible
toolbar-ruler-label = Regla
toolbar-ruler-hint = Medir una distancia: dos puntos en el modelo ({ $shortcut })
toolbar-thickness-label = Grosor
toolbar-thickness-hint = Sondear el grosor: un punto en la pared ({ $shortcut })
toolbar-measure-blocked = Termina o cancela la sesión de edición primero
toolbar-measure-needs-layer = Medir necesita una capa de malla visible
toolbar-align-label = Alinear
toolbar-align-hint = Juntar dos escaneos: un punto en cada uno ({ $shortcut })
toolbar-edit-label = Editar
toolbar-edit-open = Edición de mallas abierta
toolbar-edit-hint = Edición de mallas: selección y esculpido ({ $shortcut })
toolbar-settings-label = Ajustes
toolbar-settings-hint = Abrir preferencias

empty-open-file = Abrir un archivo 3D
empty-formats-hint = STL · PLY · OBJ · GLB · HPS — o suelta archivos aquí

## Loading and export — DRAFT.

load-queued = { $count ->
    [one] { $count } capa en cola
   *[other] { $count } capas en cola
}
load-opening = { $count ->
    [one] Abriendo { $count } archivo…
   *[other] Abriendo { $count } archivos…
}
load-adding = { $count ->
    [one] Añadiendo { $count } archivo…
   *[other] Añadiendo { $count } archivos…
}
load-open-failed-start = Falló la apertura: no arranca el cargador
load-add-failed-start = Falló la adición: no arranca el cargador
load-open-failed-stopped = Falló la apertura: el cargador se detuvo
load-loader-failed-summary = No se pudo iniciar el cargador de escena en segundo plano.
load-file-too-large = El archivo ocupa { $size } GB, por encima de los { $limit } GB que se leen de una vez
load-action-failed-open = Falló la apertura: { $detail }
load-action-failed-add = Falló la adición: { $detail }

export-nothing-visible = Nada visible que guardar
export-unsupported-format = Formato de salida no soportado
export-scene-saved = Escena guardada: { $path }
export-scene-saved-unmerged = Escena guardada (texturas no fusionadas): { $path }
export-scene-failed-title = No se pudo guardar la escena
export-scene-failed-summary = No se pudo guardar la escena: { $detail }
export-layers-saved = { $written ->
    [one] Guardada { $written } capa en { $dir }
   *[other] Guardadas { $written } capas en { $dir }
}
export-layers-saved-failed = { $written ->
    [one] Guardada { $written } capa en { $dir }
   *[other] Guardadas { $written } capas en { $dir }
}; { $failed ->
    [one] { $failed } no se pudo escribir
   *[other] { $failed } no se pudieron escribir
}
export-layers-saved-renamed = { $written ->
    [one] Guardada { $written } capa en { $dir }
   *[other] Guardadas { $written } capas en { $dir }
}; { $renamed ->
    [one] { $renamed } archivo renombrado para conservar lo existente
   *[other] { $renamed } archivos renombrados para conservar lo existente
}
export-layers-saved-failed-renamed = { $written ->
    [one] Guardada { $written } capa en { $dir }
   *[other] Guardadas { $written } capas en { $dir }
}; { $failed ->
    [one] { $failed } no se pudo escribir
   *[other] { $failed } no se pudieron escribir
}; { $renamed ->
    [one] { $renamed } archivo renombrado para conservar lo existente
   *[other] { $renamed } archivos renombrados para conservar lo existente
}
mesh-exported-aligned = { $name } exportado en su posición alineada como { $format }: { $path }
mesh-exported-aligned-warnings = { $name } exportado en su posición alineada como { $format } (avisos: { $warnings }): { $path }
mesh-exported-unmoved = { $name } exportado (sin mover) como { $format }: { $path }
mesh-exported-unmoved-warnings = { $name } exportado (sin mover) como { $format } (avisos: { $warnings }): { $path }
mesh-warning-vertex-colors = colores de vértice no incluidos
mesh-warning-uvs = UV no incluidos
mesh-warning-texture-image = imagen de textura no incluida
mesh-export-warnings = Advertencias de exportación: { $warnings }
mesh-export-failed-title = No se pudo exportar la capa
mesh-export-failed-summary = No se pudo exportar la capa: { $detail }

## Repair card and toasts — DRAFT.

repair-title = Reparación de mallas
repair-clean-headline = Nada que reparar — malla limpia
repair-copy-details = Copiar detalles
repair-copy-tooltip = Copiar el informe completo al portapapeles
repair-line-welded = { $count ->
    [one] Soldado { $grouped } vértice duplicado
   *[other] Soldados { $grouped } vértices duplicados
}
repair-line-slivers = { $count ->
    [one] Eliminada { $grouped } cara degenerada
   *[other] Eliminadas { $grouped } caras degeneradas
}
repair-line-duplicate-faces = { $count ->
    [one] Eliminada { $grouped } cara duplicada
   *[other] Eliminadas { $grouped } caras duplicadas
}
repair-line-nonmanifold = { $count ->
    [one] Arreglada { $grouped } arista no manifold
   *[other] Arregladas { $grouped } aristas no manifold
}
repair-line-bowtie = { $count ->
    [one] Dividido { $grouped } vértice bowtie
   *[other] Divididos { $grouped } vértices bowtie
}
repair-line-reoriented = { $count ->
    [one] Reorientado { $grouped } triángulo
   *[other] Reorientados { $grouped } triángulos
}
repair-line-flipped = { $count ->
    [one] Volteada { $grouped } parte invertida
   *[other] Volteadas { $grouped } partes invertidas
}
repair-line-debris = { $count ->
    [one] Eliminada { $grouped } parte residual
   *[other] Eliminadas { $grouped } partes residuales
}
repair-line-pinholes = { $count ->
    [one] Cerrado { $grouped } poro
   *[other] Cerrados { $grouped } poros
}
repair-line-unused = { $count ->
    [one] Eliminado { $grouped } vértice sin uso
   *[other] Eliminados { $grouped } vértices sin uso
}
repair-open-rims = { $count ->
    [one] { $grouped } borde abierto (límite del escaneo)
   *[other] { $grouped } bordes abiertos (límite del escaneo)
}
repair-skipped-rims = { $count ->
    [one] { $grouped } borde sin rellenar (no simple)
   *[other] { $grouped } bordes sin rellenar (no simples)
}
repair-toast-welded = { $count ->
    [one] soldado { $count } vértice
   *[other] soldados { $count } vértices
}
repair-toast-slivers = { $count ->
    [one] eliminada { $count } degenerada
   *[other] eliminadas { $count } degeneradas
}
repair-toast-duplicate-faces = { $count ->
    [one] { $count } cara duplicada
   *[other] { $count } caras duplicadas
}
repair-toast-nonmanifold = { $count ->
    [one] arreglada { $count } arista no manifold
   *[other] arregladas { $count } aristas no manifold
}
repair-toast-bowtie = { $count ->
    [one] dividido { $count } bowtie
   *[other] divididos { $count } bowties
}
repair-toast-reoriented = { $count ->
    [one] reorientado { $count } triángulo
   *[other] reorientados { $count } triángulos
}
repair-toast-flipped = { $count ->
    [one] volteada { $count } parte invertida
   *[other] volteadas { $count } partes invertidas
}
repair-toast-debris = { $count ->
    [one] eliminada { $count } parte residual
   *[other] eliminadas { $count } partes residuales
}
repair-toast-pinholes = { $count ->
    [one] cerrado { $count } poro
   *[other] cerrados { $count } poros
}
repair-toast-unused = { $count ->
    [one] eliminado { $count } vértice sin uso
   *[other] eliminados { $count } vértices sin uso
}
repair-toast-skipped = { $count ->
    [one] { $count } borde omitido (no simple)
   *[other] { $count } bordes omitidos (no simples)
}
repair-toast-done = Reparado { $layer }: { $parts }
repair-toast-clean-rims = Malla ya limpia: { $layer }, { $count ->
    [one] { $count } borde abierto
   *[other] { $count } bordes abiertos
}
repair-toast-clean = Malla ya limpia: { $layer }
repair-edit-busy = Edición de capa en curso
repair-edit-failed-title = No se pudo editar la capa
repair-edit-failed-summary = No se pudo editar la capa: { $detail }
edit-locked-status = { $status } (sin deshacer: instantánea demasiado grande)

## Layers overlay, layer menu, scene menu — DRAFT.

layers-title = Capas
layers-count = { $count ->
    [one] { $count } capa
   *[other] { $count } capas
}
layers-row-hide = Ocultar capa
layers-row-show = Mostrar capa
layers-row-opacity = Opacidad de capa
layers-row-remove = Eliminar capa

layer-menu-next-tint = Siguiente tinte
layer-menu-hide-colors = Ocultar colores del escaneo
layer-menu-show-colors = Mostrar colores del escaneo
layer-menu-disable-texture = Desactivar textura
layer-menu-show-texture = Mostrar textura
layer-menu-mesh-editing = Edición de mallas
layer-menu-split-bridge = Dividir puente…
layer-menu-repair = Reparación de mallas
layer-menu-flip-normals = Invertir normales
layer-menu-export = Exportar capa…
layer-menu-hide-wireframe = Ocultar alambre
layer-menu-show-wireframe = Alambre superpuesto
layer-menu-remove = Eliminar capa

scene-menu-title = Escena
scene-menu-save = Guardar escena como…
scene-menu-save-each = Guardar cada capa…
scene-menu-reset = Restablecer posiciones
scene-menu-fit = Encuadrar vista

## Mesh editor palette — DRAFT.

meshedit-tab-edit = Edición de mallas
meshedit-tab-sculpt = Esculpido
meshedit-cancel-session = Cancelar la sesión (se revierten las ediciones)
meshedit-header-edit = Edición de mallas
meshedit-section-selection = Selección
meshedit-section-edit-selection = Editar selección
meshedit-section-close-holes = Cerrar agujeros
meshedit-section-sculpt = Esculpido
meshedit-cell-lasso = Lazo
meshedit-cell-lasso-hint = Contorno libre: clic coloca puntos, doble clic cierra · Shift desmarca
meshedit-cell-object = Objeto
meshedit-cell-object-hint = Clic en un objeto entero de un STL multipartes · Shift desmarca
meshedit-cell-surface = Superficie
meshedit-cell-surface-hint = Marcar solo la superficie frontal visible
meshedit-cell-through = A través
meshedit-cell-through-hint = Marcar a través de la malla, incluidos reversos ocultos
meshedit-cell-all = Todo
meshedit-cell-all-hint = Marcar todas las caras (Ctrl+A)
meshedit-cell-none = Nada
meshedit-cell-none-hint = Limpiar la marca
meshedit-cell-invert = Invertir
meshedit-cell-invert-hint = Intercambiar marcadas y sin marcar
meshedit-cell-delete = Eliminar
meshedit-cell-delete-hint = Eliminar las caras marcadas
meshedit-cell-crop = Recortar
meshedit-cell-crop-hint = Conservar solo el área marcada, quitar el resto
meshedit-cell-cut = Cortar
meshedit-cell-cut-hint = Mover las caras marcadas a una malla nueva — la original queda
meshedit-cell-separate = Separar
meshedit-cell-separate-hint = Dividir la región en una malla por parte conexa
meshedit-cell-close-holes = Cerrar agujeros
meshedit-cell-close-holes-hint = Cerrar agujeros solo con las caras vecinas marcadas. Los bordes quedan abiertos.
meshedit-sculpt-addremove = Añadir / Quitar  [1]
meshedit-sculpt-addremove-hint = Aportar material arrastrando; Shift excava. Shift+rueda redimensiona, Ctrl+rueda cambia fuerza. Tecla: 1.
meshedit-sculpt-smooth = Suavizar  [2]
meshedit-sculpt-smooth-hint = Relajar la superficie arrastrando; Shift fuerza el máximo. Shift+rueda redimensiona, Ctrl+rueda cambia fuerza. Tecla: 2.
meshedit-slider-size = tamaño
meshedit-slider-size-hint = Tamaño del pincel (Shift + rueda)
meshedit-slider-force = fuerza
meshedit-slider-force-hint = Fuerza del pincel (Ctrl + rueda)
meshedit-limit-label = límite
meshedit-limit-checkbox-hint = Limitar la reparación a bordes menores que este perímetro
meshedit-limit-drag-hint = Off cierra todo agujero seguro del área; el borde queda abierto
meshedit-status-unsaved = Ediciones sin guardar
meshedit-status-unsaved-hint = Sin confirmar: Aceptar aplica, Cancelar revierte
meshedit-status-hint-sculpt = Arrastra sobre la superficie para esculpir · BRM orbita
meshedit-status-hint-object = Clic en un objeto para elegirlo entero · Shift desmarca
meshedit-status-hint-lasso = Clic perfila · doble clic cierra · Shift desmarca
meshedit-status-hint-default = Arrastra un cuadro para marcar · Shift desmarca · Supr borra
meshedit-session-undo = Deshacer
meshedit-session-undo-hint = Deshacer la última edición (Ctrl+Z)
meshedit-session-redo = Rehacer
meshedit-session-redo-hint = Rehacer la edición deshecha (Ctrl+Y)
meshedit-session-cancel = Cancelar
meshedit-session-cancel-hint = Descartar todas las ediciones de la sesión
meshedit-session-done = Aceptar
meshedit-session-done-hint = Aplicar las ediciones y cerrar el editor

## Align Scans window — DRAFT.

align-title = Alinear escaneos
align-tab-auto = Alinear
align-tab-manual = Ajustar posición
align-constraint-free = Mover/rotar en todas direcciones
align-constraint-free-hint = Arrastra el escaneo en cualquier dirección
align-constraint-z = Mover en dirección z
align-constraint-z-hint = Arrastrar solo en vertical
align-constraint-xy = Mover en plano xy
align-constraint-xy-hint = Arrastrar solo en horizontal
align-manual-drag-hint = Mueve el escaneo agarrado · Ctrl+arrastrar rota
align-undo = Deshacer
align-undo-hint = Un paso atrás
align-redo = Rehacer
align-redo-hint = Un paso adelante
align-prompt-moving = Clic en un punto de la malla que debe moverse
align-prompt-other = Clic en la misma posición de la otra malla
align-prompt-alternate = Clic alternando puntos en las mismas posiciones de ambas mallas
align-prompt-placed = { $count ->
    [one] { $count } flecha colocada
   *[other] { $count } flechas colocadas
}
align-back = Atrás
align-back-hint = Deshacer una flecha — clic derecho hace lo mismo
align-clear = Limpiar
align-clear-hint = Soltar todas las flechas y elegir dos escaneos — quedan donde están
align-fit-perform = Realizar alineación
align-fit-perform-hint = Mover la malla a las flechas — mínimo dos flechas
align-fit-refine = Ajuste fino
align-fit-refine-hint = Alinear las zonas sin cambios del escaneo preparado con el modelo original. Revisa el resultado antes de aceptarlo
align-matching-parts = partes coincidentes
align-matching-parts-hint = Proporción máxima de correspondencias para el refinamiento. Best Fit la reduce si quedan pocas zonas sin cambios
align-max-influence = influencia máx.
align-max-influence-hint = Solo influye la superficie bajo esta distancia. Un valor alto puede empeorar
align-orientation-title = La orientación debe coincidir
align-orientation-match = La orientación debe coincidir
align-orientation-inverted = La orientación debe coincidir invertida
align-orientation-ignored = Se ignora la orientación
align-orientation-either-hint = Acepta ambas caras. El cálculo suele tardar más
align-orientation-facing-hint = Cómo se enfrentan ambas superficies
align-exclude = Ajuste: excluir partes marcadas
align-exclude-hint = Pintar la superficie que el ajuste debe ignorar
align-commit-cancel = Cancelar
align-commit-cancel-hint-moved = Devolver todo y cerrar — Ctrl+Z trae la alineación
align-commit-cancel-hint-clean = Cerrar sin cambiar nada
align-commit-done = Aceptar
align-commit-done-hint = Conservar la alineación y cerrar — exporta para guardar

## Deviation map — DRAFT.

align-map-heatmap = Mapa de calor
align-map-heatmap-hint = Colorear un escaneo por su distancia al otro
align-map-requires-refine = Ejecuta primero Best fit matching
align-map-max = máx
align-map-min = mín
align-map-not-measured = no medido
align-map-not-measured-hint = Ninguna superficie del otro escaneo al alcance de estos vértices. Un diente o puente en un solo escaneo es lo normal, no un error — no hay nada que medir.

## Align roles, brush, mask commands, align status lines — DRAFT.

align-pair-decided = { $moving } → { $fixed }
align-pair-guessed = { $moving } → { $fixed } (supuesto)
align-pair-hint-decided = { $moving } se mueve, { $fixed } queda
align-pair-hint-guessed = Sin clics, supuesto por orden de apertura. Tu primer clic decide: { $moving } se mueve, { $fixed } queda
align-pair-swap = Intercambiar
align-pair-swap-hint = Ajustar al revés — las flechas van con él

align-brush-title = Pincel
align-brush-close-hint = Cerrar el pincel — se conservan las marcas
align-brush-mesh-selection = Selección de malla
align-brush-moving = Móvil
align-brush-fixed = Fija
align-brush-both = Ambos
align-brush-both-hint = Pinta y aplica en ambos escaneos — la superficie bajo el cursor recibe el trazo
align-brush-size = tamaño del pincel
align-brush-inverse = Pincel inverso
align-brush-inverse-hint = Arrastrar pela en vez de marcar. Shift lo invierte
align-brush-auto-radius = radio automático
align-brush-auto-radius-hint = Radio del área conservada en cada extremo de flecha
align-brush-size-status = Pincel { $size } mm
align-status-no-summary = No hay superficie comparable

align-mask-fit-everywhere = Ajustar en todas partes
align-mask-fit-everywhere-hint = Limpiar todas las marcas
align-mask-fit-everywhere-report = Marcas limpias — ajuste en todo el escaneo
align-mask-fit-everywhere-report-one = { $name }: marcas borradas
align-mask-fit-nowhere = No ajustar en ningún lado
align-mask-fit-nowhere-hint = Marcar toda la malla — el ajuste no tendrá efecto
align-mask-fit-nowhere-report = Malla entera marcada — el ajuste no tendrá efecto
align-mask-fit-nowhere-report-one = { $name }: escaneo completo excluido del ajuste
align-mask-invert = Invertir marcas
align-mask-invert-hint = Marcar lo sin marcar y viceversa
align-mask-invert-report = Marcas invertidas
align-mask-invert-report-one = { $name }: marcas invertidas
align-mask-automatic = Marca automática
align-mask-automatic-hint = Ajustar solo un área pequeña en cada extremo
align-mask-automatic-report = Ajuste solo en extremos de flecha
align-mask-automatic-report-one = { $name }: ajuste solo alrededor de las puntas de flecha
align-mask-automatic-empty = Coincidencia en todas partes: la región cubrió todo el escaneo, así que no se excluyó nada

align-status-half-dropped = Flecha a medias descartada
align-status-turned = Par girado
align-status-cleared = Par limpiado
align-status-click-moving = Clic en un punto del escaneo que debe moverse
align-status-click-alternate = Clic alternando puntos en las mismas posiciones
align-status-two-scans = Dos escaneos a la vista — clic en un punto de cada uno
align-status-no-surface = Una nube de puntos no tiene superficie que emparejar
align-status-now-other = Ahora clic en el punto homólogo del otro escaneo
align-status-moved = Punto movido
align-status-wrong-scan = Ese escaneo no es del par — pulsa Limpiar y reintenta
align-status-place-first = Coloca primero un punto en cada escaneo
align-status-one-scan = Uno de los escaneos
align-status-scaled = Ese escaneo trae colocación escalada, no alineable
align-status-pose-refused = El ajuste terminó, pero su escaneo ya no está
align-status-worker-unavailable = El proceso de alineación se detuvo — reinicia la herramienta
align-status-measure-dropped = Medición descartada — el pincel posee los colores
align-status-measure-unavailable = Medición no aplicada — el escaneo cambió; ejecuta Best fit matching de nuevo
align-status-map-elsewhere = El mapa está en la pestaña Automático — vuelve allí
align-status-aligned-points = Alineado por puntos

## Align result status lines — DRAFT.

align-status-aligned = Alineado por puntos — ejecuta Best fit matching para asentar las superficies.
align-status-refined = Best fit listo
align-status-measured = Mapa de calor actualizado
align-status-remeasure = { $reason } — ejecuta el ajuste fino para medir de nuevo
align-status-settings-changed = Ajustes de matching cambiados
align-status-visibility-changed = Cambió la visibilidad de un escaneo seleccionado
align-drag-moving = Moviendo { $name } a mano
align-drag-unrecorded = Movido a mano, pero este paso no quedó en el historial — Ctrl+Z no lo deshará
align-drag-moved = { $name }: movimiento de { $moved } mm a mano (Ctrl+Z deshace)
align-status-moved-hand = Movido a mano
align-pair-placed = Par { $n } colocado
align-roles-swapped = { $moving } se mueve ahora; { $fixed } no se mueve
align-status-scan-changed = El escaneo cambió
align-status-hidden = Capa oculta: { $name }. Muéstrala para alinear contra ella
align-arrow-removed = { $n ->
    [one] Flecha eliminada — queda { $n } par
   *[other] Flechas eliminadas — quedan { $n } pares
}
align-status-markings-changed = Marcas cambiadas
align-status-place-arrow-first = Coloca al menos una flecha antes de marcar
align-status-arrows-cleared = Flechas fuera — a mano desde aquí

## Unsaved-work guards and error dialog buttons — DRAFT.

guard-close-title = Ediciones de malla sin guardar
guard-close-headline-one = 1 capa editada sin guardar en disco.
guard-close-headline-many = Capas editadas sin guardar en disco.
guard-close-note = { $count } capas editadas afectadas.
guard-close-detail = Guardar exporta cada capa editada (PLY, STL u OBJ) y cierra.
guard-close-destructive = Cerrar sin guardar
guard-replace-title = Edición en curso
guard-replace-headline-session = Hay una sesión activa en { $layer }.
guard-replace-headline-one = 1 capa con cambios sin guardar.
guard-replace-headline-many = { $count } capas con cambios sin guardar.
guard-replace-detail = Abrir una escena cierra la sesión y descarta lo no guardado.
guard-replace-destructive = Descartar y abrir
guard-save = Guardar…
guard-cancel = Cancelar

error-retry-graphics = Intentar de nuevo
error-close = Cerrar
error-copy-details = Copiar detalles

about-website = Sitio web
about-source = Código
about-licenses = Licencias de terceros
about-license-kind = Apache License 2.0

## Mesh-edit operations, undo/redo, sculpt, measure, cut ruler, scene menu — DRAFT.

edit-select-faces-first = Selecciona primero caras de la malla
edit-no-changes = Sin cambios: { $layer }
edit-apply-failed-title = No se pudo editar la selección
edit-apply-failed-summary = No se pudo editar la selección: { $detail }
edit-no-changes-hidden = Sin cambios: afina la selección; las capas ocultas no se tocan
edit-selected-faces = { $faces ->
    [one] { $faces } cara seleccionada
   *[other] { $faces } caras seleccionadas
}
edit-selected-faces-across = { $faces ->
    [one] { $faces } cara seleccionada en { $layers } capas
   *[other] { $faces } caras seleccionadas en { $layers } capas
}

holes-nothing = Nada que cerrar: { $layer }
holes-partial = { $segments }, sin cerrar: { $layer }
holes-closed = { $filled ->
    [one] Cerrado { $filled } agujero
   *[other] Cerrados { $filled } agujeros
}
holes-closed-detail = { $closed }: { $layer }
holes-closed-segments = { $closed } ({ $segments }): { $layer }
holes-seg-healed = { $n ->
    [one] Curada { $n } mella
   *[other] Curadas { $n } mellas
}
holes-seg-border = borde del escaneo abierto
holes-seg-oversize-limit = { $n ->
    [one] { $n } agujero sobre el límite de { $limit } mm
   *[other] { $n } agujeros sobre el límite de { $limit } mm
}
holes-seg-oversize = { $n ->
    [one] { $n } agujero demasiado grande
   *[other] { $n } agujeros demasiado grandes
}
holes-seg-damaged = { $n ->
    [one] Omitido { $n } borde dañado
   *[other] Omitidos { $n } bordes dañados
}
batchedit-invert = Normales invertidas
batchedit-delete = Selección eliminada
batchedit-crop = Recorte a la selección
batchedit-cut = Selección cortada a nueva capa
batchedit-separate = Selección separada
batchedit-edited = Capa editada
edit-applied-status = { $action }: { $layer }
batchedit-status = { $label } en { $n ->
    [one] { $n } capa visible
   *[other] { $n } capas visibles
}
batch-close-holes = Agujeros interiores cerrados
batch-delete = Selección eliminada
batch-crop = Recorte a la selección
batch-cut = Selección cortada
batch-separate = Selección separada
batch-edited = Selección editada

select-covers-all = La selección ya cubre toda la malla: { $layer }
select-covers-remove = La selección cubre todo — mejor elimina la capa: { $layer }
select-splits = La selección se parte en { $parts } — afina la selección: { $layer }
select-faces-cannot = No se pueden elegir caras: { $layer }

undo-nothing = Nada que deshacer
redo-nothing = Nada que rehacer
undo-undid = Edición deshecha: { $layer }
undo-unavailable = Deshacer no disponible — la escena cambió: { $layer }
redo-redid = Edición rehecha: { $layer }
redo-unavailable = Rehacer no disponible — la escena cambió: { $layer }

sculpt-armed-addremove = Añadir/Quitar: arrastra para aportar, Shift excava
sculpt-armed-smooth = Suavizar: arrastra para relajar, Shift fuerza
sculpt-off = Esculpido off
sculpt-applied-undo = Esculpido aplicado (Ctrl+Z deshace)
sculpt-applied-locked = Esculpido aplicado (sin deshacer: instantánea enorme)
sculpt-failed-title = Esculpido fallido
sculpt-failed = No se puede esculpir esta capa: { $detail }
sculpt-worker-stopped = Proceso de esculpido detenido: { $detail }
sculpt-preparing = Preparando el esculpido…
sculpt-nonuniform-scale = El esculpido requiere una malla con escala uniforme
sculpt-failure-worker-panicked = El proceso de esculpido falló: { $detail }
sculpt-failure-spawn = No se pudo iniciar el proceso de esculpido: { $detail }
sculpt-failure-kernel-pool = No se pudo crear el grupo de núcleos de esculpido: { $detail }
sculpt-failure-missing-undo-baseline = El trazo de esculpido no tiene base para deshacer
sculpt-failure-shadow-poisoned = El bloqueo de sombra de esculpido se corrompió
sculpt-failure-shadow-shape = La sombra del esculpido ya no coincide con la malla activa
sculpt-failure-invalid-vertex-index = El proceso de esculpido devolvió un índice de vértice no válido
sculpt-failure-worker-state-poisoned = El estado del proceso de esculpido se corrompió — reinicia Sculpt
sculpt-failure-vertex-count-changed = El resultado de esculpido cambió el número de vértices
sculpt-failure-topology-rebuild = No se pudo reconstruir la topología del esculpido: { $detail }
sculpt-worker-unavailable = Esculpido no disponible
sculpt-finishing = Terminando trazo…
sculpt-finishing-history = Terminando esculpido antes del historial…
sculpt-lasso-armed = Lazo armado: clic o arrastre perfila; Enter, doble clic o inicio cierra
sculpt-lasso-off = Lazo desarmado
sculpt-object-on = Elegir objeto: clic para elegirlo entero
sculpt-object-off = Elegir objeto off
sculpt-selection-cleared = Selección limpiada
sculpt-through-on = Selección a través
sculpt-through-off = Selección de superficie

measure-distance = Distancia: { $len }
measure-thickness = Grosor de pared: { $len }
measure-open-wall = Superficie abierta: sin pared opuesta en la normal
measure-cannot-probe = Aquí no se puede medir: geometría degenerada
measure-cleared = Mediciones limpiadas

cut-lines = Líneas
cut-mesh = Malla
cut-dist = Dist
cut-dist-hint = Distancia: clic en dos puntos
cut-thick = Grosor
cut-thick-hint = Grosor: clic en un punto del contorno
cut-close-section = Cerrar sección
cut-snap = Imán
cut-snap-hint = Imán: los clics se pegan al contorno
cut-empty = Sin intersección
cut-footer-distance = Arrastrar = desplazar · clic 2 ptos = distancia · clic derecho limpia · rueda = zoom
cut-footer-thickness = Arrastrar = desplazar · clic contorno = grosor · clic derecho limpia · rueda = zoom

recent-clear = Limpiar recientes

scene-already-origin = Todo ya en su posición original
scene-positions-reset = Posiciones restablecidas (Ctrl+Z deshace)

## Session close-outs and layer shortcuts — DRAFT.

session-applied = Sesión aplicada
session-reverted = Sesión revertida
edit-session-busy = Termina o cancela la sesión primero
layers-none-hidden = Sin capas ocultas que traer
layer-opaque-again = Opaco de nuevo: { $label }
layer-translucent = Translúcido: { $label } (Shift+clic medio restaura)
layer-restored = Visible de nuevo: { $label }
layer-hidden = Oculta: { $label } (Shift+Ctrl+clic medio restaura)
layer-unnamed = capa { $n }
layer-removed = Capa eliminada: { $label }
layer-face-selection = Selección de caras: { $label }

## Bridge split panel and align session close-outs — DRAFT.

bridge-panel-title = Dividir puente
bridge-mode-place = Coloca el disco
bridge-mode-calculating = Calculando…
bridge-mode-ready = Listo
bridge-mode-failed = Intento fallido
bridge-kerf = Corte
bridge-disc-size = Tamaño del disco
bridge-cancel = Cancelar
bridge-apply = Dividir puente
bridge-err-miss = El disco no toca el puente. Mételo en el conector.
bridge-err-tangent = El disco solo roza. Atraviesa el conector.
bridge-err-small = Diámetro { $have } mm; aquí hacen falta { $need } mm.
bridge-err-limit = Este corte pide disco de { $need } mm, sobre el límite de { $max } mm.
bridge-err-no-result = Intento con superficie preservada, sin resultado útil. Malla original intacta.
bridge-err-invalid-cut = Intento fallido, el corte no valida. Malla original intacta.
bridge-err-invalid-side = Intento fallido, { $side } no valida. Malla original intacta.
bridge-err-gap = Intento fallido, el hueco no se conserva. Malla original intacta.
bridge-err-empty = La capa no tiene malla de triángulos que dividir.
bridge-err-invalid = Ajustes de disco inválidos. Reinicia y reintenta.
bridge-err-unusable = Sin resultado útil. Malla original intacta.

align-session-canceled = Alineación cancelada — todo vuelve atrás (Ctrl+Z la trae)
align-session-closed = Alineación cerrada
align-session-closed-running = Alineación cerrada — un ajuste corría y se soltó, todo como lo veías
align-session-kept = Alineación guardada — exporta el escaneo para escribirla

## Settings panel, bridge split, render error, tint — DRAFT.

settings-header = Ajustes
settings-section-files = Archivos y exportación
settings-save-format = Al guardar un escaneo
settings-save-format-hint = Qué escribe «Guardar capa». Se aplica un solo modo: el elegido aquí, no el formato de abajo.
settings-save-format-source = Su propio formato
settings-save-format-source-hint = Un escaneo abierto como STL se guarda como STL, uno abierto como PLY como PLY, y uno de un formato sin escritura se guarda en el formato alternativo indicado abajo.
settings-save-format-always = Formato elegido
settings-save-format-always-hint = Todos los escaneos se guardan en el formato elegido abajo, salvo si la geometría no cabe en él.
settings-save-format-source-note = Cada escaneo conserva su propio formato. Un formato no escribible se guarda como { $format }: el formato elegido en el otro modo.
settings-save-format-scene-note = Una escena completa guardada como un solo archivo usa siempre { $format }.
settings-export-format = Formato de exportación alternativo
settings-export-format-hint = Se usa para un escaneo cuyo formato propio no se puede escribir, o si el interruptor de arriba está desactivado
settings-remember-export = Recordar carpeta de exportación
settings-remember-export-hint = Misma carpeta tras reiniciar OccluView
settings-section-scene = Vista y navegación
settings-frame-on-open = Encuadrar al abrir
settings-frame-on-open-hint = Volver a la vista inicial cuando un archivo reemplaza la escena
settings-double-click = Doble clic reencuadra
settings-double-click-hint = Doble clic recentra la cámara en el punto
settings-orbit = Velocidad orbital
settings-orbit-hint = Cómo de rápido orbita arrastrando con el derecho
settings-zoom = Velocidad de zoom
settings-zoom-hint = Cuánto acerca cada muesca de rueda
settings-background = Fondo
settings-bg-gray = Gris
settings-bg-white = Blanco
settings-bg-dark = Oscuro
settings-ghost = Fantasma del lado cortado
settings-ghost-hint = En corte, mostrar el lado quitado como fantasma
settings-measurements = Mediciones
settings-section-appearance = Apariencia
settings-theme = Tema
settings-theme-light = Claro
settings-theme-dark = Oscuro
settings-scale = Escala de interfaz
settings-scale-hint = Escala todo; 1.0 mantiene el sistema
settings-section-mesh = Edición de mallas
settings-remember-brush = Recordar pincel
settings-remember-brush-hint = Conservar tamaño y fuerza entre sesiones
settings-section-updates = Actualizaciones
settings-check-auto = Comprobar al iniciar
settings-check-now = Comprobar
settings-check-disabled-hint = El entorno desactiva las comprobaciones
settings-check-busy-hint = Ya hay una comprobación en curso
settings-update-disabled = Desactivado por el entorno
settings-update-checking = Comprobando…
settings-update-current = Al día
settings-update-skipped = Versión omitida
settings-update-failed = No se pudo comprobar
settings-save-error = No se pudieron guardar los ajustes. Reintentando…
settings-save-error-hint = El archivo de ajustes no está disponible
settings-shortcuts = Atajos de teclado
settings-about = Acerca de OccluView

bridge-busy = Termina o cancela la división primero
bridge-active = División ya activa
bridge-target-gone = Objetivo perdido
bridge-needs-mesh = La división necesita malla triangular visible
bridge-place-disc = División: coloca el disco separador
bridge-canceled-scene = División cancelada: escena cerrada
bridge-canceled-camera = División cancelada: sin cámara
bridge-canceled-changed = División cancelada: la malla cambió
bridge-canceled = División cancelada
bridge-calculating = División: calculando…
bridge-unavailable = División no disponible ahora
bridge-preview-stale = Vista previa caducada
bridge-not-applied = División no aplicada
bridge-complete = División completa
bridge-complete-surface = División completa (superficie; bordes naturales intactos)
bridge-complete-locked = División completa (sin deshacer: instantánea enorme)

render-failed-title = No se pudo renderizar
render-failed-summary = El archivo abrió, pero el visor no renderiza.
render-failed-status = Falló el render

tint-choose = Elegir tinte

## Status tail: brush, lasso, loading, GPU, align jobs — DRAFT.

brush-no-mesh = Clic en un punto de cada malla, luego pinta
lasso-dropped = Lazo soltado
lasso-needs-points = El lazo necesita 3 puntos mínimo
loading-scene = Cargando escena…
gpu-failed-status = El driver reportó un problema
gpu-retry-status = Reintentando gráficos: si el problema persiste, guarde su trabajo y reinicie OccluView
gpu-failed-title = Problema de gráficos
gpu-failed-summary = El driver falló dibujando. La vista puede estar incompleta. Guarda y reinicia si se repite.
align-job-align = Alineando…
align-job-refine = Refinando…
align-job-measure = Midiendo…
align-markings-dropped = Marcas soltadas — la superficie cambió tras pintar

## Worker-built align failures — DRAFT.

align-fail-no-surface-fixed = El escaneo fijo no tiene superficie útil
align-fail-no-surface-moving = El escaneo móvil no tiene superficie útil
align-fail-recolor = Medición descartada antes de colorear
align-fail-unobservable = La superficie no permite un mapa de desviación fiable
align-reject-toofew = Coloca más flechas o acerca los escaneos
align-reject-unpaired = Completa ambos lados de cada flecha
align-reject-degenerate-plain = Distribuye los puntos por la superficie
align-reject-unit = Los escaneos usan unidades distintas
align-reject-apart = Revisa las flechas y acerca los escaneos
align-reject-runaway = Acerca los escaneos y repite Best fit matching
align-reject-no-improvement = El ajuste no confirmó una mejora — acerca los escaneos e inténtalo de nuevo
align-reject-ambiguous = El ajuste encontró varias superficies igual de probables — marca la zona correspondiente o acerca los escaneos
align-reject-nonfinite = El punto o la superficie seleccionados no son válidos
align-status-stepped = Pasos por el historial
align-status-moving-hand = Moviendo a mano

## Contactos oclusales: clic derecho en un escaneo y ver dónde se encuentra con
## el escaneo antagonista. Una lectura es papel de articular (solo marcas,
## coloreadas por profundidad), la otra el mapa de aproximación (cuán cerca,
## en todas partes). Un control mueve la profundidad que la escala considera
## carga completa, y recolorea un campo ya medido en lugar de volver a medir.
layer-menu-contacts = Mostrar contactos
layer-menu-hide-contacts = Ocultar contactos

contact-title = Contactos oclusales
contact-close-hint = Cerrar la lectura y quitar las marcas de ambos escaneos
contact-against = { $subject } contra { $antagonist }
contact-unknown-layer = un escaneo que ya no está abierto

contact-mode-marks = Contactos
contact-mode-marks-hint = Donde las superficies se encuentran, coloreado por intensidad — el resto queda limpio, como lo deja el papel de articular
contact-mode-approach = Aproximación
contact-mode-approach-hint = Cuán cerca está el otro escaneo en todas partes, carga incluida

contact-load-label = carga a
contact-load-suffix = mm
contact-load-hint = La profundidad a la que esta escala se lee como carga completa. Moverla recolorea el mapa ya medido, sin volver a medir.
contact-flatten = Un color por contacto
contact-flatten-hint = Reducir cada zona de contacto a su punto más profundo. Desactivado conserva la distribución de fuerza dentro de cada marca.
# Label above the list of layers a contact reading can be measured against.
contact-antagonist-pick = Medido contra
contact-antagonist-pick-hint = El escaneo más cercano se elige automáticamente. Elija otra capa para medir contra ella.

contact-legend-deepest = { $mm } mm dentro de la mordida

contact-stats-area = Área de contacto
contact-stats-contacts = Contactos
contact-stats-deepest = Más profundo

contact-readout-gap = holgura
contact-readout-load = carga

contact-status-measuring = Midiendo…
contact-status-measuring-hint = Se están leyendo las dos superficies entre sí
contact-status-remeasuring = Volviendo a medir…
contact-status-remeasuring-hint = Un escaneo se movió, así que las distancias cambiaron. El mapa se lee de nuevo.
contact-status-needs-second = Una lectura de contactos necesita un segundo escaneo visible contra el que medir
contact-status-no-surface = Uno de los dos escaneos no tiene superficie que medir
contact-status-worker-failed = La medición no se completó

contact-opened = Leyendo contactos en { $label }
contact-closed = Lectura de contactos cerrada
help-section-contacts = Contactos oclusales
help-hintline-contacts = Clic derecho en una capa · Mostrar contactos · mueva «carga a» para recolorear · Esc cierra
help-hint-contacts-read-its-occlusal-contacts-against-the-scan-it-bites = Leer sus contactos oclusales contra el escaneo antagonista
help-hint-contacts-read-the-contact-depth-under-the-cursor = Leer la profundidad del contacto bajo el puntero, en cualquiera de las arcadas
help-hint-contacts-move-the-depth-the-ramp-calls-fully-loaded = Mover la profundidad que la escala considera carga completa
help-hint-contacts-switch-between-marks-only-and-the-whole-approach = Cambiar entre solo marcas y toda la aproximación
help-hint-contacts-close-the-reading-and-take-the-marks-off-both-scans = Cerrar la lectura y quitar las marcas de ambos escaneos
contact-retry = Leer de nuevo
contact-status-subject-unusable = El escaneo del que trata esta lectura no se puede medir ahora mismo
contact-status-subject-unusable-hint = Muéstrelo de nuevo o déjelo como malla de triángulos, y la lectura continúa
contact-status-antagonist-unusable = El escaneo contra el que se mide no se puede medir ahora mismo
contact-status-antagonist-unusable-hint = Muéstrelo de nuevo o déjelo como malla de triángulos, y la lectura continúa
contact-status-no-overlap = Los escaneos están demasiado separados
contact-status-no-overlap-hint = Nada de ninguna de las dos superficies quedó al alcance de la lectura. Compruebe que estén en oclusión.
contact-status-failed-hint = Lea de nuevo; si sigue fallando, puede que el par necesite reparación primero.
contact-status-needs-second-hint = Abra el escaneo antagonista o muéstrelo de nuevo y empiece la lectura
contact-legend-gap = holgura hasta { $mm } mm
contact-stats-balance = Área por lado
layer-menu-contacts-unavailable = Una lectura de contactos necesita dos mallas de triángulos visibles: muestre o abra antes el escaneo antagonista

contact-details = Detalles
contact-details-hint = Los números y la regla de un color por contacto
contact-details-close = Ocultar detalles
settings-shortcuts-hint = Referencia de teclado y ratón (F1)

load-units-ambiguous = Unidades sin verificar: el formato declara metros, pero los escáneres suelen exportar milímetros — { $suggestion }
load-units-suggest-meters = el tamaño sugiere metros, así que es unas 1000 veces más pequeño de lo que debería
load-units-suggest-millimeters = el tamaño sugiere números en milímetros, así se leyó
load-units-unclear = el tamaño no lo decide; compruébelo con una medida conocida
contact-stats-balance-hover = Área de contacto dividida por la línea media: antes / después de la línea
mesh-warning-vertex-alpha = el alfa de vértice no se escribió
load-superseded-parked-open = Hay una apertura más reciente esperando: responda primero a su aviso

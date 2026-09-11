## OccluView Italian catalog — DRAFT (machine draft, native review required).
## Status: DRAFT. Requires native dental/CAD terminology review + visual UI review before APPROVED.
## Contract: exact key/attribute/variable parity with en.ftl.

app-title = OccluView
app-window-title = OccluView 3D Viewer
align-panel-title = Allinea le scansioni
meshedit-window-title = Editing delle mesh

settings-language-label = Lingua
settings-language-auto = Lingua di sistema
settings-language-auto-current = Lingua di sistema — { $language }
settings-language-catalog-fallback = { $tag } non è disponibile; viene usato l’inglese.
settings-language-save-error = Lingua non salvata. Riprovo…

about-title = Su OccluView
about-tagline = Riparazione mesh · Editing per il CAD dentale
about-version = Versione { $version }

update-available-title = Aggiornamento disponibile
update-available-body = La versione { $version } è pronta da installare.
update-current-version = Hai la { $version }.
update-download = Scarica aggiornamento
update-open-release = Apri la pagina di release
update-later = Più tardi
update-skip = Salta questa versione
update-skip-tooltip = Non proporre più questa versione; verrà proposta la prossima
update-downloading = Scaricamento di OccluView { $version }
update-ready-title = OccluView { $version } pronto da installare
update-ready-hint-windows = Installer verificato. OccluView si chiuderà mentre Windows applica l’aggiornamento.
update-ready-hint-other = Pacchetto verificato. Si aprirà l’installer di sistema — conferma lì.
update-install-close = Installa e chiudi
update-failed-title = Aggiornamento fallito
update-dismiss = Ignora

error-open-title = Impossibile aprire il file
error-add-title = Impossibile aggiungere il file
error-open-body = Impossibile aprire { $path }.

## Help surface — DRAFT. Gesture names stay invariant by contract.

help-title = Controlli da tastiera e mouse
help-subtitle = Il riferimento corrisponde ai controlli di OccluView.
help-close = Chiudi
help-toggle = Aiuto
help-toggle-tooltip = Mostra i controlli da tastiera e mouse

help-section-navigation = Navigazione
help-section-tools = Strumenti
help-section-mesh-editing = Editing delle mesh
help-section-sculpt = Scultura
help-section-align-measure = Allineamento e misura
help-section-cut-view = Vista in sezione
help-section-layers-preview = Livelli e anteprima di Explorer

help-hintline-navigation = Trascina DX orbita · CM pan · rotella zoom · clic CM fuoco
help-hintline-mesh-editing = Clic SX seleziona · Shift+clic deseleziona · rettangolo · Ctrl+Z annulla
help-hintline-sculpt = SX scolpisce · Shift cambia modo · Shift+rotella misura · Ctrl+rotella forza
help-hintline-align = SX piazza · Ctrl/Command+trascina ruota · Shift+trascina cancella · DX annulla
help-hintline-cut = SX pianta o sposta · Ctrl+rotella in Sezione ridimensiona · F ribalta · Esc chiude
help-hintline-measure = SX misura · DX pulisce · rotella zoom · Esc chiude

help-hint-navigation-orbit-the-camera = Orbita la camera
help-hint-navigation-pan-the-camera = Pan della camera
help-hint-navigation-pan-the-camera-2 = Pan della camera
help-hint-navigation-zoom-toward-the-pointer = Zoom verso il puntatore
help-hint-navigation-recenter-on-the-surface = Ricentra sulla superficie
help-hint-navigation-recenter-on-the-surface-when-enabled = Ricentra se attivo
help-hint-navigation-open-the-layer-or-scene-menu-when-stationary = Apri il menu livello/scena da fermo
help-hint-tools-open-a-file = Apri un file
help-hint-tools-open-cut-view = Apri vista in sezione
help-hint-tools-arm-the-ruler = Attiva il righello
help-hint-tools-arm-thickness = Attiva spessore
help-hint-tools-open-align = Apri allineamento
help-hint-tools-open-mesh-editing = Apri editing delle mesh
help-hint-mesh-editing-select-a-face = Seleziona una faccia
help-hint-mesh-editing-unmark-a-face-or-screen-selection = Deseleziona faccia o selezione
help-hint-mesh-editing-select-faces-in-a-screen-rectangle = Seleziona facce in un rettangolo
help-hint-mesh-editing-draw-a-freehand-selection-outline = Disegna un contorno libero
help-hint-mesh-editing-close-and-apply-a-lasso-outline = Chiudi e applica il lazo
help-hint-mesh-editing-cancel-the-active-lasso-outline = Annulla il lazo attivo
help-hint-mesh-editing-select-all-visible-faces = Seleziona tutte le facce visibili
help-hint-mesh-editing-delete-selected-faces = Elimina le facce selezionate
help-hint-mesh-editing-undo-the-last-mesh-edit = Annulla l’ultima modifica
help-hint-mesh-editing-redo-the-last-mesh-edit = Ripeti l’ultima modifica
help-hint-sculpt-choose-add-remove = Scegli aggiungi/rimuovi
help-hint-sculpt-choose-smooth = Scegli leviga
help-hint-sculpt-sculpt-under-the-brush = Scolpisci sotto il pennello
help-hint-sculpt-remove-or-strengthen-the-active-brush-mode = Rimuovi o rafforza la modalità attiva
help-hint-sculpt-change-brush-size = Cambia la misura del pennello
help-hint-sculpt-change-brush-intensity = Cambia la forza del pennello
help-hint-align-measure-place-an-alignment-point-or-measurement-point = Piazza un punto di allineamento o misura
help-hint-align-measure-rotate-a-scan-in-align-s-manual-mode = Ruota una scansione in manuale
help-hint-align-measure-erase-an-align-exclusion-region = Cancella una zona di esclusione
help-hint-align-measure-change-align-exclusion-brush-size = Cambia la misura del pennello di esclusione
help-hint-align-measure-undo-the-last-alignment-point-when-stationary = Annulla l’ultimo punto da fermo
help-hint-align-measure-clear-measurements-when-stationary = Pulisci le misure da fermo
help-hint-align-measure-close-the-active-measurement-tool = Chiudi lo strumento di misura
help-hint-cut-view-plant-or-move-the-cut-disc = Pianta o sposta il disco
help-hint-cut-view-change-disc-size = Cambia la misura del disco
help-hint-cut-view-zoom-the-section-view = Zoom della vista in sezione
help-hint-cut-view-flip-the-kept-half-while-planted = Ribalta la metà tenuta
help-hint-cut-view-unplant-the-disc-or-close-cut-view = Stacca il disco o chiudi la vista
help-hint-layers-preview-hide-the-layer-under-the-pointer = Nascondi il livello sotto il puntatore
help-hint-layers-preview-restore-the-last-hidden-layer = Ripristina l’ultimo livello nascosto
help-hint-layers-preview-toggle-layer-translucency = Alterna la traslucenza
help-hint-layers-preview-orbit-the-preview-model = Orbita il modello
help-hint-layers-preview-zoom-the-preview-model = Zoom del modello
help-hint-layers-preview-frame-the-preview-model = Inquadra il modello
help-hint-layers-preview-toggle-preview-wireframe = Alterna il wireframe

## Toolbar, empty state — DRAFT.

toolbar-open-label = Apri
toolbar-open-hint = Apri file 3D ({ $shortcut })
toolbar-recent-hint = File recenti
toolbar-add-label = Aggiungi
toolbar-add-hint = Aggiungi file alla scena
toolbar-cut-label = Vista in sezione
toolbar-cut-hint = Seziona il modello su un piano ({ $shortcut })
toolbar-cut-unavailable = La sezione vuole un livello visibile
toolbar-ruler-label = Righello
toolbar-ruler-hint = Misura una distanza: due punti sul modello ({ $shortcut })
toolbar-thickness-label = Spessore
toolbar-thickness-hint = Sonda lo spessore: un punto sul guscio ({ $shortcut })
toolbar-measure-blocked = Finisci o annulla la sessione prima
toolbar-measure-needs-layer = Misurare vuole un livello visibile
toolbar-align-label = Allinea
toolbar-align-hint = Unisci due scansioni: un punto per una ({ $shortcut })
toolbar-edit-label = Modifica
toolbar-edit-open = Editing delle mesh aperto
toolbar-edit-hint = Editing delle mesh: selezione e scultura ({ $shortcut })
toolbar-settings-label = Impostazioni
toolbar-settings-hint = Apri le preferenze

empty-open-file = Apri un file 3D
empty-formats-hint = STL · PLY · OBJ · GLB · HPS — o trascina i file qui

## Loading and export — DRAFT.

load-queued = { $count ->
    [one] { $count } livello in coda
   *[other] { $count } livelli in coda
}
load-opening = { $count ->
    [one] Apertura di { $count } file…
   *[other] Apertura di { $count } file…
}
load-adding = { $count ->
    [one] Aggiunta di { $count } file…
   *[other] Aggiunta di { $count } file…
}
load-open-failed-start = Apertura fallita: loader non partito
load-add-failed-start = Aggiunta fallita: loader non partito
load-open-failed-stopped = Apertura fallita: loader fermo
load-loader-failed-summary = Il loader di scena non si è avviato.
load-action-failed-open = Apertura fallita: { $detail }
load-action-failed-add = Aggiunta fallita: { $detail }

export-nothing-visible = Niente di visibile da salvare
export-unsupported-format = Formato di uscita non supportato
export-scene-saved = Scena salvata: { $path }
export-scene-saved-unmerged = Scena salvata (texture non fuse): { $path }
export-scene-failed-title = Salvataggio scena fallito
export-scene-failed-summary = Salvataggio scena fallito: { $detail }
export-layers-saved = { $written ->
    [one] Salvato { $written } livello in { $dir }
   *[other] Salvati { $written } livelli in { $dir }
}
export-layers-saved-failed = { $written ->
    [one] Salvato { $written } livello in { $dir }
   *[other] Salvati { $written } livelli in { $dir }
}; { $failed ->
    [one] { $failed } non scritto
   *[other] { $failed } non scritti
}
export-layers-saved-renamed = { $written ->
    [one] Salvato { $written } livello in { $dir }
   *[other] Salvati { $written } livelli in { $dir }
}; { $renamed ->
    [one] { $renamed } file rinominato per tenere l’esistente
   *[other] { $renamed } file rinominati per tenere l’esistente
}
export-layers-saved-failed-renamed = { $written ->
    [one] Salvato { $written } livello in { $dir }
   *[other] Salvati { $written } livelli in { $dir }
}; { $failed ->
    [one] { $failed } non scritto
   *[other] { $failed } non scritti
}; { $renamed ->
    [one] { $renamed } file rinominato per tenere l’esistente
   *[other] { $renamed } file rinominati per tenere l’esistente
}
mesh-exported-aligned = { $name } esportato in posizione allineata come { $format }: { $path }
mesh-exported-aligned-warnings = { $name } esportato in posizione allineata come { $format } (avvisi: { $warnings }): { $path }
mesh-exported-unmoved = { $name } esportato (non spostato) come { $format }: { $path }
mesh-exported-unmoved-warnings = { $name } esportato (non spostato) come { $format } (avvisi: { $warnings }): { $path }
mesh-warning-vertex-colors = colori dei vertici non inclusi
mesh-warning-uvs = UV non inclusi
mesh-warning-texture-image = immagine texture non inclusa
mesh-export-warnings = Avvisi di esportazione: { $warnings }
mesh-export-failed-title = Export livello fallito
mesh-export-failed-summary = Export livello fallito: { $detail }

## Repair card and toasts — DRAFT.

repair-title = Riparazione delle mesh
repair-clean-headline = Niente da riparare — mesh pulita
repair-copy-details = Copia dettagli
repair-copy-tooltip = Copia il report completo negli appunti
repair-line-welded = { $count ->
    [one] Saldato { $grouped } vertice duplicato
   *[other] Saldati { $grouped } vertici duplicati
}
repair-line-slivers = { $count ->
    [one] Rimossa { $grouped } faccia degenere
   *[other] Rimosse { $grouped } facce degeneri
}
repair-line-duplicate-faces = { $count ->
    [one] Rimossa { $grouped } faccia duplicata
   *[other] Rimosse { $grouped } facce duplicate
}
repair-line-nonmanifold = { $count ->
    [one] Sistemato { $grouped } spigolo non-manifold
   *[other] Sistemati { $grouped } spigoli non-manifold
}
repair-line-bowtie = { $count ->
    [one] Diviso { $grouped } vertice bowtie
   *[other] Divisi { $grouped } vertici bowtie
}
repair-line-reoriented = { $count ->
    [one] Riorientato { $grouped } triangolo
   *[other] Riorientati { $grouped } triangoli
}
repair-line-flipped = { $count ->
    [one] Ribaltata { $grouped } parte rovesciata
   *[other] Ribaltate { $grouped } parti rovesciate
}
repair-line-debris = { $count ->
    [one] Rimosso { $grouped } detrito
   *[other] Rimossi { $grouped } detriti
}
repair-line-pinholes = { $count ->
    [one] Chiuso { $grouped } micro-foro
   *[other] Chiusi { $grouped } micro-fori
}
repair-line-unused = { $count ->
    [one] Rimosso { $grouped } vertice inutile
   *[other] Rimossi { $grouped } vertici inutili
}
repair-open-rims = { $count ->
    [one] { $grouped } bordo aperto (confine scansione)
   *[other] { $grouped } bordi aperti (confine scansione)
}
repair-skipped-rims = { $count ->
    [one] { $grouped } bordo non riempito (non semplice)
   *[other] { $grouped } bordi non riempiti (non semplici)
}
repair-toast-welded = { $count ->
    [one] saldato { $count } vertice
   *[other] saldati { $count } vertici
}
repair-toast-slivers = { $count ->
    [one] rimossa { $count } degenere
   *[other] rimosse { $count } degeneri
}
repair-toast-duplicate-faces = { $count ->
    [one] { $count } faccia duplicata
   *[other] { $count } facce duplicate
}
repair-toast-nonmanifold = { $count ->
    [one] sistemato { $count } spigolo non-manifold
   *[other] sistemati { $count } spigoli non-manifold
}
repair-toast-bowtie = { $count ->
    [one] diviso { $count } bowtie
   *[other] divisi { $count } bowtie
}
repair-toast-reoriented = { $count ->
    [one] riorientato { $count } triangolo
   *[other] riorientati { $count } triangoli
}
repair-toast-flipped = { $count ->
    [one] ribaltata { $count } parte rovesciata
   *[other] ribaltate { $count } parti rovesciate
}
repair-toast-debris = { $count ->
    [one] rimosso { $count } detrito
   *[other] rimossi { $count } detriti
}
repair-toast-pinholes = { $count ->
    [one] chiuso { $count } micro-foro
   *[other] chiusi { $count } micro-fori
}
repair-toast-unused = { $count ->
    [one] rimosso { $count } vertice inutile
   *[other] rimossi { $count } vertici inutili
}
repair-toast-skipped = { $count ->
    [one] { $count } bordo saltato (non semplice)
   *[other] { $count } bordi saltati (non semplici)
}
repair-toast-done = Riparato { $layer }: { $parts }
repair-toast-clean-rims = Mesh già pulita: { $layer }, { $count ->
    [one] { $count } bordo aperto
   *[other] { $count } bordi aperti
}
repair-toast-clean = Mesh già pulita: { $layer }
repair-edit-busy = Editing del livello in corso
repair-edit-failed-title = Modifica del livello fallita
repair-edit-failed-summary = Modifica del livello fallita: { $detail }
edit-locked-status = { $status } (non annullabile: snapshot troppo grande)

## Layers overlay, layer menu, scene menu — DRAFT.

layers-title = Livelli
layers-count = { $count ->
    [one] { $count } livello
   *[other] { $count } livelli
}
layers-row-hide = Nascondi livello
layers-row-show = Mostra livello
layers-row-opacity = Opacità del livello
layers-row-remove = Rimuovi livello

layer-menu-next-tint = Tinta successiva
layer-menu-hide-colors = Nascondi i colori della scansione
layer-menu-show-colors = Mostra i colori della scansione
layer-menu-disable-texture = Disattiva texture
layer-menu-show-texture = Mostra texture
layer-menu-mesh-editing = Editing delle mesh
layer-menu-split-bridge = Dividi il bridge…
layer-menu-repair = Riparazione delle mesh
layer-menu-flip-normals = Inverti le normali
layer-menu-export = Esporta livello…
layer-menu-hide-wireframe = Nascondi wireframe
layer-menu-show-wireframe = Wireframe sopra
layer-menu-remove = Rimuovi livello

scene-menu-title = Scena
scene-menu-save = Salva scena come…
scene-menu-save-each = Salva ogni livello…
scene-menu-reset = Resetta le posizioni
scene-menu-fit = Inquadra la vista

## Mesh editor palette — DRAFT.

meshedit-tab-edit = Editing delle mesh
meshedit-tab-sculpt = Scultura
meshedit-cancel-session = Annulla la sessione (modifiche scartate)
meshedit-header-edit = Editing delle mesh
meshedit-section-selection = Selezione
meshedit-section-edit-selection = Modifica selezione
meshedit-section-close-holes = Chiudi i buchi
meshedit-section-sculpt = Scultura
meshedit-cell-lasso = Lazo
meshedit-cell-lasso-hint = Contorno libero: clic piazza punti, doppio clic chiude · Shift deseleziona
meshedit-cell-object = Oggetto
meshedit-cell-object-hint = Clic su un oggetto intero di un STL multiparti · Shift deseleziona
meshedit-cell-surface = Superficie
meshedit-cell-surface-hint = Marca solo la superficie frontale visibile
meshedit-cell-through = Attraverso
meshedit-cell-through-hint = Marca attraverso la mesh, anche i dorsi nascosti
meshedit-cell-all = Tutto
meshedit-cell-all-hint = Marca tutte le facce (Ctrl+A)
meshedit-cell-none = Niente
meshedit-cell-none-hint = Pulisci la marcatura
meshedit-cell-invert = Inverti
meshedit-cell-invert-hint = Scambia marcate e non marcate
meshedit-cell-delete = Elimina
meshedit-cell-delete-hint = Elimina le facce marcate
meshedit-cell-crop = Ritaglia
meshedit-cell-crop-hint = Tieni solo l’area marcata, rimuovi il resto
meshedit-cell-cut = Taglia
meshedit-cell-cut-hint = Sposta le facce in una nuova mesh — l’originale resta
meshedit-cell-separate = Separa
meshedit-cell-separate-hint = Dividi la zona in una mesh per parte connessa
meshedit-cell-close-holes = Chiudi i buchi
meshedit-cell-close-holes-hint = Chiudi solo con le facce vicine marcate. I bordi restano aperti.
meshedit-sculpt-addremove = Aggiungi / Rimuovi  [1]
meshedit-sculpt-addremove-hint = Costruisci trascinando; Shift scava. Shift+rotella ridimensiona, Ctrl+rotella cambia forza. Tasto: 1.
meshedit-sculpt-smooth = Leviga  [2]
meshedit-sculpt-smooth-hint = Rilassa trascinando; Shift forza il massimo. Shift+rotella ridimensiona, Ctrl+rotella cambia forza. Tasto: 2.
meshedit-slider-size = misura
meshedit-slider-size-hint = Misura del pennello (Shift + rotella)
meshedit-slider-force = forza
meshedit-slider-force-hint = Forza del pennello (Ctrl + rotella)
meshedit-limit-label = limite
meshedit-limit-checkbox-hint = Limita la riparazione ai bordi sotto questo perimetro
meshedit-limit-drag-hint = Off chiude ogni buco sicuro nella zona; il bordo resta aperto
meshedit-status-unsaved = Modifiche non salvate
meshedit-status-unsaved-hint = Da confermare: Fine applica, Annulla scarta
meshedit-status-hint-sculpt = Trascina per scolpire · DX orbita
meshedit-status-hint-object = Clic su un oggetto per tutto · Shift deseleziona
meshedit-status-hint-lasso = Clic profila · doppio clic chiude · Shift deseleziona
meshedit-status-hint-default = Trascina un riquadro · Shift deseleziona · Canc elimina
meshedit-session-undo = Annulla
meshedit-session-undo-hint = Annulla l’ultima modifica (Ctrl+Z)
meshedit-session-redo = Ripeti
meshedit-session-redo-hint = Ripeti la modifica annullata (Ctrl+Y)
meshedit-session-cancel = Annulla
meshedit-session-cancel-hint = Scarta tutte le modifiche della sessione
meshedit-session-done = Fine
meshedit-session-done-hint = Applica e chiudi l’editor

## Align Scans window — DRAFT.

align-title = Allinea le scansioni
align-tab-auto = Automatico
align-tab-manual = Manuale
align-constraint-free = Muovi/ruota in ogni direzione
align-constraint-free-hint = Trascina la scansione ovunque
align-constraint-z = Muovi in z
align-constraint-z-hint = Trascina solo in verticale
align-constraint-xy = Muovi nel piano xy
align-constraint-xy-hint = Trascina solo in orizzontale
align-manual-drag-hint = Muove la scansione afferrata · Ctrl+trascina ruota
align-undo = Annulla
align-undo-hint = Un passo indietro
align-redo = Ripeti
align-redo-hint = Un passo avanti
align-prompt-moving = Clic su un punto della mesh da muovere
align-prompt-other = Clic nella stessa posizione sull’altra mesh
align-prompt-alternate = Clic alternati nelle stesse posizioni delle due mesh
align-prompt-placed = { $count ->
    [one] { $count } freccia piazzata
   *[other] { $count } frecce piazzate
}
align-back = Indietro
align-back-hint = Annulla una freccia — clic destro uguale
align-clear = Pulisci
align-clear-hint = Butta le frecce e riscegli due scansioni — restano dove sono
align-fit-perform = Esegui allineamento
align-fit-perform-hint = Porta la mesh sulle frecce — almeno due frecce
align-fit-refine = Matching preciso
align-fit-refine-hint = Appoggia le superfici. Solo mesh identiche
align-matching-parts = parti coincidenti
align-matching-parts-hint = Quota di superficie su entrambe le mesh. 70-80% se le topologie si assomigliano
align-max-influence = influenza max
align-max-influence-hint = Influisce solo la superficie sotto questa distanza. Valori alti peggiorano
align-orientation-title = L’orientamento deve coincidere
align-orientation-match = L’orientamento deve coincidere
align-orientation-inverted = L’orientamento deve coincidere invertito
align-orientation-ignored = Orientamento ignorato
align-orientation-either-hint = Accetta entrambi i versi. Il calcolo spesso è molto più lungo
align-orientation-facing-hint = Come le due superfici si fronteggiano
align-exclude = Matching: escludi le parti marcate
align-exclude-hint = Dipingi la superficie da ignorare
align-commit-cancel = Annulla
align-commit-cancel-hint-moved = Rimette tutto e chiudi — Ctrl+Z riporta l’allineamento
align-commit-cancel-hint-clean = Chiudi senza cambiare niente
align-commit-done = Fine
align-commit-done-hint = Tieni l’allineamento e chiudi — esporta per scrivere

## Deviation map — DRAFT.

align-map-heatmap = Mappa di calore
align-map-heatmap-hint = Colora una scansione per distanza dall’altra
align-map-requires-refine = Esegui prima Best fit matching
align-map-max = max
align-map-not-measured = non misurato
align-map-not-measured-hint = Nessuna superficie dell’altra scansione a portata di questi vertici. Dente o bridge su una sola scansione: normale, non un errore — niente da misurare.

## Align roles, brush, mask commands, align status lines — DRAFT.

align-pair-decided = { $moving } → { $fixed }
align-pair-guessed = { $moving } → { $fixed } (ipotesi)
align-pair-hint-decided = { $moving } si muove, { $fixed } resta
align-pair-hint-guessed = Niente clic, ipotesi per ordine di apertura. Il primo clic decide: { $moving } si muove, { $fixed } resta
align-pair-swap = Scambia
align-pair-swap-hint = Adatta al contrario — le frecce seguono

align-brush-title = Pennello
align-brush-close-hint = Chiudi il pennello — i segni restano
align-brush-mesh-selection = Selezione mesh
align-brush-moving = Mobile
align-brush-fixed = Fissa
align-brush-size = misura del pennello
align-brush-inverse = Pennello inverso
align-brush-inverse-hint = Un trascinamento cancella invece di marcare. Shift inverte di nuovo
align-brush-auto-radius = raggio automatico
align-brush-auto-radius-hint = Raggio tenuto a ogni capo freccia
align-brush-size-status = Pennello { $size } mm
align-status-no-summary = Nessuna superficie comparabile

align-mask-fit-everywhere = Adatta ovunque
align-mask-fit-everywhere-hint = Pulisci ogni marcatura
align-mask-fit-everywhere-report = Marcature pulite — matching su tutta la scansione
align-mask-fit-nowhere = Non adattare da nessuna parte
align-mask-fit-nowhere-hint = Marca tutta la mesh — matching senza effetto
align-mask-fit-nowhere-report = Mesh intera marcata — matching senza effetto
align-mask-invert = Inverti marcature
align-mask-invert-hint = Marca le non marcate e viceversa
align-mask-invert-report = Marcature invertite
align-mask-automatic = Marca automatica
align-mask-automatic-hint = Adatta solo su una piccola zona a ogni capo
align-mask-automatic-report = Matching ai capi freccia

align-status-half-dropped = Freccia a metà scartata
align-status-turned = Coppia girata
align-status-cleared = Coppia pulita
align-status-click-moving = Clic su un punto della scansione da muovere
align-status-click-alternate = Clic alternati nelle stesse posizioni
align-status-two-scans = Due scansioni in vista — un punto per una
align-status-no-surface = Una nuvola di punti non ha superficie da accoppiare
align-status-now-other = Ora clic nel punto gemello dell’altra scansione
align-status-moved = Punto spostato
align-status-wrong-scan = Questa scansione non è della coppia — Pulisci e ricomincia
align-status-place-first = Prima un punto per scansione
align-status-one-scan = Una delle scansioni
align-status-scaled = Questa scansione ha un placement scalato, non allineabile
align-status-pose-refused = Fit finito, ma la sua scansione non c’è più
align-status-worker-unavailable = Il worker di allineamento si è fermato — riavvia lo strumento
align-status-measure-dropped = Misura scartata — il pennello ha i colori
align-status-measure-unavailable = Misura non applicata — la scansione è cambiata; esegui di nuovo Best fit matching
align-status-map-elsewhere = La mappa è nel tab Automatico — torna lì
align-status-aligned-points = Allineato sui punti

## Align result status lines — DRAFT.

align-status-aligned = Allineato sui punti — esegui Best fit matching per posare le superfici.
align-status-refined = Best fit pronto
align-status-measured = Mappa di calore aggiornata
align-status-remeasure = { $reason } — rilancia il matching per misurare
align-status-settings-changed = Impostazioni di matching cambiate
align-status-visibility-changed = Visibilità di una scansione selezionata modificata
align-brush-not-in-alignment = Questa scansione non fa parte di questo allineamento
align-drag-moving = Spostamento di { $name } a mano
align-drag-unrecorded = Spostato a mano, ma questo passo non è entrato nella cronologia — Ctrl+Z non lo annullerà
align-drag-moved = { $name }: spostamento di { $moved } mm a mano (Ctrl+Z annulla)
align-status-moved-hand = Spostato a mano
align-pair-placed = Coppia { $n } piazzata
align-roles-swapped = { $moving } si muove ora; { $fixed } resta al suo posto
align-status-scan-changed = La scansione è cambiata
align-status-hidden = Livello nascosto: { $name }. Mostralo per allineare su di esso
align-arrow-removed = { $n ->
    [one] Freccia rimossa — resta { $n } coppia
   *[other] Frecce rimosse — restano { $n } coppie
}
align-status-markings-changed = Marcature cambiate
align-status-place-arrow-first = Piazza almeno una freccia prima di marcare
align-status-arrows-cleared = Frecce via — a mano da qui

## Unsaved-work guards and error dialog buttons — DRAFT.

guard-close-title = Modifiche mesh non salvate
guard-close-headline-one = 1 livello modificato non salvato.
guard-close-headline-many = Livelli modificati non salvati.
guard-close-note = { $count } livelli modificati coinvolti.
guard-close-detail = Salva esporta ogni livello (PLY, STL o OBJ) e chiude.
guard-close-destructive = Chiudi senza salvare
guard-replace-title = Editing in corso
guard-replace-headline-session = Sessione attiva su { $layer }.
guard-replace-headline-one = 1 livello modificato non salvato.
guard-replace-headline-many = { $count } livelli modificati non salvati.
guard-replace-detail = Aprire una scena chiude la sessione e scarta il non salvato.
guard-replace-destructive = Scarta e apri
guard-save = Salva…
guard-cancel = Annulla

error-close = Chiudi
error-copy-details = Copia dettagli

about-website = Sito web
about-source = Sorgenti
about-licenses = Licenze di terze parti
about-license-kind = Licenza Apache 2.0

## Mesh-edit operations, undo/redo, sculpt, measure, cut ruler, scene menu — DRAFT.

edit-select-faces-first = Seleziona prima le facce della mesh
edit-no-changes = Niente cambiato: { $layer }
edit-apply-failed-title = Modifica della selezione fallita
edit-apply-failed-summary = Modifica della selezione fallita: { $detail }
edit-no-changes-hidden = Niente cambiato: affina la selezione; i livelli nascosti restano intatti
edit-selected-faces = { $faces ->
    [one] { $faces } faccia selezionata
   *[other] { $faces } facce selezionate
}
edit-selected-faces-across = { $faces ->
    [one] { $faces } faccia selezionata su { $layers } livelli
   *[other] { $faces } facce selezionate su { $layers } livelli
}

holes-nothing = Niente da chiudere: { $layer }
holes-partial = { $segments }, niente chiuso: { $layer }
holes-closed = { $filled ->
    [one] Chiuso { $filled } buco
   *[other] Chiusi { $filled } buchi
}
holes-closed-detail = { $closed }: { $layer }
holes-closed-segments = { $closed } ({ $segments }): { $layer }
holes-seg-healed = { $n ->
    [one] Rimarginata { $n } tacca
   *[other] Rimarginate { $n } tacche
}
holes-seg-border = bordo scansione tenuto aperto
holes-seg-oversize-limit = { $n ->
    [one] { $n } buco oltre il limite di { $limit } mm
   *[other] { $n } buchi oltre il limite di { $limit } mm
}
holes-seg-oversize = { $n ->
    [one] { $n } buco troppo grande
   *[other] { $n } buchi troppo grandi
}
holes-seg-damaged = { $n ->
    [one] Saltato { $n } bordo danneggiato
   *[other] Saltati { $n } bordi danneggiati
}
batchedit-invert = Normali invertite
batchedit-close-holes = Buchi interni sicuri chiusi
batchedit-delete = Selezione eliminata
batchedit-crop = Ritaglio alla selezione
batchedit-cut = Selezione tagliata in nuovo livello
batchedit-separate = Selezione separata
batchedit-edited = Livello modificato
edit-applied-status = { $action }: { $layer }
batchedit-status = { $label } su { $n ->
    [one] { $n } livello visibile
   *[other] { $n } livelli visibili
}
batchedit-no-changes = Niente cambiato: affina la selezione; i nascosti restano intatti
batch-close-holes = Buchi interni chiusi
batch-delete = Selezione eliminata
batch-crop = Ritaglio alla selezione
batch-cut = Selezione tagliata
batch-separate = Selezione separata
batch-edited = Selezione modificata

select-covers-all = La selezione copre già tutta la mesh: { $layer }
select-covers-remove = La selezione copre tutto — meglio rimuovere il livello: { $layer }
select-splits = La selezione si divide in { $parts } — affina la selezione: { $layer }
select-faces-cannot = Facce non selezionabili: { $layer }

undo-nothing = Niente da annullare
redo-nothing = Niente da ripetere
undo-undid = Modifica annullata: { $layer }
undo-unavailable = Annulla non disponibile — la scena è cambiata: { $layer }
redo-redid = Modifica ripetuta: { $layer }
redo-unavailable = Ripeti non disponibile — la scena è cambiata: { $layer }

sculpt-armed-addremove = Aggiungi/Rimuovi: trascina per costruire, Shift scava
sculpt-armed-smooth = Leviga: trascina per rilassare, Shift forza
sculpt-off = Scultura off
sculpt-applied-undo = Scultura applicata (Ctrl+Z annulla)
sculpt-applied-locked = Scultura applicata (non annullabile: snapshot enorme)
sculpt-failed-title = Scultura non riuscita
sculpt-failed = Questo livello non si scolpisce: { $detail }
sculpt-worker-stopped = Worker di scultura fermo: { $detail }
sculpt-preparing = Preparazione della scultura…
sculpt-nonuniform-scale = La scultura richiede una mesh con scala uniforme
sculpt-failure-worker-panicked = Worker di scultura in crash: { $detail }
sculpt-failure-spawn = Impossibile avviare il worker di scultura: { $detail }
sculpt-failure-kernel-pool = Impossibile creare il pool di kernel di scultura: { $detail }
sculpt-failure-missing-undo-baseline = Il tratto di scultura non ha una base di annullamento
sculpt-failure-shadow-poisoned = Il lock dell'ombra di scultura è avvelenato
sculpt-failure-shadow-shape = L’ombra della scultura non corrisponde più alla mesh attiva
sculpt-failure-invalid-vertex-index = Il worker di scultura ha restituito un indice di vertice non valido
sculpt-failure-worker-state-poisoned = Lo stato del worker di scultura è corrotto — riavvia Sculpt
sculpt-failure-vertex-count-changed = Il risultato di scultura ha modificato il numero di vertici
sculpt-failure-topology-rebuild = Ricostruzione della topologia di scultura non riuscita: { $detail }
sculpt-worker-unavailable = Scultura non disponibile
sculpt-finishing = Finitura del tratto…
sculpt-finishing-history = Finitura scultura prima della storia…
sculpt-lasso-armed = Lazo armato: clic o trascina profila; Invio, doppio clic o inizio chiude
sculpt-lasso-off = Lazo disarmato
sculpt-object-on = Scegli oggetto: clic per tutto l’oggetto
sculpt-object-off = Scegli oggetto off
sculpt-selection-cleared = Selezione pulita
sculpt-through-on = Selezione attraverso
sculpt-through-off = Selezione di superficie

measure-distance = Distanza: { $len }
measure-thickness = Spessore parete: { $len }
measure-open-wall = Superficie aperta: nessuna parete opposta nella normale
measure-cannot-probe = Qui non si misura: geometria degenere
measure-cleared = Misure pulite

cut-lines = Linee
cut-mesh = Mesh
cut-dist = Dist
cut-dist-hint = Distanza: clic su due punti
cut-thick = Spess
cut-thick-hint = Spessore: clic su un punto del contorno
cut-close-section = Chiudi sezione
cut-snap = Magnete
cut-snap-hint = Magnete: i clic si attaccano al contorno
cut-empty = Nessuna intersezione
cut-footer-distance = Trascina = pan · clic 2 pti = distanza · destro pulisce · rotella = zoom
cut-footer-thickness = Trascina = pan · clic contorno = spessore · destro pulisce · rotella = zoom

recent-clear = Pulisci recenti

scene-already-origin = Tutto già in posizione originale
scene-positions-reset = Posizioni resettate (Ctrl+Z annulla)

## Session close-outs and layer shortcuts — DRAFT.

session-applied = Sessione applicata
session-reverted = Sessione scartata
edit-session-busy = Finisci o annulla prima la sessione
layers-none-hidden = Nessun livello nascosto da tirare su
layer-opaque-again = Opaco di nuovo: { $label }
layer-translucent = Traslucido: { $label } (Shift+clic centrale ripristina)
layer-restored = Mostrato di nuovo: { $label }
layer-hidden = Nascosto: { $label } (Shift+Ctrl+clic centrale ripristina)
layer-unnamed = livello { $n }
layer-removed = Livello rimosso: { $label }
layer-face-selection = Selezione facce: { $label }

## Bridge split panel and align session close-outs — DRAFT.

bridge-panel-title = Dividi il bridge
bridge-mode-place = Piazza il disco
bridge-mode-calculating = Calcolo…
bridge-mode-ready = Pronto
bridge-mode-failed = Tentativo fallito
bridge-kerf = Taglio
bridge-disc-size = Misura del disco
bridge-cancel = Annulla
bridge-apply = Dividi il bridge
bridge-err-miss = Il disco manca il bridge. Mettilo nel connettore.
bridge-err-tangent = Il disco tocca e basta. Passa attraverso il connettore.
bridge-err-small = Diametro { $have } mm; qui servono { $need } mm.
bridge-err-limit = Questo taglio vuole un disco da { $need } mm, sopra il limite di { $max } mm.
bridge-err-no-result = Tentativo a superficie preservata, senza risultato utile. Mesh originale intatta.
bridge-err-invalid-cut = Tentativo fallito, taglio non validabile. Mesh originale intatta.
bridge-err-invalid-side = Tentativo fallito, { $side } non validabile. Mesh originale intatta.
bridge-err-gap = Tentativo fallito, gap non tenuto. Mesh originale intatta.
bridge-err-empty = Il livello non ha mesh di triangoli da dividere.
bridge-err-invalid = Impostazioni disco non valide. Resetta e riprova.
bridge-err-unusable = Senza risultato utile. Mesh originale intatta.

align-session-canceled = Allineamento annullato — tutto torna indietro (Ctrl+Z lo riporta)
align-session-closed = Allineamento chiuso
align-session-closed-running = Allineamento chiuso — un fit girava ed è stato mollato, tutto come lo vedevi
align-session-kept = Allineamento tenuto — esporta la scansione per scriverlo

## Settings panel, bridge split, render error, tint — DRAFT.

settings-header = Impostazioni
settings-section-files = File ed esportazione
settings-export-format = Formato di export di riserva
settings-export-format-hint = Quando il formato origine non si esporta
settings-remember-export = Ricorda la cartella di export
settings-remember-export-hint = Stessa cartella dopo il riavvio
settings-section-scene = Vista e navigazione
settings-frame-on-open = Inquadra all’apertura
settings-frame-on-open-hint = Torna alla vista home quando un file sostituisce la scena
settings-double-click = Doppio clic ricentra
settings-double-click-hint = Doppio clic ricentra la camera sul punto
settings-orbit = Velocità orbitale
settings-orbit-hint = Quanto orbita trascinando col destro
settings-zoom = Velocità di zoom
settings-zoom-hint = Quanto avvicina ogni scatto di rotella
settings-recent = Scene recenti
settings-recent-hint = Voci nel menu Apri
settings-background = Sfondo
settings-bg-gray = Grigio
settings-bg-white = Bianco
settings-bg-dark = Scuro
settings-ghost = Fantasma del lato tagliato
settings-ghost-hint = In sezione mostra il lato tolto come fantasma
settings-measurements = Misure
settings-section-appearance = Aspetto
settings-theme = Tema
settings-theme-light = Chiaro
settings-theme-dark = Scuro
settings-scale = Scala dell’interfaccia
settings-scale-hint = Scala tutto; 1.0 tiene il sistema
settings-section-mesh = Editing delle mesh
settings-remember-brush = Ricorda il pennello
settings-remember-brush-hint = Tieni misura e forza tra le sessioni
settings-section-updates = Aggiornamenti
settings-check-auto = Controlla all’avvio
settings-check-now = Controlla
settings-check-disabled-hint = Controlli disattivati dall’ambiente
settings-check-busy-hint = Un controllo è già in corso
settings-update-disabled = Disattivato dall’ambiente
settings-update-checking = Controllo…
settings-update-current = Aggiornato
settings-update-skipped = Versione saltata
settings-update-failed = Controllo fallito
settings-save-error = Preferenze non salvate. Riprovo…
settings-save-error-hint = File delle preferenze non disponibile
settings-about = Su OccluView

bridge-busy = Finisci o annulla prima la divisione
bridge-active = Divisione già attiva
bridge-target-gone = Obiettivo perso
bridge-needs-mesh = La divisione vuole una mesh visibile
bridge-place-disc = Divisione: piazza il disco separatore
bridge-canceled-scene = Divisione annullata: scena chiusa
bridge-canceled-camera = Divisione annullata: niente camera
bridge-canceled-changed = Divisione annullata: la mesh è cambiata
bridge-canceled = Divisione annullata
bridge-calculating = Divisione: calcolo…
bridge-unavailable = Divisione non disponibile ora
bridge-preview-stale = Anteprima scaduta
bridge-not-applied = Divisione non applicata
bridge-complete = Divisione completa
bridge-complete-surface = Divisione completa (superficie; bordi naturali intatti)
bridge-complete-locked = Divisione completa (non annullabile: snapshot enorme)

render-failed-title = Render fallito
render-failed-summary = Il file si apre, ma il viewport non renderizza.
render-failed-status = Render fallito

tint-choose = Scegli tinta

## Status tail: brush, lasso, loading, GPU, align jobs — DRAFT.

brush-no-mesh = Clic su un punto per mesh, poi dipingi
lasso-dropped = Lazo mollato
lasso-needs-points = Il lazo vuole almeno 3 punti
loading-scene = Caricamento scena…
gpu-failed-status = Il driver segnala un problema
gpu-failed-title = Problema grafico
gpu-failed-summary = Il driver ha toppato il disegno. La vista può essere incompleta. Salva e riavvia se ricapita.
align-job-align = Allineo…
align-job-refine = Rifinisco…
align-job-measure = Misuro…
align-markings-dropped = Marcature mollate — la superficie è cambiata dopo

## Worker-built align failures — DRAFT.

align-fail-no-surface-fixed = La scansione fissa non ha superficie utile
align-fail-no-surface-moving = La scansione mobile non ha superficie utile
align-fail-recolor = Misura scartata prima di colorare
align-fail-unobservable = La superficie non consente una mappa degli scostamenti affidabile
align-reject-toofew = Aggiungi frecce o avvicina le scansioni
align-reject-unpaired = Completa entrambi i lati di ogni freccia
align-reject-degenerate-plain = Distribuisci i punti sulla superficie
align-reject-unit = Le scansioni usano unità diverse
align-reject-apart = Controlla le frecce e avvicina le scansioni
align-reject-runaway = Avvicina le scansioni e riprova Best fit matching
align-reject-no-improvement = Nessun miglioramento confermato — avvicina le scansioni e riprova
align-reject-ambiguous = Trovate più superfici ugualmente plausibili — marca la zona corretta o avvicina le scansioni
align-reject-nonfinite = Il punto o la superficie selezionati non sono validi
align-status-stepped = Passi nella storia
align-status-moving-hand = Muovo a mano

## OccluView German catalog — DRAFT (machine draft, native review required).
## Status: DRAFT. Requires native dental/CAD terminology review + visual UI review before APPROVED.
## Contract: exact key/attribute/variable parity with en.ftl.

app-window-title = OccluView 3D Viewer
align-panel-title = Scans ausrichten
meshedit-window-title = Netzbearbeitung

settings-language-label = Sprache
settings-language-auto = Systemsprache
settings-language-auto-current = Systemsprache — { $language }
settings-language-catalog-fallback = { $tag } ist nicht verfügbar; Englisch wird verwendet.
settings-language-save-error = Spracheinstellung konnte nicht gespeichert werden. Neuer Versuch…

about-title = Über OccluView
about-tagline = Netzreparatur · Mesh-Bearbeitung für Dental-CAD
about-version = Version { $version }

update-available-title = Update verfügbar
update-available-body = Version { $version } ist zur Installation bereit.
update-current-version = Installiert ist { $version }.
update-download = Update herunterladen
update-open-release = Release-Seite öffnen
update-later = Später
update-skip = Diese Version überspringen
update-skip-tooltip = Diese Version nicht mehr anbieten; das nächste Release wird angeboten
update-downloading = OccluView { $version } wird heruntergeladen
update-ready-title = OccluView { $version } ist bereit zur Installation
update-ready-hint-windows = Installer verifiziert. OccluView schließt, während Windows das Update anwendet.
update-ready-hint-other = Paket verifiziert. Der System-Installer öffnet sich — Update dort bestätigen.
update-install-close = Installieren und schließen
update-failed-title = Update fehlgeschlagen
update-dismiss = Schließen

error-open-title = Datei kann nicht geöffnet werden
error-add-title = Datei kann nicht hinzugefügt werden

## Help surface — DRAFT. Gesture names stay invariant by contract.

help-title = Tastatur- und Maussteuerung
help-subtitle = Die Referenz entspricht den derzeit in OccluView verfügbaren Steuerelementen.
help-close = Schließen

help-section-navigation = Navigation
help-section-tools = Werkzeuge
help-section-mesh-editing = Netz bearbeiten
help-section-sculpt = Sculpting
help-section-align-measure = Ausrichtung und Messung
help-section-cut-view = Schnittansicht
help-section-layers-preview = Ebenen und Explorer-Vorschau

help-hintline-navigation = RMB Drehen · MMB Schwenken · Rad Zoom · MMB Fokus
help-hintline-mesh-editing = LMB Auswahl · Shift+Klick abwählen · Rechteck · Strg+Z rückgängig
help-hintline-sculpt = LMB sculpten · Shift wechselt Modus · Shift+Rad Größe · Strg+Rad Stärke
help-hintline-align = LMB Punkt · Strg/Cmd+Ziehen drehen · Shift+Ziehen löschen · RMB rückgängig
help-hintline-cut = LMB setzen/verschieben · Strg+Rad im Schnitt skaliert · F spiegelt · Esc schließt
help-hintline-measure = LMB messen · RMB löscht · Rad Zoom · Esc schließt

help-hint-navigation-orbit-the-camera = Kamera drehen
help-hint-navigation-pan-the-camera = Kamera schwenken
help-hint-navigation-pan-the-camera-2 = Kamera schwenken
help-hint-navigation-zoom-toward-the-pointer = Zum Zeiger zoomen
help-hint-navigation-recenter-on-the-surface = Auf Oberfläche zentrieren
help-hint-navigation-recenter-on-the-surface-when-enabled = Auf Oberfläche zentrieren, wenn aktiviert
help-hint-navigation-open-the-layer-or-scene-menu-when-stationary = Ebenen- oder Szenenmenü per ruhigem Klick öffnen
help-hint-tools-open-a-file = Datei öffnen
help-hint-tools-open-cut-view = Schnittansicht öffnen
help-hint-tools-arm-the-ruler = Lineal aktivieren
help-hint-tools-arm-thickness = Dicke aktivieren
help-hint-tools-open-align = Ausrichtung öffnen
help-hint-tools-open-mesh-editing = Netzbearbeitung öffnen
help-hint-mesh-editing-select-a-face = Fläche auswählen
help-hint-mesh-editing-unmark-a-face-or-screen-selection = Flächen- oder Bildschirmauswahl aufheben
help-hint-mesh-editing-select-faces-in-a-screen-rectangle = Flächen im Bildschirmrechteck auswählen
help-hint-mesh-editing-draw-a-freehand-selection-outline = Freihand-Auswahlkontur zeichnen
help-hint-mesh-editing-close-and-apply-a-lasso-outline = Lasso schließen und anwenden
help-hint-mesh-editing-cancel-the-active-lasso-outline = Aktives Lasso abbrechen
help-hint-mesh-editing-select-all-visible-faces = Alle sichtbaren Flächen auswählen
help-hint-mesh-editing-delete-selected-faces = Ausgewählte Flächen löschen
help-hint-mesh-editing-undo-the-last-mesh-edit = Letzte Netzbearbeitung rückgängig
help-hint-mesh-editing-redo-the-last-mesh-edit = Letzte Netzbearbeitung wiederholen
help-hint-sculpt-choose-add-remove = Hinzufügen/Entfernen wählen
help-hint-sculpt-choose-smooth = Glätten wählen
help-hint-sculpt-sculpt-under-the-brush = Unter dem Pinsel sculpten
help-hint-sculpt-remove-or-strengthen-the-active-brush-mode = Aktiven Pinselmodus entfernen oder verstärken
help-hint-sculpt-change-brush-size = Pinselgröße ändern
help-hint-sculpt-change-brush-intensity = Pinselstärke ändern
help-hint-align-measure-place-an-alignment-point-or-measurement-point = Ausrichtungs- oder Messpunkt setzen
help-hint-align-measure-end-a-ruler-on-a-ruler-line = Nach dem ersten Punkt das Lineal auf einer Messlinie beenden; dieses Ende lässt sich entlang der Linie ziehen
help-hint-align-measure-switch-between-any-angle-and-90 = Zwischen beliebigem Winkel und 90° zur Linie wechseln
help-hint-align-measure-rotate-a-scan-in-align-s-manual-mode = Scan im manuellen Modus drehen
help-hint-align-measure-erase-an-align-exclusion-region = Ausschlussbereich löschen
help-hint-align-measure-change-align-exclusion-brush-size = Ausschlusspinselgröße ändern
help-hint-align-measure-undo-the-last-alignment-point-when-stationary = Letzten Punkt per ruhigem Klick rückgängig
help-hint-align-measure-clear-measurements-when-stationary = Messungen per ruhigem Klick löschen
help-hint-align-measure-close-the-active-measurement-tool = Aktives Messwerkzeug schließen
help-hint-cut-view-plant-or-move-the-cut-disc = Schnittscheibe setzen oder verschieben
help-hint-cut-view-change-disc-size = Scheibengröße ändern
help-hint-cut-view-zoom-the-section-view = Schnittansicht zoomen
help-hint-cut-view-flip-the-kept-half-while-planted = Behaltene Hälfte spiegeln
help-hint-cut-view-unplant-the-disc-or-close-cut-view = Scheibe entfernen oder Schnitt schließen
help-hint-layers-preview-hide-the-layer-under-the-pointer = Ebene unter dem Zeiger ausblenden
help-hint-layers-preview-restore-the-last-hidden-layer = Zuletzt ausgeblendete Ebene einblenden
help-hint-layers-preview-toggle-layer-translucency = Ebenentransparenz umschalten
help-hint-layers-preview-orbit-the-preview-model = Vorschaumodell drehen
help-hint-layers-preview-zoom-the-preview-model = Vorschaumodell zoomen
help-hint-layers-preview-frame-the-preview-model = Vorschaumodell einpassen
help-hint-layers-preview-toggle-preview-wireframe = Vorschau-Drahtgitter umschalten

## Toolbar, empty state — DRAFT.

toolbar-open-label = Öffnen
toolbar-open-hint = 3D-Dateien öffnen ({ $shortcut })
toolbar-recent-hint = Zuletzt verwendet
toolbar-add-label = Hinzufügen
toolbar-add-hint = Weitere Dateien zur Szene hinzufügen
toolbar-cut-label = Schnittansicht
toolbar-cut-hint = Modell entlang einer Ebene schneiden ({ $shortcut })
toolbar-cut-unavailable = Schnittansicht braucht eine sichtbare Ebene
toolbar-ruler-label = Lineal
toolbar-ruler-hint = Abstand messen: zwei Punkte auf dem Modell ({ $shortcut })
toolbar-thickness-label = Dicke
toolbar-thickness-hint = Wandstärke prüfen: Punkt auf der Schale ({ $shortcut })
toolbar-measure-blocked = Mesh-Sitzung zuerst beenden oder abbrechen
toolbar-measure-needs-layer = Messung braucht eine sichtbare Netzebene
toolbar-align-label = Ausrichtung
toolbar-align-hint = Zwei Scans zusammenführen: je ein Punkt ({ $shortcut })
toolbar-edit-label = Bearbeiten
toolbar-edit-open = Netzbearbeitung ist offen
toolbar-edit-hint = Netzbearbeitung: Auswahl und Sculpting ({ $shortcut })
toolbar-settings-label = Einstellungen
toolbar-settings-hint = Einstellungen öffnen

empty-open-file = 3D-Datei öffnen
empty-formats-hint = STL · PLY · OBJ · GLB · HPS — oder Dateien hierher ziehen

## Loading and export — DRAFT.

load-queued = { $count ->
    [one] { $count } Ebene in Warteschlange
   *[other] { $count } Ebenen in Warteschlange
}
load-opening = { $count ->
    [one] { $count } Datei wird geöffnet…
   *[other] { $count } Dateien werden geöffnet…
}
load-adding = { $count ->
    [one] { $count } Datei wird hinzugefügt…
   *[other] { $count } Dateien werden hinzugefügt…
}
load-open-failed-start = Öffnen fehlgeschlagen: Loader startet nicht
load-add-failed-start = Hinzufügen fehlgeschlagen: Loader startet nicht
load-open-failed-stopped = Öffnen fehlgeschlagen: Loader angehalten
load-loader-failed-summary = Szenen-Loader konnte nicht gestartet werden.
load-file-too-large = Die Datei ist { $size } GB groß — mehr als die { $limit } GB, die das Programm am Stück liest
load-action-failed-open = Öffnen fehlgeschlagen: { $detail }
load-action-failed-add = Hinzufügen fehlgeschlagen: { $detail }

export-nothing-visible = Nichts Sichtbares zu speichern
export-unsupported-format = Nicht unterstütztes Format
export-scene-saved = Szene gespeichert: { $path }
export-scene-saved-unmerged = Szene gespeichert (Texturen nicht zusammengeführt): { $path }
export-scene-failed-title = Szene konnte nicht gespeichert werden
export-scene-failed-summary = Szene konnte nicht gespeichert werden: { $detail }
export-layers-saved = { $written ->
    [one] { $written } Ebene nach { $dir } gespeichert
   *[other] { $written } Ebenen nach { $dir } gespeichert
}
export-layers-saved-failed = { $written ->
    [one] { $written } Ebene nach { $dir } gespeichert
   *[other] { $written } Ebenen nach { $dir } gespeichert
}; { $failed ->
    [one] { $failed } konnte nicht geschrieben werden
   *[other] { $failed } konnten nicht geschrieben werden
}
export-layers-saved-renamed = { $written ->
    [one] { $written } Ebene nach { $dir } gespeichert
   *[other] { $written } Ebenen nach { $dir } gespeichert
}; { $renamed ->
    [one] { $renamed } Datei umbenannt, um Bestehendes zu erhalten
   *[other] { $renamed } Dateien umbenannt, um Bestehendes zu erhalten
}
export-layers-saved-failed-renamed = { $written ->
    [one] { $written } Ebene nach { $dir } gespeichert
   *[other] { $written } Ebenen nach { $dir } gespeichert
}; { $failed ->
    [one] { $failed } konnte nicht geschrieben werden
   *[other] { $failed } konnten nicht geschrieben werden
}; { $renamed ->
    [one] { $renamed } Datei umbenannt, um Bestehendes zu erhalten
   *[other] { $renamed } Dateien umbenannt, um Bestehendes zu erhalten
}
mesh-exported-aligned = { $name } in ausgerichteter Position als { $format } exportiert: { $path }
mesh-exported-aligned-warnings = { $name } in ausgerichteter Position als { $format } exportiert (Warnungen: { $warnings }): { $path }
mesh-exported-unmoved = { $name } exportiert (Scan wurde nicht bewegt) als { $format }: { $path }
mesh-exported-unmoved-warnings = { $name } exportiert (Scan wurde nicht bewegt) als { $format } (Warnungen: { $warnings }): { $path }
mesh-warning-vertex-colors = Vertexfarben nicht geschrieben
mesh-warning-uvs = UVs nicht geschrieben
mesh-warning-texture-image = Texturbild nicht geschrieben
mesh-export-warnings = Exportwarnungen: { $warnings }
mesh-export-failed-title = Ebene konnte nicht exportiert werden
mesh-export-failed-summary = Ebene konnte nicht exportiert werden: { $detail }

## Repair card and toasts — DRAFT.

repair-title = Netzreparatur
repair-clean-headline = Nichts zu reparieren — Netz ist sauber
repair-copy-details = Details kopieren
repair-copy-tooltip = Vollständigen Bericht in die Zwischenablage kopieren
repair-line-welded = { $count ->
    [one] { $grouped } doppelten Vertex verschweißt
   *[other] { $grouped } doppelte Vertices verschweißt
}
repair-line-slivers = { $count ->
    [one] { $grouped } Sliver-Fläche entfernt
   *[other] { $grouped } Sliver-Flächen entfernt
}
repair-line-duplicate-faces = { $count ->
    [one] { $grouped } doppelte Fläche entfernt
   *[other] { $grouped } doppelte Flächen entfernt
}
repair-line-nonmanifold = { $count ->
    [one] { $grouped } nicht-mannigfaltige Kante repariert
   *[other] { $grouped } nicht-mannigfaltige Kanten repariert
}
repair-line-bowtie = { $count ->
    [one] { $grouped } Bowtie-Vertex getrennt
   *[other] { $grouped } Bowtie-Vertices getrennt
}
repair-line-reoriented = { $count ->
    [one] { $grouped } Dreieck neu orientiert
   *[other] { $grouped } Dreiecke neu orientiert
}
repair-line-flipped = { $count ->
    [one] { $grouped } umgekrempeltes Teil gewendet
   *[other] { $grouped } umgekrempelte Teile gewendet
}
repair-line-debris = { $count ->
    [one] { $grouped } Trümmerteil entfernt
   *[other] { $grouped } Trümmerteile entfernt
}
repair-line-pinholes = { $count ->
    [one] { $grouped } Pinhole geschlossen
   *[other] { $grouped } Pinholes geschlossen
}
repair-line-unused = { $count ->
    [one] { $grouped } ungenutzten Vertex entfernt
   *[other] { $grouped } ungenutzte Vertices entfernt
}
repair-open-rims = { $count ->
    [one] { $grouped } offener Rand (Scangrenze)
   *[other] { $grouped } offene Ränder (Scangrenze)
}
repair-skipped-rims = { $count ->
    [one] { $grouped } Rand nicht gefüllt (nicht simpel)
   *[other] { $grouped } Ränder nicht gefüllt (nicht simpel)
}
repair-toast-welded = { $count ->
    [one] { $count } Vertex verschweißt
   *[other] { $count } Vertices verschweißt
}
repair-toast-slivers = { $count ->
    [one] { $count } Sliver entfernt
   *[other] { $count } Sliver entfernt
}
repair-toast-duplicate-faces = { $count ->
    [one] { $count } doppelte Fläche
   *[other] { $count } doppelte Flächen
}
repair-toast-nonmanifold = { $count ->
    [one] { $count } nicht-mannigfaltige Kante repariert
   *[other] { $count } nicht-mannigfaltige Kanten repariert
}
repair-toast-bowtie = { $count ->
    [one] { $count } Bowtie getrennt
   *[other] { $count } Bowties getrennt
}
repair-toast-reoriented = { $count ->
    [one] { $count } Dreieck neu orientiert
   *[other] { $count } Dreiecke neu orientiert
}
repair-toast-flipped = { $count ->
    [one] { $count } Teil gewendet
   *[other] { $count } Teile gewendet
}
repair-toast-debris = { $count ->
    [one] { $count } Trümmerteil verworfen
   *[other] { $count } Trümmerteile verworfen
}
repair-toast-pinholes = { $count ->
    [one] { $count } Pinhole geschlossen
   *[other] { $count } Pinholes geschlossen
}
repair-toast-unused = { $count ->
    [one] { $count } Vertex entfernt
   *[other] { $count } Vertices entfernt
}
repair-toast-skipped = { $count ->
    [one] { $count } Rand übersprungen (nicht simpel)
   *[other] { $count } Ränder übersprungen (nicht simpel)
}
repair-toast-done = Repariert { $layer }: { $parts }
repair-toast-clean-rims = Netz bereits sauber: { $layer }, { $count ->
    [one] { $count } offener Rand
   *[other] { $count } offene Ränder
}
repair-toast-clean = Netz bereits sauber: { $layer }
repair-edit-busy = Ebenenbearbeitung läuft bereits
repair-edit-failed-title = Ebene konnte nicht bearbeitet werden
repair-edit-failed-summary = Ebene konnte nicht bearbeitet werden: { $detail }
edit-locked-status = { $status } (nicht rückgängig: Snapshot zu groß)

## Layers overlay, layer menu, scene menu — DRAFT.

layers-title = Ebenen
layers-count = { $count ->
    [one] { $count } Ebene
   *[other] { $count } Ebenen
}
layers-row-hide = Ebene ausblenden
layers-row-show = Ebene einblenden
layers-row-opacity = Ebenendeckkraft
layers-row-remove = Ebene entfernen

layer-menu-next-tint = Nächste Tönung
layer-menu-hide-colors = Scanfarben ausblenden
layer-menu-show-colors = Scanfarben anzeigen
layer-menu-disable-texture = Textur deaktivieren
layer-menu-show-texture = Textur anzeigen
layer-menu-mesh-editing = Netzbearbeitung
layer-menu-split-bridge = Brücke trennen…
layer-menu-repair = Netzreparatur
layer-menu-flip-normals = Normalen umkehren
layer-menu-export = Ebene exportieren…
layer-menu-hide-wireframe = Drahtgitter ausblenden
layer-menu-show-wireframe = Drahtgitter-Overlay
layer-menu-remove = Ebene entfernen

scene-menu-title = Szene
scene-menu-save = Szene speichern unter…
scene-menu-save-each = Jede Ebene speichern…
scene-menu-reset = Positionen zurücksetzen
scene-menu-fit = Ansicht einpassen

## Mesh editor palette — DRAFT.

meshedit-tab-edit = Netzbearbeitung
meshedit-tab-sculpt = Sculpting
meshedit-cancel-session = Sitzung abbrechen (Änderungen werden verworfen)
meshedit-header-edit = Netzbearbeitung
meshedit-section-selection = Auswahl
meshedit-section-edit-selection = Auswahl bearbeiten
meshedit-section-close-holes = Löcher schließen
meshedit-section-sculpt = Sculpting
meshedit-cell-lasso = Lasso
meshedit-cell-lasso-hint = Freihandkontur: Klick setzt Punkte, Doppelklick schließt · Shift hebt auf
meshedit-cell-object = Objekt
meshedit-cell-object-hint = Ganzes Objekt eines mehrteiligen STL anklicken · Shift hebt auf
meshedit-cell-surface = Oberfläche
meshedit-cell-surface-hint = Nur sichtbare Vorderseite markieren
meshedit-cell-through = Durchgehend
meshedit-cell-through-hint = Durch das Netz markieren, inkl. verdeckter Seiten
meshedit-cell-all = Alle
meshedit-cell-all-hint = Alle Flächen markieren (Strg+A)
meshedit-cell-none = Keine
meshedit-cell-none-hint = Markierung löschen
meshedit-cell-invert = Umkehren
meshedit-cell-invert-hint = Markierte und unmarkierte Flächen tauschen
meshedit-cell-delete = Löschen
meshedit-cell-delete-hint = Markierte Flächen löschen
meshedit-cell-crop = Zuschneiden
meshedit-cell-crop-hint = Nur markierten Bereich behalten, Rest entfernen
meshedit-cell-cut = Ausschneiden
meshedit-cell-cut-hint = Markierte Flächen in neues Netz verschieben — Original bleibt
meshedit-cell-separate = Trennen
meshedit-cell-separate-hint = Markierten Bereich pro Zusammenhangskomponente trennen
meshedit-cell-close-holes = Löcher schließen
meshedit-cell-close-holes-hint = Löcher nur schließen, wenn umliegende Flächen gewählt sind. Scangrenzen bleiben offen.
meshedit-sculpt-addremove = Hinzufügen / Entfernen  [1]
meshedit-sculpt-addremove-hint = Material per Ziehen aufbauen; Shift trägt ab. Shift+Rad skaliert, Strg+Rad ändert Stärke. Taste: 1.
meshedit-sculpt-smooth = Glätten  [2]
meshedit-sculpt-smooth-hint = Oberfläche per Ziehen entspannen; Shift erzwingt maximale Glättung. Shift+Rad skaliert, Strg+Rad ändert Stärke. Taste: 2.
meshedit-slider-size = Größe
meshedit-slider-size-hint = Pinselgröße (Shift + Mausrad)
meshedit-slider-force = Stärke
meshedit-slider-force-hint = Pinselstärke (Strg + Mausrad)
meshedit-limit-label = Limit
meshedit-limit-checkbox-hint = Reparatur auf Ränder bis zu diesem Umfang beschränken
meshedit-limit-drag-hint = Aus schließt alle sicheren Löcher im Bereich; Scangrenze bleibt offen
meshedit-status-unsaved = Ungespeicherte Änderungen
meshedit-status-unsaved-hint = Offene Änderungen: Fertig übernimmt, Abbrechen verwirft
meshedit-status-hint-sculpt = Auf der Oberfläche ziehen zum Sculpten · RMB dreht
meshedit-status-hint-object = Objekt anklicken für Gesamtauswahl · Shift hebt auf
meshedit-status-hint-lasso = Klick umreißt · Doppelklick schließt · Shift hebt auf
meshedit-status-hint-default = Kasten ziehen zum Markieren · Shift abwählen · Entf löscht
meshedit-session-undo = Zurück
meshedit-session-undo-hint = Letzte Netzbearbeitung rückgängig (Strg+Z)
meshedit-session-redo = Wiederholen
meshedit-session-redo-hint = Rückgängige Bearbeitung wiederholen (Strg+Y)
meshedit-session-cancel = Abbrechen
meshedit-session-cancel-hint = Alle Änderungen der Sitzung verwerfen
meshedit-session-done = Fertig
meshedit-session-done-hint = Änderungen übernehmen und Editor schließen

## Align Scans window — DRAFT.

align-title = Scans ausrichten
align-tab-auto = Ausrichten
align-tab-manual = Position anpassen
align-constraint-free = In alle Richtungen bewegen/drehen
align-constraint-free-hint = Scan in beliebige Richtung ziehen
align-constraint-z = In z-Richtung bewegen
align-constraint-z-hint = Nur entlang der Vertikalen ziehen
align-constraint-xy = In xy-Ebene bewegen
align-constraint-xy-hint = Nur in der horizontalen Ebene ziehen
align-manual-drag-hint = Bewegt den gegriffenen Scan · Strg+Ziehen dreht ihn um den gegriffenen Punkt
align-undo = Zurück
align-undo-hint = Einen Schritt zurück
align-redo = Wiederholen
align-redo-hint = Einen Schritt vor
align-prompt-moving = Punkt auf dem zu bewegenden Netz anklicken
align-prompt-other = Dieselbe Position auf dem anderen Netz anklicken
align-prompt-alternate = Abwechselnd Punkte an gleichen Positionen beider Netze anklicken
align-prompt-placed = { $count ->
    [one] { $count } Pfeil gesetzt
   *[other] { $count } Pfeile gesetzt
}
align-back = Zurück
align-back-hint = Pfeil rückgängig — Rechtsklick in der Ansicht tut dasselbe
align-clear = Leeren
align-clear-hint = Alle Pfeile verwerfen und zwei Scans neu wählen — Scans bleiben wo sie sind
align-fit-perform = Ausrichtung durchführen
align-fit-perform-hint = Netz auf die Pfeile bewegen — braucht mindestens zwei Pfeile
align-fit-refine = Best-Fit-Matching
align-fit-refine-hint = Unveränderte Bereiche des präparierten Scans am Originalmodell ausrichten. Ergebnis vor dem Bestätigen prüfen
align-matching-parts = passende Teile
align-matching-parts-hint = Maximaler Anteil der Treffer für die Verfeinerung. Best Fit senkt ihn bei wenigen unveränderten Bereichen automatisch
align-max-influence = max. Einfluss
align-max-influence-hint = Nur Oberfläche unterhalb dieser Distanz beeinflusst das Matching. Großer Wert kann verschlechtern
align-orientation-title = Flächenorientierung muss übereinstimmen
align-orientation-match = Flächenorientierung muss übereinstimmen
align-orientation-inverted = Flächenorientierung muss invertiert übereinstimmen
align-orientation-ignored = Flächenorientierung wird ignoriert
align-orientation-either-hint = Akzeptiert beide Richtungen. Berechnung dauert oft deutlich länger
align-orientation-facing-hint = Wie die beiden Oberflächen zueinander stehen
align-exclude = Matching: Gewählte Teile ausschließen
align-exclude-hint = Oberfläche malen, die das Matching ignorieren muss
align-commit-cancel = Abbrechen
align-commit-cancel-hint-moved = Alle Scans zurücksetzen und schließen — Strg+Z holt die Ausrichtung zurück
align-commit-cancel-hint-clean = Schließen ohne etwas zu ändern
align-commit-done = Fertig
align-commit-done-hint = Ausrichtung behalten und schließen — Scan exportieren zum Speichern

## Deviation map — DRAFT.

align-map-heatmap = Heatmap
align-map-heatmap-hint = Einen Scan nach Abstand zum anderen einfärben
align-map-requires-refine = Zuerst „Best fit matching“ ausführen
align-map-max = max
align-map-min = min
align-map-not-measured = nicht gemessen
align-map-not-measured-hint = Keine Oberfläche des anderen Scans in Reichweite dieser Vertices. Zahn oder Brücke nur auf einem Scan ist der übliche Grund, kein Fehler — dort gibt es nichts zu messen.

## Align roles, brush, mask commands, align status lines — DRAFT.

align-pair-decided = { $moving } → { $fixed }
align-pair-guessed = { $moving } → { $fixed } (Vermutung)
align-pair-hint-decided = { $moving } bewegt sich, { $fixed } bleibt
align-pair-hint-guessed = Noch nichts angeklickt, daher aus Dateireihenfolge geraten. Erster Klick entscheidet: { $moving } bewegt sich, { $fixed } bleibt
align-pair-swap = Tauschen
align-pair-swap-hint = Andersherum fitten — Pfeile wandern mit

align-brush-title = Pinsel
align-brush-close-hint = Pinsel schließen — Markierungen bleiben
align-brush-mesh-selection = Mesh-Auswahl
align-brush-moving = Beweglich
align-brush-fixed = Fest
align-brush-both = Beide
align-brush-both-hint = Beide Scans bemalen und bearbeiten — der Scan unter dem Cursor nimmt den Strich an
align-brush-size = Pinselgröße
align-brush-inverse = Pinsel invertieren
align-brush-inverse-hint = Einfaches Ziehen löscht statt zu markieren. Shift kehrt wieder um
align-brush-auto-radius = Automatischer Radius
align-brush-auto-radius-hint = Radius des Netzbereichs an jedem Pfeilende
align-brush-size-status = Pinsel { $size } mm
align-status-no-summary = Keine vergleichbare Oberfläche

align-mask-fit-everywhere = Überall fitten
align-mask-fit-everywhere-hint = Alle Markierungen löschen
align-mask-fit-everywhere-report = Markierungen gelöscht — Matching auf dem ganzen Scan
align-mask-fit-everywhere-report-one = { $name }: Markierungen gelöscht
align-mask-fit-nowhere = Nirgends fitten
align-mask-fit-nowhere-hint = Ganzes Netz markieren — Matching ohne Wirkung
align-mask-fit-nowhere-report = Ganzes Netz markiert — Matching ohne Wirkung
align-mask-fit-nowhere-report-one = { $name }: ganzer Scan vom Abgleich ausgenommen
align-mask-invert = Markierungen invertieren
align-mask-invert-hint = Unmarkierte Bereiche markieren und umgekehrt
align-mask-invert-report = Markierungen invertiert
align-mask-invert-report-one = { $name }: Markierungen umgekehrt
align-mask-automatic = Automatisch markieren
align-mask-automatic-hint = Nur kleine Bereiche an den Pfeilenden matchen
align-mask-automatic-report = Matching nur an den Pfeilenden
align-mask-automatic-report-one = { $name }: Abgleich nur um die Pfeilenden
align-mask-automatic-empty = Überall zuordnen: der Bereich bedeckte den ganzen Scan, es wurde nichts ausgeschlossen

align-status-half-dropped = Halb gesetzter Pfeil verworfen
align-status-turned = Paar umgedreht
align-status-cleared = Paar geleert
align-status-click-moving = Punkt auf dem zu bewegenden Scan anklicken
align-status-click-alternate = Abwechselnd Punkte an gleichen Positionen beider Scans anklicken
align-status-two-scans = Zwei Scans in Ansicht — je einen Punkt zum Paaren anklicken
align-status-no-surface = Punktwolke hat keine Oberfläche zum Paaren
align-status-now-other = Jetzt passende Stelle auf dem anderen Scan anklicken
align-status-moved = Punkt verschoben
align-status-wrong-scan = Dieser Scan gehört nicht zum Paar — Leeren und von vorn
align-status-one-scan = Einer der Scans
align-status-place-first = Erst je einen Punkt pro Scan setzen
align-status-scaled = Dieser Scan trägt eine skalierte Platzierung, nicht ausrichtbar
align-status-pose-refused = Fit fertig, aber der Scan dafür ist nicht mehr verfügbar
align-status-worker-unavailable = Align-Worker angehalten — Alignment-Tool neu starten
align-status-measure-dropped = Messung verworfen — Pinsel besitzt die Farben
align-status-measure-unavailable = Messung nicht angewendet — Scan geändert; Best fit matching erneut ausführen
align-status-map-elsewhere = Distanzkarte liegt auf dem Automatik-Tab — dort kommt sie zurück
align-status-aligned-points = Auf Punkten ausgerichtet

## Align result status lines — DRAFT.

align-status-aligned = Nach Punkten ausgerichtet — zuerst „Best fit matching“ ausführen.
align-status-refined = Best fit bereit
align-status-measured = Heatmap aktualisiert
align-status-remeasure = { $reason } — Best-Fit-Matching erneut laufen lassen
align-status-settings-changed = Matching-Einstellungen geändert
align-status-visibility-changed = Sichtbarkeit eines ausgewählten Scans geändert
align-drag-moving = { $name } wird von Hand bewegt
align-drag-unrecorded = Von Hand bewegt, aber dieser Schritt landete nicht in der Historie — Strg+Z macht ihn nicht rückgängig
align-drag-moved = { $name } wurde { $moved } mm von Hand bewegt (Strg+Z macht rückgängig)
align-status-moved-hand = Von Hand bewegt
align-pair-placed = Paar { $n } gesetzt
align-roles-swapped = { $moving } bewegt sich jetzt, { $fixed } bleibt
align-status-scan-changed = Der Scan hat sich geändert
align-status-hidden = { $name } ist ausgeblendet — einblenden, um danach auszurichten
align-arrow-removed = { $n ->
    [one] Pfeil entfernt — { $n } Paar übrig
   *[other] Pfeile entfernt — { $n } Paare übrig
}
align-status-markings-changed = Markierungen geändert
align-status-place-arrow-first = Erst mindestens einen Pfeil setzen, dann automatisch markieren
align-status-arrows-cleared = Pfeile weg — ab hier von Hand

## Unsaved-work guards and error dialog buttons — DRAFT.

guard-close-title = Ungespeicherte Netzänderungen
guard-close-headline-one = 1 bearbeitete Ebene ist nicht gespeichert.
guard-close-headline-many = Bearbeitete Ebenen sind nicht gespeichert.
guard-close-note = { $count } bearbeitete Ebenen betroffen.
guard-close-detail = Speichern exportiert jede bearbeitete Ebene (PLY, STL oder OBJ) und schließt dann.
guard-close-destructive = Ohne Speichern schließen
guard-replace-title = Bearbeitung läuft
guard-replace-headline-session = Auf { $layer } läuft eine Bearbeitungssitzung.
guard-replace-headline-one = 1 bearbeitete Ebene mit ungespeicherten Änderungen.
guard-replace-headline-many = { $count } bearbeitete Ebenen mit ungespeicherten Änderungen.
guard-replace-detail = Szenenöffnung schließt die Sitzung und verwirft ungespeicherte Änderungen.
guard-replace-destructive = Verwerfen und öffnen
guard-save = Speichern…
guard-cancel = Abbrechen

error-retry-graphics = Erneut versuchen
error-close = Schließen
error-copy-details = Details kopieren

about-website = Website
about-source = Quellcode
about-licenses = Drittlizenzen
about-license-kind = Apache License 2.0

## Mesh-edit operations, undo/redo, sculpt, measure, cut ruler, scene menu — DRAFT.

edit-select-faces-first = Erst Netzflächen auswählen
edit-no-changes = Keine Änderungen: { $layer }
edit-apply-failed-title = Auswahl konnte nicht bearbeitet werden
edit-apply-failed-summary = Auswahl konnte nicht bearbeitet werden: { $detail }
edit-no-changes-hidden = Keine Änderungen: Auswahl verfeinern; ausgeblendete Ebenen bleiben unberührt
edit-selected-faces = { $faces ->
    [one] { $faces } Fläche ausgewählt
   *[other] { $faces } Flächen ausgewählt
}
edit-selected-faces-across = { $faces ->
    [one] { $faces } Fläche auf { $layers } Ebenen ausgewählt
   *[other] { $faces } Flächen auf { $layers } Ebenen ausgewählt
}

holes-nothing = Nichts zu schließen: { $layer }
holes-partial = { $segments }, nichts geschlossen: { $layer }
holes-closed = { $filled ->
    [one] { $filled } Loch geschlossen
   *[other] { $filled } Löcher geschlossen
}
holes-closed-detail = { $closed }: { $layer }
holes-closed-segments = { $closed } ({ $segments }): { $layer }
holes-seg-healed = { $n ->
    [one] { $n } Kerbe geheilt
   *[other] { $n } Kerben geheilt
}
holes-seg-border = Scangrenze offen gelassen
holes-seg-oversize-limit = { $n ->
    [one] { $n } Loch über dem Limit von { $limit } mm
   *[other] { $n } Löcher über dem Limit von { $limit } mm
}
holes-seg-oversize = { $n ->
    [one] { $n } Loch zu groß
   *[other] { $n } Löcher zu groß
}
holes-seg-damaged = { $n ->
    [one] { $n } beschädigter Rand übersprungen
   *[other] { $n } beschädigte Ränder übersprungen
}
batchedit-delete = Auswahl gelöscht
batchedit-crop = Auf Auswahl zugeschnitten
batchedit-cut = Auswahl in neue Ebene geschnitten
batchedit-separate = Auswahl getrennt
batchedit-invert = Normalen umgekehrt
batch-close-holes = Sichere Innenlöcher geschlossen
batch-delete = Auswahl gelöscht
batch-crop = Auf Auswahl zugeschnitten
batch-cut = Auswahl geschnitten
batch-separate = Auswahl getrennt
batch-edited = Auswahl bearbeitet
batchedit-edited = Ebene bearbeitet
edit-applied-status = { $action }: { $layer }
batchedit-status = { $label } auf { $n ->
    [one] { $n } sichtbarer Ebene
   *[other] { $n } sichtbaren Ebenen
}

select-covers-all = Auswahl deckt bereits das ganze Netz ab: { $layer }
select-covers-remove = Auswahl deckt das ganze Netz ab — stattdessen Ebene entfernen: { $layer }
select-splits = Auswahl zerfällt in { $parts } Teile — Auswahl verfeinern: { $layer }
select-faces-cannot = Flächen nicht wählbar: { $layer }

undo-nothing = Nichts rückgängig zu machen
redo-nothing = Nichts wiederherzustellen
undo-undid = Netzbearbeitung rückgängig: { $layer }
undo-unavailable = Undo nicht möglich — Szene hat sich seitdem geändert: { $layer }
redo-redid = Netzbearbeitung wiederholt: { $layer }
redo-unavailable = Redo nicht möglich — Szene hat sich seitdem geändert: { $layer }

sculpt-armed-addremove = Hinzufügen/Entfernen: ziehen zum Aufbauen, Shift trägt ab
sculpt-armed-smooth = Glätten: ziehen zum Entspannen, Shift forciert
sculpt-off = Sculpting aus
sculpt-applied-undo = Sculpting angewendet (Strg+Z macht rückgängig)
sculpt-applied-locked = Sculpting angewendet (nicht rückgängig: Snapshot zu groß)
sculpt-failed-title = Sculpt fehlgeschlagen
sculpt-failed = Diese Ebene lässt sich nicht sculpten: { $detail }
sculpt-worker-stopped = Sculpt-Worker angehalten: { $detail }
sculpt-preparing = Sculpting wird vorbereitet…
sculpt-nonuniform-scale = Sculpting benötigt eine gleichmäßig skalierte Mesh
sculpt-failure-worker-panicked = Sculpt-Worker abgestürzt: { $detail }
sculpt-failure-spawn = Sculpt-Worker konnte nicht gestartet werden: { $detail }
sculpt-failure-kernel-pool = Sculpt-Kernel-Pool konnte nicht erstellt werden: { $detail }
sculpt-failure-missing-undo-baseline = Für den Sculpt-Strich gibt es keine Undo-Basis
sculpt-failure-shadow-poisoned = Sculpt-Shadow-Sperre ist vergiftet
sculpt-failure-shadow-shape = Sculpt-Shadow entspricht nicht mehr dem Live-Mesh
sculpt-failure-invalid-vertex-index = Sculpt-Worker lieferte einen ungültigen Vertex-Index
sculpt-failure-worker-state-poisoned = Sculpt-Workerstatus beschädigt — Sculpt neu starten
sculpt-failure-vertex-count-changed = Das Sculpt-Ergebnis hat die Vertex-Anzahl verändert
sculpt-failure-topology-rebuild = Wiederherstellung der Sculpt-Topologie fehlgeschlagen: { $detail }
sculpt-worker-unavailable = Sculpt-Worker nicht verfügbar
sculpt-finishing = Stroke wird fertiggestellt…
sculpt-finishing-history = Sculpting vor Historienwechsel fertigstellen…
sculpt-lasso-armed = Lasso aktiv: Klick oder Ziehen umreißt; Enter, Doppelklick oder Startklick schließt
sculpt-lasso-off = Lasso aus
sculpt-object-on = Objektauswahl: Objekt für Gesamtauswahl anklicken
sculpt-object-off = Objektauswahl aus
sculpt-selection-cleared = Auswahl gelöscht
sculpt-through-on = Netzdurchgreifende Auswahl
sculpt-through-off = Oberflächenauswahl

## Session close-outs and layer shortcuts. — DRAFT.
session-applied = Netzbearbeitungssitzung übernommen
session-reverted = Netzbearbeitungssitzung verworfen
edit-session-busy = Erst Bearbeitungssitzung beenden oder abbrechen
layers-none-hidden = Keine ausgeblendeten Ebenen
layer-opaque-again = Wieder opak: { $label }
layer-translucent = Transparent: { $label } (Shift+Mittelklick stellt wieder her)
layer-restored = Wieder eingeblendet: { $label }
layer-hidden = Ausgeblendet: { $label } (Shift+Strg+Mittelklick stellt wieder her)
layer-unnamed = Ebene { $n }
layer-removed = Ebene entfernt: { $label }
layer-face-selection = Flächenauswahl: { $label }

measure-distance = Abstand: { $len }
measure-perpendicular = Lot: { $len }
measure-to-line = Bis zur Linie: { $len }, Winkel { $angle }
measure-line-angle = Ende auf einer Linie
measure-line-angle-free = Beliebiger Winkel
measure-line-angle-right = 90°
measure-line-angle-hint = Wie ein Lineal auf die Messlinie eines anderen Lineals trifft. Bei beliebigem Winkel liegt das Ende dort, wo Sie auf die Linie klicken, und der Winkel wird angezeigt; bei 90° ist es der Lotfußpunkt.
measure-line-angle-shift = wechselt, solange gedrückt
measure-thickness = Wandstärke: { $len }
measure-open-wall = Offene Oberfläche: keine Gegenwand entlang der Innennormale
measure-cannot-probe = Hier nicht messbar: degenerierte Geometrie
measure-cleared = Messungen gelöscht

cut-lines = Linien
cut-mesh = Netz
cut-dist = Dist
cut-dist-hint = Abstand: zwei Punkte anklicken
cut-thick = Dicke
cut-thick-hint = Wandstärke: einen Konturpunkt anklicken
cut-close-section = Schnitt schließen
cut-snap = Fang
cut-snap-hint = Magnet: Klicks rasten auf der Schnittkontur ein
cut-empty = Keine Schnittmenge
cut-footer-distance = Ziehen = Schwenken · Klick 2 Pkte = Abstand · Rechtsklick löscht · Rad = Zoom
cut-footer-thickness = Ziehen = Schwenken · Klick Kontur = Wandstärke · Rechtsklick löscht · Rad = Zoom

## Worker-built align failures — DRAFT.

align-fail-no-surface-fixed = Fixer Scan ohne brauchbare Oberfläche
align-fail-no-surface-moving = Bewegter Scan ohne brauchbare Oberfläche
align-fail-recolor = Messung vor dem Einfärben verworfen
align-fail-unobservable = Die Oberfläche reicht für eine verlässliche Abweichungskarte nicht aus
align-reject-toofew = Weitere Pfeilpaare setzen oder Scans näher platzieren
align-reject-unpaired = Beide Seiten jedes Pfeilpaares vervollständigen
align-reject-degenerate-plain = Matching-Punkte über die Fläche verteilen
align-reject-unit = Scans verwenden unterschiedliche Einheiten
align-reject-apart = Pfeilpaare prüfen und Scans näher platzieren
align-reject-runaway = Scans näher platzieren und Best-Fit-Matching erneut ausführen
align-reject-no-improvement = Best-Fit konnte keine Verbesserung bestätigen — Scans näher platzieren und erneut versuchen
align-reject-ambiguous = Best-Fit fand mehrere gleich plausible Flächen — passende Bereiche markieren oder Scans näher platzieren
align-reject-nonfinite = Ausgewählter Punkt oder Oberfläche ist ungültig
align-status-stepped = Durch Historie gegangen
align-status-moving-hand = Von Hand bewegt

## Bridge split panel and align session close-outs — DRAFT.

bridge-panel-title = Brückentrennung
bridge-mode-place = Scheibe setzen
bridge-mode-calculating = Berechnet…
bridge-mode-ready = Bereit
bridge-mode-failed = Trennversuch fehlgeschlagen
bridge-kerf = Schnittfuge
bridge-disc-size = Scheibengröße
bridge-cancel = Abbrechen
bridge-apply = Brücke trennen
bridge-err-miss = Scheibe verfehlt die Brücke. In den Konnektor schieben.
bridge-err-tangent = Scheibe berührt nur die Oberfläche. Durch den Konnektor schieben.
bridge-err-small = Scheibendurchmesser { $have } mm; mindestens { $need } mm nötig.
bridge-err-limit = Dieser Schnitt braucht eine { $need }-mm-Scheibe, über dem { $max }-mm-Limit.
bridge-err-no-result = Trennung mit erhaltener Quellfläche versucht, kein brauchbares Ergebnis. Originalnetz behalten.
bridge-err-invalid-cut = Trennung versucht, aber Schnitt nicht validierbar. Originalnetz behalten.
bridge-err-invalid-side = Trennung versucht, aber { $side } nicht validierbar. Originalnetz behalten.
bridge-err-gap = Trennung versucht, aber Spalt nicht haltbar. Originalnetz behalten.
bridge-err-empty = Gewählte Ebene hat kein Dreiecksnetz zum Trennen.
bridge-err-invalid = Scheibeneinstellungen ungültig. Tool zurücksetzen und erneut versuchen.
bridge-err-unusable = Trennung ohne brauchbares Ergebnis. Originalnetz behalten.

align-session-canceled = Ausrichtung abgebrochen — alle Scans zurück (Strg+Z holt sie zurück)
align-session-closed = Ausrichtung geschlossen
align-session-closed-running = Ausrichtung geschlossen — Fit lief noch und wurde verworfen, Scans wie zuletzt gesehen
align-session-kept = Ausrichtung behalten — Scan speichern für die Platte

recent-clear = Zuletzt verwendete löschen

scene-already-origin = Alle Ebenen bereits in Ausgangsposition
scene-positions-reset = Ebenenpositionen zurückgesetzt (Strg+Z macht rückgängig)

## Settings panel, bridge split, render error, tint — DRAFT.

settings-header = Einstellungen
settings-section-files = Dateien & Export
settings-remember-export = Exportordner merken
settings-remember-export-hint = Gleichen Ordner nach Neustart verwenden
settings-section-scene = Ansicht & Navigation
settings-frame-on-open = Szene beim Öffnen einpassen
settings-frame-on-open-hint = Kamera auf Heimansicht zurücksetzen, wenn eine neue Datei die Szene ersetzt
settings-double-click = Doppelklick zentriert Ansicht
settings-double-click-hint = Doppelklick zentriert die Kamera auf den gewählten Punkt
settings-orbit = Orbitgeschwindigkeit
settings-orbit-hint = Wie schnell die Ansicht bei gedrückter rechter Maustaste kreist
settings-zoom = Zoomgeschwindigkeit
settings-zoom-hint = Wie stark jede Raststufe zoomt
settings-background = Hintergrund
settings-bg-gray = Grau
settings-bg-white = Weiß
settings-bg-dark = Dunkel
settings-ghost = Abgetrennte Seite als Geist
settings-ghost-hint = In der Schnittansicht die entfernte Seite transparent zeigen
settings-measurements = Messungen
settings-section-appearance = Darstellung
settings-theme = Design
settings-theme-light = Hell
settings-theme-dark = Dunkel
settings-scale = UI-Skalierung
settings-scale-hint = Skaliert alle Elemente; 1.0 behält Systemstandard
settings-section-mesh = Netzbearbeitung
settings-remember-brush = Sculpt-Pinsel merken
settings-remember-brush-hint = Größen- und Stärkeregler zwischen Sitzungen behalten
settings-section-updates = Updates
settings-check-auto = Beim Start automatisch prüfen
settings-check-now = Jetzt prüfen
settings-check-disabled-hint = Updateprüfungen sind per Umgebung deaktiviert
settings-check-busy-hint = Eine Updateprüfung läuft bereits
settings-update-disabled = Per Umgebung deaktiviert
settings-update-checking = Prüfe…
settings-update-current = Aktuell
settings-update-skipped = Version übersprungen
settings-update-failed = Konnte nicht prüfen
settings-save-error = Einstellungen konnten nicht gespeichert werden. Neuer Versuch…
settings-save-error-hint = Einstellungsdatei derzeit nicht verfügbar
settings-shortcuts = Tastenkürzel
settings-about = Über OccluView

bridge-busy = Brückentrennung erst beenden oder abbrechen
bridge-active = Brückentrennung bereits aktiv
bridge-target-gone = Ziel der Brückentrennung nicht mehr verfügbar
bridge-needs-mesh = Brückentrennung braucht ein sichtbares Dreiecksnetz
bridge-place-disc = Brückentrennung: Trennscheibe setzen
bridge-canceled-scene = Brückentrennung abgebrochen: Szene geschlossen
bridge-canceled-camera = Brückentrennung abgebrochen: Kamera nicht verfügbar
bridge-canceled-changed = Brückentrennung abgebrochen: Quellnetz geändert
bridge-canceled = Brückentrennung abgebrochen
bridge-calculating = Brückentrennung: berechnet…
bridge-unavailable = Brückentrennung vorübergehend nicht verfügbar
bridge-preview-stale = Vorschau der Trennung veraltet
bridge-not-applied = Brückentrennung nicht angewendet
bridge-complete = Brückentrennung abgeschlossen
bridge-complete-surface = Brückentrennung abgeschlossen (Oberflächenergebnis; natürliche Ränder erhalten)
bridge-complete-locked = Brückentrennung abgeschlossen (nicht rückgängig: Snapshot zu groß)

render-failed-title = Szene konnte nicht gerendert werden
render-failed-summary = Datei geöffnet, aber Viewport nicht renderbar.
render-failed-status = Rendern fehlgeschlagen

tint-choose = Tönung wählen

## Status tail: brush, lasso, loading, GPU, align jobs. — DRAFT.
brush-no-mesh = Je einen Punkt auf beiden Netzen anklicken, dann malen
lasso-dropped = Lassokontur verworfen
lasso-needs-points = Lasso braucht mindestens 3 Punkte
loading-scene = Szene lädt…
gpu-failed-status = Grafiktreiber meldet ein Problem
gpu-retry-status = Grafik wird erneut versucht — bleibt das Problem, Arbeit speichern und OccluView neu starten
gpu-failed-title = Grafikproblem
gpu-failed-summary = Der Grafiktreiber meldet ein Problem beim Zeichnen. Die Ansicht kann unvollständig sein. Arbeit speichern und OccluView neu starten, falls es wiederkehrt.
align-job-align = Richte aus…
align-job-refine = Verfeinere…
align-job-measure = Messe…
align-markings-dropped = Markierungen verworfen — Scanoberfläche hat sich seitdem geändert

## Okklusalkontakte: Rechtsklick auf einen Scan und lesen, wo er den Gegenscan
## berührt. Die eine Lesart ist Artikulationspapier (nur Marken, nach Tiefe
## gefärbt), die andere die Annäherungskarte (wie nah, überall). Ein Regler
## verschiebt die Tiefe, ab der die Skala als voll belastet gilt, und färbt eine
## bereits gemessene Karte neu, statt neu zu messen.
layer-menu-contacts = Kontakte anzeigen
layer-menu-hide-contacts = Kontakte ausblenden

contact-title = Okklusalkontakte
contact-close-hint = Lesung schließen und die Marken von beiden Scans nehmen
contact-against = { $subject } gegen { $antagonist }
contact-unknown-layer = ein Scan, der nicht mehr geöffnet ist

contact-mode-marks = Kontakte
contact-mode-marks-hint = Wo die Flächen sich berühren, nach Stärke gefärbt — der Rest bleibt frei, wie Artikulationspapier ihn lässt
contact-mode-approach = Annäherung
contact-mode-approach-hint = Wie nah der Gegenscan überall ist, Belastung eingeschlossen

contact-load-label = belastet ab
contact-load-suffix = mm
contact-load-hint = Die Tiefe, ab der diese Skala als voll belastet gilt. Verschieben färbt die bereits gemessene Karte neu — ohne neue Messung.
contact-flatten = Eine Farbe je Kontakt
contact-flatten-hint = Jede Kontaktfläche auf ihren tiefsten Punkt reduzieren. Aus behält die Kraftverteilung innerhalb jeder Marke.
# Label above the list of layers a contact reading can be measured against.
contact-antagonist-pick = Gemessen gegen
contact-antagonist-pick-hint = Der nächste Scan wird automatisch gewählt. Wählen Sie eine andere Ebene, um dagegen zu messen.

contact-legend-deepest = { $mm } mm in den Biss

contact-stats-area = Kontaktfläche
contact-stats-contacts = Kontakte
contact-stats-deepest = Am tiefsten

contact-readout-gap = Abstand
contact-readout-load = Belastung

contact-status-measuring = Messung…
contact-status-measuring-hint = Die beiden Flächen werden gegeneinander gelesen
contact-status-remeasuring = Erneute Messung…
contact-status-remeasuring-hint = Ein Scan hat sich bewegt, die Abstände haben sich geändert. Die Karte wird neu gelesen.
contact-status-needs-second = Eine Kontaktlesung braucht einen zweiten sichtbaren Scan zum Messen
contact-status-no-surface = Einer der beiden Scans hat keine Fläche zum Messen
contact-status-worker-failed = Die Messung wurde nicht abgeschlossen

contact-opened = Kontakte auf { $label } werden gelesen
contact-closed = Kontaktlesung geschlossen
help-section-contacts = Okklusalkontakte
help-hintline-contacts = Rechtsklick auf eine Ebene · Kontakte anzeigen · Regler „belastet ab“ färbt neu · Esc schließt
help-hint-contacts-read-its-occlusal-contacts-against-the-scan-it-bites = Die Okklusalkontakte gegen den Gegenscan lesen
help-hint-contacts-read-the-contact-depth-under-the-cursor = Die Kontakttiefe unter dem Zeiger lesen, auf beiden Zahnbögen
help-hint-contacts-move-the-depth-the-ramp-calls-fully-loaded = Die Tiefe verschieben, ab der die Skala voll belastet heißt
help-hint-contacts-switch-between-marks-only-and-the-whole-approach = Zwischen nur Marken und der ganzen Annäherung wechseln
help-hint-contacts-close-the-reading-and-take-the-marks-off-both-scans = Die Lesung schließen und die Marken von beiden Scans nehmen
contact-retry = Erneut lesen
contact-status-subject-unusable = Der Scan, um den es in dieser Lesung geht, lässt sich gerade nicht messen
contact-status-subject-unusable-hint = Wieder einblenden oder als Dreiecksnetz belassen — die Lesung läuft weiter
contact-status-antagonist-unusable = Der Scan, gegen den gemessen wird, lässt sich gerade nicht messen
contact-status-antagonist-unusable-hint = Wieder einblenden oder als Dreiecksnetz belassen — die Lesung läuft weiter
contact-status-no-overlap = Die Scans liegen zu weit auseinander
contact-status-no-overlap-hint = Nichts auf beiden Flächen lag in Reichweite der Lesung. Prüfen, ob die Scans in Okklusion stehen.
contact-status-failed-hint = Erneut lesen; scheitert es weiter, muss das Paar eventuell zuerst repariert werden.
contact-status-needs-second-hint = Den Gegenscan öffnen oder wieder einblenden und die Lesung starten
contact-legend-gap = Abstand bis { $mm } mm
contact-stats-balance = Fläche je Seite
layer-menu-contacts-unavailable = Eine Kontaktlesung braucht zwei sichtbare Dreiecksnetze — zuerst den Gegenscan einblenden oder öffnen

contact-details = Details
contact-details-hint = Die Zahlen und die Regel „eine Farbe pro Kontakt“
contact-details-close = Details ausblenden
settings-shortcuts-hint = Tastatur- und Mausreferenz (F1)

load-units-ambiguous = Einheiten nicht bestätigt: Das Format deklariert Meter, Scanner exportieren aber meist Millimeter — { $suggestion }
load-units-suggest-meters = die Größe deutet auf Meter hin, das Modell ist also etwa 1000x zu klein
load-units-suggest-millimeters = die Größe deutet auf Millimetrzahlen hin, so wurde es gelesen
load-units-unclear = die Größe entscheidet es nicht; prüfen Sie mit einem bekannten Maß
contact-stats-balance-hover = Kontaktfläche geteilt an der Mittellinie: vor / hinter der Linie
mesh-warning-vertex-alpha = Vertex-Alpha wurde nicht geschrieben
load-superseded-parked-open = Eine neuere Datei wartet: beantworten Sie zuerst deren Abfrage

## OccluView French catalog — DRAFT (machine draft, native review required).
## Status: DRAFT. Requires native dental/CAD terminology review + visual UI review before APPROVED.
## Contract: exact key/attribute/variable parity with en.ftl.

app-title = OccluView
app-window-title = OccluView 3D Viewer
align-panel-title = Aligner les scans
meshedit-window-title = Édition de maillages

settings-language-label = Langue
settings-language-auto = Langue du système
settings-language-auto-current = Langue du système — { $language }
settings-language-catalog-fallback = { $tag } n’est pas disponible ; l’anglais est utilisé.
settings-language-save-error = Langue non enregistrée. Nouvel essai…

about-title = À propos d’OccluView
about-tagline = Réparation de maillages · Édition pour la CFAO dentaire
about-version = Version { $version }

update-available-title = Mise à jour disponible
update-available-body = La version { $version } est prête à installer.
update-current-version = Vous avez la { $version }.
update-download = Télécharger la mise à jour
update-open-release = Ouvrir la page de version
update-later = Plus tard
update-skip = Ignorer cette version
update-skip-tooltip = Ne plus proposer cette version ; la suivante sera proposée
update-downloading = Téléchargement d’OccluView { $version }
update-ready-title = OccluView { $version } prêt à installer
update-ready-hint-windows = Programme vérifié. OccluView se fermera pendant l’installation Windows.
update-ready-hint-other = Paquet vérifié. L’installateur système va s’ouvrir — confirmez-y.
update-install-close = Installer et fermer
update-failed-title = Échec de la mise à jour
update-dismiss = Rejeter

error-open-title = Impossible d’ouvrir le fichier
error-add-title = Impossible d’ajouter le fichier
error-open-body = Ouverture impossible de { $path }.

## Help surface — DRAFT. Gesture names stay invariant by contract.

help-title = Commandes clavier et souris
help-subtitle = Cette référence correspond aux commandes d’OccluView.
help-close = Fermer
help-toggle = Aide
help-toggle-tooltip = Afficher les commandes clavier et souris

help-section-navigation = Navigation
help-section-tools = Outils
help-section-mesh-editing = Édition de maillages
help-section-sculpt = Sculpture
help-section-align-measure = Alignement et mesure
help-section-cut-view = Vue en coupe
help-section-layers-preview = Calques et aperçu de l’Explorateur

help-hintline-navigation = Glisser BRD orbite · BMM panoramique · molette zoom · clic BMM focus
help-hintline-mesh-editing = Clic BG sélectionne · Shift+clic désélectionne · rectangle · Ctrl+Z annule
help-hintline-sculpt = BG sculpte · Shift change de mode · Shift+molette taille · Ctrl+molette force
help-hintline-align = BG place · Ctrl/Command+glisser pivote · Shift+glisser efface · BRD annule
help-hintline-cut = BG plante ou déplace · Ctrl+molette en Section redimensionne · F inverse · Échap ferme
help-hintline-measure = BG mesure · BRD efface · molette zoom · Échap ferme

help-hint-navigation-orbit-the-camera = Orbiter la caméra
help-hint-navigation-pan-the-camera = Déplacer la caméra
help-hint-navigation-pan-the-camera-2 = Déplacer la caméra
help-hint-navigation-zoom-toward-the-pointer = Zoomer vers le pointeur
help-hint-navigation-recenter-on-the-surface = Recentrer sur la surface
help-hint-navigation-recenter-on-the-surface-when-enabled = Recentrer si activé
help-hint-navigation-open-the-layer-or-scene-menu-when-stationary = Ouvrir le menu calque/scène d’un clic fixe
help-hint-tools-open-a-file = Ouvrir un fichier
help-hint-tools-open-cut-view = Ouvrir la vue en coupe
help-hint-tools-arm-the-ruler = Armer la règle
help-hint-tools-arm-thickness = Armer l’épaisseur
help-hint-tools-open-align = Ouvrir l’alignement
help-hint-tools-open-mesh-editing = Ouvrir l’édition de maillages
help-hint-mesh-editing-select-a-face = Sélectionner une face
help-hint-mesh-editing-unmark-a-face-or-screen-selection = Désélectionner face ou sélection
help-hint-mesh-editing-select-faces-in-a-screen-rectangle = Sélectionner dans un rectangle
help-hint-mesh-editing-draw-a-freehand-selection-outline = Tracer un contour libre
help-hint-mesh-editing-close-and-apply-a-lasso-outline = Fermer et appliquer le lasso
help-hint-mesh-editing-cancel-the-active-lasso-outline = Annuler le lasso actif
help-hint-mesh-editing-select-all-visible-faces = Sélectionner toutes les faces visibles
help-hint-mesh-editing-delete-selected-faces = Supprimer les faces sélectionnées
help-hint-mesh-editing-undo-the-last-mesh-edit = Annuler la dernière édition
help-hint-mesh-editing-redo-the-last-mesh-edit = Rétablir la dernière édition
help-hint-sculpt-choose-add-remove = Choisir ajouter/enlever
help-hint-sculpt-choose-smooth = Choisir lisser
help-hint-sculpt-sculpt-under-the-brush = Sculpter sous le pinceau
help-hint-sculpt-remove-or-strengthen-the-active-brush-mode = Enlever ou renforcer le mode actif
help-hint-sculpt-change-brush-size = Changer la taille du pinceau
help-hint-sculpt-change-brush-intensity = Changer la force du pinceau
help-hint-align-measure-place-an-alignment-point-or-measurement-point = Placer un point d’alignement ou de mesure
help-hint-align-measure-rotate-a-scan-in-align-s-manual-mode = Pivoter un scan en mode manuel
help-hint-align-measure-erase-an-align-exclusion-region = Effacer une zone d’exclusion
help-hint-align-measure-change-align-exclusion-brush-size = Changer la taille du pinceau d’exclusion
help-hint-align-measure-undo-the-last-alignment-point-when-stationary = Annuler le dernier point d’un clic fixe
help-hint-align-measure-clear-measurements-when-stationary = Effacer les mesures d’un clic fixe
help-hint-align-measure-close-the-active-measurement-tool = Fermer l’outil de mesure
help-hint-cut-view-plant-or-move-the-cut-disc = Planter ou déplacer le disque
help-hint-cut-view-change-disc-size = Changer la taille du disque
help-hint-cut-view-zoom-the-section-view = Zoom de la vue en coupe
help-hint-cut-view-flip-the-kept-half-while-planted = Inverser la moitié gardée
help-hint-cut-view-unplant-the-disc-or-close-cut-view = Retirer le disque ou fermer la vue
help-hint-layers-preview-hide-the-layer-under-the-pointer = Masquer le calque sous le pointeur
help-hint-layers-preview-restore-the-last-hidden-layer = Restaurer le dernier calque masqué
help-hint-layers-preview-toggle-layer-translucency = Basculer la translucidité
help-hint-layers-preview-orbit-the-preview-model = Orbiter le modèle
help-hint-layers-preview-zoom-the-preview-model = Zoom du modèle
help-hint-layers-preview-frame-the-preview-model = Cadrer le modèle
help-hint-layers-preview-toggle-preview-wireframe = Basculer le filaire

## Toolbar, empty state — DRAFT.

toolbar-open-label = Ouvrir
toolbar-open-hint = Ouvrir des fichiers 3D ({ $shortcut })
toolbar-recent-hint = Fichiers récents
toolbar-add-label = Ajouter
toolbar-add-hint = Ajouter des fichiers à la scène
toolbar-cut-label = Vue en coupe
toolbar-cut-hint = Couper le modèle par un plan ({ $shortcut })
toolbar-cut-unavailable = La coupe exige un calque visible
toolbar-ruler-label = Règle
toolbar-ruler-hint = Mesurer une distance : deux points sur le modèle ({ $shortcut })
toolbar-thickness-label = Épaisseur
toolbar-thickness-hint = Sonder l’épaisseur : un point sur la paroi ({ $shortcut })
toolbar-measure-blocked = Terminer ou annuler la session d’édition
toolbar-measure-needs-layer = Mesurer exige un calque visible
toolbar-align-label = Aligner
toolbar-align-hint = Assembler deux scans : un point sur chacun ({ $shortcut })
toolbar-edit-label = Éditer
toolbar-edit-open = Édition de maillages ouverte
toolbar-edit-hint = Édition de maillages : sélection et sculpture ({ $shortcut })
toolbar-settings-label = Réglages
toolbar-settings-hint = Ouvrir les préférences

empty-open-file = Ouvrir un fichier 3D
empty-formats-hint = STL · PLY · OBJ · GLB · HPS — ou déposez des fichiers ici

## Loading and export — DRAFT.

load-queued = { $count ->
    [one] { $count } calque en file
   *[other] { $count } calques en file
}
load-opening = { $count ->
    [one] Ouverture de { $count } fichier…
   *[other] Ouverture de { $count } fichiers…
}
load-adding = { $count ->
    [one] Ajout de { $count } fichier…
   *[other] Ajout de { $count } fichiers…
}
load-open-failed-start = Échec d’ouverture : chargeur non démarré
load-add-failed-start = Échec d’ajout : chargeur non démarré
load-open-failed-stopped = Échec d’ouverture : chargeur arrêté
load-loader-failed-summary = Le chargeur de scène n’a pas pu démarrer.
load-action-failed-open = Échec d’ouverture : { $detail }
load-action-failed-add = Échec d’ajout : { $detail }

export-nothing-visible = Rien de visible à enregistrer
export-unsupported-format = Format de sortie non pris en charge
export-scene-saved = Scène enregistrée : { $path }
export-scene-saved-unmerged = Scène enregistrée (textures non fusionnées) : { $path }
export-scene-failed-title = Enregistrement impossible
export-scene-failed-summary = Enregistrement impossible : { $detail }
export-layers-saved = { $written ->
    [one] { $written } calque enregistré dans { $dir }
   *[other] { $written } calques enregistrés dans { $dir }
}
export-layers-saved-failed = { $written ->
    [one] { $written } calque enregistré dans { $dir }
   *[other] { $written } calques enregistrés dans { $dir }
}; { $failed ->
    [one] { $failed } non écrit
   *[other] { $failed } non écrits
}
export-layers-saved-renamed = { $written ->
    [one] { $written } calque enregistré dans { $dir }
   *[other] { $written } calques enregistrés dans { $dir }
}; { $renamed ->
    [one] { $renamed } fichier renommé pour garder l’existant
   *[other] { $renamed } fichiers renommés pour garder l’existant
}
export-layers-saved-failed-renamed = { $written ->
    [one] { $written } calque enregistré dans { $dir }
   *[other] { $written } calques enregistrés dans { $dir }
}; { $failed ->
    [one] { $failed } non écrit
   *[other] { $failed } non écrits
}; { $renamed ->
    [one] { $renamed } fichier renommé pour garder l’existant
   *[other] { $renamed } fichiers renommés pour garder l’existant
}
mesh-exported-aligned = { $name } exporté en position alignée comme { $format } : { $path }
mesh-exported-aligned-warnings = { $name } exporté en position alignée comme { $format } (avertissements : { $warnings }) : { $path }
mesh-exported-unmoved = { $name } exporté (scan non déplacé) comme { $format } : { $path }
mesh-exported-unmoved-warnings = { $name } exporté (scan non déplacé) comme { $format } (avertissements : { $warnings }) : { $path }
mesh-warning-point-cloud = nuage de points omis pour STL
mesh-warning-vertex-colors = couleurs de sommets non incluses
mesh-warning-uvs = UV non inclus
mesh-warning-texture-image = image de texture non incluse
mesh-export-failed-title = Export du calque impossible
mesh-export-failed-summary = Export du calque impossible : { $detail }

## Repair card and toasts — DRAFT.

repair-title = Réparation de maillages
repair-clean-headline = Rien à réparer — maillage propre
repair-copy-details = Copier les détails
repair-copy-tooltip = Copier le rapport complet dans le presse-papiers
repair-line-welded = { $count ->
    [one] { $grouped } sommet dupliqué soudé
   *[other] { $grouped } sommets dupliqués soudés
}
repair-line-slivers = { $count ->
    [one] { $grouped } face dégénérée supprimée
   *[other] { $grouped } faces dégénérées supprimées
}
repair-line-duplicate-faces = { $count ->
    [one] { $grouped } face dupliquée supprimée
   *[other] { $grouped } faces dupliquées supprimées
}
repair-line-nonmanifold = { $count ->
    [one] { $grouped } arête non-manifold réparée
   *[other] { $grouped } arêtes non-manifold réparées
}
repair-line-bowtie = { $count ->
    [one] { $grouped } sommet bowtie divisé
   *[other] { $grouped } sommets bowtie divisés
}
repair-line-reoriented = { $count ->
    [one] { $grouped } triangle réorienté
   *[other] { $grouped } triangles réorientés
}
repair-line-flipped = { $count ->
    [one] { $grouped } partie retournée
   *[other] { $grouped } parties retournées
}
repair-line-debris = { $count ->
    [one] { $grouped } débris supprimé
   *[other] { $grouped } débris supprimés
}
repair-line-pinholes = { $count ->
    [one] { $grouped } micro-trou rebouché
   *[other] { $grouped } micro-trous rebouchés
}
repair-line-unused = { $count ->
    [one] { $grouped } sommet inutile supprimé
   *[other] { $grouped } sommets inutiles supprimés
}
repair-open-rims = { $count ->
    [one] { $grouped } bord ouvert (limite du scan)
   *[other] { $grouped } bords ouverts (limite du scan)
}
repair-skipped-rims = { $count ->
    [one] { $grouped } bord non rebouché (non simple)
   *[other] { $grouped } bords non rebouchés (non simples)
}
repair-toast-welded = { $count ->
    [one] soudé { $count } sommet
   *[other] soudés { $count } sommets
}
repair-toast-slivers = { $count ->
    [one] supprimée { $count } dégénérée
   *[other] supprimées { $count } dégénérées
}
repair-toast-duplicate-faces = { $count ->
    [one] { $count } face dupliquée
   *[other] { $count } faces dupliquées
}
repair-toast-nonmanifold = { $count ->
    [one] réparée { $count } arête non-manifold
   *[other] réparées { $count } arêtes non-manifold
}
repair-toast-bowtie = { $count ->
    [one] divisé { $count } bowtie
   *[other] divisés { $count } bowties
}
repair-toast-reoriented = { $count ->
    [one] réorienté { $count } triangle
   *[other] réorientés { $count } triangles
}
repair-toast-flipped = { $count ->
    [one] retournée { $count } partie inversée
   *[other] retournées { $count } parties inversées
}
repair-toast-debris = { $count ->
    [one] supprimé { $count } débris
   *[other] supprimés { $count } débris
}
repair-toast-pinholes = { $count ->
    [one] rebouché { $count } micro-trou
   *[other] rebouchés { $count } micro-trous
}
repair-toast-unused = { $count ->
    [one] supprimé { $count } sommet inutile
   *[other] supprimés { $count } sommets inutiles
}
repair-toast-skipped = { $count ->
    [one] { $count } bord ignoré (non simple)
   *[other] { $count } bords ignorés (non simples)
}
repair-toast-done = Réparé { $layer } : { $parts }
repair-toast-clean-rims = Maillage déjà propre : { $layer }, { $count ->
    [one] { $count } bord ouvert
   *[other] { $count } bords ouverts
}
repair-toast-clean = Maillage déjà propre : { $layer }
repair-edit-busy = Édition de calque en cours
repair-edit-failed-title = Édition du calque impossible
repair-edit-failed-summary = Édition du calque impossible : { $detail }
edit-locked-status = { $status } (non annulable : instantané trop gros)

## Layers overlay, layer menu, scene menu — DRAFT.

layers-title = Calques
layers-count = { $count ->
    [one] { $count } calque
   *[other] { $count } calques
}
layers-row-hide = Masquer le calque
layers-row-show = Afficher le calque
layers-row-opacity = Opacité du calque
layers-row-remove = Supprimer le calque

layer-menu-next-tint = Teinte suivante
layer-menu-hide-colors = Masquer les couleurs du scan
layer-menu-show-colors = Afficher les couleurs du scan
layer-menu-disable-texture = Désactiver la texture
layer-menu-show-texture = Afficher la texture
layer-menu-mesh-editing = Édition de maillages
layer-menu-split-bridge = Séparer le bridge…
layer-menu-repair = Réparation de maillages
layer-menu-flip-normals = Inverser les normales
layer-menu-export = Exporter le calque…
layer-menu-hide-wireframe = Masquer le filaire
layer-menu-show-wireframe = Filaire superposé
layer-menu-remove = Supprimer le calque

scene-menu-title = Scène
scene-menu-save = Enregistrer la scène sous…
scene-menu-save-each = Enregistrer chaque calque…
scene-menu-reset = Réinitialiser les positions
scene-menu-fit = Cadrer la vue

## Mesh editor palette — DRAFT.

meshedit-tab-edit = Édition de maillages
meshedit-tab-sculpt = Sculpture
meshedit-cancel-session = Annuler la session (modifs annulées)
meshedit-header-edit = Édition de maillages
meshedit-section-selection = Sélection
meshedit-section-edit-selection = Modifier la sélection
meshedit-section-close-holes = Reboucher les trous
meshedit-section-sculpt = Sculpture
meshedit-cell-lasso = Lasso
meshedit-cell-lasso-hint = Contour libre : clic place des points, double-clic ferme · Shift désélectionne
meshedit-cell-object = Objet
meshedit-cell-object-hint = Clic sur un objet entier d’un STL multiparties · Shift désélectionne
meshedit-cell-surface = Surface
meshedit-cell-surface-hint = Ne marquer que la surface avant visible
meshedit-cell-through = À travers
meshedit-cell-through-hint = Marquer à travers le maillage, dos cachés inclus
meshedit-cell-all = Tout
meshedit-cell-all-hint = Tout marquer (Ctrl+A)
meshedit-cell-none = Rien
meshedit-cell-none-hint = Effacer le marquage
meshedit-cell-invert = Inverser
meshedit-cell-invert-hint = Échanger marqués et non marqués
meshedit-cell-delete = Supprimer
meshedit-cell-delete-hint = Supprimer les faces marquées
meshedit-cell-crop = Rogner
meshedit-cell-crop-hint = Ne garder que la zone marquée, supprimer le reste
meshedit-cell-cut = Couper
meshedit-cell-cut-hint = Déplacer les faces vers un nouveau maillage — l’original reste
meshedit-cell-separate = Séparer
meshedit-cell-separate-hint = Diviser la région en un maillage par partie connexe
meshedit-cell-close-holes = Reboucher
meshedit-cell-close-holes-hint = Reboucher seulement si les faces voisines sont marquées. Bords du scan ouverts.
meshedit-sculpt-addremove = Ajouter / Enlever  [1]
meshedit-sculpt-addremove-hint = Apporter de la matière en glissant ; Shift creuse. Shift+molette redimensionne, Ctrl+molette change la force. Touche : 1.
meshedit-sculpt-smooth = Lisser  [2]
meshedit-sculpt-smooth-hint = Détendre la surface en glissant ; Shift force le lissage max. Shift+molette redimensionne, Ctrl+molette change la force. Touche : 2.
meshedit-slider-size = taille
meshedit-slider-size-hint = Taille du pinceau (Shift + molette)
meshedit-slider-force = force
meshedit-slider-force-hint = Force du pinceau (Ctrl + molette)
meshedit-limit-label = limite
meshedit-limit-checkbox-hint = Limiter la réparation aux bords sous ce périmètre
meshedit-limit-drag-hint = Off rebouche tout trou sûr de la zone ; le bord reste ouvert
meshedit-status-unsaved = Modifs non enregistrées
meshedit-status-unsaved-hint = Non validé : OK applique, Annuler annule
meshedit-status-hint-sculpt = Glisser pour sculpter · BRD orbite
meshedit-status-hint-object = Clic sur un objet pour le prendre entier · Shift désélectionne
meshedit-status-hint-lasso = Clic trace · double-clic ferme · Shift désélectionne
meshedit-status-hint-default = Glisser un cadre · Shift désélectionne · Suppr efface
meshedit-session-undo = Annuler
meshedit-session-undo-hint = Annuler la dernière édition (Ctrl+Z)
meshedit-session-redo = Rétablir
meshedit-session-redo-hint = Rétablir l’édition annulée (Ctrl+Y)
meshedit-session-cancel = Annuler
meshedit-session-cancel-hint = Jeter toutes les éditions de la session
meshedit-session-done = OK
meshedit-session-done-hint = Appliquer et fermer l’éditeur

## Align Scans window — DRAFT.

align-title = Aligner les scans
align-tab-auto = Automatique
align-tab-manual = Manuel
align-constraint-free = Bouger/pivoter dans tous les sens
align-constraint-free-hint = Glisser le scan dans tous les sens
align-constraint-z = Bouger en z
align-constraint-z-hint = Glisser seulement en vertical
align-constraint-xy = Bouger dans le plan xy
align-constraint-xy-hint = Glisser seulement à l’horizontale
align-manual-drag-hint = Bouge le scan attrapé · Ctrl+glisser pivote
align-undo = Annuler
align-undo-hint = Un pas en arrière
align-redo = Rétablir
align-redo-hint = Un pas en avant
align-prompt-moving = Clic sur un point du maillage à bouger
align-prompt-other = Clic au même endroit sur l’autre maillage
align-prompt-alternate = Clic en alternant aux mêmes endroits des deux maillages
align-prompt-placed = { $count ->
    [one] { $count } flèche placée
   *[other] { $count } flèches placées
}
align-back = Retour
align-back-hint = Annuler une flèche — clic droit pareil
align-clear = Effacer
align-clear-hint = Jeter les flèches et rechoisir deux scans — ils restent en place
align-fit-perform = Lancer l’alignement
align-fit-perform-hint = Amener le maillage sur les flèches — deux flèches minimum
align-fit-refine = Ajustement fin
align-fit-refine-hint = Plaquer les surfaces. Maillages identiques uniquement
align-matching-parts = parties communes
align-matching-parts-hint = Part de surface présente sur les deux maillages. 70-80% si topologies proches
align-max-influence = influence max.
align-max-influence-hint = Seule la surface sous cette distance influence. Une grande valeur nuit
align-orientation-title = L’orientation doit correspondre
align-orientation-match = L’orientation doit correspondre
align-orientation-inverted = L’orientation doit correspondre inversée
align-orientation-ignored = Orientation ignorée
align-orientation-either-hint = Accepte les deux sens. Calcul souvent bien plus long
align-orientation-facing-hint = Comment les deux surfaces se font face
align-exclude = Ajustement : exclure les parties marquées
align-exclude-hint = Peindre la surface à ignorer
align-commit-cancel = Annuler
align-commit-cancel-hint-moved = Tout remettre et fermer — Ctrl+Z ramène l’alignement
align-commit-cancel-hint-clean = Fermer sans rien changer
align-commit-done = OK
align-commit-done-hint = Garder l’alignement et fermer — exporter pour écrire

## Deviation map — DRAFT.

align-map-more = Plus de réglages
align-map-heatmap = Carte de chaleur
align-map-heatmap-hint = Colorer un scan par sa distance à l’autre
align-map-range-label = plage
align-map-preset-hint = Sous { $min } mm = accord, { $max } mm sature
align-map-min = min
align-map-max = max
align-map-auto = auto
align-map-auto-hint = Réajuster la plage à la mesure
align-map-advice-far = Maillages à { $mm } mm — alignez avant de lire la carte
align-map-advice-saturated = Presque tout dépasse { $mm } mm, couleurs collées aux bouts — élargir la plage
align-map-not-enough = Pas assez de surface — { $measured } sommets sur { $total } atteignent l’autre scan
align-map-within = { $pct }% dans { $tol } mm
align-map-rms = rms { $rms }
align-map-grey-tooltip = Gris n’est pas une mesure. Sans vis-à-vis à portée, pas de mesure — un bridge ou une dent sur un seul scan, c’est normal, pas une erreur.
align-map-grey-out = { $n } sans surface opposée
align-map-grey-excluded = { $n } exclus
align-map-grey-unusable = { $n } inutilisables dans le fichier
align-map-grey-total = { $total } sommets gris : { $parts }
align-map-not-measured = non mesuré
align-map-not-measured-hint = Aucune surface de l’autre scan à portée de ces sommets. Dent ou bridge sur un seul scan : normal, pas une erreur — rien à mesurer.
align-map-stepped = Bandes en escalier
align-map-stepped-hint = Marches au lieu du fondu
align-map-colours = Couleurs
align-map-ramp-distance = distance
align-map-ramp-distance-hint = Froid où ça concorde, chaud où non
align-map-ramp-signed = signée
align-map-ramp-signed-hint = Bleu sous la surface, vert nominal, rouge dessus

## Align roles, brush, mask commands, align status lines — DRAFT.

align-pair-decided = { $moving } → { $fixed }
align-pair-guessed = { $moving } → { $fixed } (supposé)
align-pair-hint-decided = { $moving } bouge, { $fixed } reste
align-pair-hint-guessed = Rien cliqué, supposé par ordre d’ouverture. Premier clic décide : { $moving } bouge, { $fixed } reste
align-pair-swap = Inverser
align-pair-swap-hint = Ajuster dans l’autre sens — les flèches suivent

align-brush-title = Pinceau
align-brush-subtitle = Peindre la surface à ignorer, sur les deux maillages
align-brush-hint-inverse = Glisser efface · Shift marque · Shift+molette redimensionne
align-brush-hint-mark = Glisser marque · Shift efface · Shift+molette redimensionne
align-brush-close-hint = Fermer le pinceau — marques gardées
align-brush-size = taille du pinceau
align-brush-inverse = Pinceau inversé
align-brush-inverse-hint = Un glisser simple efface au lieu de marquer. Shift inverse à nouveau
align-brush-auto-radius = rayon auto
align-brush-auto-radius-hint = Rayon gardé à chaque bout de flèche
align-brush-all-marked = Tout marqué — ajustement sans effet
align-brush-nothing-marked = Rien marqué
align-brush-percent-marked = { $pct }% marqué hors ajustement
align-brush-size-status = Pinceau { $size } mm
align-status-no-summary = Rien à mesurer avec une portée de { $reach } mm — { $measured } sommets sur { $total } ont trouvé l'autre numérisation. Rapprochez les numérisations ou augmentez la portée dans Plus de réglages.

align-mask-fit-everywhere = Ajuster partout
align-mask-fit-everywhere-hint = Effacer tout marquage
align-mask-fit-everywhere-report = Marques effacées — ajustement sur tout le scan
align-mask-fit-nowhere = N’ajuster nulle part
align-mask-fit-nowhere-hint = Marquer tout le maillage — ajustement sans effet
align-mask-fit-nowhere-report = Maillage entier marqué — ajustement sans effet
align-mask-invert = Inverser les marques
align-mask-invert-hint = Marquer le non marqué et inversement
align-mask-invert-report = Marques inversées
align-mask-automatic = Marquage auto
align-mask-automatic-hint = Ajuster sur une petite zone à chaque bout
align-mask-automatic-report = Ajustement aux bouts de flèche

align-status-half-dropped = Flèche à moitié posée jetée
align-status-turned = Paire retournée
align-status-cleared = Paire effacée
align-status-click-moving = Clic sur un point du scan à bouger
align-status-click-alternate = Clic en alternant aux mêmes endroits
align-status-two-scans = Deux scans en vue — un point sur chacun
align-status-no-surface = Un nuage de points n’a pas de surface à apparier
align-status-now-other = Clic au point homologue de l’autre scan
align-status-moved = Point déplacé
align-status-wrong-scan = Ce scan n’est pas de la paire — Effacer et recommencer
align-status-place-first = D’abord un point sur chaque scan
align-status-one-scan = L’un des scans
align-status-scaled = Ce scan porte un placement à l’échelle, non alignable
align-status-pose-refused = Ajustement fini, mais son scan n’est plus là
align-status-measure-dropped = Mesure jetée — le pinceau possède les couleurs
align-status-map-elsewhere = La carte est sur l’onglet Auto — elle y revient
align-status-aligned-points = Aligné sur points

## Align result status lines — DRAFT.

align-status-aligned = Aligné — { $rms } mm sur les points{ $dropped }. Affinez pour asseoir.
align-status-outlier = , paire { $pairs } ignorée comme aberrante
align-status-refined = Affiné — { $rms } mm sur { $pct }% de { $surface }{ $settled }{ $weak }
align-status-surface = la surface
align-status-surface-unmarked = la surface non marquée
align-status-settled-limit = , arrêté à la limite d’itérations
align-status-weak-slide =  — l’ajustement peut encore glisser le long de { $axes }
align-status-weak-turn =  — l’ajustement peut encore tourner autour de { $axes }
align-status-weak-both =  — l’ajustement peut encore glisser le long de { $sliding } et tourner autour de { $spinning }
align-status-measured = { $pct }% dans { $tol } mm, { $n ->
    [one] { $n } sommet sans vis-à-vis{ $blind }
   *[other] { $n } sommets sans vis-à-vis{ $blind }
}
align-status-blind-free =  — ces surfaces glissent librement, n’importe quel déplacement peut se cacher derrière
align-status-blind-hidden =  — un désajustement rigide jusqu’à { $mm } mm pourrait se lire pareil
align-status-remeasure = { $reason } — relancer l’ajustement fin pour mesurer
align-brush-not-in-alignment = Ce scan ne fait pas partie de cet alignement
align-drag-moving = Déplacement de { $name } à la main
align-drag-unrecorded = Déplacé à la main, mais cette étape n’a pas rejoint l’historique — Ctrl+Z ne l’annulera pas
align-drag-moved = { $name } : déplacement de { $moved } mm à la main (Ctrl+Z annule)
align-status-moved-hand = Déplacé à la main
align-pair-placed = Paire { $n } posée
align-roles-swapped = { $moving } bouge maintenant, { $fixed } reste en place
align-status-scan-changed = Le scan a changé
align-status-hidden = Calque masqué : { $name }. Affichez-le pour aligner dessus
align-arrow-removed = { $n ->
    [one] Flèche retirée — il reste { $n } paire
   *[other] Flèches retirées — il reste { $n } paires
}
align-status-markings-changed = Marques changées
align-status-place-arrow-first = Placer au moins une flèche avant de marquer
align-status-arrows-cleared = Flèches retirées — à la main d’ici

## Unsaved-work guards and error dialog buttons — DRAFT.

guard-close-title = Modifs de maillage non enregistrées
guard-close-headline-one = 1 calque modifié non enregistré.
guard-close-headline-many = Calques modifiés non enregistrés.
guard-close-note = { $count } calques modifiés concernés.
guard-close-detail = Enregistrer exporte chaque calque (PLY, STL ou OBJ) puis ferme.
guard-close-destructive = Fermer sans enregistrer
guard-replace-title = Édition en cours
guard-replace-headline-session = Une session est active sur { $layer }.
guard-replace-headline-one = 1 calque modifié non enregistré.
guard-replace-headline-many = { $count } calques modifiés non enregistrés.
guard-replace-detail = Ouvrir une scène ferme la session et jette le non enregistré.
guard-replace-destructive = Jeter et ouvrir
guard-save = Enregistrer…
guard-cancel = Annuler

error-close = Fermer
error-copy-details = Copier les détails

about-website = Site web
about-source = Sources
about-licenses = Licences tierces
about-license-kind = Licence Apache 2.0

## Mesh-edit operations, undo/redo, sculpt, measure, cut ruler, scene menu — DRAFT.

edit-select-faces-first = Sélectionner d’abord des faces du maillage
edit-no-changes = Rien changé : { $layer }
edit-apply-failed-title = Édition de la sélection impossible
edit-apply-failed-summary = Édition de la sélection impossible : { $detail }
edit-no-changes-hidden = Rien changé : affinez la sélection ; les calques masqués restent intacts
edit-selected-faces = { $faces ->
    [one] { $faces } face sélectionnée
   *[other] { $faces } faces sélectionnées
}
edit-selected-faces-across = { $faces ->
    [one] { $faces } face sélectionnée sur { $layers } calques
   *[other] { $faces } faces sélectionnées sur { $layers } calques
}

holes-nothing = Rien à reboucher : { $layer }
holes-partial = { $segments }, rien rebouché : { $layer }
holes-closed = { $filled ->
    [one] Rebouché { $filled } trou
   *[other] Rebouchés { $filled } trous
}
holes-closed-detail = { $closed } : { $layer }
holes-closed-segments = { $closed } ({ $segments }) : { $layer }
holes-seg-healed = { $n ->
    [one] { $n } accroc réparé
   *[other] { $n } accrocs réparés
}
holes-seg-border = bord du scan laissé ouvert
holes-seg-oversize-limit = { $n ->
    [one] { $n } trou au-delà de la limite de { $limit } mm
   *[other] { $n } trous au-delà de la limite de { $limit } mm
}
holes-seg-oversize = { $n ->
    [one] { $n } trou trop grand
   *[other] { $n } trous trop grands
}
holes-seg-damaged = { $n ->
    [one] { $n } bord abîmé ignoré
   *[other] { $n } bords abîmés ignorés
}
batchedit-invert = Normales inversées
batchedit-close-holes = Trous intérieurs sûrs rebouchés
batchedit-delete = Sélection supprimée
batchedit-crop = Rogné à la sélection
batchedit-cut = Sélection coupée vers nouveau calque
batchedit-separate = Sélection séparée
batchedit-edited = Calque édité
edit-applied-status = { $action } : { $layer }
batchedit-status = { $label } sur { $n ->
    [one] { $n } calque visible
   *[other] { $n } calques visibles
}
batchedit-no-changes = Rien changé : affinez ; le caché reste intact
batch-close-holes = Trous intérieurs rebouchés
batch-delete = Sélection supprimée
batch-crop = Rogné à la sélection
batch-cut = Sélection coupée
batch-separate = Sélection séparée
batch-edited = Sélection éditée

select-covers-all = La sélection couvre déjà tout : { $layer }
select-covers-remove = Tout est couvert — supprimez plutôt le calque : { $layer }
select-splits = La sélection se divise en { $parts } — affinez : { $layer }
select-faces-cannot = Faces non sélectionnables : { $layer }

undo-nothing = Rien à annuler
redo-nothing = Rien à rétablir
undo-undid = Édition annulée : { $layer }
undo-unavailable = Annulation impossible — la scène a changé : { $layer }
redo-redid = Édition rétablie : { $layer }
redo-unavailable = Rétablissement impossible — la scène a changé : { $layer }

sculpt-armed-addremove = Ajouter/Enlever : glisser pour apporter, Shift creuse
sculpt-armed-smooth = Lisser : glisser pour détendre, Shift force
sculpt-off = Sculpture off
sculpt-applied-undo = Sculpture appliquée (Ctrl+Z annule)
sculpt-applied-locked = Sculpture appliquée (non annulable : instantané énorme)
sculpt-failed = Impossible de sculpter ce calque : { $detail }
sculpt-worker-stopped = Processus de sculpture arrêté : { $detail }
sculpt-failure-worker-panicked = Le processus de sculpture a planté : { $detail }
sculpt-failure-spawn = Impossible de démarrer le processus de sculpture : { $detail }
sculpt-failure-kernel-pool = Impossible de créer le pool de noyaux de sculpture : { $detail }
sculpt-failure-missing-undo-baseline = Le trait de sculpture n'a pas de référence d'annulation
sculpt-failure-shadow-poisoned = Le verrou d'ombre de sculpture est empoisonné
sculpt-failure-vertex-count-changed = Le résultat de sculpture a modifié le nombre de sommets
sculpt-worker-unavailable = Sculpture indisponible
sculpt-finishing = Finition du trait…
sculpt-finishing-history = Finition avant l’historique…
sculpt-lasso-armed = Lasso armé : clic ou glisser trace ; Entrée, double-clic ou départ ferme
sculpt-lasso-off = Lasso désarmé
sculpt-object-on = Choisir objet : clic pour le prendre entier
sculpt-object-off = Choisir objet off
sculpt-selection-cleared = Sélection effacée
sculpt-through-on = Sélection à travers
sculpt-through-off = Sélection de surface

measure-distance = Distance : { $len }
measure-thickness = Épaisseur de paroi : { $len }
measure-open-wall = Surface ouverte : pas de paroi opposée dans la normale
measure-cannot-probe = Mesure impossible ici : géométrie dégénérée
measure-cleared = Mesures effacées

cut-lines = Lignes
cut-mesh = Maillage
cut-dist = Dist
cut-dist-hint = Distance : clic sur deux points
cut-thick = Épais
cut-thick-hint = Épaisseur : clic sur un point du contour
cut-close-section = Fermer la section
cut-snap = Aimant
cut-snap-hint = Aimant : les clics s’accrochent au contour
cut-empty = Pas d’intersection
cut-footer-distance = Glisser = déplacer · clic 2 pts = distance · clic droit efface · molette = zoom
cut-footer-thickness = Glisser = déplacer · clic contour = épaisseur · clic droit efface · molette = zoom

recent-clear = Effacer les récents

scene-already-origin = Tout déjà à sa place d’origine
scene-positions-reset = Positions réinitialisées (Ctrl+Z annule)

## Session close-outs and layer shortcuts — DRAFT.

session-applied = Session appliquée
session-reverted = Session annulée
edit-session-busy = Terminer ou annuler la session d’abord
layers-none-hidden = Aucun calque masqué à ramener
layer-opaque-again = Opaque à nouveau : { $label }
layer-translucent = Translucide : { $label } (Shift+clic BMM restaure)
layer-restored = Affiché à nouveau : { $label }
layer-hidden = Masqué : { $label } (Shift+Ctrl+clic BMM restaure)
layer-unnamed = calque { $n }
layer-removed = Calque supprimé : { $label }
layer-face-selection = Sélection de faces : { $label }

## Bridge split panel and align session close-outs — DRAFT.

bridge-panel-title = Séparer le bridge
bridge-mode-place = Poser le disque
bridge-mode-calculating = Calcul…
bridge-mode-ready = Prêt
bridge-mode-failed = Tentative ratée
bridge-kerf = Trait de scie
bridge-disc-size = Taille du disque
bridge-cancel = Annuler
bridge-apply = Séparer le bridge
bridge-err-miss = Le disque rate le bridge. Placez-le dans le connecteur.
bridge-err-tangent = Le disque ne fait que toucher. Traversez le connecteur.
bridge-err-small = Diamètre { $have } mm ; { $need } mm minimum ici.
bridge-err-limit = Cette coupe veut un disque de { $need } mm, au-delà de la limite de { $max } mm.
bridge-err-no-result = Tentative à surface préservée, sans résultat utile. Maillage d’origine gardé.
bridge-err-invalid-cut = Tentative ratée, coupe non validable. Maillage d’origine gardé.
bridge-err-invalid-side = Tentative ratée, { $side } non validable. Maillage d’origine gardé.
bridge-err-gap = Tentative ratée, jeu non conservé. Maillage d’origine gardé.
bridge-err-empty = Le calque n’a pas de maillage à séparer.
bridge-err-invalid = Réglages du disque invalides. Réinitialiser et réessayer.
bridge-err-unusable = Sans résultat utile. Maillage d’origine gardé.

align-session-canceled = Alignement annulé — tout est revenu (Ctrl+Z le ramène)
align-session-closed = Alignement fermé
align-session-closed-running = Alignement fermé — un ajustement tournait, lâché tel quel
align-session-kept = Alignement gardé — exporter le scan pour l’écrire

## Settings panel, bridge split, render error, tint — DRAFT.

settings-header = Réglages
settings-section-files = Fichiers et export
settings-export-format = Format d’export de repli
settings-export-format-hint = Quand le format source ne s’exporte pas
settings-remember-export = Mémoriser le dossier d’export
settings-remember-export-hint = Même dossier après redémarrage
settings-section-scene = Vue et navigation
settings-frame-on-open = Cadrer à l’ouverture
settings-frame-on-open-hint = Revenir à la vue d’accueil quand un fichier remplace la scène
settings-double-click = Double-clic recentre
settings-double-click-hint = Double-clic recentre la caméra sur le point
settings-orbit = Vitesse orbitale
settings-orbit-hint = Vitesse d’orbite au bouton droit
settings-zoom = Vitesse de zoom
settings-zoom-hint = Ce que chaque cran rapproche
settings-recent = Scènes récentes
settings-recent-hint = Entrées dans le menu Ouvrir
settings-background = Fond
settings-bg-gray = Gris
settings-bg-white = Blanc
settings-bg-dark = Sombre
settings-ghost = Fantôme du côté coupé
settings-ghost-hint = En coupe, montrer le côté retiré en fantôme
settings-measurements = Mesures
settings-section-appearance = Apparence
settings-theme = Thème
settings-theme-light = Clair
settings-theme-dark = Sombre
settings-scale = Échelle de l’interface
settings-scale-hint = Met tout à l’échelle ; 1.0 garde le système
settings-section-mesh = Édition de maillages
settings-remember-brush = Mémoriser le pinceau
settings-remember-brush-hint = Garder taille et force entre sessions
settings-section-updates = Mises à jour
settings-check-auto = Vérifier au démarrage
settings-check-now = Vérifier
settings-check-disabled-hint = Vérifications désactivées par l’environnement
settings-check-busy-hint = Une vérification est déjà en cours
settings-update-disabled = Désactivé par l’environnement
settings-update-checking = Vérification…
settings-update-current = À jour
settings-update-skipped = Version ignorée
settings-update-failed = Vérification impossible
settings-save-error = Préférences non enregistrées. Nouvel essai…
settings-save-error-hint = Fichier de préférences indisponible
settings-about = À propos d’OccluView

bridge-busy = Terminer ou annuler la division d’abord
bridge-active = Division déjà active
bridge-target-gone = Cible perdue
bridge-needs-mesh = La division exige un maillage visible
bridge-place-disc = Division : poser le disque séparateur
bridge-canceled-scene = Division annulée : scène fermée
bridge-canceled-camera = Division annulée : pas de caméra
bridge-canceled-changed = Division annulée : maillage changé
bridge-canceled = Division annulée
bridge-calculating = Division : calcul…
bridge-unavailable = Division indisponible pour l’instant
bridge-preview-stale = Aperçu périmé
bridge-not-applied = Division non appliquée
bridge-complete = Division terminée
bridge-complete-surface = Division terminée (surface ; bords naturels intacts)
bridge-complete-locked = Division terminée (non annulable : instantané énorme)

render-failed-title = Rendu impossible
render-failed-summary = Fichier ouvert, mais viewport non rendable.
render-failed-status = Rendu raté

tint-choose = Choisir la teinte

## Status tail: brush, lasso, loading, GPU, align jobs — DRAFT.

brush-no-mesh = Clic sur un point de chaque maillage, puis peindre
lasso-dropped = Lasso lâché
lasso-needs-points = Le lasso veut 3 points minimum
loading-scene = Chargement de la scène…
gpu-failed-status = Le pilote signale un problème
gpu-failed-title = Problème graphique
gpu-failed-summary = Le pilote a raté le dessin. La vue est peut-être incomplète. Enregistrez et relancez si ça revient.
align-job-align = Alignement…
align-job-refine = Affinage…
align-job-measure = Mesure…
align-markings-dropped = Marques jetées — la surface a changé depuis

## Worker-built align failures — DRAFT.

align-fail-no-surface-fixed = Le scan fixe n’a pas de surface utile
align-fail-no-surface-moving = Le scan mobile n’a pas de surface utile
align-fail-recolor = Mesure jetée avant coloriage
align-reject-toofew = Seulement { $a } sur { $b } correspondances — une flèche de plus, ou monter l’influence
align-reject-unpaired = { $a } points sur un scan et { $b } sur l’autre — sans partenaire
align-reject-degenerate-plain = Les points ne fixent pas de rotation — écartez-les
align-reject-degenerate-line = Points alignés : rotation autour de { $a } indéterminée
align-reject-unit = Les scans diffèrent de { $a }x en taille — unités différentes
align-reject-apart = L’ajustement laisse { $a } mm au lieu de superposer ({ $b } mm) — vérifiez les paires
align-reject-runaway = L’ajustement a erré de { $a } mm, plus que le scan ({ $b } mm) — des flèches, ou baisser l’influence
align-reject-nonfinite = Un point ou une normale n’est pas un nombre fini
align-status-stepped = Parcours de l’historique
align-status-moving-hand = Déplacé à la main

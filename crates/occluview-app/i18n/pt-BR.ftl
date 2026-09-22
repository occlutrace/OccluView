## OccluView Brazilian Portuguese catalog — DRAFT (machine draft, native review required).
## Status: DRAFT. Requires native dental/CAD terminology review + visual UI review before APPROVED.
## Contract: exact key/attribute/variable parity with en.ftl.
## Never use this catalog for pt-PT or other Portuguese variants.

app-title = OccluView
app-window-title = OccluView 3D Viewer
align-panel-title = Alinhar escaneamentos
meshedit-window-title = Edição de malhas

settings-language-label = Idioma
settings-language-auto = Idioma do sistema
settings-language-auto-current = Idioma do sistema — { $language }
settings-language-catalog-fallback = { $tag } não está disponível; o inglês será usado.
settings-language-save-error = Não deu para salvar o idioma. Tentando de novo…

about-title = Sobre o OccluView
about-tagline = Reparo de malhas · Edição para CAD odontológico
about-version = Versão { $version }

update-available-title = Atualização disponível
update-available-body = A versão { $version } está pronta para instalar.
update-current-version = Você está na { $version }.
update-download = Baixar atualização
update-open-release = Abrir a página da versão
update-later = Depois
update-skip = Pular esta versão
update-skip-tooltip = Não oferecer mais esta versão; a próxima será oferecida
update-downloading = Baixando o OccluView { $version }
update-ready-title = OccluView { $version } pronto para instalar
update-ready-hint-windows = Instalador verificado. O OccluView vai fechar enquanto o Windows aplica a atualização.
update-ready-hint-other = Pacote verificado. O instalador do sistema vai abrir — confirme lá.
update-install-close = Instalar e fechar
update-failed-title = Falha na atualização
update-dismiss = Dispensar

error-open-title = Não dá para abrir o arquivo
error-add-title = Não dá para adicionar o arquivo
error-open-body = Não foi possível abrir { $path }.

## Help surface — DRAFT. Gesture names stay invariant by contract.

help-title = Controles de teclado e mouse
help-subtitle = A referência corresponde aos controles do OccluView.
help-close = Fechar

help-section-navigation = Navegação
help-section-tools = Ferramentas
help-section-mesh-editing = Edição de malhas
help-section-sculpt = Escultura
help-section-align-measure = Alinhamento e medição
help-section-cut-view = Vista de corte
help-section-layers-preview = Camadas e prévia do Explorer

help-hintline-navigation = Arrastar BRD orbita · BRM desloca · roda zoom · clique BRM foca
help-hintline-mesh-editing = Clique EQM seleciona · Shift+clique desmarca · retângulo · Ctrl+Z desfaz
help-hintline-sculpt = EQM esculpe · Shift troca o modo · Shift+roda tamanho · Ctrl+roda força
help-hintline-align = EQM posiciona · Ctrl/Command+arrastar gira · Shift+arrastar apaga · BRD desfaz
help-hintline-cut = EQM planta ou move · Ctrl+roda em Seção redimensiona · F inverte · Esc fecha
help-hintline-measure = EQM mede · BRD limpa · roda zoom · Esc fecha

help-hint-navigation-orbit-the-camera = Orbitar a câmera
help-hint-navigation-pan-the-camera = Deslocar a câmera
help-hint-navigation-pan-the-camera-2 = Deslocar a câmera
help-hint-navigation-zoom-toward-the-pointer = Zoom no ponteiro
help-hint-navigation-recenter-on-the-surface = Recentralizar na superfície
help-hint-navigation-recenter-on-the-surface-when-enabled = Recentralizar se ativado
help-hint-navigation-open-the-layer-or-scene-menu-when-stationary = Abrir o menu da camada ou cena com clique parado
help-hint-tools-open-a-file = Abrir um arquivo
help-hint-tools-open-cut-view = Abrir vista de corte
help-hint-tools-arm-the-ruler = Ativar a régua
help-hint-tools-arm-thickness = Ativar espessura
help-hint-tools-open-align = Abrir alinhamento
help-hint-tools-open-mesh-editing = Abrir edição de malhas
help-hint-mesh-editing-select-a-face = Selecionar uma face
help-hint-mesh-editing-unmark-a-face-or-screen-selection = Desmarcar face ou seleção
help-hint-mesh-editing-select-faces-in-a-screen-rectangle = Selecionar faces num retângulo
help-hint-mesh-editing-draw-a-freehand-selection-outline = Desenhar um contorno livre
help-hint-mesh-editing-close-and-apply-a-lasso-outline = Fechar e aplicar o laço
help-hint-mesh-editing-cancel-the-active-lasso-outline = Cancelar o laço ativo
help-hint-mesh-editing-select-all-visible-faces = Selecionar todas as faces visíveis
help-hint-mesh-editing-delete-selected-faces = Excluir as faces selecionadas
help-hint-mesh-editing-undo-the-last-mesh-edit = Desfazer a última edição
help-hint-mesh-editing-redo-the-last-mesh-edit = Refazer a última edição
help-hint-sculpt-choose-add-remove = Escolher adicionar/remover
help-hint-sculpt-choose-smooth = Escolher suavizar
help-hint-sculpt-sculpt-under-the-brush = Esculpir sob o pincel
help-hint-sculpt-remove-or-strengthen-the-active-brush-mode = Remover ou reforçar o modo ativo
help-hint-sculpt-change-brush-size = Mudar o tamanho do pincel
help-hint-sculpt-change-brush-intensity = Mudar a força do pincel
help-hint-align-measure-place-an-alignment-point-or-measurement-point = Posicionar ponto de alinhamento ou medição
help-hint-align-measure-rotate-a-scan-in-align-s-manual-mode = Girar um escaneamento no modo manual
help-hint-align-measure-erase-an-align-exclusion-region = Apagar uma região de exclusão
help-hint-align-measure-change-align-exclusion-brush-size = Mudar o pincel de exclusão
help-hint-align-measure-undo-the-last-alignment-point-when-stationary = Desfazer o último ponto parado
help-hint-align-measure-clear-measurements-when-stationary = Limpar medições parado
help-hint-align-measure-close-the-active-measurement-tool = Fechar a medição ativa
help-hint-cut-view-plant-or-move-the-cut-disc = Plantar ou mover o disco
help-hint-cut-view-change-disc-size = Mudar o tamanho do disco
help-hint-cut-view-zoom-the-section-view = Zoom da vista de corte
help-hint-cut-view-flip-the-kept-half-while-planted = Inverter a metade mantida
help-hint-cut-view-unplant-the-disc-or-close-cut-view = Soltar o disco ou fechar a vista
help-hint-layers-preview-hide-the-layer-under-the-pointer = Ocultar a camada sob o ponteiro
help-hint-layers-preview-restore-the-last-hidden-layer = Restaurar a última camada oculta
help-hint-layers-preview-toggle-layer-translucency = Alternar a translucidez
help-hint-layers-preview-orbit-the-preview-model = Orbitar o modelo
help-hint-layers-preview-zoom-the-preview-model = Zoom do modelo
help-hint-layers-preview-frame-the-preview-model = Enquadrar o modelo
help-hint-layers-preview-toggle-preview-wireframe = Alternar o wireframe

## Toolbar, empty state — DRAFT.

toolbar-open-label = Abrir
toolbar-open-hint = Abrir arquivos 3D ({ $shortcut })
toolbar-recent-hint = Arquivos recentes
toolbar-add-label = Adicionar
toolbar-add-hint = Adicionar arquivos à cena
toolbar-cut-label = Vista de corte
toolbar-cut-hint = Cortar o modelo por um plano ({ $shortcut })
toolbar-cut-unavailable = O corte precisa de uma camada visível
toolbar-ruler-label = Régua
toolbar-ruler-hint = Medir distância: dois pontos no modelo ({ $shortcut })
toolbar-thickness-label = Espessura
toolbar-thickness-hint = Sondar a espessura: um ponto na parede ({ $shortcut })
toolbar-measure-blocked = Termine ou cancele a sessão antes
toolbar-measure-needs-layer = Medir precisa de camada visível
toolbar-align-label = Alinhar
toolbar-align-hint = Juntar dois escaneamentos: um ponto em cada ({ $shortcut })
toolbar-edit-label = Editar
toolbar-edit-open = Edição de malhas aberta
toolbar-edit-hint = Edição de malhas: seleção e escultura ({ $shortcut })
toolbar-settings-label = Ajustes
toolbar-settings-hint = Abrir preferências

empty-open-file = Abrir um arquivo 3D
empty-formats-hint = STL · PLY · OBJ · GLB · HPS — ou solte arquivos aqui

## Loading and export — DRAFT.

load-queued = { $count ->
    [one] { $count } camada na fila
   *[other] { $count } camadas na fila
}
load-opening = { $count ->
    [one] Abrindo { $count } arquivo…
   *[other] Abrindo { $count } arquivos…
}
load-adding = { $count ->
    [one] Adicionando { $count } arquivo…
   *[other] Adicionando { $count } arquivos…
}
load-open-failed-start = Falha ao abrir: carregador não iniciou
load-add-failed-start = Falha ao adicionar: carregador não iniciou
load-open-failed-stopped = Falha ao abrir: carregador parou
load-loader-failed-summary = Não deu para iniciar o carregador de cena.
load-action-failed-open = Falha ao abrir: { $detail }
load-action-failed-add = Falha ao adicionar: { $detail }

export-nothing-visible = Nada visível para salvar
export-unsupported-format = Formato de saída não suportado
export-scene-saved = Cena salva: { $path }
export-scene-saved-unmerged = Cena salva (texturas não fundidas): { $path }
export-scene-failed-title = Não deu para salvar a cena
export-scene-failed-summary = Não deu para salvar a cena: { $detail }
export-layers-saved = { $written ->
    [one] Salva { $written } camada em { $dir }
   *[other] Salvas { $written } camadas em { $dir }
}
export-layers-saved-failed = { $written ->
    [one] Salva { $written } camada em { $dir }
   *[other] Salvas { $written } camadas em { $dir }
}; { $failed ->
    [one] { $failed } não gravou
   *[other] { $failed } não gravaram
}
export-layers-saved-renamed = { $written ->
    [one] Salva { $written } camada em { $dir }
   *[other] Salvas { $written } camadas em { $dir }
}; { $renamed ->
    [one] { $renamed } arquivo renomeado para manter o existente
   *[other] { $renamed } arquivos renomeados para manter o existente
}
export-layers-saved-failed-renamed = { $written ->
    [one] Salva { $written } camada em { $dir }
   *[other] Salvas { $written } camadas em { $dir }
}; { $failed ->
    [one] { $failed } não gravou
   *[other] { $failed } não gravaram
}; { $renamed ->
    [one] { $renamed } arquivo renomeado para manter o existente
   *[other] { $renamed } arquivos renomeados para manter o existente
}
mesh-exported-aligned = { $name } exportado alinhado como { $format }: { $path }
mesh-exported-aligned-warnings = { $name } exportado alinhado como { $format } (avisos: { $warnings }): { $path }
mesh-exported-unmoved = { $name } exportado (parado) como { $format }: { $path }
mesh-exported-unmoved-warnings = { $name } exportado (parado) como { $format } (avisos: { $warnings }): { $path }
mesh-warning-vertex-colors = cores de vértice não incluídas
mesh-warning-uvs = UVs não incluídos
mesh-warning-texture-image = imagem de textura não incluída
mesh-export-warnings = Avisos de exportação: { $warnings }
mesh-export-failed-title = Não deu para exportar a camada
mesh-export-failed-summary = Não deu para exportar a camada: { $detail }

## Repair card and toasts — DRAFT.

repair-title = Reparo de malhas
repair-clean-headline = Nada para reparar — malha limpa
repair-copy-details = Copiar detalhes
repair-copy-tooltip = Copiar o relatório completo
repair-line-welded = { $count ->
    [one] Soldado { $grouped } vértice duplicado
   *[other] Soldados { $grouped } vértices duplicados
}
repair-line-slivers = { $count ->
    [one] Removida { $grouped } face degenerada
   *[other] Removidas { $grouped } faces degeneradas
}
repair-line-duplicate-faces = { $count ->
    [one] Removida { $grouped } face duplicada
   *[other] Removidas { $grouped } faces duplicadas
}
repair-line-nonmanifold = { $count ->
    [one] Corrigida { $grouped } aresta não-manifold
   *[other] Corrigidas { $grouped } arestas não-manifold
}
repair-line-bowtie = { $count ->
    [one] Dividido { $grouped } vértice bowtie
   *[other] Divididos { $grouped } vértices bowtie
}
repair-line-reoriented = { $count ->
    [one] Reorientado { $grouped } triângulo
   *[other] Reorientados { $grouped } triângulos
}
repair-line-flipped = { $count ->
    [one] Virada { $grouped } parte invertida
   *[other] Viradas { $grouped } partes invertidas
}
repair-line-debris = { $count ->
    [one] Removida { $grouped } parte residual
   *[other] Removidas { $grouped } partes residuais
}
repair-line-pinholes = { $count ->
    [one] Fechado { $grouped } furo
   *[other] Fechados { $grouped } furos
}
repair-line-unused = { $count ->
    [one] Removido { $grouped } vértice inútil
   *[other] Removidos { $grouped } vértices inúteis
}
repair-open-rims = { $count ->
    [one] { $grouped } borda aberta (limite do escaneamento)
   *[other] { $grouped } bordas abertas (limite do escaneamento)
}
repair-skipped-rims = { $count ->
    [one] { $grouped } borda sem preencher (não simples)
   *[other] { $grouped } bordas sem preencher (não simples)
}
repair-toast-welded = { $count ->
    [one] soldado { $count } vértice
   *[other] soldados { $count } vértices
}
repair-toast-slivers = { $count ->
    [one] removida { $count } degenerada
   *[other] removidas { $count } degeneradas
}
repair-toast-duplicate-faces = { $count ->
    [one] { $count } face duplicada
   *[other] { $count } faces duplicadas
}
repair-toast-nonmanifold = { $count ->
    [one] corrigida { $count } aresta não-manifold
   *[other] corrigidas { $count } arestas não-manifold
}
repair-toast-bowtie = { $count ->
    [one] dividido { $count } bowtie
   *[other] divididos { $count } bowties
}
repair-toast-reoriented = { $count ->
    [one] reorientado { $count } triângulo
   *[other] reorientados { $count } triângulos
}
repair-toast-flipped = { $count ->
    [one] virada { $count } parte invertida
   *[other] viradas { $count } partes invertidas
}
repair-toast-debris = { $count ->
    [one] removida { $count } parte residual
   *[other] removidas { $count } partes residuais
}
repair-toast-pinholes = { $count ->
    [one] fechado { $count } furo
   *[other] fechados { $count } furos
}
repair-toast-unused = { $count ->
    [one] removido { $count } vértice inútil
   *[other] removidos { $count } vértices inúteis
}
repair-toast-skipped = { $count ->
    [one] { $count } borda pulada (não simples)
   *[other] { $count } bordas puladas (não simples)
}
repair-toast-done = Reparado { $layer }: { $parts }
repair-toast-clean-rims = Malha limpa: { $layer }, { $count ->
    [one] { $count } borda aberta
   *[other] { $count } bordas abertas
}
repair-toast-clean = Malha limpa: { $layer }
repair-edit-busy = Edição em curso
repair-edit-failed-title = Não deu para editar a camada
repair-edit-failed-summary = Não deu para editar a camada: { $detail }
edit-locked-status = { $status } (sem desfazer: snapshot grande demais)

## Layers overlay, layer menu, scene menu — DRAFT.
## Never use this catalog for pt-PT or other Portuguese variants.

layers-title = Camadas
layers-count = { $count ->
    [one] { $count } camada
   *[other] { $count } camadas
}
layers-row-hide = Ocultar camada
layers-row-show = Mostrar camada
layers-row-opacity = Opacidade da camada
layers-row-remove = Remover camada

layer-menu-next-tint = Próximo matiz
layer-menu-hide-colors = Ocultar cores do escaneamento
layer-menu-show-colors = Mostrar cores do escaneamento
layer-menu-disable-texture = Desativar textura
layer-menu-show-texture = Mostrar textura
layer-menu-mesh-editing = Edição de malhas
layer-menu-split-bridge = Dividir bridge…
layer-menu-repair = Reparo de malhas
layer-menu-flip-normals = Inverter normais
layer-menu-export = Exportar camada…
layer-menu-hide-wireframe = Ocultar wireframe
layer-menu-show-wireframe = Wireframe sobreposto
layer-menu-remove = Remover camada

scene-menu-title = Cena
scene-menu-save = Salvar cena como…
scene-menu-save-each = Salvar cada camada…
scene-menu-reset = Redefinir posições
scene-menu-fit = Enquadrar vista

## Mesh editor palette — DRAFT.

meshedit-tab-edit = Edição de malhas
meshedit-tab-sculpt = Escultura
meshedit-cancel-session = Cancelar a sessão (reverte as edições)
meshedit-header-edit = Edição de malhas
meshedit-section-selection = Seleção
meshedit-section-edit-selection = Editar seleção
meshedit-section-close-holes = Fechar buracos
meshedit-section-sculpt = Escultura
meshedit-cell-lasso = Laço
meshedit-cell-lasso-hint = Contorno livre: clique posiciona, duplo-clique fecha · Shift desmarca
meshedit-cell-object = Objeto
meshedit-cell-object-hint = Clique num objeto inteiro de STL multipartes · Shift desmarca
meshedit-cell-surface = Superfície
meshedit-cell-surface-hint = Marcar só a superfície frontal visível
meshedit-cell-through = Através
meshedit-cell-through-hint = Marcar através da malha, incluindo ocultos
meshedit-cell-all = Tudo
meshedit-cell-all-hint = Marcar todas as faces (Ctrl+A)
meshedit-cell-none = Nada
meshedit-cell-none-hint = Limpar a marcação
meshedit-cell-invert = Inverter
meshedit-cell-invert-hint = Trocar marcadas e desmarcadas
meshedit-cell-delete = Excluir
meshedit-cell-delete-hint = Excluir as faces marcadas
meshedit-cell-crop = Recortar
meshedit-cell-crop-hint = Manter só a área marcada, remover o resto
meshedit-cell-cut = Cortar
meshedit-cell-cut-hint = Mover as faces para uma malha nova — a original fica
meshedit-cell-separate = Separar
meshedit-cell-separate-hint = Dividir a região em uma malha por parte conexa
meshedit-cell-close-holes = Fechar buracos
meshedit-cell-close-holes-hint = Fechar buracos só com as faces vizinhas marcadas. Bordas abertas.
meshedit-sculpt-addremove = Adicionar / Remover  [1]
meshedit-sculpt-addremove-hint = Depositar arrastando; Shift escava. Shift+roda redimensiona, Ctrl+roda muda a força. Tecla: 1.
meshedit-sculpt-smooth = Suavizar  [2]
meshedit-sculpt-smooth-hint = Relaxar arrastando; Shift força o máximo. Shift+roda redimensiona, Ctrl+roda muda a força. Tecla: 2.
meshedit-slider-size = tamanho
meshedit-slider-size-hint = Tamanho do pincel (Shift + roda)
meshedit-slider-force = força
meshedit-slider-force-hint = Força do pincel (Ctrl + roda)
meshedit-limit-label = limite
meshedit-limit-checkbox-hint = Limitar o reparo a bordas menores que este perímetro
meshedit-limit-drag-hint = Off fecha todo buraco seguro da área; a borda fica aberta
meshedit-status-unsaved = Edições não salvas
meshedit-status-unsaved-hint = Pendente: Concluir aplica, Cancelar reverte
meshedit-status-hint-sculpt = Arraste para esculpir · BRD orbita
meshedit-status-hint-object = Clique num objeto para inteiro · Shift desmarca
meshedit-status-hint-lasso = Clique contorna · duplo-clique fecha · Shift desmarca
meshedit-status-hint-default = Arraste uma caixa · Shift desmarca · Del exclui
meshedit-session-undo = Desfazer
meshedit-session-undo-hint = Desfazer a última edição (Ctrl+Z)
meshedit-session-redo = Refazer
meshedit-session-redo-hint = Refazer a edição desfeita (Ctrl+Y)
meshedit-session-cancel = Cancelar
meshedit-session-cancel-hint = Descartar todas as edições da sessão
meshedit-session-done = Concluir
meshedit-session-done-hint = Aplicar e fechar o editor

## Align Scans window — DRAFT.

align-title = Alinhar escaneamentos
align-tab-auto = Alinhar
align-tab-manual = Ajustar posição
align-constraint-free = Mover/girar em todas as direções
align-constraint-free-hint = Arraste o escaneamento para qualquer lado
align-constraint-z = Mover em z
align-constraint-z-hint = Arrastar só na vertical
align-constraint-xy = Mover no plano xy
align-constraint-xy-hint = Arrastar só na horizontal
align-manual-drag-hint = Move o escaneamento pego · Ctrl+arrastar gira
align-undo = Desfazer
align-undo-hint = Um passo atrás
align-redo = Refazer
align-redo-hint = Um passo à frente
align-prompt-moving = Clique num ponto da malha que deve mover
align-prompt-other = Clique na mesma posição da outra malha
align-prompt-alternate = Clique alternando nas mesmas posições das duas malhas
align-prompt-placed = { $count ->
    [one] { $count } seta posicionada
   *[other] { $count } setas posicionadas
}
align-back = Voltar
align-back-hint = Desfazer uma seta — botão direito faz o mesmo
align-clear = Limpar
align-clear-hint = Soltar as setas e escolher dois escaneamentos — ficam onde estão
align-fit-perform = Executar alinhamento
align-fit-perform-hint = Mover a malha para as setas — mínimo duas setas
align-fit-refine = Ajuste fino
align-fit-refine-hint = Alinhar as áreas inalteradas do escaneamento preparado ao modelo original. Confira o resultado antes de aceitar
align-matching-parts = partes coincidentes
align-matching-parts-hint = Fração máxima das correspondências usadas no refinamento. O Best Fit a reduz quando restam poucas áreas inalteradas
align-max-influence = influência máx.
align-max-influence-hint = Só influi a superfície abaixo desta distância. Valor alto piora
align-orientation-title = A orientação deve coincidir
align-orientation-match = A orientação deve coincidir
align-orientation-inverted = A orientação deve coincidir invertida
align-orientation-ignored = Orientação ignorada
align-orientation-either-hint = Aceita os dois lados. O cálculo costuma demorar mais
align-orientation-facing-hint = Como as duas superfícies se encaram
align-exclude = Ajuste: excluir partes marcadas
align-exclude-hint = Pintar a superfície a ignorar
align-commit-cancel = Cancelar
align-commit-cancel-hint-moved = Devolver tudo e fechar — Ctrl+Z traz o alinhamento
align-commit-cancel-hint-clean = Fechar sem mudar nada
align-commit-done = Concluir
align-commit-done-hint = Manter o alinhamento e fechar — exporte para gravar

## Deviation map — DRAFT.

align-map-heatmap = Mapa de calor
align-map-heatmap-hint = Colorir um escaneamento pela distância ao outro
align-map-requires-refine = Execute Best fit matching primeiro
align-map-max = máx
align-map-min = mín
align-map-not-measured = não medido
align-map-not-measured-hint = Nenhuma superfície do outro escaneamento ao alcance destes vértices. Dente ou ponte num só escaneamento: normal, não erro — nada para medir.

## Align roles, brush, mask commands, align status lines — DRAFT.

align-pair-decided = { $moving } → { $fixed }
align-pair-guessed = { $moving } → { $fixed } (palpite)
align-pair-hint-decided = { $moving } move, { $fixed } fica
align-pair-hint-guessed = Sem cliques, palpite pela ordem de abertura. O primeiro clique decide: { $moving } move, { $fixed } fica
align-pair-swap = Trocar
align-pair-swap-hint = Ajustar ao contrário — as setas acompanham

align-brush-title = Pincel
align-brush-close-hint = Fechar o pincel — marcas mantidas
align-brush-mesh-selection = Seleção de malha
align-brush-moving = Móvel
align-brush-fixed = Fixa
align-brush-both = Ambos
align-brush-both-hint = Pinte e aplique nos dois scans — a superfície sob o cursor recebe o traço
align-brush-size = tamanho do pincel
align-brush-inverse = Pincel inverso
align-brush-inverse-hint = Arrastar apaga em vez de marcar. Shift inverte de novo
align-brush-auto-radius = raio automático
align-brush-auto-radius-hint = Raio mantido em cada ponta de seta
align-brush-size-status = Pincel { $size } mm
align-status-no-summary = Nenhuma superfície comparável

align-mask-fit-everywhere = Ajustar em tudo
align-mask-fit-everywhere-hint = Limpar todas as marcas
align-mask-fit-everywhere-report = Marcas limpas — ajuste no escaneamento todo
align-mask-fit-everywhere-report-one = { $name }: marcações apagadas
align-mask-fit-nowhere = Não ajustar em nada
align-mask-fit-nowhere-hint = Marcar a malha toda — ajuste sem efeito
align-mask-fit-nowhere-report = Malha toda marcada — ajuste sem efeito
align-mask-fit-nowhere-report-one = { $name }: scan inteiro excluído da correspondência
align-mask-invert = Inverter marcas
align-mask-invert-hint = Marcar o desmarcado e vice-versa
align-mask-invert-report = Marcas invertidas
align-mask-invert-report-one = { $name }: marcações invertidas
align-mask-automatic = Marca automática
align-mask-automatic-hint = Ajustar só numa área pequena em cada ponta
align-mask-automatic-report = Ajuste nas pontas de seta
align-mask-automatic-report-one = { $name }: correspondência só em torno das pontas das setas

align-status-half-dropped = Seta pela metade descartada
align-status-turned = Par virado
align-status-cleared = Par limpo
align-status-click-moving = Clique num ponto do escaneamento que deve mover
align-status-click-alternate = Clique alternando nas mesmas posições
align-status-two-scans = Dois escaneamentos à vista — um ponto em cada
align-status-no-surface = Nuvem de pontos não tem superfície para parear
align-status-now-other = Clique no ponto correspondente do outro
align-status-moved = Ponto movido
align-status-wrong-scan = Esse escaneamento não é do par — Limpe e recomece
align-status-place-first = Posicione um ponto em cada escaneamento
align-status-one-scan = Um dos escaneamentos
align-status-scaled = Esse escaneamento tem escala aplicada, não alinhável
align-status-pose-refused = Ajuste pronto, mas seu escaneamento sumiu
align-status-worker-unavailable = O processo de alinhamento parou — reinicie a ferramenta
align-status-measure-dropped = Medição descartada — o pincel tem as cores
align-status-measure-unavailable = Medição não aplicada — o escaneamento mudou; execute Best fit matching novamente
align-status-map-elsewhere = O mapa está na aba Automático — volta lá
align-status-aligned-points = Alinhado por pontos

## Align result status lines — DRAFT.

align-status-aligned = Alinhado pelos pontos — execute Best fit matching para assentar as superfícies.
align-status-refined = Best fit pronto
align-status-measured = Mapa de calor atualizado
align-status-remeasure = { $reason } — rode o ajuste fino para medir de novo
align-status-settings-changed = Configurações de matching alteradas
align-status-visibility-changed = Visibilidade de um escaneamento selecionado alterada
align-brush-not-in-alignment = Esse escaneamento não faz parte deste alinhamento
align-drag-moving = Movendo { $name } à mão
align-drag-unrecorded = Movido à mão, mas esta etapa não entrou no histórico — Ctrl+Z não a desfará
align-drag-moved = { $name }: movimento de { $moved } mm à mão (Ctrl+Z desfaz)
align-status-moved-hand = Movido à mão
align-pair-placed = Par { $n } posicionado
align-roles-swapped = { $moving } move agora; { $fixed } não se move
align-status-scan-changed = O escaneamento mudou
align-status-hidden = Camada oculta: { $name }. Mostre-a para alinhar contra ela
align-arrow-removed = { $n ->
    [one] Seta removida — resta { $n } par
   *[other] Setas removidas — restam { $n } pares
}
align-status-markings-changed = Marcas mudadas
align-status-place-arrow-first = Posicione ao menos uma seta antes de marcar
align-status-arrows-cleared = Setas fora — na mão daqui em diante

## Unsaved-work guards and error dialog buttons — DRAFT.

guard-close-title = Edições de malha não salvas
guard-close-headline-one = 1 camada editada não salva.
guard-close-headline-many = Camadas editadas não salvas.
guard-close-note = { $count } camadas editadas afetadas.
guard-close-detail = Salvar exporta cada camada (PLY, STL ou OBJ) e fecha.
guard-close-destructive = Fechar sem salvar
guard-replace-title = Edição em curso
guard-replace-headline-session = Há uma sessão ativa em { $layer }.
guard-replace-headline-one = 1 camada com mudanças não salvas.
guard-replace-headline-many = { $count } camadas com mudanças não salvas.
guard-replace-detail = Abrir uma cena fecha a sessão e descarta o não salvo.
guard-replace-destructive = Descartar e abrir
guard-save = Salvar…
guard-cancel = Cancelar

error-retry-graphics = Tentar de novo
error-close = Fechar
error-copy-details = Copiar detalhes

about-website = Site
about-source = Código
about-licenses = Licenças de terceiros
about-license-kind = Licença Apache 2.0

## Mesh-edit operations, undo/redo, sculpt, measure, cut ruler, scene menu — DRAFT.

edit-select-faces-first = Selecione faces da malha antes
edit-no-changes = Sem mudanças: { $layer }
edit-apply-failed-title = Não deu para editar a seleção
edit-apply-failed-summary = Não deu para editar a seleção: { $detail }
edit-no-changes-hidden = Sem mudanças: refine a seleção; camadas ocultas seguem intactas
edit-selected-faces = { $faces ->
    [one] { $faces } face selecionada
   *[other] { $faces } faces selecionadas
}
edit-selected-faces-across = { $faces ->
    [one] { $faces } face selecionada em { $layers } camadas
   *[other] { $faces } faces selecionadas em { $layers } camadas
}

holes-nothing = Nada para fechar: { $layer }
holes-partial = { $segments }, nada fechado: { $layer }
holes-closed = { $filled ->
    [one] Fechado { $filled } buraco
   *[other] Fechados { $filled } buracos
}
holes-closed-detail = { $closed }: { $layer }
holes-closed-segments = { $closed } ({ $segments }): { $layer }
holes-seg-healed = { $n ->
    [one] Curada { $n } fissura
   *[other] Curadas { $n } fissuras
}
holes-seg-border = borda do escaneamento aberta
holes-seg-oversize-limit = { $n ->
    [one] { $n } buraco acima do limite de { $limit } mm
   *[other] { $n } buracos acima do limite de { $limit } mm
}
holes-seg-oversize = { $n ->
    [one] { $n } buraco grande demais
   *[other] { $n } buracos grandes demais
}
holes-seg-damaged = { $n ->
    [one] Pulada { $n } borda danificada
   *[other] Puladas { $n } bordas danificadas
}
batchedit-invert = Normais invertidas
batchedit-close-holes = Buracos internos seguros fechados
batchedit-delete = Seleção excluída
batchedit-crop = Recorte na seleção
batchedit-cut = Seleção cortada para nova camada
batchedit-separate = Seleção separada
batchedit-edited = Camada editada
edit-applied-status = { $action }: { $layer }
batchedit-status = { $label } em { $n ->
    [one] { $n } camada visível
   *[other] { $n } camadas visíveis
}
batchedit-no-changes = Sem mudanças: refine a seleção; ocultas intactas
batch-close-holes = Buracos internos fechados
batch-delete = Seleção excluída
batch-crop = Recorte na seleção
batch-cut = Seleção cortada
batch-separate = Seleção separada
batch-edited = Seleção editada

select-covers-all = A seleção já cobre a malha toda: { $layer }
select-covers-remove = A seleção cobre tudo — remova a camada: { $layer }
select-splits = A seleção divide em { $parts } — refine a seleção: { $layer }
select-faces-cannot = Não dá para selecionar faces: { $layer }

undo-nothing = Nada para desfazer
redo-nothing = Nada para refazer
undo-undid = Edição desfeita: { $layer }
undo-unavailable = Desfazer indisponível — a cena mudou: { $layer }
redo-redid = Edição refeita: { $layer }
redo-unavailable = Refazer indisponível — a cena mudou: { $layer }

sculpt-armed-addremove = Adicionar/Remover: arraste para depositar, Shift escava
sculpt-armed-smooth = Suavizar: arraste para relaxar, Shift força
sculpt-off = Escultura off
sculpt-applied-undo = Escultura aplicada (Ctrl+Z desfaz)
sculpt-applied-locked = Escultura aplicada (sem desfazer: snapshot enorme)
sculpt-failed-title = Escultura falhou
sculpt-failed = Não dá para esculpir esta camada: { $detail }
sculpt-worker-stopped = Processo de escultura parado: { $detail }
sculpt-preparing = Preparando a escultura…
sculpt-nonuniform-scale = A escultura exige uma malha com escala uniforme
sculpt-failure-worker-panicked = Processo de escultura falhou: { $detail }
sculpt-failure-spawn = Não foi possível iniciar o processo de escultura: { $detail }
sculpt-failure-kernel-pool = Não foi possível criar o pool de núcleos de escultura: { $detail }
sculpt-failure-missing-undo-baseline = O traço de escultura não tem base para desfazer
sculpt-failure-shadow-poisoned = O bloqueio de sombra de escultura foi corrompido
sculpt-failure-shadow-shape = A sombra da escultura não corresponde mais à malha ativa
sculpt-failure-invalid-vertex-index = O processo de escultura retornou um índice de vértice inválido
sculpt-failure-worker-state-poisoned = O estado do processo de escultura foi corrompido — reinicie o Sculpt
sculpt-failure-vertex-count-changed = O resultado da escultura mudou o número de vértices
sculpt-failure-topology-rebuild = Falha ao reconstruir a topologia da escultura: { $detail }
sculpt-worker-unavailable = Escultura indisponível
sculpt-finishing = Terminando o traço…
sculpt-finishing-history = Terminando escultura antes do histórico…
sculpt-lasso-armed = Laço armado: clique ou arraste contorna; Enter, duplo-clique ou início fecha
sculpt-lasso-off = Laço desarmado
sculpt-object-on = Escolher objeto: clique para inteiro
sculpt-object-off = Escolher objeto off
sculpt-selection-cleared = Seleção limpa
sculpt-through-on = Seleção através
sculpt-through-off = Seleção de superfície

measure-distance = Distância: { $len }
measure-thickness = Espessura de parede: { $len }
measure-open-wall = Superfície aberta: sem parede oposta na normal
measure-cannot-probe = Aqui não se mede: geometria degenerada
measure-cleared = Medições limpas

cut-lines = Linhas
cut-mesh = Malha
cut-dist = Dist
cut-dist-hint = Distância: clique em dois pontos
cut-thick = Espes
cut-thick-hint = Espessura: clique num ponto do contorno
cut-close-section = Fechar seção
cut-snap = Ímã
cut-snap-hint = Ímã: cliques grudam no contorno
cut-empty = Sem interseção
cut-footer-distance = Arrastar = deslocar · clique 2 pts = distância · direito limpa · roda = zoom
cut-footer-thickness = Arrastar = deslocar · clique contorno = espessura · direito limpa · roda = zoom

recent-clear = Limpar recentes

scene-already-origin = Tudo já na posição original
scene-positions-reset = Posições redefinidas (Ctrl+Z desfaz)

## Session close-outs and layer shortcuts — DRAFT.

session-applied = Sessão aplicada
session-reverted = Sessão revertida
edit-session-busy = Termine ou cancele a sessão antes
layers-none-hidden = Sem camadas ocultas para trazer
layer-opaque-again = Opaco de novo: { $label }
layer-translucent = Translúcido: { $label } (Shift+clique do meio restaura)
layer-restored = Visível de novo: { $label }
layer-hidden = Oculta: { $label } (Shift+Ctrl+clique do meio restaura)
layer-unnamed = camada { $n }
layer-removed = Camada removida: { $label }
layer-face-selection = Seleção de faces: { $label }

## Bridge split panel and align session close-outs — DRAFT.

bridge-panel-title = Dividir bridge
bridge-mode-place = Posicione o disco
bridge-mode-calculating = Calculando…
bridge-mode-ready = Pronto
bridge-mode-failed = Tentativa falha
bridge-kerf = Corte
bridge-disc-size = Tamanho do disco
bridge-cancel = Cancelar
bridge-apply = Dividir bridge
bridge-err-miss = O disco errou o bridge. Posicione no conector.
bridge-err-tangent = O disco só encosta. Atravesse o conector.
bridge-err-small = Diâmetro { $have } mm; aqui precisa { $need } mm.
bridge-err-limit = Este corte pede disco de { $need } mm, acima do limite de { $max } mm.
bridge-err-no-result = Tentativa com superfície preservada, sem resultado útil. Malha original intacta.
bridge-err-invalid-cut = Tentativa falha, corte não validável. Malha original intacta.
bridge-err-invalid-side = Tentativa falha, { $side } não validável. Malha original intacta.
bridge-err-gap = Tentativa falha, vão não preservado. Malha original intacta.
bridge-err-empty = A camada não tem malha para dividir.
bridge-err-invalid = Ajustes do disco inválidos. Reinicie e tente de novo.
bridge-err-unusable = Sem resultado útil. Malha original intacta.

align-session-canceled = Alinhamento cancelado — tudo de volta (Ctrl+Z traz)
align-session-closed = Alinhamento fechado
align-session-closed-running = Alinhamento fechado — um ajuste rodava e foi solto, tudo como visto
align-session-kept = Alinhamento mantido — exporte para gravar

## Settings panel, bridge split, render error, tint — DRAFT.

settings-header = Ajustes
settings-section-files = Arquivos e exportação
settings-save-format = Ao salvar uma digitalização
settings-save-format-hint = O que «Salvar camada» grava. Vale um modo só: o escolhido aqui, não o formato abaixo.
settings-save-format-source = Formato próprio
settings-save-format-source-hint = Uma digitalização aberta como STL é salva como STL, uma aberta como PLY como PLY; um formato sem escritor é salvo como PLY.
settings-save-format-always = Formato escolhido
settings-save-format-always-hint = Toda digitalização é salva no formato escolhido abaixo, salvo se a geometria não puder ser gravada nele.
settings-save-format-source-note = Cada digitalização mantém o formato em que foi aberta. Um formato não gravável é salvo como PLY.
settings-export-format = Formato de exportação alternativo
settings-export-format-hint = Usado para um scan cujo formato próprio não pode ser gravado, ou quando a opção acima está desligada
settings-remember-export = Lembrar pasta de exportação
settings-remember-export-hint = Mesma pasta após reiniciar
settings-section-scene = Visualização e navegação
settings-frame-on-open = Enquadrar ao abrir
settings-frame-on-open-hint = Voltar à vista inicial quando um arquivo substitui a cena
settings-double-click = Duplo-clique recentraliza
settings-double-click-hint = Duplo-clique recentraliza a câmera no ponto
settings-orbit = Velocidade orbital
settings-orbit-hint = Quão rápido orbita arrastando com o direito
settings-zoom = Velocidade de zoom
settings-zoom-hint = Quanto cada clique da roda aproxima
settings-background = Fundo
settings-bg-gray = Cinza
settings-bg-white = Branco
settings-bg-dark = Escuro
settings-ghost = Fantasma do lado cortado
settings-ghost-hint = Na vista de corte, mostrar o lado removido como fantasma
settings-theme = Tema
settings-theme-light = Claro
settings-theme-dark = Escuro
settings-scale = Escala da interface
settings-scale-hint = Escala tudo; 1.0 mantém o sistema
settings-measurements = Medições
settings-section-appearance = Aparência
settings-section-mesh = Edição de malhas
settings-remember-brush = Lembrar pincel
settings-remember-brush-hint = Manter tamanho e força entre sessões
settings-section-updates = Atualizações
settings-check-auto = Verificar ao iniciar
settings-check-now = Verificar
settings-check-disabled-hint = Verificações desativadas pelo ambiente
settings-check-busy-hint = Já há verificação em curso
settings-update-disabled = Desativado pelo ambiente
settings-update-checking = Verificando…
settings-update-current = Em dia
settings-update-skipped = Versão pulada
settings-update-failed = Não deu para verificar
settings-save-error = Preferências não salvas. Tentando de novo…
settings-save-error-hint = Arquivo de preferências indisponível
settings-shortcuts = Atalhos de teclado
settings-about = Sobre o OccluView

bridge-busy = Termine ou cancele a divisão antes
bridge-active = Divisão já ativa
bridge-target-gone = Alvo perdido
bridge-needs-mesh = A divisão precisa de malha visível
bridge-place-disc = Divisão: posicione o disco separador
bridge-canceled-scene = Divisão cancelada: cena fechada
bridge-canceled-camera = Divisão cancelada: sem câmera
bridge-canceled-changed = Divisão cancelada: a malha mudou
bridge-canceled = Divisão cancelada
bridge-calculating = Divisão: calculando…
bridge-unavailable = Divisão indisponível agora
bridge-preview-stale = Prévia expirada
bridge-not-applied = Divisão não aplicada
bridge-complete = Divisão completa
bridge-complete-surface = Divisão completa (superfície; bordas naturais intactas)
bridge-complete-locked = Divisão completa (sem desfazer: snapshot enorme)

render-failed-title = Não deu para renderizar
render-failed-summary = O arquivo abriu, mas a vista não renderiza.
render-failed-status = Falha no render

tint-choose = Escolher matiz

## Status tail: brush, lasso, loading, GPU, align jobs — DRAFT.

brush-no-mesh = Clique num ponto de cada malha, depois pinte
lasso-dropped = Laço solto
lasso-needs-points = O laço precisa de 3 pontos
loading-scene = Carregando cena…
gpu-failed-status = O driver reportou um problema
gpu-retry-status = Tentando gráficos de novo — se o problema continuar, salve o trabalho e reinicie o OccluView
gpu-failed-title = Problema gráfico
gpu-failed-summary = O driver falhou ao desenhar. A vista pode estar incompleta. Salve e reinicie se repetir.
align-job-align = Alinhando…
align-job-refine = Refinando…
align-job-measure = Medindo…
align-markings-dropped = Marcas soltas — a superfície mudou depois

## Worker-built align failures — DRAFT.

align-fail-no-surface-fixed = O escaneamento fixo não tem superfície útil
align-fail-no-surface-moving = O escaneamento móvel não tem superfície útil
align-fail-recolor = Medição descartada antes de colorir
align-fail-unobservable = A superfície não é observável o suficiente para um mapa de desvios confiável
align-reject-toofew = Coloque mais setas ou aproxime os escaneamentos
align-reject-unpaired = Complete os dois lados de cada seta
align-reject-degenerate-plain = Distribua os pontos pela superfície
align-reject-unit = Os escaneamentos usam unidades diferentes
align-reject-apart = Revise as setas e aproxime os escaneamentos
align-reject-runaway = Aproxime os escaneamentos e tente Best fit matching novamente
align-reject-no-improvement = Nenhuma melhora confirmada — aproxime os escaneamentos e tente novamente
align-reject-ambiguous = O ajuste encontrou várias superfícies igualmente prováveis — marque a área correspondente ou aproxime os escaneamentos
align-reject-nonfinite = O ponto ou a superfície selecionados são inválidos
align-status-stepped = Passos pelo histórico
align-status-moving-hand = Movendo à mão

## Contatos oclusais: clique com o botão direito em um escaneamento e veja onde
## ele encontra o escaneamento antagonista. Uma leitura é o papel de articular
## (apenas marcas, coloridas por profundidade), a outra o mapa de aproximação
## (quão perto, em toda parte). Um controle move a profundidade que a escala lê
## como carga total, e recolore um campo já medido em vez de medir de novo.
layer-menu-contacts = Mostrar contatos
layer-menu-hide-contacts = Ocultar contatos

contact-title = Contatos oclusais
contact-close-hint = Fechar a leitura e retirar as marcas dos dois escaneamentos
contact-against = { $subject } contra { $antagonist }
contact-unknown-layer = um escaneamento que não está mais aberto

contact-mode-marks = Contatos
contact-mode-marks-hint = Onde as superfícies se encontram, colorido pela intensidade — o resto fica limpo, como o papel de articular deixa
contact-mode-approach = Aproximação
contact-mode-approach-hint = Quão perto o outro escaneamento está em toda parte, carga incluída

contact-load-label = carga em
contact-load-suffix = mm
contact-load-hint = A profundidade em que esta escala é lida como carga total. Movê-la recolore o mapa já medido, sem medir de novo.
contact-flatten = Uma cor por contato
contact-flatten-hint = Reduzir cada área de contato ao seu ponto mais profundo. Desligado mantém a distribuição de força dentro de cada marca.
# Label above the list of layers a contact reading can be measured against.
contact-antagonist-pick = Medido contra
contact-antagonist-pick-hint = A digitalização mais próxima é escolhida automaticamente. Escolha outra camada para medir contra ela.

contact-legend-deepest = { $mm } mm na oclusão

contact-stats-area = Área de contato
contact-stats-contacts = Contatos
contact-stats-deepest = Mais profundo

contact-readout-gap = folga
contact-readout-load = carga

contact-status-measuring = Medindo…
contact-status-measuring-hint = As duas superfícies estão sendo lidas uma contra a outra
contact-status-remeasuring = Medindo de novo…
contact-status-remeasuring-hint = Um escaneamento se moveu, então as distâncias mudaram. O mapa está sendo lido novamente.
contact-status-needs-second = Uma leitura de contatos precisa de um segundo escaneamento visível para medir
contact-status-no-surface = Um dos escaneamentos não tem superfície para medir
contact-status-worker-failed = A medição não foi concluída

contact-opened = Lendo contatos em { $label }
contact-closed = Leitura de contatos encerrada
help-section-contacts = Contatos oclusais
help-hintline-contacts = Clique com o botão direito em uma camada · Mostrar contatos · o controle «carga em» recolore · Esc fecha
help-hint-contacts-read-its-occlusal-contacts-against-the-scan-it-bites = Ler os contatos oclusais contra o escaneamento antagonista
help-hint-contacts-read-the-contact-depth-under-the-cursor = Ler a profundidade do contato sob o ponteiro, em qualquer das arcadas
help-hint-contacts-move-the-depth-the-ramp-calls-fully-loaded = Mover a profundidade que a escala lê como carga total
help-hint-contacts-switch-between-marks-only-and-the-whole-approach = Alternar entre apenas marcas e toda a aproximação
help-hint-contacts-close-the-reading-and-take-the-marks-off-both-scans = Fechar a leitura e retirar as marcas dos dois escaneamentos
contact-retry = Ler novamente
contact-status-subject-unusable = O escaneamento desta leitura não pode ser medido agora
contact-status-subject-unusable-hint = Mostre-o de novo ou deixe-o como malha de triângulos, e a leitura continua
contact-status-antagonist-unusable = O escaneamento usado como referência não pode ser medido agora
contact-status-antagonist-unusable-hint = Mostre-o de novo ou deixe-o como malha de triângulos, e a leitura continua
contact-status-no-overlap = Os escaneamentos estão longe demais
contact-status-no-overlap-hint = Nada em nenhuma das superfícies ficou ao alcance da leitura. Verifique se estão em oclusão.
contact-status-failed-hint = Leia novamente; se continuar falhando, o par talvez precise de reparo antes.
contact-status-needs-second-hint = Abra o escaneamento antagonista ou mostre-o de novo e inicie a leitura
contact-legend-gap = folga até { $mm } mm
contact-stats-balance = Área por lado
contact-stats-balance-hint = Área de contato de cada lado da linha média do próprio escaneamento. A divisão segue as coordenadas do escaneamento, então um caso girado ou espelhado pode trocar os números.
layer-menu-contacts-unavailable = Uma leitura de contatos precisa de duas malhas de triângulos visíveis — mostre ou abra antes o escaneamento antagonista

contact-details = Detalhes
contact-details-hint = Os números e a regra de uma cor por contato
contact-details-close = Ocultar detalhes
settings-shortcuts-hint = Referência de teclado e mouse (F1)

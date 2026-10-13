# Measurements and units

OccluView works in millimeters. STL, PLY, OBJ, OFF, and HPS do not establish a
trusted physical unit, so their numbers are read as millimeters. GLB declares
meters, but scanner exports also use millimeter numbers. An ambiguous GLB keeps
its coordinates and carries a persistent unit warning.

Use the layer's context menu to confirm millimeters or meters. The size
recommendation belongs to that layer's original import bounds and is advisory.
Only the explicit decision changes scale; confirming meters applies ×1000 in
the layer's local frame. The decision also updates pieces of that same source
in the current scene, with one Undo step; other scenes remain independent.
Placement is preserved. Undo restores the previous
interpretation and scale, and repeating a confirmation does not apply it twice.
An operator decision is recorded separately from format-declared certainty.
The warning beside measurements remains while a visible layer has unresolved
units. Exported STL/PLY/OBJ geometry uses the accepted scene transform; these
formats still do not carry the GLB decision as a project history.

## Deviation

The summary uses measured vertices: mean absolute deviation, RMS, percentiles,
and the share within the selected tolerance all give each measured vertex one
weight. More densely sampled regions contribute more. Remeshing the same
surface can therefore change the summary without changing its physical shape.

Search-candidate residual evidence uses its own area-weighted population. It is
separate from this vertex-weighted deviation summary; their RMS and percentile
values need not agree.

The maximum is sampled over the measured moving vertices; it is not a
certified continuous-surface maximum. Unmeasured vertices are excluded, with
their causes retained separately. If too little is measured, the summary is
absent. Absence does not mean zero deviation or perfect alignment.

The heatmap smooths a copy of the measured field for display. Statistics retain
the raw readings. Display smoothing uses three triangle-edge neighbourhood
passes after coincident positions are joined; its physical radius depends on
the tessellation. A smooth image does not establish measurement accuracy.

## Contacts

Contact area is approximate: each triangle contributes its area multiplied by
the fraction of its three corners marked as touching. This does not clip an
exact contact contour through every triangle. The reported negative-X and
positive-X portions use the subject's raw coordinate X relative to its vertex
mean, rather than an anatomical left/right reference. Rotating or mirroring a
scan can change that split.

Positive gaps use Euclidean closest-point distance. Negative penetration uses
the displacement projected onto the closest triangle's normal; near an edge,
its magnitude can be smaller than the Euclidean distance. It is a local
normal-depth estimate, not a signed distance field.

Penetration and contact colors describe geometry in millimeters. They do not
measure physical force. Comparisons require known units, a consistent pose and
sampling policy, and a suitable reference for the intended use.

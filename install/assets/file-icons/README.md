# File-type icon sources

`occluview-3d.ico` is what the installers ship: the multi-resolution icon
Windows shows for `.stl`, `.ply`, `.obj`, `.glb` and `.hps` files.

`occluview-3d.svg` is the drawing, and `occluview-3d-master.png` is the render
the `.ico` is built from. Neither is referenced by any build, on purpose:
they exist so the icon can be regenerated at another size or with a corrected
colour without redrawing it. Deleting them because "nothing uses them" would
mean redrawing the icon the next time Windows asks for a resolution the `.ico`
does not carry.

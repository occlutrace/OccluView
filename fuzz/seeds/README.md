# Fuzz seeds

One minimal, valid file per reader, small enough that libFuzzer can mutate
every byte and structured enough that it starts past the header check.

Without seeds, sixty seconds of mutation from random bytes spends its whole
budget rediscovering magic numbers. The formats that most need coverage are the
ones furthest behind a gate: GLB needs the `glTF` magic, a parseable JSON chunk
and a coherent accessor table before `gltf/accessor.rs` does any arithmetic at
all; HPS needs well-formed XML and valid base64.

The ordinary mesh seeds were verified by the shipping reader —
`occluview-cli info` reports 3 vertices and 1 triangle for each. Resource-limit
seeds deliberately claim counts that exceed the import budget and must be
refused before allocation.

`fuzz/corpus/` is the accumulating working corpus libFuzzer writes into. It is
generated, gitignored, and cached between CI runs so a weekly deep run feeds the
next one; these seeds are the floor it starts from.
